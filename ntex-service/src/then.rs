use std::{
    future::Future, marker::PhantomData, pin::Pin, rc::Rc, task::Context, task::Poll,
};

use super::{Service, ServiceFactory};

/// Service for the `then` combinator, chaining a computation onto the end of
/// another service.
///
/// This is created by the `Pipeline::then` method.
pub struct Then<A, B, R>(Rc<(A, B)>, PhantomData<R>);

impl<A, B, R> Then<A, B, R> {
    /// Create new `.then()` combinator
    pub(crate) fn new(a: A, b: B) -> Then<A, B, R>
    where
        A: Service<R>,
        B: Service<Result<A::Response, A::Error>, Error = A::Error>,
    {
        Self(Rc::new((a, b)), PhantomData)
    }
}

impl<A, B, R> Clone for Then<A, B, R> {
    fn clone(&self) -> Self {
        Then(self.0.clone(), PhantomData)
    }
}

impl<A, B, R> Service<R> for Then<A, B, R>
where
    A: Service<R>,
    B: Service<Result<A::Response, A::Error>, Error = A::Error>,
{
    type Response = B::Response;
    type Error = B::Error;
    type Future = ThenServiceResponse<A, B, R>;

    #[inline]
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let srv = self.0.as_ref();
        let not_ready = !srv.0.poll_ready(cx)?.is_ready();
        if !srv.1.poll_ready(cx)?.is_ready() || not_ready {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_shutdown(&self, cx: &mut Context<'_>, is_error: bool) -> Poll<()> {
        let srv = self.0.as_ref();

        if srv.0.poll_shutdown(cx, is_error).is_ready()
            && srv.1.poll_shutdown(cx, is_error).is_ready()
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn call(&self, req: R) -> Self::Future {
        ThenServiceResponse {
            state: State::A {
                fut: self.0.as_ref().0.call(req),
                b: Some(self.0.clone()),
            },
        }
    }
}

pin_project_lite::pin_project! {
    pub struct ThenServiceResponse<A, B, R>
    where
        A: Service<R>,
        B: Service<Result<A::Response, A::Error>>,
    {
        #[pin]
        state: State<A, B, R>,
    }
}

pin_project_lite::pin_project! {
    #[project = StateProject]
    enum State<A, B, R>
    where
        A: Service<R>,
        B: Service<Result<A::Response, A::Error>>,
    {
        A { #[pin] fut: A::Future, b: Option<Rc<(A, B)>> },
        B { #[pin] fut: B::Future },
        Empty,
    }
}

impl<A, B, R> Future for ThenServiceResponse<A, B, R>
where
    A: Service<R>,
    B: Service<Result<A::Response, A::Error>>,
{
    type Output = Result<B::Response, B::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        match this.state.as_mut().project() {
            StateProject::A { fut, b } => match fut.poll(cx) {
                Poll::Ready(res) => {
                    let b = b.take().unwrap();
                    this.state.set(State::Empty); // drop fut A
                    let fut = b.as_ref().1.call(res);
                    this.state.set(State::B { fut });
                    self.poll(cx)
                }
                Poll::Pending => Poll::Pending,
            },
            StateProject::B { fut } => fut.poll(cx).map(|r| {
                this.state.set(State::Empty);
                r
            }),
            StateProject::Empty => {
                panic!("future must not be polled after it returned `Poll::Ready`")
            }
        }
    }
}

/// `.then()` service factory combinator
pub struct ThenFactory<A, B, R>(Rc<(A, B)>, PhantomData<R>);

impl<A, B, R> ThenFactory<A, B, R> {
    /// Create new `AndThen` combinator
    pub(crate) fn new(a: A, b: B) -> Self {
        Self(Rc::new((a, b)), PhantomData)
    }
}

impl<A, B, R, C> ServiceFactory<R, C> for ThenFactory<A, B, R>
where
    A: ServiceFactory<R, C>,
    C: Clone,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        C,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Response = B::Response;
    type Error = A::Error;

    type Service = Then<A::Service, B::Service, R>;
    type InitError = A::InitError;
    type Future = ThenFactoryResponse<A, B, R, C>;

    fn new_service(&self, cfg: C) -> Self::Future {
        let srv = &*self.0;
        ThenFactoryResponse::new(srv.0.new_service(cfg.clone()), srv.1.new_service(cfg))
    }
}

impl<A, B, R> Clone for ThenFactory<A, B, R> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

pin_project_lite::pin_project! {
    pub struct ThenFactoryResponse<A, B, R, C>
    where
        A: ServiceFactory<R, C>,
        B: ServiceFactory<Result<A::Response, A::Error>, C,
           Error = A::Error,
           InitError = A::InitError,
        >,
    {
        #[pin]
        fut_b: B::Future,
        #[pin]
        fut_a: A::Future,
        a: Option<A::Service>,
        b: Option<B::Service>,
    }
}

impl<A, B, R, C> ThenFactoryResponse<A, B, R, C>
where
    A: ServiceFactory<R, C>,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        C,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    fn new(fut_a: A::Future, fut_b: B::Future) -> Self {
        Self {
            fut_a,
            fut_b,
            a: None,
            b: None,
        }
    }
}

impl<A, B, R, C> Future for ThenFactoryResponse<A, B, R, C>
where
    A: ServiceFactory<R, C>,
    B: ServiceFactory<
        Result<A::Response, A::Error>,
        C,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Output = Result<Then<A::Service, B::Service, R>, A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if this.a.is_none() {
            if let Poll::Ready(service) = this.fut_a.poll(cx)? {
                *this.a = Some(service);
            }
        }
        if this.b.is_none() {
            if let Poll::Ready(service) = this.fut_b.poll(cx)? {
                *this.b = Some(service);
            }
        }
        if this.a.is_some() && this.b.is_some() {
            Poll::Ready(Ok(Then::new(
                this.a.take().unwrap(),
                this.b.take().unwrap(),
            )))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use ntex_util::future::{lazy, Ready};
    use std::{cell::Cell, rc::Rc, task::Context, task::Poll};

    use crate::{pipeline, pipeline_factory, Service, ServiceFactory};

    #[derive(Clone)]
    struct Srv1(Rc<Cell<usize>>);

    impl Service<Result<&'static str, &'static str>> for Srv1 {
        type Response = &'static str;
        type Error = ();
        type Future = Ready<Self::Response, Self::Error>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Ok(()))
        }

        fn call(&self, req: Result<&'static str, &'static str>) -> Self::Future {
            match req {
                Ok(msg) => Ready::Ok(msg),
                Err(_) => Ready::Err(()),
            }
        }
    }

    struct Srv2(Rc<Cell<usize>>);

    impl Service<Result<&'static str, ()>> for Srv2 {
        type Response = (&'static str, &'static str);
        type Error = ();
        type Future = Ready<Self::Response, ()>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.0.set(self.0.get() + 1);
            Poll::Ready(Err(()))
        }

        fn call(&self, req: Result<&'static str, ()>) -> Self::Future {
            match req {
                Ok(msg) => Ready::Ok((msg, "ok")),
                Err(()) => Ready::Ok(("srv2", "err")),
            }
        }
    }

    #[ntex::test]
    async fn test_poll_ready() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).then(Srv2(cnt.clone()));
        let res = lazy(|cx| srv.poll_ready(cx)).await;
        assert_eq!(res, Poll::Ready(Err(())));
        assert_eq!(cnt.get(), 2);
        let res = lazy(|cx| srv.poll_shutdown(cx, false)).await;
        assert_eq!(res, Poll::Ready(()));
    }

    #[ntex::test]
    async fn test_call() {
        let cnt = Rc::new(Cell::new(0));
        let srv = pipeline(Srv1(cnt.clone())).then(Srv2(cnt)).clone();

        let res = srv.call(Ok("srv1")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "ok"));

        let res = srv.call(Err("srv")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv2", "err"));
    }

    #[ntex::test]
    async fn test_factory() {
        let cnt = Rc::new(Cell::new(0));
        let cnt2 = cnt.clone();
        let blank = move || Ready::<_, ()>::Ok(Srv1(cnt2.clone()));
        let factory = pipeline_factory(blank)
            .then(move || Ready::Ok(Srv2(cnt.clone())))
            .clone();
        let srv = factory.new_service(&()).await.unwrap();
        let res = srv.call(Ok("srv1")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv1", "ok"));

        let res = srv.call(Err("srv")).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), ("srv2", "err"));
    }
}
