# Application

Actix web provides some primitives to build web servers and applications with Rust.
It provides routing, middlewares, pre-processing of requests, and post-processing of responses,
websocket protocol handling, multipart streams, etc.

All actix web server is built around `Application` instance.
It is used for registering routes for resources, middlewares.
Also it stores application specific state that is shared across all handlers
within same application.

Application acts as namespace for all routes, i.e all routes for specific application
has same url path prefix. Application prefix always contains leading "/" slash. 
If supplied prefix does not contain leading slash, it get inserted. 
Prefix should consists of value path segments. i.e for application with prefix `/app` 
any request with following paths `/app`, `/app/` or `/app/test` would match,
but path `/application` would not match.

```rust,ignore
# extern crate actix_web;
# extern crate tokio_core;
# use actix_web::*;
# fn index(req: HttpRequest) -> &'static str {
#    "Hello world!"
# }
# fn main() {
   let app = Application::new()
       .prefix("/app")
       .resource("/index.html", |r| r.method(Method::GET).f(index))
       .finish()
# }
```

In this example application with `/app` prefix and `index.html` resource
get created. This resource is available as on `/app/index.html` url.
For more information check 
[*URL Matching*](./qs_5.html#using-a-application-prefix-to-compose-applications) section.

Multiple applications could be served with one server:

```rust
# extern crate actix_web;
# extern crate tokio_core;
# use tokio_core::net::TcpStream;
# use std::net::SocketAddr;
use actix_web::*;

fn main() {
    HttpServer::new(|| vec![
        Application::new()
            .prefix("/app1")
            .resource("/", |r| r.f(|r| httpcodes::HttpOk)),
        Application::new()
            .prefix("/app2")
            .resource("/", |r| r.f(|r| httpcodes::HttpOk)),
        Application::new()
            .resource("/", |r| r.f(|r| httpcodes::HttpOk)),
    ]);
}
```

All `/app1` requests route to first application, `/app2` to second and then all other to third.
Applications get matched based on registration order, if application with more general
prefix is registered before less generic, that would effectively block less generic
application to get matched. For example if *application* with prefix "/" get registered
as first application, it would match all incoming requests.

## State

Application state is shared with all routes and resources within same application.
State could be accessed with `HttpRequest::state()` method as a read-only item
but interior mutability pattern with `RefCell` could be used to archive state mutability.
State could be accessed with `HttpContext::state()` in case of http actor.
State also available to route matching predicates and middlewares.

Let's write simple application that uses shared state. We are going to store requests count
in the state:

```rust
# extern crate actix;
# extern crate actix_web;
#
use actix_web::*;
use std::cell::Cell;

// This struct represents state
struct AppState {
    counter: Cell<usize>,
}

fn index(req: HttpRequest<AppState>) -> String {
    let count = req.state().counter.get() + 1; // <- get count
    req.state().counter.set(count);            // <- store new count in state

    format!("Request number: {}", count)       // <- response with count
}

fn main() {
    Application::with_state(AppState{counter: Cell::new(0)})
        .resource("/", |r| r.method(Method::GET).f(index))
        .finish();
}
```

Note on application state, http server accepts application factory rather than application
instance. Http server construct application instance for each thread, so application state
must be constructed multiple times. If you want to share state between different thread
shared object should be used, like `Arc`. Application state does not need to be `Send` and `Sync`
but application factory must be `Send` + `Sync`.
