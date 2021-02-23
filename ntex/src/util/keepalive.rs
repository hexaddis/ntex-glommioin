use std::cell::RefCell;
use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::{ok, Ready};

use crate::rt::time::{delay_until, Delay, Instant};
use crate::{Service, ServiceFactory};

use super::time::{LowResTime, LowResTimeService};

/// KeepAlive service factory
///
/// Controls min time between requests.
pub struct KeepAlive<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTime,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    /// Construct KeepAlive service factory.
    ///
    /// ka - keep-alive timeout
    /// err - error factory function
    pub fn new(ka: Duration, time: LowResTime, err: F) -> Self {
        KeepAlive {
            ka,
            time,
            f: err,
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Clone for KeepAlive<R, E, F>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        KeepAlive {
            f: self.f.clone(),
            ka: self.ka,
            time: self.time.clone(),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> ServiceFactory for KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type InitError = Infallible;
    type Config = ();
    type Service = KeepAliveService<R, E, F>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        ok(KeepAliveService::new(
            self.ka,
            self.time.timer(),
            self.f.clone(),
        ))
    }
}

pub struct KeepAliveService<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTimeService,
    inner: RefCell<Inner>,
    _t: PhantomData<(R, E)>,
}

struct Inner {
    delay: Pin<Box<Delay>>,
    expire: Instant,
}

impl<R, E, F> KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    pub fn new(ka: Duration, time: LowResTimeService, f: F) -> Self {
        let expire = Instant::from_std(time.now() + ka);
        KeepAliveService {
            f,
            ka,
            time,
            inner: RefCell::new(Inner {
                expire,
                delay: Box::pin(delay_until(expire)),
            }),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Service for KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type Future = Ready<Result<R, E>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut inner = self.inner.borrow_mut();

        match Pin::new(&mut inner.delay).poll(cx) {
            Poll::Ready(_) => {
                let now = Instant::from_std(self.time.now());
                if inner.expire <= now {
                    Poll::Ready(Err((self.f)()))
                } else {
                    let expire = inner.expire;
                    inner.delay.as_mut().reset(expire);
                    let _ = Pin::new(&mut inner.delay).poll(cx);
                    Poll::Ready(Ok(()))
                }
            }
            Poll::Pending => Poll::Ready(Ok(())),
        }
    }

    fn call(&self, req: R) -> Self::Future {
        self.inner.borrow_mut().expire = Instant::from_std(self.time.now() + self.ka);
        ok(req)
    }
}

#[cfg(test)]
mod tests {
    use futures::future::lazy;

    use super::*;
    use crate::rt::time::delay_for;
    use crate::service::{Service, ServiceFactory};

    #[derive(Debug, PartialEq)]
    struct TestErr;

    #[ntex_rt::test]
    async fn test_ka() {
        let factory = KeepAlive::new(
            Duration::from_millis(100),
            LowResTime::with(Duration::from_millis(10)),
            || TestErr,
        );
        let _ = factory.clone();

        let service = factory.new_service(()).await.unwrap();

        assert_eq!(service.call(1usize).await, Ok(1usize));
        assert!(lazy(|cx| service.poll_ready(cx)).await.is_ready());

        delay_for(Duration::from_millis(500)).await;
        assert_eq!(
            lazy(|cx| service.poll_ready(cx)).await,
            Poll::Ready(Err(TestErr))
        );
    }
}
