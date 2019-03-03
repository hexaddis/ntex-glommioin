use std::cell::RefCell;
use std::rc::Rc;

use actix_http::{Error, Response};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    ApplyNewService, IntoNewService, IntoNewTransform, NewService, NewTransform, Service,
};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::handler::{AsyncFactory, Factory, FromRequest};
use crate::responder::Responder;
use crate::route::{CreateRouteService, Route, RouteService};
use crate::service::{ServiceRequest, ServiceResponse};

type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Resource<P, T = ResourceEndpoint<P>> {
    routes: Vec<Route<P>>,
    endpoint: T,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
    factory_ref: Rc<RefCell<Option<ResourceFactory<P>>>>,
}

impl<P> Resource<P> {
    pub fn new() -> Resource<P> {
        let fref = Rc::new(RefCell::new(None));

        Resource {
            routes: Vec::new(),
            endpoint: ResourceEndpoint::new(fref.clone()),
            factory_ref: fref,
            default: Rc::new(RefCell::new(None)),
        }
    }
}

impl<P> Default for Resource<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: 'static, T> Resource<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    /// Register a new route and return mutable reference to *Route* object.
    /// *Route* is used for route configuration, i.e. adding predicates,
    /// setting up handler.
    ///
    /// ```rust,ignore
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/", |r| {
    ///             r.route()
    ///                 .filter(pred::Any(pred::Get()).or(pred::Put()))
    ///                 .filter(pred::Header("Content-Type", "text/plain"))
    ///                 .f(|r| HttpResponse::Ok())
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn route(mut self, route: Route<P>) -> Self {
        self.routes.push(route.finish());
        self
    }

    /// Register a new route and add handler.
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::*;
    /// fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    ///
    /// App::new().resource("/", |r| r.with(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().resource("/", |r| r.route().with(index));
    /// ```
    pub fn to<F, I, R>(mut self, handler: F) -> Self
    where
        F: Factory<I, R> + 'static,
        I: FromRequest<P> + 'static,
        R: Responder + 'static,
    {
        self.routes.push(Route::new().to(handler));
        self
    }

    /// Register a new route and add async handler.
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// use actix_web::*;
    /// use futures::future::Future;
    ///
    /// fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ///     unimplemented!()
    /// }
    ///
    /// App::new().resource("/", |r| r.with_async(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use actix_web::*;
    /// # use futures::future::Future;
    /// # fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    /// #     unimplemented!()
    /// # }
    /// App::new().resource("/", |r| r.route().with_async(index));
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn to_async<F, I, R>(mut self, handler: F) -> Self
    where
        F: AsyncFactory<I, R>,
        I: FromRequest<P> + 'static,
        R: IntoFuture + 'static,
        R::Item: Into<Response>,
        R::Error: Into<Error>,
    {
        self.routes.push(Route::new().to_async(handler));
        self
    }

    /// Register a resource middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middlewares get invoked on resource level.
    pub fn middleware<M, F>(
        self,
        mw: F,
    ) -> Resource<
        P,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: NewTransform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
        F: IntoNewTransform<M, T::Service>,
    {
        let endpoint = ApplyNewService::new(mw, self.endpoint);
        Resource {
            endpoint,
            routes: self.routes,
            default: self.default,
            factory_ref: self.factory_ref,
        }
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> R,
        R: IntoNewService<U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::new_service(
            f(Resource::new()).into_new_service().map_init_err(|_| ()),
        )))));

        self
    }

    pub(crate) fn get_default(&self) -> Rc<RefCell<Option<Rc<HttpNewService<P>>>>> {
        self.default.clone()
    }
}

impl<P, T> IntoNewService<T> for Resource<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    fn into_new_service(self) -> T {
        *self.factory_ref.borrow_mut() = Some(ResourceFactory {
            routes: self.routes,
            default: self.default,
        });

        self.endpoint
    }
}

pub struct ResourceFactory<P> {
    routes: Vec<Route<P>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
}

impl<P: 'static> NewService for ResourceFactory<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = ResourceService<P>;
    type Future = CreateResourceService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        let default_fut = if let Some(ref default) = *self.default.borrow() {
            Some(default.new_service(&()))
        } else {
            None
        };

        CreateResourceService {
            fut: self
                .routes
                .iter()
                .map(|route| CreateRouteServiceItem::Future(route.new_service(&())))
                .collect(),
            default: None,
            default_fut,
        }
    }
}

enum CreateRouteServiceItem<P> {
    Future(CreateRouteService<P>),
    Service(RouteService<P>),
}

pub struct CreateResourceService<P> {
    fut: Vec<CreateRouteServiceItem<P>>,
    default: Option<HttpService<P>>,
    default_fut: Option<Box<Future<Item = HttpService<P>, Error = ()>>>,
}

impl<P> Future for CreateResourceService<P> {
    type Item = ResourceService<P>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut done = true;

        if let Some(ref mut fut) = self.default_fut {
            match fut.poll()? {
                Async::Ready(default) => self.default = Some(default),
                Async::NotReady => done = false,
            }
        }

        // poll http services
        for item in &mut self.fut {
            match item {
                CreateRouteServiceItem::Future(ref mut fut) => match fut.poll()? {
                    Async::Ready(route) => {
                        *item = CreateRouteServiceItem::Service(route)
                    }
                    Async::NotReady => {
                        done = false;
                    }
                },
                CreateRouteServiceItem::Service(_) => continue,
            };
        }

        if done {
            let routes = self
                .fut
                .drain(..)
                .map(|item| match item {
                    CreateRouteServiceItem::Service(service) => service,
                    CreateRouteServiceItem::Future(_) => unreachable!(),
                })
                .collect();
            Ok(Async::Ready(ResourceService {
                routes,
                default: self.default.take(),
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct ResourceService<P> {
    routes: Vec<RouteService<P>>,
    default: Option<HttpService<P>>,
}

impl<P> Service for ResourceService<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type Future = Either<
        Box<Future<Item = ServiceResponse, Error = ()>>,
        Either<
            Box<Future<Item = Self::Response, Error = Self::Error>>,
            FutureResult<Self::Response, Self::Error>,
        >,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, mut req: ServiceRequest<P>) -> Self::Future {
        for route in self.routes.iter_mut() {
            if route.check(&mut req) {
                return Either::A(route.call(req));
            }
        }
        if let Some(ref mut default) = self.default {
            Either::B(Either::A(default.call(req)))
        } else {
            let req = req.into_request();
            Either::B(Either::B(ok(ServiceResponse::new(
                req,
                Response::NotFound().finish(),
            ))))
        }
    }
}

#[doc(hidden)]
pub struct ResourceEndpoint<P> {
    factory: Rc<RefCell<Option<ResourceFactory<P>>>>,
}

impl<P> ResourceEndpoint<P> {
    fn new(factory: Rc<RefCell<Option<ResourceFactory<P>>>>) -> Self {
        ResourceEndpoint { factory }
    }
}

impl<P: 'static> NewService for ResourceEndpoint<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = ResourceService<P>;
    type Future = CreateResourceService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}
