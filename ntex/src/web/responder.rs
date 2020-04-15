use std::convert::TryFrom;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures::future::{ready, Either as EitherFuture, Ready};
use futures::ready;
use pin_project::pin_project;

use crate::http::error::HttpError;
use crate::http::header::{HeaderMap, HeaderName, IntoHeaderValue};
use crate::http::{Response, ResponseBuilder, StatusCode};

use super::error::{
    DefaultError, ErrorContainer, ErrorRenderer, InternalError, WebResponseError,
};
use super::httprequest::HttpRequest;

/// Trait implemented by types that can be converted to a http response.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder<Err = DefaultError> {
    /// The associated error which can be returned.
    type Error;

    /// The future response value.
    type Future: Future<Output = Response>;

    /// Convert itself to `AsyncResult` or `Error`.
    fn respond_to(self, req: &HttpRequest) -> Self::Future;

    /// Override a status code for a Responder.
    ///
    /// ```rust
    /// use ntex::http::StatusCode;
    /// use ntex::web::{HttpRequest, Responder};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// # fn main() {}
    /// ```
    fn with_status(self, status: StatusCode) -> CustomResponder<Self, Err>
    where
        Self: Sized,
    {
        CustomResponder::new(self).with_status(status)
    }

    /// Add header to the Responder's response.
    ///
    /// ```rust
    /// use ntex::web::{self, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// async fn index(req: HttpRequest) -> impl Responder {
    ///     web::types::Json(
    ///         MyObj { name: "Name".to_string() }
    ///     )
    ///     .with_header("x-version", "1.2.3")
    /// }
    /// # fn main() {}
    /// ```
    fn with_header<K, V>(self, key: K, value: V) -> CustomResponder<Self, Err>
    where
        Self: Sized,
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        CustomResponder::new(self).with_header(key, value)
    }
}

impl<Err: ErrorRenderer> Responder<Err> for Response {
    type Error = Err::Container;
    type Future = Ready<Response>;

    #[inline]
    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(self)
    }
}

impl<Err: ErrorRenderer> Responder<Err> for ResponseBuilder {
    type Error = Err::Container;
    type Future = Ready<Response>;

    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> Self::Future {
        ready(self.finish())
    }
}

impl<T, Err> Responder<Err> for Option<T>
where
    T: Responder<Err>,
    Err: ErrorRenderer,
{
    type Error = T::Error;
    type Future = EitherFuture<T::Future, Ready<Response>>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Some(t) => EitherFuture::Left(t.respond_to(req)),
            None => EitherFuture::Right(ready(
                Response::build(StatusCode::NOT_FOUND).finish(),
            )),
        }
    }
}

impl<T, E, Err> Responder<Err> for Result<T, E>
where
    T: Responder<Err>,
    E: Into<Err::Container>,
    Err: ErrorRenderer,
{
    type Error = T::Error;
    type Future = EitherFuture<T::Future, Ready<Response>>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Ok(val) => EitherFuture::Left(val.respond_to(req)),
            Err(e) => EitherFuture::Right(ready(e.into().error_response(req))),
        }
    }
}

impl<T, Err> Responder<Err> for (T, StatusCode)
where
    T: Responder<Err>,
    Err: ErrorRenderer,
{
    type Error = T::Error;
    type Future = CustomResponderFut<T, Err>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        CustomResponderFut {
            fut: self.0.respond_to(req),
            status: Some(self.1),
            headers: None,
        }
    }
}

impl<Err: ErrorRenderer> Responder<Err> for &'static str {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("text/plain; charset=utf-8")
                .body(self),
        )
    }
}

impl<Err: ErrorRenderer> Responder<Err> for &'static [u8] {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("application/octet-stream")
                .body(self),
        )
    }
}

impl<Err: ErrorRenderer> Responder<Err> for String {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("text/plain; charset=utf-8")
                .body(self),
        )
    }
}

impl<'a, Err: ErrorRenderer> Responder<Err> for &'a String {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("text/plain; charset=utf-8")
                .body(self),
        )
    }
}

impl<Err: ErrorRenderer> Responder<Err> for Bytes {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("application/octet-stream")
                .body(self),
        )
    }
}

impl<Err: ErrorRenderer> Responder<Err> for BytesMut {
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ready(
            Response::build(StatusCode::OK)
                .content_type("application/octet-stream")
                .body(self),
        )
    }
}

/// Allows to override status code and headers for a responder.
pub struct CustomResponder<T: Responder<Err>, Err> {
    responder: T,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
    error: Option<HttpError>,
    _t: PhantomData<Err>,
}

impl<T: Responder<Err>, Err> CustomResponder<T, Err> {
    fn new(responder: T) -> Self {
        CustomResponder {
            responder,
            status: None,
            headers: None,
            error: None,
            _t: PhantomData,
        }
    }

    /// Override a status code for the Responder's response.
    ///
    /// ```rust
    /// use ntex::http::StatusCode;
    /// use ntex::web::{HttpRequest, Responder};
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     "Welcome!".with_status(StatusCode::OK)
    /// }
    /// # fn main() {}
    /// ```
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }

    /// Add header to the Responder's response.
    ///
    /// ```rust
    /// use ntex::web::{self, HttpRequest, Responder};
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(req: HttpRequest) -> impl Responder {
    ///     web::types::Json(
    ///         MyObj{name: "Name".to_string()}
    ///     )
    ///     .with_header("x-version", "1.2.3")
    /// }
    /// # fn main() {}
    /// ```
    pub fn with_header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        if self.headers.is_none() {
            self.headers = Some(HeaderMap::new());
        }

        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.headers.as_mut().unwrap().append(key, value);
                }
                Err(e) => self.error = Some(e.into()),
            },
            Err(e) => self.error = Some(e.into()),
        };
        self
    }
}

impl<T: Responder<Err>, Err: ErrorRenderer> Responder<Err> for CustomResponder<T, Err> {
    type Error = T::Error;
    type Future = CustomResponderFut<T, Err>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        CustomResponderFut {
            fut: self.responder.respond_to(req),
            status: self.status,
            headers: self.headers,
        }
    }
}

#[pin_project]
pub struct CustomResponderFut<T: Responder<Err>, Err: ErrorRenderer> {
    #[pin]
    fut: T::Future,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
}

impl<T: Responder<Err>, Err: ErrorRenderer> Future for CustomResponderFut<T, Err> {
    type Output = Response;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let mut res = ready!(this.fut.poll(cx));
        if let Some(status) = this.status.take() {
            *res.status_mut() = status;
        }
        if let Some(ref headers) = this.headers {
            for (k, v) in headers {
                res.headers_mut().insert(k.clone(), v.clone());
            }
        }
        Poll::Ready(res)
    }
}

/// Combines two different responder types into a single type
///
/// ```rust
/// use ntex::web::{Either, HttpResponse};
///
/// fn index() -> Either<HttpResponse, &'static str> {
///     if is_a_variant() {
///         // <- choose left variant
///         Either::Left(HttpResponse::BadRequest().body("Bad data"))
///     } else {
///         // <- Right variant
///         Either::Right("Hello!")
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
impl<A, B, Err> Responder<Err> for either::Either<A, B>
where
    A: Responder<Err>,
    B: Responder<Err>,
    Err: ErrorRenderer,
{
    type Error = Err::Container;
    type Future = EitherFuture<A::Future, B::Future>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            either::Either::Left(a) => EitherFuture::Left(a.respond_to(req)),
            either::Either::Right(b) => EitherFuture::Right(b.respond_to(req)),
        }
    }
}

impl<T, Err> Responder<Err> for InternalError<T, Err>
where
    T: std::fmt::Debug + std::fmt::Display + 'static,
    Err: ErrorRenderer,
{
    type Error = Err::Container;
    type Future = Ready<Response>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        ready(self.error_response(req))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::http::body::{Body, ResponseBody};
    use crate::http::header::{HeaderValue, CONTENT_TYPE};
    use crate::http::{Response as HttpResponse, StatusCode};
    use crate::web;
    use crate::web::test::{init_service, TestRequest};
    use crate::Service;

    fn responder<T: Responder<DefaultError>>(
        responder: T,
    ) -> impl Responder<DefaultError, Error = T::Error> {
        responder
    }

    #[ntex_rt::test]
    async fn test_either_responder() {
        let srv = init_service(web::App::new().service(
            web::resource("/index.html").to(|req: HttpRequest| async move {
                if req.query_string().is_empty() {
                    either::Either::Left(HttpResponse::BadRequest())
                } else {
                    either::Either::Right("hello")
                }
            }),
        ))
        .await;

        let req = TestRequest::with_uri("/index.html").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/index.html?query=test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[ntex_rt::test]
    async fn test_option_responder() {
        let srv = init_service(
            web::App::new()
                .service(
                    web::resource("/none").to(|| async { Option::<&'static str>::None }),
                )
                .service(web::resource("/some").to(|| async { Some("some") })),
        )
        .await;

        let req = TestRequest::with_uri("/none").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/some").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        match resp.response().body() {
            ResponseBody::Body(Body::Bytes(ref b)) => {
                let bytes: Bytes = b.clone().into();
                assert_eq!(bytes, Bytes::from_static(b"some"));
            }
            _ => panic!(),
        }
    }

    #[ntex_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let resp: HttpResponse = responder("test").respond_to(&req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(&b"test"[..]).respond_to(&req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = responder("test".to_string()).respond_to(&req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(&"test".to_string()).respond_to(&req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(Bytes::from_static(b"test"))
            .respond_to(&req)
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = responder(BytesMut::from(b"test".as_ref()))
            .respond_to(&req)
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        // InternalError
        let resp: HttpResponse =
            responder(InternalError::new("err", StatusCode::BAD_REQUEST))
                .respond_to(&req)
                .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[ntex_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp: HttpResponse = Responder::<DefaultError>::respond_to(
            Ok::<String, std::convert::Infallible>("test".to_string()),
            &req,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let res = responder(Err::<String, _>(InternalError::new(
            "err",
            StatusCode::BAD_REQUEST,
        )))
        .respond_to(&req)
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[ntex_rt::test]
    async fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = responder("test".to_string())
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req)
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let res = responder("test".to_string())
            .with_header("content-type", "json")
            .respond_to(&req)
            .await;

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }

    #[ntex_rt::test]
    async fn test_tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = Responder::<DefaultError>::respond_to(
            ("test".to_string(), StatusCode::BAD_REQUEST),
            &req,
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let req = TestRequest::default().to_http_request();
        let res = CustomResponder::<_, DefaultError>::new((
            "test".to_string(),
            StatusCode::OK,
        ))
        .with_header("content-type", "json")
        .respond_to(&req)
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }
}
