//! Actix web r2d2 example
extern crate serde;
extern crate serde_json;
extern crate uuid;
extern crate futures;
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate r2d2;
extern crate r2d2_sqlite;
extern crate rusqlite;

use actix::*;
use actix_web::*;
use futures::future::Future;
use r2d2_sqlite::SqliteConnectionManager;

mod db;
use db::{CreateUser, DbExecutor};


/// State with DbExecutor address
struct State {
    db: Addr<Syn, DbExecutor>,
}

/// Async request handler
fn index(req: HttpRequest<State>) -> Box<Future<Item=HttpResponse, Error=Error>> {
    let name = &req.match_info()["name"];

    req.state().db.send(CreateUser{name: name.to_owned()})
        .from_err()
        .and_then(|res| {
            match res {
                Ok(user) => Ok(httpcodes::HTTPOk.build().json(user)?),
                Err(_) => Ok(httpcodes::HTTPInternalServerError.into())
            }
        })
        .responder()
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=debug");
    let _ = env_logger::init();
    let sys = actix::System::new("r2d2-example");

    let manager = SqliteConnectionManager::file("test.db");
    let pool = r2d2::Pool::new(manager).unwrap();

    // Start db executor actors
    let addr = SyncArbiter::start(3, move || DbExecutor(pool.clone()));

    // Start http server
    let _addr = HttpServer::new(move || {
        Application::with_state(State{db: addr.clone()})
            // enable logger
            .middleware(middleware::Logger::default())
            .resource("/{name}", |r| r.method(Method::GET).a(index))})
        .bind("127.0.0.1:8080").unwrap()
        .start();

    let _ = sys.run();
}
