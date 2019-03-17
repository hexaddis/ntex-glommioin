use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_server_config::ServerConfig;
use actix_service::boxed::{self, BoxedNewService};
use actix_service::{
    ApplyTransform, IntoNewService, IntoTransform, NewService, Transform,
};
use futures::IntoFuture;

use crate::app_service::{AppChain, AppEntry, AppInit, AppRouting, AppRoutingFactory};
use crate::config::{AppConfig, AppConfigInner};
use crate::data::{Data, DataFactory};
use crate::dev::{PayloadStream, ResourceDef};
use crate::error::Error;
use crate::resource::Resource;
use crate::route::Route;
use crate::service::{
    HttpServiceFactory, ServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};

type HttpNewService<P> =
    BoxedNewService<(), ServiceRequest<P>, ServiceResponse, Error, ()>;

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<P, T>
where
    T: NewService<Request = ServiceRequest, Response = ServiceRequest<P>>,
{
    chain: T,
    data: Vec<Box<DataFactory>>,
    config: AppConfigInner,
    _t: PhantomData<(P,)>,
}

impl App<PayloadStream, AppChain> {
    /// Create application builder. Application can be configured with a builder-like pattern.
    pub fn new() -> Self {
        App {
            chain: AppChain,
            data: Vec::new(),
            config: AppConfigInner::default(),
            _t: PhantomData,
        }
    }
}

impl<P, T> App<P, T>
where
    P: 'static,
    T: NewService<
        Request = ServiceRequest,
        Response = ServiceRequest<P>,
        Error = Error,
        InitError = (),
    >,
{
    /// Set application data. Applicatin data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    ///
    /// **Note**: http server accepts an application factory rather than
    /// an application instance. Http server constructs an application
    /// instance for each thread, thus application data must be constructed
    /// multiple times. If you want to share data between different
    /// threads, a shared object should be used, e.g. `Arc`. Application
    /// data does not need to be `Send` or `Sync`.
    ///
    /// ```rust
    /// use std::cell::Cell;
    /// use actix_web::{web, App};
    ///
    /// struct MyData {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// fn index(data: web::Data<MyData>) {
    ///     data.counter.set(data.counter.get() + 1);
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .data(MyData{ counter: Cell::new(0) })
    ///         .service(
    ///             web::resource("/index.html").route(
    ///                 web::get().to(index)));
    /// }
    /// ```
    pub fn data<S: 'static>(mut self, data: S) -> Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Set application data factory. This function is
    /// similar to `.data()` but it accepts data factory. Data object get
    /// constructed asynchronously during application initialization.
    pub fn data_factory<F, Out>(mut self, data: F) -> Self
    where
        F: Fn() -> Out + 'static,
        Out: IntoFuture + 'static,
        Out::Error: std::fmt::Debug,
    {
        self.data.push(Box::new(data));
        self
    }

    /// Register a middleware.
    pub fn middleware<M, B, F>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        P,
        B,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            AppRouting<P>,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
        F: IntoTransform<M, AppRouting<P>>,
    {
        let fref = Rc::new(RefCell::new(None));
        let endpoint = ApplyTransform::new(mw, AppEntry::new(fref.clone()));
        AppRouter {
            endpoint,
            chain: self.chain,
            data: self.data,
            services: Vec::new(),
            default: None,
            factory_ref: fref,
            config: self.config,
            external: Vec::new(),
            _t: PhantomData,
        }
    }

    /// Register a request modifier. It can modify any request parameters
    /// including payload stream.
    pub fn chain<C, F, P1>(
        self,
        chain: C,
    ) -> App<
        P1,
        impl NewService<
            Request = ServiceRequest,
            Response = ServiceRequest<P1>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        C: NewService<
            Request = ServiceRequest<P>,
            Response = ServiceRequest<P1>,
            Error = Error,
            InitError = (),
        >,
        F: IntoNewService<C>,
    {
        let chain = self.chain.and_then(chain.into_new_service());
        App {
            chain,
            data: self.data,
            config: self.config,
            _t: PhantomData,
        }
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can be used multiple times with same path, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// }
    /// ```
    pub fn route(
        self,
        path: &str,
        mut route: Route<P>,
    ) -> AppRouter<T, P, Body, AppEntry<P>> {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    pub fn service<F>(self, service: F) -> AppRouter<T, P, Body, AppEntry<P>>
    where
        F: HttpServiceFactory<P> + 'static,
    {
        let fref = Rc::new(RefCell::new(None));

        AppRouter {
            chain: self.chain,
            default: None,
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            data: self.data,
            config: self.config,
            services: vec![Box::new(ServiceFactoryWrapper::new(service))],
            external: Vec::new(),
            _t: PhantomData,
        }
    }

    /// Set server host name.
    ///
    /// Host name is used by application router as a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn hostname(mut self, val: &str) -> Self {
        self.config.host = val.to_owned();
        self
    }
}

/// Application router builder - Structure that follows the builder pattern
/// for building application instances.
pub struct AppRouter<C, P, B, T> {
    chain: C,
    endpoint: T,
    services: Vec<Box<ServiceFactory<P>>>,
    default: Option<Rc<HttpNewService<P>>>,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
    data: Vec<Box<DataFactory>>,
    config: AppConfigInner,
    external: Vec<ResourceDef>,
    _t: PhantomData<(P, B)>,
}

impl<C, P, B, T> AppRouter<C, P, B, T>
where
    P: 'static,
    B: MessageBody,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can not be could multiple times, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// }
    /// ```
    pub fn route(self, path: &str, mut route: Route<P>) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    ///
    /// Http service is any type that implements `HttpServiceFactory` trait.
    ///
    /// Actix web provides several services implementations:
    ///
    /// * *Resource* is an entry in route table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    /// * "StaticFiles" is a service for static files support
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory<P> + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Register a middleware.
    pub fn middleware<M, B1, F>(
        self,
        mw: F,
    ) -> AppRouter<
        C,
        P,
        B1,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
        B1: MessageBody,
        F: IntoTransform<M, T::Service>,
    {
        let endpoint = ApplyTransform::new(mw, self.endpoint);
        AppRouter {
            endpoint,
            chain: self.chain,
            data: self.data,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            config: self.config,
            external: self.external,
            _t: PhantomData,
        }
    }

    /// Default resource to be used if no matching route could be found.
    ///
    /// Default resource works with resources only and does not work with
    /// custom services.
    pub fn default_resource<F, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::new_service(
            f(Resource::new("")).into_new_service().map_init_err(|_| ()),
        )));

        self
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpRequest, HttpResponse, Result};
    ///
    /// fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     let url = req.url_for("youtube", &["asdlkjqme"])?;
    ///     assert_eq!(url.as_str(), "https://youtube.com/watch/asdlkjqme");
    ///     Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(web::resource("/index.html").route(
    ///             web::get().to(index)))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}");
    /// }
    /// ```
    pub fn external_resource<N, U>(mut self, name: N, url: U) -> Self
    where
        N: AsRef<str>,
        U: AsRef<str>,
    {
        let mut rdef = ResourceDef::new(url.as_ref());
        *rdef.name_mut() = name.as_ref().to_string();
        self.external.push(rdef);
        self
    }
}

impl<C, T, P: 'static, B: MessageBody> IntoNewService<AppInit<C, T, P, B>, ServerConfig>
    for AppRouter<C, P, B, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
    C: NewService<
        Request = ServiceRequest,
        Response = ServiceRequest<P>,
        Error = Error,
        InitError = (),
    >,
{
    fn into_new_service(self) -> AppInit<C, T, P, B> {
        AppInit {
            chain: self.chain,
            data: self.data,
            endpoint: self.endpoint,
            services: RefCell::new(self.services),
            external: RefCell::new(self.external),
            default: self.default,
            factory_ref: self.factory_ref,
            config: RefCell::new(AppConfig(Rc::new(self.config))),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;

    use super::*;
    use crate::http::{Method, StatusCode};
    use crate::test::{block_on, init_service, TestRequest};
    use crate::{web, HttpResponse};

    #[test]
    fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(web::resource("/test").to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = init_service(
            App::new()
                .service(web::resource("/test").to(|| HttpResponse::Ok()))
                .service(
                    web::resource("/test2")
                        .default_resource(|r| r.to(|| HttpResponse::Created()))
                        .route(web::get().to(|| HttpResponse::Ok())),
                )
                .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed())),
        );

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test2")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_data() {
        let mut srv =
            init_service(App::new().data(10usize).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));

        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data(10u32).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_data_factory() {
        let mut srv =
            init_service(App::new().data_factory(|| Ok::<_, ()>(10usize)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data_factory(|| Ok::<_, ()>(10u32)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
