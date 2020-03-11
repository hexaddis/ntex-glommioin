//! Essentials helper functions and types for application registration.
use std::fmt;
use std::future::Future;

use actix_router::IntoPattern;

use crate::http::body::MessageBody;
use crate::http::error::{BlockingError, ResponseError};
use crate::http::{Method, Request, Response};
use crate::{IntoServiceFactory, Service, ServiceFactory};

use super::config::AppConfig;
use super::extract::FromRequest;
use super::handler::Factory;
use super::resource::Resource;
use super::responder::Responder;
use super::route::Route;
use super::scope::Scope;
use super::server::HttpServer;
use super::service::WebService;

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
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/users/{userid}/{friend}")
///         .route(web::get().to(|| async { web::HttpResponse::Ok() }))
///         .route(web::head().to(|| async { web::HttpResponse::MethodNotAllowed() }))
/// );
/// ```
pub fn resource<T: IntoPattern, Err: 'static>(path: T) -> Resource<Err> {
    Resource::new(path)
}

/// Configure scope for common root path.
///
/// Scopes collect multiple paths under a common path prefix.
/// Scope path can contain variable path segments as resources.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::scope("/{project_id}")
///         .service(web::resource("/path1").to(|| async { web::HttpResponse::Ok() }))
///         .service(web::resource("/path2").to(|| async { web::HttpResponse::Ok() }))
///         .service(web::resource("/path3").to(|| async { web::HttpResponse::MethodNotAllowed() }))
/// );
/// ```
///
/// In the above example, three routes get added:
///  * /{project_id}/path1
///  * /{project_id}/path2
///  * /{project_id}/path3
///
pub fn scope<Err: 'static>(path: &str) -> Scope<Err> {
    Scope::new(path)
}

/// Create *route* without configuration.
pub fn route<Err: 'static>() -> Route<Err> {
    Route::new()
}

/// Create *route* with `GET` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///        .route(web::get().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `GET` route gets added:
///  * /{project_id}
///
pub fn get<Err: 'static>() -> Route<Err> {
    method(Method::GET)
}

/// Create *route* with `POST` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::post().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `POST` route gets added:
///  * /{project_id}
///
pub fn post<Err: 'static>() -> Route<Err> {
    method(Method::POST)
}

/// Create *route* with `PUT` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::put().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `PUT` route gets added:
///  * /{project_id}
///
pub fn put<Err: 'static>() -> Route<Err> {
    method(Method::PUT)
}

/// Create *route* with `PATCH` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::patch().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `PATCH` route gets added:
///  * /{project_id}
///
pub fn patch<Err: 'static>() -> Route<Err> {
    method(Method::PATCH)
}

/// Create *route* with `DELETE` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::delete().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `DELETE` route gets added:
///  * /{project_id}
///
pub fn delete<Err: 'static>() -> Route<Err> {
    method(Method::DELETE)
}

/// Create *route* with `HEAD` method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::head().to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `HEAD` route gets added:
///  * /{project_id}
///
pub fn head<Err: 'static>() -> Route<Err> {
    method(Method::HEAD)
}

/// Create *route* and add method guard.
///
/// ```rust
/// use ntex::web;
///
/// let app = web::App::new().service(
///     web::resource("/{project_id}")
///         .route(web::method(http::Method::GET).to(|| async { web::HttpResponse::Ok() }))
/// );
/// ```
///
/// In the above example, one `GET` route gets added:
///  * /{project_id}
///
pub fn method<Err: 'static>(method: Method) -> Route<Err> {
    Route::new().method(method)
}

/// Create a new route and add handler.
///
/// ```rust
/// use ntex::web;
///
/// async fn index() -> impl web::Responder {
///    web::HttpResponse::Ok()
/// }
///
/// web::App::new().service(
///     web::resource("/").route(web::to(index))
/// );
/// ```
pub fn to<F, I, R, U, Err>(handler: F) -> Route<Err>
where
    F: Factory<I, R, U, Err>,
    I: FromRequest<Err> + 'static,
    R: Future<Output = U> + 'static,
    U: Responder<Err> + 'static,
    Err: 'static,
{
    Route::new().to(handler)
}

/// Create raw service for a specific path.
///
/// ```rust
/// use ntex::web::{self, dev, guard, App, HttpResponse, DefaultError, WebError};
///
/// async fn my_service(req: dev::WebRequest) -> Result<dev::WebResponse, WebError<DefaultError>> {
///     Ok(req.into_response(HttpResponse::Ok().finish()))
/// }
///
/// let app = App::new().service(
///     web::service("/users/*")
///         .guard(guard::Header("content-type", "text/plain"))
///         .finish(my_service)
/// );
/// ```
pub fn service<T: IntoPattern>(path: T) -> WebService {
    WebService::new(path)
}

/// Execute blocking function on a thread pool, returns future that resolves
/// to result of the function execution.
pub async fn block<F, I, E>(f: F) -> Result<I, BlockingError<E>>
where
    F: FnOnce() -> Result<I, E> + Send + 'static,
    I: Send + 'static,
    E: Send + std::fmt::Debug + 'static,
{
    actix_threadpool::run(f).await
}

/// Create new http server with application factory.
///
/// ```rust,no_run
/// use ntex::web;
///
/// #[ntex::main]
/// async fn main() -> std::io::Result<()> {
///     web::server(
///         || web::App::new()
///             .service(web::resource("/").to(|| async { web::HttpResponse::Ok() })))
///         .bind("127.0.0.1:59090")?
///         .run()
///         .await
/// }
/// ```
pub fn server<F, I, S, B>(factory: F) -> HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S>,
    S: ServiceFactory<Config = AppConfig, Request = Request>,
    S::Error: ResponseError + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    HttpServer::new(factory)
}
