use std::io;
use std::sync::Arc;
use std::task::{Context, Poll};

pub use rust_tls::Session;
pub use tokio_rustls::{client::TlsStream, rustls::ClientConfig};

use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};
use tokio_rustls::{self, TlsConnector};
use trust_dns_resolver::AsyncResolver;
use webpki::DNSNameRef;

use crate::connect::Address;
use crate::rt::net::TcpStream;
use crate::service::{Service, ServiceFactory};

use super::{Connect, ConnectError, Connector};

/// Rustls connector factory
pub struct RustlsConnector<T> {
    connector: Connector<T>,
    config: Arc<ClientConfig>,
}

impl<T> RustlsConnector<T> {
    pub fn new(config: Arc<ClientConfig>) -> Self {
        RustlsConnector {
            config,
            connector: Connector::default(),
        }
    }

    /// Construct new connect service with custom dns resolver
    pub fn with_resolver(config: Arc<ClientConfig>, resolver: AsyncResolver) -> Self {
        RustlsConnector {
            config,
            connector: Connector::new(resolver),
        }
    }
}

impl<T> Clone for RustlsConnector<T> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            connector: self.connector.clone(),
        }
    }
}

impl<T: Address + 'static> ServiceFactory for RustlsConnector<T> {
    type Request = Connect<T>;
    type Response = TlsStream<TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = RustlsConnector<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(self.clone())
    }
}

impl<T: Address + 'static> Service for RustlsConnector<T> {
    type Request = Connect<T>;
    type Response = TlsStream<TcpStream>;
    type Error = ConnectError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Connect<T>) -> Self::Future {
        let host = req.host().to_string();
        let conn = self.connector.call(req);
        let config = self.config.clone();

        async move {
            let io = conn.await?;
            trace!("SSL Handshake start for: {:?}", host);

            let host = DNSNameRef::try_from_ascii_str(&host)
                .expect("rustls currently only handles hostname-based connections. See https://github.com/briansmith/webpki/issues/54");

            match TlsConnector::from(config).connect(host, io).await {
                Ok(io) => {
                    trace!("SSL Handshake success: {:?}", host);
                    Ok(io)
                }
                Err(e) => {
                    trace!("SSL Handshake error: {:?}", e);
                    Err(io::Error::new(io::ErrorKind::Other, format!("{}", e))
                        .into())
                }
            }
        }
        .boxed_local()
    }
}
