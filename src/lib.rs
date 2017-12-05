//! Web framework for [Actix](https://github.com/actix/actix)

#![cfg_attr(actix_nightly, feature(
    specialization, // for impl ErrorResponse for std::error::Error
))]

#[macro_use]
extern crate log;
extern crate time;
extern crate bytes;
extern crate sha1;
extern crate regex;
#[macro_use]
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;

extern crate failure;
#[macro_use] extern crate failure_derive;

extern crate cookie;
extern crate http;
extern crate httparse;
extern crate http_range;
extern crate mime;
extern crate mime_guess;
extern crate url;
extern crate libc;
extern crate serde;
extern crate serde_json;
extern crate flate2;
extern crate brotli2;
extern crate percent_encoding;
extern crate actix;
extern crate h2 as http2;

#[cfg(test)]
#[macro_use] extern crate serde_derive;

#[cfg(feature="tls")]
extern crate native_tls;
#[cfg(feature="tls")]
extern crate tokio_tls;

#[cfg(feature="openssl")]
extern crate openssl;
#[cfg(feature="openssl")]
extern crate tokio_openssl;

mod application;
mod body;
mod context;
mod date;
mod encoding;
mod httprequest;
mod httpresponse;
mod payload;
mod route;
mod resource;
mod recognizer;
mod handler;
mod pipeline;
mod server;
mod channel;
mod wsframe;
mod wsproto;
mod h1;
mod h2;
mod h1writer;
mod h2writer;

pub mod fs;
pub mod ws;
pub mod dev;
pub mod error;
pub mod httpcodes;
pub mod multipart;
pub mod middlewares;
pub mod pred;
pub use error::{Error, Result};
pub use encoding::ContentEncoding;
pub use body::{Body, Binary};
pub use application::Application;
pub use httprequest::{HttpRequest, UrlEncoded};
pub use httpresponse::HttpResponse;
pub use payload::{Payload, PayloadItem};
pub use handler::{Reply, Json, FromRequest};
pub use route::Route;
pub use resource::Resource;
pub use recognizer::Params;
pub use server::HttpServer;
pub use context::HttpContext;

// re-exports
pub use http::{Method, StatusCode, Version};
pub use cookie::Cookie;
pub use http_range::HttpRange;

#[cfg(feature="tls")]
pub use native_tls::Pkcs12;

#[cfg(feature="openssl")]
pub use openssl::pkcs12::Pkcs12;
