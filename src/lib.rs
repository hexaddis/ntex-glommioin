//! Actix http framework

#[macro_use]
extern crate log;
extern crate time;
extern crate bytes;
extern crate sha1;
extern crate url;
#[macro_use]
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
#[macro_use]
extern crate hyper;
extern crate unicase;

extern crate http;
extern crate httparse;
extern crate route_recognizer;
extern crate actix;

mod application;
mod context;
mod error;
mod date;
mod decode;
mod httpmessage;
mod resource;
mod route;
mod router;
mod task;
mod reader;
mod server;

pub mod ws;
mod wsframe;
mod wsproto;

pub mod httpcodes;
pub use application::Application;
pub use httpmessage::{HttpRequest, HttpResponse, IntoHttpResponse};
pub use router::RoutingMap;
pub use resource::{Reply, Resource};
pub use route::{Route, RouteFactory, RouteHandler, Payload, PayloadItem};
pub use server::HttpServer;
pub use context::HttpContext;
pub use route_recognizer::Params;
