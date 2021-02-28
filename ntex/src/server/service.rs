use std::marker::PhantomData;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::{err, ok, LocalBoxFuture, Ready};
use futures::{FutureExt, TryFutureExt};
use log::error;

use crate::rt::spawn;
use crate::service::{Service, ServiceFactory};
use crate::util::counter::CounterGuard;

use super::socket::{FromStream, Stream};
use super::Token;

/// Server message
pub(super) enum ServerMessage {
    /// New stream
    Connect(Stream),
    /// Gracefull shutdown
    Shutdown(Duration),
    /// Force shutdown
    ForceShutdown,
}

pub trait StreamServiceFactory<Stream: FromStream>: Send + Clone + 'static {
    type Factory: ServiceFactory<Config = (), Request = Stream>;

    fn create(&self) -> Self::Factory;
}

pub(super) trait InternalServiceFactory: Send {
    fn name(&self, token: Token) -> &str;

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory>;

    fn create(
        &self,
    ) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>>;
}

pub(super) type BoxedServerService = Box<
    dyn Service<
        Request = (Option<CounterGuard>, ServerMessage),
        Response = (),
        Error = (),
        Future = Ready<Result<(), ()>>,
    >,
>;

pub(super) struct StreamService<T> {
    service: T,
}

impl<T> StreamService<T> {
    pub(crate) fn new(service: T) -> Self {
        StreamService { service }
    }
}

impl<T, I> Service for StreamService<T>
where
    T: Service<Request = I>,
    T::Future: 'static,
    T::Error: 'static,
    I: FromStream,
{
    type Request = (Option<CounterGuard>, ServerMessage);
    type Response = ();
    type Error = ();
    type Future = Ready<Result<(), ()>>;

    #[inline]
    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx).map_err(|_| ())
    }

    #[inline]
    fn poll_shutdown(&self, cx: &mut Context<'_>, is_error: bool) -> Poll<()> {
        self.service.poll_shutdown(cx, is_error)
    }

    fn call(&self, (guard, req): (Option<CounterGuard>, ServerMessage)) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream = FromStream::from_stream(stream).map_err(|e| {
                    error!("Can not convert to an async io stream: {}", e);
                });

                if let Ok(stream) = stream {
                    let f = self.service.call(stream);
                    spawn(async move {
                        let _ = f.await;
                        drop(guard);
                    });
                    ok(())
                } else {
                    err(())
                }
            }
            _ => ok(()),
        }
    }
}

pub(super) struct Factory<F: StreamServiceFactory<Io>, Io: FromStream> {
    name: String,
    inner: F,
    token: Token,
    addr: SocketAddr,
    _t: PhantomData<Io>,
}

impl<F, Io> Factory<F, Io>
where
    F: StreamServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    pub(crate) fn create(
        name: String,
        token: Token,
        inner: F,
        addr: SocketAddr,
    ) -> Box<dyn InternalServiceFactory> {
        Box::new(Self {
            name,
            token,
            inner,
            addr,
            _t: PhantomData,
        })
    }
}

impl<F, Io> InternalServiceFactory for Factory<F, Io>
where
    F: StreamServiceFactory<Io>,
    Io: FromStream + Send + 'static,
{
    fn name(&self, _: Token) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            token: self.token,
            addr: self.addr,
            _t: PhantomData,
        })
    }

    fn create(
        &self,
    ) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>> {
        let token = self.token;
        self.inner
            .create()
            .new_service(())
            .map_err(|_| ())
            .map_ok(move |inner| {
                let service: BoxedServerService = Box::new(StreamService::new(inner));
                vec![(token, service)]
            })
            .boxed_local()
    }
}

impl InternalServiceFactory for Box<dyn InternalServiceFactory> {
    fn name(&self, token: Token) -> &str {
        self.as_ref().name(token)
    }

    fn clone_factory(&self) -> Box<dyn InternalServiceFactory> {
        self.as_ref().clone_factory()
    }

    fn create(
        &self,
    ) -> LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>> {
        self.as_ref().create()
    }
}

impl<F, T, I> StreamServiceFactory<I> for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: ServiceFactory<Config = (), Request = I>,
    I: FromStream,
{
    type Factory = T;

    #[inline]
    fn create(&self) -> T {
        (self)()
    }
}
