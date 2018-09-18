use std::marker::PhantomData;

use futures::{Async, Future, IntoFuture, Poll};

use super::{IntoNewService, IntoService, NewService, Service};

/// `Apply` service combinator
pub struct Apply<T, F, R, Req> {
    service: T,
    f: F,
    r: PhantomData<(Req, R)>,
}

impl<T, F, R, Req> Apply<T, F, R, Req>
where
    T: Service,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R,
    R: IntoFuture,
{
    /// Create new `Apply` combinator
    pub fn new<I: IntoService<T>>(service: I, f: F) -> Self {
        Self {
            service: service.into_service(),
            f,
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Clone for Apply<T, F, R, Req>
where
    T: Service + Clone,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R + Clone,
    R: IntoFuture,
{
    fn clone(&self) -> Self {
        Apply {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Service for Apply<T, F, R, Req>
where
    T: Service,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T) -> R,
    R: IntoFuture,
{
    type Request = Req;
    type Response = <R::Future as Future>::Item;
    type Error = <R::Future as Future>::Error;
    type Future = R::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|e| e.into())
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        (self.f)(req, &mut self.service).into_future()
    }
}

/// `ApplyNewService` new service combinator
pub struct ApplyNewService<T, F, R, Req> {
    service: T,
    f: F,
    r: PhantomData<Fn(Req) -> R>,
}

impl<T, F, R, Req> ApplyNewService<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    /// Create new `ApplyNewService` new service instance
    pub fn new<F1: IntoNewService<T>>(service: F1, f: F) -> Self {
        Self {
            f,
            service: service.into_new_service(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Clone for ApplyNewService<T, F, R, Req>
where
    T: NewService + Clone,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: IntoFuture,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> NewService for ApplyNewService<T, F, R, Req>
where
    T: NewService,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T::Service) -> R + Clone,
    R: IntoFuture,
{
    type Request = Req;
    type Response = <R::Future as Future>::Item;
    type Error = <R::Future as Future>::Error;
    type Service = Apply<T::Service, F, R, Req>;

    type InitError = T::InitError;
    type Future = ApplyNewServiceFuture<T, F, R, Req>;

    fn new_service(&self) -> Self::Future {
        ApplyNewServiceFuture::new(self.service.new_service(), self.f.clone())
    }
}

pub struct ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    fut: T::Future,
    f: Option<F>,
    r: PhantomData<Fn(Req) -> R>,
}

impl<T, F, R, Req> ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    fn new(fut: T::Future, f: F) -> Self {
        ApplyNewServiceFuture {
            f: Some(f),
            fut,
            r: PhantomData,
        }
    }
}

impl<T, F, R, Req> Future for ApplyNewServiceFuture<T, F, R, Req>
where
    T: NewService,
    T::Error: Into<<R::Future as Future>::Error>,
    F: Fn(Req, &mut T::Service) -> R,
    R: IntoFuture,
{
    type Item = Apply<T::Service, F, R, Req>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(service) = self.fut.poll()? {
            Ok(Async::Ready(Apply::new(service, self.f.take().unwrap())))
        } else {
            Ok(Async::NotReady)
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, FutureResult};
    use futures::{Async, Future, Poll};

    use service::{
        IntoNewService, IntoService, NewService, NewServiceExt, Service, ServiceExt,
    };

    #[derive(Clone)]
    struct Srv;
    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Async::Ready(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[test]
    fn test_call() {
        let blank = |req| Ok(req);

        let mut srv = blank.into_service().apply(Srv, |req: &'static str, srv| {
            srv.call(()).map(move |res| (req, res))
        });
        assert!(srv.poll_ready().is_ok());
        let res = srv.call("srv").poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>((|req| Ok(req)).into_service());

        let new_srv = blank.into_new_service().apply(
            || Ok(Srv),
            |req: &'static str, srv| srv.call(()).map(move |res| (req, res)),
        );
        if let Async::Ready(mut srv) = new_srv.new_service().poll().unwrap() {
            assert!(srv.poll_ready().is_ok());
            let res = srv.call("srv").poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
        } else {
            panic!()
        }
    }
}
