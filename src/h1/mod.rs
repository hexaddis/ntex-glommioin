//! HTTP/1 implementation
use actix_net::codec::Framed;
use bytes::Bytes;

mod client;
mod codec;
mod decoder;
mod dispatcher;
mod encoder;
mod service;

pub use self::client::ClientCodec;
pub use self::codec::Codec;
pub use self::decoder::{PayloadDecoder, RequestDecoder};
pub use self::dispatcher::Dispatcher;
pub use self::service::{H1Service, H1ServiceHandler, OneRequest};

use request::Request;

/// H1 service response type
pub enum H1ServiceResult<T> {
    Disconnected,
    Shutdown(T),
    Unhandled(Request, Framed<T, Codec>),
}

#[derive(Debug)]
/// Codec message
pub enum Message<T> {
    /// Http message
    Item(T),
    /// Payload chunk
    Chunk(Option<Bytes>),
}

impl<T> From<T> for Message<T> {
    fn from(item: T) -> Self {
        Message::Item(item)
    }
}

/// Incoming request type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    None,
    Payload,
    Unhandled,
}
