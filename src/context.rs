extern crate actix;

use futures::sync::oneshot;
use futures::sync::oneshot::Sender;
use futures::{Async, Future, Poll};
use smallvec::SmallVec;
use std::marker::PhantomData;

use self::actix::dev::{
    AsyncContextParts, ContextFut, ContextParts, Envelope, Mailbox, ToEnvelope,
};
use self::actix::fut::ActorFuture;
use self::actix::{
    Actor, ActorContext, ActorState, Addr, AsyncContext, Handler, Message, SpawnHandle,
};

use body::{Binary, Body};
use error::{Error, ErrorInternalServerError};
use httprequest::HttpRequest;

pub trait ActorHttpContext: 'static {
    fn disconnected(&mut self);
    fn poll(&mut self) -> Poll<Option<SmallVec<[Frame; 4]>>, Error>;
}

#[derive(Debug)]
pub enum Frame {
    Chunk(Option<Binary>),
    Drain(oneshot::Sender<()>),
}

impl Frame {
    pub fn len(&self) -> usize {
        match *self {
            Frame::Chunk(Some(ref bin)) => bin.len(),
            _ => 0,
        }
    }
}

/// Execution context for http actors
pub struct HttpContext<A, S = ()>
where
    A: Actor<Context = HttpContext<A, S>>,
{
    inner: ContextParts<A>,
    stream: Option<SmallVec<[Frame; 4]>>,
    request: HttpRequest<S>,
    disconnected: bool,
}

impl<A, S> ActorContext for HttpContext<A, S>
where
    A: Actor<Context = Self>,
{
    fn stop(&mut self) {
        self.inner.stop();
    }
    fn terminate(&mut self) {
        self.inner.terminate()
    }
    fn state(&self) -> ActorState {
        self.inner.state()
    }
}

impl<A, S> AsyncContext<A> for HttpContext<A, S>
where
    A: Actor<Context = Self>,
{
    #[inline]
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.spawn(fut)
    }
    #[inline]
    fn wait<F>(&mut self, fut: F)
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.wait(fut)
    }
    #[doc(hidden)]
    #[inline]
    fn waiting(&self) -> bool {
        self.inner.waiting()
            || self.inner.state() == ActorState::Stopping
            || self.inner.state() == ActorState::Stopped
    }
    #[inline]
    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.inner.cancel_future(handle)
    }
    #[inline]
    fn address(&self) -> Addr<A> {
        self.inner.address()
    }
}

impl<A, S: 'static> HttpContext<A, S>
where
    A: Actor<Context = Self>,
{
    #[inline]
    /// Create a new HTTP Context from a request and an actor
    pub fn create(req: HttpRequest<S>, actor: A) -> Body {
        let mb = Mailbox::default();
        let ctx = HttpContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: None,
            request: req,
            disconnected: false,
        };
        Body::Actor(Box::new(HttpContextFut::new(ctx, actor, mb)))
    }

    /// Create a new HTTP Context
    pub fn with_factory<F>(req: HttpRequest<S>, f: F) -> Body
    where
        F: FnOnce(&mut Self) -> A + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = HttpContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: None,
            request: req,
            disconnected: false,
        };

        let act = f(&mut ctx);
        Body::Actor(Box::new(HttpContextFut::new(ctx, act, mb)))
    }
}

impl<A, S> HttpContext<A, S>
where
    A: Actor<Context = Self>,
{
    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.request.state()
    }

    /// Incoming request
    #[inline]
    pub fn request(&mut self) -> &mut HttpRequest<S> {
        &mut self.request
    }

    /// Write payload
    #[inline]
    pub fn write<B: Into<Binary>>(&mut self, data: B) {
        if !self.disconnected {
            self.add_frame(Frame::Chunk(Some(data.into())));
        } else {
            warn!("Trying to write to disconnected response");
        }
    }

    /// Indicate end of streaming payload. Also this method calls `Self::close`.
    #[inline]
    pub fn write_eof(&mut self) {
        self.add_frame(Frame::Chunk(None));
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let (tx, rx) = oneshot::channel();
        self.add_frame(Frame::Drain(tx));
        Drain::new(rx)
    }

    /// Check if connection still open
    #[inline]
    pub fn connected(&self) -> bool {
        !self.disconnected
    }

    #[inline]
    fn add_frame(&mut self, frame: Frame) {
        if self.stream.is_none() {
            self.stream = Some(SmallVec::new());
        }
        if let Some(s) = self.stream.as_mut() {
            s.push(frame)
        }
    }

    /// Handle of the running future
    ///
    /// SpawnHandle is the handle returned by `AsyncContext::spawn()` method.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.curr_handle()
    }
}

impl<A, S> AsyncContextParts<A> for HttpContext<A, S>
where
    A: Actor<Context = Self>,
{
    fn parts(&mut self) -> &mut ContextParts<A> {
        &mut self.inner
    }
}

struct HttpContextFut<A, S>
where
    A: Actor<Context = HttpContext<A, S>>,
{
    fut: ContextFut<A, HttpContext<A, S>>,
}

impl<A, S> HttpContextFut<A, S>
where
    A: Actor<Context = HttpContext<A, S>>,
{
    fn new(ctx: HttpContext<A, S>, act: A, mailbox: Mailbox<A>) -> Self {
        let fut = ContextFut::new(ctx, act, mailbox);
        HttpContextFut { fut }
    }
}

impl<A, S> ActorHttpContext for HttpContextFut<A, S>
where
    A: Actor<Context = HttpContext<A, S>>,
    S: 'static,
{
    #[inline]
    fn disconnected(&mut self) {
        self.fut.ctx().disconnected = true;
        self.fut.ctx().stop();
    }

    fn poll(&mut self) -> Poll<Option<SmallVec<[Frame; 4]>>, Error> {
        if self.fut.alive() {
            match self.fut.poll() {
                Ok(Async::NotReady) | Ok(Async::Ready(())) => (),
                Err(_) => return Err(ErrorInternalServerError("error")),
            }
        }

        // frames
        if let Some(data) = self.fut.ctx().stream.take() {
            Ok(Async::Ready(Some(data)))
        } else if self.fut.alive() {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<A, M, S> ToEnvelope<A, M> for HttpContext<A, S>
where
    A: Actor<Context = HttpContext<A, S>> + Handler<M>,
    M: Message + Send + 'static,
    M::Result: Send,
{
    fn pack(msg: M, tx: Option<Sender<M::Result>>) -> Envelope<A> {
        Envelope::new(msg, tx)
    }
}

/// Consume a future
pub struct Drain<A> {
    fut: oneshot::Receiver<()>,
    _a: PhantomData<A>,
}

impl<A> Drain<A> {
    /// Create a drain from a future
    pub fn new(fut: oneshot::Receiver<()>) -> Self {
        Drain {
            fut,
            _a: PhantomData,
        }
    }
}

impl<A: Actor> ActorFuture for Drain<A> {
    type Item = ();
    type Error = ();
    type Actor = A;

    #[inline]
    fn poll(
        &mut self, _: &mut A, _: &mut <Self::Actor as Actor>::Context,
    ) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(|_| ())
    }
}
