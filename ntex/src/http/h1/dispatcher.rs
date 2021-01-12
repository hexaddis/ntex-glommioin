use std::{fmt, io, mem, net, pin::Pin, rc::Rc, task::Context, task::Poll};

use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use futures::{ready, Future};

use crate::codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed, FramedParts};
use crate::http::body::{Body, BodySize, MessageBody, ResponseBody};
use crate::http::config::DispatcherConfig;
use crate::http::error::{DispatchError, ParseError, PayloadError, ResponseError};
use crate::http::helpers::DataFactory;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::rt::time::{delay_until, Delay, Instant};
use crate::Service;

use super::codec::Codec;
use super::payload::{Payload, PayloadSender, PayloadStatus};
use super::{Message, MessageType};

const READ_LW_BUFFER_SIZE: usize = 1024;
const READ_HW_BUFFER_SIZE: usize = 4096;
const WRITE_LW_BUFFER_SIZE: usize = 2048;
const WRITE_HW_BUFFER_SIZE: usize = 8192;
const BUFFER_SIZE: usize = 32_768;

bitflags! {
    pub struct Flags: u16 {
        /// We parsed one complete request message
        const STARTED            = 0b0000_0001;
        /// Keep-alive is enabled on current connection
        const KEEPALIVE          = 0b0000_0010;
        /// Socket is disconnected, read or write side
        const DISCONNECT         = 0b0000_0100;
        /// Connection is upgraded or request parse error (bad request)
        const STOP_READING       = 0b0000_1000;
        /// Shutdown is in process (flushing and io shutdown timer)
        const SHUTDOWN           = 0b0001_0000;
        /// Io shutdown process started
        const SHUTDOWN_IO        = 0b0010_0000;
        /// Shutdown timer is started
        const SHUTDOWN_TM        = 0b0100_0000;
        /// Connection is upgraded
        const UPGRADE            = 0b1000_0000;
        /// All data has been read
        const READ_EOF           = 0b0001_0000_0000;
        /// Keep alive is enabled
        const HAS_KEEPALIVE      = 0b0010_0000_0000;
    }
}

pin_project_lite::pin_project! {
/// Dispatcher for HTTP/1.1 protocol
pub struct Dispatcher<T, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: ResponseError,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: ResponseError,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    #[pin]
    call: CallState<S, X>,
    inner: InnerDispatcher<T, S, B, X, U>,
    #[pin]
    upgrade: Option<U::Future>,
}
}

struct InnerDispatcher<T, S, B, X, U>
where
    S: Service<Request = Request>,
    S::Error: ResponseError,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: ResponseError,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    config: Rc<DispatcherConfig<S, X, U>>,
    on_connect: Option<Box<dyn DataFactory>>,
    peer_addr: Option<net::SocketAddr>,
    flags: Flags,
    error: Option<DispatchError>,

    res_payload: Option<ResponseBody<B>>,
    req_payload: Option<PayloadSender>,

    ka_expire: Instant,
    ka_timer: Option<Delay>,

    io: Option<T>,
    read_buf: BytesMut,
    write_buf: BytesMut,
    codec: Codec,
}

enum DispatcherMessage {
    Request(Request),
    Upgrade(Request),
    Error(Response<()>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PollWrite {
    /// allowed to process next request
    AllowNext,
    /// write buffer is full
    Pending,
    /// waiting for response stream (app response)
    /// or write buffer is full
    PendingResponse,
}

pin_project_lite::pin_project! {
    #[project = CallStateProject]
    enum CallState<S: Service, X: Service> {
        Io,
        Expect { #[pin] fut: X::Future },
        Service { #[pin] fut: S::Future },
    }
}

enum CallProcess<S: Service, X: Service, U: Service> {
    /// next call is available
    Next(CallState<S, X>),
    /// waiting for service call response completion
    Pending,
    /// call queue is empty
    Io,
    /// Upgrade connection
    Upgrade(U::Future),
}

impl<T, S, B, X, U> Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: ResponseError,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: ResponseError,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    /// Create http/1 dispatcher.
    pub(in crate::http) fn new(
        config: Rc<DispatcherConfig<S, X, U>>,
        stream: T,
        peer_addr: Option<net::SocketAddr>,
        on_connect: Option<Box<dyn DataFactory>>,
    ) -> Self {
        let codec = Codec::new(config.timer.clone(), config.keep_alive_enabled());
        // slow request timer
        let timeout = config.client_timer();

        Dispatcher::with_timeout(
            config,
            stream,
            codec,
            BytesMut::with_capacity(READ_HW_BUFFER_SIZE),
            timeout,
            peer_addr,
            on_connect,
        )
    }

    /// Create http/1 dispatcher with slow request timeout.
    pub(in crate::http) fn with_timeout(
        config: Rc<DispatcherConfig<S, X, U>>,
        io: T,
        codec: Codec,
        read_buf: BytesMut,
        timeout: Option<Delay>,
        peer_addr: Option<net::SocketAddr>,
        on_connect: Option<Box<dyn DataFactory>>,
    ) -> Self {
        let keepalive = config.keep_alive_enabled();
        let mut flags = if keepalive {
            Flags::KEEPALIVE | Flags::READ_EOF
        } else {
            Flags::READ_EOF
        };
        if config.keep_alive_timer_enabled() {
            flags |= Flags::HAS_KEEPALIVE;
        }

        // keep-alive timer
        let (ka_expire, ka_timer) = if let Some(delay) = timeout {
            (delay.deadline(), Some(delay))
        } else if let Some(delay) = config.keep_alive_timer() {
            (delay.deadline(), Some(delay))
        } else {
            (config.now(), None)
        };

        Dispatcher {
            call: CallState::Io,
            upgrade: None,
            inner: InnerDispatcher {
                write_buf: BytesMut::with_capacity(WRITE_HW_BUFFER_SIZE),
                req_payload: None,
                res_payload: None,
                error: None,
                io: Some(io),
                config,
                codec,
                read_buf,
                flags,
                peer_addr,
                on_connect,
                ka_expire,
                ka_timer,
            },
        }
    }
}

impl<T, S, B, X, U> Future for Dispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: ResponseError,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: ResponseError,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    #[allow(clippy::cognitive_complexity)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        // handle upgrade request
        if this.inner.flags.contains(Flags::UPGRADE) {
            return this.upgrade.as_pin_mut().unwrap().poll(cx).map_err(|e| {
                error!("Upgrade handler error: {}", e);
                DispatchError::Upgrade
            });
        }

        // shutdown process
        if this.inner.flags.contains(Flags::SHUTDOWN) {
            return this.inner.poll_shutdown(cx);
        }

        // process incoming bytes stream
        let mut not_completed = !this.inner.poll_read(cx);
        this.inner.decode_payload();

        loop {
            // process incoming bytes stream, but only if
            // previous iteration didnt read whole buffer
            if not_completed {
                not_completed = !this.inner.poll_read(cx);
            }

            let st = match this.call.project() {
                CallStateProject::Service { mut fut } => {
                    loop {
                        // we have to loop because of read back-pressure,
                        // check Poll::Pending processing
                        match fut.poll(cx) {
                            Poll::Ready(result) => match result {
                                Ok(res) => {
                                    break this.inner.process_response(res.into())?
                                }
                                Err(e) => {
                                    let res: Response = e.into();
                                    break this.inner.process_response(
                                        res.map_body(|_, body| body.into_body()),
                                    )?;
                                }
                            },
                            Poll::Pending => {
                                // if read back-pressure is enabled, we might need
                                // to read more data (ie service future can wait for payload data)
                                if this.inner.req_payload.is_some() && not_completed {
                                    // read more from io stream
                                    not_completed = !this.inner.poll_read(cx);

                                    // more payload chunks has been decoded
                                    if this.inner.decode_payload() {
                                        // restore consumed future
                                        this = self.as_mut().project();
                                        fut = {
                                            match this.call.project() {
                                                CallStateProject::Service { fut } => fut,
                                                _ => panic!(),
                                            }
                                        };
                                        continue;
                                    }
                                }
                                break CallProcess::Pending;
                            }
                        }
                    }
                }
                // handle EXPECT call
                CallStateProject::Expect { fut } => match fut.poll(cx) {
                    Poll::Ready(result) => match result {
                        Ok(req) => {
                            this.inner
                                .write_buf
                                .extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
                            CallProcess::Next(CallState::Service {
                                fut: this.inner.config.service.call(req),
                            })
                        }
                        Err(e) => {
                            let res: Response = e.into();
                            this.inner.process_response(
                                res.map_body(|_, body| body.into_body()),
                            )?
                        }
                    },
                    Poll::Pending => {
                        // expect service call must resolve before
                        // we can do any more io processing.
                        //
                        // TODO: check keep-alive timer interaction
                        return Poll::Pending;
                    }
                },
                CallStateProject::Io => CallProcess::Io,
            };

            let idle = match st {
                CallProcess::Next(st) => {
                    // we have next call state, just proceed with it
                    this = self.as_mut().project();
                    this.call.set(st);
                    continue;
                }
                CallProcess::Pending => {
                    // service response is in process,
                    // we just flush output and that is it
                    this.inner.poll_write(cx)?;
                    false
                }
                CallProcess::Io => {
                    // service call queue is empty, we can process next request
                    let write = if !this.inner.flags.contains(Flags::STARTED) {
                        PollWrite::AllowNext
                    } else {
                        this.inner.decode_payload();
                        this.inner.poll_write(cx)?
                    };
                    match write {
                        PollWrite::AllowNext => {
                            match this.inner.process_messages(CallProcess::Io)? {
                                CallProcess::Next(st) => {
                                    this = self.as_mut().project();
                                    this.call.set(st);
                                    continue;
                                }
                                CallProcess::Upgrade(fut) => {
                                    this.upgrade.set(Some(fut));
                                    return self.poll(cx);
                                }
                                CallProcess::Io => true,
                                CallProcess::Pending => unreachable!(),
                            }
                        }
                        PollWrite::Pending => this.inner.res_payload.is_none(),
                        PollWrite::PendingResponse => {
                            this.inner.flags.contains(Flags::DISCONNECT)
                        }
                    }
                }
                CallProcess::Upgrade(fut) => {
                    this.upgrade.set(Some(fut));
                    return self.poll(cx);
                }
            };

            // socket is closed and we are not processing any service responses
            if this
                .inner
                .flags
                .intersects(Flags::DISCONNECT | Flags::STOP_READING)
                && idle
            {
                trace!("Shutdown connection (no more work) {:?}", this.inner.flags);
                this.inner.flags.insert(Flags::SHUTDOWN);
            }
            // we dont have any parsed requests and output buffer is flushed
            else if idle && this.inner.write_buf.is_empty() {
                if let Some(err) = this.inner.error.take() {
                    trace!("Dispatcher error {:?}", err);
                    return Poll::Ready(Err(err));
                }

                // disconnect if keep-alive is not enabled
                if this.inner.flags.contains(Flags::STARTED)
                    && !this.inner.flags.contains(Flags::KEEPALIVE)
                {
                    trace!("Shutdown, keep-alive is not enabled");
                    this.inner.flags.insert(Flags::SHUTDOWN);
                }
            }

            // disconnect if shutdown
            return if this.inner.flags.contains(Flags::SHUTDOWN) {
                this.inner.poll_shutdown(cx)
            } else {
                if this.inner.poll_flush(cx)? {
                    // some data has been written to io stream
                    this = self.as_mut().project();
                    continue;
                }

                // keep-alive book-keeping
                if this.inner.ka_timer.is_some() && this.inner.poll_keepalive(cx, idle) {
                    this.inner.poll_shutdown(cx)
                } else {
                    Poll::Pending
                }
            };
        }
    }
}

impl<T, S, B, X, U> InnerDispatcher<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: ResponseError,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: ResponseError,
    U: Service<Request = (Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    /// shutdown process
    fn poll_shutdown(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), DispatchError>> {
        // we can not do anything here
        if self.flags.contains(Flags::DISCONNECT) {
            return Poll::Ready(Ok(()));
        }

        if !self.flags.contains(Flags::SHUTDOWN_IO) {
            self.poll_flush(cx)?;

            if self.write_buf.is_empty() {
                ready!(Pin::new(self.io.as_mut().unwrap()).poll_shutdown(cx)?);
                self.flags.insert(Flags::SHUTDOWN_IO);
            }
        }

        // read until 0 or err
        let mut buf = [0u8; 512];
        while let Poll::Ready(res) =
            Pin::new(self.io.as_mut().unwrap()).poll_read(cx, &mut buf)
        {
            match res {
                Err(_) | Ok(0) => return Poll::Ready(Ok(())),
                _ => (),
            }
        }

        // shutdown timeout
        if self.ka_timer.is_none() {
            if self.flags.contains(Flags::SHUTDOWN_TM) {
                // shutdown timeout is not enabled
                Poll::Pending
            } else {
                self.flags.insert(Flags::SHUTDOWN_TM);
                if let Some(interval) = self.config.client_disconnect_timer() {
                    trace!("Start shutdown timer for {:?}", interval);
                    self.ka_timer = Some(delay_until(interval));
                    let _ = Pin::new(&mut self.ka_timer.as_mut().unwrap()).poll(cx);
                }
                Poll::Pending
            }
        } else {
            let mut timer = self.ka_timer.as_mut().unwrap();

            // configure timer
            if !self.flags.contains(Flags::SHUTDOWN_TM) {
                if let Some(interval) = self.config.client_disconnect_timer() {
                    self.flags.insert(Flags::SHUTDOWN_TM);
                    timer.reset(interval);
                } else {
                    let _ = self.ka_timer.take();
                    return Poll::Pending;
                }
            }

            match Pin::new(&mut timer).poll(cx) {
                Poll::Ready(_) => {
                    // if we get timeout during shutdown, drop connection
                    Poll::Ready(Err(DispatchError::DisconnectTimeout))
                }
                _ => Poll::Pending,
            }
        }
    }

    /// Flush stream
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Result<bool, DispatchError> {
        let len = self.write_buf.len();
        if len == 0 {
            return Ok(false);
        }

        let mut written = 0;
        let mut io = self.io.as_mut().unwrap();

        while written < len {
            match Pin::new(&mut io).poll_write(cx, &self.write_buf[written..]) {
                Poll::Ready(Ok(n)) => {
                    if n == 0 {
                        trace!("Disconnected during flush, written {}", written);
                        return Err(DispatchError::Io(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to write frame to transport",
                        )));
                    } else {
                        written += n
                    }
                }
                Poll::Pending => break,
                Poll::Ready(Err(e)) => {
                    trace!("Error during flush: {}", e);
                    return Err(DispatchError::Io(e));
                }
            }
        }
        if written == len {
            // flushed whole buffer, we dont need to reallocate
            unsafe { self.write_buf.set_len(0) }
        } else {
            self.write_buf.advance(written);
        }
        Ok(written != 0)
    }

    fn send_response(
        &mut self,
        msg: Response<()>,
        body: ResponseBody<B>,
    ) -> Result<bool, DispatchError> {
        trace!("Sending response: {:?} body: {:?}", msg, body.size());
        // we dont need to process responses if socket is disconnected
        // but we still want to handle requests with app service
        // so we skip response processing for disconnected connection
        if !self.flags.contains(Flags::DISCONNECT) {
            self.codec
                .encode(Message::Item((msg, body.size())), &mut self.write_buf)
                .map_err(|err| {
                    if let Some(mut payload) = self.req_payload.take() {
                        payload.set_error(PayloadError::Incomplete(None));
                    }
                    DispatchError::Io(err)
                })?;

            self.flags.set(Flags::KEEPALIVE, self.codec.keepalive());

            match body.size() {
                BodySize::None | BodySize::Empty => {
                    // update keep-alive timer
                    if self.flags.contains(Flags::HAS_KEEPALIVE) {
                        if let Some(expire) = self.config.keep_alive_expire() {
                            self.ka_expire = expire;
                        }
                    }
                    Ok(true)
                }
                _ => {
                    self.res_payload = Some(body);
                    Ok(false)
                }
            }
        } else {
            Ok(false)
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>) -> Result<PollWrite, DispatchError> {
        while let Some(ref mut stream) = self.res_payload {
            let len = self.write_buf.len();

            if len < BUFFER_SIZE {
                // increase write buffer
                let remaining = self.write_buf.capacity() - len;
                if remaining < WRITE_LW_BUFFER_SIZE {
                    self.write_buf.reserve(BUFFER_SIZE - remaining);
                }

                match stream.poll_next_chunk(cx) {
                    Poll::Ready(Some(Ok(item))) => {
                        trace!("Got response chunk: {:?}", item.len());
                        self.codec
                            .encode(Message::Chunk(Some(item)), &mut self.write_buf)?;
                    }
                    Poll::Ready(None) => {
                        trace!("Response payload eof");
                        self.codec
                            .encode(Message::Chunk(None), &mut self.write_buf)?;
                        self.res_payload = None;

                        // update keep-alive timer
                        if self.flags.contains(Flags::HAS_KEEPALIVE) {
                            if let Some(expire) = self.config.keep_alive_expire() {
                                self.ka_expire = expire;
                            }
                        }
                        break;
                    }
                    Poll::Ready(Some(Err(e))) => {
                        trace!("Error during response body poll: {:?}", e);
                        return Err(DispatchError::Unknown);
                    }
                    Poll::Pending => {
                        // response payload stream is not ready we can only flush
                        return Ok(PollWrite::PendingResponse);
                    }
                }
            } else {
                // write buffer is full, we need to flush
                return Ok(PollWrite::PendingResponse);
            }
        }

        // we have enought space in write bffer
        if self.write_buf.len() < BUFFER_SIZE {
            Ok(PollWrite::AllowNext)
        } else {
            Ok(PollWrite::Pending)
        }
    }

    /// Read data from io stream
    fn poll_read(&mut self, cx: &mut Context<'_>) -> bool {
        let mut completed = false;

        // read socket data into a buf
        if !self
            .flags
            .intersects(Flags::DISCONNECT | Flags::STOP_READING)
        {
            // drain until request payload is consumed and requires more data (backpressure off)
            if !self
                .req_payload
                .as_ref()
                .map(|info| info.need_read(cx) == PayloadStatus::Read)
                .unwrap_or(true)
            {
                return false;
            }

            // read data from socket
            let io = self.io.as_mut().unwrap();
            let buf = &mut self.read_buf;

            // increase read buffer size
            let remaining = buf.capacity() - buf.len();
            if remaining < READ_LW_BUFFER_SIZE {
                buf.reserve(BUFFER_SIZE);
            }

            while buf.len() < BUFFER_SIZE {
                match Pin::new(&mut *io).poll_read_buf(cx, buf) {
                    Poll::Pending => {
                        completed = true;
                        break;
                    }
                    Poll::Ready(Ok(n)) => {
                        if n == 0 {
                            trace!(
                                "Disconnected during read, buffer size {}",
                                buf.len()
                            );
                            self.flags.insert(Flags::DISCONNECT);
                            break;
                        }
                        self.flags.remove(Flags::READ_EOF);
                    }
                    Poll::Ready(Err(e)) => {
                        trace!("Error during read: {:?}", e);
                        self.flags.insert(Flags::DISCONNECT);
                        self.error = Some(DispatchError::Io(e));
                        break;
                    }
                }
            }
        }

        completed
    }

    fn internal_error(&mut self, msg: &'static str) -> DispatcherMessage {
        error!("{}", msg);
        self.flags.insert(Flags::DISCONNECT | Flags::READ_EOF);
        self.error = Some(DispatchError::InternalError);
        DispatcherMessage::Error(Response::InternalServerError().finish().drop_body())
    }

    fn decode_error(&mut self, e: ParseError) -> DispatcherMessage {
        // error during request decoding
        if let Some(mut payload) = self.req_payload.take() {
            payload.set_error(PayloadError::EncodingCorrupted);
        }

        // Malformed requests should be responded with 400
        self.flags.insert(Flags::STOP_READING);
        self.read_buf.clear();
        self.error = Some(e.into());
        DispatcherMessage::Error(Response::BadRequest().finish().drop_body())
    }

    fn decode_payload(&mut self) -> bool {
        if self.flags.contains(Flags::READ_EOF)
            || self.req_payload.is_none()
            || self.read_buf.is_empty()
        {
            return false;
        }

        let mut updated = false;
        loop {
            match self.codec.decode(&mut self.read_buf) {
                Ok(Some(msg)) => match msg {
                    Message::Chunk(chunk) => {
                        updated = true;
                        if let Some(ref mut payload) = self.req_payload {
                            if let Some(chunk) = chunk {
                                payload.feed_data(chunk);
                            } else {
                                payload.feed_eof();
                                self.req_payload = None;
                            }
                        } else {
                            self.internal_error(
                                "Internal server error: unexpected payload chunk",
                            );
                            break;
                        }
                    }
                    Message::Item(_) => {
                        self.internal_error(
                            "Internal server error: unexpected http message",
                        );
                        break;
                    }
                },
                Ok(None) => {
                    self.flags.insert(Flags::READ_EOF);
                    break;
                }
                Err(e) => {
                    self.decode_error(e);
                    break;
                }
            }
        }

        updated
    }

    fn decode_message(&mut self) -> Option<DispatcherMessage> {
        if self.flags.contains(Flags::READ_EOF) || self.read_buf.is_empty() {
            return None;
        }

        match self.codec.decode(&mut self.read_buf) {
            Ok(Some(msg)) => {
                self.flags.insert(Flags::STARTED);

                match msg {
                    Message::Item(mut req) => {
                        let pl = self.codec.message_type();
                        req.head_mut().peer_addr = self.peer_addr;

                        // set on_connect data
                        if let Some(ref on_connect) = self.on_connect {
                            on_connect.set(&mut req.extensions_mut());
                        }

                        // handle upgrade request
                        if pl == MessageType::Stream && self.config.upgrade.is_some() {
                            self.flags.insert(Flags::STOP_READING);
                            Some(DispatcherMessage::Upgrade(req))
                        } else {
                            // handle request with payload
                            if pl == MessageType::Payload || pl == MessageType::Stream {
                                let (ps, pl) = Payload::create(false);
                                let (req1, _) =
                                    req.replace_payload(crate::http::Payload::H1(pl));
                                req = req1;
                                self.req_payload = Some(ps);
                            }

                            Some(DispatcherMessage::Request(req))
                        }
                    }
                    Message::Chunk(_) => Some(self.internal_error(
                        "Internal server error: unexpected payload chunk",
                    )),
                }
            }
            Ok(None) => {
                self.flags.insert(Flags::READ_EOF);
                None
            }
            Err(e) => Some(self.decode_error(e)),
        }
    }

    /// keep-alive timer
    fn poll_keepalive(&mut self, cx: &mut Context<'_>, idle: bool) -> bool {
        let ka_timer = self.ka_timer.as_mut().unwrap();
        // do nothing for disconnected or upgrade socket or if keep-alive timer is disabled
        if self.flags.contains(Flags::DISCONNECT) {
            return false;
        }
        // slow request timeout
        else if !self.flags.contains(Flags::STARTED) {
            if Pin::new(ka_timer).poll(cx).is_ready() {
                // timeout on first request (slow request) return 408
                trace!("Slow request timeout");
                let _ = self.send_response(
                    Response::RequestTimeout().finish().drop_body(),
                    ResponseBody::Other(Body::Empty),
                );
                self.flags.insert(Flags::STARTED | Flags::SHUTDOWN);
                return true;
            }
        }
        // normal keep-alive, but only if we are not processing any requests
        else if idle {
            // keep-alive timer
            if Pin::new(&mut *ka_timer).poll(cx).is_ready() {
                if ka_timer.deadline() >= self.ka_expire {
                    // check for any outstanding tasks
                    if self.write_buf.is_empty() {
                        trace!("Keep-alive timeout, close connection");
                        self.flags.insert(Flags::SHUTDOWN);
                        return true;
                    } else if let Some(dl) = self.config.keep_alive_expire() {
                        // extend keep-alive timer
                        ka_timer.reset(dl);
                    }
                } else {
                    ka_timer.reset(self.ka_expire);
                }
                let _ = Pin::new(ka_timer).poll(cx);
            }
        }
        false
    }

    fn process_response(
        &mut self,
        res: Response<B>,
    ) -> Result<CallProcess<S, X, U>, DispatchError> {
        let (res, body) = res.replace_body(());
        if self.send_response(res, body)? {
            // response does not have body, so we can process next request
            self.process_messages(CallProcess::Next(CallState::Io))
        } else {
            Ok(CallProcess::Next(CallState::Io))
        }
    }

    fn process_messages(
        &mut self,
        io: CallProcess<S, X, U>,
    ) -> Result<CallProcess<S, X, U>, DispatchError> {
        while let Some(msg) = self.decode_message() {
            return match msg {
                DispatcherMessage::Request(req) => {
                    if self.req_payload.is_some() {
                        self.decode_payload();
                    }

                    // Handle `EXPECT: 100-Continue` header
                    Ok(CallProcess::Next(if req.head().expect() {
                        CallState::Expect {
                            fut: self.config.expect.call(req),
                        }
                    } else {
                        CallState::Service {
                            fut: self.config.service.call(req),
                        }
                    }))
                }
                // switch to upgrade handler
                DispatcherMessage::Upgrade(req) => {
                    self.flags.insert(Flags::UPGRADE);
                    let mut parts = FramedParts::with_read_buf(
                        self.io.take().unwrap(),
                        mem::take(&mut self.codec),
                        mem::take(&mut self.read_buf),
                    );
                    parts.write_buf = mem::take(&mut self.write_buf);
                    let framed = Framed::from_parts(parts);

                    Ok(CallProcess::Upgrade(
                        self.config.upgrade.as_ref().unwrap().call((req, framed)),
                    ))
                }
                DispatcherMessage::Error(res) => {
                    if self.send_response(res, ResponseBody::Other(Body::Empty))? {
                        // response does not have body, so we can process next request
                        continue;
                    } else {
                        return Ok(io);
                    }
                }
            };
        }
        Ok(io)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::future::{lazy, ok, Future, FutureExt};
    use futures::StreamExt;
    use rand::Rng;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::http::config::{DispatcherConfig, ServiceConfig};
    use crate::http::h1::{ClientCodec, ExpectHandler, UpgradeHandler};
    use crate::http::{body, Request, ResponseHead, StatusCode};
    use crate::rt::time::delay_for;
    use crate::service::IntoService;
    use crate::testing::Io;

    /// Create http/1 dispatcher.
    pub(crate) fn h1<F, S, B>(
        stream: Io,
        service: F,
    ) -> Dispatcher<Io, S, B, ExpectHandler, UpgradeHandler<Io>>
    where
        F: IntoService<S>,
        S: Service<Request = Request>,
        S::Error: ResponseError,
        S::Response: Into<Response<B>>,
        B: MessageBody,
    {
        Dispatcher::new(
            Rc::new(DispatcherConfig::new(
                ServiceConfig::default(),
                service.into_service(),
                ExpectHandler,
                None,
            )),
            stream,
            None,
            None,
        )
    }

    pub(crate) fn spawn_h1<F, S, B>(stream: Io, service: F)
    where
        F: IntoService<S>,
        S: Service<Request = Request> + 'static,
        S::Error: ResponseError,
        S::Response: Into<Response<B>>,
        B: MessageBody + 'static,
    {
        crate::rt::spawn(
            Dispatcher::<Io, S, B, ExpectHandler, UpgradeHandler<Io>>::new(
                Rc::new(DispatcherConfig::new(
                    ServiceConfig::default(),
                    service.into_service(),
                    ExpectHandler,
                    None,
                )),
                stream,
                None,
                None,
            ),
        );
    }

    fn load(decoder: &mut ClientCodec, buf: &mut BytesMut) -> ResponseHead {
        decoder.decode(buf).unwrap().unwrap()
    }

    #[ntex_rt::test]
    async fn test_req_parse_err() {
        let (client, server) = Io::create();
        client.remote_buffer_cap(1024);
        client.write("GET /test HTTP/1\r\n\r\n");

        let mut h1 = h1(server, |_| ok::<_, io::Error>(Response::Ok().finish()));
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());
        assert!(h1.inner.flags.contains(Flags::SHUTDOWN));
        client
            .local_buffer(|buf| assert_eq!(&buf[..26], b"HTTP/1.1 400 Bad Request\r\n"));

        client.close().await;
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_ready());
        assert!(h1.inner.flags.contains(Flags::SHUTDOWN_IO));
    }

    #[ntex_rt::test]
    async fn test_pipeline() {
        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);
        let mut decoder = ClientCodec::default();
        spawn_h1(server, |_| ok::<_, io::Error>(Response::Ok().finish()));

        client.write("GET /test HTTP/1.1\r\n\r\n");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(!client.is_server_dropped());

        client.write("GET /test HTTP/1.1\r\n\r\n");
        client.write("GET /test HTTP/1.1\r\n\r\n");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(decoder.decode(&mut buf).unwrap().is_none());
        assert!(!client.is_server_dropped());

        client.close().await;
        assert!(client.is_server_dropped());
    }

    #[ntex_rt::test]
    async fn test_pipeline_with_payload() {
        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);
        let mut decoder = ClientCodec::default();
        spawn_h1(server, |mut req: Request| async move {
            let mut p = req.take_payload();
            while let Some(_) = p.next().await {}
            Ok::<_, io::Error>(Response::Ok().finish())
        });

        client.write("GET /test HTTP/1.1\r\ncontent-length: 5\r\n\r\n");
        delay_for(Duration::from_millis(50)).await;
        client.write("xxxxx");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(!client.is_server_dropped());

        client.write("GET /test HTTP/1.1\r\n\r\n");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(decoder.decode(&mut buf).unwrap().is_none());
        assert!(!client.is_server_dropped());

        client.close().await;
        assert!(client.is_server_dropped());
    }

    #[ntex_rt::test]
    async fn test_pipeline_with_delay() {
        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);
        let mut decoder = ClientCodec::default();
        spawn_h1(server, |_| async {
            delay_for(Duration::from_millis(100)).await;
            Ok::<_, io::Error>(Response::Ok().finish())
        });

        client.write("GET /test HTTP/1.1\r\n\r\n");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(!client.is_server_dropped());

        client.write("GET /test HTTP/1.1\r\n\r\n");
        client.write("GET /test HTTP/1.1\r\n\r\n");
        delay_for(Duration::from_millis(50)).await;
        client.write("GET /test HTTP/1.1\r\n\r\n");

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());

        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(decoder.decode(&mut buf).unwrap().is_none());
        assert!(!client.is_server_dropped());

        buf.extend(client.read().await.unwrap());
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(decoder.decode(&mut buf).unwrap().is_none());
        assert!(!client.is_server_dropped());

        client.close().await;
        assert!(client.is_server_dropped());
    }

    #[ntex_rt::test]
    /// if socket is disconnected
    /// h1 dispatcher still processes all incoming requests
    /// but it does not write any data to socket
    async fn test_write_disconnected() {
        let num = Arc::new(AtomicUsize::new(0));
        let num2 = num.clone();

        let (client, server) = Io::create();
        spawn_h1(server, move |_| {
            num2.fetch_add(1, Ordering::Relaxed);
            ok::<_, io::Error>(Response::Ok().finish())
        });

        client.remote_buffer_cap(1024);
        client.write("GET /test HTTP/1.1\r\n\r\n");
        client.write("GET /test HTTP/1.1\r\n\r\n");
        client.write("GET /test HTTP/1.1\r\n\r\n");
        client.close().await;
        assert!(client.is_server_dropped());
        assert!(client.read_any().is_empty());

        // all request must be handled
        assert_eq!(num.load(Ordering::Relaxed), 3);
    }

    #[ntex_rt::test]
    async fn test_read_large_message() {
        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);

        let mut h1 = h1(server, |_| ok::<_, io::Error>(Response::Ok().finish()));
        let mut decoder = ClientCodec::default();

        let data = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(70_000)
            .map(char::from)
            .collect::<String>();
        client.write("GET /test HTTP/1.1\r\nContent-Length: ");
        client.write(data);

        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());
        assert!(h1.inner.flags.contains(Flags::SHUTDOWN));

        let mut buf = client.read().await.unwrap();
        assert_eq!(load(&mut decoder, &mut buf).status, StatusCode::BAD_REQUEST);
    }

    #[ntex_rt::test]
    async fn test_read_backpressure() {
        let mark = Arc::new(AtomicBool::new(false));
        let mark2 = mark.clone();

        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);
        spawn_h1(server, move |mut req: Request| {
            let m = mark2.clone();
            async move {
                // read one chunk
                let mut pl = req.take_payload();
                let _ = pl.next().await.unwrap().unwrap();
                m.store(true, Ordering::Relaxed);
                // sleep
                delay_for(Duration::from_secs(999_999)).await;
                Ok::<_, io::Error>(Response::Ok().finish())
            }
        });

        client.write("GET /test HTTP/1.1\r\nContent-Length: 1048576\r\n\r\n");
        delay_for(Duration::from_millis(50)).await;

        // buf must be consumed
        assert_eq!(client.remote_buffer(|buf| buf.len()), 0);

        // io should be drained only by no more than MAX_BUFFER_SIZE
        let random_bytes: Vec<u8> =
            (0..1_048_576).map(|_| rand::random::<u8>()).collect();
        client.write(random_bytes);

        delay_for(Duration::from_millis(50)).await;
        assert!(client.remote_buffer(|buf| buf.len()) > 1_048_576 - BUFFER_SIZE * 3);
        assert!(mark.load(Ordering::Relaxed));
    }

    #[ntex_rt::test]
    async fn test_write_backpressure() {
        let num = Arc::new(AtomicUsize::new(0));
        let num2 = num.clone();

        struct Stream(Arc<AtomicUsize>);

        impl body::MessageBody for Stream {
            fn size(&self) -> body::BodySize {
                body::BodySize::Stream
            }
            fn poll_next_chunk(
                &mut self,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Bytes, Box<dyn std::error::Error>>>> {
                let data = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(65_536)
                    .map(char::from)
                    .collect::<String>();
                self.0.fetch_add(data.len(), Ordering::Relaxed);

                Poll::Ready(Some(Ok(Bytes::from(data))))
            }
        }

        let (client, server) = Io::create();
        let mut h1 = h1(server, move |_| {
            let n = num2.clone();
            async move { Ok::<_, io::Error>(Response::Ok().message_body(Stream(n.clone()))) }
            .boxed_local()
        });

        // do not allow to write to socket
        client.remote_buffer_cap(0);
        client.write("GET /test HTTP/1.1\r\n\r\n");
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());

        // buf must be consumed
        assert_eq!(client.remote_buffer(|buf| buf.len()), 0);

        // amount of generated data
        assert_eq!(num.load(Ordering::Relaxed), 65_536);

        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());
        assert_eq!(num.load(Ordering::Relaxed), 65_536);
        // response message + chunking encoding
        assert_eq!(h1.inner.write_buf.len(), 65629);

        client.remote_buffer_cap(65536);
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());
        assert_eq!(num.load(Ordering::Relaxed), 65_536 * 2);
    }

    #[ntex_rt::test]
    async fn test_disconnect_during_response_body_pending() {
        struct Stream(bool);

        impl body::MessageBody for Stream {
            fn size(&self) -> body::BodySize {
                body::BodySize::Sized(2048)
            }
            fn poll_next_chunk(
                &mut self,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Bytes, Box<dyn std::error::Error>>>> {
                if self.0 {
                    Poll::Pending
                } else {
                    self.0 = true;
                    let data = rand::thread_rng()
                        .sample_iter(&rand::distributions::Alphanumeric)
                        .take(1024)
                        .map(char::from)
                        .collect::<String>();
                    Poll::Ready(Some(Ok(Bytes::from(data))))
                }
            }
        }

        let (client, server) = Io::create();
        client.remote_buffer_cap(4096);
        let mut h1 = h1(server, |_| {
            ok::<_, io::Error>(Response::Ok().message_body(Stream(false)))
        });

        client.write("GET /test HTTP/1.1\r\n\r\n");
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());

        // buf must be consumed
        assert_eq!(client.remote_buffer(|buf| buf.len()), 0);

        let mut decoder = ClientCodec::default();
        let mut buf = client.read().await.unwrap();
        assert!(load(&mut decoder, &mut buf).status.is_success());
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_pending());

        client.close().await;
        assert!(lazy(|cx| Pin::new(&mut h1).poll(cx)).await.is_ready());
    }
}
