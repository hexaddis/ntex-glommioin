#![cfg_attr(feature="cargo-clippy", allow(needless_pass_by_value))]
//! There are two level of statfulness in actix-web. Application has state
//! that is shared across all handlers within same Application.
//! And individual handler can have state.

extern crate actix;
extern crate actix_web;
extern crate env_logger;

use actix::*;
use actix_web::*;
use std::cell::Cell;

struct AppState {
    counter: Cell<usize>,
}

/// somple handle
fn index(req: HttpRequest<AppState>) -> HttpResponse {
    println!("{:?}", req);
    req.state().counter.set(req.state().counter.get() + 1);

    httpcodes::HTTPOk.with_body(
        format!("Num of requests: {}", req.state().counter.get()))
}

/// `MyWebSocket` counts how many messages it receives from peer,
/// websocket-client.py could be used for tests
struct MyWebSocket {
    counter: usize,
}

impl Actor for MyWebSocket {
    type Context = HttpContext<Self, AppState>;
}

impl StreamHandler<ws::Message> for MyWebSocket {}
impl Handler<ws::Message> for MyWebSocket {
    fn handle(&mut self, msg: ws::Message, ctx: &mut Self::Context)
              -> Response<Self, ws::Message>
    {
        self.counter += 1;
        println!("WS({}): {:?}", self.counter, msg);
        match msg {
            ws::Message::Ping(msg) => ws::WsWriter::pong(ctx, &msg),
            ws::Message::Text(text) => ws::WsWriter::text(ctx, &text),
            ws::Message::Binary(bin) => ws::WsWriter::binary(ctx, bin),
            ws::Message::Closed | ws::Message::Error => {
                ctx.stop();
            }
            _ => (),
        }
        Self::empty()
    }
}

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    HttpServer::new(
        Application::build("/", AppState{counter: Cell::new(0)})
            // enable logger
            .middleware(middlewares::Logger::default())
            // websocket route
            .resource("/ws/", |r| r.get(|r| ws::start(r, MyWebSocket{counter: 0})))
            // register simple handler, handle all methods
            .handler("/", index))
        .serve::<_, ()>("127.0.0.1:8080").unwrap();

    println!("Started http server: 127.0.0.1:8080");
    let _ = sys.run();
}
