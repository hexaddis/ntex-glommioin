use std;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use futures::{Async, Future, Stream, Poll};
use futures::sync::oneshot::Sender;

use actix::{Actor, ActorState, ActorContext, AsyncContext,
            Handler, Subscriber, ResponseType};
use actix::fut::ActorFuture;
use actix::dev::{AsyncContextApi, ActorAddressCell, ActorItemsCell, ActorWaitCell, SpawnHandle,
                 Envelope, ToEnvelope, RemoteEnvelope};

use task::{IoContext, DrainFut};
use body::Binary;
use error::Error;
use route::Frame;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;


/// Http actor execution context
pub struct HttpContext<A, S=()> where A: Actor<Context=HttpContext<A, S>>,
{
    act: A,
    state: ActorState,
    modified: bool,
    items: ActorItemsCell<A>,
    address: ActorAddressCell<A>,
    stream: VecDeque<Frame>,
    wait: ActorWaitCell<A>,
    request: HttpRequest<S>,
    disconnected: bool,
}

impl<A, S> IoContext for HttpContext<A, S> where A: Actor<Context=Self>, S: 'static {

    fn disconnected(&mut self) {
        self.items.stop();
        self.disconnected = true;
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
    }
}

impl<A, S> ActorContext for HttpContext<A, S> where A: Actor<Context=Self>
{
    /// Stop actor execution
    fn stop(&mut self) {
        self.stream.push_back(Frame::Payload(None));
        self.items.stop();
        self.address.close();
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
    }

    /// Terminate actor execution
    fn terminate(&mut self) {
        self.address.close();
        self.items.close();
        self.state = ActorState::Stopped;
    }

    /// Actor execution state
    fn state(&self) -> ActorState {
        self.state
    }
}

impl<A, S> AsyncContext<A> for HttpContext<A, S> where A: Actor<Context=Self>
{
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.modified = true;
        self.items.spawn(fut)
    }

    fn wait<F>(&mut self, fut: F)
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.modified = true;
        self.wait.add(fut);
    }

    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.modified = true;
        self.items.cancel_future(handle)
    }

    fn cancel_future_on_stop(&mut self, handle: SpawnHandle) {
        self.items.cancel_future_on_stop(handle)
    }
}

#[doc(hidden)]
impl<A, S> AsyncContextApi<A> for HttpContext<A, S> where A: Actor<Context=Self> {
    fn address_cell(&mut self) -> &mut ActorAddressCell<A> {
        &mut self.address
    }
}

impl<A, S> HttpContext<A, S> where A: Actor<Context=Self> {

    pub fn new(req: HttpRequest<S>, actor: A) -> HttpContext<A, S>
    {
        HttpContext {
            act: actor,
            state: ActorState::Started,
            modified: false,
            items: ActorItemsCell::default(),
            address: ActorAddressCell::default(),
            wait: ActorWaitCell::default(),
            stream: VecDeque::new(),
            request: req,
            disconnected: false,
        }
    }
}

impl<A, S> HttpContext<A, S> where A: Actor<Context=Self> {

    /// Shared application state
    pub fn state(&self) -> &S {
        self.request.state()
    }

    /// Incoming request
    pub fn request(&mut self) -> &mut HttpRequest<S> {
        &mut self.request
    }


    /// Start response processing
    pub fn start<R: Into<HttpResponse>>(&mut self, response: R) {
        self.stream.push_back(Frame::Message(response.into()))
    }

    /// Write payload
    pub fn write<B: Into<Binary>>(&mut self, data: B) {
        self.stream.push_back(Frame::Payload(Some(data.into())))
    }

    /// Indicate end of streamimng payload. Also this method calls `Self::close`.
    pub fn write_eof(&mut self) {
        self.stop();
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let fut = Rc::new(RefCell::new(DrainFut::default()));
        self.stream.push_back(Frame::Drain(Rc::clone(&fut)));
        self.modified = true;
        Drain{ a: PhantomData, inner: fut }
    }

    /// Check if connection still open
    pub fn connected(&self) -> bool {
        !self.disconnected
    }
}

impl<A, S> HttpContext<A, S> where A: Actor<Context=Self> {

    #[doc(hidden)]
    pub fn subscriber<M>(&mut self) -> Box<Subscriber<M>>
        where A: Handler<M>,
              M: ResponseType + 'static,
    {
        Box::new(self.address.unsync_address())
    }

    #[doc(hidden)]
    pub fn sync_subscriber<M>(&mut self) -> Box<Subscriber<M> + Send>
        where A: Handler<M>,
              M: ResponseType + Send + 'static,
              M::Item: Send,
              M::Error: Send,
    {
        Box::new(self.address.sync_address())
    }
}

#[doc(hidden)]
impl<A, S> Stream for HttpContext<A, S> where A: Actor<Context=Self>
{
    type Item = Frame;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Frame>, Error> {
        let act: &mut A = unsafe {
            std::mem::transmute(&mut self.act as &mut A)
        };
        let ctx: &mut HttpContext<A, S> = unsafe {
            std::mem::transmute(self as &mut HttpContext<A, S>)
        };

        // update state
        match self.state {
            ActorState::Started => {
                Actor::started(act, ctx);
                self.state = ActorState::Running;
            },
            ActorState::Stopping => {
                Actor::stopping(act, ctx);
            }
            _ => ()
        }

        let mut prep_stop = false;
        loop {
            self.modified = false;

            // check wait futures
            if self.wait.poll(act, ctx) {
                // get frame
                if let Some(frame) = self.stream.pop_front() {
                    return Ok(Async::Ready(Some(frame)))
                }
                return Ok(Async::NotReady)
            }

            // incoming messages
            self.address.poll(act, ctx);

            // spawned futures and streams
            self.items.poll(act, ctx);

            // are we done
            if self.modified {
                continue
            }

            // get frame
            if let Some(frame) = self.stream.pop_front() {
                return Ok(Async::Ready(Some(frame)))
            }

            // check state
            match self.state {
                ActorState::Stopped => {
                    self.state = ActorState::Stopped;
                    Actor::stopped(act, ctx);
                    return Ok(Async::Ready(None))
                },
                ActorState::Stopping => {
                    if prep_stop {
                        if self.address.connected() || !self.items.is_empty() {
                            self.state = ActorState::Running;
                            continue
                        } else {
                            self.state = ActorState::Stopped;
                            Actor::stopped(act, ctx);
                            return Ok(Async::Ready(None))
                        }
                    } else {
                        Actor::stopping(act, ctx);
                        prep_stop = true;
                        continue
                    }
                },
                ActorState::Running => {
                    if !self.address.connected() && self.items.is_empty() {
                        self.state = ActorState::Stopping;
                        Actor::stopping(act, ctx);
                        prep_stop = true;
                        continue
                    }
                },
                _ => (),
            }

            return Ok(Async::NotReady)
        }
    }
}

impl<A, S> ToEnvelope<A> for HttpContext<A, S>
    where A: Actor<Context=HttpContext<A, S>>,
{
    fn pack<M>(msg: M, tx: Option<Sender<Result<M::Item, M::Error>>>) -> Envelope<A>
        where A: Handler<M>,
              M: ResponseType + Send + 'static,
              M::Item: Send,
              M::Error: Send
    {
        RemoteEnvelope::new(msg, tx).into()
    }
}


pub struct Drain<A> {
    a: PhantomData<A>,
    inner: Rc<RefCell<DrainFut>>
}

impl<A> ActorFuture for Drain<A>
    where A: Actor
{
    type Item = ();
    type Error = ();
    type Actor = A;

    fn poll(&mut self, _: &mut A, _: &mut <Self::Actor as Actor>::Context) -> Poll<(), ()> {
        self.inner.borrow_mut().poll()
    }
}
