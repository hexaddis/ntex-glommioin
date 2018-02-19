use std::{fmt, mem};
use std::io::Write;

use cookie::{Cookie, CookieJar};
use bytes::{BytesMut, BufMut};
use http::{HeaderMap, Method, Version, Uri, HttpTryFrom, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use serde_json;
use serde::Serialize;

use body::Body;
use error::Error;
use headers::ContentEncoding;
use super::pipeline::SendRequest;

/// An HTTP Client Request
pub struct ClientRequest {
    uri: Uri,
    method: Method,
    version: Version,
    headers: HeaderMap,
    body: Body,
    chunked: bool,
    upgrade: bool,
    encoding: ContentEncoding,
}

impl Default for ClientRequest {

    fn default() -> ClientRequest {
        ClientRequest {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            body: Body::Empty,
            chunked: false,
            upgrade: false,
            encoding: ContentEncoding::Auto,
        }
    }
}

impl ClientRequest {

    /// Create request builder for `GET` request
    pub fn get<U>(uri: U) -> ClientRequestBuilder where Uri: HttpTryFrom<U> {
        let mut builder = ClientRequest::build();
        builder.method(Method::GET).uri(uri);
        builder
    }

    /// Create request builder for `HEAD` request
    pub fn head<U>(uri: U) -> ClientRequestBuilder where Uri: HttpTryFrom<U> {
        let mut builder = ClientRequest::build();
        builder.method(Method::HEAD).uri(uri);
        builder
    }

    /// Create request builder for `POST` request
    pub fn post<U>(uri: U) -> ClientRequestBuilder where Uri: HttpTryFrom<U> {
        let mut builder = ClientRequest::build();
        builder.method(Method::POST).uri(uri);
        builder
    }

    /// Create request builder for `PUT` request
    pub fn put<U>(uri: U) -> ClientRequestBuilder where Uri: HttpTryFrom<U> {
        let mut builder = ClientRequest::build();
        builder.method(Method::PUT).uri(uri);
        builder
    }

    /// Create request builder for `DELETE` request
    pub fn delete<U>(uri: U) -> ClientRequestBuilder where Uri: HttpTryFrom<U> {
        let mut builder = ClientRequest::build();
        builder.method(Method::DELETE).uri(uri);
        builder
    }
}

impl ClientRequest {

    /// Create client request builder
    pub fn build() -> ClientRequestBuilder {
        ClientRequestBuilder {
            request: Some(ClientRequest::default()),
            err: None,
            cookies: None,
        }
    }

    /// Get the request uri
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Set client request uri
    #[inline]
    pub fn set_uri(&mut self, uri: Uri) {
        self.uri = uri
    }

    /// Get the request method
    #[inline]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Set http `Method` for the request
    #[inline]
    pub fn set_method(&mut self, method: Method) {
        self.method = method
    }

    /// Get http version for the request
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Set http `Version` for the request
    #[inline]
    pub fn set_version(&mut self, version: Version) {
        self.version = version
    }

    /// Get the headers from the request
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// is chunked encoding enabled
    #[inline]
    pub fn chunked(&self) -> bool {
        self.chunked
    }

    /// is upgrade request
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.upgrade
    }

    /// Content encoding
    #[inline]
    pub fn content_encoding(&self) -> ContentEncoding {
        self.encoding
    }

    /// Get body os this response
    #[inline]
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Set a body
    pub fn set_body<B: Into<Body>>(&mut self, body: B) {
        self.body = body.into();
    }

    /// Extract body, replace it with Empty
    pub(crate) fn replace_body(&mut self, body: Body) -> Body {
        mem::replace(&mut self.body, body)
    }

    /// Send request
    pub fn send(self) -> SendRequest {
        SendRequest::new(self)
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nClientRequest {:?} {}:{}\n",
                         self.version, self.method, self.uri);
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


/// An HTTP Client request builder
///
/// This type can be used to construct an instance of `ClientRequest` through a
/// builder-like pattern.
pub struct ClientRequestBuilder {
    request: Option<ClientRequest>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
}

impl ClientRequestBuilder {
    /// Set HTTP uri of request.
    #[inline]
    pub fn uri<U>(&mut self, uri: U) -> &mut Self where Uri: HttpTryFrom<U> {
        match Uri::try_from(uri) {
            Ok(uri) => {
                // set request host header
                if let Some(host) = uri.host() {
                    self.set_header(header::HOST, host);
                }
                if let Some(parts) = parts(&mut self.request, &self.err) {
                    parts.uri = uri;
                }
            },
            Err(e) => self.err = Some(e.into(),),
        }
        self
    }

    /// Set HTTP method of this request.
    #[inline]
    pub fn method(&mut self, method: Method) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.method = method;
        }
        self
    }

    /// Set HTTP version of this request.
    ///
    /// By default requests's http version depends on network stream
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.version = version;
        }
        self
    }

    /// Add a header.
    ///
    /// Header get appended to existing header.
    /// To override header use `set_header()` method.
    ///
    /// ```rust
    /// # extern crate http;
    /// # extern crate actix_web;
    /// # use actix_web::client::*;
    /// #
    /// use http::header;
    ///
    /// fn main() {
    ///     let req = ClientRequest::build()
    ///         .header("X-TEST", "value")
    ///         .header(header::CONTENT_TYPE, "application/json")
    ///         .finish().unwrap();
    /// }
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>, HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
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

    /// Replace a header.
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>, HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    match HeaderValue::try_from(value) {
                        Ok(value) => { parts.headers.insert(key, value); }
                        Err(e) => self.err = Some(e.into()),
                    }
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set content encoding.
    ///
    /// By default `ContentEncoding::Identity` is used.
    #[inline]
    pub fn content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.encoding = enc;
        }
        self
    }

    /// Enables automatic chunked transfer encoding
    #[inline]
    pub fn chunked(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.chunked = true;
        }
        self
    }

    /// Enable connection upgrade
    #[inline]
    pub fn upgrade(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.upgrade = true;
        }
        self
    }

    /// Set request's content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.request, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => { parts.headers.insert(header::CONTENT_TYPE, value); },
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
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # use actix_web::httpcodes::*;
    /// #
    /// use actix_web::headers::Cookie;
    /// use actix_web::client::ClientRequest;
    ///
    /// fn main() {
    ///     let req = ClientRequest::build()
    ///         .cookie(
    ///             Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish())
    ///         .finish().unwrap();
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

    /// Remove cookie, cookie has to be cookie from `HttpRequest::cookies()` method.
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

    /// This method calls provided closure with builder reference if value is true.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
        where F: FnOnce(&mut ClientRequestBuilder)
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is Some.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
        where F: FnOnce(T, &mut ClientRequestBuilder)
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Set a body and generate `ClientRequest`.
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Result<ClientRequest, HttpError> {
        if let Some(e) = self.err.take() {
            return Err(e)
        }

        let mut request = self.request.take().expect("cannot reuse request builder");

        // set cookies
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                request.headers.append(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie.to_string())?);
            }
        }
        request.body = body.into();
        Ok(request)
    }

    /// Set a json body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn json<T: Serialize>(&mut self, value: T) -> Result<ClientRequest, Error> {
        let body = serde_json::to_string(&value)?;

        let contains = if let Some(parts) = parts(&mut self.request, &self.err) {
            parts.headers.contains_key(header::CONTENT_TYPE)
        } else {
            true
        };
        if !contains {
            self.header(header::CONTENT_TYPE, "application/json");
        }

        Ok(self.body(body)?)
    }

    /// Set an empty body and generate `ClientRequest`
    ///
    /// `ClientRequestBuilder` can not be used after this call.
    pub fn finish(&mut self) -> Result<ClientRequest, HttpError> {
        self.body(Body::Empty)
    }

    /// This method construct new `ClientRequestBuilder`
    pub fn take(&mut self) -> ClientRequestBuilder {
        ClientRequestBuilder {
            request: self.request.take(),
            err: self.err.take(),
            cookies: self.cookies.take(),
        }
    }
}

#[inline]
fn parts<'a>(parts: &'a mut Option<ClientRequest>, err: &Option<HttpError>)
             -> Option<&'a mut ClientRequest>
{
    if err.is_some() {
        return None
    }
    parts.as_mut()
}
