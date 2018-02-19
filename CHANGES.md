# Changes

## 0.4.0 (2018-02-..)

* Actix 0.5 compatibility

* Fix request json loader

* Simplify HttpServer type definition

* Added HttpRequest::mime_type() method

* Added HttpRequest::uri_mut(), allows to modify request uri

* Added StaticFiles::index_file()

* Added basic websocket client

* Added TestServer::ws(), test websockets client

* Allow to override content encoding on application level


## 0.3.3 (2018-01-25)

* Stop processing any events after context stop

* Re-enable write back-pressure for h1 connections

* Refactor HttpServer::start_ssl() method

* Upgrade openssl to 0.10


## 0.3.2 (2018-01-21)

* Fix HEAD requests handling

* Log request processing errors

* Always enable content encoding if encoding explicitly selected

* Allow multiple Applications on a single server with different state #49

* CORS middleware: allowed_headers is defaulting to None #50


## 0.3.1 (2018-01-13)

* Fix directory entry path #47

* Do not enable chunked encoding for HTTP/1.0

* Allow explicitly disable chunked encoding


## 0.3.0 (2018-01-12)

* HTTP/2 Support

* Refactor streaming responses

* Refactor error handling

* Asynchronous middlewares

* Refactor logger middleware

* Content compression/decompression (br, gzip, deflate)

* Server multi-threading

* Gracefull shutdown support


## 0.2.1 (2017-11-03)

* Allow to start tls server with `HttpServer::serve_tls`

* Export `Frame` enum

* Add conversion impl from `HttpResponse` and `BinaryBody` to a `Frame`


## 0.2.0 (2017-10-30)

* Do not use `http::Uri` as it can not parse some valid paths

* Refactor response `Body`

* Refactor `RouteRecognizer` usability

* Refactor `HttpContext::write`

* Refactor `Payload` stream

* Re-use `BinaryBody` for `Frame::Payload`

* Stop http actor on `write_eof`

* Fix disconnection handling.


## 0.1.0 (2017-10-23)

* First release
