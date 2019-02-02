//! Contains `Either` service and related types and functions.
use actix_service::{NewService, Service};
use futures::{future, try_ready, Async, Future, Poll};

/// Combine two different service types into a single type.
///
/// Both services must be of the same request, response, and error types.
/// `EitherService` is useful for handling conditional branching in service
/// middleware to different inner service types.
pub enum EitherService<A, B> {
    A(A),
    B(B),
}

impl<A: Clone, B: Clone> Clone for EitherService<A, B> {
    fn clone(&self) -> Self {
        match self {
            EitherService::A(srv) => EitherService::A(srv.clone()),
            EitherService::B(srv) => EitherService::B(srv.clone()),
        }
    }
}

impl<A, B> Service for EitherService<A, B>
where
    A: Service,
    B: Service<Request = A::Request, Response = A::Response, Error = A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;
    type Future = future::Either<A::Future, B::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        match self {
            EitherService::A(ref mut inner) => inner.poll_ready(),
            EitherService::B(ref mut inner) => inner.poll_ready(),
        }
    }

    fn call(&mut self, req: A::Request) -> Self::Future {
        match self {
            EitherService::A(ref mut inner) => future::Either::A(inner.call(req)),
            EitherService::B(ref mut inner) => future::Either::B(inner.call(req)),
        }
    }
}

/// Combine two different new service types into a single type.
pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A, B> Either<A, B> {
    pub fn new_a(srv: A) -> Self
    where
        A: NewService,
        B: NewService<
            Request = A::Request,
            Response = A::Response,
            Error = A::Error,
            InitError = A::InitError,
        >,
    {
        Either::A(srv)
    }

    pub fn new_b(srv: B) -> Self
    where
        A: NewService,
        B: NewService<
            Request = A::Request,
            Response = A::Response,
            Error = A::Error,
            InitError = A::InitError,
        >,
    {
        Either::B(srv)
    }
}

impl<A, B> NewService for Either<A, B>
where
    A: NewService,
    B: NewService<
        Request = A::Request,
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = A::Error;
    type InitError = A::InitError;
    type Service = EitherService<A::Service, B::Service>;
    type Future = EitherNewService<A, B>;

    fn new_service(&self) -> Self::Future {
        match self {
            Either::A(ref inner) => EitherNewService::A(inner.new_service()),
            Either::B(ref inner) => EitherNewService::B(inner.new_service()),
        }
    }
}

impl<A: Clone, B: Clone> Clone for Either<A, B> {
    fn clone(&self) -> Self {
        match self {
            Either::A(srv) => Either::A(srv.clone()),
            Either::B(srv) => Either::B(srv.clone()),
        }
    }
}

#[doc(hidden)]
pub enum EitherNewService<A: NewService, B: NewService> {
    A(A::Future),
    B(B::Future),
}

impl<A, B> Future for EitherNewService<A, B>
where
    A: NewService,
    B: NewService<
        Request = A::Request,
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Item = EitherService<A::Service, B::Service>;
    type Error = A::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            EitherNewService::A(ref mut fut) => {
                let service = try_ready!(fut.poll());
                Ok(Async::Ready(EitherService::A(service)))
            }
            EitherNewService::B(ref mut fut) => {
                let service = try_ready!(fut.poll());
                Ok(Async::Ready(EitherService::B(service)))
            }
        }
    }
}
