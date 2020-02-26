use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use futures::future::{ready, FutureExt, LocalBoxFuture};

use crate::http::{Error, Method};
use crate::{Service, ServiceFactory};

use crate::web::extract::FromRequest;
use crate::web::guard::{self, Guard};
use crate::web::handler::{Extract, Factory, Handler};
use crate::web::responder::Responder;
use crate::web::service::{WebRequest, WebResponse};
use crate::web::HttpResponse;

type BoxedRouteService<Req, Res> = Box<
    dyn Service<
        Request = Req,
        Response = Res,
        Error = Error,
        Future = LocalBoxFuture<'static, Result<Res, Error>>,
    >,
>;

type BoxedRouteNewService<Req, Res> = Box<
    dyn ServiceFactory<
        Config = (),
        Request = Req,
        Response = Res,
        Error = Error,
        InitError = (),
        Service = BoxedRouteService<Req, Res>,
        Future = LocalBoxFuture<'static, Result<BoxedRouteService<Req, Res>, ()>>,
    >,
>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route {
    service: BoxedRouteNewService<WebRequest, WebResponse>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Route {
    /// Create new route which matches any request.
    pub fn new() -> Route {
        Route {
            service: Box::new(RouteNewService::new(Extract::new(Handler::new(|| {
                ready(HttpResponse::NotFound())
            })))),
            guards: Rc::new(Vec::new()),
        }
    }

    pub(crate) fn take_guards(&mut self) -> Vec<Box<dyn Guard>> {
        std::mem::replace(Rc::get_mut(&mut self.guards).unwrap(), Vec::new())
    }
}

impl ServiceFactory for Route {
    type Config = ();
    type Request = WebRequest;
    type Response = WebResponse;
    type Error = Error;
    type InitError = ();
    type Service = RouteService;
    type Future = CreateRouteService;

    fn new_service(&self, _: ()) -> Self::Future {
        CreateRouteService {
            fut: self.service.new_service(()),
            guards: self.guards.clone(),
        }
    }
}

type RouteFuture =
    LocalBoxFuture<'static, Result<BoxedRouteService<WebRequest, WebResponse>, ()>>;

#[pin_project::pin_project]
pub struct CreateRouteService {
    #[pin]
    fut: RouteFuture,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl Future for CreateRouteService {
    type Output = Result<RouteService, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match this.fut.poll(cx)? {
            Poll::Ready(service) => Poll::Ready(Ok(RouteService {
                service,
                guards: this.guards.clone(),
            })),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct RouteService {
    service: BoxedRouteService<WebRequest, WebResponse>,
    guards: Rc<Vec<Box<dyn Guard>>>,
}

impl RouteService {
    pub fn check(&self, req: &mut WebRequest) -> bool {
        for f in self.guards.iter() {
            if !f.check(req.head()) {
                return false;
            }
        }
        true
    }
}

impl Service for RouteService {
    type Request = WebRequest;
    type Response = WebResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: WebRequest) -> Self::Future {
        self.service.call(req).boxed_local()
    }
}

impl Route {
    /// Add method guard to the route.
    ///
    /// ```rust
    /// # use ntex::web::{self, *};
    /// # fn main() {
    /// App::new().service(web::resource("/path").route(
    ///     web::get()
    ///         .method(http::Method::CONNECT)
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// );
    /// # }
    /// ```
    pub fn method(mut self, method: Method) -> Self {
        Rc::get_mut(&mut self.guards)
            .unwrap()
            .push(Box::new(guard::Method(method)));
        self
    }

    /// Add guard to the route.
    ///
    /// ```rust
    /// # use ntex::web::{self, *};
    /// # fn main() {
    /// App::new().service(web::resource("/path").route(
    ///     web::route()
    ///         .guard(guard::Get())
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// );
    /// # }
    /// ```
    pub fn guard<F: Guard + 'static>(mut self, f: F) -> Self {
        Rc::get_mut(&mut self.guards).unwrap().push(Box::new(f));
        self
    }

    /// Set handler function, use request extractors for parameters.
    ///
    /// ```rust
    /// use ntex::web;
    /// use serde_derive::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// async fn index(info: web::types::Path<Info>) -> String {
    ///     format!("Welcome {}!", info.username)
    /// }
    ///
    /// fn main() {
    ///     let app = web::App::new().service(
    ///         web::resource("/{username}/index.html") // <- define path parameters
    ///             .route(web::get().to(index))        // <- register handler
    ///     );
    /// }
    /// ```
    ///
    /// It is possible to use multiple extractors for one handler function.
    ///
    /// ```rust
    /// # use std::collections::HashMap;
    /// # use serde_derive::Deserialize;
    /// use ntex::web;
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// async fn index(path: web::types::Path<Info>, query: web::types::Query<HashMap<String, String>>, body: web::types::Json<Info>) -> String {
    ///     format!("Welcome {}!", path.username)
    /// }
    ///
    /// fn main() {
    ///     let app = web::App::new().service(
    ///         web::resource("/{username}/index.html") // <- define path parameters
    ///             .route(web::get().to(index))
    ///     );
    /// }
    /// ```
    pub fn to<F, T, R, U>(mut self, handler: F) -> Self
    where
        F: Factory<T, R, U>,
        T: FromRequest + 'static,
        R: Future<Output = U> + 'static,
        U: Responder + 'static,
    {
        self.service =
            Box::new(RouteNewService::new(Extract::new(Handler::new(handler))));
        self
    }
}

struct RouteNewService<T>
where
    T: ServiceFactory<Request = WebRequest, Error = (Error, WebRequest)>,
{
    service: T,
}

impl<T> RouteNewService<T>
where
    T: ServiceFactory<
        Config = (),
        Request = WebRequest,
        Response = WebResponse,
        Error = (Error, WebRequest),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        RouteNewService { service }
    }
}

impl<T> ServiceFactory for RouteNewService<T>
where
    T: ServiceFactory<
        Config = (),
        Request = WebRequest,
        Response = WebResponse,
        Error = (Error, WebRequest),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Config = ();
    type Request = WebRequest;
    type Response = WebResponse;
    type Error = Error;
    type InitError = ();
    type Service = BoxedRouteService<WebRequest, Self::Response>;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        self.service
            .new_service(())
            .map(|result| match result {
                Ok(service) => {
                    let service: BoxedRouteService<_, _> =
                        Box::new(RouteServiceWrapper { service });
                    Ok(service)
                }
                Err(_) => Err(()),
            })
            .boxed_local()
    }
}

struct RouteServiceWrapper<T: Service> {
    service: T,
}

impl<T> Service for RouteServiceWrapper<T>
where
    T::Future: 'static,
    T: Service<
        Request = WebRequest,
        Response = WebResponse,
        Error = (Error, WebRequest),
    >,
{
    type Request = WebRequest;
    type Response = WebResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(|(e, _)| e)
    }

    fn call(&mut self, req: WebRequest) -> Self::Future {
        self.service
            .call(req)
            .map(|res| match res {
                Ok(res) => Ok(res),
                Err((err, req)) => Ok(req.error_response(err)),
            })
            .boxed_local()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_rt::time::delay_for;
    use bytes::Bytes;
    use serde_derive::Serialize;

    use crate::http::{error, Method, StatusCode};
    use crate::web::test::{call_service, init_service, read_body, TestRequest};
    use crate::web::{self, App, HttpResponse};

    #[derive(Serialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    #[actix_rt::test]
    async fn test_route() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::resource("/test")
                        .route(web::get().to(|| HttpResponse::Ok()))
                        .route(web::put().to(|| async {
                            Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                        }))
                        .route(web::post().to(|| async {
                            delay_for(Duration::from_millis(100)).await;
                            HttpResponse::Created()
                        }))
                        .route(web::delete().to(|| async {
                            delay_for(Duration::from_millis(100)).await;
                            Err::<HttpResponse, _>(error::ErrorBadRequest("err"))
                        })),
                )
                .service(web::resource("/json").route(web::get().to(|| async {
                    delay_for(Duration::from_millis(25)).await;
                    web::types::Json(MyObject {
                        name: "test".to_string(),
                    })
                }))),
        )
        .await;

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/test")
            .method(Method::HEAD)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/json").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"{\"name\":\"test\"}"));
    }
}
