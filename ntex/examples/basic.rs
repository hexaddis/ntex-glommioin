use ntex::http;
use ntex::web::{self, get, middleware, App, HttpRequest, HttpResponse, HttpServer};

// #[get("/resource1/{name}/index.html")]
async fn index(req: HttpRequest, name: web::types::Path<String>) -> String {
    println!("REQ: {:?}", req);
    format!("Hello: {}!\r\n", name)
}

async fn index_async(req: HttpRequest) -> &'static str {
    println!("REQ: {:?}", req);
    "Hello world!\r\n"
}

// #[get("/")]
async fn no_params() -> &'static str {
    "Hello world!\r\n"
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "ntex=trace");
    env_logger::init();

    HttpServer::new(|| {
        App::new()
            // .wrap(middleware::Logger::default())
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
    .bind("0.0.0.0:8081")?
    .workers(4)
    .keep_alive(http::KeepAlive::Disabled)
    .run()
    .await
}
