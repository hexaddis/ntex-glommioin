mod dispatcher;
mod error;
mod handshake;
mod service;

pub use self::error::ServiceError;
pub use self::handshake::{Handshake, HandshakeResult};
pub use self::service::{Builder, FactoryBuilder};

#[doc(hidden)]
pub type Connect<T, U> = Handshake<T, U>;
#[doc(hidden)]
pub type ConnectResult<Io, St, Codec, Out> = HandshakeResult<Io, St, Codec, Out>;
