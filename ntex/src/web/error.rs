//! Web error
use std::io::Write;
use std::str::Utf8Error;
use std::{fmt, io};

use bytes::BytesMut;
use derive_more::{Display, From};
use serde::de::value::Error as DeError;
pub use serde_json::error::Error as JsonError;
use serde_urlencoded::ser::Error as FormError;
pub use url::ParseError as UrlParseError;

use super::HttpResponse;
use crate::http::body::Body;
use crate::http::error::InternalError;
use crate::http::helpers::Writer;
use crate::http::{error, header, StatusCode};
use crate::util::timeout::TimeoutError;

/// Default error type
#[derive(Clone, Copy, Default)]
pub struct DefaultError;

pub trait WebResponseError<Err>: fmt::Debug + fmt::Display {
    /// Response's status code
    ///
    /// Internal server error is generated by default.
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// Create response for error
    ///
    /// Internal server error is generated by default.
    fn error_response(&self) -> HttpResponse {
        let mut resp = HttpResponse::new(self.status_code());
        let mut buf = BytesMut::new();
        let _ = write!(Writer(&mut buf), "{}", self);
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        resp.set_body(Body::from(buf))
    }
}

/// Generic error for error that supports `DefaultError` renderer.
pub struct Error {
    cause: Box<dyn WebResponseError<DefaultError>>,
}

impl Error {
    pub fn new<T: WebResponseError<DefaultError> + 'static>(err: T) -> Error {
        Error {
            cause: Box::new(err),
        }
    }
}

/// `Error` for any error that implements `WebResponseError<DefaultError>`
impl<T: WebResponseError<DefaultError> + 'static> From<T> for Error {
    fn from(err: T) -> Self {
        Error {
            cause: Box::new(err),
        }
    }
}

impl IntoWebError<DefaultError> for Error {
    fn into_error(self) -> WebError<DefaultError> {
        self.into()
    }
}

pub struct WebError<T> {
    cause: Box<dyn WebResponseError<T>>,
}

pub trait IntoWebError<Err>: Sized {
    fn into_error(self) -> WebError<Err>;
}

impl<Err> WebError<Err> {
    pub fn new<T: WebResponseError<Err> + 'static>(err: T) -> WebError<Err> {
        WebError {
            cause: Box::new(err),
        }
    }

    /// Returns the reference to the underlying `WebResponseError`.
    pub fn as_response_error(&self) -> &dyn WebResponseError<Err> {
        self.cause.as_ref()
    }
}

/// Convert `Error` to a `Response` instance
impl<Err> From<WebError<Err>> for HttpResponse {
    fn from(err: WebError<Err>) -> Self {
        err.cause.error_response()
    }
}

/// `WebError` for any error that implements `WebResponseError`
impl<T: WebResponseError<Err> + 'static, Err: 'static> IntoWebError<Err> for T {
    fn into_error(self) -> WebError<Err> {
        // use std::any::TypeId;
        // if TypeId::of::<T>() == TypeId::of::<Box<dyn WebResponseError<Err>>>() {
        //     unsafe {
        //         let t1: Box<dyn WebResponseError<Err>> = std::mem::transmute(&self);
        //         WebError { cause: t1 }
        //     }
        // } else {
        WebError {
            cause: Box::new(self),
        }
        // }
    }
}

/// Convert `Error` to a `WebError<DefaultError>` instance
impl From<Error> for WebError<DefaultError> {
    fn from(err: Error) -> Self {
        WebError { cause: err.cause }
    }
}

impl<Err> WebResponseError<Err> for WebError<Err> {
    #[inline]
    fn status_code(&self) -> StatusCode {
        self.cause.status_code()
    }

    fn error_response(&self) -> HttpResponse {
        self.cause.error_response()
    }
}

impl<Err> error::ResponseError for WebError<Err> {
    fn error_response(&self) -> HttpResponse {
        self.cause.error_response()
    }
}

impl<Err> fmt::Display for WebError<Err> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl<Err> fmt::Debug for WebError<Err> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.cause, f)
    }
}

/// Return `GATEWAY_TIMEOUT` for `TimeoutError`
impl<E: WebResponseError<DefaultError>> WebResponseError<DefaultError>
    for TimeoutError<E>
{
    fn status_code(&self) -> StatusCode {
        match self {
            TimeoutError::Service(e) => e.status_code(),
            TimeoutError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

/// `InternalServerError` for `JsonError`
impl WebResponseError<DefaultError> for JsonError {}

/// `InternalServerError` for `FormError`
impl WebResponseError<DefaultError> for FormError {}

#[cfg(feature = "openssl")]
/// `InternalServerError` for `openssl::ssl::Error`
impl WebResponseError<DefaultError> for actix_connect::ssl::openssl::SslError {}

#[cfg(feature = "openssl")]
/// `InternalServerError` for `openssl::ssl::HandshakeError`
impl<T: std::fmt::Debug> WebResponseError<DefaultError>
    for actix_tls::openssl::HandshakeError<T>
{
}

/// Return `BAD_REQUEST` for `de::value::Error`
impl WebResponseError<DefaultError> for DeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// `InternalServerError` for `Canceled`
impl WebResponseError<DefaultError> for error::Canceled {}

/// `InternalServerError` for `BlockingError`
impl<E: fmt::Debug> WebResponseError<DefaultError> for error::BlockingError<E> {}

/// Return `BAD_REQUEST` for `Utf8Error`
impl WebResponseError<DefaultError> for Utf8Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Return `InternalServerError` for `HttpError`,
/// Response generation can return `HttpError`, so it is internal error
impl WebResponseError<DefaultError> for error::HttpError {}

/// Return `InternalServerError` for `io::Error`
impl WebResponseError<DefaultError> for io::Error {
    fn status_code(&self) -> StatusCode {
        match self.kind() {
            io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
            io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Errors which can occur when attempting to generate resource uri.
#[derive(Debug, PartialEq, Display, From)]
pub enum UrlGenerationError {
    /// Resource not found
    #[display(fmt = "Resource not found")]
    ResourceNotFound,
    /// Not all path pattern covered
    #[display(fmt = "Not all path pattern covered")]
    NotEnoughElements,
    /// URL parse error
    #[display(fmt = "{}", _0)]
    ParseError(UrlParseError),
}

/// `InternalServerError` for `UrlGeneratorError`
impl WebResponseError<DefaultError> for UrlGenerationError {}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Debug, Display, From)]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding
    #[display(fmt = "Can not decode chunked transfer encoding")]
    Chunked,
    /// Payload size is bigger than allowed. (default: 256kB)
    #[display(
        fmt = "Urlencoded payload size is bigger ({} bytes) than allowed (default: {} bytes)",
        size,
        limit
    )]
    Overflow { size: usize, limit: usize },
    /// Payload size is now known
    #[display(fmt = "Payload size is now known")]
    UnknownLength,
    /// Content type error
    #[display(fmt = "Content type error")]
    ContentType,
    /// Parse error
    #[display(fmt = "Parse error")]
    Parse,
    /// Payload error
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(error::PayloadError),
}

/// Response renderer for `UrlencodedError`
impl<DefaultError> WebResponseError<DefaultError> for UrlencodedError {
    fn status_code(&self) -> StatusCode {
        match *self {
            UrlencodedError::Overflow { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            UrlencodedError::UnknownLength => StatusCode::LENGTH_REQUIRED,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Display, From)]
pub enum JsonPayloadError {
    /// Payload size is bigger than allowed. (default: 32kB)
    #[display(fmt = "Json payload size is bigger than allowed")]
    Overflow,
    /// Content type error
    #[display(fmt = "Content type error")]
    ContentType,
    /// Deserialize error
    #[display(fmt = "Json deserialize error: {}", _0)]
    Deserialize(serde_json::error::Error),
    /// Payload error
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(error::PayloadError),
}

/// Return `BadRequest` for `JsonPayloadError`
impl<DefaultError> WebResponseError<DefaultError> for JsonPayloadError {
    fn status_code(&self) -> StatusCode {
        match *self {
            JsonPayloadError::Overflow => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// A set of errors that can occur during parsing request paths
#[derive(Debug, Display, From)]
pub enum PathError {
    /// Deserialize error
    #[display(fmt = "Path deserialize error: {}", _0)]
    Deserialize(serde::de::value::Error),
}

/// Error renderer for `PathError`
impl<DefaultError> WebResponseError<DefaultError> for PathError {
    fn status_code(&self) -> StatusCode {
        StatusCode::NOT_FOUND
    }
}

/// A set of errors that can occur during parsing query strings
#[derive(Debug, Display, From)]
pub enum QueryPayloadError {
    /// Deserialize error
    #[display(fmt = "Query deserialize error: {}", _0)]
    Deserialize(serde::de::value::Error),
}

/// Error renderer `QueryPayloadError`
impl<DefaultError> WebResponseError<DefaultError> for QueryPayloadError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

#[derive(Debug, Display, From)]
pub enum PayloadError {
    /// Http error.
    #[display(fmt = "{:?}", _0)]
    Http(error::HttpError),
    #[display(fmt = "{}", _0)]
    Payload(error::PayloadError),
    #[display(fmt = "{}", _0)]
    ContentType(error::ContentTypeError),
    #[display(fmt = "Can not decode body")]
    Decoding,
}

impl<DefaultError> WebResponseError<DefaultError> for PayloadError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl<T, Err> WebResponseError<Err> for InternalError<T>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    fn error_response(&self) -> HttpResponse {
        use crate::http::error::ResponseError;
        ResponseError::error_response(self)
    }
}

/// Helper function that creates wrapper of any error and generate *BAD
/// REQUEST* response.
#[allow(non_snake_case)]
pub fn ErrorBadRequest<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::BAD_REQUEST).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *UNAUTHORIZED* response.
#[allow(non_snake_case)]
pub fn ErrorUnauthorized<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNAUTHORIZED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *PAYMENT_REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorPaymentRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PAYMENT_REQUIRED).into_error()
}

/// Helper function that creates wrapper of any error and generate *FORBIDDEN*
/// response.
#[allow(non_snake_case)]
pub fn ErrorForbidden<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::FORBIDDEN).into_error()
}

/// Helper function that creates wrapper of any error and generate *NOT FOUND*
/// response.
#[allow(non_snake_case)]
pub fn ErrorNotFound<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_FOUND).into_error()
}

/// Helper function that creates wrapper of any error and generate *METHOD NOT
/// ALLOWED* response.
#[allow(non_snake_case)]
pub fn ErrorMethodNotAllowed<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::METHOD_NOT_ALLOWED).into_error()
}

/// Helper function that creates wrapper of any error and generate *NOT
/// ACCEPTABLE* response.
#[allow(non_snake_case)]
pub fn ErrorNotAcceptable<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_ACCEPTABLE).into_error()
}

/// Helper function that creates wrapper of any error and generate *PROXY
/// AUTHENTICATION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorProxyAuthenticationRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PROXY_AUTHENTICATION_REQUIRED).into_error()
}

/// Helper function that creates wrapper of any error and generate *REQUEST
/// TIMEOUT* response.
#[allow(non_snake_case)]
pub fn ErrorRequestTimeout<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::REQUEST_TIMEOUT).into_error()
}

/// Helper function that creates wrapper of any error and generate *CONFLICT*
/// response.
#[allow(non_snake_case)]
pub fn ErrorConflict<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::CONFLICT).into_error()
}

/// Helper function that creates wrapper of any error and generate *GONE*
/// response.
#[allow(non_snake_case)]
pub fn ErrorGone<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::GONE).into_error()
}

/// Helper function that creates wrapper of any error and generate *LENGTH
/// REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorLengthRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LENGTH_REQUIRED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *PAYLOAD TOO LARGE* response.
#[allow(non_snake_case)]
pub fn ErrorPayloadTooLarge<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PAYLOAD_TOO_LARGE).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *URI TOO LONG* response.
#[allow(non_snake_case)]
pub fn ErrorUriTooLong<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::URI_TOO_LONG).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *UNSUPPORTED MEDIA TYPE* response.
#[allow(non_snake_case)]
pub fn ErrorUnsupportedMediaType<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNSUPPORTED_MEDIA_TYPE).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *RANGE NOT SATISFIABLE* response.
#[allow(non_snake_case)]
pub fn ErrorRangeNotSatisfiable<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::RANGE_NOT_SATISFIABLE).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *IM A TEAPOT* response.
#[allow(non_snake_case)]
pub fn ErrorImATeapot<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::IM_A_TEAPOT).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *MISDIRECTED REQUEST* response.
#[allow(non_snake_case)]
pub fn ErrorMisdirectedRequest<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::MISDIRECTED_REQUEST).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *UNPROCESSABLE ENTITY* response.
#[allow(non_snake_case)]
pub fn ErrorUnprocessableEntity<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNPROCESSABLE_ENTITY).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *LOCKED* response.
#[allow(non_snake_case)]
pub fn ErrorLocked<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LOCKED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *FAILED DEPENDENCY* response.
#[allow(non_snake_case)]
pub fn ErrorFailedDependency<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::FAILED_DEPENDENCY).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *UPGRADE REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorUpgradeRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UPGRADE_REQUIRED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *PRECONDITION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorPreconditionFailed<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PRECONDITION_FAILED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *PRECONDITION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorPreconditionRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PRECONDITION_REQUIRED).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *TOO MANY REQUESTS* response.
#[allow(non_snake_case)]
pub fn ErrorTooManyRequests<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::TOO_MANY_REQUESTS).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *REQUEST HEADER FIELDS TOO LARGE* response.
#[allow(non_snake_case)]
pub fn ErrorRequestHeaderFieldsTooLarge<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *UNAVAILABLE FOR LEGAL REASONS* response.
#[allow(non_snake_case)]
pub fn ErrorUnavailableForLegalReasons<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS).into_error()
}

/// Helper function that creates wrapper of any error and generate
/// *EXPECTATION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorExpectationFailed<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::EXPECTATION_FAILED).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *INTERNAL SERVER ERROR* response.
#[allow(non_snake_case)]
pub fn ErrorInternalServerError<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::INTERNAL_SERVER_ERROR).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *NOT IMPLEMENTED* response.
#[allow(non_snake_case)]
pub fn ErrorNotImplemented<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_IMPLEMENTED).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *BAD GATEWAY* response.
#[allow(non_snake_case)]
pub fn ErrorBadGateway<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::BAD_GATEWAY).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *SERVICE UNAVAILABLE* response.
#[allow(non_snake_case)]
pub fn ErrorServiceUnavailable<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::SERVICE_UNAVAILABLE).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *GATEWAY TIMEOUT* response.
#[allow(non_snake_case)]
pub fn ErrorGatewayTimeout<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::GATEWAY_TIMEOUT).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *HTTP VERSION NOT SUPPORTED* response.
#[allow(non_snake_case)]
pub fn ErrorHttpVersionNotSupported<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::HTTP_VERSION_NOT_SUPPORTED).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *VARIANT ALSO NEGOTIATES* response.
#[allow(non_snake_case)]
pub fn ErrorVariantAlsoNegotiates<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::VARIANT_ALSO_NEGOTIATES).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *INSUFFICIENT STORAGE* response.
#[allow(non_snake_case)]
pub fn ErrorInsufficientStorage<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::INSUFFICIENT_STORAGE).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *LOOP DETECTED* response.
#[allow(non_snake_case)]
pub fn ErrorLoopDetected<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LOOP_DETECTED).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *NOT EXTENDED* response.
#[allow(non_snake_case)]
pub fn ErrorNotExtended<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_EXTENDED).into_error()
}

/// Helper function that creates wrapper of any error and
/// generate *NETWORK AUTHENTICATION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorNetworkAuthenticationRequired<T, Err: 'static>(err: T) -> WebError<Err>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NETWORK_AUTHENTICATION_REQUIRED).into_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoded_error() {
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &UrlencodedError::Overflow { size: 0, limit: 0 },
        );
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &UrlencodedError::UnknownLength,
        );
        assert_eq!(resp.status(), StatusCode::LENGTH_REQUIRED);
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &UrlencodedError::ContentType,
        );
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_json_payload_error() {
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &JsonPayloadError::Overflow,
        );
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &JsonPayloadError::ContentType,
        );
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_query_payload_error() {
        let resp: HttpResponse = WebResponseError::<DefaultError>::error_response(
            &QueryPayloadError::Deserialize(
                serde_urlencoded::from_str::<i32>("bad query").unwrap_err(),
            ),
        );
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_error_helpers() {
        let r: HttpResponse = ErrorBadRequest::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);

        let r: HttpResponse = ErrorUnauthorized::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        let r: HttpResponse = ErrorPaymentRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::PAYMENT_REQUIRED);

        let r: HttpResponse = ErrorForbidden::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        let r: HttpResponse = ErrorNotFound::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        let r: HttpResponse = ErrorMethodNotAllowed::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);

        let r: HttpResponse = ErrorNotAcceptable::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::NOT_ACCEPTABLE);

        let r: HttpResponse =
            ErrorProxyAuthenticationRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);

        let r: HttpResponse = ErrorRequestTimeout::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::REQUEST_TIMEOUT);

        let r: HttpResponse = ErrorConflict::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::CONFLICT);

        let r: HttpResponse = ErrorGone::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::GONE);

        let r: HttpResponse = ErrorLengthRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::LENGTH_REQUIRED);

        let r: HttpResponse = ErrorPreconditionFailed::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);

        let r: HttpResponse = ErrorPayloadTooLarge::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let r: HttpResponse = ErrorUriTooLong::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::URI_TOO_LONG);

        let r: HttpResponse = ErrorUnsupportedMediaType::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let r: HttpResponse = ErrorRangeNotSatisfiable::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let r: HttpResponse = ErrorExpectationFailed::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::EXPECTATION_FAILED);

        let r: HttpResponse = ErrorImATeapot::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::IM_A_TEAPOT);

        let r: HttpResponse = ErrorMisdirectedRequest::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST);

        let r: HttpResponse = ErrorUnprocessableEntity::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let r: HttpResponse = ErrorLocked::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::LOCKED);

        let r: HttpResponse = ErrorFailedDependency::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::FAILED_DEPENDENCY);

        let r: HttpResponse = ErrorUpgradeRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::UPGRADE_REQUIRED);

        let r: HttpResponse = ErrorPreconditionRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::PRECONDITION_REQUIRED);

        let r: HttpResponse = ErrorTooManyRequests::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);

        let r: HttpResponse =
            ErrorRequestHeaderFieldsTooLarge::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);

        let r: HttpResponse =
            ErrorUnavailableForLegalReasons::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);

        let r: HttpResponse = ErrorInternalServerError::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let r: HttpResponse = ErrorNotImplemented::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::NOT_IMPLEMENTED);

        let r: HttpResponse = ErrorBadGateway::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);

        let r: HttpResponse = ErrorServiceUnavailable::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);

        let r: HttpResponse = ErrorGatewayTimeout::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::GATEWAY_TIMEOUT);

        let r: HttpResponse =
            ErrorHttpVersionNotSupported::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::HTTP_VERSION_NOT_SUPPORTED);

        let r: HttpResponse =
            ErrorVariantAlsoNegotiates::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::VARIANT_ALSO_NEGOTIATES);

        let r: HttpResponse = ErrorInsufficientStorage::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);

        let r: HttpResponse = ErrorLoopDetected::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::LOOP_DETECTED);

        let r: HttpResponse = ErrorNotExtended::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::NOT_EXTENDED);

        let r: HttpResponse =
            ErrorNetworkAuthenticationRequired::<_, DefaultError>("err").into();
        assert_eq!(r.status(), StatusCode::NETWORK_AUTHENTICATION_REQUIRED);
    }
}
