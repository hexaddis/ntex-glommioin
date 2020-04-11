use std::cell::{Ref, RefMut};
use std::marker::PhantomData;
use std::rc::Rc;
use std::{fmt, net};

use crate::http::{
    Extensions, HeaderMap, HttpMessage, Method, Payload, PayloadStream, RequestHead,
    Response, Uri, Version,
};
use crate::router::{Path, Resource};

use super::config::AppConfig;
use super::error::ErrorRenderer;
use super::httprequest::HttpRequest;
use super::info::ConnectionInfo;
use super::response::WebResponse;
use super::rmap::ResourceMap;
use super::types::Data;

/// An service http request
///
/// WebRequest allows mutable access to request's internal structures
pub struct WebRequest<Err> {
    req: HttpRequest,
    _t: PhantomData<Err>,
}

impl<Err: ErrorRenderer> WebRequest<Err> {
    /// Create web response for error
    #[inline]
    pub fn error_response<E: Into<Err::Container>>(self, err: E) -> WebResponse {
        WebResponse::from_err::<Err, E>(err, self.req)
    }
}

impl<Err> WebRequest<Err> {
    /// Construct web request
    pub(crate) fn new(req: HttpRequest) -> Self {
        WebRequest {
            req,
            _t: PhantomData,
        }
    }

    /// Deconstruct request into parts
    pub fn into_parts(mut self) -> (HttpRequest, Payload) {
        let pl = Rc::get_mut(&mut (self.req).0).unwrap().payload.take();
        (self.req, pl)
    }

    /// Construct request from parts.
    ///
    /// `WebRequest` can be re-constructed only if `req` hasnt been cloned.
    pub fn from_parts(
        mut req: HttpRequest,
        pl: Payload,
    ) -> Result<Self, (HttpRequest, Payload)> {
        if Rc::strong_count(&req.0) == 1 && Rc::weak_count(&req.0) == 0 {
            Rc::get_mut(&mut req.0).unwrap().payload = pl;
            Ok(WebRequest::new(req))
        } else {
            Err((req, pl))
        }
    }

    /// Construct request from request.
    ///
    /// `HttpRequest` implements `Clone` trait via `Rc` type. `WebRequest`
    /// can be re-constructed only if rc's strong pointers count eq 1 and
    /// weak pointers count is 0.
    pub fn from_request(req: HttpRequest) -> Result<Self, HttpRequest> {
        if Rc::strong_count(&req.0) == 1 && Rc::weak_count(&req.0) == 0 {
            Ok(WebRequest::new(req))
        } else {
            Err(req)
        }
    }

    /// Create web response
    #[inline]
    pub fn into_response<R: Into<Response>>(self, res: R) -> WebResponse {
        WebResponse::new(self.req, res.into())
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.req.head()
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        self.req.head_mut()
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

    #[inline]
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    /// Returns mutable request's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
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

    /// Peer socket address
    ///
    /// Peer address is actual socket address, if proxy is used in front of
    /// actix http server, then peer address would be address of this proxy.
    ///
    /// To get client connection information `ConnectionInfo` should be used.
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Get *ConnectionInfo* for the current request.
    #[inline]
    pub fn connection_info(&self) -> Ref<'_, ConnectionInfo> {
        ConnectionInfo::get(self.head(), &*self.app_config())
    }

    /// Get a reference to the Path parameters.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Path<Uri> {
        self.req.match_info()
    }

    #[inline]
    /// Get a mutable reference to the Path parameters.
    pub fn match_info_mut(&mut self) -> &mut Path<Uri> {
        self.req.match_info_mut()
    }

    #[inline]
    /// Get a reference to a `ResourceMap` of current application.
    pub fn resource_map(&self) -> &ResourceMap {
        self.req.resource_map()
    }

    /// Service configuration
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.req.app_config()
    }

    #[inline]
    /// Get an application data stored with `App::data()` method during
    /// application configuration.
    pub fn app_data<T: 'static>(&self) -> Option<Data<T>> {
        if let Some(st) = (self.req).0.app_data.get::<Data<T>>() {
            Some(st.clone())
        } else {
            None
        }
    }

    #[inline]
    /// Get request's payload
    pub fn take_payload(&mut self) -> Payload<PayloadStream> {
        Rc::get_mut(&mut (self.req).0).unwrap().payload.take()
    }

    #[inline]
    /// Set request payload.
    pub fn set_payload(&mut self, payload: Payload) {
        Rc::get_mut(&mut (self.req).0).unwrap().payload = payload;
    }

    #[doc(hidden)]
    /// Set new app data container
    pub fn set_data_container(&mut self, extensions: Rc<Extensions>) {
        Rc::get_mut(&mut (self.req).0).unwrap().app_data = extensions;
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.req.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.req.extensions_mut()
    }
}

impl<Err> Resource<Uri> for WebRequest<Err> {
    fn path(&self) -> &str {
        self.match_info().path()
    }

    fn resource_path(&mut self) -> &mut Path<Uri> {
        self.match_info_mut()
    }
}

impl<Err> HttpMessage for WebRequest<Err> {
    #[inline]
    /// Returns Request's headers.
    fn message_headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn message_extensions(&self) -> Ref<'_, Extensions> {
        self.req.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn message_extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.req.extensions_mut()
    }
}

impl<Err: ErrorRenderer> fmt::Debug for WebRequest<Err> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "\nWebRequest {:?} {}:{}",
            self.head().version,
            self.head().method,
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
