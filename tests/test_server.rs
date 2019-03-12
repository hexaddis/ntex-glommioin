use std::io::{Read, Write};
use std::time::Duration;
use std::{net, thread};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http_test::TestServer;
use actix_server_config::ServerConfig;
use actix_service::{fn_cfg_factory, NewService};
use bytes::Bytes;
use futures::future::{self, ok, Future};
use futures::stream::once;

use actix_http::body::Body;
use actix_http::{
    body, client, error, http, http::header, Error, HttpMessage as HttpMessage2,
    HttpService, KeepAlive, Request, Response,
};

#[test]
fn test_h1() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let req = client::ClientRequest::get(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
}

#[test]
fn test_h1_2() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .client_timeout(1000)
            .client_disconnect(1000)
            .finish(|req: Request| {
                assert_eq!(req.version(), http::Version::HTTP_11);
                future::ok::<_, ()>(Response::Ok().finish())
            })
            .map(|_| ())
    });

    let req = client::ClientRequest::get(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
}

#[cfg(feature = "ssl")]
fn ssl_acceptor<T: AsyncRead + AsyncWrite>(
) -> std::io::Result<actix_server::ssl::OpensslAcceptor<T, ()>> {
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("tests/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("tests/cert.pem")
        .unwrap();
    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(openssl::ssl::AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x02h2")?;
    Ok(actix_server::ssl::OpensslAcceptor::new(builder.build()))
}

#[cfg(feature = "ssl")]
#[test]
fn test_h2() -> std::io::Result<()> {
    let openssl = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| future::ok::<_, Error>(Response::Ok().finish()))
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::get(srv.surl("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[cfg(feature = "ssl")]
#[test]
fn test_h2_1() -> std::io::Result<()> {
    let openssl = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .finish(|req: Request| {
                        assert_eq!(req.version(), http::Version::HTTP_2);
                        future::ok::<_, Error>(Response::Ok().finish())
                    })
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::get(srv.surl("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    Ok(())
}

#[cfg(feature = "ssl")]
#[test]
fn test_h2_body() -> std::io::Result<()> {
    let data = "HELLOWORLD".to_owned().repeat(64 * 1024);
    let openssl = ssl_acceptor()?;
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|mut req: Request<_>| {
                        req.body()
                            .limit(1024 * 1024)
                            .and_then(|body| Ok(Response::Ok().body(body)))
                    })
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::get(srv.surl("/"))
        .body(data.clone())
        .unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    let body = srv.block_on(response.body().limit(1024 * 1024)).unwrap();
    assert_eq!(&body, data.as_bytes());
    Ok(())
}

#[test]
fn test_slow_request() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .client_timeout(100)
            .finish(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 408 Request Timeout"));
}

#[test]
fn test_http1_malformed_request() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP1.1\r\n");
    let mut data = String::new();
    let _ = stream.read_to_string(&mut data);
    assert!(data.starts_with("HTTP/1.1 400 Bad Request"));
}

#[test]
fn test_http1_keepalive() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
}

#[test]
fn test_http1_keepalive_timeout() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(1)
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
    thread::sleep(Duration::from_millis(1100));

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_http1_keepalive_close() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ =
        stream.write_all(b"GET /test/tests/test HTTP/1.1\r\nconnection: close\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_http10_keepalive_default_close() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_http10_keepalive() {
    let srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream
        .write_all(b"GET /test/tests/test HTTP/1.0\r\nconnection: keep-alive\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.0\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.0 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_http1_keepalive_disabled() {
    let srv = TestServer::new(|| {
        HttpService::build()
            .keep_alive(KeepAlive::Disabled)
            .h1(|_| future::ok::<_, ()>(Response::Ok().finish()))
    });

    let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
    let _ = stream.write_all(b"GET /test/tests/test HTTP/1.1\r\n\r\n");
    let mut data = vec![0; 1024];
    let _ = stream.read(&mut data);
    assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");

    let mut data = vec![0; 1024];
    let res = stream.read(&mut data).unwrap();
    assert_eq!(res, 0);
}

#[test]
fn test_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };

    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|req: Request| {
            let indx: usize = req.uri().path()[1..].parse().unwrap();
            let statuses = [
                StatusCode::NO_CONTENT,
                StatusCode::CONTINUE,
                StatusCode::SWITCHING_PROTOCOLS,
                StatusCode::PROCESSING,
                StatusCode::OK,
                StatusCode::NOT_FOUND,
            ];
            future::ok::<_, ()>(Response::new(statuses[indx]))
        })
    });

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = client::ClientRequest::get(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = client::ClientRequest::head(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = client::ClientRequest::get(srv.url(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_h2_content_length() {
    use actix_http::http::{
        header::{HeaderName, HeaderValue},
        StatusCode,
    };
    let openssl = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|req: Request| {
                        let indx: usize = req.uri().path()[1..].parse().unwrap();
                        let statuses = [
                            StatusCode::NO_CONTENT,
                            StatusCode::CONTINUE,
                            StatusCode::SWITCHING_PROTOCOLS,
                            StatusCode::PROCESSING,
                            StatusCode::OK,
                            StatusCode::NOT_FOUND,
                        ];
                        future::ok::<_, ()>(Response::new(statuses[indx]))
                    })
                    .map_err(|_| ()),
            )
    });

    let header = HeaderName::from_static("content-length");
    let value = HeaderValue::from_static("0");

    {
        for i in 0..4 {
            let req = client::ClientRequest::get(srv.surl(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);

            let req = client::ClientRequest::head(srv.surl(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), None);
        }

        for i in 4..6 {
            let req = client::ClientRequest::get(srv.surl(&format!("/{}", i)))
                .finish()
                .unwrap();
            let response = srv.send_request(req).unwrap();
            assert_eq!(response.headers().get(&header), Some(&value));
        }
    }
}

#[test]
fn test_h1_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();

    let mut srv = TestServer::new(move || {
        let data = data.clone();
        HttpService::build().h1(move |_| {
            let mut builder = Response::Ok();
            for idx in 0..90 {
                builder.header(
                    format!("X-TEST-{}", idx).as_str(),
                    "TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST ",
                );
            }
            future::ok::<_, ()>(builder.body(data.clone()))
        })
    });
    let mut connector = srv.new_connector();

    let req = srv.get().finish().unwrap();

    let mut response = srv.block_on(req.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data2));
}

#[test]
fn test_h2_headers() {
    let data = STR.repeat(10);
    let data2 = data.clone();
    let openssl = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        let data = data.clone();
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
        HttpService::build().h2(move |_| {
            let mut builder = Response::Ok();
            for idx in 0..90 {
                builder.header(
                    format!("X-TEST-{}", idx).as_str(),
                    "TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST \
                        TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST TEST ",
                );
            }
            future::ok::<_, ()>(builder.body(data.clone()))
        }).map_err(|_| ()))
    });
    let mut connector = srv.new_connector();

    let req = client::ClientRequest::get(srv.surl("/")).finish().unwrap();
    let mut response = srv.block_on(req.send(&mut connector)).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from(data2));
}

const STR: &str = "Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World \
                   Hello World Hello World Hello World Hello World Hello World";

#[test]
fn test_h1_body() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_body2() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| future::ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let req = srv.sget().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_head_empty() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_head_empty() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .finish(|_| ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::head(srv.surl("/")).finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.version(), http::Version::HTTP_2);

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h1_head_binary() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            ok::<_, ()>(Response::Ok().content_length(STR.len() as u64).body(STR))
        })
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_head_binary() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        ok::<_, ()>(
                            Response::Ok().content_length(STR.len() as u64).body(STR),
                        )
                    })
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::head(srv.surl("/")).finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h1_head_binary2() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| ok::<_, ()>(Response::Ok().body(STR)))
    });

    let req = client::ClientRequest::head(srv.url("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }
}

#[test]
fn test_h2_head_binary2() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| ok::<_, ()>(Response::Ok().body(STR)))
                    .map_err(|_| ()),
            )
    });

    let req = client::ClientRequest::head(srv.surl("/")).finish().unwrap();
    let response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    {
        let len = response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap();
        assert_eq!(format!("{}", STR.len()), len.to_str().unwrap());
    }
}

#[test]
fn test_h1_body_length() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(
                Response::Ok()
                    .body(Body::from_message(body::SizedStream::new(STR.len(), body))),
            )
        })
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_body_length() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        let body = once(Ok(Bytes::from_static(STR.as_ref())));
                        ok::<_, ()>(Response::Ok().body(Body::from_message(
                            body::SizedStream::new(STR.len(), body),
                        )))
                    })
                    .map_err(|_| ()),
            )
    });

    let req = srv.sget().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_body_chunked_explicit() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(
                Response::Ok()
                    .header(header::TRANSFER_ENCODING, "chunked")
                    .streaming(body),
            )
        })
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get(header::TRANSFER_ENCODING)
            .unwrap()
            .to_str()
            .unwrap(),
        "chunked"
    );

    // read response
    let bytes = srv.block_on(response.body()).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h2_body_chunked_explicit() {
    let openssl = ssl_acceptor().unwrap();
    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| {
                        let body =
                            once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
                        ok::<_, ()>(
                            Response::Ok()
                                .header(header::TRANSFER_ENCODING, "chunked")
                                .streaming(body),
                        )
                    })
                    .map_err(|_| ()),
            )
    });

    let req = srv.sget().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    assert!(!response.headers().contains_key(header::TRANSFER_ENCODING));

    // read response
    let bytes = srv.block_on(response.body()).unwrap();

    // decode
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_body_chunked_implicit() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(|_| {
            let body = once::<_, Error>(Ok(Bytes::from_static(STR.as_ref())));
            ok::<_, ()>(Response::Ok().streaming(body))
        })
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response
            .headers()
            .get(header::TRANSFER_ENCODING)
            .unwrap()
            .to_str()
            .unwrap(),
        "chunked"
    );

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert_eq!(bytes, Bytes::from_static(STR.as_ref()));
}

#[test]
fn test_h1_response_http_error_handling() {
    let mut srv = TestServer::new(|| {
        HttpService::build().h1(fn_cfg_factory(|_: &ServerConfig| {
            Ok::<_, ()>(|_| {
                let broken_header = Bytes::from_static(b"\0\0\0");
                ok::<_, ()>(
                    Response::Ok()
                        .header(http::header::CONTENT_TYPE, broken_header)
                        .body(STR),
                )
            })
        }))
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_response_http_error_handling() {
    let openssl = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(fn_cfg_factory(|_: &ServerConfig| {
                        Ok::<_, ()>(|_| {
                            let broken_header = Bytes::from_static(b"\0\0\0");
                            ok::<_, ()>(
                                Response::Ok()
                                    .header(http::header::CONTENT_TYPE, broken_header)
                                    .body(STR),
                            )
                        })
                    }))
                    .map_err(|_| ()),
            )
    });

    let req = srv.sget().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h1_service_error() {
    let mut srv = TestServer::new(|| {
        HttpService::build()
            .h1(|_| Err::<Response, Error>(error::ErrorBadRequest("error")))
    });

    let req = srv.get().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn test_h2_service_error() {
    let openssl = ssl_acceptor().unwrap();

    let mut srv = TestServer::new(move || {
        openssl
            .clone()
            .map_err(|e| println!("Openssl error: {}", e))
            .and_then(
                HttpService::build()
                    .h2(|_| Err::<Response, Error>(error::ErrorBadRequest("error")))
                    .map_err(|_| ()),
            )
    });

    let req = srv.sget().finish().unwrap();
    let mut response = srv.send_request(req).unwrap();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

    // read response
    let bytes = srv.block_on(response.body()).unwrap();
    assert!(bytes.is_empty());
}
