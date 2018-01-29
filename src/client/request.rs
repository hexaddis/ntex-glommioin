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


pub struct ClientRequest {
    uri: Uri,
    method: Method,
    version: Version,
    headers: HeaderMap,
    body: Body,
    chunked: Option<bool>,
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
            chunked: None,
            encoding: ContentEncoding::Auto,
        }
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

    /// Get body os this response
    #[inline]
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

    /// Set a header.
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
            parts.chunked = Some(true);
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
    ///
    /// fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     Ok(HTTPOk.build()
    ///         .cookie(
    ///             Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish())
    ///         .finish()?)
    /// }
    /// fn main() {}
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
