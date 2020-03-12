//! Request extractors
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};

use crate::http::Payload;

use super::error::{DefaultError, IntoWebError, WebError, WebResponseError};
use super::request::HttpRequest;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<Err = DefaultError>: Sized {
    /// The associated error which can be returned.
    type Error: WebResponseError<Err>;

    /// Future that resolves to a Self
    type Future: Future<Output = Result<Self, Self::Error>>;

    /// Configuration for this extractor
    type Config: Default + 'static;

    /// Convert request to a Self
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future;

    /// Convert request to a Self
    ///
    /// This method uses `Payload::None` as payload stream.
    fn extract(req: &HttpRequest) -> Self::Future {
        Self::from_request(req, &mut Payload::None)
    }
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// use ntex::http;
/// use ntex::web::{self, error, dev, App, HttpRequest, FromRequest, DefaultError, WebError};
/// use futures::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest<DefaultError> for Thing {
///     type Error = WebError<DefaultError>;
///     type Future = Ready<Result<Self, Self::Error>>;
///     type Config = ();
///
///     fn from_request(req: &HttpRequest, payload: &mut http::Payload) -> Self::Future {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(error::ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Option<Thing>) -> String {
///     match supplied_thing {
///         // Puns not intended
///         Some(thing) => format!("Got something: {:?}", thing),
///         None => format!("No thing!")
///     }
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(
///             web::post().to(index))
///     );
/// }
/// ```
impl<T: 'static, Err: 'static> FromRequest<Err> for Option<T>
where
    T: FromRequest<Err>,
    T::Future: 'static,
{
    type Config = T::Config;
    type Error = WebError<Err>;
    type Future = LocalBoxFuture<'static, Result<Option<T>, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        T::from_request(req, payload)
            .then(|r| match r {
                Ok(v) => ok(Some(v)),
                Err(e) => {
                    log::debug!("Error for Option<T> extractor: {}", e.into_error());
                    ok(None)
                }
            })
            .boxed_local()
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the `FromRequest` for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// use ntex::http;
/// use ntex::web::{self, error, App, HttpRequest, FromRequest, WebError, DefaultError};
/// use futures::future::{ok, err, Ready};
/// use serde_derive::Deserialize;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl FromRequest<DefaultError> for Thing {
///     type Error = WebError<DefaultError>;
///     type Future = Ready<Result<Thing, Self::Error>>;
///     type Config = ();
///
///     fn from_request(req: &HttpRequest, payload: &mut http::Payload) -> Self::Future {
///         if rand::random() {
///             ok(Thing { name: "thingy".into() })
///         } else {
///             err(error::ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract `Thing` from request
/// async fn index(supplied_thing: Result<Thing, WebError<DefaultError>>) -> String {
///     match supplied_thing {
///         Ok(thing) => format!("Got thing: {:?}", thing),
///         Err(e) => format!("Error extracting thing: {}", e)
///     }
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/users/:first").route(web::post().to(index))
///     );
/// }
/// ```
impl<T, E> FromRequest<E> for Result<T, T::Error>
where
    T: FromRequest<E> + 'static,
    T::Error: 'static,
    T::Future: 'static,
    E: 'static,
{
    type Config = T::Config;
    type Error = T::Error;
    type Future = LocalBoxFuture<'static, Result<Result<T, T::Error>, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        T::from_request(req, payload)
            .then(|res| match res {
                Ok(v) => ok(Ok(v)),
                Err(e) => ok(Err(e)),
            })
            .boxed_local()
    }
}

#[doc(hidden)]
impl<E: 'static> FromRequest<E> for () {
    type Config = ();
    type Error = WebError<E>;
    type Future = Ready<Result<(), WebError<E>>>;

    fn from_request(_: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(())
    }
}

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    #[doc(hidden)]
    #[allow(unused_parens)]
    impl<Err: 'static, $($T: FromRequest<Err> + 'static),+> FromRequest<Err> for ($($T,)+)
    {
        type Error = WebError<Err>;
        type Future = $fut_type<Err, $($T),+>;
        type Config = ($($T::Config),+);

        fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
            $fut_type {
                items: <($(Option<$T>,)+)>::default(),
                futs: ($($T::from_request(req, payload),)+),
            }
        }
    }

    #[doc(hidden)]
    #[pin_project::pin_project]
    pub struct $fut_type<Err: 'static, $($T: FromRequest<Err>),+> {
        items: ($(Option<$T>,)+),
        futs: ($($T::Future,)+),
    }

    impl<Err: 'static, $($T: FromRequest<Err>),+> Future for $fut_type<Err, $($T),+>
    {
        type Output = Result<($($T,)+), WebError<Err>>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();

            let mut ready = true;
            $(
                if this.items.$n.is_none() {
                    match unsafe { Pin::new_unchecked(&mut this.futs.$n) }.poll(cx) {
                        Poll::Ready(Ok(item)) => {
                            this.items.$n = Some(item);
                        }
                        Poll::Pending => ready = false,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e.into_error())),
                    }
                }
            )+

                if ready {
                    Poll::Ready(Ok(
                        ($(this.items.$n.take().unwrap(),)+)
                    ))
                } else {
                    Poll::Pending
                }
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

tuple_from_req!(TupleFromRequest1, (0, A));
tuple_from_req!(TupleFromRequest2, (0, A), (1, B));
tuple_from_req!(TupleFromRequest3, (0, A), (1, B), (2, C));
tuple_from_req!(TupleFromRequest4, (0, A), (1, B), (2, C), (3, D));
tuple_from_req!(TupleFromRequest5, (0, A), (1, B), (2, C), (3, D), (4, E));
tuple_from_req!(TupleFromRequest6, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
tuple_from_req!(TupleFromRequest7, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
tuple_from_req!(TupleFromRequest8, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H));
tuple_from_req!(TupleFromRequest9, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I));
tuple_from_req!(TupleFromRequest10, (0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G), (7, H), (8, I), (9, J));
}

// #[cfg(test)]
// mod tests {
//     use bytes::Bytes;
//     use serde_derive::Deserialize;

//     use super::*;
//     use crate::http::header;
//     use crate::web::error::DefaultError;
//     use crate::web::test::TestRequest;
//     use crate::web::types::{Form, FormConfig};

//     #[derive(Deserialize, Debug, PartialEq)]
//     struct Info {
//         hello: String,
//     }

//     fn extract<T: FromRequest<DefaultError>>(
//         extract: T,
//     ) -> impl FromRequest<DefaultError, Error = T::Error> {
//         extract
//     }

//     #[actix_rt::test]
//     async fn test_option() {
//         let (req, mut pl) = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .data(FormConfig::default().limit(4096))
//         .to_http_parts();

//         let r = Option::<Form<Info>>::from_request(&req, &mut pl)
//             .await
//             .unwrap();
//         assert_eq!(r, None);

//         let (req, mut pl) = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .header(header::CONTENT_LENGTH, "9")
//         .set_payload(Bytes::from_static(b"hello=world"))
//         .to_http_parts();

//         let r = Option::<Form<Info>>::from_request(&req, &mut pl)
//             .await
//             .unwrap();
//         assert_eq!(
//             r,
//             Some(Form(Info {
//                 hello: "world".into()
//             }))
//         );

//         let (req, mut pl) = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .header(header::CONTENT_LENGTH, "9")
//         .set_payload(Bytes::from_static(b"bye=world"))
//         .to_http_parts();

//         let r = Option::<Form<Info>>::from_request(&req, &mut pl)
//             .await
//             .unwrap();
//         assert_eq!(r, None);
//     }

//     #[actix_rt::test]
//     async fn test_result() {
//         let (req, mut pl) = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .header(header::CONTENT_LENGTH, "11")
//         .set_payload(Bytes::from_static(b"hello=world"))
//         .to_http_parts();

//         let r: Result<Form<Info>, WebError> = FromRequest::<DefaultError>::from_request(&req, &mut pl)
//             .await
//             .unwrap();
//         assert_eq!(
//             r.unwrap(),
//             Form(Info {
//                 hello: "world".into()
//             })
//         );

//         let (req, mut pl) = TestRequest::with_header(
//             header::CONTENT_TYPE,
//             "application/x-www-form-urlencoded",
//         )
//         .header(header::CONTENT_LENGTH, "9")
//         .set_payload(Bytes::from_static(b"bye=world"))
//         .to_http_parts();

//         let r: Result::<Form<Info>, WebError> = FromRequest::from_request(&req, &mut pl)
//             .await
//             .unwrap();
//         assert!(r.is_err());
//     }
// }
