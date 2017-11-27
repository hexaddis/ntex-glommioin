use std::rc::Rc;
use std::cell::RefCell;
use std::marker::PhantomData;

use actix::Actor;
use http::{header, Version};
use futures::Stream;

use task::{Task, DrainFut};
use body::Binary;
use error::{Error, ExpectError};
use context::HttpContext;
use resource::Reply;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

#[doc(hidden)]
#[derive(Debug)]
#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
pub enum Frame {
    Message(HttpResponse),
    Payload(Option<Binary>),
    Drain(Rc<RefCell<DrainFut>>),
}

impl Frame {
    pub fn eof() -> Frame {
        Frame::Payload(None)
    }
}

/// Trait defines object that could be regestered as resource route
#[allow(unused_variables)]
pub trait RouteHandler<S>: 'static {
    /// Handle request
    fn handle(&self, req: HttpRequest<S>) -> Task;

    /// Set route prefix
    fn set_prefix(&mut self, prefix: String) {}
}

/// Request handling result.
pub type RouteResult<T> = Result<Reply<T>, Error>;

/// Actors with ability to handle http requests.
#[allow(unused_variables)]
pub trait Route: Actor {
    /// Shared state. State is shared with all routes within same application
    /// and could be accessed with `HttpRequest::state()` method.
    type State;

    /// Handle `EXPECT` header. By default respones with `HTTP/1.1 100 Continue`
    fn expect(req: &mut HttpRequest<Self::State>, ctx: &mut Self::Context) -> Result<(), Error>
        where Self: Actor<Context=HttpContext<Self>>
    {
        // handle expect header only for HTTP/1.1
        if req.version() == Version::HTTP_11 {
            if let Some(expect) = req.headers().get(header::EXPECT) {
                if let Ok(expect) = expect.to_str() {
                    if expect.to_lowercase() == "100-continue" {
                        ctx.write("HTTP/1.1 100 Continue\r\n\r\n");
                        Ok(())
                    } else {
                        Err(ExpectError::UnknownExpect.into())
                    }
                } else {
                    Err(ExpectError::Encoding.into())
                }
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    /// Handle incoming request. Route actor can return
    /// result immediately with `Reply::reply`.
    /// Actor itself can be returned with `Reply::stream` for handling streaming
    /// request/response or websocket connection.
    /// In that case `HttpContext::start` and `HttpContext::write` has to be used
    /// for writing response.
    fn request(req: HttpRequest<Self::State>, ctx: &mut Self::Context) -> RouteResult<Self>;

    /// This method creates `RouteFactory` for this actor.
    fn factory() -> RouteFactory<Self, Self::State> {
        RouteFactory(PhantomData)
    }
}

/// This is used for routes registration within `Resource`
pub struct RouteFactory<A: Route<State=S>, S>(PhantomData<A>);

impl<A, S> RouteHandler<S> for RouteFactory<A, S>
    where A: Actor<Context=HttpContext<A>> + Route<State=S>,
          S: 'static
{
    fn handle(&self, mut req: HttpRequest<A::State>) -> Task {
        let mut ctx = HttpContext::new(req.clone_state());

        // handle EXPECT header
        if req.headers().contains_key(header::EXPECT) {
            if let Err(resp) = A::expect(&mut req, &mut ctx) {
                return Task::reply(resp)
            }
        }
        match A::request(req, &mut ctx) {
            Ok(reply) => reply.into(ctx),
            Err(err) => Task::reply(err),
        }
    }
}

/// Fn() route handler
pub(crate)
struct FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<HttpResponse>,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<HttpResponse> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        FnHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<HttpResponse> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>) -> Task {
        Task::reply((self.f)(req).into())
    }
}

/// Async route handler
pub(crate)
struct StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        StreamHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>) -> Task {
        Task::with_stream((self.f)(req))
    }
}
