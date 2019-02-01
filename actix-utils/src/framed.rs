//! Framed dispatcher service and related utilities
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::mem;

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed};
use actix_service::{IntoNewService, IntoService, NewService, Service};
use futures::future::{ok, FutureResult};
use futures::task::AtomicTask;
use futures::{Async, Future, Poll, Sink, Stream};
use log::debug;

use crate::cell::Cell;

type Request<U> = <U as Decoder>::Item;
type Response<U> = <U as Encoder>::Item;

pub struct FramedNewService<S, T, U> {
    factory: S,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> FramedNewService<S, T, U>
where
    S: NewService<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    <S::Service as Service<Request<U>>>::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    pub fn new<F1: IntoNewService<S, Request<U>>>(factory: F1) -> Self {
        Self {
            factory: factory.into_new_service(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> Clone for FramedNewService<S, T, U>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> NewService<Framed<T, U>> for FramedNewService<S, T, U>
where
    S: NewService<Request<U>, Response = Response<U>> + Clone,
    S::Error: 'static,
    <S::Service as Service<Request<U>>>::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    type Response = FramedTransport<S::Service, T, U>;
    type Error = S::InitError;
    type InitError = S::InitError;
    type Service = FramedService<S, T, U>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(FramedService {
            factory: self.factory.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedService<S, T, U> {
    factory: S,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> Clone for FramedService<S, T, U>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> Service<Framed<T, U>> for FramedService<S, T, U>
where
    S: NewService<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    <S::Service as Service<Request<U>>>::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    type Response = FramedTransport<S::Service, T, U>;
    type Error = S::InitError;
    type Future = FramedServiceResponseFuture<S, T, U>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Framed<T, U>) -> Self::Future {
        FramedServiceResponseFuture {
            fut: self.factory.new_service(),

            framed: Some(req),
        }
    }
}

#[doc(hidden)]
pub struct FramedServiceResponseFuture<S, T, U>
where
    S: NewService<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    <S::Service as Service<Request<U>>>::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    fut: S::Future,
    framed: Option<Framed<T, U>>,
}

impl<S, T, U> Future for FramedServiceResponseFuture<S, T, U>
where
    S: NewService<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    <S::Service as Service<Request<U>>>::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    type Item = FramedTransport<S::Service, T, U>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(service) => Ok(Async::Ready(FramedTransport::new(
                self.framed.take().unwrap(),
                service,
            ))),
        }
    }
}

/// Framed transport errors
pub enum FramedTransportError<E, U: Encoder + Decoder> {
    Service(E),
    Encoder(<U as Encoder>::Error),
    Decoder(<U as Decoder>::Error),
}

impl<E, U: Encoder + Decoder> From<E> for FramedTransportError<E, U> {
    fn from(err: E) -> Self {
        FramedTransportError::Service(err)
    }
}

/// FramedTransport - is a future that reads frames from Framed object
/// and pass then to the service.
pub struct FramedTransport<S, T, U>
where
    S: Service<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Encoder + Decoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    service: S,
    state: TransportState<S, U>,
    framed: Framed<T, U>,
    inner: Cell<FramedTransportInner<<U as Encoder>::Item, S::Error>>,
}

enum TransportState<S: Service<Request<U>>, U: Encoder + Decoder> {
    Processing,
    Error(FramedTransportError<S::Error, U>),
    FramedError(FramedTransportError<S::Error, U>),
    Stopping,
}

struct FramedTransportInner<I, E> {
    buf: VecDeque<Result<I, E>>,
    task: AtomicTask,
}

impl<S, T, U> FramedTransport<S, T, U>
where
    S: Service<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    fn poll_read(&mut self) -> bool {
        loop {
            match self.service.poll_ready() {
                Ok(Async::Ready(_)) => loop {
                    let item = match self.framed.poll() {
                        Ok(Async::Ready(Some(el))) => el,
                        Err(err) => {
                            self.state =
                                TransportState::FramedError(FramedTransportError::Decoder(err));
                            return true;
                        }
                        Ok(Async::NotReady) => return false,
                        Ok(Async::Ready(None)) => {
                            self.state = TransportState::Stopping;
                            return true;
                        }
                    };

                    let mut cell = self.inner.clone();
                    cell.get_mut().task.register();
                    tokio_current_thread::spawn(self.service.call(item).then(move |item| {
                        let inner = cell.get_mut();
                        inner.buf.push_back(item);
                        inner.task.notify();
                        Ok(())
                    }));
                },
                Ok(Async::NotReady) => return false,
                Err(err) => {
                    self.state = TransportState::Error(FramedTransportError::Service(err));
                    return true;
                }
            }
        }
    }

    /// write to framed object
    fn poll_write(&mut self) -> bool {
        let inner = self.inner.get_mut();
        loop {
            while !self.framed.is_write_buf_full() {
                if let Some(msg) = inner.buf.pop_front() {
                    match msg {
                        Ok(msg) => {
                            if let Err(err) = self.framed.force_send(msg) {
                                self.state = TransportState::FramedError(
                                    FramedTransportError::Encoder(err),
                                );
                                return true;
                            }
                        }
                        Err(err) => {
                            self.state =
                                TransportState::Error(FramedTransportError::Service(err));
                            return true;
                        }
                    }
                } else {
                    break;
                }
            }

            if !self.framed.is_write_buf_empty() {
                match self.framed.poll_complete() {
                    Ok(Async::NotReady) => break,
                    Err(err) => {
                        debug!("Error sending data: {:?}", err);
                        self.state =
                            TransportState::FramedError(FramedTransportError::Encoder(err));
                        return true;
                    }
                    Ok(Async::Ready(_)) => (),
                }
            } else {
                break;
            }
        }

        false
    }
}

impl<S, T, U> FramedTransport<S, T, U>
where
    S: Service<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    pub fn new<F: IntoService<S, Request<U>>>(framed: Framed<T, U>, service: F) -> Self {
        FramedTransport {
            framed,
            service: service.into_service(),
            state: TransportState::Processing,
            inner: Cell::new(FramedTransportInner {
                buf: VecDeque::new(),
                task: AtomicTask::new(),
            }),
        }
    }

    /// Get reference to a service wrapped by `FramedTransport` instance.
    pub fn get_ref(&self) -> &S {
        &self.service
    }

    /// Get mutable reference to a service wrapped by `FramedTransport`
    /// instance.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.service
    }

    /// Get reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed(&self) -> &Framed<T, U> {
        &self.framed
    }

    /// Get mutable reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed_mut(&mut self) -> &mut Framed<T, U> {
        &mut self.framed
    }
}

impl<S, T, U> Future for FramedTransport<S, T, U>
where
    S: Service<Request<U>, Response = Response<U>>,
    S::Error: 'static,
    S::Future: 'static,
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: std::fmt::Debug,
{
    type Item = ();
    type Error = FramedTransportError<S::Error, U>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match mem::replace(&mut self.state, TransportState::Processing) {
            TransportState::Processing => {
                if self.poll_read() || self.poll_write() {
                    self.poll()
                } else {
                    Ok(Async::NotReady)
                }
            }
            TransportState::Error(err) => {
                if self.framed.is_write_buf_empty()
                    || (self.poll_write() || self.framed.is_write_buf_empty())
                {
                    Err(err)
                } else {
                    self.state = TransportState::Error(err);
                    Ok(Async::NotReady)
                }
            }
            TransportState::FramedError(err) => Err(err),
            TransportState::Stopping => Ok(Async::Ready(())),
        }
    }
}

pub struct IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    factory: F,
    _t: PhantomData<(T,)>,
}

impl<T, U, F> IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    pub fn new(factory: F) -> Self {
        IntoFramed {
            factory,
            _t: PhantomData,
        }
    }
}

impl<T, U, F> NewService<T> for IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    type Response = Framed<T, U>;
    type Error = ();
    type InitError = ();
    type Service = IntoFramedService<T, U, F>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(IntoFramedService {
            factory: self.factory.clone(),
            _t: PhantomData,
        })
    }
}

pub struct IntoFramedService<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    factory: F,
    _t: PhantomData<(T,)>,
}

impl<T, U, F> Service<T> for IntoFramedService<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    type Response = Framed<T, U>;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: T) -> Self::Future {
        ok(Framed::new(req, (self.factory)()))
    }
}
