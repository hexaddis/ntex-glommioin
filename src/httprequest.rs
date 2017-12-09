//! HTTP Request message related code.
use std::{str, fmt, mem};
use std::rc::Rc;
use std::net::SocketAddr;
use std::collections::HashMap;
use bytes::BytesMut;
use futures::{Async, Future, Stream, Poll};
use url::{Url, form_urlencoded};
use cookie::Cookie;
use http_range::HttpRange;
use http::{header, Uri, Method, Version, HeaderMap, Extensions};

use info::ConnectionInfo;
use param::Params;
use router::Router;
use payload::Payload;
use multipart::Multipart;
use error::{ParseError, PayloadError, UrlGenerationError,
            MultipartError, CookieParseError, HttpRangeError, UrlencodedError};


pub struct HttpMessage {
    pub version: Version,
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub extensions: Extensions,
    pub params: Params<'static>,
    pub cookies: Option<Vec<Cookie<'static>>>,
    pub addr: Option<SocketAddr>,
    pub payload: Payload,
    pub info: Option<ConnectionInfo<'static>>,
}

impl Default for HttpMessage {

    fn default() -> HttpMessage {
        HttpMessage {
            method: Method::GET,
            uri: Uri::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Params::default(),
            cookies: None,
            addr: None,
            payload: Payload::empty(),
            extensions: Extensions::new(),
            info: None,
        }
    }
}

impl HttpMessage {

    /// Checks if a connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        if let Some(conn) = self.headers.get(header::CONNECTION) {
            if let Ok(conn) = conn.to_str() {
                if self.version == Version::HTTP_10 && conn.contains("keep-alive") {
                    true
                } else {
                    self.version == Version::HTTP_11 &&
                        !(conn.contains("close") || conn.contains("upgrade"))
                }
            } else {
                false
            }
        } else {
            self.version != Version::HTTP_10
        }
    }
}

/// An HTTP Request
pub struct HttpRequest<S=()>(Rc<HttpMessage>, Rc<S>, Option<Router<S>>);

impl HttpRequest<()> {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, uri: Uri,
               version: Version, headers: HeaderMap, payload: Payload) -> HttpRequest
    {
        HttpRequest(
            Rc::new(HttpMessage {
                method: method,
                uri: uri,
                version: version,
                headers: headers,
                params: Params::default(),
                cookies: None,
                addr: None,
                payload: payload,
                extensions: Extensions::new(),
                info: None,
            }),
            Rc::new(()),
            None,
        )
    }

    /// Construct new http request with state.
    pub fn with_state<S>(self, state: Rc<S>, router: Router<S>) -> HttpRequest<S> {
        HttpRequest(self.0, state, Some(router))
    }
}

impl<S> HttpRequest<S> {

    /// Construct new http request without state.
    pub fn clone_without_state(&self) -> HttpRequest {
        HttpRequest(Rc::clone(&self.0), Rc::new(()), None)
    }

    // get mutable reference for inner message
    // mutable reference should not be returned as result for request's method
    #[inline]
    #[allow(mutable_transmutes)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref))]
    fn as_mut(&self) -> &mut HttpMessage {
        let r: &HttpMessage = self.0.as_ref();
        unsafe{mem::transmute(r)}
    }

    pub(crate) fn get_inner(&mut self) -> &mut HttpMessage {
        self.as_mut()
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        &self.1
    }

    /// Protocol extensions.
    #[inline]
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.as_mut().extensions
    }

    #[doc(hidden)]
    pub fn prefix_len(&self) -> usize {
        if let Some(router) = self.router() {
            router.prefix().len()
        } else {
            0
        }
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri { &self.0.uri }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method { &self.0.method }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.0.version
    }

    /// Read the Request Headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.0.headers
    }

    #[doc(hidden)]
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.as_mut().headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.0.uri.path()
    }

    /// Get *ConnectionInfo* for currect request.
    pub fn connection_info(&self) -> &ConnectionInfo {
        if self.0.info.is_none() {
            let info: ConnectionInfo<'static> = unsafe{
                mem::transmute(ConnectionInfo::new(self))};
            self.as_mut().info = Some(info);
        }
        self.0.info.as_ref().unwrap()
    }

    pub fn url_for<U, I>(&self, name: &str, elements: U) -> Result<Url, UrlGenerationError>
        where U: IntoIterator<Item=I>,
              I: AsRef<str>,
    {
        if self.router().is_none() {
            Err(UrlGenerationError::RouterNotAvailable)
        } else {
            let path = self.router().unwrap().resource_path(name, elements)?;
            if path.starts_with('/') {
                let conn = self.connection_info();
                Ok(Url::parse(&format!("{}://{}{}", conn.scheme(), conn.host(), path))?)
            } else {
                Ok(Url::parse(&path)?)
            }
        }
    }

    #[inline]
    pub fn router(&self) -> Option<&Router<S>> {
        self.2.as_ref()
    }

    #[inline]
    pub fn peer_addr(&self) -> Option<&SocketAddr> {
        self.0.addr.as_ref()
    }

    #[inline]
    pub(crate) fn set_peer_addr(&mut self, addr: Option<SocketAddr>) {
        self.as_mut().addr = addr
    }

    /// Return a new iterator that yields pairs of `Cow<str>` for query parameters
    pub fn query(&self) -> HashMap<String, String> {
        let mut q: HashMap<String, String> = HashMap::new();
        if let Some(query) = self.0.uri.query().as_ref() {
            for (key, val) in form_urlencoded::parse(query.as_ref()) {
                q.insert(key.to_string(), val.to_string());
            }
        }
        q
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        if let Some(query) = self.0.uri.query().as_ref() {
            query
        } else {
            ""
        }
    }

    /// Load request cookies.
    #[inline]
    pub fn cookies(&self) -> Result<&Vec<Cookie<'static>>, CookieParseError> {
        if self.0.cookies.is_none() {
            let msg = self.as_mut();
            let mut cookies = Vec::new();
            if let Some(val) = msg.headers.get(header::COOKIE) {
                let s = str::from_utf8(val.as_bytes())
                    .map_err(CookieParseError::from)?;
                for cookie in s.split("; ") {
                    cookies.push(Cookie::parse_encoded(cookie)?.into_owned());
                }
            }
            msg.cookies = Some(cookies)
        }
        Ok(self.0.cookies.as_ref().unwrap())
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies {
                if cookie.name() == name {
                    return Some(cookie)
                }
            }
        }
        None
    }

    /// Get a reference to the Params object.
    /// Params is a container for url parameters.
    /// Route supports glob patterns: * for a single wildcard segment and :param
    /// for matching storing that segment of the request url in the Params object.
    #[inline]
    pub fn match_info(&self) -> &Params {
        unsafe{ mem::transmute(&self.0.params) }
    }

    /// Set request Params.
    #[inline]
    pub(crate) fn match_info_mut(&mut self) -> &mut Params {
        unsafe{ mem::transmute(&mut self.as_mut().params) }
    }

    /// Checks if a connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        if let Some(conn) = self.0.headers.get(header::CONNECTION) {
            if let Ok(conn) = conn.to_str() {
                if self.0.version == Version::HTTP_10 && conn.contains("keep-alive") {
                    true
                } else {
                    self.0.version == Version::HTTP_11 &&
                        !(conn.contains("close") || conn.contains("upgrade"))
                }
            } else {
                false
            }
        } else {
            self.0.version != Version::HTTP_10
        }
    }

    /// Read the request content type
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.0.headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type
            }
        }
        ""
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.0.headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        self.0.method == Method::CONNECT
    }

    /// Check if request has chunked transfer encoding
    pub fn chunked(&self) -> Result<bool, ParseError> {
        if let Some(encodings) = self.0.headers.get(header::TRANSFER_ENCODING) {
            if let Ok(s) = encodings.to_str() {
                Ok(s.to_lowercase().contains("chunked"))
            } else {
                Err(ParseError::Header)
            }
        } else {
            Ok(false)
        }
    }

    /// Parses Range HTTP header string as per RFC 2616.
    /// `size` is full size of response (file).
    pub fn range(&self, size: u64) -> Result<Vec<HttpRange>, HttpRangeError> {
        if let Some(range) = self.0.headers.get(header::RANGE) {
            HttpRange::parse(unsafe{str::from_utf8_unchecked(range.as_bytes())}, size)
                .map_err(|e| e.into())
        } else {
            Ok(Vec::new())
        }
    }

    /// Returns reference to the associated http payload.
    #[inline]
    pub fn payload(&self) -> &Payload {
        &self.0.payload
    }

    /// Returns mutable reference to the associated http payload.
    #[inline]
    pub fn payload_mut(&mut self) -> &mut Payload {
        &mut self.as_mut().payload
    }

    /// Return payload
    #[inline]
    pub fn take_payload(&mut self) -> Payload {
        mem::replace(&mut self.as_mut().payload, Payload::empty())
    }
    
    /// Return stream to process BODY as multipart.
    ///
    /// Content-type: multipart/form-data;
    pub fn multipart(&mut self) -> Result<Multipart, MultipartError> {
        let boundary = Multipart::boundary(&self.0.headers)?;
        Ok(Multipart::new(boundary, self.take_payload()))
    }

    /// Parse `application/x-www-form-urlencoded` encoded body.
    /// Return `UrlEncoded` future. It resolves to a `HashMap<String, String>` which
    /// contains decoded parameters.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/x-www-form-urlencoded`
    /// * transfer encoding is `chunked`.
    /// * content-length is greater than 256k
    pub fn urlencoded(&mut self) -> Result<UrlEncoded, UrlencodedError> {
        if let Ok(true) = self.chunked() {
            return Err(UrlencodedError::Chunked)
        }

        if let Some(len) = self.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    if len > 262_144 {
                        return Err(UrlencodedError::Overflow)
                    }
                } else {
                    return Err(UrlencodedError::UnknownLength)
                }
            } else {
                return Err(UrlencodedError::UnknownLength)
            }
        }

        // check content type
        let t = if let Some(content_type) = self.0.headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                content_type.to_lowercase() == "application/x-www-form-urlencoded"
            } else {
                false
            }
        } else {
            false
        };

        if t {
            Ok(UrlEncoded{pl: self.take_payload(), body: BytesMut::new()})
        } else {
            Err(UrlencodedError::ContentType)
        }
    }
}

impl Default for HttpRequest<()> {

    /// Construct default request
    fn default() -> HttpRequest {
        HttpRequest(Rc::new(HttpMessage::default()), Rc::new(()), None)
    }
}

impl<S> Clone for HttpRequest<S> {
    fn clone(&self) -> HttpRequest<S> {
        HttpRequest(Rc::clone(&self.0), Rc::clone(&self.1), None)
    }
}

impl<S> fmt::Debug for HttpRequest<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpRequest {:?} {}:{}\n",
                         self.0.version, self.0.method, self.0.uri);
        if !self.query_string().is_empty() {
            let _ = write!(f, "  query: ?{:?}\n", self.query_string());
        }
        if !self.match_info().is_empty() {
            let _ = write!(f, "  params: {:?}\n", self.0.params);
        }
        let _ = write!(f, "  headers:\n");
        for key in self.0.headers.keys() {
            let vals: Vec<_> = self.0.headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

/// Future that resolves to a parsed urlencoded values.
pub struct UrlEncoded {
    pl: Payload,
    body: BytesMut,
}

impl Future for UrlEncoded {
    type Item = HashMap<String, String>;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            return match self.pl.poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(None)) => {
                    let mut m = HashMap::new();
                    for (k, v) in form_urlencoded::parse(&self.body) {
                        m.insert(k.into(), v.into());
                    }
                    Ok(Async::Ready(m))
                },
                Ok(Async::Ready(Some(item))) => {
                    self.body.extend(item.0);
                    continue
                },
                Err(err) => Err(err),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Uri;
    use std::str::FromStr;
    use router::Pattern;
    use payload::Payload;
    use resource::Resource;

    #[test]
    fn test_urlencoded_error() {
        let mut headers = HeaderMap::new();
        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_static("chunked"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::Chunked);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("xxxx"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11,
            headers, Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::UnknownLength);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("1000000"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::Overflow);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("text/plain"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("10"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::ContentType);
    }

    #[test]
    fn test_url_for() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST,
                       header::HeaderValue::from_static("www.rust-lang.org"));
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, Payload::empty());

        let mut resource = Resource::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("index", "/user/{name}.{ext}"), Some(resource));
        let router = Router::new("", map);
        assert!(router.has_route("/user/test.html"));
        assert!(!router.has_route("/test/unknown"));

        assert_eq!(req.url_for("unknown", &["test"]),
                   Err(UrlGenerationError::RouterNotAvailable));

        let req = req.with_state(Rc::new(()), router);

        assert_eq!(req.url_for("unknown", &["test"]),
                   Err(UrlGenerationError::ResourceNotFound));
        assert_eq!(req.url_for("index", &["test"]),
                   Err(UrlGenerationError::NotEnoughElements));
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(url.ok().unwrap().as_str(), "http://www.rust-lang.org/user/test.html");
    }

    #[test]
    fn test_url_for_external() {
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("youtube", "https://youtube.com/watch/{video_id}"), None);
        let router = Router::new("", map);
        assert!(!router.has_route("https://youtube.com/watch/unknown"));

        let req = req.with_state(Rc::new(()), router);
        let url = req.url_for("youtube", &["oHg5SJYRHA0"]);
        assert_eq!(url.ok().unwrap().as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    }
}
