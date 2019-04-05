//! Actix connect - tcp connector service
//!
//! ## Package feature
//!
//! * `ssl` - enables ssl support via `openssl` crate
//! * `rust-tls` - enables ssl support via `rustls` crate

#[macro_use]
extern crate log;

mod connect;
mod connector;
mod error;
mod resolver;
pub mod ssl;

#[cfg(feature = "uri")]
mod uri;

pub use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
pub use trust_dns_resolver::system_conf::read_system_conf;
pub use trust_dns_resolver::{error::ResolveError, AsyncResolver};

pub use self::connect::{Address, Connect, Connection};
pub use self::connector::{TcpConnector, TcpConnectorFactory};
pub use self::error::ConnectError;
pub use self::resolver::{Resolver, ResolverFactory};

use actix_service::{NewService, Service, ServiceExt};
use tokio_tcp::TcpStream;

#[doc(hidden)]
#[deprecated(since = "0.1.2", note = "please use `actix_connect::TcpConnector`")]
pub type Connector<T> = TcpConnector<T>;

#[doc(hidden)]
#[deprecated(
    since = "0.1.2",
    note = "please use `actix_connect::TcpConnectorFactory`"
)]
pub type ConnectorFactory<T> = TcpConnectorFactory<T>;

pub fn start_resolver(cfg: ResolverConfig, opts: ResolverOpts) -> AsyncResolver {
    let (resolver, bg) = AsyncResolver::new(cfg, opts);
    tokio_current_thread::spawn(bg);
    resolver
}

pub fn start_default_resolver() -> AsyncResolver {
    let (cfg, opts) = match read_system_conf() {
        Ok((cfg, opts)) => (cfg, opts),
        Err(e) => {
            log::error!("TRust-DNS can not load system config: {}", e);
            (ResolverConfig::default(), ResolverOpts::default())
        }
    };

    let (resolver, bg) = AsyncResolver::new(cfg, opts);
    tokio_current_thread::spawn(bg);
    resolver
}

/// Create tcp connector service
pub fn new_connector<T: Address>(
    resolver: AsyncResolver,
) -> impl Service<Request = Connect<T>, Response = Connection<T, TcpStream>, Error = ConnectError>
         + Clone {
    Resolver::new(resolver).and_then(TcpConnector::new())
}

/// Create tcp connector service
pub fn new_connector_factory<T: Address>(
    resolver: AsyncResolver,
) -> impl NewService<
    Request = Connect<T>,
    Response = Connection<T, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    ResolverFactory::new(resolver).and_then(TcpConnectorFactory::new())
}

/// Create connector service with default parameters
pub fn default_connector<T: Address>(
) -> impl Service<Request = Connect<T>, Response = Connection<T, TcpStream>, Error = ConnectError>
         + Clone {
    Resolver::new(start_default_resolver()).and_then(TcpConnector::new())
}

/// Create connector service factory with default parameters
pub fn default_connector_factory<T: Address>() -> impl NewService<
    Request = Connect<T>,
    Response = Connection<T, TcpStream>,
    Error = ConnectError,
    InitError = (),
> + Clone {
    ResolverFactory::new(start_default_resolver()).and_then(TcpConnectorFactory::new())
}
