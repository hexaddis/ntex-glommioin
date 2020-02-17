//! Http related errors
use std::any::TypeId;
use std::cell::RefCell;
use std::io::Write;
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use std::{fmt, io};

use actix_codec::{Decoder, Encoder};
use actix_utils::framed::DispatcherError as FramedDispatcherError;
use actix_utils::timeout::TimeoutError;
use bytes::BytesMut;
use derive_more::{Display, From};
use http::uri::InvalidUri;
use http::{header, StatusCode};
use httparse;
use serde::de::value::Error as DeError;
use serde_json::error::Error as JsonError;
use serde_urlencoded::ser::Error as FormError;

#[cfg(feature = "cookie")]
pub use coo_kie::ParseError as CookieParseError;

// re-export for convinience
pub use actix_threadpool::BlockingError;
pub use futures::channel::oneshot::Canceled;
pub use http::Error as HttpError;

use super::body::Body;
use super::helpers::Writer;
use super::response::{Response, ResponseBuilder};

/// General purpose actix web error.
///
/// An actix web error is used to carry errors from `failure` or `std::error`
/// through actix in a convenient way.  It can be created through
/// converting errors with `into()`.
///
/// Whenever it is created from an external object a response error is created
/// for it that can be used to create an http response from it this means that
/// if you have access to an actix `Error` you can always get a
/// `ResponseError` reference from it.
pub struct Error {
    cause: Box<dyn ResponseError>,
}

impl Error {
    /// Returns the reference to the underlying `ResponseError`.
    pub fn as_response_error(&self) -> &dyn ResponseError {
        self.cause.as_ref()
    }

    /// Similar to `as_response_error` but downcasts.
    pub fn as_error<T: ResponseError + 'static>(&self) -> Option<&T> {
        ResponseError::downcast_ref(self.cause.as_ref())
    }
}

/// Error that can be converted to `Response`
pub trait ResponseError: fmt::Debug + fmt::Display {
    /// Response's status code
    ///
    /// Internal server error is generated by default.
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// Create response for error
    ///
    /// Internal server error is generated by default.
    fn error_response(&self) -> Response {
        let mut resp = Response::new(self.status_code());
        let mut buf = BytesMut::new();
        let _ = write!(Writer(&mut buf), "{}", self);
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        resp.set_body(Body::from(buf))
    }

    #[doc(hidden)]
    fn __private_get_type_id__(&self) -> TypeId
    where
        Self: 'static,
    {
        TypeId::of::<Self>()
    }
}

impl dyn ResponseError + 'static {
    /// Downcasts a response error to a specific type.
    pub fn downcast_ref<T: ResponseError + 'static>(&self) -> Option<&T> {
        if self.__private_get_type_id__() == TypeId::of::<T>() {
            unsafe { Some(&*(self as *const dyn ResponseError as *const T)) }
        } else {
            None
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.cause)
    }
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        None
    }

    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<()> for Error {
    fn from(_: ()) -> Self {
        Error::from(UnitError)
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(_: std::convert::Infallible) -> Self {
        // `std::convert::Infallible` indicates an error
        // that will never happen
        unreachable!()
    }
}

/// Convert `Error` to a `Response` instance
impl From<Error> for Response {
    fn from(err: Error) -> Self {
        Response::from_error(err)
    }
}

/// `Error` for any error that implements `ResponseError`
impl<T: ResponseError + 'static> From<T> for Error {
    fn from(err: T) -> Error {
        Error {
            cause: Box::new(err),
        }
    }
}

/// Convert Response to a Error
impl From<Response> for Error {
    fn from(res: Response) -> Error {
        InternalError::from_response("", res).into()
    }
}

/// Convert ResponseBuilder to a Error
impl From<ResponseBuilder> for Error {
    fn from(mut res: ResponseBuilder) -> Error {
        InternalError::from_response("", res.finish()).into()
    }
}

/// Return `GATEWAY_TIMEOUT` for `TimeoutError`
impl<E: ResponseError> ResponseError for TimeoutError<E> {
    fn status_code(&self) -> StatusCode {
        match self {
            TimeoutError::Service(e) => e.status_code(),
            TimeoutError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

#[derive(Debug, Display)]
#[display(fmt = "UnknownError")]
struct UnitError;

/// `InternalServerError` for `UnitError`
impl ResponseError for UnitError {}

/// `InternalServerError` for `JsonError`
impl ResponseError for JsonError {}

/// `InternalServerError` for `FormError`
impl ResponseError for FormError {}

#[cfg(feature = "openssl")]
/// `InternalServerError` for `openssl::ssl::Error`
impl ResponseError for actix_connect::ssl::openssl::SslError {}

#[cfg(feature = "openssl")]
/// `InternalServerError` for `openssl::ssl::HandshakeError`
impl<T: std::fmt::Debug> ResponseError for actix_tls::openssl::HandshakeError<T> {}

/// Return `BAD_REQUEST` for `de::value::Error`
impl ResponseError for DeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// `InternalServerError` for `Canceled`
impl ResponseError for Canceled {}

/// `InternalServerError` for `BlockingError`
impl<E: fmt::Debug> ResponseError for BlockingError<E> {}

/// Return `BAD_REQUEST` for `Utf8Error`
impl ResponseError for Utf8Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Return `InternalServerError` for `HttpError`,
/// Response generation can return `HttpError`, so it is internal error
impl ResponseError for HttpError {}

/// Return `InternalServerError` for `io::Error`
impl ResponseError for io::Error {
    fn status_code(&self) -> StatusCode {
        match self.kind() {
            io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
            io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// `BadRequest` for `InvalidHeaderValue`
impl ResponseError for header::InvalidHeaderValue {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// A set of errors that can occur during parsing HTTP streams
#[derive(Debug, Display)]
pub enum ParseError {
    /// An invalid `Method`, such as `GE.T`.
    #[display(fmt = "Invalid Method specified")]
    Method,
    /// An invalid `Uri`, such as `exam ple.domain`.
    #[display(fmt = "Uri error: {}", _0)]
    Uri(InvalidUri),
    /// An invalid `HttpVersion`, such as `HTP/1.1`
    #[display(fmt = "Invalid HTTP version specified")]
    Version,
    /// An invalid `Header`.
    #[display(fmt = "Invalid Header provided")]
    Header,
    /// A message head is too large to be reasonable.
    #[display(fmt = "Message head is too large")]
    TooLarge,
    /// A message reached EOF, but is not complete.
    #[display(fmt = "Message is incomplete")]
    Incomplete,
    /// An invalid `Status`, such as `1337 ELITE`.
    #[display(fmt = "Invalid Status provided")]
    Status,
    /// A timeout occurred waiting for an IO event.
    #[allow(dead_code)]
    #[display(fmt = "Timeout")]
    Timeout,
    /// An `io::Error` that occurred while trying to read or write to a network
    /// stream.
    #[display(fmt = "IO error: {}", _0)]
    Io(io::Error),
    /// Parsing a field as string failed
    #[display(fmt = "UTF8 error: {}", _0)]
    Utf8(Utf8Error),
}

/// Return `BadRequest` for `ParseError`
impl ResponseError for ParseError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl From<io::Error> for ParseError {
    fn from(err: io::Error) -> ParseError {
        ParseError::Io(err)
    }
}

impl From<InvalidUri> for ParseError {
    fn from(err: InvalidUri) -> ParseError {
        ParseError::Uri(err)
    }
}

impl From<Utf8Error> for ParseError {
    fn from(err: Utf8Error) -> ParseError {
        ParseError::Utf8(err)
    }
}

impl From<FromUtf8Error> for ParseError {
    fn from(err: FromUtf8Error) -> ParseError {
        ParseError::Utf8(err.utf8_error())
    }
}

impl From<httparse::Error> for ParseError {
    fn from(err: httparse::Error) -> ParseError {
        match err {
            httparse::Error::HeaderName
            | httparse::Error::HeaderValue
            | httparse::Error::NewLine
            | httparse::Error::Token => ParseError::Header,
            httparse::Error::Status => ParseError::Status,
            httparse::Error::TooManyHeaders => ParseError::TooLarge,
            httparse::Error::Version => ParseError::Version,
        }
    }
}

#[derive(Display, Debug)]
/// A set of errors that can occur during payload parsing
pub enum PayloadError {
    /// A payload reached EOF, but is not complete.
    #[display(
        fmt = "A payload reached EOF, but is not complete. With error: {:?}",
        _0
    )]
    Incomplete(Option<io::Error>),
    /// Content encoding stream corruption
    #[display(fmt = "Can not decode content-encoding.")]
    EncodingCorrupted,
    /// A payload reached size limit.
    #[display(fmt = "A payload reached size limit.")]
    Overflow,
    /// A payload length is unknown.
    #[display(fmt = "A payload length is unknown.")]
    UnknownLength,
    /// Http2 payload error
    #[display(fmt = "{}", _0)]
    Http2Payload(h2::Error),
    /// Io error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

impl From<h2::Error> for PayloadError {
    fn from(err: h2::Error) -> Self {
        PayloadError::Http2Payload(err)
    }
}

impl From<Option<io::Error>> for PayloadError {
    fn from(err: Option<io::Error>) -> Self {
        PayloadError::Incomplete(err)
    }
}

impl From<io::Error> for PayloadError {
    fn from(err: io::Error) -> Self {
        PayloadError::Incomplete(Some(err))
    }
}

impl From<BlockingError<io::Error>> for PayloadError {
    fn from(err: BlockingError<io::Error>) -> Self {
        match err {
            BlockingError::Error(e) => PayloadError::Io(e),
            BlockingError::Canceled => PayloadError::Io(io::Error::new(
                io::ErrorKind::Other,
                "Operation is canceled",
            )),
        }
    }
}

/// `PayloadError` returns two possible results:
///
/// - `Overflow` returns `PayloadTooLarge`
/// - Other errors returns `BadRequest`
impl ResponseError for PayloadError {
    fn status_code(&self) -> StatusCode {
        match *self {
            PayloadError::Overflow => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

#[cfg(feature = "cookie")]
/// Return `BadRequest` for `cookie::ParseError`
impl ResponseError for coo_kie::ParseError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

#[derive(Debug, Display, From)]
/// A set of errors that can occur during dispatching http requests
pub enum DispatchError {
    /// Service error
    Service(Error),

    /// Upgrade service error
    Upgrade,

    /// An `io::Error` that occurred while trying to read or write to a network
    /// stream.
    #[display(fmt = "IO error: {}", _0)]
    Io(io::Error),

    /// Http request parse error.
    #[display(fmt = "Parse error: {}", _0)]
    Parse(ParseError),

    /// Http/2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// The first request did not complete within the specified timeout.
    #[display(fmt = "The first request did not complete within the specified timeout")]
    SlowRequestTimeout,

    /// Disconnect timeout. Makes sense for ssl streams.
    #[display(fmt = "Connection shutdown timeout")]
    DisconnectTimeout,

    /// Payload is not consumed
    #[display(fmt = "Task is completed but request's payload is not consumed")]
    PayloadIsNotConsumed,

    /// Malformed request
    #[display(fmt = "Malformed request")]
    MalformedRequest,

    /// Internal error
    #[display(fmt = "Internal error")]
    InternalError,

    /// Unknown error
    #[display(fmt = "Unknown error")]
    Unknown,
}

/// A set of error that can occure during parsing content type
#[derive(PartialEq, Debug, Display)]
pub enum ContentTypeError {
    /// Can not parse content type
    #[display(fmt = "Can not parse content type")]
    ParseError,
    /// Unknown content encoding
    #[display(fmt = "Unknown content encoding")]
    UnknownEncoding,
}

/// Return `BadRequest` for `ContentTypeError`
impl ResponseError for ContentTypeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl<E, U: Encoder + Decoder> ResponseError for FramedDispatcherError<E, U>
where
    E: fmt::Debug + fmt::Display,
    <U as Encoder>::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
{
}

/// Helper type that can wrap any error and generate custom response.
///
/// In following example any `io::Error` will be converted into "BAD REQUEST"
/// response as opposite to *INTERNAL SERVER ERROR* which is defined by
/// default.
///
/// ```rust
/// use ntex::http::{error, Request};
///
/// fn index(req: Request) -> Result<&'static str, error::Error> {
///     Err(error::ErrorBadRequest(std::io::Error::new(std::io::ErrorKind::Other, "error")))
/// }
/// ```
pub struct InternalError<T> {
    cause: T,
    status: InternalErrorType,
}

enum InternalErrorType {
    Status(StatusCode),
    Response(RefCell<Option<Response>>),
}

impl<T> InternalError<T> {
    /// Create `InternalError` instance
    pub fn new(cause: T, status: StatusCode) -> Self {
        InternalError {
            cause,
            status: InternalErrorType::Status(status),
        }
    }

    /// Create `InternalError` with predefined `Response`.
    pub fn from_response(cause: T, response: Response) -> Self {
        InternalError {
            cause,
            status: InternalErrorType::Response(RefCell::new(Some(response))),
        }
    }
}

impl<T> fmt::Debug for InternalError<T>
where
    T: fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.cause, f)
    }
}

impl<T> fmt::Display for InternalError<T>
where
    T: fmt::Display + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl<T> ResponseError for InternalError<T>
where
    T: fmt::Debug + fmt::Display + 'static,
{
    fn status_code(&self) -> StatusCode {
        match self.status {
            InternalErrorType::Status(st) => st,
            InternalErrorType::Response(ref resp) => {
                if let Some(resp) = resp.borrow().as_ref() {
                    resp.head().status
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        }
    }

    fn error_response(&self) -> Response {
        match self.status {
            InternalErrorType::Status(st) => {
                let mut res = Response::new(st);
                let mut buf = BytesMut::new();
                let _ = write!(Writer(&mut buf), "{}", self);
                res.headers_mut().insert(
                    header::CONTENT_TYPE,
                    header::HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                res.set_body(Body::from(buf))
            }
            InternalErrorType::Response(ref resp) => {
                if let Some(resp) = resp.borrow_mut().take() {
                    resp
                } else {
                    Response::new(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
    }
}

/// Helper function that creates wrapper of any error and generate *BAD
/// REQUEST* response.
#[allow(non_snake_case)]
pub fn ErrorBadRequest<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::BAD_REQUEST).into()
}

/// Helper function that creates wrapper of any error and generate
/// *UNAUTHORIZED* response.
#[allow(non_snake_case)]
pub fn ErrorUnauthorized<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNAUTHORIZED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *PAYMENT_REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorPaymentRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PAYMENT_REQUIRED).into()
}

/// Helper function that creates wrapper of any error and generate *FORBIDDEN*
/// response.
#[allow(non_snake_case)]
pub fn ErrorForbidden<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::FORBIDDEN).into()
}

/// Helper function that creates wrapper of any error and generate *NOT FOUND*
/// response.
#[allow(non_snake_case)]
pub fn ErrorNotFound<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_FOUND).into()
}

/// Helper function that creates wrapper of any error and generate *METHOD NOT
/// ALLOWED* response.
#[allow(non_snake_case)]
pub fn ErrorMethodNotAllowed<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::METHOD_NOT_ALLOWED).into()
}

/// Helper function that creates wrapper of any error and generate *NOT
/// ACCEPTABLE* response.
#[allow(non_snake_case)]
pub fn ErrorNotAcceptable<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_ACCEPTABLE).into()
}

/// Helper function that creates wrapper of any error and generate *PROXY
/// AUTHENTICATION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorProxyAuthenticationRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PROXY_AUTHENTICATION_REQUIRED).into()
}

/// Helper function that creates wrapper of any error and generate *REQUEST
/// TIMEOUT* response.
#[allow(non_snake_case)]
pub fn ErrorRequestTimeout<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::REQUEST_TIMEOUT).into()
}

/// Helper function that creates wrapper of any error and generate *CONFLICT*
/// response.
#[allow(non_snake_case)]
pub fn ErrorConflict<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::CONFLICT).into()
}

/// Helper function that creates wrapper of any error and generate *GONE*
/// response.
#[allow(non_snake_case)]
pub fn ErrorGone<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::GONE).into()
}

/// Helper function that creates wrapper of any error and generate *LENGTH
/// REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorLengthRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LENGTH_REQUIRED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *PAYLOAD TOO LARGE* response.
#[allow(non_snake_case)]
pub fn ErrorPayloadTooLarge<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PAYLOAD_TOO_LARGE).into()
}

/// Helper function that creates wrapper of any error and generate
/// *URI TOO LONG* response.
#[allow(non_snake_case)]
pub fn ErrorUriTooLong<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::URI_TOO_LONG).into()
}

/// Helper function that creates wrapper of any error and generate
/// *UNSUPPORTED MEDIA TYPE* response.
#[allow(non_snake_case)]
pub fn ErrorUnsupportedMediaType<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNSUPPORTED_MEDIA_TYPE).into()
}

/// Helper function that creates wrapper of any error and generate
/// *RANGE NOT SATISFIABLE* response.
#[allow(non_snake_case)]
pub fn ErrorRangeNotSatisfiable<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::RANGE_NOT_SATISFIABLE).into()
}

/// Helper function that creates wrapper of any error and generate
/// *IM A TEAPOT* response.
#[allow(non_snake_case)]
pub fn ErrorImATeapot<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::IM_A_TEAPOT).into()
}

/// Helper function that creates wrapper of any error and generate
/// *MISDIRECTED REQUEST* response.
#[allow(non_snake_case)]
pub fn ErrorMisdirectedRequest<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::MISDIRECTED_REQUEST).into()
}

/// Helper function that creates wrapper of any error and generate
/// *UNPROCESSABLE ENTITY* response.
#[allow(non_snake_case)]
pub fn ErrorUnprocessableEntity<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNPROCESSABLE_ENTITY).into()
}

/// Helper function that creates wrapper of any error and generate
/// *LOCKED* response.
#[allow(non_snake_case)]
pub fn ErrorLocked<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LOCKED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *FAILED DEPENDENCY* response.
#[allow(non_snake_case)]
pub fn ErrorFailedDependency<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::FAILED_DEPENDENCY).into()
}

/// Helper function that creates wrapper of any error and generate
/// *UPGRADE REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorUpgradeRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UPGRADE_REQUIRED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *PRECONDITION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorPreconditionFailed<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PRECONDITION_FAILED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *PRECONDITION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorPreconditionRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::PRECONDITION_REQUIRED).into()
}

/// Helper function that creates wrapper of any error and generate
/// *TOO MANY REQUESTS* response.
#[allow(non_snake_case)]
pub fn ErrorTooManyRequests<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::TOO_MANY_REQUESTS).into()
}

/// Helper function that creates wrapper of any error and generate
/// *REQUEST HEADER FIELDS TOO LARGE* response.
#[allow(non_snake_case)]
pub fn ErrorRequestHeaderFieldsTooLarge<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE).into()
}

/// Helper function that creates wrapper of any error and generate
/// *UNAVAILABLE FOR LEGAL REASONS* response.
#[allow(non_snake_case)]
pub fn ErrorUnavailableForLegalReasons<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS).into()
}

/// Helper function that creates wrapper of any error and generate
/// *EXPECTATION FAILED* response.
#[allow(non_snake_case)]
pub fn ErrorExpectationFailed<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::EXPECTATION_FAILED).into()
}

/// Helper function that creates wrapper of any error and
/// generate *INTERNAL SERVER ERROR* response.
#[allow(non_snake_case)]
pub fn ErrorInternalServerError<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::INTERNAL_SERVER_ERROR).into()
}

/// Helper function that creates wrapper of any error and
/// generate *NOT IMPLEMENTED* response.
#[allow(non_snake_case)]
pub fn ErrorNotImplemented<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_IMPLEMENTED).into()
}

/// Helper function that creates wrapper of any error and
/// generate *BAD GATEWAY* response.
#[allow(non_snake_case)]
pub fn ErrorBadGateway<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::BAD_GATEWAY).into()
}

/// Helper function that creates wrapper of any error and
/// generate *SERVICE UNAVAILABLE* response.
#[allow(non_snake_case)]
pub fn ErrorServiceUnavailable<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::SERVICE_UNAVAILABLE).into()
}

/// Helper function that creates wrapper of any error and
/// generate *GATEWAY TIMEOUT* response.
#[allow(non_snake_case)]
pub fn ErrorGatewayTimeout<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::GATEWAY_TIMEOUT).into()
}

/// Helper function that creates wrapper of any error and
/// generate *HTTP VERSION NOT SUPPORTED* response.
#[allow(non_snake_case)]
pub fn ErrorHttpVersionNotSupported<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::HTTP_VERSION_NOT_SUPPORTED).into()
}

/// Helper function that creates wrapper of any error and
/// generate *VARIANT ALSO NEGOTIATES* response.
#[allow(non_snake_case)]
pub fn ErrorVariantAlsoNegotiates<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::VARIANT_ALSO_NEGOTIATES).into()
}

/// Helper function that creates wrapper of any error and
/// generate *INSUFFICIENT STORAGE* response.
#[allow(non_snake_case)]
pub fn ErrorInsufficientStorage<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::INSUFFICIENT_STORAGE).into()
}

/// Helper function that creates wrapper of any error and
/// generate *LOOP DETECTED* response.
#[allow(non_snake_case)]
pub fn ErrorLoopDetected<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::LOOP_DETECTED).into()
}

/// Helper function that creates wrapper of any error and
/// generate *NOT EXTENDED* response.
#[allow(non_snake_case)]
pub fn ErrorNotExtended<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NOT_EXTENDED).into()
}

/// Helper function that creates wrapper of any error and
/// generate *NETWORK AUTHENTICATION REQUIRED* response.
#[allow(non_snake_case)]
pub fn ErrorNetworkAuthenticationRequired<T>(err: T) -> Error
where
    T: fmt::Debug + fmt::Display + 'static,
{
    InternalError::new(err, StatusCode::NETWORK_AUTHENTICATION_REQUIRED).into()
}

#[cfg(feature = "failure")]
/// Compatibility for `failure::Error`
impl ResponseError for fail_ure::Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Error as HttpError, StatusCode};
    use httparse;
    use std::io;

    #[test]
    fn test_into_response() {
        let resp: Response = ParseError::Incomplete.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err: HttpError = StatusCode::from_u16(10000).err().unwrap().into();
        let resp: Response = err.error_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "cookie")]
    #[test]
    fn test_cookie_parse() {
        let resp: Response = CookieParseError::EmptyName.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_as_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let e: Error = ParseError::Io(orig).into();
        assert_eq!(format!("{}", e.as_response_error()), "IO error: other");
    }

    #[test]
    fn test_error_cause() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.to_string();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e.as_response_error()), desc);
    }

    #[test]
    fn test_error_display() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.to_string();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e), desc);
    }

    #[test]
    fn test_error_http_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let e = Error::from(orig);
        let resp: Response = e.into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_payload_error() {
        let err: PayloadError =
            io::Error::new(io::ErrorKind::Other, "ParseError").into();
        assert!(format!("{}", err).contains("ParseError"));

        let err = PayloadError::Incomplete(None);
        assert_eq!(
            format!("{}", err),
            "A payload reached EOF, but is not complete. With error: None"
        );
    }

    macro_rules! from {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    assert!(format!("{}", e).len() >= 5);
                }
                e => unreachable!("{:?}", e),
            }
        };
    }

    macro_rules! from_and_cause {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    let desc = format!("{}", e);
                    assert_eq!(desc, format!("IO error: {}", $from));
                }
                _ => unreachable!("{:?}", $from),
            }
        };
    }

    #[test]
    fn test_from() {
        from_and_cause!(io::Error::new(io::ErrorKind::Other, "other") => ParseError::Io(..));
        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderValue => ParseError::Header);
        from!(httparse::Error::NewLine => ParseError::Header);
        from!(httparse::Error::Status => ParseError::Status);
        from!(httparse::Error::Token => ParseError::Header);
        from!(httparse::Error::TooManyHeaders => ParseError::TooLarge);
        from!(httparse::Error::Version => ParseError::Version);
    }

    #[test]
    fn test_internal_error() {
        let err =
            InternalError::from_response(ParseError::Method, Response::Ok().into());
        let resp: Response = err.error_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_error_casting() {
        let err = PayloadError::Overflow;
        let resp_err: &dyn ResponseError = &err;
        let err = resp_err.downcast_ref::<PayloadError>().unwrap();
        assert_eq!(err.to_string(), "A payload reached size limit.");
        let not_err = resp_err.downcast_ref::<ContentTypeError>();
        assert!(not_err.is_none());
    }

    #[test]
    fn test_error_helpers() {
        let r: Response = ErrorBadRequest("err").into();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);

        let r: Response = ErrorUnauthorized("err").into();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        let r: Response = ErrorPaymentRequired("err").into();
        assert_eq!(r.status(), StatusCode::PAYMENT_REQUIRED);

        let r: Response = ErrorForbidden("err").into();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        let r: Response = ErrorNotFound("err").into();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);

        let r: Response = ErrorMethodNotAllowed("err").into();
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);

        let r: Response = ErrorNotAcceptable("err").into();
        assert_eq!(r.status(), StatusCode::NOT_ACCEPTABLE);

        let r: Response = ErrorProxyAuthenticationRequired("err").into();
        assert_eq!(r.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);

        let r: Response = ErrorRequestTimeout("err").into();
        assert_eq!(r.status(), StatusCode::REQUEST_TIMEOUT);

        let r: Response = ErrorConflict("err").into();
        assert_eq!(r.status(), StatusCode::CONFLICT);

        let r: Response = ErrorGone("err").into();
        assert_eq!(r.status(), StatusCode::GONE);

        let r: Response = ErrorLengthRequired("err").into();
        assert_eq!(r.status(), StatusCode::LENGTH_REQUIRED);

        let r: Response = ErrorPreconditionFailed("err").into();
        assert_eq!(r.status(), StatusCode::PRECONDITION_FAILED);

        let r: Response = ErrorPayloadTooLarge("err").into();
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let r: Response = ErrorUriTooLong("err").into();
        assert_eq!(r.status(), StatusCode::URI_TOO_LONG);

        let r: Response = ErrorUnsupportedMediaType("err").into();
        assert_eq!(r.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let r: Response = ErrorRangeNotSatisfiable("err").into();
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let r: Response = ErrorExpectationFailed("err").into();
        assert_eq!(r.status(), StatusCode::EXPECTATION_FAILED);

        let r: Response = ErrorImATeapot("err").into();
        assert_eq!(r.status(), StatusCode::IM_A_TEAPOT);

        let r: Response = ErrorMisdirectedRequest("err").into();
        assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST);

        let r: Response = ErrorUnprocessableEntity("err").into();
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let r: Response = ErrorLocked("err").into();
        assert_eq!(r.status(), StatusCode::LOCKED);

        let r: Response = ErrorFailedDependency("err").into();
        assert_eq!(r.status(), StatusCode::FAILED_DEPENDENCY);

        let r: Response = ErrorUpgradeRequired("err").into();
        assert_eq!(r.status(), StatusCode::UPGRADE_REQUIRED);

        let r: Response = ErrorPreconditionRequired("err").into();
        assert_eq!(r.status(), StatusCode::PRECONDITION_REQUIRED);

        let r: Response = ErrorTooManyRequests("err").into();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);

        let r: Response = ErrorRequestHeaderFieldsTooLarge("err").into();
        assert_eq!(r.status(), StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);

        let r: Response = ErrorUnavailableForLegalReasons("err").into();
        assert_eq!(r.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);

        let r: Response = ErrorInternalServerError("err").into();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let r: Response = ErrorNotImplemented("err").into();
        assert_eq!(r.status(), StatusCode::NOT_IMPLEMENTED);

        let r: Response = ErrorBadGateway("err").into();
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);

        let r: Response = ErrorServiceUnavailable("err").into();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);

        let r: Response = ErrorGatewayTimeout("err").into();
        assert_eq!(r.status(), StatusCode::GATEWAY_TIMEOUT);

        let r: Response = ErrorHttpVersionNotSupported("err").into();
        assert_eq!(r.status(), StatusCode::HTTP_VERSION_NOT_SUPPORTED);

        let r: Response = ErrorVariantAlsoNegotiates("err").into();
        assert_eq!(r.status(), StatusCode::VARIANT_ALSO_NEGOTIATES);

        let r: Response = ErrorInsufficientStorage("err").into();
        assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);

        let r: Response = ErrorLoopDetected("err").into();
        assert_eq!(r.status(), StatusCode::LOOP_DETECTED);

        let r: Response = ErrorNotExtended("err").into();
        assert_eq!(r.status(), StatusCode::NOT_EXTENDED);

        let r: Response = ErrorNetworkAuthenticationRequired("err").into();
        assert_eq!(r.status(), StatusCode::NETWORK_AUTHENTICATION_REQUIRED);
    }
}
