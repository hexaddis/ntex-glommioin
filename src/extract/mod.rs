use std::rc::Rc;

use actix_http::error::Error;
use actix_http::Extensions;
use futures::future::ok;
use futures::{future, Async, Future, IntoFuture, Poll};

use crate::service::ServiceFromRequest;

mod form;
mod json;
mod path;
mod payload;
mod query;

pub use self::form::{Form, FormConfig};
pub use self::json::{Json, JsonConfig};
pub use self::path::Path;
pub use self::payload::{Payload, PayloadConfig};
pub use self::query::Query;

/// Trait implemented by types that can be extracted from request.
///
/// Types that implement this trait can be used with `Route` handlers.
pub trait FromRequest<P>: Sized {
    /// The associated error which can be returned.
    type Error: Into<Error>;

    /// Future that resolves to a Self
    type Future: IntoFuture<Item = Self, Error = Self::Error>;

    /// Configuration for the extractor
    type Config: ExtractorConfig;

    /// Convert request to a Self
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future;
}

/// Storage for extractor configs
#[derive(Default)]
pub struct ConfigStorage {
    pub(crate) storage: Option<Rc<Extensions>>,
}

impl ConfigStorage {
    pub fn store<C: ExtractorConfig>(&mut self, config: C) {
        if self.storage.is_none() {
            self.storage = Some(Rc::new(Extensions::new()));
        }
        if let Some(ref mut ext) = self.storage {
            Rc::get_mut(ext).unwrap().insert(config);
        }
    }
}

pub trait ExtractorConfig: Default + Clone + 'static {
    /// Set default configuration to config storage
    fn store_default(ext: &mut ConfigStorage) {
        ext.store(Self::default())
    }
}

impl ExtractorConfig for () {
    fn store_default(_: &mut ConfigStorage) {}
}

/// Optionally extract a field from the request
///
/// If the FromRequest for T fails, return None rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Error, FromRequest, ServiceFromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl<P> FromRequest<P> for Thing {
///     type Error = Error;
///     type Future = Result<Self, Self::Error>;
///     type Config = ();
///
///     fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///
///     }
/// }
///
/// /// extract `Thing` from request
/// fn index(supplied_thing: Option<Thing>) -> String {
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
impl<T: 'static, P> FromRequest<P> for Option<T>
where
    T: FromRequest<P>,
    T::Future: 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Option<T>, Error = Error>>;
    type Config = T::Config;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|r| match r {
            Ok(v) => future::ok(Some(v)),
            Err(e) => {
                log::debug!("Error for Option<T> extractor: {}", e.into());
                future::ok(None)
            }
        }))
    }
}

/// Optionally extract a field from the request or extract the Error if unsuccessful
///
/// If the `FromRequest` for T fails, inject Err into handler rather than returning an error response
///
/// ## Example
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Result, Error, FromRequest, ServiceFromRequest};
/// use actix_web::error::ErrorBadRequest;
/// use rand;
///
/// #[derive(Debug, Deserialize)]
/// struct Thing {
///     name: String
/// }
///
/// impl<P> FromRequest<P> for Thing {
///     type Error = Error;
///     type Future = Result<Thing, Error>;
///     type Config = ();
///
///     fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
///         if rand::random() {
///             Ok(Thing { name: "thingy".into() })
///         } else {
///             Err(ErrorBadRequest("no luck"))
///         }
///     }
/// }
///
/// /// extract `Thing` from request
/// fn index(supplied_thing: Result<Thing>) -> String {
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
impl<T: 'static, P> FromRequest<P> for Result<T, T::Error>
where
    T: FromRequest<P>,
    T::Future: 'static,
    T::Error: 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Result<T, T::Error>, Error = Error>>;
    type Config = T::Config;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Box::new(T::from_request(req).into_future().then(|res| match res {
            Ok(v) => ok(Ok(v)),
            Err(e) => ok(Err(e)),
        }))
    }
}

#[doc(hidden)]
impl<P> FromRequest<P> for () {
    type Error = Error;
    type Future = Result<(), Error>;
    type Config = ();

    fn from_request(_req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(())
    }
}

macro_rules! tuple_config ({ $($T:ident),+} => {
    impl<$($T,)+> ExtractorConfig for ($($T,)+)
    where $($T: ExtractorConfig + Clone,)+
    {
        fn store_default(ext: &mut ConfigStorage) {
            $($T::store_default(ext);)+
        }
    }
});

macro_rules! tuple_from_req ({$fut_type:ident, $(($n:tt, $T:ident)),+} => {

    /// FromRequest implementation for tuple
    #[doc(hidden)]
    impl<P, $($T: FromRequest<P> + 'static),+> FromRequest<P> for ($($T,)+)
    {
        type Error = Error;
        type Future = $fut_type<P, $($T),+>;
        type Config = ($($T::Config,)+);

        fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
            $fut_type {
                items: <($(Option<$T>,)+)>::default(),
                futs: ($($T::from_request(req).into_future(),)+),
            }
        }
    }

    #[doc(hidden)]
    pub struct $fut_type<P, $($T: FromRequest<P>),+> {
        items: ($(Option<$T>,)+),
        futs: ($(<$T::Future as futures::IntoFuture>::Future,)+),
    }

    impl<P, $($T: FromRequest<P>),+> Future for $fut_type<P, $($T),+>
    {
        type Item = ($($T,)+);
        type Error = Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let mut ready = true;

            $(
                if self.items.$n.is_none() {
                    match self.futs.$n.poll() {
                        Ok(Async::Ready(item)) => {
                            self.items.$n = Some(item);
                        }
                        Ok(Async::NotReady) => ready = false,
                        Err(e) => return Err(e.into()),
                    }
                }
            )+

                if ready {
                    Ok(Async::Ready(
                        ($(self.items.$n.take().unwrap(),)+)
                    ))
                } else {
                    Ok(Async::NotReady)
                }
        }
    }
});

#[rustfmt::skip]
mod m {
    use super::*;

tuple_config!(A);
tuple_config!(A, B);
tuple_config!(A, B, C);
tuple_config!(A, B, C, D);
tuple_config!(A, B, C, D, E);
tuple_config!(A, B, C, D, E, F);
tuple_config!(A, B, C, D, E, F, G);
tuple_config!(A, B, C, D, E, F, G, H);
tuple_config!(A, B, C, D, E, F, G, H, I);
tuple_config!(A, B, C, D, E, F, G, H, I, J);

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

#[cfg(test)]
mod tests {
    use actix_http::http::header;
    use actix_router::ResourceDef;
    use bytes::Bytes;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::{block_on, TestRequest};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_bytes() {
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = block_on(Bytes::from_request(&mut req)).unwrap();
        assert_eq!(s, Bytes::from_static(b"hello=world"));
    }

    #[test]
    fn test_string() {
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = block_on(String::from_request(&mut req)).unwrap();
        assert_eq!(s, "hello=world");
    }

    #[test]
    fn test_form() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let s = block_on(Form::<Info>::from_request(&mut req)).unwrap();
        assert_eq!(s.hello, "world");
    }

    #[test]
    fn test_option() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .config(FormConfig::default().limit(4096))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(r, None);

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(
            r,
            Some(Form(Info {
                hello: "world".into()
            }))
        );

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_from();

        let r = block_on(Option::<Form<Info>>::from_request(&mut req)).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn test_result() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let r = block_on(Result::<Form<Info>, Error>::from_request(&mut req))
            .unwrap()
            .unwrap();
        assert_eq!(
            r,
            Form(Info {
                hello: "world".into()
            })
        );

        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "9")
        .set_payload(Bytes::from_static(b"bye=world"))
        .to_from();

        let r = block_on(Result::<Form<Info>, Error>::from_request(&mut req)).unwrap();
        assert!(r.is_err());
    }

    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Deserialize)]
    struct Id {
        id: String,
    }

    #[derive(Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[test]
    fn test_request_extract() {
        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_from();

        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let s = Path::<MyStruct>::from_request(&mut req).unwrap();
        assert_eq!(s.key, "name");
        assert_eq!(s.value, "user1");

        let s = Path::<(String, String)>::from_request(&mut req).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, "user1");

        let s = Query::<Id>::from_request(&mut req).unwrap();
        assert_eq!(s.id, "test");

        let mut req = TestRequest::with_uri("/name/32/").to_from();
        let resource = ResourceDef::new("/{key}/{value}/");
        resource.match_path(req.match_info_mut());

        let s = Path::<Test2>::from_request(&mut req).unwrap();
        assert_eq!(s.as_ref().key, "name");
        assert_eq!(s.value, 32);

        let s = Path::<(String, u8)>::from_request(&mut req).unwrap();
        assert_eq!(s.0, "name");
        assert_eq!(s.1, 32);

        let res = Path::<Vec<String>>::from_request(&mut req).unwrap();
        assert_eq!(res[0], "name".to_owned());
        assert_eq!(res[1], "32".to_owned());
    }

    #[test]
    fn test_extract_path_single() {
        let resource = ResourceDef::new("/{value}/");

        let mut req = TestRequest::with_uri("/32/").to_from();
        resource.match_path(req.match_info_mut());

        assert_eq!(*Path::<i8>::from_request(&mut req).unwrap(), 32);
    }

    #[test]
    fn test_tuple_extract() {
        let resource = ResourceDef::new("/{key}/{value}/");

        let mut req = TestRequest::with_uri("/name/user1/?id=test").to_from();
        resource.match_path(req.match_info_mut());

        let res = block_on(<(Path<(String, String)>,)>::from_request(&mut req)).unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");

        let res = block_on(
            <(Path<(String, String)>, Path<(String, String)>)>::from_request(&mut req),
        )
        .unwrap();
        assert_eq!((res.0).0, "name");
        assert_eq!((res.0).1, "user1");
        assert_eq!((res.1).0, "name");
        assert_eq!((res.1).1, "user1");

        let () = <()>::from_request(&mut req).unwrap();
    }
}
