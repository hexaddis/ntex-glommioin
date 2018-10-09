use std::collections::VecDeque;
use std::fmt::Debug;
use std::time::Instant;

use actix_net::codec::Framed;
use actix_net::service::Service;

use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_timer::Delay;

use error::{ParseError, PayloadError};
use payload::{Payload, PayloadSender, PayloadStatus, PayloadWriter};

use body::{Body, BodyStream};
use config::ServiceConfig;
use error::DispatchError;
use request::Request;
use response::Response;

use super::codec::{Codec, InMessage, OutMessage};

const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    pub struct Flags: u8 {
        const STARTED            = 0b0000_0001;
        const KEEPALIVE_ENABLED  = 0b0000_0010;
        const KEEPALIVE          = 0b0000_0100;
        const POLLED             = 0b0000_1000;
        const FLUSHED            = 0b0001_0000;
        const SHUTDOWN           = 0b0010_0000;
        const DISCONNECTED       = 0b0100_0000;
    }
}

/// Dispatcher for HTTP/1.1 protocol
pub struct Dispatcher<T, S: Service>
where
    S::Error: Debug,
{
    service: S,
    flags: Flags,
    framed: Framed<T, Codec>,
    error: Option<DispatchError<S::Error>>,
    config: ServiceConfig,

    state: State<S>,
    payload: Option<PayloadSender>,
    messages: VecDeque<Message>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,
}

enum Message {
    Item(Request),
    Error(Response),
}

enum State<S: Service> {
    None,
    ServiceCall(S::Future),
    SendResponse(Option<(OutMessage, Body)>),
    SendPayload(Option<BodyStream>, Option<OutMessage>),
}

impl<S: Service> State<S> {
    fn is_empty(&self) -> bool {
        if let State::None = self {
            true
        } else {
            false
        }
    }
}

impl<T, S> Dispatcher<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response>,
    S::Error: Debug,
{
    /// Create http/1 dispatcher.
    pub fn new(stream: T, config: ServiceConfig, service: S) -> Self {
        Dispatcher::with_timeout(stream, config, None, service)
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub fn with_timeout(
        stream: T, config: ServiceConfig, timeout: Option<Delay>, service: S,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let flags = if keepalive {
            Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED | Flags::FLUSHED
        } else {
            Flags::FLUSHED
        };
        let framed = Framed::new(stream, Codec::new(config.clone()));

        // keep-alive timer
        let (ka_expire, ka_timer) = if let Some(delay) = timeout {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = config.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (config.now(), None)
        };

        Dispatcher {
            payload: None,
            state: State::None,
            error: None,
            messages: VecDeque::new(),
            service,
            flags,
            framed,
            config,
            ka_expire,
            ka_timer,
        }
    }

    fn can_read(&self) -> bool {
        if self.flags.contains(Flags::DISCONNECTED) {
            return false;
        }

        if let Some(ref info) = self.payload {
            info.need_read() == PayloadStatus::Read
        } else {
            true
        }
    }

    // if checked is set to true, delay disconnect until all tasks have finished.
    fn client_disconnected(&mut self) {
        self.flags.insert(Flags::DISCONNECTED);
        if let Some(mut payload) = self.payload.take() {
            payload.set_error(PayloadError::Incomplete);
        }
    }

    /// Flush stream
    fn poll_flush(&mut self) -> Poll<(), DispatchError<S::Error>> {
        if !self.flags.contains(Flags::FLUSHED) {
            match self.framed.poll_complete() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    Err(err.into())
                }
                Ok(Async::Ready(_)) => {
                    // if payload is not consumed we can not use connection
                    if self.payload.is_some() && self.state.is_empty() {
                        return Err(DispatchError::PayloadIsNotConsumed);
                    }
                    self.flags.insert(Flags::FLUSHED);
                    Ok(Async::Ready(()))
                }
            }
        } else {
            Ok(Async::Ready(()))
        }
    }

    fn poll_response(&mut self) -> Result<(), DispatchError<S::Error>> {
        let mut retry = self.can_read();

        // process
        loop {
            let state = match self.state {
                State::None => loop {
                    break if let Some(msg) = self.messages.pop_front() {
                        match msg {
                            Message::Item(req) => Some(self.handle_request(req)?),
                            Message::Error(res) => Some(State::SendResponse(Some((
                                OutMessage::Response(res),
                                Body::Empty,
                            )))),
                        }
                    } else {
                        None
                    };
                },
                // call inner service
                State::ServiceCall(ref mut fut) => {
                    match fut.poll().map_err(DispatchError::Service)? {
                        Async::Ready(mut res) => {
                            self.framed.get_codec_mut().prepare_te(&mut res);
                            let body = res.replace_body(Body::Empty);
                            Some(State::SendResponse(Some((
                                OutMessage::Response(res),
                                body,
                            ))))
                        }
                        Async::NotReady => None,
                    }
                }
                // send respons
                State::SendResponse(ref mut item) => {
                    let (msg, body) = item.take().expect("SendResponse is empty");
                    match self.framed.start_send(msg) {
                        Ok(AsyncSink::Ready) => {
                            self.flags.set(
                                Flags::KEEPALIVE,
                                self.framed.get_codec().keepalive(),
                            );
                            self.flags.remove(Flags::FLUSHED);
                            match body {
                                Body::Empty => Some(State::None),
                                Body::Binary(bin) => Some(State::SendPayload(
                                    None,
                                    Some(OutMessage::Chunk(bin.into())),
                                )),
                                Body::Streaming(stream) => {
                                    Some(State::SendPayload(Some(stream), None))
                                }
                            }
                        }
                        Ok(AsyncSink::NotReady(msg)) => {
                            *item = Some((msg, body));
                            return Ok(());
                        }
                        Err(err) => {
                            if let Some(mut payload) = self.payload.take() {
                                payload.set_error(PayloadError::Incomplete);
                            }
                            return Err(DispatchError::Io(err));
                        }
                    }
                }
                // Send payload
                State::SendPayload(ref mut stream, ref mut bin) => {
                    if let Some(item) = bin.take() {
                        match self.framed.start_send(item) {
                            Ok(AsyncSink::Ready) => {
                                self.flags.remove(Flags::FLUSHED);
                            }
                            Ok(AsyncSink::NotReady(item)) => {
                                *bin = Some(item);
                                return Ok(());
                            }
                            Err(err) => return Err(DispatchError::Io(err)),
                        }
                    }
                    if let Some(ref mut stream) = stream {
                        match stream.poll() {
                            Ok(Async::Ready(Some(item))) => match self
                                .framed
                                .start_send(OutMessage::Chunk(Some(item.into())))
                            {
                                Ok(AsyncSink::Ready) => {
                                    self.flags.remove(Flags::FLUSHED);
                                    continue;
                                }
                                Ok(AsyncSink::NotReady(msg)) => {
                                    *bin = Some(msg);
                                    return Ok(());
                                }
                                Err(err) => return Err(DispatchError::Io(err)),
                            },
                            Ok(Async::Ready(None)) => Some(State::SendPayload(
                                None,
                                Some(OutMessage::Chunk(None)),
                            )),
                            Ok(Async::NotReady) => return Ok(()),
                            // Err(err) => return Err(DispatchError::Io(err)),
                            Err(_) => return Err(DispatchError::Unknown),
                        }
                    } else {
                        Some(State::None)
                    }
                }
            };

            match state {
                Some(state) => self.state = state,
                None => {
                    // if read-backpressure is enabled and we consumed some data.
                    // we may read more dataand retry
                    if !retry && self.can_read() && self.poll_request()? {
                        retry = self.can_read();
                        continue;
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    fn handle_request(
        &mut self, req: Request,
    ) -> Result<State<S>, DispatchError<S::Error>> {
        let mut task = self.service.call(req);
        match task.poll().map_err(DispatchError::Service)? {
            Async::Ready(mut res) => {
                self.framed.get_codec_mut().prepare_te(&mut res);
                let body = res.replace_body(Body::Empty);
                Ok(State::SendResponse(Some((OutMessage::Response(res), body))))
            }
            Async::NotReady => Ok(State::ServiceCall(task)),
        }
    }

    /// Process one incoming requests
    pub(self) fn poll_request(&mut self) -> Result<bool, DispatchError<S::Error>> {
        // limit a mount of non processed requests
        if self.messages.len() >= MAX_PIPELINED_MESSAGES {
            return Ok(false);
        }

        let mut updated = false;
        'outer: loop {
            match self.framed.poll() {
                Ok(Async::Ready(Some(msg))) => {
                    updated = true;
                    self.flags.insert(Flags::STARTED);

                    match msg {
                        InMessage::Message { req, payload } => {
                            if payload {
                                let (ps, pl) = Payload::new(false);
                                *req.inner.payload.borrow_mut() = Some(pl);
                                self.payload = Some(ps);
                            }

                            // handle request early
                            if self.state.is_empty() {
                                self.state = self.handle_request(req)?;
                            } else {
                                self.messages.push_back(Message::Item(req));
                            }
                        }
                        InMessage::Chunk(Some(chunk)) => {
                            if let Some(ref mut payload) = self.payload {
                                payload.feed_data(chunk);
                            } else {
                                error!(
                                    "Internal server error: unexpected payload chunk"
                                );
                                self.flags.insert(Flags::DISCONNECTED);
                                self.messages.push_back(Message::Error(
                                    Response::InternalServerError().finish(),
                                ));
                                self.error = Some(DispatchError::InternalError);
                            }
                        }
                        InMessage::Chunk(None) => {
                            if let Some(mut payload) = self.payload.take() {
                                payload.feed_eof();
                            } else {
                                error!("Internal server error: unexpected eof");
                                self.flags.insert(Flags::DISCONNECTED);
                                self.messages.push_back(Message::Error(
                                    Response::InternalServerError().finish(),
                                ));
                                self.error = Some(DispatchError::InternalError);
                            }
                        }
                    }
                }
                Ok(Async::Ready(None)) => {
                    self.client_disconnected();
                    break;
                }
                Ok(Async::NotReady) => break,
                Err(ParseError::Io(e)) => {
                    self.client_disconnected();
                    self.error = Some(DispatchError::Io(e));
                    break;
                }
                Err(e) => {
                    if let Some(mut payload) = self.payload.take() {
                        payload.set_error(PayloadError::EncodingCorrupted);
                    }

                    // Malformed requests should be responded with 400
                    self.messages
                        .push_back(Message::Error(Response::BadRequest().finish()));
                    self.flags.insert(Flags::DISCONNECTED);
                    self.error = Some(e.into());
                    break;
                }
            }
        }

        if self.ka_timer.is_some() && updated {
            if let Some(expire) = self.config.keep_alive_expire() {
                self.ka_expire = expire;
            }
        }
        Ok(updated)
    }

    /// keep-alive timer
    fn poll_keepalive(&mut self) -> Result<(), DispatchError<S::Error>> {
        if let Some(ref mut timer) = self.ka_timer {
            match timer.poll() {
                Ok(Async::Ready(_)) => {
                    // if we get timer during shutdown, just drop connection
                    if self.flags.contains(Flags::SHUTDOWN) {
                        return Err(DispatchError::DisconnectTimeout);
                    } else if timer.deadline() >= self.ka_expire {
                        // check for any outstanding response processing
                        if self.state.is_empty() {
                            if self.flags.contains(Flags::STARTED) {
                                trace!("Keep-alive timeout, close connection");
                                self.flags.insert(Flags::SHUTDOWN);

                                // start shutdown timer
                                if let Some(deadline) =
                                    self.config.client_disconnect_timer()
                                {
                                    timer.reset(deadline)
                                } else {
                                    return Ok(());
                                }
                            } else {
                                // timeout on first request (slow request) return 408
                                trace!("Slow request timeout");
                                self.flags.insert(Flags::STARTED | Flags::DISCONNECTED);
                                self.state = State::SendResponse(Some((
                                    OutMessage::Response(
                                        Response::RequestTimeout().finish(),
                                    ),
                                    Body::Empty,
                                )));
                            }
                        } else if let Some(deadline) = self.config.keep_alive_expire() {
                            timer.reset(deadline)
                        }
                    } else {
                        timer.reset(self.ka_expire)
                    }
                }
                Ok(Async::NotReady) => (),
                Err(e) => {
                    error!("Timer error {:?}", e);
                    return Err(DispatchError::Unknown);
                }
            }
        }

        Ok(())
    }
}

impl<T, S> Future for Dispatcher<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response>,
    S::Error: Debug,
{
    type Item = ();
    type Error = DispatchError<S::Error>;

    #[inline]
    fn poll(&mut self) -> Poll<(), Self::Error> {
        if self.flags.contains(Flags::SHUTDOWN) {
            self.poll_keepalive()?;
            try_ready!(self.poll_flush());
            Ok(AsyncWrite::shutdown(self.framed.get_mut())?)
        } else {
            self.poll_keepalive()?;
            self.poll_request()?;
            self.poll_response()?;
            self.poll_flush()?;

            // keep-alive and stream errors
            if self.state.is_empty() && self.flags.contains(Flags::FLUSHED) {
                if let Some(err) = self.error.take() {
                    Err(err)
                } else if self.flags.contains(Flags::DISCONNECTED) {
                    Ok(Async::Ready(()))
                }
                // disconnect if keep-alive is not enabled
                else if self.flags.contains(Flags::STARTED) && !self
                    .flags
                    .intersects(Flags::KEEPALIVE | Flags::KEEPALIVE_ENABLED)
                {
                    self.flags.insert(Flags::SHUTDOWN);
                    self.poll()
                } else {
                    Ok(Async::NotReady)
                }
            } else {
                Ok(Async::NotReady)
            }
        }
    }
}
