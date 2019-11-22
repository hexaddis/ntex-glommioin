use actix_http::HttpService;
use actix_http_test::{block_on, TestServer};
use actix_web::{http, web::Path, App, HttpResponse, Responder};
use actix_web_codegen::{connect, delete, get, head, options, patch, post, put, trace};
use futures::{future, Future};

#[get("/test")]
async fn test() -> impl Responder {
    HttpResponse::Ok()
}

#[put("/test")]
async fn put_test() -> impl Responder {
    HttpResponse::Created()
}

#[patch("/test")]
async fn patch_test() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/test")]
async fn post_test() -> impl Responder {
    HttpResponse::NoContent()
}

#[head("/test")]
async fn head_test() -> impl Responder {
    HttpResponse::Ok()
}

#[connect("/test")]
async fn connect_test() -> impl Responder {
    HttpResponse::Ok()
}

#[options("/test")]
async fn options_test() -> impl Responder {
    HttpResponse::Ok()
}

#[trace("/test")]
async fn trace_test() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/test")]
fn auto_async() -> impl Future<Output = Result<HttpResponse, actix_web::Error>> {
    future::ok(HttpResponse::Ok().finish())
}

#[get("/test")]
fn auto_sync() -> impl Future<Output = Result<HttpResponse, actix_web::Error>> {
    future::ok(HttpResponse::Ok().finish())
}

#[put("/test/{param}")]
async fn put_param_test(_: Path<String>) -> impl Responder {
    HttpResponse::Created()
}

#[delete("/test/{param}")]
async fn delete_param_test(_: Path<String>) -> impl Responder {
    HttpResponse::NoContent()
}

#[get("/test/{param}")]
async fn get_param_test(_: Path<String>) -> impl Responder {
    HttpResponse::Ok()
}

#[test]
fn test_params() {
    block_on(async {
        let srv = TestServer::start(|| {
            HttpService::new(
                App::new()
                    .service(get_param_test)
                    .service(put_param_test)
                    .service(delete_param_test),
            )
        });

        let request = srv.request(http::Method::GET, srv.url("/test/it"));
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);

        let request = srv.request(http::Method::PUT, srv.url("/test/it"));
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), http::StatusCode::CREATED);

        let request = srv.request(http::Method::DELETE, srv.url("/test/it"));
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
    })
}

#[test]
fn test_body() {
    block_on(async {
        let srv = TestServer::start(|| {
            HttpService::new(
                App::new()
                    .service(post_test)
                    .service(put_test)
                    .service(head_test)
                    .service(connect_test)
                    .service(options_test)
                    .service(trace_test)
                    .service(patch_test)
                    .service(test),
            )
        });
        let request = srv.request(http::Method::GET, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::HEAD, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::CONNECT, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::OPTIONS, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::TRACE, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::PATCH, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let request = srv.request(http::Method::PUT, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.status(), http::StatusCode::CREATED);

        let request = srv.request(http::Method::POST, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.status(), http::StatusCode::NO_CONTENT);

        let request = srv.request(http::Method::GET, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
    })
}

#[test]
fn test_auto_async() {
    block_on(async {
        let srv = TestServer::start(|| HttpService::new(App::new().service(auto_async)));

        let request = srv.request(http::Method::GET, srv.url("/test"));
        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
    })
}
