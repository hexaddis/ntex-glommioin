use std::marker::PhantomData;

use futures::{future::ok, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{Error, SslAcceptor, SslConnector};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, ConnectAsync, SslAcceptorExt, SslConnectorExt, SslStream};

use super::MAX_CONN_COUNTER;
use connector::Connect;
use counter::{Counter, CounterGuard};
use service::{NewService, Service};

/// Support `SSL` connections via openssl package
///
/// `ssl` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T> OpensslAcceptor<T> {
    /// Create default `OpensslAcceptor`
    pub fn new(acceptor: SslAcceptor) -> Self {
        OpensslAcceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> Clone for OpensslAcceptor<T> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService for OpensslAcceptor<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Service = OpensslAcceptorService<T>;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ok(OpensslAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct OpensslAcceptorService<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
    conns: Counter,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslAcceptorService<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Future = OpensslAcceptorServiceFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.conns.available() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        OpensslAcceptorServiceFut {
            _guard: self.conns.get(),
            fut: SslAcceptorExt::accept_async(&self.acceptor, req),
        }
    }
}

pub struct OpensslAcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    fut: AcceptAsync<T>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite> Future for OpensslAcceptorServiceFut<T> {
    type Item = SslStream<T>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll()
    }
}

/// Openssl connector factory
pub struct OpensslConnector<T, E> {
    connector: SslConnector,
    _t: PhantomData<(T, E)>,
}

impl<T, E> OpensslConnector<T, E> {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector {
            connector,
            _t: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> OpensslConnector<T, ()> {
    pub fn service(
        connector: SslConnector,
    ) -> impl Service<Request = (Connect, T), Response = (Connect, SslStream<T>), Error = Error>
    {
        OpensslConnectorService {
            connector: connector,
            _t: PhantomData,
        }
    }
}

impl<T, E> Clone for OpensslConnector<T, E> {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite, E> NewService for OpensslConnector<T, E> {
    type Request = (Connect, T);
    type Response = (Connect, SslStream<T>);
    type Error = Error;
    type Service = OpensslConnectorService<T>;
    type InitError = E;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(OpensslConnectorService {
            connector: self.connector.clone(),
            _t: PhantomData,
        })
    }
}

pub struct OpensslConnectorService<T> {
    connector: SslConnector,
    _t: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslConnectorService<T> {
    type Request = (Connect, T);
    type Response = (Connect, SslStream<T>);
    type Error = Error;
    type Future = ConnectAsyncExt<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, stream): Self::Request) -> Self::Future {
        ConnectAsyncExt {
            fut: SslConnectorExt::connect_async(&self.connector, &req.host, stream),
            req: Some(req),
        }
    }
}

pub struct ConnectAsyncExt<T> {
    fut: ConnectAsync<T>,
    req: Option<Connect>,
}

impl<T> Future for ConnectAsyncExt<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = (Connect, SslStream<T>);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(stream) => Ok(Async::Ready((self.req.take().unwrap(), stream))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
