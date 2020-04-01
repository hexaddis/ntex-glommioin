use ntex::web::{self, middleware, App, Error, HttpRequest, HttpResponse, HttpServer};

// #[get("/resource1/{name}/index.html")]
async fn index(req: HttpRequest, name: web::types::Path<String>) -> String {
    println!("REQ: {:?}", req);
    format!("Hello: {}!\r\n", name)
}

async fn index_async(req: HttpRequest) -> Result<&'static str, Error> {
    println!("REQ: {:?}", req);
    Ok("Hello world!\r\n")
}

// #[get("/")]
async fn no_params() -> &'static str {
    "Hello world!\r\n"
}

#[cfg(unix)]
#[ntex::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    env_logger::init();

    HttpServer::new(|| {
        App::new()
            .wrap(middleware::DefaultHeaders::new().header("X-Version", "0.2"))
            .wrap(middleware::Logger::default())
            .service(web::resource("/resource1/{name}/index.html").to(index))
            .service(web::resource("/").route(web::get().to(no_params)))
            // .service(index)
            // .service(no_params)
            .service(
                web::resource("/resource2/index.html")
                    .wrap(
                        middleware::DefaultHeaders::new().header("X-Version-R2", "0.3"),
                    )
                    .default_service(
                        web::route().to(|| async { HttpResponse::MethodNotAllowed() }),
                    )
                    .route(web::get().to(index_async)),
            )
            .service(web::resource("/test1.html").to(|| async { "Test\r\n" }))
    })
    .bind_uds("/Users/fafhrd91/uds-test")?
    .workers(1)
    .run()
    .await
}

#[cfg(not(unix))]
fn main() {}
