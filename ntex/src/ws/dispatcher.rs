use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::codec::{AsyncRead, AsyncWrite, Framed};
use crate::framed;
use crate::service::{IntoService, Service};

use super::{Codec, Frame, Message};

/// WebSockets protocol dispatcher
pub struct Dispatcher<S, T>
where
    S: Service<Request = Frame, Response = Message> + 'static,
    T: AsyncRead + AsyncWrite,
{
    inner: framed::Dispatcher<S, T, Codec>,
}

impl<S, T> Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    pub fn new<F: IntoService<S>>(io: T, service: F) -> Self {
        Dispatcher {
            inner: framed::Dispatcher::new(Framed::new(io, Codec::new()), service),
        }
    }

    pub fn with<F: IntoService<S>>(framed: Framed<T, Codec>, service: F) -> Self {
        Dispatcher {
            inner: framed::Dispatcher::new(framed, service),
        }
    }
}

impl<S, T> Future for Dispatcher<S, T>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Frame, Response = Message>,
    S::Future: 'static,
    S::Error: 'static,
{
    type Output = Result<(), framed::ServiceError<S::Error, Codec>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx)
    }
}
