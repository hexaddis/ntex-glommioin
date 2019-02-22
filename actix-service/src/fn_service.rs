use std::marker::PhantomData;

use futures::future::{ok, FutureResult};
use futures::{Async, IntoFuture, Poll};

use super::{IntoNewService, IntoService, NewService, Service};

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

impl<F, R, S, E> IntoNewService<FnNewServiceNoConfig<F, R, S, E>, ()> for F
where
    F: Fn() -> R + Clone,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    fn into_new_service(self) -> FnNewServiceNoConfig<F, R, S, E> {
        FnNewServiceNoConfig::new(self)
    }
}

pub struct FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R + Clone,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    f: F,
}

impl<F, R, S, E> FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R + Clone,
    R: IntoFuture<Item = S, Error = E>,
    S: Service,
{
    pub fn new(f: F) -> Self {
        FnNewServiceNoConfig { f }
    }
}

impl<F, R, S, E> NewService<()> for FnNewServiceNoConfig<F, R, S, E>
where
    F: Fn() -> R + Clone,
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
