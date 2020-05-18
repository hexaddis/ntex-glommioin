//! Contains `Either` service and related types and functions.
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{future, ready, Future};

use crate::service::{Service, ServiceFactory};

/// Construct `Either` service factory.
///
/// Either service allow to combine two different services into a single service.
pub fn either<A, B>(left: A, right: B) -> Either<A, B>
where
    A: ServiceFactory,
    A::Config: Clone,
    B: ServiceFactory<
        Config = A::Config,
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    Either { left, right }
}

/// Combine two different service types into a single type.
///
/// Both services must be of the same request, response, and error types.
/// `EitherService` is useful for handling conditional branching in service
/// middleware to different inner service types.
pub struct EitherService<A, B> {
    left: A,
    right: B,
}

impl<A: Clone, B: Clone> Clone for EitherService<A, B> {
    fn clone(&self) -> Self {
        EitherService {
            left: self.left.clone(),
            right: self.right.clone(),
        }
    }
}

impl<A, B> Service for EitherService<A, B>
where
    A: Service,
    B: Service<Response = A::Response, Error = A::Error>,
{
    type Request = either::Either<A::Request, B::Request>;
    type Response = A::Response;
    type Error = A::Error;
    type Future = future::Either<A::Future, B::Future>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let left = self.left.poll_ready(cx)?;
        let right = self.right.poll_ready(cx)?;

        if left.is_ready() && right.is_ready() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn poll_shutdown(&self, cx: &mut Context<'_>, is_error: bool) -> Poll<()> {
        let left = self.left.poll_shutdown(cx, is_error).is_ready();
        let right = self.right.poll_shutdown(cx, is_error).is_ready();

        if left && right {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn call(&self, req: either::Either<A::Request, B::Request>) -> Self::Future {
        match req {
            either::Either::Left(req) => future::Either::Left(self.left.call(req)),
            either::Either::Right(req) => future::Either::Right(self.right.call(req)),
        }
    }
}

/// Combine two different new service types into a single service.
pub struct Either<A, B> {
    left: A,
    right: B,
}

impl<A, B> ServiceFactory for Either<A, B>
where
    A: ServiceFactory,
    A::Config: Clone,
    B: ServiceFactory<
        Config = A::Config,
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Request = either::Either<A::Request, B::Request>;
    type Response = A::Response;
    type Error = A::Error;
    type InitError = A::InitError;
    type Config = A::Config;
    type Service = EitherService<A::Service, B::Service>;
    type Future = EitherResponse<A, B>;

    fn new_service(&self, cfg: A::Config) -> Self::Future {
        EitherResponse {
            left: None,
            right: None,
            left_fut: self.left.new_service(cfg.clone()),
            right_fut: self.right.new_service(cfg),
        }
    }
}

impl<A: Clone, B: Clone> Clone for Either<A, B> {
    fn clone(&self) -> Self {
        Self {
            left: self.left.clone(),
            right: self.right.clone(),
        }
    }
}

pin_project_lite::pin_project! {
    #[doc(hidden)]
    pub struct EitherResponse<A: ServiceFactory, B: ServiceFactory> {
        left: Option<A::Service>,
        right: Option<B::Service>,
        #[pin]
        left_fut: A::Future,
        #[pin]
        right_fut: B::Future,
    }
}

impl<A, B> Future for EitherResponse<A, B>
where
    A: ServiceFactory,
    B: ServiceFactory<
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Output = Result<EitherService<A::Service, B::Service>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.left.is_none() {
            *this.left = Some(ready!(this.left_fut.poll(cx))?);
        }
        if this.right.is_none() {
            *this.right = Some(ready!(this.right_fut.poll(cx))?);
        }

        if this.left.is_some() && this.right.is_some() {
            Poll::Ready(Ok(EitherService {
                left: this.left.take().unwrap(),
                right: this.right.take().unwrap(),
            }))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{lazy, ok, Ready};
    use std::task::{Context, Poll};

    use super::*;
    use crate::service::{fn_factory, Service, ServiceFactory};

    #[derive(Clone)]
    struct Srv1;

    impl Service for Srv1 {
        type Request = ();
        type Response = usize;
        type Error = ();
        type Future = Ready<Result<usize, ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(&self, _: &mut Context<'_>, _: bool) -> Poll<()> {
            Poll::Ready(())
        }

        fn call(&self, _: ()) -> Self::Future {
            ok::<_, ()>(1)
        }
    }

    #[derive(Clone)]
    struct Srv2;

    impl Service for Srv2 {
        type Request = ();
        type Response = usize;
        type Error = ();
        type Future = Ready<Result<usize, ()>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(&self, _: &mut Context<'_>, _: bool) -> Poll<()> {
            Poll::Ready(())
        }

        fn call(&self, _: ()) -> Self::Future {
            ok::<_, ()>(2)
        }
    }

    #[ntex_rt::test]
    async fn test_service() {
        let service = EitherService {
            left: Srv1,
            right: Srv2,
        }
        .clone();
        assert!(lazy(|cx| service.poll_ready(cx)).await.is_ready());
        assert!(lazy(|cx| service.poll_shutdown(cx, true)).await.is_ready());

        assert_eq!(service.call(either::Either::Left(())).await, Ok(1));
        assert_eq!(service.call(either::Either::Right(())).await, Ok(2));
    }

    #[ntex_rt::test]
    async fn test_factory() {
        let factory = either(
            fn_factory(|| ok::<_, ()>(Srv1)),
            fn_factory(|| ok::<_, ()>(Srv2)),
        )
        .clone();
        let service = factory.new_service(&()).await.unwrap();

        assert!(lazy(|cx| service.poll_ready(cx)).await.is_ready());
        assert!(lazy(|cx| service.poll_shutdown(cx, true)).await.is_ready());

        assert_eq!(service.call(either::Either::Left(())).await, Ok(1));
        assert_eq!(service.call(either::Either::Right(())).await, Ok(2));
    }
}
