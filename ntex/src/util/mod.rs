pub mod buffer;
pub mod counter;
mod extensions;
pub mod inflight;
pub mod keepalive;
pub mod sink;
pub mod stream;
pub mod time;
pub mod timeout;
pub mod variant;

pub use self::extensions::Extensions;

pub use ntex_service::util::{lazy, Lazy, Ready};

pub use bytes::{Buf, BufMut, Bytes, BytesMut};
pub use bytestring::ByteString;
pub use either::Either;

pub type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
pub type HashSet<V> = std::collections::HashSet<V, ahash::RandomState>;
