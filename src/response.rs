//! Http response
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::{fmt, mem, str};

use bytes::{BufMut, Bytes, BytesMut};
use cookie::{Cookie, CookieJar};
use futures::Stream;
use http::header::{self, HeaderName, HeaderValue};
use http::{Error as HttpError, HeaderMap, HttpTryFrom, StatusCode, Version};
use serde::Serialize;
use serde_json;

use body::Body;
use error::Error;
use header::{ContentEncoding, Header, IntoHeaderValue};
use message::{Head, MessageFlags, ResponseHead};

/// max write buffer size 64k
pub(crate) const MAX_WRITE_BUFFER_SIZE: usize = 65_536;

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    /// Close connection after response
    Close,
    /// Keep connection alive after response
    KeepAlive,
    /// Connection is upgraded to different type
    Upgrade,
}

/// An HTTP Response
pub struct Response(Box<InnerResponse>);

impl Response {
    #[inline]
    fn get_ref(&self) -> &InnerResponse {
        self.0.as_ref()
    }

    #[inline]
    fn get_mut(&mut self) -> &mut InnerResponse {
        self.0.as_mut()
    }

    /// Create http response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> ResponseBuilder {
        ResponsePool::get(status)
    }

    /// Create http response builder
    #[inline]
    pub fn build_from<T: Into<ResponseBuilder>>(source: T) -> ResponseBuilder {
        source.into()
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode) -> Response {
        ResponsePool::with_body(status, Body::Empty)
    }

    /// Constructs a response with body
    #[inline]
    pub fn with_body<B: Into<Body>>(status: StatusCode, body: B) -> Response {
        ResponsePool::with_body(status, body.into())
    }

    /// Constructs an error response
    #[inline]
    pub fn from_error(error: Error) -> Response {
        let mut resp = error.as_response_error().error_response();
        resp.get_mut().error = Some(error);
        resp
    }

    /// Convert `Response` to a `ResponseBuilder`
    #[inline]
    pub fn into_builder(self) -> ResponseBuilder {
        // If this response has cookies, load them into a jar
        let mut jar: Option<CookieJar> = None;
        for c in self.cookies() {
            if let Some(ref mut j) = jar {
                j.add_original(c.into_owned());
            } else {
                let mut j = CookieJar::new();
                j.add_original(c.into_owned());
                jar = Some(j);
            }
        }

        ResponseBuilder {
            response: Some(self.0),
            err: None,
            cookies: jar,
        }
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.get_ref().error.as_ref()
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.get_ref().head.headers
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.get_mut().head.headers
    }

    /// Get an iterator for the cookies set by this response
    #[inline]
    pub fn cookies(&self) -> CookieIter {
        CookieIter {
            iter: self
                .get_ref()
                .head
                .headers
                .get_all(header::SET_COOKIE)
                .iter(),
        }
    }

    /// Add a cookie to this response
    #[inline]
    pub fn add_cookie(&mut self, cookie: &Cookie) -> Result<(), HttpError> {
        let h = &mut self.get_mut().head.headers;
        HeaderValue::from_str(&cookie.to_string())
            .map(|c| {
                h.append(header::SET_COOKIE, c);
            }).map_err(|e| e.into())
    }

    /// Remove all cookies with the given name from this response. Returns
    /// the number of cookies removed.
    #[inline]
    pub fn del_cookie(&mut self, name: &str) -> usize {
        let h = &mut self.get_mut().head.headers;
        let vals: Vec<HeaderValue> = h
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_owned())
            .collect();
        h.remove(header::SET_COOKIE);

        let mut count: usize = 0;
        for v in vals {
            if let Ok(s) = v.to_str() {
                if let Ok(c) = Cookie::parse_encoded(s) {
                    if c.name() == name {
                        count += 1;
                        continue;
                    }
                }
            }
            h.append(header::SET_COOKIE, v);
        }
        count
    }

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.get_ref().head.status
    }

    /// Set the `StatusCode` for this response
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.get_mut().head.status
    }

    /// Get custom reason for the response
    #[inline]
    pub fn reason(&self) -> &str {
        if let Some(reason) = self.get_ref().head.reason {
            reason
        } else {
            self.get_ref()
                .head
                .status
                .canonical_reason()
                .unwrap_or("<unknown status code>")
        }
    }

    /// Set the custom reason for the response
    #[inline]
    pub fn set_reason(&mut self, reason: &'static str) -> &mut Self {
        self.get_mut().head.reason = Some(reason);
        self
    }

    /// Set connection type
    pub fn set_connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        self.get_mut().connection_type = Some(conn);
        self
    }

    /// Connection upgrade status
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.get_ref().connection_type == Some(ConnectionType::Upgrade)
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> Option<bool> {
        if let Some(ct) = self.get_ref().connection_type {
            match ct {
                ConnectionType::KeepAlive => Some(true),
                ConnectionType::Close | ConnectionType::Upgrade => Some(false),
            }
        } else {
            None
        }
    }

    /// is chunked encoding enabled
    #[inline]
    pub fn chunked(&self) -> Option<bool> {
        self.get_ref().chunked
    }

    /// Content encoding
    #[inline]
    pub fn content_encoding(&self) -> Option<ContentEncoding> {
        self.get_ref().encoding
    }

    /// Set content encoding
    pub fn set_content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        self.get_mut().encoding = Some(enc);
        self
    }

    /// Get body os this response
    #[inline]
    pub fn body(&self) -> &Body {
        &self.get_ref().body
    }

    /// Set a body
    pub fn set_body<B: Into<Body>>(&mut self, body: B) {
        self.get_mut().body = body.into();
    }

    /// Set a body and return previous body value
    pub fn replace_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.get_mut().body, body.into())
    }

    /// Size of response in bytes, excluding HTTP headers
    pub fn response_size(&self) -> u64 {
        self.get_ref().response_size
    }

    /// Set content encoding
    pub(crate) fn set_response_size(&mut self, size: u64) {
        self.get_mut().response_size = size;
    }

    /// Set write buffer capacity
    pub fn write_buffer_capacity(&self) -> usize {
        self.get_ref().write_capacity
    }

    /// Set write buffer capacity
    pub fn set_write_buffer_capacity(&mut self, cap: usize) {
        self.get_mut().write_capacity = cap;
    }

    pub(crate) fn release(self) {
        ResponsePool::release(self.0);
    }

    pub(crate) fn into_parts(self) -> ResponseParts {
        self.0.into_parts()
    }

    pub(crate) fn from_parts(parts: ResponseParts) -> Response {
        Response(Box::new(InnerResponse::from_parts(parts)))
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(
            f,
            "\nResponse {:?} {}{}",
            self.get_ref().head.version,
            self.get_ref().head.status,
            self.get_ref().head.reason.unwrap_or("")
        );
        let _ = writeln!(f, "  encoding: {:?}", self.get_ref().encoding);
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.get_ref().head.headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

pub struct CookieIter<'a> {
    iter: header::ValueIter<'a, HeaderValue>,
}

impl<'a> Iterator for CookieIter<'a> {
    type Item = Cookie<'a>;

    #[inline]
    fn next(&mut self) -> Option<Cookie<'a>> {
        for v in self.iter.by_ref() {
            if let Ok(c) = Cookie::parse_encoded(v.to_str().ok()?) {
                return Some(c);
            }
        }
        None
    }
}

/// An HTTP response builder
///
/// This type can be used to construct an instance of `Response` through a
/// builder-like pattern.
pub struct ResponseBuilder {
    response: Option<Box<InnerResponse>>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
}

impl ResponseBuilder {
    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.head.status = status;
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, Request, Response, Result};
    ///
    /// fn index(req: HttpRequest) -> Result<Response> {
    ///     Ok(Response::Ok()
    ///         .set(http::header::IfModifiedSince(
    ///             "Sun, 07 Nov 1994 08:48:37 GMT".parse()?,
    ///         ))
    ///         .finish())
    /// }
    /// fn main() {}
    /// ```
    #[doc(hidden)]
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match hdr.try_into() {
                Ok(value) => {
                    parts.head.headers.append(H::name(), value);
                }
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, Request, Response};
    ///
    /// fn index(req: HttpRequest) -> Response {
    ///     Response::Ok()
    ///         .header("X-TEST", "value")
    ///         .header(http::header::CONTENT_TYPE, "application/json")
    ///         .finish()
    /// }
    /// fn main() {}
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        parts.head.headers.append(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(&mut self, reason: &'static str) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.head.reason = Some(reason);
        }
        self
    }

    /// Set content encoding.
    ///
    /// By default `ContentEncoding::Auto` is used, which automatically
    /// negotiates content encoding based on request's `Accept-Encoding`
    /// headers. To enforce specific encoding, use specific
    /// ContentEncoding` value.
    #[inline]
    pub fn content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.encoding = Some(enc);
        }
        self
    }

    /// Set connection type
    #[inline]
    #[doc(hidden)]
    pub fn connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.connection_type = Some(conn);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    #[doc(hidden)]
    pub fn upgrade(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Upgrade)
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Close)
    }

    /// Enables automatic chunked transfer encoding
    #[inline]
    pub fn chunked(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.chunked = Some(true);
        }
        self
    }

    /// Force disable chunked encoding
    #[inline]
    pub fn no_chunking(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.chunked = Some(false);
        }
        self
    }

    /// Set response content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => {
                    parts.head.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(&mut self, len: u64) -> &mut Self {
        let mut wrt = BytesMut::new().writer();
        let _ = write!(wrt, "{}", len);
        self.header(header::CONTENT_LENGTH, wrt.get_mut().take().freeze())
    }

    /// Set a cookie
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, HttpRequest, Response, Result};
    ///
    /// fn index(req: HttpRequest) -> Response {
    ///     Response::Ok()
    ///         .cookie(
    ///             http::Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish(),
    ///         )
    ///         .finish()
    /// }
    /// ```
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Remove cookie
    ///
    /// ```rust,ignore
    /// # extern crate actix_web;
    /// use actix_web::{http, HttpRequest, Response, Result};
    ///
    /// fn index(req: &HttpRequest) -> Response {
    ///     let mut builder = Response::Ok();
    ///
    ///     if let Some(ref cookie) = req.cookie("name") {
    ///         builder.del_cookie(cookie);
    ///     }
    ///
    ///     builder.finish()
    /// }
    /// ```
    pub fn del_cookie<'a>(&mut self, cookie: &Cookie<'a>) -> &mut Self {
        {
            if self.cookies.is_none() {
                self.cookies = Some(CookieJar::new())
            }
            let jar = self.cookies.as_mut().unwrap();
            let cookie = cookie.clone().into_owned();
            jar.add_original(cookie.clone());
            jar.remove(cookie);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is
    /// true.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
    where
        F: FnOnce(&mut ResponseBuilder),
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is
    /// Some.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
    where
        F: FnOnce(T, &mut ResponseBuilder),
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Set write buffer capacity
    ///
    /// This parameter makes sense only for streaming response
    /// or actor. If write buffer reaches specified capacity, stream or actor
    /// get paused.
    ///
    /// Default write buffer capacity is 64kb
    pub fn write_buffer_capacity(&mut self, cap: usize) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.write_capacity = cap;
        }
        self
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Response {
        if let Some(e) = self.err.take() {
            return Error::from(e).into();
        }
        let mut response = self.response.take().expect("cannot reuse response builder");
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                match HeaderValue::from_str(&cookie.to_string()) {
                    Ok(val) => response.head.headers.append(header::SET_COOKIE, val),
                    Err(e) => return Error::from(e).into(),
                };
            }
        }
        response.body = body.into();
        Response(response)
    }

    #[inline]
    /// Set a streaming body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn streaming<S, E>(&mut self, stream: S) -> Response
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error>,
    {
        self.body(Body::Streaming(Box::new(stream.map_err(|e| e.into()))))
    }

    /// Set a json body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn json<T: Serialize>(&mut self, value: T) -> Response {
        self.json2(&value)
    }

    /// Set a json body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn json2<T: Serialize>(&mut self, value: &T) -> Response {
        match serde_json::to_string(value) {
            Ok(body) => {
                let contains = if let Some(parts) = parts(&mut self.response, &self.err)
                {
                    parts.head.headers.contains_key(header::CONTENT_TYPE)
                } else {
                    true
                };
                if !contains {
                    self.header(header::CONTENT_TYPE, "application/json");
                }

                self.body(body)
            }
            Err(e) => Error::from(e).into(),
        }
    }

    #[inline]
    /// Set an empty body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn finish(&mut self) -> Response {
        self.body(Body::Empty)
    }

    /// This method construct new `ResponseBuilder`
    pub fn take(&mut self) -> ResponseBuilder {
        ResponseBuilder {
            response: self.response.take(),
            err: self.err.take(),
            cookies: self.cookies.take(),
        }
    }
}

#[inline]
#[cfg_attr(feature = "cargo-clippy", allow(borrowed_box))]
fn parts<'a>(
    parts: &'a mut Option<Box<InnerResponse>>,
    err: &Option<HttpError>,
) -> Option<&'a mut Box<InnerResponse>> {
    if err.is_some() {
        return None;
    }
    parts.as_mut()
}

/// Helper converters
impl<I: Into<Response>, E: Into<Error>> From<Result<I, E>> for Response {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into().into(),
        }
    }
}

impl From<ResponseBuilder> for Response {
    fn from(mut builder: ResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<&'static str> for Response {
    fn from(val: &'static str) -> Self {
        Response::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl From<&'static [u8]> for Response {
    fn from(val: &'static [u8]) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

impl From<String> for Response {
    fn from(val: String) -> Self {
        Response::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl<'a> From<&'a String> for Response {
    fn from(val: &'a String) -> Self {
        Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl From<Bytes> for Response {
    fn from(val: Bytes) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

impl From<BytesMut> for Response {
    fn from(val: BytesMut) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

struct InnerResponse {
    head: ResponseHead,
    body: Body,
    chunked: Option<bool>,
    encoding: Option<ContentEncoding>,
    connection_type: Option<ConnectionType>,
    write_capacity: usize,
    response_size: u64,
    error: Option<Error>,
    pool: &'static ResponsePool,
}

pub(crate) struct ResponseParts {
    head: ResponseHead,
    body: Option<Bytes>,
    encoding: Option<ContentEncoding>,
    connection_type: Option<ConnectionType>,
    error: Option<Error>,
}

impl InnerResponse {
    #[inline]
    fn new(
        status: StatusCode,
        body: Body,
        pool: &'static ResponsePool,
    ) -> InnerResponse {
        InnerResponse {
            head: ResponseHead {
                status,
                version: Version::default(),
                headers: HeaderMap::with_capacity(16),
                reason: None,
                flags: MessageFlags::empty(),
            },
            body,
            pool,
            chunked: None,
            encoding: None,
            connection_type: None,
            response_size: 0,
            write_capacity: MAX_WRITE_BUFFER_SIZE,
            error: None,
        }
    }

    /// This is for failure, we can not have Send + Sync on Streaming and Actor response
    fn into_parts(mut self) -> ResponseParts {
        let body = match mem::replace(&mut self.body, Body::Empty) {
            Body::Empty => None,
            Body::Binary(mut bin) => Some(bin.take()),
            Body::Streaming(_) => {
                error!("Streaming or Actor body is not support by error response");
                None
            }
        };

        ResponseParts {
            body,
            head: self.head,
            encoding: self.encoding,
            connection_type: self.connection_type,
            error: self.error,
        }
    }

    fn from_parts(parts: ResponseParts) -> InnerResponse {
        let body = if let Some(ref body) = parts.body {
            Body::Binary(body.clone().into())
        } else {
            Body::Empty
        };

        InnerResponse {
            body,
            head: parts.head,
            chunked: None,
            encoding: parts.encoding,
            connection_type: parts.connection_type,
            response_size: 0,
            write_capacity: MAX_WRITE_BUFFER_SIZE,
            error: parts.error,
            pool: ResponsePool::pool(),
        }
    }
}

/// Internal use only!
pub(crate) struct ResponsePool(RefCell<VecDeque<Box<InnerResponse>>>);

thread_local!(static POOL: &'static ResponsePool = ResponsePool::pool());

impl ResponsePool {
    fn pool() -> &'static ResponsePool {
        let pool = ResponsePool(RefCell::new(VecDeque::with_capacity(128)));
        Box::leak(Box::new(pool))
    }

    pub fn get_pool() -> &'static ResponsePool {
        POOL.with(|p| *p)
    }

    #[inline]
    pub fn get_builder(
        pool: &'static ResponsePool,
        status: StatusCode,
    ) -> ResponseBuilder {
        if let Some(mut msg) = pool.0.borrow_mut().pop_front() {
            msg.head.status = status;
            ResponseBuilder {
                response: Some(msg),
                err: None,
                cookies: None,
            }
        } else {
            let msg = Box::new(InnerResponse::new(status, Body::Empty, pool));
            ResponseBuilder {
                response: Some(msg),
                err: None,
                cookies: None,
            }
        }
    }

    #[inline]
    pub fn get_response(
        pool: &'static ResponsePool,
        status: StatusCode,
        body: Body,
    ) -> Response {
        if let Some(mut msg) = pool.0.borrow_mut().pop_front() {
            msg.head.status = status;
            msg.body = body;
            Response(msg)
        } else {
            Response(Box::new(InnerResponse::new(status, body, pool)))
        }
    }

    #[inline]
    fn get(status: StatusCode) -> ResponseBuilder {
        POOL.with(|pool| ResponsePool::get_builder(pool, status))
    }

    #[inline]
    fn with_body(status: StatusCode, body: Body) -> Response {
        POOL.with(|pool| ResponsePool::get_response(pool, status, body))
    }

    #[inline]
    fn release(mut inner: Box<InnerResponse>) {
        let mut p = inner.pool.0.borrow_mut();
        if p.len() < 128 {
            inner.head.clear();
            inner.chunked = None;
            inner.encoding = None;
            inner.connection_type = None;
            inner.response_size = 0;
            inner.error = None;
            inner.write_capacity = MAX_WRITE_BUFFER_SIZE;
            p.push_front(inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use body::Binary;
    use http;
    use http::header::{HeaderValue, CONTENT_TYPE, COOKIE};

    use header::ContentEncoding;
    // use test::TestRequest;

    #[test]
    fn test_debug() {
        let resp = Response::Ok()
            .header(COOKIE, HeaderValue::from_static("cookie1=value1; "))
            .header(COOKIE, HeaderValue::from_static("cookie2=value2; "))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("Response"));
    }

    // #[test]
    // fn test_response_cookies() {
    //     let req = TestRequest::default()
    //         .header(COOKIE, "cookie1=value1")
    //         .header(COOKIE, "cookie2=value2")
    //         .finish();
    //     let cookies = req.cookies().unwrap();

    //     let resp = Response::Ok()
    //         .cookie(
    //             http::Cookie::build("name", "value")
    //                 .domain("www.rust-lang.org")
    //                 .path("/test")
    //                 .http_only(true)
    //                 .max_age(Duration::days(1))
    //                 .finish(),
    //         ).del_cookie(&cookies[0])
    //         .finish();

    //     let mut val: Vec<_> = resp
    //         .headers()
    //         .get_all("Set-Cookie")
    //         .iter()
    //         .map(|v| v.to_str().unwrap().to_owned())
    //         .collect();
    //     val.sort();
    //     assert!(val[0].starts_with("cookie1=; Max-Age=0;"));
    //     assert_eq!(
    //         val[1],
    //         "name=value; HttpOnly; Path=/test; Domain=www.rust-lang.org; Max-Age=86400"
    //     );
    // }

    #[test]
    fn test_update_response_cookies() {
        let mut r = Response::Ok()
            .cookie(http::Cookie::new("original", "val100"))
            .finish();

        r.add_cookie(&http::Cookie::new("cookie2", "val200"))
            .unwrap();
        r.add_cookie(&http::Cookie::new("cookie2", "val250"))
            .unwrap();
        r.add_cookie(&http::Cookie::new("cookie3", "val300"))
            .unwrap();

        assert_eq!(r.cookies().count(), 4);
        r.del_cookie("cookie2");

        let mut iter = r.cookies();
        let v = iter.next().unwrap();
        assert_eq!((v.name(), v.value()), ("original", "val100"));
        let v = iter.next().unwrap();
        assert_eq!((v.name(), v.value()), ("cookie3", "val300"));
    }

    #[test]
    fn test_basic_builder() {
        let resp = Response::Ok().header("X-TEST", "value").finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = Response::build(StatusCode::OK).upgrade().finish();
        assert!(resp.upgrade())
    }

    #[test]
    fn test_force_close() {
        let resp = Response::build(StatusCode::OK).force_close().finish();
        assert!(!resp.keep_alive().unwrap())
    }

    #[test]
    fn test_content_type() {
        let resp = Response::build(StatusCode::OK)
            .content_type("text/plain")
            .body(Body::Empty);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[test]
    fn test_content_encoding() {
        let resp = Response::build(StatusCode::OK).finish();
        assert_eq!(resp.content_encoding(), None);

        #[cfg(feature = "brotli")]
        {
            let resp = Response::build(StatusCode::OK)
                .content_encoding(ContentEncoding::Br)
                .finish();
            assert_eq!(resp.content_encoding(), Some(ContentEncoding::Br));
        }

        let resp = Response::build(StatusCode::OK)
            .content_encoding(ContentEncoding::Gzip)
            .finish();
        assert_eq!(resp.content_encoding(), Some(ContentEncoding::Gzip));
    }

    #[test]
    fn test_json() {
        let resp = Response::build(StatusCode::OK).json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            *resp.body(),
            Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]"))
        );
    }

    #[test]
    fn test_json_ct() {
        let resp = Response::build(StatusCode::OK)
            .header(CONTENT_TYPE, "text/json")
            .json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(
            *resp.body(),
            Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]"))
        );
    }

    #[test]
    fn test_json2() {
        let resp = Response::build(StatusCode::OK).json2(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(
            *resp.body(),
            Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]"))
        );
    }

    #[test]
    fn test_json2_ct() {
        let resp = Response::build(StatusCode::OK)
            .header(CONTENT_TYPE, "text/json")
            .json2(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(
            *resp.body(),
            Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]"))
        );
    }

    impl Body {
        pub(crate) fn bin_ref(&self) -> &Binary {
            match *self {
                Body::Binary(ref bin) => bin,
                _ => panic!(),
            }
        }
    }

    #[test]
    fn test_into_response() {
        let resp: Response = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from("test"));

        let resp: Response = b"test".as_ref().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from(b"test".as_ref()));

        let resp: Response = "test".to_owned().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from("test".to_owned()));

        let resp: Response = (&"test".to_owned()).into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from(&"test".to_owned()));

        let b = Bytes::from_static(b"test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.body().bin_ref(),
            &Binary::from(Bytes::from_static(b"test"))
        );

        let b = Bytes::from_static(b"test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from(BytesMut::from("test")));

        let b = BytesMut::from("test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), &Binary::from(BytesMut::from("test")));
    }

    #[test]
    fn test_into_builder() {
        let mut resp: Response = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);

        resp.add_cookie(&http::Cookie::new("cookie1", "val100"))
            .unwrap();

        let mut builder = resp.into_builder();
        let resp = builder.status(StatusCode::BAD_REQUEST).finish();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let cookie = resp.cookies().next().unwrap();
        assert_eq!((cookie.name(), cookie.value()), ("cookie1", "val100"));
    }
}
