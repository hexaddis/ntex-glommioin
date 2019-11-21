use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_connect::{
    default_connector, Connect as TcpConnect, Connection as TcpConnection,
};
use actix_service::{apply_fn, Service};
use actix_utils::timeout::{TimeoutError, TimeoutService};
use futures::future::Ready;
use http::Uri;
use tokio_net::tcp::TcpStream;

use super::connection::Connection;
use super::error::ConnectError;
use super::pool::{ConnectionPool, Protocol};
use super::Connect;

#[cfg(feature = "openssl")]
use open_ssl::ssl::SslConnector as OpensslConnector;

#[cfg(feature = "rustls")]
use rust_tls::ClientConfig;
#[cfg(feature = "rustls")]
use std::sync::Arc;

#[cfg(any(feature = "openssl", feature = "rustls"))]
enum SslConnector {
    #[cfg(feature = "openssl")]
    Openssl(OpensslConnector),
    #[cfg(feature = "rustls")]
    Rustls(Arc<ClientConfig>),
}
#[cfg(not(any(feature = "openssl", feature = "rustls")))]
type SslConnector = ();

/// Manages http client network connectivity
/// The `Connector` type uses a builder-like combinator pattern for service
/// construction that finishes by calling the `.finish()` method.
///
/// ```rust,ignore
/// use std::time::Duration;
/// use actix_http::client::Connector;
///
/// let connector = Connector::new()
///      .timeout(Duration::from_secs(5))
///      .finish();
/// ```
pub struct Connector<T, U> {
    connector: T,
    timeout: Duration,
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    disconnect_timeout: Duration,
    limit: usize,
    #[allow(dead_code)]
    ssl: SslConnector,
    _t: PhantomData<U>,
}

trait Io: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> Io for T {}

impl Connector<(), ()> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Connector<
        impl Service<
                Request = TcpConnect<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = actix_connect::ConnectError,
            > + Clone,
        TcpStream,
    > {
        let ssl = {
            #[cfg(feature = "openssl")]
            {
                use open_ssl::ssl::SslMethod;

                let mut ssl = OpensslConnector::builder(SslMethod::tls()).unwrap();
                let _ = ssl
                    .set_alpn_protos(b"\x02h2\x08http/1.1")
                    .map_err(|e| error!("Can not set alpn protocol: {:?}", e));
                SslConnector::Openssl(ssl.build())
            }
            #[cfg(all(not(feature = "openssl"), feature = "rustls"))]
            {
                let protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                let mut config = ClientConfig::new();
                config.set_protocols(&protos);
                config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
                SslConnector::Rustls(Arc::new(config))
            }
            #[cfg(not(any(feature = "openssl", feature = "rustls")))]
            {}
        };

        Connector {
            ssl,
            connector: default_connector(),
            timeout: Duration::from_secs(1),
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            disconnect_timeout: Duration::from_millis(3000),
            limit: 100,
            _t: PhantomData,
        }
    }
}

impl<T, U> Connector<T, U> {
    /// Use custom connector.
    pub fn connector<T1, U1>(self, connector: T1) -> Connector<T1, U1>
    where
        U1: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
        T1: Service<
                Request = TcpConnect<Uri>,
                Response = TcpConnection<Uri, U1>,
                Error = actix_connect::ConnectError,
            > + Clone,
        T1::Future: Unpin,
    {
        Connector {
            connector,
            timeout: self.timeout,
            conn_lifetime: self.conn_lifetime,
            conn_keep_alive: self.conn_keep_alive,
            disconnect_timeout: self.disconnect_timeout,
            limit: self.limit,
            ssl: self.ssl,
            _t: PhantomData,
        }
    }
}

impl<T, U> Connector<T, U>
where
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
    T: Service<
            Request = TcpConnect<Uri>,
            Response = TcpConnection<Uri, U>,
            Error = actix_connect::ConnectError,
        > + Clone
        + 'static,
{
    /// Connection timeout, i.e. max time to connect to remote host including dns name resolution.
    /// Set to 1 second by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(feature = "openssl")]
    /// Use custom `SslConnector` instance.
    pub fn ssl(mut self, connector: OpensslConnector) -> Self {
        self.ssl = SslConnector::Openssl(connector);
        self
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(mut self, connector: Arc<ClientConfig>) -> Self {
        self.ssl = SslConnector::Rustls(connector);
        self
    }

    /// Set total number of simultaneous connections per type of scheme.
    ///
    /// If limit is 0, the connector has no limit.
    /// The default limit size is 100.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set keep-alive period for opened connection.
    ///
    /// Keep-alive period is the period between connection usage. If
    /// the delay between repeated usages of the same connection
    /// exceeds this period, the connection is closed.
    /// Default keep-alive period is 15 seconds.
    pub fn conn_keep_alive(mut self, dur: Duration) -> Self {
        self.conn_keep_alive = dur;
        self
    }

    /// Set max lifetime period for connection.
    ///
    /// Connection lifetime is max lifetime of any opened connection
    /// until it is closed regardless of keep-alive period.
    /// Default lifetime period is 75 seconds.
    pub fn conn_lifetime(mut self, dur: Duration) -> Self {
        self.conn_lifetime = dur;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the socket get dropped. This timeout affects only secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 3000 milliseconds.
    pub fn disconnect_timeout(mut self, dur: Duration) -> Self {
        self.disconnect_timeout = dur;
        self
    }

    /// Finish configuration process and create connector service.
    /// The Connector builder always concludes by calling `finish()` last in
    /// its combinator chain.
    pub fn finish(
        self,
    ) -> impl Service<Request = Connect, Response = impl Connection, Error = ConnectError>
           + Clone {
        #[cfg(not(any(feature = "openssl", feature = "rustls")))]
        {
            let connector = TimeoutService::new(
                self.timeout,
                apply_fn(UnpinWrapper(self.connector), |msg: Connect, srv| {
                    srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                })
                .map_err(ConnectError::from)
                .map(|stream| (stream.into_parts().0, Protocol::Http1)),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    connector,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    None,
                    self.limit,
                ),
            }
        }
        #[cfg(any(feature = "openssl", feature = "rustls"))]
        {
            const H2: &[u8] = b"h2";
            #[cfg(feature = "openssl")]
            use actix_connect::ssl::OpensslConnector;
            #[cfg(feature = "rustls")]
            use actix_connect::ssl::RustlsConnector;
            use actix_service::{boxed::service, pipeline};
            #[cfg(feature = "rustls")]
            use rust_tls::Session;

            let ssl_service = TimeoutService::new(
                self.timeout,
                pipeline(
                    apply_fn(
                        UnpinWrapper(self.connector.clone()),
                        |msg: Connect, srv| {
                            srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                        },
                    )
                    .map_err(ConnectError::from),
                )
                .and_then(match self.ssl {
                    #[cfg(feature = "openssl")]
                    SslConnector::Openssl(ssl) => OpensslConnector::service(ssl)
                        .map(|stream| {
                            let sock = stream.into_parts().0;
                            let h2 = sock
                                .ssl()
                                .selected_alpn_protocol()
                                .map(|protos| protos.windows(2).any(|w| w == H2))
                                .unwrap_or(false);
                            if h2 {
                                (Box::new(sock) as Box<dyn Io + Unpin>, Protocol::Http2)
                            } else {
                                (Box::new(sock) as Box<dyn Io + Unpin>, Protocol::Http1)
                            }
                        })
                        .map_err(ConnectError::from),

                    #[cfg(feature = "rustls")]
                    SslConnector::Rustls(ssl) => service(
                        UnpinWrapper(RustlsConnector::service(ssl))
                            .map_err(ConnectError::from)
                            .map(|stream| {
                                let sock = stream.into_parts().0;
                                let h2 = sock
                                    .get_ref()
                                    .1
                                    .get_alpn_protocol()
                                    .map(|protos| protos.windows(2).any(|w| w == H2))
                                    .unwrap_or(false);
                                if h2 {
                                    (
                                        Box::new(sock) as Box<dyn Io + Unpin>,
                                        Protocol::Http2,
                                    )
                                } else {
                                    (
                                        Box::new(sock) as Box<dyn Io + Unpin>,
                                        Protocol::Http1,
                                    )
                                }
                            }),
                    ),
                }),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            let tcp_service = TimeoutService::new(
                self.timeout,
                apply_fn(UnpinWrapper(self.connector), |msg: Connect, srv| {
                    srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                })
                .map_err(ConnectError::from)
                .map(|stream| (stream.into_parts().0, Protocol::Http1)),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    tcp_service,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    None,
                    self.limit,
                ),
                ssl_pool: ConnectionPool::new(
                    ssl_service,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    Some(self.disconnect_timeout),
                    self.limit,
                ),
            }
        }
    }
}

#[derive(Clone)]
struct UnpinWrapper<T: Clone>(T);

impl<T: Clone> Unpin for UnpinWrapper<T> {}

impl<T: Service + Clone> Service for UnpinWrapper<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = UnpinWrapperFut<T>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), T::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: T::Request) -> Self::Future {
        UnpinWrapperFut {
            fut: self.0.call(req),
        }
    }
}

struct UnpinWrapperFut<T: Service> {
    fut: T::Future,
}

impl<T: Service> Unpin for UnpinWrapperFut<T> {}

impl<T: Service> Future for UnpinWrapperFut<T> {
    type Output = Result<T::Response, T::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        unsafe { Pin::new_unchecked(&mut self.get_mut().fut) }.poll(cx)
    }
}

#[cfg(not(any(feature = "openssl", feature = "rustls")))]
mod connect_impl {
    use std::task::{Context, Poll};

    use futures::future::{err, Either, Ready};
    use futures::ready;

    use super::*;
    use crate::client::connection::IoConnection;

    pub(crate) struct InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
    {
        pub(crate) tcp_pool: ConnectionPool<T, Io>,
    }

    impl<T, Io> Clone for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
            }
        }
    }

    impl<T, Io> Service for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T::Future: Unpin,
    {
        type Request = Connect;
        type Response = IoConnection<Io>;
        type Error = ConnectError;
        type Future = Either<
            <ConnectionPool<T, Io> as Service>::Future,
            Ready<Result<IoConnection<Io>, ConnectError>>,
        >;

        fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
            self.tcp_pool.poll_ready(cx)
        }

        fn call(&mut self, req: Connect) -> Self::Future {
            match req.uri.scheme_str() {
                Some("https") | Some("wss") => {
                    Either::Right(err(ConnectError::SslIsNotSupported))
                }
                _ => Either::Left(self.tcp_pool.call(req)),
            }
        }
    }
}

#[cfg(any(feature = "openssl", feature = "rustls"))]
mod connect_impl {
    use std::marker::PhantomData;

    use futures::future::Either;
    use futures::ready;

    use super::*;
    use crate::client::connection::EitherConnection;

    pub(crate) struct InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>,
        T1::Future: Unpin,
        T2::Future: Unpin,
    {
        pub(crate) tcp_pool: ConnectionPool<T1, Io1>,
        pub(crate) ssl_pool: ConnectionPool<T2, Io2>,
    }

    impl<T1, T2, Io1, Io2> Clone for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T1::Future: Unpin,
        T2::Future: Unpin,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
                ssl_pool: self.ssl_pool.clone(),
            }
        }
    }

    impl<T1, T2, Io1, Io2> Service for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T1::Future: Unpin,
        T2::Future: Unpin,
    {
        type Request = Connect;
        type Response = EitherConnection<Io1, Io2>;
        type Error = ConnectError;
        type Future = Either<
            InnerConnectorResponseA<T1, Io1, Io2>,
            InnerConnectorResponseB<T2, Io1, Io2>,
        >;

        fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
            self.tcp_pool.poll_ready(cx)
        }

        fn call(&mut self, req: Connect) -> Self::Future {
            match req.uri.scheme_str() {
                Some("https") | Some("wss") => Either::Right(InnerConnectorResponseB {
                    fut: self.ssl_pool.call(req),
                    _t: PhantomData,
                }),
                _ => Either::Left(InnerConnectorResponseA {
                    fut: self.tcp_pool.call(req),
                    _t: PhantomData,
                }),
            }
        }
    }

    pub(crate) struct InnerConnectorResponseA<T, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T::Future: Unpin,
    {
        fut: <ConnectionPool<T, Io1> as Service>::Future,
        _t: PhantomData<Io2>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseA<T, Io1, Io2>
    where
        T: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T::Future: Unpin,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        type Output = Result<EitherConnection<Io1, Io2>, ConnectError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
            Poll::Ready(
                ready!(Pin::new(&mut self.get_mut().fut).poll(cx))
                    .map(|res| EitherConnection::A(res)),
            )
        }
    }

    pub(crate) struct InnerConnectorResponseB<T, Io1, Io2>
    where
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T::Future: Unpin,
    {
        fut: <ConnectionPool<T, Io2> as Service>::Future,
        _t: PhantomData<Io1>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseB<T, Io1, Io2>
    where
        T: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>
            + Unpin
            + 'static,
        T::Future: Unpin,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        type Output = Result<EitherConnection<Io1, Io2>, ConnectError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
            Poll::Ready(
                ready!(Pin::new(&mut self.get_mut().fut).poll(cx))
                    .map(|res| EitherConnection::B(res)),
            )
        }
    }
}
