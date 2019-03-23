use std::cell::{Ref, RefMut};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{Error, Extensions, HttpMessage, Message, Payload, RequestHead};
use actix_router::{Path, Url};

use crate::config::AppConfig;
use crate::error::UrlGenerationError;
use crate::extract::FromRequest;
use crate::info::ConnectionInfo;
use crate::rmap::ResourceMap;
use crate::service::ServiceFromRequest;

#[derive(Clone)]
/// An HTTP Request
pub struct HttpRequest {
    pub(crate) head: Message<RequestHead>,
    pub(crate) path: Path<Url>,
    rmap: Rc<ResourceMap>,
    config: AppConfig,
}

impl HttpRequest {
    #[inline]
    pub(crate) fn new(
        head: Message<RequestHead>,
        path: Path<Url>,
        rmap: Rc<ResourceMap>,
        config: AppConfig,
    ) -> HttpRequest {
        HttpRequest {
            head,
            path,
            rmap,
            config,
        }
    }
}

impl HttpRequest {
    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.head
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        if let Some(query) = self.uri().query().as_ref() {
            query
        } else {
            ""
        }
    }

    /// Get a reference to the Path parameters.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        &self.path
    }

    /// App config
    #[inline]
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Generate url for named resource
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::{web, App, HttpRequest, HttpResponse};
    /// #
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     let url = req.url_for("foo", &["1", "2", "3"]); // <- generate url for "foo" resource
    ///     HttpResponse::Ok().into()
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(web::resource("/test/{one}/{two}/{three}")
    ///              .name("foo")  // <- set resource name, then it could be used in `url_for`
    ///              .route(web::get().to(|| HttpResponse::Ok()))
    ///         );
    /// }
    /// ```
    pub fn url_for<U, I>(
        &self,
        name: &str,
        elements: U,
    ) -> Result<url::Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        self.rmap.url_for(&self, name, elements)
    }

    /// Generate url for named resource
    ///
    /// This method is similar to `HttpRequest::url_for()` but it can be used
    /// for urls that do not contain variable parts.
    pub fn url_for_static(&self, name: &str) -> Result<url::Url, UrlGenerationError> {
        const NO_PARAMS: [&str; 0] = [];
        self.url_for(name, &NO_PARAMS)
    }

    /// Get *ConnectionInfo* for the current request.
    #[inline]
    pub fn connection_info(&self) -> Ref<ConnectionInfo> {
        ConnectionInfo::get(&*self, &*self.config())
    }
}

impl Deref for HttpRequest {
    type Target = RequestHead;

    fn deref(&self) -> &RequestHead {
        self.head()
    }
}

impl HttpMessage for HttpRequest {
    type Stream = ();

    #[inline]
    /// Returns Request's headers.
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        Payload::None
    }
}

/// It is possible to get `HttpRequest` as an extractor handler parameter
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, HttpRequest};
///
/// /// extract `Thing` from request
/// fn index(req: HttpRequest) -> String {
///    format!("Got thing: {:?}", req)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/{first}").route(
///             web::get().to(index))
///     );
/// }
/// ```
impl<P> FromRequest<P> for HttpRequest {
    type Error = Error;
    type Future = Result<Self, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(req.clone())
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nHttpRequest {:?} {}:{}",
            self.head.version,
            self.head.method,
            self.path()
        )?;
        if !self.query_string().is_empty() {
            writeln!(f, "  query: ?{:?}", self.query_string())?;
        }
        if !self.match_info().is_empty() {
            writeln!(f, "  params: {:?}", self.match_info())?;
        }
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::{ResourceDef, ResourceMap};
    use crate::http::header;
    use crate::test::TestRequest;

    #[test]
    fn test_debug() {
        let req =
            TestRequest::with_header("content-type", "text/plain").to_http_request();
        let dbg = format!("{:?}", req);
        assert!(dbg.contains("HttpRequest"));
    }

    #[test]
    #[cfg(feature = "cookies")]
    fn test_no_request_cookies() {
        let req = TestRequest::default().to_http_request();
        assert!(req.cookies().unwrap().is_empty());
    }

    #[test]
    #[cfg(feature = "cookies")]
    fn test_request_cookies() {
        let req = TestRequest::default()
            .header(header::COOKIE, "cookie1=value1")
            .header(header::COOKIE, "cookie2=value2")
            .to_http_request();
        {
            let cookies = req.cookies().unwrap();
            assert_eq!(cookies.len(), 2);
            assert_eq!(cookies[0].name(), "cookie1");
            assert_eq!(cookies[0].value(), "value1");
            assert_eq!(cookies[1].name(), "cookie2");
            assert_eq!(cookies[1].value(), "value2");
        }

        let cookie = req.cookie("cookie1");
        assert!(cookie.is_some());
        let cookie = cookie.unwrap();
        assert_eq!(cookie.name(), "cookie1");
        assert_eq!(cookie.value(), "value1");

        let cookie = req.cookie("cookie-unknown");
        assert!(cookie.is_none());
    }

    #[test]
    fn test_request_query() {
        let req = TestRequest::with_uri("/?id=test").to_http_request();
        assert_eq!(req.query_string(), "id=test");
    }

    #[test]
    fn test_url_for() {
        let mut res = ResourceDef::new("/user/{name}.{ext}");
        *res.name_mut() = "index".to_string();

        let mut rmap = ResourceMap::new(ResourceDef::new(""));
        rmap.add(&mut res, None);
        assert!(rmap.has_resource("/user/test.html"));
        assert!(!rmap.has_resource("/test/unknown"));

        let req = TestRequest::with_header(header::HOST, "www.rust-lang.org")
            .rmap(rmap)
            .to_http_request();

        assert_eq!(
            req.url_for("unknown", &["test"]),
            Err(UrlGenerationError::ResourceNotFound)
        );
        assert_eq!(
            req.url_for("index", &["test"]),
            Err(UrlGenerationError::NotEnoughElements)
        );
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/user/test.html"
        );
    }

    #[test]
    fn test_url_for_static() {
        let mut rdef = ResourceDef::new("/index.html");
        *rdef.name_mut() = "index".to_string();

        let mut rmap = ResourceMap::new(ResourceDef::new(""));
        rmap.add(&mut rdef, None);

        assert!(rmap.has_resource("/index.html"));

        let req = TestRequest::with_uri("/test")
            .header(header::HOST, "www.rust-lang.org")
            .rmap(rmap)
            .to_http_request();
        let url = req.url_for_static("index");
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/index.html"
        );
    }

    #[test]
    fn test_url_for_external() {
        let mut rdef = ResourceDef::new("https://youtube.com/watch/{video_id}");

        *rdef.name_mut() = "youtube".to_string();

        let mut rmap = ResourceMap::new(ResourceDef::new(""));
        rmap.add(&mut rdef, None);
        assert!(rmap.has_resource("https://youtube.com/watch/unknown"));

        let req = TestRequest::default().rmap(rmap).to_http_request();
        let url = req.url_for("youtube", &["oHg5SJYRHA0"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "https://youtube.com/watch/oHg5SJYRHA0"
        );
    }
}
