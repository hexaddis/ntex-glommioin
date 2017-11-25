//! Middlewares
use futures::Future;

use error::Error;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

mod logger;
mod session;
pub use self::logger::Logger;
pub use self::session::{RequestSession, Session, SessionImpl,
                        SessionBackend, SessionStorage, CookieSessionBackend};

/// Middleware start result
pub enum Started {
    /// Moddleware error
    Err(Error),
    /// Execution completed
    Done(HttpRequest),
    /// New http response got generated. If middleware generates response
    /// handler execution halts.
    Response(HttpRequest, HttpResponse),
    /// Execution completed, runs future to completion.
    Future(Box<Future<Item=(HttpRequest, Option<HttpResponse>), Error=Error>>),
}

/// Middleware execution result
pub enum Response {
    /// Moddleware error
    Err(Error),
    /// New http response got generated
    Done(HttpResponse),
    /// Result is a future that resolves to a new http response
    Future(Box<Future<Item=HttpResponse, Error=Error>>),
}

/// Middleware finish result
pub enum Finished {
    /// Execution completed
    Done,
    /// Execution completed, but run future to completion
    Future(Box<Future<Item=(), Error=Error>>),
}

/// Middleware definition
#[allow(unused_variables)]
pub trait Middleware {

    /// Method is called when request is ready. It may return
    /// future, which should resolve before next middleware get called.
    fn start(&self, req: HttpRequest) -> Started {
        Started::Done(req)
    }

    /// Method is called when handler returns response,
    /// but before sending http message to peer.
    fn response(&self, req: &mut HttpRequest, resp: HttpResponse) -> Response {
        Response::Done(resp)
    }

    /// Method is called after body stream get sent to peer.
    fn finish(&self, req: &mut HttpRequest, resp: &HttpResponse) -> Finished {
        Finished::Done
    }
}
