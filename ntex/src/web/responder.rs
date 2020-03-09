use std::convert::TryFrom;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures::future::{err, ok, Either as EitherFuture, Ready};
use futures::ready;
use pin_project::{pin_project, project};

use crate::http::error::{HttpError, InternalError};
use crate::http::header::{HeaderMap, HeaderName, IntoHeaderValue};
use crate::http::{Response, ResponseBuilder, StatusCode};

use super::error::{DefaultError, IntoWebError, WebError};
use super::request::HttpRequest;

/// Trait implemented by types that can be converted to a http response.
///
/// Types that implement this trait can be used as the return type of a handler.
pub trait Responder<Err = DefaultError> {
    /// The associated error which can be returned.
    type Error: IntoWebError<Err>;

    /// The future response value.
    type Future: Future<Output = Result<Response, Self::Error>>;

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
    ///         MyObj{name: "Name".to_string()}
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

impl<Err: 'static> Responder<Err> for Response {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, WebError<Err>>>;

    #[inline]
    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(self)
    }
}

impl<T, Err> Responder<Err> for Option<T>
where
    T: Responder<Err>,
{
    type Error = T::Error;
    type Future = EitherFuture<T::Future, Ready<Result<Response, T::Error>>>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Some(t) => EitherFuture::Left(t.respond_to(req)),
            None => {
                EitherFuture::Right(ok(Response::build(StatusCode::NOT_FOUND).finish()))
            }
        }
    }
}

impl<T, E, Err> Responder<Err> for Result<T, E>
where
    T: Responder<Err>,
    E: IntoWebError<Err>,
    Err: 'static,
{
    type Error = WebError<Err>;
    type Future = EitherFuture<
        ResponseFuture<T::Future, T::Error, Err>,
        Ready<Result<Response, WebError<Err>>>,
    >;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Ok(val) => EitherFuture::Left(ResponseFuture::new(val.respond_to(req))),
            Err(e) => EitherFuture::Right(err(e.into_error())),
        }
    }
}

impl<Err: 'static> Responder<Err> for ResponseBuilder {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, WebError<Err>>>;

    #[inline]
    fn respond_to(mut self, _: &HttpRequest) -> Self::Future {
        ok(self.finish())
    }
}

impl<T, Err> Responder<Err> for (T, StatusCode)
where
    T: Responder<Err>,
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

impl<Err: 'static> Responder<Err> for &'static str {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl<Err: 'static> Responder<Err> for &'static [u8] {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl<Err: 'static> Responder<Err> for String {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl<'a, Err: 'static> Responder<Err> for &'a String {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self))
    }
}

impl<Err: 'static> Responder<Err> for Bytes {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
    }
}

impl<Err: 'static> Responder<Err> for BytesMut {
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        ok(Response::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self))
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

impl<T: Responder<Err>, Err> Responder<Err> for CustomResponder<T, Err> {
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
pub struct CustomResponderFut<T: Responder<Err>, Err> {
    #[pin]
    fut: T::Future,
    status: Option<StatusCode>,
    headers: Option<HeaderMap>,
}

impl<T: Responder<Err>, Err> Future for CustomResponderFut<T, Err> {
    type Output = Result<Response, T::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let mut res = match ready!(this.fut.poll(cx)) {
            Ok(res) => res,
            Err(e) => return Poll::Ready(Err(e)),
        };
        if let Some(status) = this.status.take() {
            *res.status_mut() = status;
        }
        if let Some(ref headers) = this.headers {
            for (k, v) in headers {
                res.headers_mut().insert(k.clone(), v.clone());
            }
        }
        Poll::Ready(Ok(res))
    }
}

/// Combines two different responder types into a single type
///
/// ```rust
/// use ntex::http::Error;
/// use ntex::web::{Either, HttpResponse};
///
/// type RegisterResult = Either<HttpResponse, Result<HttpResponse, Error>>;
///
/// fn index() -> RegisterResult {
///     if is_a_variant() {
///         // <- choose left variant
///         Either::A(HttpResponse::BadRequest().body("Bad data"))
///     } else {
///         Either::B(
///             // <- Right variant
///             Ok(HttpResponse::Ok()
///                 .content_type("text/html")
///                 .body("Hello!"))
///         )
///     }
/// }
/// # fn is_a_variant() -> bool { true }
/// # fn main() {}
/// ```
#[derive(Debug, PartialEq)]
pub enum Either<A, B> {
    /// First branch of the type
    A(A),
    /// Second branch of the type
    B(B),
}

impl<A, B, Err> Responder<Err> for Either<A, B>
where
    A: Responder<Err>,
    B: Responder<Err>,
    Err: 'static,
{
    type Error = WebError<Err>;
    type Future = EitherResponder<A, B, Err>;

    fn respond_to(self, req: &HttpRequest) -> Self::Future {
        match self {
            Either::A(a) => EitherResponder::A(a.respond_to(req)),
            Either::B(b) => EitherResponder::B(b.respond_to(req)),
        }
    }
}

#[pin_project]
pub enum EitherResponder<A, B, Err>
where
    A: Responder<Err>,
    B: Responder<Err>,
{
    A(#[pin] A::Future),
    B(#[pin] B::Future),
}

impl<A, B, Err> Future for EitherResponder<A, B, Err>
where
    A: Responder<Err>,
    B: Responder<Err>,
    Err: 'static,
{
    type Output = Result<Response, WebError<Err>>;

    #[project]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[project]
        match self.project() {
            EitherResponder::A(fut) => {
                Poll::Ready(ready!(fut.poll(cx)).map_err(|e| e.into_error()))
            }
            EitherResponder::B(fut) => {
                Poll::Ready(ready!(fut.poll(cx).map_err(|e| e.into_error())))
            }
        }
    }
}

impl<T, Err> Responder<Err> for InternalError<T>
where
    T: std::fmt::Debug + std::fmt::Display + 'static,
    Err: 'static,
{
    type Error = WebError<Err>;
    type Future = Ready<Result<Response, Self::Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let err: crate::http::Error = self.into();
        ok(err.into())
    }
}

#[pin_project]
pub struct ResponseFuture<T, E, Err> {
    #[pin]
    fut: T,
    _t: PhantomData<(E, Err)>,
}

impl<T, E, Err> ResponseFuture<T, E, Err> {
    pub fn new(fut: T) -> Self {
        ResponseFuture {
            fut,
            _t: PhantomData,
        }
    }
}

impl<T, E, Err> Future for ResponseFuture<T, E, Err>
where
    T: Future<Output = Result<Response, E>>,
    E: IntoWebError<Err>,
    Err: 'static,
{
    type Output = Result<Response, WebError<Err>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(ready!(self.project().fut.poll(cx)).map_err(|e| e.into_error()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::http::body::{Body, ResponseBody};
    use crate::http::header::{HeaderValue, CONTENT_TYPE};
    use crate::http::{error, Response as HttpResponse, StatusCode};
    use crate::web;
    use crate::web::test::{init_service, TestRequest};
    use crate::Service;

    fn responder<T: Responder<DefaultError>>(
        responder: T,
    ) -> impl Responder<DefaultError, Error = T::Error> {
        responder
    }

    #[actix_rt::test]
    async fn test_option_responder() {
        let mut srv = init_service(
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

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let resp: HttpResponse = responder("test").respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(&b"test"[..]).respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = responder("test".to_string())
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(&"test".to_string())
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let resp: HttpResponse = responder(Bytes::from_static(b"test"))
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        let resp: HttpResponse = responder(BytesMut::from(b"test".as_ref()))
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        // InternalError
        let resp: HttpResponse =
            responder(error::InternalError::new("err", StatusCode::BAD_REQUEST))
                .respond_to(&req)
                .await
                .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_result_responder() {
        let req = TestRequest::default().to_http_request();

        // Result<I, E>
        let resp: HttpResponse = Ok::<_, web::Error>("test".to_string())
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().bin_ref(), b"test");
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );

        let res = responder(Err::<String, _>(error::InternalError::new(
            "err",
            StatusCode::BAD_REQUEST,
        )))
        .respond_to(&req)
        .await;
        assert!(res.is_err());
    }

    #[actix_rt::test]
    async fn test_custom_responder() {
        let req = TestRequest::default().to_http_request();
        let res = responder("test".to_string())
            .with_status(StatusCode::BAD_REQUEST)
            .respond_to(&req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let res = responder("test".to_string())
            .with_header("content-type", "json")
            .respond_to(&req)
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }

    #[actix_rt::test]
    async fn test_tuple_responder_with_status_code() {
        let req = TestRequest::default().to_http_request();
        let res = Responder::<DefaultError>::respond_to(
            ("test".to_string(), StatusCode::BAD_REQUEST),
            &req,
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.body().bin_ref(), b"test");

        let req = TestRequest::default().to_http_request();
        let res = CustomResponder::<_, DefaultError>::new((
            "test".to_string(),
            StatusCode::OK,
        ))
        .with_header("content-type", "json")
        .respond_to(&req)
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.body().bin_ref(), b"test");
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("json")
        );
    }
}
