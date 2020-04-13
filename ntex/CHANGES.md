# Changes

## [0.1.10] - 2020-04-13

* ntex::channel: mpsc::Sender::close() must close receiver

## [0.1.9] - 2020-04-13

* ntex::util: Refcator framed dispatcher

* ntex::framed: Use framed dispatcher instead of custom one

* ntex::channel: Fix mpsc::Sender close method.

## [0.1.8] - 2020-04-12

* ntex::web: Fix definition of `ok_service` and `default_service`.

* ntex::web: Add default error impl for `http::PayloadError`

* ntex::web: Add default error impl for `http::client::SendRequestError`

* ntex::web: Move `web::Data` to `web::types::Data`

* ntex::web: Simplify Responder trait

* ntex::web: Simplify WebResponse, remove `B` generic parameter

## [0.1.7] - 2020-04-10

* ntex::http: Fix handling of large http messages

* ntex::http: Refine read/write back-pressure for h1 dispatcher

* ntex::web: Restore proc macros for handler registration

## [0.1.6] - 2020-04-09

* ntex::web: Allow to add multiple services at once

* ntex::http: Remove ResponseBuilder::json2 method

## [0.1.5] - 2020-04-07

* ntex::http: enable client disconnect timeout by default

* ntex::http: properly close h1 connection

* ntex::framed: add connection disconnect timeout to framed service

## [0.1.4] - 2020-04-06

* Remove unneeded RefCell from client connector

* Add trace entries for http1 disaptcher

* Properly set timeout for test http client

## [0.1.3] - 2020-04-06

* Add server ssl handshake timeout

* Simplify server ssl erroor

## [0.1.2] - 2020-04-05

* HTTP1 dispatcher refactoring

* Replace net2 with socket2 crate

## [0.1.1] - 2020-04-01

* Project fork
