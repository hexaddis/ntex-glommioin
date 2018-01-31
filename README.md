# Actix web [![Build Status](https://travis-ci.org/actix/actix-web.svg?branch=master)](https://travis-ci.org/actix/actix-web) [![Build status](https://ci.appveyor.com/api/projects/status/kkdb4yce7qhm5w85/branch/master?svg=true)](https://ci.appveyor.com/project/fafhrd91/actix-web-hdy9d/branch/master) [![codecov](https://codecov.io/gh/actix/actix-web/branch/master/graph/badge.svg)](https://codecov.io/gh/actix/actix-web) [![crates.io](http://meritbadge.herokuapp.com/actix-web)](https://crates.io/crates/actix-web) [![Join the chat at https://gitter.im/actix/actix](https://badges.gitter.im/actix/actix.svg)](https://gitter.im/actix/actix?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

Actix web is a small, fast, pragmatic, open source rust web framework.

* Supported *HTTP/1.x* and *HTTP/2.0* protocols
* Streaming and pipelining
* Keep-alive and slow requests handling
* [WebSockets](https://actix.github.io/actix-web/actix_web/ws/index.html)
* Transparent content compression/decompression (br, gzip, deflate)
* Configurable request routing
* Graceful server shutdown
* Multipart streams
* Middlewares ([Logger](https://actix.github.io/actix-web/guide/qs_10.html#logging),
  [Session](https://actix.github.io/actix-web/guide/qs_10.html#user-sessions),
  [DefaultHeaders](https://actix.github.io/actix-web/guide/qs_10.html#default-headers),
  [CORS](https://actix.github.io/actix-web/actix_web/middleware/cors/index.html))
* Built on top of [Actix](https://github.com/actix/actix).

## Documentation

* [User Guide](http://actix.github.io/actix-web/guide/)
* [API Documentation (Development)](http://actix.github.io/actix-web/actix_web/)
* [API Documentation (Releases)](https://docs.rs/actix-web/)
* [Chat on gitter](https://gitter.im/actix/actix)
* Cargo package: [actix-web](https://crates.io/crates/actix-web)
* Minimum supported Rust version: 1.21 or later

## Example

```rust,ignore
extern crate actix_web;
use actix_web::*;

fn index(req: HttpRequest) -> String {
    format!("Hello {}!", &req.match_info()["name"])
}

fn main() {
    HttpServer::new(
        || Application::new()
            .resource("/{name}", |r| r.f(index)))
        .bind("127.0.0.1:8080").unwrap()
        .run();
}
```

### More examples

* [Basics](https://github.com/actix/actix-web/tree/master/examples/basics/)
* [Stateful](https://github.com/actix/actix-web/tree/master/examples/state/)
* [Mulitpart streams](https://github.com/actix/actix-web/tree/master/examples/multipart/)
* [Simple websocket session](https://github.com/actix/actix-web/tree/master/examples/websocket/)
* [Tera templates](https://github.com/actix/actix-web/tree/master/examples/template_tera/)
* [Diesel integration](https://github.com/actix/actix-web/tree/master/examples/diesel/)
* [SSL / HTTP/2.0](https://github.com/actix/actix-web/tree/master/examples/tls/)
* [Tcp/Websocket chat](https://github.com/actix/actix-web/tree/master/examples/websocket-chat/)
* [SockJS Server](https://github.com/actix/actix-sockjs)
* [Json](https://github.com/actix/actix-web/tree/master/examples/json/)

## Benchmarks

Some basic benchmarks could be found in this [respository](https://github.com/fafhrd91/benchmarks).

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))

at your option.

## Code of Conduct

Contribution to the actix-web crate is organized under the terms of the
Contributor Covenant, the maintainer of actix-web, @fafhrd91, promises to
intervene to uphold that code of conduct.
