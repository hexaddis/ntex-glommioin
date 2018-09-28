use std::net;
use std::time::Duration;

use futures::future::{err, ok, FutureResult};
use futures::{Future, Poll};
use tokio_current_thread::spawn;
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

use counter::CounterGuard;
use service::{NewService, Service};

/// Server message
pub enum ServerMessage {
    /// New stream
    Connect(net::TcpStream),
    /// Gracefull shutdown
    Shutdown(Duration),
    /// Force shutdown
    ForceShutdown,
}

pub trait StreamServiceFactory: Send + Clone + 'static {
    type NewService: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()>;

    fn create(&self) -> Self::NewService;
}

pub trait ServiceFactory: Send + Clone + 'static {
    type NewService: NewService<
        Request = ServerMessage,
        Response = (),
        Error = (),
        InitError = (),
    >;

    fn create(&self) -> Self::NewService;
}

pub(crate) trait InternalServiceFactory: Send {
    fn name(&self) -> &str;

    fn clone_factory(&self) -> Box<InternalServiceFactory>;

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>>;
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
    fn new(service: T) -> Self {
        StreamService { service }
    }
}

impl<T> Service for StreamService<T>
where
    T: Service<Request = TcpStream, Response = (), Error = ()>,
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
                    spawn(self.service.call(stream).map(move |val| {
                        drop(guard);
                        val
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

pub(crate) struct ServerService<T> {
    service: T,
}

impl<T> ServerService<T> {
    fn new(service: T) -> Self {
        ServerService { service }
    }
}

impl<T> Service for ServerService<T>
where
    T: Service<Request = ServerMessage, Response = (), Error = ()>,
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
        spawn(self.service.call(req).map(move |val| {
            drop(guard);
            val
        }));
        ok(())
    }
}

pub(crate) struct ServiceNewService<F: ServiceFactory> {
    name: String,
    inner: F,
}

impl<F> ServiceNewService<F>
where
    F: ServiceFactory,
{
    pub(crate) fn create(name: String, inner: F) -> Box<InternalServiceFactory> {
        Box::new(Self { name, inner })
    }
}

impl<F> InternalServiceFactory for ServiceNewService<F>
where
    F: ServiceFactory,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        Box::new(self.inner.create().new_service().map(move |inner| {
            let service: BoxedServerService = Box::new(ServerService::new(inner));
            service
        }))
    }
}

pub(crate) struct StreamNewService<F: StreamServiceFactory> {
    name: String,
    inner: F,
}

impl<F> StreamNewService<F>
where
    F: StreamServiceFactory,
{
    pub(crate) fn create(name: String, inner: F) -> Box<InternalServiceFactory> {
        Box::new(Self { name, inner })
    }
}

impl<F> InternalServiceFactory for StreamNewService<F>
where
    F: StreamServiceFactory,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        Box::new(self.inner.create().new_service().map(move |inner| {
            let service: BoxedServerService = Box::new(StreamService::new(inner));
            service
        }))
    }
}

impl InternalServiceFactory for Box<InternalServiceFactory> {
    fn name(&self) -> &str {
        self.as_ref().name()
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        self.as_ref().create()
    }
}

impl<F, T> ServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = ServerMessage, Response = (), Error = (), InitError = ()>,
{
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}

impl<F, T> StreamServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()>,
{
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}
