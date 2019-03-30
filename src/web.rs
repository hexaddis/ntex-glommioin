//! Essentials helper functions and types for application registration.
use actix_http::{http::Method, Response};
use futures::{Future, IntoFuture};

pub use actix_http::Response as HttpResponse;
pub use bytes::{Bytes, BytesMut};

use crate::error::{BlockingError, Error};
use crate::extract::FromRequest;
use crate::handler::{AsyncFactory, Factory};
use crate::resource::Resource;
use crate::responder::Responder;
use crate::route::Route;
use crate::scope::Scope;

pub use crate::data::{Data, RouteData};
pub use crate::request::HttpRequest;
pub use crate::types::*;

/// Create resource for a specific path.
///
/// Resources may have variable path segments. For example, a
/// resource with the path `/a/{name}/c` would match all incoming
/// requests with paths such as `/a/b/c`, `/a/1/c`, or `/a/etc/c`.
///
/// A variable segment is specified in the form `{identifier}`,
/// where the identifier can be used later in a request handler to
/// access the matched value for that segment. This is done by
/// looking up the identifier in the `Params` object returned by
/// `HttpRequest.match_info()` method.
///
/// By default, each segment matches the regular expression `[^{}/]+`.
///
/// You can also specify a custom regex in the form `{identifier:regex}`:
///
/// For instance, to route `GET`-requests on any route matching
/// `/users/{userid}/{friend}` and store `userid` and `friend` in
/// the exposed `Params` object:
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/{userid}/{friend}")
///             .route(web::get().to(|| HttpResponse::Ok()))
///             .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
pub fn resource<P: 'static>(path: &str) -> Resource<P> {
    Resource::new(path)
}

/// Configure scope for common root path.
///
/// Scopes collect multiple paths under a common path prefix.
/// Scope path can contain variable path segments as resources.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::scope("/{project_id}")
///             .service(web::resource("/path1").to(|| HttpResponse::Ok()))
///             .service(web::resource("/path2").to(|| HttpResponse::Ok()))
///             .service(web::resource("/path3").to(|| HttpResponse::MethodNotAllowed()))
///     );
/// }
/// ```
///
/// In the above example, three routes get added:
///  * /{project_id}/path1
///  * /{project_id}/path2
///  * /{project_id}/path3
///
pub fn scope<P: 'static>(path: &str) -> Scope<P> {
    Scope::new(path)
}

/// Create *route* without configuration.
pub fn route<P: 'static>() -> Route<P> {
    Route::new()
}

/// Create *route* with `GET` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::get().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `GET` route get added:
///  * /{project_id}
///
pub fn get<P: 'static>() -> Route<P> {
    Route::new().method(Method::GET)
}

/// Create *route* with `POST` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::post().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `POST` route get added:
///  * /{project_id}
///
pub fn post<P: 'static>() -> Route<P> {
    Route::new().method(Method::POST)
}

/// Create *route* with `PUT` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::put().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `PUT` route get added:
///  * /{project_id}
///
pub fn put<P: 'static>() -> Route<P> {
    Route::new().method(Method::PUT)
}

/// Create *route* with `PATCH` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::patch().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `PATCH` route get added:
///  * /{project_id}
///
pub fn patch<P: 'static>() -> Route<P> {
    Route::new().method(Method::PATCH)
}

/// Create *route* with `DELETE` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::delete().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `DELETE` route get added:
///  * /{project_id}
///
pub fn delete<P: 'static>() -> Route<P> {
    Route::new().method(Method::DELETE)
}

/// Create *route* with `HEAD` method guard.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::head().to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `HEAD` route get added:
///  * /{project_id}
///
pub fn head<P: 'static>() -> Route<P> {
    Route::new().method(Method::HEAD)
}

/// Create *route* and add method guard.
///
/// ```rust
/// use actix_web::{web, http, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/{project_id}")
///             .route(web::method(http::Method::GET).to(|| HttpResponse::Ok()))
///     );
/// }
/// ```
///
/// In the above example, one `GET` route get added:
///  * /{project_id}
///
pub fn method<P: 'static>(method: Method) -> Route<P> {
    Route::new().method(method)
}

/// Create a new route and add handler.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn index() -> HttpResponse {
///    unimplemented!()
/// }
///
/// App::new().service(
///     web::resource("/").route(
///         web::to(index))
/// );
/// ```
pub fn to<F, I, R, P: 'static>(handler: F) -> Route<P>
where
    F: Factory<I, R> + 'static,
    I: FromRequest<P> + 'static,
    R: Responder + 'static,
{
    Route::new().to(handler)
}

/// Create a new route and add async handler.
///
/// ```rust
/// # use futures::future::{ok, Future};
/// use actix_web::{web, App, HttpResponse, Error};
///
/// fn index() -> impl Future<Item=HttpResponse, Error=Error> {
///     ok(HttpResponse::Ok().finish())
/// }
///
/// App::new().service(web::resource("/").route(
///     web::to_async(index))
/// );
/// ```
pub fn to_async<F, I, R, P: 'static>(handler: F) -> Route<P>
where
    F: AsyncFactory<I, R>,
    I: FromRequest<P> + 'static,
    R: IntoFuture + 'static,
    R::Item: Into<Response>,
    R::Error: Into<Error>,
{
    Route::new().to_async(handler)
}

/// Execute blocking function on a thread pool, returns future that resolves
/// to result of the function execution.
pub fn block<F, I, E>(f: F) -> impl Future<Item = I, Error = BlockingError<E>>
where
    F: FnOnce() -> Result<I, E> + Send + 'static,
    I: Send + 'static,
    E: Send + std::fmt::Debug + 'static,
{
    actix_threadpool::run(f).from_err()
}
