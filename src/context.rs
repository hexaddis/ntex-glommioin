use std;
use std::rc::Rc;
use std::collections::VecDeque;
use futures::{Async, Stream, Poll};
use futures::sync::oneshot::Sender;

use actix::{Actor, ActorState, ActorContext, AsyncContext,
            Handler, Subscriber, ResponseType};
use actix::fut::ActorFuture;
use actix::dev::{AsyncContextApi, ActorAddressCell, ActorItemsCell, ActorWaitCell, SpawnHandle,
                 Envelope, ToEnvelope, RemoteEnvelope};

use body::BinaryBody;
use route::{Route, Frame};
use httpresponse::HttpResponse;


/// Actor execution context
pub struct HttpContext<A> where A: Actor<Context=HttpContext<A>> + Route,
{
    act: Option<A>,
    state: ActorState,
    modified: bool,
    items: ActorItemsCell<A>,
    address: ActorAddressCell<A>,
    stream: VecDeque<Frame>,
    wait: ActorWaitCell<A>,
    app_state: Rc<<A as Route>::State>,
}


impl<A> ActorContext<A> for HttpContext<A> where A: Actor<Context=Self> + Route
{
    /// Stop actor execution
    fn stop(&mut self) {
        self.address.close();
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
        self.write_eof();
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

impl<A> AsyncContext<A> for HttpContext<A> where A: Actor<Context=Self> + Route
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
}

#[doc(hidden)]
impl<A> AsyncContextApi<A> for HttpContext<A> where A: Actor<Context=Self> + Route {
    fn address_cell(&mut self) -> &mut ActorAddressCell<A> {
        &mut self.address
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    pub fn new(state: Rc<<A as Route>::State>) -> HttpContext<A>
    {
        HttpContext {
            act: None,
            state: ActorState::Started,
            modified: false,
            items: ActorItemsCell::default(),
            address: ActorAddressCell::default(),
            wait: ActorWaitCell::default(),
            stream: VecDeque::new(),
            app_state: state,
        }
    }

    pub(crate) fn set_actor(&mut self, act: A) {
        self.act = Some(act)
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    /// Shared application state
    pub fn state(&self) -> &<A as Route>::State {
        &self.app_state
    }
    
    /// Start response processing
    pub fn start<R: Into<HttpResponse>>(&mut self, response: R) {
        self.stream.push_back(Frame::Message(response.into()))
    }

    /// Write payload
    pub fn write<B: Into<BinaryBody>>(&mut self, data: B) {
        self.stream.push_back(Frame::Payload(Some(data.into())))
    }

    /// Indicate end of streamimng payload
    pub fn write_eof(&mut self) {
        self.stream.push_back(Frame::Payload(None))
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

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
impl<A> Stream for HttpContext<A> where A: Actor<Context=Self> + Route
{
    type Item = Frame;
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Option<Frame>, std::io::Error> {
        if self.act.is_none() {
            return Ok(Async::NotReady)
        }

        let act: &mut A = unsafe {
            std::mem::transmute(self.act.as_mut().unwrap() as &mut A)
        };
        let ctx: &mut HttpContext<A> = unsafe {
            std::mem::transmute(self as &mut HttpContext<A>)
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

impl<A> ToEnvelope<A> for HttpContext<A>
    where A: Actor<Context=HttpContext<A>> + Route,
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
