//! Payload/Bytes/String extractors
use std::future::Future;
use std::pin::Pin;
use std::str;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use encoding_rs::UTF_8;
use futures::future::{err, ok, Either, FutureExt, LocalBoxFuture, Ready};
use futures::{Stream, StreamExt};
use mime::Mime;

use crate::http::{error, header, HttpMessage};
use crate::web::error::{ErrorRenderer, PayloadError};
use crate::web::extract::FromRequest;
use crate::web::request::HttpRequest;

/// Payload extractor returns request 's payload stream.
///
/// ## Example
///
/// ```rust
/// use bytes::BytesMut;
/// use futures::{Future, Stream, StreamExt};
/// use ntex::web::{self, error, App, HttpResponse};
///
/// /// extract binary data from request
/// async fn index(mut body: web::types::Payload) -> Result<HttpResponse, error::PayloadError>
/// {
///     let mut bytes = BytesMut::new();
///     while let Some(item) = body.next().await {
///         bytes.extend_from_slice(&item?);
///     }
///
///     format!("Body {:?}!", bytes);
///     Ok(HttpResponse::Ok().finish())
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get().to(index))
///     );
/// }
/// ```
pub struct Payload(pub crate::http::Payload);

impl Payload {
    /// Deconstruct to a inner value
    pub fn into_inner(self) -> crate::http::Payload {
        self.0
    }
}

impl Stream for Payload {
    type Item = Result<Bytes, error::PayloadError>;

    #[inline]
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}

/// Get request's payload stream
///
/// ## Example
///
/// ```rust
/// use bytes::BytesMut;
/// use futures::{Future, Stream, StreamExt};
/// use ntex::web::{self, error, App, Error, HttpResponse};
///
/// /// extract binary data from request
/// async fn index(mut body: web::types::Payload) -> Result<HttpResponse, error::PayloadError>
/// {
///     let mut bytes = BytesMut::new();
///     while let Some(item) = body.next().await {
///         bytes.extend_from_slice(&item?);
///     }
///
///     format!("Body {:?}!", bytes);
///     Ok(HttpResponse::Ok().finish())
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get().to(index))
///     );
/// }
/// ```
impl<Err: ErrorRenderer> FromRequest<Err> for Payload {
    type Error = Err::Container;
    type Future = Ready<Result<Payload, Self::Error>>;

    #[inline]
    fn from_request(
        _: &HttpRequest,
        payload: &mut crate::http::Payload,
    ) -> Self::Future {
        ok(Payload(payload.take()))
    }
}

/// Request binary data from a request's payload.
///
/// Loads request's payload and construct Bytes instance.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use bytes::Bytes;
/// use ntex::web;
///
/// /// extract binary data from request
/// async fn index(body: Bytes) -> String {
///     format!("Body {:?}!", body)
/// }
///
/// fn main() {
///     let app = web::App::new().service(
///         web::resource("/index.html").route(
///             web::get().to(index))
///     );
/// }
/// ```
impl<Err: ErrorRenderer> FromRequest<Err> for Bytes {
    type Error = PayloadError;
    type Future = Either<
        LocalBoxFuture<'static, Result<Bytes, Self::Error>>,
        Ready<Result<Bytes, Self::Error>>,
    >;

    #[inline]
    fn from_request(
        req: &HttpRequest,
        payload: &mut crate::http::Payload,
    ) -> Self::Future {
        let tmp;
        let cfg = if let Some(cfg) = req.app_data::<PayloadConfig>() {
            cfg
        } else {
            tmp = PayloadConfig::default();
            &tmp
        };

        if let Err(e) = cfg.check_mimetype(req) {
            return Either::Right(err(e));
        }

        let limit = cfg.limit;
        let fut = HttpMessageBody::new(req, payload).limit(limit);
        Either::Left(async move { Ok(fut.await?) }.boxed_local())
    }
}

/// Extract text information from a request's body.
///
/// Text extractor automatically decode body according to the request's charset.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use ntex::web::{self, App, FromRequest};
///
/// /// extract text data from request
/// async fn index(text: String) -> String {
///     format!("Body {}!", text)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html")
///             .app_data(
///                 web::types::PayloadConfig::new(4096)  // <- limit size of the payload
///             )
///             .route(web::get().to(index))  // <- register handler with extractor params
///     );
/// }
/// ```
impl<Err: ErrorRenderer> FromRequest<Err> for String {
    type Error = PayloadError;
    type Future = Either<
        LocalBoxFuture<'static, Result<String, Self::Error>>,
        Ready<Result<String, Self::Error>>,
    >;

    #[inline]
    fn from_request(
        req: &HttpRequest,
        payload: &mut crate::http::Payload,
    ) -> Self::Future {
        let tmp;
        let cfg = if let Some(cfg) = req.app_data::<PayloadConfig>() {
            cfg
        } else {
            tmp = PayloadConfig::default();
            &tmp
        };

        // check content-type
        if let Err(e) = cfg.check_mimetype(req) {
            return Either::Right(err(e));
        }

        // check charset
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(e) => return Either::Right(err(PayloadError::from(e))),
        };
        let limit = cfg.limit;
        let fut = HttpMessageBody::new(req, payload).limit(limit);

        Either::Left(
            async move {
                let body = fut.await?;

                if encoding == UTF_8 {
                    Ok(str::from_utf8(body.as_ref())
                        .map_err(|_| PayloadError::Decoding)?
                        .to_owned())
                } else {
                    Ok(encoding
                        .decode_without_bom_handling_and_without_replacement(&body)
                        .map(|s| s.into_owned())
                        .ok_or_else(|| PayloadError::Decoding)?)
                }
            }
            .boxed_local(),
        )
    }
}
/// Payload configuration for request's payload.
#[derive(Clone)]
pub struct PayloadConfig {
    limit: usize,
    mimetype: Option<Mime>,
}

impl PayloadConfig {
    /// Create `PayloadConfig` instance and set max size of payload.
    pub fn new(limit: usize) -> Self {
        let mut cfg = Self::default();
        cfg.limit = limit;
        cfg
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set required mime-type of the request. By default mime type is not
    /// enforced.
    pub fn mimetype(mut self, mt: Mime) -> Self {
        self.mimetype = Some(mt);
        self
    }

    fn check_mimetype(&self, req: &HttpRequest) -> Result<(), PayloadError> {
        // check content-type
        if let Some(ref mt) = self.mimetype {
            match req.mime_type() {
                Ok(Some(ref req_mt)) => {
                    if mt != req_mt {
                        return Err(PayloadError::from(
                            error::ContentTypeError::Unexpected,
                        ));
                    }
                }
                Ok(None) => {
                    return Err(PayloadError::from(error::ContentTypeError::Expected));
                }
                Err(err) => {
                    return Err(err.into());
                }
            }
        }
        Ok(())
    }
}

impl Default for PayloadConfig {
    fn default() -> Self {
        PayloadConfig {
            limit: 262_144,
            mimetype: None,
        }
    }
}

/// Future that resolves to a complete http message body.
///
/// Load http message body.
///
/// By default only 256Kb payload reads to a memory, then
/// `PayloadError::Overflow` get returned. Use `MessageBody::limit()`
/// method to change upper limit.
struct HttpMessageBody {
    limit: usize,
    length: Option<usize>,
    #[cfg(feature = "compress")]
    stream: Option<crate::http::encoding::Decoder<crate::http::Payload>>,
    #[cfg(not(feature = "compress"))]
    stream: Option<crate::http::Payload>,
    err: Option<PayloadError>,
    fut: Option<LocalBoxFuture<'static, Result<Bytes, PayloadError>>>,
}

impl HttpMessageBody {
    /// Create `MessageBody` for request.
    fn new(req: &HttpRequest, payload: &mut crate::http::Payload) -> HttpMessageBody {
        let mut len = None;
        if let Some(l) = req.headers().get(&header::CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(PayloadError::Payload(
                        error::PayloadError::UnknownLength,
                    ));
                }
            } else {
                return Self::err(PayloadError::Payload(
                    error::PayloadError::UnknownLength,
                ));
            }
        }

        #[cfg(feature = "compress")]
        let stream = Some(crate::http::encoding::Decoder::from_headers(
            payload.take(),
            req.headers(),
        ));
        #[cfg(not(feature = "compress"))]
        let stream = Some(payload.take());

        HttpMessageBody {
            stream,
            limit: 262_144,
            length: len,
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(e: PayloadError) -> Self {
        HttpMessageBody {
            stream: None,
            limit: 262_144,
            fut: None,
            err: Some(e),
            length: None,
        }
    }
}

impl Future for HttpMessageBody {
    type Output = Result<Bytes, PayloadError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(ref mut fut) = self.fut {
            return Pin::new(fut).poll(cx);
        }

        if let Some(err) = self.err.take() {
            return Poll::Ready(Err(err));
        }

        if let Some(len) = self.length.take() {
            if len > self.limit {
                return Poll::Ready(Err(PayloadError::from(
                    error::PayloadError::Overflow,
                )));
            }
        }

        // future
        let limit = self.limit;
        let mut stream = self.stream.take().unwrap();
        self.fut = Some(
            async move {
                let mut body = BytesMut::with_capacity(8192);

                while let Some(item) = stream.next().await {
                    let chunk = item?;
                    if body.len() + chunk.len() > limit {
                        return Err(PayloadError::from(error::PayloadError::Overflow));
                    } else {
                        body.extend_from_slice(&chunk);
                    }
                }
                Ok(body.freeze())
            }
            .boxed_local(),
        );
        self.poll(cx)
    }
}

// #[cfg(test)]
// mod tests {
//     use bytes::Bytes;

//     use super::*;
//     use crate::http::header;
//     use crate::web::test::TestRequest;

//     #[ntex_rt::test]
//     async fn test_payload_config() {
//         let req = TestRequest::default().to_http_request();
//         let cfg = PayloadConfig::default().mimetype(mime::APPLICATION_JSON);
//         assert!(cfg.check_mimetype(&req).is_err());

//         let req = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .to_http_request();
//         assert!(cfg.check_mimetype(&req).is_err());

//         let req = TestRequest::with_header(header::CONTENT_TYPE, "application/json")
//             .to_http_request();
//         assert!(cfg.check_mimetype(&req).is_ok());
//     }

//     #[ntex_rt::test]
//     async fn test_bytes() {
//         let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "11")
//             .set_payload(Bytes::from_static(b"hello=world"))
//             .to_http_parts();

//         let s = Bytes::from_request(&req, &mut pl).await.unwrap();
//         assert_eq!(s, Bytes::from_static(b"hello=world"));
//     }

//     #[ntex_rt::test]
//     async fn test_string() {
//         let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "11")
//             .set_payload(Bytes::from_static(b"hello=world"))
//             .to_http_parts();

//         let s = String::from_request(&req, &mut pl).await.unwrap();
//         assert_eq!(s, "hello=world");
//     }

//     #[ntex_rt::test]
//     async fn test_message_body() {
//         let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "xxxx")
//             .to_srv_request()
//             .into_parts();
//         let res = HttpMessageBody::new(&req, &mut pl).await;
//         match res.err().unwrap() {
//             PayloadError::Payload(error::PayloadError::UnknownLength) => (),
//             _ => unreachable!("error"),
//         }

//         let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "1000000")
//             .to_srv_request()
//             .into_parts();
//         let res = HttpMessageBody::new(&req, &mut pl).await;
//         match res.err().unwrap() {
//             PayloadError::Payload(error::PayloadError::Overflow) => (),
//             _ => unreachable!("error"),
//         }

//         let (req, mut pl) = TestRequest::default()
//             .set_payload(Bytes::from_static(b"test"))
//             .to_http_parts();
//         let res = HttpMessageBody::new(&req, &mut pl).await;
//         assert_eq!(res.ok().unwrap(), Bytes::from_static(b"test"));

//         let (req, mut pl) = TestRequest::default()
//             .set_payload(Bytes::from_static(b"11111111111111"))
//             .to_http_parts();
//         let res = HttpMessageBody::new(&req, &mut pl).limit(5).await;
//         match res.err().unwrap() {
//             PayloadError::Payload(error::PayloadError::Overflow) => (),
//             _ => unreachable!("error"),
//         }
//     }
// }