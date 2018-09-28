#[cfg(any(feature = "alpn", feature = "ssl"))]
mod openssl;
#[cfg(any(feature = "alpn", feature = "ssl"))]
pub use self::openssl::{openssl_acceptor_with_flags, OpensslAcceptor};

#[cfg(feature = "tls")]
mod nativetls;

#[cfg(feature = "rust-tls")]
mod rustls;
#[cfg(feature = "rust-tls")]
pub use self::rustls::RustlsAcceptor;
