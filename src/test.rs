//! Test Various helpers for Actix applications to use during testing.
use std::str::FromStr;

use bytes::Bytes;
use cookie::Cookie;
use http::header::HeaderName;
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};

use crate::header::{Header, IntoHeaderValue};
use crate::payload::Payload;
use crate::Request;

/// Test `Request` builder
///
/// ```rust,ignore
/// # extern crate http;
/// # extern crate actix_web;
/// # use http::{header, StatusCode};
/// # use actix_web::*;
/// use actix_web::test::TestRequest;
///
/// fn index(req: &HttpRequest) -> Response {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         Response::Ok().into()
///     } else {
///         Response::BadRequest().into()
///     }
/// }
///
/// fn main() {
///     let resp = TestRequest::with_header("content-type", "text/plain")
///         .run(&index)
///         .unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let resp = TestRequest::default().run(&index).unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest(Option<Inner>);

struct Inner {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    _cookies: Option<Vec<Cookie<'static>>>,
    payload: Option<Payload>,
    prefix: u16,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest(Some(Inner {
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            _cookies: None,
            payload: None,
            prefix: 0,
        }))
    }
}

impl TestRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest::default().uri(path).take()
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest {
        TestRequest::default().set(hdr).take()
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value).take()
    }

    /// Set HTTP version of this request
    pub fn version(&mut self, ver: Version) -> &mut Self {
        parts(&mut self.0).version = ver;
        self
    }

    /// Set HTTP method of this request
    pub fn method(&mut self, meth: Method) -> &mut Self {
        parts(&mut self.0).method = meth;
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(&mut self, path: &str) -> &mut Self {
        parts(&mut self.0).uri = Uri::from_str(path).unwrap();
        self
    }

    /// Set a header
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Ok(value) = hdr.try_into() {
            parts(&mut self.0).headers.append(H::name(), value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Set a header
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                parts(&mut self.0).headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(&mut self, data: B) -> &mut Self {
        let mut payload = crate::h1::Payload::empty();
        payload.unread_data(data.into());
        parts(&mut self.0).payload = Some(payload.into());
        self
    }

    pub fn take(&mut self) -> TestRequest {
        TestRequest(self.0.take())
    }

    /// Complete request creation and generate `Request` instance
    pub fn finish(&mut self) -> Request {
        let Inner {
            method,
            uri,
            version,
            headers,
            payload,
            ..
        } = self.0.take().expect("cannot reuse test request builder");;

        let mut req = if let Some(pl) = payload {
            Request::with_payload(pl)
        } else {
            Request::with_payload(crate::h1::Payload::empty().into())
        };

        let head = req.head_mut();
        head.uri = uri;
        head.method = method;
        head.version = version;
        head.headers = headers;

        // req.set_cookies(cookies);
        req
    }

    // /// This method generates `HttpRequest` instance and runs handler
    // /// with generated request.
    // pub fn run<H: Handler<S>>(self, h: &H) -> Result<Response, Error> {
    //     let req = self.finish();
    //     let resp = h.handle(&req);

    //     match resp.respond_to(&req) {
    //         Ok(resp) => match resp.into().into() {
    //             AsyncResultItem::Ok(resp) => Ok(resp),
    //             AsyncResultItem::Err(err) => Err(err),
    //             AsyncResultItem::Future(fut) => {
    //                 let mut sys = System::new("test");
    //                 sys.block_on(fut)
    //             }
    //         },
    //         Err(err) => Err(err.into()),
    //     }
    // }

    // /// This method generates `HttpRequest` instance and runs handler
    // /// with generated request.
    // ///
    // /// This method panics is handler returns actor.
    // pub fn run_async<H, R, F, E>(self, h: H) -> Result<Response, E>
    // where
    //     H: Fn(HttpRequest<S>) -> F + 'static,
    //     F: Future<Item = R, Error = E> + 'static,
    //     R: Responder<Error = E> + 'static,
    //     E: Into<Error> + 'static,
    // {
    //     let req = self.finish();
    //     let fut = h(req.clone());

    //     let mut sys = System::new("test");
    //     match sys.block_on(fut) {
    //         Ok(r) => match r.respond_to(&req) {
    //             Ok(reply) => match reply.into().into() {
    //                 AsyncResultItem::Ok(resp) => Ok(resp),
    //                 _ => panic!("Nested async replies are not supported"),
    //             },
    //             Err(e) => Err(e),
    //         },
    //         Err(err) => Err(err),
    //     }
    // }

    // /// This method generates `HttpRequest` instance and executes handler
    // pub fn run_async_result<F, R, I, E>(self, f: F) -> Result<I, E>
    // where
    //     F: FnOnce(&HttpRequest<S>) -> R,
    //     R: Into<AsyncResult<I, E>>,
    // {
    //     let req = self.finish();
    //     let res = f(&req);

    //     match res.into().into() {
    //         AsyncResultItem::Ok(resp) => Ok(resp),
    //         AsyncResultItem::Err(err) => Err(err),
    //         AsyncResultItem::Future(fut) => {
    //             let mut sys = System::new("test");
    //             sys.block_on(fut)
    //         }
    //     }
    // }

    // /// This method generates `HttpRequest` instance and executes handler
    // pub fn execute<F, R>(self, f: F) -> Result<Response, Error>
    // where
    //     F: FnOnce(&HttpRequest<S>) -> R,
    //     R: Responder + 'static,
    // {
    //     let req = self.finish();
    //     let resp = f(&req);

    //     match resp.respond_to(&req) {
    //         Ok(resp) => match resp.into().into() {
    //             AsyncResultItem::Ok(resp) => Ok(resp),
    //             AsyncResultItem::Err(err) => Err(err),
    //             AsyncResultItem::Future(fut) => {
    //                 let mut sys = System::new("test");
    //                 sys.block_on(fut)
    //             }
    //         },
    //         Err(err) => Err(err.into()),
    //     }
    // }
}

#[inline]
fn parts<'a>(parts: &'a mut Option<Inner>) -> &'a mut Inner {
    parts.as_mut().expect("cannot reuse test request builder")
}
