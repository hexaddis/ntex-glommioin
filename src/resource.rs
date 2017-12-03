use std::marker::PhantomData;
use std::collections::HashMap;

use http::Method;
use futures::Future;

use error::Error;
use route::{Reply, Handler, FromRequest, RouteHandler, AsyncHandler, WrapHandler};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::{HTTPNotFound, HTTPMethodNotAllowed};

/// Http resource
///
/// `Resource` is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route corresponds to handling HTTP method by calling route handler.
///
/// ```rust
/// extern crate actix_web;
///
/// fn main() {
///     let app = actix_web::Application::default("/")
///         .resource("/", |r| r.get(|_| actix_web::HttpResponse::Ok()))
///         .finish();
/// }
pub struct Resource<S=()> {
    name: String,
    state: PhantomData<S>,
    routes: HashMap<Method, Box<RouteHandler<S>>>,
    default: Box<RouteHandler<S>>,
}

impl<S> Default for Resource<S> {
    fn default() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPMethodNotAllowed)}
    }
}

impl<S> Resource<S> where S: 'static {

    pub(crate) fn default_not_found() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPNotFound)}
    }

    /// Set resource name
    pub fn name<T: Into<String>>(&mut self, name: T) {
        self.name = name.into();
    }

    /// Register handler for specified method.
    pub fn handler<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: FromRequest + 'static,
    {
        self.routes.insert(method, Box::new(WrapHandler::new(handler)));
    }

    /// Register async handler for specified method.
    pub fn async<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Future<Item=HttpResponse, Error=Error> + 'static,
    {
        self.routes.insert(method, Box::new(AsyncHandler::new(handler)));
    }

    /// Default handler is used if no matched route found.
    /// By default `HTTPMethodNotAllowed` is used.
    pub fn default_handler<H>(&mut self, handler: H) where H: Handler<S>
    {
        self.default = Box::new(WrapHandler::new(handler));
    }

    /// Register handler for `GET` method.
    pub fn get<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: FromRequest + 'static, {
        self.routes.insert(Method::GET, Box::new(WrapHandler::new(handler)));
    }

    /// Register handler for `POST` method.
    pub fn post<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: FromRequest + 'static, {
        self.routes.insert(Method::POST, Box::new(WrapHandler::new(handler)));
    }

    /// Register handler for `PUT` method.
    pub fn put<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: FromRequest + 'static, {
        self.routes.insert(Method::PUT, Box::new(WrapHandler::new(handler)));
    }

    /// Register handler for `DELETE` method.
    pub fn delete<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: FromRequest + 'static, {
        self.routes.insert(Method::DELETE, Box::new(WrapHandler::new(handler)));
    }
}

impl<S: 'static> RouteHandler<S> for Resource<S> {

    fn handle(&self, req: HttpRequest<S>) -> Reply {
        if let Some(handler) = self.routes.get(req.method()) {
            handler.handle(req)
        } else {
            self.default.handle(req)
        }
    }
}
