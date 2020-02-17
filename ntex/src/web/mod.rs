//! Web framework for Rust.
//!
//! ```rust,no_run
//! use ntex::web;
//!
//! async fn index(info: web::types::Path<(String, u32)>) -> impl web::Responder {
//!     format!("Hello {}! id:{}", info.0, info.1)
//! }
//!
//! #[ntex::main]
//! async fn main() -> std::io::Result<()> {
//!     web::server(|| web::App::new().service(
//!         web::resource("/{name}/{id}/index.html").to(index))
//!     )
//!         .bind("127.0.0.1:8080")?
//!         .run()
//!         .await
//! }
//! ```
//!
//! ## Documentation & community resources
//!
//! Besides the API documentation (which you are currently looking
//! at!), several other resources are available:
//!
//! * [User Guide](https://docs.rs/ntex/)
//! * [GitHub repository](https://github.com/fafhrd91/ntex)
//! * [Cargo package](https://crates.io/crates/ntex)
//!
//! To get started navigating the API documentation you may want to
//! consider looking at the following pages:
//!
//! * [App](struct.App.html): This struct represents an actix-web
//!   application and is used to configure routes and other common
//!   settings.
//!
//! * [HttpServer](struct.HttpServer.html): This struct
//!   represents an HTTP server instance and is used to instantiate and
//!   configure servers.
//!
//! * [HttpRequest](struct.HttpRequest.html) and
//!   [HttpResponse](struct.HttpResponse.html): These structs
//!   represent HTTP requests and responses and expose various methods
//!   for inspecting, creating and otherwise utilizing them.
//!
//! ## Features
//!
//! * Supported *HTTP/1.x* and *HTTP/2.0* protocols
//! * Streaming and pipelining
//! * Keep-alive and slow requests handling
//! * *WebSockets* server/client
//! * Transparent content compression/decompression (br, gzip, deflate)
//! * Configurable request routing
//! * Multipart streams
//! * SSL support with OpenSSL or `rustls`
//! * Middlewares
//! * Supported Rust version: 1.39 or later
//!
//! ## Package feature
//!
//! * `cookie` - enables http cookie support
//! * `compress` - enables content encoding compression support
//! * `openssl` - enables ssl support via `openssl` crate, supports `http/2`
//! * `rustls` - enables ssl support via `rustls` crate, supports `http/2`
#![allow(clippy::type_complexity, clippy::new_without_default)]

mod app;
mod app_service;
mod config;
mod data;
pub mod error;
mod extract;
pub mod guard;
mod handler;
mod info;
pub mod middleware;
mod request;
mod resource;
mod responder;
mod rmap;
mod route;
mod scope;
mod server;
mod service;
pub mod test;
pub mod types;
mod util;

pub use ntex_web_macros::*;

pub use crate::http::Response as HttpResponse;

pub use self::app::App;
pub use self::config::ServiceConfig;
pub use self::data::Data;
pub use self::extract::FromRequest;
pub use self::request::HttpRequest;
pub use self::resource::Resource;
pub use self::responder::{Either, Responder};
pub use self::route::Route;
pub use self::scope::Scope;
pub use self::server::HttpServer;
pub use self::service::WebService;
pub use self::util::*;

pub mod dev {
    //! The `actix-web` prelude for library developers
    //!
    //! The purpose of this module is to alleviate imports of many common actix
    //! traits by adding a glob import to the top of actix heavy modules:

    pub use crate::web::config::{AppConfig, AppService};
    #[doc(hidden)]
    pub use crate::web::handler::Factory;
    pub use crate::web::info::ConnectionInfo;
    pub use crate::web::rmap::ResourceMap;
    pub use crate::web::service::{
        HttpServiceFactory, ServiceRequest, ServiceResponse, WebService,
    };

    pub use crate::web::types::form::UrlEncoded;
    pub use crate::web::types::json::JsonBody;

    pub use actix_router::{Path, ResourceDef, ResourcePath, Url};

    pub(crate) fn insert_slash(mut patterns: Vec<String>) -> Vec<String> {
        for path in &mut patterns {
            if !path.is_empty() && !path.starts_with('/') {
                path.insert(0, '/');
            };
        }
        patterns
    }

    use crate::http::header::ContentEncoding;
    use crate::http::{Response, ResponseBuilder};

    struct Enc(ContentEncoding);

    /// Helper trait that allows to set specific encoding for response.
    pub trait BodyEncoding {
        /// Get content encoding
        fn get_encoding(&self) -> Option<ContentEncoding>;

        /// Set content encoding
        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self;
    }

    impl BodyEncoding for ResponseBuilder {
        fn get_encoding(&self) -> Option<ContentEncoding> {
            if let Some(ref enc) = self.extensions().get::<Enc>() {
                Some(enc.0)
            } else {
                None
            }
        }

        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
            self.extensions_mut().insert(Enc(encoding));
            self
        }
    }

    impl<B> BodyEncoding for Response<B> {
        fn get_encoding(&self) -> Option<ContentEncoding> {
            if let Some(ref enc) = self.extensions().get::<Enc>() {
                Some(enc.0)
            } else {
                None
            }
        }

        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
            self.extensions_mut().insert(Enc(encoding));
            self
        }
    }
}
