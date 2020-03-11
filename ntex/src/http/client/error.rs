//! Http client errors
use std::error::Error;
use std::io;

use actix_connect::resolver::ResolveError;
use derive_more::{Display, From};
use serde_json::error::Error as JsonError;

#[cfg(feature = "openssl")]
use actix_connect::ssl::openssl::{HandshakeError, SslError};

use crate::http::error::{HttpError, ParseError, PayloadError, ResponseError};
use crate::http::header::HeaderValue;
use crate::http::StatusCode;
use crate::ws::ProtocolError;

/// Websocket client error
#[derive(Debug, Display, From)]
pub enum WsClientError {
    /// Invalid response status
    #[display(fmt = "Invalid response status")]
    InvalidResponseStatus(StatusCode),
    /// Invalid upgrade header
    #[display(fmt = "Invalid upgrade header")]
    InvalidUpgradeHeader,
    /// Invalid connection header
    #[display(fmt = "Invalid connection header")]
    InvalidConnectionHeader(HeaderValue),
    /// Missing CONNECTION header
    #[display(fmt = "Missing CONNECTION header")]
    MissingConnectionHeader,
    /// Missing SEC-WEBSOCKET-ACCEPT header
    #[display(fmt = "Missing SEC-WEBSOCKET-ACCEPT header")]
    MissingWebSocketAcceptHeader,
    /// Invalid challenge response
    #[display(fmt = "Invalid challenge response")]
    InvalidChallengeResponse(String, HeaderValue),
    /// Protocol error
    #[display(fmt = "{}", _0)]
    Protocol(ProtocolError),
    /// Send request error
    #[display(fmt = "{}", _0)]
    SendRequest(SendRequestError),
}

impl From<InvalidUrl> for WsClientError {
    fn from(err: InvalidUrl) -> Self {
        WsClientError::SendRequest(err.into())
    }
}

impl From<HttpError> for WsClientError {
    fn from(err: HttpError) -> Self {
        WsClientError::SendRequest(err.into())
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Display, From)]
pub enum JsonPayloadError {
    /// Content type error
    #[display(fmt = "Content type error")]
    ContentType,
    /// Deserialize error
    #[display(fmt = "Json deserialize error: {}", _0)]
    Deserialize(JsonError),
    /// Payload error
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),
}

/// Return `InternalServerError` for `JsonPayloadError`
impl ResponseError for JsonPayloadError {}

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Display, From)]
pub enum ConnectError {
    /// SSL feature is not enabled
    #[display(fmt = "SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "openssl")]
    #[display(fmt = "{}", _0)]
    SslError(SslError),

    /// SSL Handshake error
    #[cfg(feature = "openssl")]
    #[display(fmt = "{}", _0)]
    SslHandshakeError(String),

    /// Failed to resolve the hostname
    #[display(fmt = "Failed resolving hostname: {}", _0)]
    Resolver(ResolveError),

    /// No dns records
    #[display(fmt = "No dns records found for the input")]
    NoRecords,

    /// Http2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// Connecting took too long
    #[display(fmt = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[display(fmt = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Unresolved host name
    #[display(fmt = "Connector received `Connect` method with unresolved host")]
    Unresolverd,

    /// Connection io error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

impl From<actix_connect::ConnectError> for ConnectError {
    fn from(err: actix_connect::ConnectError) -> ConnectError {
        match err {
            actix_connect::ConnectError::Resolver(e) => ConnectError::Resolver(e),
            actix_connect::ConnectError::NoRecords => ConnectError::NoRecords,
            actix_connect::ConnectError::InvalidInput => panic!(),
            actix_connect::ConnectError::Unresolverd => ConnectError::Unresolverd,
            actix_connect::ConnectError::Io(e) => ConnectError::Io(e),
        }
    }
}

#[cfg(feature = "openssl")]
impl<T: std::fmt::Debug> From<HandshakeError<T>> for ConnectError {
    fn from(err: HandshakeError<T>) -> ConnectError {
        ConnectError::SslHandshakeError(format!("{:?}", err))
    }
}

#[derive(Debug, Display, From)]
pub enum InvalidUrl {
    #[display(fmt = "Missing url scheme")]
    MissingScheme,
    #[display(fmt = "Unknown url scheme")]
    UnknownScheme,
    #[display(fmt = "Missing host name")]
    MissingHost,
    #[display(fmt = "Url parse error: {}", _0)]
    Http(HttpError),
}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Display, From)]
pub enum SendRequestError {
    /// Invalid URL
    #[display(fmt = "Invalid URL: {}", _0)]
    Url(InvalidUrl),
    /// Failed to connect to host
    #[display(fmt = "Failed to connect to host: {}", _0)]
    Connect(ConnectError),
    /// Error sending request
    Send(io::Error),
    /// Error parsing response
    Response(ParseError),
    /// Http error
    #[display(fmt = "{}", _0)]
    Http(HttpError),
    /// Http2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),
    /// Response took too long
    #[display(fmt = "Timeout out while waiting for response")]
    Timeout,
    /// Tunnels are not supported for http2 connection
    #[display(fmt = "Tunnels are not supported for http2 connection")]
    TunnelNotSupported,
    /// Error sending request body
    Body(Box<dyn Error>),
}

/// Convert `SendRequestError` to a server `Response`
impl ResponseError for SendRequestError {
    fn status_code(&self) -> StatusCode {
        match *self {
            SendRequestError::Connect(ConnectError::Timeout) => {
                StatusCode::GATEWAY_TIMEOUT
            }
            SendRequestError::Connect(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// A set of errors that can occur during freezing a request
#[derive(Debug, Display, From)]
pub enum FreezeRequestError {
    /// Invalid URL
    #[display(fmt = "Invalid URL: {}", _0)]
    Url(InvalidUrl),
    /// Http error
    #[display(fmt = "{}", _0)]
    Http(HttpError),
}

impl From<FreezeRequestError> for SendRequestError {
    fn from(e: FreezeRequestError) -> Self {
        match e {
            FreezeRequestError::Url(e) => e.into(),
            FreezeRequestError::Http(e) => e.into(),
        }
    }
}
