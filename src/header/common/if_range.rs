use std::fmt::{self, Display, Write};
use error::ParseError;
use httpmessage::HttpMessage;
use header::{http, from_one_raw_str};
use header::{IntoHeaderValue, Header, EntityTag, HttpDate, Writer};

/// `If-Range` header, defined in [RFC7233](http://tools.ietf.org/html/rfc7233#section-3.2)
///
/// If a client has a partial copy of a representation and wishes to have
/// an up-to-date copy of the entire representation, it could use the
/// Range header field with a conditional GET (using either or both of
/// If-Unmodified-Since and If-Match.)  However, if the precondition
/// fails because the representation has been modified, the client would
/// then have to make a second request to obtain the entire current
/// representation.
///
/// The `If-Range` header field allows a client to \"short-circuit\" the
/// second request.  Informally, its meaning is as follows: if the
/// representation is unchanged, send me the part(s) that I am requesting
/// in Range; otherwise, send me the entire representation.
///
/// # ABNF
///
/// ```text
/// If-Range = entity-tag / HTTP-date
/// ```
///
/// # Example values
///
/// * `Sat, 29 Oct 1994 19:43:31 GMT`
/// * `\"xyzzy\"`
///
/// # Examples
///
/// ```rust
/// use actix_web::httpcodes;
/// use actix_web::header::{IfRange, EntityTag};
///
/// let mut builder = httpcodes::HttpOk.build();
/// builder.set(IfRange::EntityTag(EntityTag::new(false, "xyzzy".to_owned())));
/// ```
///
/// ```rust
/// use actix_web::httpcodes;
/// use actix_web::header::IfRange;
/// use std::time::{SystemTime, Duration};
///
/// let mut builder = httpcodes::HttpOk.build();
/// let fetched = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
/// builder.set(IfRange::Date(fetched.into()));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum IfRange {
    /// The entity-tag the client has of the resource
    EntityTag(EntityTag),
    /// The date when the client retrieved the resource
    Date(HttpDate),
}

impl Header for IfRange {
    fn name() -> http::HeaderName {
        http::IF_RANGE
    }
    #[inline]
    fn parse<T>(msg: &T) -> Result<Self, ParseError> where T: HttpMessage
    {
        let etag: Result<EntityTag, _> = from_one_raw_str(msg.headers().get(http::IF_RANGE));
        if let Ok(etag) = etag {
            return Ok(IfRange::EntityTag(etag));
        }
        let date: Result<HttpDate, _> = from_one_raw_str(msg.headers().get(http::IF_RANGE));
        if let Ok(date) = date {
            return Ok(IfRange::Date(date));
        }
        Err(ParseError::Header)
    }
}

impl Display for IfRange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            IfRange::EntityTag(ref x) => Display::fmt(x, f),
            IfRange::Date(ref x) => Display::fmt(x, f),
        }
    }
}

impl IntoHeaderValue for IfRange {
    type Error = http::InvalidHeaderValueBytes;

    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        let mut writer = Writer::new();
        let _ = write!(&mut writer, "{}", self);
        http::HeaderValue::from_shared(writer.take())
    }
}


#[cfg(test)]
mod test_if_range {
    use std::str;
    use header::*;
    use super::IfRange as HeaderField;
    test_header!(test1, vec![b"Sat, 29 Oct 1994 19:43:31 GMT"]);
    test_header!(test2, vec![b"\"xyzzy\""]);
    test_header!(test3, vec![b"this-is-invalid"], None::<IfRange>);
}
