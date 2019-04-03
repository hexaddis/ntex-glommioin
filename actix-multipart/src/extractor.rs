//! Multipart payload support
use bytes::Bytes;
use futures::Stream;

use actix_web::dev::ServiceFromRequest;
use actix_web::error::{Error, PayloadError};
use actix_web::FromRequest;
use actix_web::HttpMessage;

use crate::server::Multipart;

/// Get request's payload as multipart stream
///
/// Content-type: multipart/form-data;
///
/// ## Server example
///
/// ```rust
/// # use futures::{Future, Stream};
/// # use futures::future::{ok, result, Either};
/// use actix_web::{web, HttpResponse, Error};
/// use actix_multipart as mp;
///
/// fn index(payload: mp::Multipart) -> impl Future<Item = HttpResponse, Error = Error> {
///     payload.from_err()               // <- get multipart stream for current request
///        .and_then(|item| match item { // <- iterate over multipart items
///            mp::Item::Field(field) => {
///                // Field in turn is stream of *Bytes* object
///                Either::A(field.from_err()
///                          .fold((), |_, chunk| {
///                              println!("-- CHUNK: \n{:?}", std::str::from_utf8(&chunk));
///                              Ok::<_, Error>(())
///                          }))
///             },
///             mp::Item::Nested(mp) => {
///                 // Or item could be nested Multipart stream
///                 Either::B(ok(()))
///             }
///         })
///         .fold((), |_, _| Ok::<_, Error>(()))
///         .map(|_| HttpResponse::Ok().into())
/// }
/// # fn main() {}
/// ```
impl<P> FromRequest<P> for Multipart
where
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future = Result<Multipart, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let pl = req.take_payload();
        Ok(Multipart::new(req.headers(), pl))
    }
}
