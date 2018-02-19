use std::{io, mem};
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll};

use actix::prelude::*;

use error::PayloadError;
use server::shared::SharedBytes;
use super::{ClientRequest, ClientResponse};
use super::{Connect, Connection, ClientConnector, ClientConnectorError};
use super::HttpClientWriter;
use super::{HttpResponseParser, HttpResponseParserError};

pub enum SendRequestError {
    Connector(ClientConnectorError),
    ParseError(HttpResponseParserError),
    Io(io::Error),
}

impl From<io::Error> for SendRequestError {
    fn from(err: io::Error) -> SendRequestError {
        SendRequestError::Io(err)
    }
}

enum State {
    New,
    Connect(actix::dev::Request<Unsync, ClientConnector, Connect>),
    Send(Box<Pipeline>),
    None,
}

/// `SendRequest` is a `Future` which represents asynchronous request sending process.
#[must_use = "SendRequest do nothing unless polled"]
pub struct SendRequest {
    req: ClientRequest,
    state: State,
}

impl SendRequest {
    pub(crate) fn new(req: ClientRequest) -> SendRequest {
        SendRequest{
            req: req,
            state: State::New,
        }
    }
}

impl Future for SendRequest {
    type Item = ClientResponse;
    type Error = SendRequestError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let state = mem::replace(&mut self.state, State::None);

            match state {
                State::New =>
                    self.state = State::Connect(
                        ClientConnector::from_registry().send(Connect(self.req.uri().clone()))),
                State::Connect(mut conn) => match conn.poll() {
                    Ok(Async::NotReady) => {
                        self.state = State::Connect(conn);
                        return Ok(Async::NotReady);
                    },
                    Ok(Async::Ready(result)) => match result {
                        Ok(stream) => {
                            let mut pl = Box::new(Pipeline {
                                conn: stream,
                                writer: HttpClientWriter::new(SharedBytes::default()),
                                parser: HttpResponseParser::default(),
                                parser_buf: BytesMut::new(),
                            });
                            pl.writer.start(&mut self.req)?;
                            self.state = State::Send(pl);
                        },
                        Err(err) => return Err(SendRequestError::Connector(err)),
                    },
                    Err(_) =>
                        return Err(SendRequestError::Connector(ClientConnectorError::Disconnected))
                },
                State::Send(mut pl) => {
                    pl.poll_write()?;
                    match pl.parse() {
                        Ok(Async::Ready(mut resp)) => {
                            resp.set_pipeline(pl);
                            return Ok(Async::Ready(resp))
                        },
                        Ok(Async::NotReady) => {
                            self.state = State::Send(pl);
                            return Ok(Async::NotReady)
                        },
                        Err(err) => return Err(SendRequestError::ParseError(err))
                    }
                }
                State::None => unreachable!(),
            }
        }
    }
}


pub(crate) struct Pipeline {
    conn: Connection,
    writer: HttpClientWriter,
    parser: HttpResponseParser,
    parser_buf: BytesMut,
}

impl Pipeline {

    #[inline]
    pub fn parse(&mut self) -> Poll<ClientResponse, HttpResponseParserError> {
        self.parser.parse(&mut self.conn, &mut self.parser_buf)
    }

    #[inline]
    pub fn poll(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        self.parser.parse_payload(&mut self.conn, &mut self.parser_buf)
    }

    #[inline]
    pub fn poll_write(&mut self) -> Poll<(), io::Error> {
        self.writer.poll_completed(&mut self.conn, false)
    }
}
