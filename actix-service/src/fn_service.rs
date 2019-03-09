use std::marker::PhantomData;

use futures::future::{ok, Future, FutureResult};
use futures::{try_ready, Async, IntoFuture, Poll};

use crate::{IntoConfigurableNewService, IntoNewService, IntoService, NewService, Service};

/// Create `NewService` for function that can act as Service
pub fn fn_service<F, Req, Out, Cfg>(f: F) -> FnNewService<F, Req, Out, Cfg>
where
    F: FnMut(Req) -> Out + Clone,
    Out: IntoFuture,
{
    FnNewService::new(f)
}

/// Create `NewService` for function that can produce services
pub fn fn_factory<F, R, S, E>(f: F) -> FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    FnNewServiceNoConfig::new(f)
}

/// Create `NewService` for function that can produce services with configuration
pub fn fn_cfg_factory<F, C, R, S, E>(f: F) -> FnNewServiceConfig<F, C, R, S, E>
where
    F: Fn(&C) -> R,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    FnNewServiceConfig::new(f)
}

pub struct FnService<F, Req, Out>
where
    F: FnMut(Req) -> Out,
    Out: IntoFuture,
{
    f: F,
    _t: PhantomData<(Req,)>,
}

impl<F, Req, Out> FnService<F, Req, Out>
where
    F: FnMut(Req) -> Out,
    Out: IntoFuture,
{
    pub fn new(f: F) -> Self {
        FnService { f, _t: PhantomData }
    }
}

impl<F, Req, Out> Clone for FnService<F, Req, Out>
where
    F: FnMut(Req) -> Out + Clone,
    Out: IntoFuture,
{
    fn clone(&self) -> Self {
        FnService {
            f: self.f.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, Req, Out> Service for FnService<F, Req, Out>
where
    F: FnMut(Req) -> Out,
    Out: IntoFuture,
{
    type Request = Req;
    type Response = Out::Item;
    type Error = Out::Error;
    type Future = Out::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Req) -> Self::Future {
        (self.f)(req).into_future()
    }
}

impl<F, Req, Out> IntoService<FnService<F, Req, Out>> for F
where
    F: FnMut(Req) -> Out + 'static,
    Out: IntoFuture,
{
    fn into_service(self) -> FnService<F, Req, Out> {
        FnService::new(self)
    }
}

pub struct FnNewService<F, Req, Out, Cfg>
where
    F: FnMut(Req) -> Out,
    Out: IntoFuture,
{
    f: F,
    _t: PhantomData<(Req, Cfg)>,
}

impl<F, Req, Out, Cfg> FnNewService<F, Req, Out, Cfg>
where
    F: FnMut(Req) -> Out + Clone,
    Out: IntoFuture,
{
    pub fn new(f: F) -> Self {
        FnNewService { f, _t: PhantomData }
    }
}

impl<F, Req, Out, Cfg> NewService<Cfg> for FnNewService<F, Req, Out, Cfg>
where
    F: FnMut(Req) -> Out + Clone,
    Out: IntoFuture,
{
    type Request = Req;
    type Response = Out::Item;
    type Error = Out::Error;
    type Service = FnService<F, Req, Out>;

    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &Cfg) -> Self::Future {
        ok(FnService::new(self.f.clone()))
    }
}

impl<F, Req, Out, Cfg> Clone for FnNewService<F, Req, Out, Cfg>
where
    F: FnMut(Req) -> Out + Clone,
    Out: IntoFuture,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, Req, Out, Cfg> IntoNewService<FnNewService<F, Req, Out, Cfg>, Cfg> for F
where
    F: Fn(Req) -> Out + Clone,
    Out: IntoFuture,
{
    fn into_new_service(self) -> FnNewService<F, Req, Out, Cfg> {
        FnNewService::new(self)
    }
}

/// Converter for `Fn() -> Future<Service>` fn
pub struct FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    f: F,
}

impl<F, R, S, E> FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    pub fn new(f: F) -> Self {
        FnNewServiceNoConfig { f }
    }
}

impl<F, R, S, E> NewService<()> for FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;

    type InitError = E;
    type Future = R::Future;

    fn new_service(&self, _: &()) -> Self::Future {
        (self.f)().into_future()
    }
}

impl<F, R, S, E> Clone for FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R + Clone,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, R, S, E> IntoNewService<FnNewServiceNoConfig<F, R, S, E>, ()> for F
where
    F: Fn() -> R,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    fn into_new_service(self) -> FnNewServiceNoConfig<F, R, S, E> {
        FnNewServiceNoConfig::new(self)
    }
}

/// Convert `Fn(&Config) -> Future<Service>` fn to NewService
pub struct FnNewServiceConfig<F, C, R, S, E>
where
    F: Fn(&C) -> R,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    f: F,
    _t: PhantomData<(C, R, S, E)>,
}

impl<F, C, R, S, E> FnNewServiceConfig<F, C, R, S, E>
where
    F: Fn(&C) -> R,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    pub fn new(f: F) -> Self {
        FnNewServiceConfig { f, _t: PhantomData }
    }
}

impl<F, C, R, S, E> NewService<C> for FnNewServiceConfig<F, C, R, S, E>
where
    F: Fn(&C) -> R,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;

    type InitError = E;
    type Future = FnNewServiceConfigFut<R, S, E>;

    fn new_service(&self, cfg: &C) -> Self::Future {
        FnNewServiceConfigFut {
            fut: (self.f)(cfg).into_future(),
            _t: PhantomData,
        }
    }
}

pub struct FnNewServiceConfigFut<R, S, E>
where
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    fut: R::Future,
    _t: PhantomData<(S,)>,
}

impl<R, S, E> Future for FnNewServiceConfigFut<R, S, E>
where
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    type Item = S;
    type Error = R::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(try_ready!(self.fut.poll()).into_service()))
    }
}

impl<F, C, R, S, E> Clone for FnNewServiceConfig<F, C, R, S, E>
where
    F: Fn(&C) -> R + Clone,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    fn clone(&self) -> Self {
        Self::new(self.f.clone())
    }
}

impl<F, C, R, S, E> IntoConfigurableNewService<FnNewServiceConfig<F, C, R, S, E>, C> for F
where
    F: Fn(&C) -> R,
    R: IntoFuture<Error = E>,
    R::Item: IntoService<S>,
    S: Service,
{
    fn into_new_service(self) -> FnNewServiceConfig<F, C, R, S, E> {
        FnNewServiceConfig::new(self)
    }
}
