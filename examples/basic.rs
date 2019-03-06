use futures::IntoFuture;

use actix_web::{
    http::Method, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer,
};

fn index(req: HttpRequest) -> &'static str {
    println!("REQ: {:?}", req);
    "Hello world!\r\n"
}

fn index_async(req: HttpRequest) -> impl IntoFuture<Item = &'static str, Error = Error> {
    println!("REQ: {:?}", req);
    Ok("Hello world!\r\n")
}

fn no_params() -> &'static str {
    "Hello world!\r\n"
}

fn main() -> std::io::Result<()> {
    ::std::env::set_var("RUST_LOG", "actix_server=info,actix_web2=info");
    env_logger::init();
    let sys = actix_rt::System::new("hello-world");

    HttpServer::new(|| {
        App::new()
            .middleware(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
            .middleware(middleware::Compress::default())
            .resource("/resource1/index.html", |r| r.route(web::get().to(index)))
            .resource("/resource2/index.html", |r| {
                r.middleware(
                    middleware::DefaultHeaders::new().header("X-Version-R2", "0.3"),
                )
                .default_resource(|r| {
                    r.route(web::route().to(|| HttpResponse::MethodNotAllowed()))
                })
                .route(web::method(Method::GET).to_async(index_async))
            })
            .resource("/test1.html", |r| r.to(|| "Test\r\n"))
            .resource("/", |r| r.to(no_params))
    })
    .bind("127.0.0.1:8080")?
    .workers(1)
    .start();

    let _ = sys.run();
    Ok(())
}
