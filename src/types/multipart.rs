//! Multipart payload support
use std::cell::{RefCell, UnsafeCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::{cmp, fmt};

use bytes::Bytes;
use futures::task::{current as current_task, Task};
use futures::{Async, Poll, Stream};
use httparse;
use mime;

use crate::error::{Error, MultipartError, ParseError, PayloadError};
use crate::extract::FromRequest;
use crate::http::header::{
    self, ContentDisposition, HeaderMap, HeaderName, HeaderValue,
};
use crate::http::HttpTryFrom;
use crate::service::ServiceFromRequest;
use crate::HttpMessage;

const MAX_HEADERS: usize = 32;

type PayloadBuffer =
    actix_http::h1::PayloadBuffer<Box<dyn Stream<Item = Bytes, Error = PayloadError>>>;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `MultipartItem` instances via its
/// Stream implementation.
/// `MultipartItem::Field` contains multipart field. `MultipartItem::Multipart`
/// is used for nested multipart streams.
pub struct Multipart {
    safety: Safety,
    error: Option<MultipartError>,
    inner: Option<Rc<RefCell<InnerMultipart>>>,
}

/// Multipart item
pub enum MultipartItem {
    /// Multipart field
    Field(MultipartField),
    /// Nested multipart stream
    Nested(Multipart),
}

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
///
/// fn index(payload: web::Multipart) -> impl Future<Item = HttpResponse, Error = Error> {
///     payload.from_err()               // <- get multipart stream for current request
///        .and_then(|item| match item { // <- iterate over multipart items
///            web::MultipartItem::Field(field) => {
///                // Field in turn is stream of *Bytes* object
///                Either::A(field.from_err()
///                          .fold((), |_, chunk| {
///                              println!("-- CHUNK: \n{:?}", std::str::from_utf8(&chunk));
///                              Ok::<_, Error>(())
///                          }))
///             },
///             web::MultipartItem::Nested(mp) => {
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

enum InnerMultipartItem {
    None,
    Field(Rc<RefCell<InnerField>>),
    Multipart(Rc<RefCell<InnerMultipart>>),
}

#[derive(PartialEq, Debug)]
enum InnerState {
    /// Stream eof
    Eof,
    /// Skip data until first boundary
    FirstBoundary,
    /// Reading boundary
    Boundary,
    /// Reading Headers,
    Headers,
}

struct InnerMultipart {
    payload: PayloadRef,
    boundary: String,
    state: InnerState,
    item: InnerMultipartItem,
}

impl Multipart {
    /// Create multipart instance for boundary.
    pub fn new<S>(headers: &HeaderMap, stream: S) -> Multipart
    where
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        match Self::boundary(headers) {
            Ok(boundary) => Multipart {
                error: None,
                safety: Safety::new(),
                inner: Some(Rc::new(RefCell::new(InnerMultipart {
                    boundary,
                    payload: PayloadRef::new(PayloadBuffer::new(Box::new(stream))),
                    state: InnerState::FirstBoundary,
                    item: InnerMultipartItem::None,
                }))),
            },
            Err(err) => Multipart {
                error: Some(err),
                safety: Safety::new(),
                inner: None,
            },
        }
    }

    /// Extract boundary info from headers.
    fn boundary(headers: &HeaderMap) -> Result<String, MultipartError> {
        if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                if let Ok(ct) = content_type.parse::<mime::Mime>() {
                    if let Some(boundary) = ct.get_param(mime::BOUNDARY) {
                        Ok(boundary.as_str().to_owned())
                    } else {
                        Err(MultipartError::Boundary)
                    }
                } else {
                    Err(MultipartError::ParseContentType)
                }
            } else {
                Err(MultipartError::ParseContentType)
            }
        } else {
            Err(MultipartError::NoContentType)
        }
    }
}

impl Stream for Multipart {
    type Item = MultipartItem;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(err) = self.error.take() {
            Err(err)
        } else if self.safety.current() {
            self.inner.as_mut().unwrap().borrow_mut().poll(&self.safety)
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl InnerMultipart {
    fn read_headers(payload: &mut PayloadBuffer) -> Poll<HeaderMap, MultipartError> {
        match payload.read_until(b"\r\n\r\n")? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(bytes)) => {
                let mut hdrs = [httparse::EMPTY_HEADER; MAX_HEADERS];
                match httparse::parse_headers(&bytes, &mut hdrs) {
                    Ok(httparse::Status::Complete((_, hdrs))) => {
                        // convert headers
                        let mut headers = HeaderMap::with_capacity(hdrs.len());
                        for h in hdrs {
                            if let Ok(name) = HeaderName::try_from(h.name) {
                                if let Ok(value) = HeaderValue::try_from(h.value) {
                                    headers.append(name, value);
                                } else {
                                    return Err(ParseError::Header.into());
                                }
                            } else {
                                return Err(ParseError::Header.into());
                            }
                        }
                        Ok(Async::Ready(headers))
                    }
                    Ok(httparse::Status::Partial) => Err(ParseError::Header.into()),
                    Err(err) => Err(ParseError::from(err).into()),
                }
            }
        }
    }

    fn read_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Poll<bool, MultipartError> {
        // TODO: need to read epilogue
        match payload.readline()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(chunk)) => {
                if chunk.len() == boundary.len() + 4
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                {
                    Ok(Async::Ready(false))
                } else if chunk.len() == boundary.len() + 6
                    && &chunk[..2] == b"--"
                    && &chunk[2..boundary.len() + 2] == boundary.as_bytes()
                    && &chunk[boundary.len() + 2..boundary.len() + 4] == b"--"
                {
                    Ok(Async::Ready(true))
                } else {
                    Err(MultipartError::Boundary)
                }
            }
        }
    }

    fn skip_until_boundary(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Poll<bool, MultipartError> {
        let mut eof = false;
        loop {
            match payload.readline()? {
                Async::Ready(Some(chunk)) => {
                    if chunk.is_empty() {
                        //ValueError("Could not find starting boundary %r"
                        //% (self._boundary))
                    }
                    if chunk.len() < boundary.len() {
                        continue;
                    }
                    if &chunk[..2] == b"--"
                        && &chunk[2..chunk.len() - 2] == boundary.as_bytes()
                    {
                        break;
                    } else {
                        if chunk.len() < boundary.len() + 2 {
                            continue;
                        }
                        let b: &[u8] = boundary.as_ref();
                        if &chunk[..boundary.len()] == b
                            && &chunk[boundary.len()..boundary.len() + 2] == b"--"
                        {
                            eof = true;
                            break;
                        }
                    }
                }
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(None) => return Err(MultipartError::Incomplete),
            }
        }
        Ok(Async::Ready(eof))
    }

    fn poll(&mut self, safety: &Safety) -> Poll<Option<MultipartItem>, MultipartError> {
        if self.state == InnerState::Eof {
            Ok(Async::Ready(None))
        } else {
            // release field
            loop {
                // Nested multipart streams of fields has to be consumed
                // before switching to next
                if safety.current() {
                    let stop = match self.item {
                        InnerMultipartItem::Field(ref mut field) => {
                            match field.borrow_mut().poll(safety)? {
                                Async::NotReady => return Ok(Async::NotReady),
                                Async::Ready(Some(_)) => continue,
                                Async::Ready(None) => true,
                            }
                        }
                        InnerMultipartItem::Multipart(ref mut multipart) => {
                            match multipart.borrow_mut().poll(safety)? {
                                Async::NotReady => return Ok(Async::NotReady),
                                Async::Ready(Some(_)) => continue,
                                Async::Ready(None) => true,
                            }
                        }
                        _ => false,
                    };
                    if stop {
                        self.item = InnerMultipartItem::None;
                    }
                    if let InnerMultipartItem::None = self.item {
                        break;
                    }
                }
            }

            let headers = if let Some(payload) = self.payload.get_mut(safety) {
                match self.state {
                    // read until first boundary
                    InnerState::FirstBoundary => {
                        match InnerMultipart::skip_until_boundary(
                            payload,
                            &self.boundary,
                        )? {
                            Async::Ready(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Ok(Async::Ready(None));
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                            Async::NotReady => return Ok(Async::NotReady),
                        }
                    }
                    // read boundary
                    InnerState::Boundary => {
                        match InnerMultipart::read_boundary(payload, &self.boundary)? {
                            Async::NotReady => return Ok(Async::NotReady),
                            Async::Ready(eof) => {
                                if eof {
                                    self.state = InnerState::Eof;
                                    return Ok(Async::Ready(None));
                                } else {
                                    self.state = InnerState::Headers;
                                }
                            }
                        }
                    }
                    _ => (),
                }

                // read field headers for next field
                if self.state == InnerState::Headers {
                    if let Async::Ready(headers) = InnerMultipart::read_headers(payload)?
                    {
                        self.state = InnerState::Boundary;
                        headers
                    } else {
                        return Ok(Async::NotReady);
                    }
                } else {
                    unreachable!()
                }
            } else {
                log::debug!("NotReady: field is in flight");
                return Ok(Async::NotReady);
            };

            // content type
            let mut mt = mime::APPLICATION_OCTET_STREAM;
            if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if let Ok(ct) = content_type.parse::<mime::Mime>() {
                        mt = ct;
                    }
                }
            }

            self.state = InnerState::Boundary;

            // nested multipart stream
            if mt.type_() == mime::MULTIPART {
                let inner = if let Some(boundary) = mt.get_param(mime::BOUNDARY) {
                    Rc::new(RefCell::new(InnerMultipart {
                        payload: self.payload.clone(),
                        boundary: boundary.as_str().to_owned(),
                        state: InnerState::FirstBoundary,
                        item: InnerMultipartItem::None,
                    }))
                } else {
                    return Err(MultipartError::Boundary);
                };

                self.item = InnerMultipartItem::Multipart(Rc::clone(&inner));

                Ok(Async::Ready(Some(MultipartItem::Nested(Multipart {
                    safety: safety.clone(),
                    error: None,
                    inner: Some(inner),
                }))))
            } else {
                let field = Rc::new(RefCell::new(InnerField::new(
                    self.payload.clone(),
                    self.boundary.clone(),
                    &headers,
                )?));
                self.item = InnerMultipartItem::Field(Rc::clone(&field));

                Ok(Async::Ready(Some(MultipartItem::Field(
                    MultipartField::new(safety.clone(), headers, mt, field),
                ))))
            }
        }
    }
}

impl Drop for InnerMultipart {
    fn drop(&mut self) {
        // InnerMultipartItem::Field has to be dropped first because of Safety.
        self.item = InnerMultipartItem::None;
    }
}

/// A single field in a multipart stream
pub struct MultipartField {
    ct: mime::Mime,
    headers: HeaderMap,
    inner: Rc<RefCell<InnerField>>,
    safety: Safety,
}

impl MultipartField {
    fn new(
        safety: Safety,
        headers: HeaderMap,
        ct: mime::Mime,
        inner: Rc<RefCell<InnerField>>,
    ) -> Self {
        MultipartField {
            ct,
            headers,
            inner,
            safety,
        }
    }

    /// Get a map of headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get the content type of the field
    pub fn content_type(&self) -> &mime::Mime {
        &self.ct
    }

    /// Get the content disposition of the field, if it exists
    pub fn content_disposition(&self) -> Option<ContentDisposition> {
        // RFC 7578: 'Each part MUST contain a Content-Disposition header field
        // where the disposition type is "form-data".'
        if let Some(content_disposition) = self.headers.get(header::CONTENT_DISPOSITION)
        {
            ContentDisposition::from_raw(content_disposition).ok()
        } else {
            None
        }
    }
}

impl Stream for MultipartField {
    type Item = Bytes;
    type Error = MultipartError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.safety.current() {
            self.inner.borrow_mut().poll(&self.safety)
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl fmt::Debug for MultipartField {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "\nMultipartField: {}", self.ct)?;
        writeln!(f, "  boundary: {}", self.inner.borrow().boundary)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers.iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

struct InnerField {
    payload: Option<PayloadRef>,
    boundary: String,
    eof: bool,
    length: Option<u64>,
}

impl InnerField {
    fn new(
        payload: PayloadRef,
        boundary: String,
        headers: &HeaderMap,
    ) -> Result<InnerField, PayloadError> {
        let len = if let Some(len) = headers.get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    Some(len)
                } else {
                    return Err(PayloadError::Incomplete(None));
                }
            } else {
                return Err(PayloadError::Incomplete(None));
            }
        } else {
            None
        };

        Ok(InnerField {
            boundary,
            payload: Some(payload),
            eof: false,
            length: len,
        })
    }

    /// Reads body part content chunk of the specified size.
    /// The body part must has `Content-Length` header with proper value.
    fn read_len(
        payload: &mut PayloadBuffer,
        size: &mut u64,
    ) -> Poll<Option<Bytes>, MultipartError> {
        if *size == 0 {
            Ok(Async::Ready(None))
        } else {
            match payload.readany() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(None)) => Err(MultipartError::Incomplete),
                Ok(Async::Ready(Some(mut chunk))) => {
                    let len = cmp::min(chunk.len() as u64, *size);
                    *size -= len;
                    let ch = chunk.split_to(len as usize);
                    if !chunk.is_empty() {
                        payload.unprocessed(chunk);
                    }
                    Ok(Async::Ready(Some(ch)))
                }
                Err(err) => Err(err.into()),
            }
        }
    }

    /// Reads content chunk of body part with unknown length.
    /// The `Content-Length` header for body part is not necessary.
    fn read_stream(
        payload: &mut PayloadBuffer,
        boundary: &str,
    ) -> Poll<Option<Bytes>, MultipartError> {
        match payload.read_until(b"\r")? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(None) => Err(MultipartError::Incomplete),
            Async::Ready(Some(mut chunk)) => {
                if chunk.len() == 1 {
                    payload.unprocessed(chunk);
                    match payload.read_exact(boundary.len() + 4)? {
                        Async::NotReady => Ok(Async::NotReady),
                        Async::Ready(None) => Err(MultipartError::Incomplete),
                        Async::Ready(Some(mut chunk)) => {
                            if &chunk[..2] == b"\r\n"
                                && &chunk[2..4] == b"--"
                                && &chunk[4..] == boundary.as_bytes()
                            {
                                payload.unprocessed(chunk);
                                Ok(Async::Ready(None))
                            } else {
                                // \r might be part of data stream
                                let ch = chunk.split_to(1);
                                payload.unprocessed(chunk);
                                Ok(Async::Ready(Some(ch)))
                            }
                        }
                    }
                } else {
                    let to = chunk.len() - 1;
                    let ch = chunk.split_to(to);
                    payload.unprocessed(chunk);
                    Ok(Async::Ready(Some(ch)))
                }
            }
        }
    }

    fn poll(&mut self, s: &Safety) -> Poll<Option<Bytes>, MultipartError> {
        if self.payload.is_none() {
            return Ok(Async::Ready(None));
        }

        let result = if let Some(payload) = self.payload.as_ref().unwrap().get_mut(s) {
            let res = if let Some(ref mut len) = self.length {
                InnerField::read_len(payload, len)?
            } else {
                InnerField::read_stream(payload, &self.boundary)?
            };

            match res {
                Async::NotReady => Async::NotReady,
                Async::Ready(Some(bytes)) => Async::Ready(Some(bytes)),
                Async::Ready(None) => {
                    self.eof = true;
                    match payload.readline()? {
                        Async::NotReady => Async::NotReady,
                        Async::Ready(None) => Async::Ready(None),
                        Async::Ready(Some(line)) => {
                            if line.as_ref() != b"\r\n" {
                                log::warn!("multipart field did not read all the data or it is malformed");
                            }
                            Async::Ready(None)
                        }
                    }
                }
            }
        } else {
            Async::NotReady
        };

        if Async::Ready(None) == result {
            self.payload.take();
        }
        Ok(result)
    }
}

struct PayloadRef {
    payload: Rc<UnsafeCell<PayloadBuffer>>,
}

impl PayloadRef {
    fn new(payload: PayloadBuffer) -> PayloadRef {
        PayloadRef {
            payload: Rc::new(payload.into()),
        }
    }

    fn get_mut<'a, 'b>(&'a self, s: &'b Safety) -> Option<&'a mut PayloadBuffer>
    where
        'a: 'b,
    {
        // Unsafe: Invariant is inforced by Safety Safety is used as ref counter,
        // only top most ref can have mutable access to payload.
        if s.current() {
            let payload: &mut PayloadBuffer = unsafe { &mut *self.payload.get() };
            Some(payload)
        } else {
            None
        }
    }
}

impl Clone for PayloadRef {
    fn clone(&self) -> PayloadRef {
        PayloadRef {
            payload: Rc::clone(&self.payload),
        }
    }
}

/// Counter. It tracks of number of clones of payloads and give access to
/// payload only to top most task panics if Safety get destroyed and it not top
/// most task.
#[derive(Debug)]
struct Safety {
    task: Option<Task>,
    level: usize,
    payload: Rc<PhantomData<bool>>,
}

impl Safety {
    fn new() -> Safety {
        let payload = Rc::new(PhantomData);
        Safety {
            task: None,
            level: Rc::strong_count(&payload),
            payload,
        }
    }

    fn current(&self) -> bool {
        Rc::strong_count(&self.payload) == self.level
    }
}

impl Clone for Safety {
    fn clone(&self) -> Safety {
        let payload = Rc::clone(&self.payload);
        Safety {
            task: Some(current_task()),
            level: Rc::strong_count(&payload),
            payload,
        }
    }
}

impl Drop for Safety {
    fn drop(&mut self) {
        // parent task is dead
        if Rc::strong_count(&self.payload) != self.level {
            panic!("Safety get dropped but it is not from top-most task");
        }
        if let Some(task) = self.task.take() {
            task.notify()
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::unsync::mpsc;

    use super::*;
    use crate::http::header::{DispositionParam, DispositionType};
    use crate::test::run_on;

    #[test]
    fn test_boundary() {
        let headers = HeaderMap::new();
        match Multipart::boundary(&headers) {
            Err(MultipartError::NoContentType) => (),
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("test"),
        );

        match Multipart::boundary(&headers) {
            Err(MultipartError::ParseContentType) => (),
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("multipart/mixed"),
        );
        match Multipart::boundary(&headers) {
            Err(MultipartError::Boundary) => (),
            _ => unreachable!("should not happen"),
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(
                "multipart/mixed; boundary=\"5c02368e880e436dab70ed54e1c58209\"",
            ),
        );

        assert_eq!(
            Multipart::boundary(&headers).unwrap(),
            "5c02368e880e436dab70ed54e1c58209"
        );
    }

    fn create_stream() -> (
        mpsc::UnboundedSender<Result<Bytes, PayloadError>>,
        impl Stream<Item = Bytes, Error = PayloadError>,
    ) {
        let (tx, rx) = mpsc::unbounded();

        (tx, rx.map_err(|_| panic!()).and_then(|res| res))
    }

    #[test]
    fn test_multipart() {
        run_on(|| {
            let (sender, payload) = create_stream();

            let bytes = Bytes::from(
                "testasdadsad\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"fn.txt\"\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 test\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0\r\n\
                 Content-Type: text/plain; charset=utf-8\r\nContent-Length: 4\r\n\r\n\
                 data\r\n\
                 --abbc761f78ff4d7cb7573b5a23f96ef0--\r\n",
            );
            sender.unbounded_send(Ok(bytes)).unwrap();

            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(
                    "multipart/mixed; boundary=\"abbc761f78ff4d7cb7573b5a23f96ef0\"",
                ),
            );

            let mut multipart = Multipart::new(&headers, payload);
            match multipart.poll() {
                Ok(Async::Ready(Some(item))) => match item {
                    MultipartItem::Field(mut field) => {
                        {
                            let cd = field.content_disposition().unwrap();
                            assert_eq!(cd.disposition, DispositionType::FormData);
                            assert_eq!(
                                cd.parameters[0],
                                DispositionParam::Name("file".into())
                            );
                        }
                        assert_eq!(field.content_type().type_(), mime::TEXT);
                        assert_eq!(field.content_type().subtype(), mime::PLAIN);

                        match field.poll() {
                            Ok(Async::Ready(Some(chunk))) => assert_eq!(chunk, "test"),
                            _ => unreachable!(),
                        }
                        match field.poll() {
                            Ok(Async::Ready(None)) => (),
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }

            match multipart.poll() {
                Ok(Async::Ready(Some(item))) => match item {
                    MultipartItem::Field(mut field) => {
                        assert_eq!(field.content_type().type_(), mime::TEXT);
                        assert_eq!(field.content_type().subtype(), mime::PLAIN);

                        match field.poll() {
                            Ok(Async::Ready(Some(chunk))) => assert_eq!(chunk, "data"),
                            _ => unreachable!(),
                        }
                        match field.poll() {
                            Ok(Async::Ready(None)) => (),
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }

            match multipart.poll() {
                Ok(Async::Ready(None)) => (),
                _ => unreachable!(),
            }
        });
    }
}
