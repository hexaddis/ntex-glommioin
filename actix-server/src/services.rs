use std::net;
use std::time::Duration;

use actix_rt::spawn;
use actix_service::{NewService, Service};
use futures::future::{err, ok, FutureResult};
use futures::{Future, Poll};
use log::error;
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

use super::Token;
use crate::counter::CounterGuard;

/// Server message
pub(crate) enum ServerMessage {
    /// New stream
    Connect(net::TcpStream),
    /// Gracefull shutdown
    Shutdown(Duration),
    /// Force shutdown
    ForceShutdown,
}

pub trait ServiceFactory: Send + Clone + 'static {
    type NewService: NewService<Request = TcpStream>;

    fn create(&self) -> Self::NewService;
}

pub(crate) trait InternalServiceFactory: Send {
    fn name(&self, token: Token) -> &str;

    fn clone_factory(&self) -> Box<InternalServiceFactory>;

    fn create(&self) -> Box<Future<Item = Vec<(Token, BoxedServerService)>, Error = ()>>;
}

pub(crate) type BoxedServerService = Box<
    Service<
        Request = (Option<CounterGuard>, ServerMessage),
        Response = (),
        Error = (),
        Future = FutureResult<(), ()>,
    >,
>;

pub(crate) struct StreamService<T> {
    service: T,
}

impl<T> StreamService<T> {
    pub(crate) fn new(service: T) -> Self {
        StreamService { service }
    }
}

impl<T> Service for StreamService<T>
where
    T: Service<Request = TcpStream>,
    T::Future: 'static,
    T::Error: 'static,
{
    type Request = (Option<CounterGuard>, ServerMessage);
    type Response = ();
    type Error = ();
    type Future = FutureResult<(), ()>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, (guard, req): (Option<CounterGuard>, ServerMessage)) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream = TcpStream::from_std(stream, &Handle::default()).map_err(|e| {
                    error!("Can not convert to an async tcp stream: {}", e);
                });

                if let Ok(stream) = stream {
                    spawn(self.service.call(stream).then(move |res| {
                        drop(guard);
                        res.map_err(|_| ()).map(|_| ())
                    }));
                    ok(())
                } else {
                    err(())
                }
            }
            _ => ok(()),
        }
    }
}

pub(crate) struct StreamNewService<F: ServiceFactory> {
    name: String,
    inner: F,
    token: Token,
}

impl<F> StreamNewService<F>
where
    F: ServiceFactory,
{
    pub(crate) fn create(name: String, token: Token, inner: F) -> Box<InternalServiceFactory> {
        Box::new(Self { name, token, inner })
    }
}

impl<F> InternalServiceFactory for StreamNewService<F>
where
    F: ServiceFactory,
{
    fn name(&self, _: Token) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
            token: self.token,
        })
    }

    fn create(&self) -> Box<Future<Item = Vec<(Token, BoxedServerService)>, Error = ()>> {
        let token = self.token;
        Box::new(
            self.inner
                .create()
                .new_service(&())
                .map_err(|_| ())
                .map(move |inner| {
                    let service: BoxedServerService = Box::new(StreamService::new(inner));
                    vec![(token, service)]
                }),
        )
    }
}

impl InternalServiceFactory for Box<InternalServiceFactory> {
    fn name(&self, token: Token) -> &str {
        self.as_ref().name(token)
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> Box<Future<Item = Vec<(Token, BoxedServerService)>, Error = ()>> {
        self.as_ref().create()
    }
}

impl<F, T> ServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream>,
{
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}
