//! Simple websocket client.

#![allow(unused_variables)]
extern crate actix;
extern crate actix_web;
extern crate env_logger;
extern crate futures;
extern crate tokio_core;

use std::{io, thread};
use std::time::Duration;

use actix::*;
use futures::Future;
use actix_web::ws::{Message, WsClientError, WsClient, WsClientWriter};


fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    let _ = env_logger::init();
    let sys = actix::System::new("ws-example");

    Arbiter::handle().spawn(
        WsClient::new("http://127.0.0.1:8080/ws/")
            .connect().unwrap()
            .map_err(|e| {
                println!("Error: {}", e);
                ()
            })
            .map(|(reader, writer)| {
                let addr: Addr<Syn<_>> = ChatClient::create(|ctx| {
                    ChatClient::add_stream(reader, ctx);
                    ChatClient(writer)
                });

                // start console loop
                thread::spawn(move|| {
                    loop {
                        let mut cmd = String::new();
                        if io::stdin().read_line(&mut cmd).is_err() {
                            println!("error");
                            return
                        }
                        addr.send(ClientCommand(cmd));
                    }
                });

                ()
            })
    );

    let _ = sys.run();
}


struct ChatClient(WsClientWriter);

#[derive(Message)]
struct ClientCommand(String);

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        // start heartbeats otherwise server will disconnect after 10 seconds
        self.hb(ctx)
    }

    fn stopping(&mut self, _: &mut Context<Self>) -> bool {
        println!("Stopping");

        // Stop application on disconnect
        Arbiter::system().send(actix::msgs::SystemExit(0));

        true
    }
}

impl ChatClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(1, 0), |act, ctx| {
            act.0.ping("");
            act.hb(ctx);
        });
    }
}

/// Handle stdin commands
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, ctx: &mut Context<Self>) {
        self.0.text(msg.0.as_str())
    }
}

/// Handle server websocket messages
impl StreamHandler<Message, WsClientError> for ChatClient {

    fn handle(&mut self, msg: Message, ctx: &mut Context<Self>) {
        match msg {
            Message::Text(txt) => println!("Server: {:?}", txt),
            _ => ()
        }
    }

    fn started(&mut self, ctx: &mut Context<Self>) {
        println!("Connected");
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        println!("Server disconnected");
        ctx.stop()
    }
}
