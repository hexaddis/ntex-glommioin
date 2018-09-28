use std::marker::PhantomData;
use std::{fmt, net};

use actix_net::either::Either;
use actix_net::server::{Server, ServiceFactory};
use actix_net::service::{NewService, NewServiceExt};

use super::acceptor::{
    AcceptorServiceFactory, AcceptorTimeout, ServerMessageAcceptor, TcpAcceptor,
};
use super::error::AcceptorError;
use super::handler::{HttpHandler, IntoHttpHandler};
use super::service::HttpService;
use super::settings::{ServerSettings, WorkerSettings};
use super::{IoStream, KeepAlive};

pub(crate) trait ServiceProvider {
    fn register(
        &self, server: Server, lst: net::TcpListener, host: Option<String>,
        addr: net::SocketAddr, keep_alive: KeepAlive, client_timeout: usize,
    ) -> Server;
}

/// Utility type that builds complete http pipeline
pub struct HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
{
    factory: F,
    acceptor: A,
    pipeline: P,
    no_client_timer: bool,
}

impl<F, H, A, P> HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone + 'static,
    H: IntoHttpHandler,
    A: AcceptorServiceFactory,
    <A::NewService as NewService>::InitError: fmt::Debug,
    P: HttpPipelineFactory<H::Handler, Io = A::Io>,
{
    /// Create http service builder
    pub fn new(factory: F, acceptor: A, pipeline: P) -> Self {
        Self {
            factory,
            pipeline,
            acceptor,
            no_client_timer: false,
        }
    }

    pub(crate) fn no_client_timer(mut self) -> Self {
        self.no_client_timer = true;
        self
    }

    /// Use different acceptor factory
    pub fn acceptor<A1>(self, acceptor: A1) -> HttpServiceBuilder<F, H, A1, P>
    where
        A1: AcceptorServiceFactory,
        <A1::NewService as NewService>::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            acceptor,
            pipeline: self.pipeline,
            factory: self.factory.clone(),
            no_client_timer: self.no_client_timer,
        }
    }

    /// Use different pipeline factory
    pub fn pipeline<P1>(self, pipeline: P1) -> HttpServiceBuilder<F, H, A, P1>
    where
        P1: HttpPipelineFactory<H::Handler>,
    {
        HttpServiceBuilder {
            pipeline,
            acceptor: self.acceptor,
            factory: self.factory.clone(),
            no_client_timer: self.no_client_timer,
        }
    }

    fn finish(
        &self, host: Option<String>, addr: net::SocketAddr, keep_alive: KeepAlive,
        client_timeout: usize,
    ) -> impl ServiceFactory {
        let timeout = if self.no_client_timer {
            0
        } else {
            client_timeout
        };
        let factory = self.factory.clone();
        let pipeline = self.pipeline.clone();
        let acceptor = self.acceptor.clone();
        move || {
            let app = (factory)().into_handler();
            let settings = WorkerSettings::new(
                app,
                keep_alive,
                timeout as u64,
                ServerSettings::new(Some(addr), &host, false),
            );

            if timeout == 0 {
                Either::A(ServerMessageAcceptor::new(
                    settings.clone(),
                    TcpAcceptor::new(acceptor.create().map_err(AcceptorError::Service))
                        .map_err(|_| ())
                        .map_init_err(|_| ())
                        .and_then(
                            pipeline
                                .create(settings)
                                .map_init_err(|_| ())
                                .map_err(|_| ()),
                        ),
                ))
            } else {
                Either::B(ServerMessageAcceptor::new(
                    settings.clone(),
                    TcpAcceptor::new(AcceptorTimeout::new(timeout, acceptor.create()))
                        .map_err(|_| ())
                        .map_init_err(|_| ())
                        .and_then(
                            pipeline
                                .create(settings)
                                .map_init_err(|_| ())
                                .map_err(|_| ()),
                        ),
                ))
            }
        }
    }
}

impl<F, H, A, P> Clone for HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
    H: IntoHttpHandler,
    A: AcceptorServiceFactory,
    P: HttpPipelineFactory<H::Handler, Io = A::Io>,
{
    fn clone(&self) -> Self {
        HttpServiceBuilder {
            factory: self.factory.clone(),
            acceptor: self.acceptor.clone(),
            pipeline: self.pipeline.clone(),
            no_client_timer: self.no_client_timer,
        }
    }
}

impl<F, H, A, P> ServiceProvider for HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone + 'static,
    A: AcceptorServiceFactory,
    <A::NewService as NewService>::InitError: fmt::Debug,
    P: HttpPipelineFactory<H::Handler, Io = A::Io>,
    H: IntoHttpHandler,
{
    fn register(
        &self, server: Server, lst: net::TcpListener, host: Option<String>,
        addr: net::SocketAddr, keep_alive: KeepAlive, client_timeout: usize,
    ) -> Server {
        server.listen2(
            "actix-web",
            lst,
            self.finish(host, addr, keep_alive, client_timeout),
        )
    }
}

pub trait HttpPipelineFactory<H: HttpHandler>: Send + Clone + 'static {
    type Io: IoStream;
    type NewService: NewService<Request = Self::Io, Response = ()>;

    fn create(&self, settings: WorkerSettings<H>) -> Self::NewService;
}

impl<F, T, H> HttpPipelineFactory<H> for F
where
    F: Fn(WorkerSettings<H>) -> T + Send + Clone + 'static,
    T: NewService<Response = ()>,
    T::Request: IoStream,
    H: HttpHandler,
{
    type Io = T::Request;
    type NewService = T;

    fn create(&self, settings: WorkerSettings<H>) -> T {
        (self)(settings)
    }
}

pub(crate) struct DefaultPipelineFactory<H, Io> {
    _t: PhantomData<(H, Io)>,
}

unsafe impl<H, Io> Send for DefaultPipelineFactory<H, Io> {}

impl<H, Io> DefaultPipelineFactory<H, Io>
where
    Io: IoStream + Send,
    H: HttpHandler + 'static,
{
    pub fn new() -> Self {
        Self { _t: PhantomData }
    }
}

impl<H, Io> Clone for DefaultPipelineFactory<H, Io>
where
    Io: IoStream,
    H: HttpHandler,
{
    fn clone(&self) -> Self {
        Self { _t: PhantomData }
    }
}

impl<H, Io> HttpPipelineFactory<H> for DefaultPipelineFactory<H, Io>
where
    Io: IoStream,
    H: HttpHandler + 'static,
{
    type Io = Io;
    type NewService = HttpService<H, Io>;

    fn create(&self, settings: WorkerSettings<H>) -> Self::NewService {
        HttpService::new(settings)
    }
}
