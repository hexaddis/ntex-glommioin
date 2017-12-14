//! Pieces pertaining to the HTTP response.
use std::{mem, str, fmt};
use std::convert::Into;

use cookie::CookieJar;
use bytes::{Bytes, BytesMut};
use http::{StatusCode, Version, HeaderMap, HttpTryFrom, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use serde_json;
use serde::Serialize;
use cookie::Cookie;

use body::Body;
use error::Error;
use handler::Responder;
use encoding::ContentEncoding;
use httprequest::HttpRequest;

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
pub struct HttpResponse {
    version: Option<Version>,
    headers: HeaderMap,
    status: StatusCode,
    reason: Option<&'static str>,
    body: Body,
    chunked: bool,
    encoding: ContentEncoding,
    connection_type: Option<ConnectionType>,
    response_size: u64,
    error: Option<Error>,
}

impl HttpResponse {

    /// Create http response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder {
            response: Some(HttpResponse::new(status, Body::Empty)),
            err: None,
            cookies: None,
        }
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode, body: Body) -> HttpResponse {
        HttpResponse {
            version: None,
            headers: HeaderMap::with_capacity(8),
            status: status,
            reason: None,
            body: body,
            chunked: false,
            encoding: ContentEncoding::Auto,
            connection_type: None,
            response_size: 0,
            error: None,
        }
    }

    /// Constructs a error response
    #[inline]
    pub fn from_error(error: Error) -> HttpResponse {
        let mut resp = error.cause().error_response();
        resp.error = Some(error);
        resp
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Option<Version> {
        self.version
    }

    /// Get the headers from the response.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }

    /// Get custom reason for the response.
    #[inline]
    pub fn reason(&self) -> &str {
        if let Some(reason) = self.reason {
            reason
        } else {
            self.status.canonical_reason().unwrap_or("<unknown status code>")
        }
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn set_reason(&mut self, reason: &'static str) -> &mut Self {
        self.reason = Some(reason);
        self
    }

    /// Set connection type
    pub fn set_connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        self.connection_type = Some(conn);
        self
    }

    /// Connection upgrade status
    pub fn upgrade(&self) -> bool {
        self.connection_type == Some(ConnectionType::Upgrade)
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> Option<bool> {
        if let Some(ct) = self.connection_type {
            match ct {
                ConnectionType::KeepAlive => Some(true),
                ConnectionType::Close | ConnectionType::Upgrade => Some(false),
            }
        } else {
            None
        }
    }

    /// is chunked encoding enabled
    pub fn chunked(&self) -> bool {
        self.chunked
    }

    /// Content encoding
    pub fn content_encoding(&self) -> &ContentEncoding {
        &self.encoding
    }

    /// Set content encoding
    pub fn set_content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        self.encoding = enc;
        self
    }

    /// Get body os this response
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Set a body
    pub fn set_body<B: Into<Body>>(&mut self, body: B) {
        self.body = body.into();
    }

    /// Set a body and return previous body value
    pub fn replace_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.body, body.into())
    }

    /// Size of response in bytes, excluding HTTP headers
    pub fn response_size(&self) -> u64 {
        self.response_size
    }

    /// Set content encoding
    pub(crate) fn set_response_size(&mut self, size: u64) {
        self.response_size = size;
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpResponse {:?} {}{}\n",
                         self.version, self.status, self.reason.unwrap_or(""));
        let _ = write!(f, "  encoding: {:?}\n", self.encoding);
        let _ = write!(f, "  headers:\n");
        for key in self.headers.keys() {
            let vals: Vec<_> = self.headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

/// An HTTP response builder
///
/// This type can be used to construct an instance of `HttpResponse` through a
/// builder-like pattern.
#[derive(Debug)]
pub struct HttpResponseBuilder {
    response: Option<HttpResponse>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
}

impl HttpResponseBuilder {
    /// Set the HTTP version of this response.
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.version = Some(version);
        }
        self
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.status = status;
        }
        self
    }

    /// Set a header.
    #[inline]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    match HeaderValue::try_from(value) {
                        Ok(value) => { parts.headers.append(key, value); }
                        Err(e) => self.err = Some(e.into()),
                    }
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
            parts.reason = Some(reason);
        }
        self
    }

    /// Set content encoding.
    ///
    /// By default `ContentEncoding::Auto` is used, which automatically
    /// negotiates content encoding based on request's `Accept-Encoding` headers.
    /// To enforce specific encoding, use specific ContentEncoding` value.
    #[inline]
    pub fn content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.encoding = enc;
        }
        self
    }

    /// Set connection type
    #[inline]
    pub fn connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.connection_type = Some(conn);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
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
            parts.chunked = true;
        }
        self
    }

    /// Set response content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => { parts.headers.insert(header::CONTENT_TYPE, value); },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a cookie
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

    /// Remote cookie, cookie has to be cookie from `HttpRequest::cookies()` method.
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

    /// Calls provided closure with builder reference if value is true.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
        where F: Fn(&mut HttpResponseBuilder) + 'static
    {
        if value {
            f(self);
        }
        self
    }

    /// Set a body and generate `HttpResponse`.
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Result<HttpResponse, HttpError> {
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        let mut response = self.response.take().expect("cannot reuse response builder");
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                response.headers.append(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie.to_string())?);
            }
        }
        response.body = body.into();
        Ok(response)
    }

    /// Set a json body and generate `HttpResponse`
    pub fn json<T: Serialize>(&mut self, value: T) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&value)?;

        let contains = if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.headers.contains_key(header::CONTENT_TYPE)
        } else {
            true
        };
        if !contains {
            self.header(header::CONTENT_TYPE, "application/json");
        }

        Ok(self.body(body)?)
    }

    /// Set an empty body and generate `HttpResponse`
    pub fn finish(&mut self) -> Result<HttpResponse, HttpError> {
        self.body(Body::Empty)
    }
}

fn parts<'a>(parts: &'a mut Option<HttpResponse>, err: &Option<HttpError>)
             -> Option<&'a mut HttpResponse>
{
    if err.is_some() {
        return None
    }
    parts.as_mut()
}

/// Helper converters
impl<I: Into<HttpResponse>, E: Into<Error>> From<Result<I, E>> for HttpResponse {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into().into(),
        }
    }
}

impl From<HttpResponseBuilder> for HttpResponse {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish().into()
    }
}

impl Responder for HttpResponseBuilder {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(mut self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.finish()
    }
}

impl From<&'static str> for HttpResponse {
    fn from(val: &'static str) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl Responder for &'static str {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl From<&'static [u8]> for HttpResponse {
    fn from(val: &'static [u8]) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for &'static [u8] {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

impl From<String> for HttpResponse {
    fn from(val: String) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl Responder for String {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl<'a> From<&'a String> for HttpResponse {
    fn from(val: &'a String) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl<'a> Responder for &'a String {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl From<Bytes> for HttpResponse {
    fn from(val: Bytes) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for Bytes {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

impl From<BytesMut> for HttpResponse {
    fn from(val: BytesMut) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for BytesMut {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use time::Duration;
    use http::{Method, Uri};
    use body::Binary;
    use {headers, httpcodes};

    #[test]
    fn test_debug() {
        let resp = HttpResponse::Ok().finish().unwrap();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("HttpResponse"));
    }

    #[test]
    fn test_response_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE,
                       header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);
        let cookies = req.cookies().unwrap();

        let resp = httpcodes::HTTPOk
            .build()
            .cookie(headers::Cookie::build("name", "value")
                    .domain("www.rust-lang.org")
                    .path("/test")
                    .http_only(true)
                    .max_age(Duration::days(1))
                    .finish())
            .del_cookie(&cookies[0])
            .body(Body::Empty);

        assert!(resp.is_ok());
        let resp = resp.unwrap();

        let mut val: Vec<_> = resp.headers().get_all("Set-Cookie")
            .iter().map(|v| v.to_str().unwrap().to_owned()).collect();
        val.sort();
        assert!(val[0].starts_with("cookie1=; Max-Age=0;"));
        assert_eq!(
            val[1],"name=value; HttpOnly; Path=/test; Domain=www.rust-lang.org; Max-Age=86400");
    }

    #[test]
    fn test_basic_builder() {
        let resp = HttpResponse::Ok()
            .status(StatusCode::NO_CONTENT)
            .header("X-TEST", "value")
            .version(Version::HTTP_10)
            .finish().unwrap();
        assert_eq!(resp.version(), Some(Version::HTTP_10));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn test_upgrade() {
        let resp = HttpResponse::build(StatusCode::OK)
            .upgrade().body(Body::Empty).unwrap();
        assert!(resp.upgrade())
    }

    #[test]
    fn test_force_close() {
        let resp = HttpResponse::build(StatusCode::OK)
            .force_close().body(Body::Empty).unwrap();
        assert!(!resp.keep_alive().unwrap())
    }

    #[test]
    fn test_content_type() {
        let resp = HttpResponse::build(StatusCode::OK)
            .content_type("text/plain").body(Body::Empty).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[test]
    fn test_content_encoding() {
        let resp = HttpResponse::build(StatusCode::OK).finish().unwrap();
        assert_eq!(*resp.content_encoding(), ContentEncoding::Auto);

        let resp = HttpResponse::build(StatusCode::OK)
            .content_encoding(ContentEncoding::Br).finish().unwrap();
        assert_eq!(*resp.content_encoding(), ContentEncoding::Br);
    }

    #[test]
    fn test_json() {
        let resp = HttpResponse::build(StatusCode::OK)
            .json(vec!["v1", "v2", "v3"]).unwrap();
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, header::HeaderValue::from_static("application/json"));
        assert_eq!(*resp.body(), Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]")));
    }

    #[test]
    fn test_json_ct() {
        let resp = HttpResponse::build(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/json")
            .json(vec!["v1", "v2", "v3"]).unwrap();
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, header::HeaderValue::from_static("text/json"));
        assert_eq!(*resp.body(), Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]")));
    }

    impl Body {
        pub(crate) fn binary(&self) -> Option<&Binary> {
            match *self {
                Body::Binary(ref bin) => Some(bin),
                _ => None,
            }
        }
    }

    #[test]
    fn test_into_response() {
        let req = HttpRequest::default();

        let resp: HttpResponse = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test"));

        let resp: HttpResponse = "test".respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test"));

        let resp: HttpResponse = b"test".as_ref().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(b"test".as_ref()));

        let resp: HttpResponse = b"test".as_ref().respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(b"test".as_ref()));

        let resp: HttpResponse = "test".to_owned().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test".to_owned()));

        let resp: HttpResponse = "test".to_owned().respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test".to_owned()));

        let resp: HttpResponse = (&"test".to_owned()).into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from((&"test".to_owned())));

        let resp: HttpResponse = (&"test".to_owned()).respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from((&"test".to_owned())));

        let b = Bytes::from_static(b"test");
        let resp: HttpResponse = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(Bytes::from_static(b"test")));

        let b = Bytes::from_static(b"test");
        let resp: HttpResponse = b.respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(Bytes::from_static(b"test")));

        let b = BytesMut::from("test");
        let resp: HttpResponse = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(BytesMut::from("test")));

        let b = BytesMut::from("test");
        let resp: HttpResponse = b.respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(BytesMut::from("test")));
    }
}
