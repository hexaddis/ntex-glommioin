use std::io::Write;
use std::marker::PhantomData;
use std::ptr::copy_nonoverlapping;
use std::slice::from_raw_parts_mut;
use std::{cmp, io, mem, ptr, slice};

use bytes::{buf::BufMutExt, BufMut, BytesMut};

use crate::http::body::BodySize;
use crate::http::config::DateService;
use crate::http::header::{map, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use crate::http::helpers;
use crate::http::message::{ConnectionType, RequestHeadType};
use crate::http::response::Response;
use crate::http::{HeaderMap, StatusCode, Version};

const AVERAGE_HEADER_SIZE: usize = 30;

#[derive(Debug)]
pub(super) struct MessageEncoder<T: MessageType> {
    pub(super) length: BodySize,
    pub(super) te: TransferEncoding,
    _t: PhantomData<T>,
}

impl<T: MessageType> Default for MessageEncoder<T> {
    fn default() -> Self {
        MessageEncoder {
            length: BodySize::None,
            te: TransferEncoding::empty(),
            _t: PhantomData,
        }
    }
}

pub(crate) trait MessageType: Sized {
    fn status(&self) -> Option<StatusCode>;

    fn headers(&self) -> &HeaderMap;

    fn extra_headers(&self) -> Option<&HeaderMap>;

    fn camel_case(&self) -> bool {
        false
    }

    fn chunked(&self) -> bool;

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()>;

    fn encode_headers(
        &mut self,
        dst: &mut BytesMut,
        version: Version,
        mut length: BodySize,
        ctype: ConnectionType,
        timer: &DateService,
    ) -> io::Result<()> {
        let chunked = self.chunked();
        let mut skip_len = length != BodySize::Stream;
        let camel_case = self.camel_case();

        // Content length
        if let Some(status) = self.status() {
            match status {
                StatusCode::NO_CONTENT
                | StatusCode::CONTINUE
                | StatusCode::PROCESSING => length = BodySize::None,
                StatusCode::SWITCHING_PROTOCOLS => {
                    skip_len = true;
                    length = BodySize::Stream;
                }
                _ => (),
            }
        }
        match length {
            BodySize::Stream => {
                if chunked {
                    if camel_case {
                        dst.put_slice(b"\r\nTransfer-Encoding: chunked\r\n")
                    } else {
                        dst.put_slice(b"\r\ntransfer-encoding: chunked\r\n")
                    }
                } else {
                    skip_len = false;
                    dst.put_slice(b"\r\n");
                }
            }
            BodySize::Empty => {
                if camel_case {
                    dst.put_slice(b"\r\nContent-Length: 0\r\n");
                } else {
                    dst.put_slice(b"\r\ncontent-length: 0\r\n");
                }
            }
            BodySize::Sized(len) => write_content_length(len, dst),
            BodySize::Sized64(len) => {
                if camel_case {
                    dst.put_slice(b"\r\nContent-Length: ");
                } else {
                    dst.put_slice(b"\r\ncontent-length: ");
                }
                #[allow(clippy::write_with_newline)]
                write!(dst.writer(), "{}\r\n", len)?;
            }
            BodySize::None => dst.put_slice(b"\r\n"),
        }

        // Connection
        match ctype {
            ConnectionType::Upgrade => dst.put_slice(b"connection: upgrade\r\n"),
            ConnectionType::KeepAlive if version < Version::HTTP_11 => {
                if camel_case {
                    dst.put_slice(b"Connection: keep-alive\r\n")
                } else {
                    dst.put_slice(b"connection: keep-alive\r\n")
                }
            }
            ConnectionType::Close if version >= Version::HTTP_11 => {
                if camel_case {
                    dst.put_slice(b"Connection: close\r\n")
                } else {
                    dst.put_slice(b"connection: close\r\n")
                }
            }
            _ => (),
        }

        // merging headers from head and extra headers. HeaderMap::new() does not allocate.
        let empty_headers = HeaderMap::new();
        let extra_headers = self.extra_headers().unwrap_or(&empty_headers);
        let headers = self
            .headers()
            .inner
            .iter()
            .filter(|(name, _)| !extra_headers.contains_key(*name))
            .chain(extra_headers.inner.iter());

        // write headers
        let mut pos = 0;
        let mut has_date = false;
        let mut remaining = dst.capacity() - dst.len();
        let mut buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
        for (key, value) in headers {
            match *key {
                CONNECTION => continue,
                TRANSFER_ENCODING | CONTENT_LENGTH if skip_len => continue,
                DATE => {
                    has_date = true;
                }
                _ => (),
            }
            let k = key.as_str().as_bytes();
            match value {
                map::Value::One(ref val) => {
                    let v = val.as_ref();
                    let v_len = v.len();
                    let k_len = k.len();
                    let len = k_len + v_len + 4;

                    unsafe {
                        if len > remaining {
                            dst.advance_mut(pos);
                            pos = 0;
                            dst.reserve(len * 2);
                            remaining = dst.capacity() - dst.len();
                            buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
                        }
                        if camel_case {
                            write_camel_case(k, from_raw_parts_mut(buf, k_len))
                        } else {
                            copy_nonoverlapping(k.as_ptr(), buf, k_len)
                        }
                        buf = buf.add(k_len);
                        copy_nonoverlapping(b": ".as_ptr(), buf, 2);
                        buf = buf.add(2);
                        copy_nonoverlapping(v.as_ptr(), buf, v_len);
                        buf = buf.add(v_len);
                        copy_nonoverlapping(b"\r\n".as_ptr(), buf, 2);
                        buf = buf.add(2);
                    }
                    pos += len;
                    remaining -= len;
                }
                map::Value::Multi(ref vec) => {
                    for val in vec {
                        let v = val.as_ref();
                        let v_len = v.len();
                        let k_len = k.len();
                        let len = k_len + v_len + 4;

                        unsafe {
                            if len > remaining {
                                dst.advance_mut(pos);
                                pos = 0;
                                dst.reserve(len * 2);
                                remaining = dst.capacity() - dst.len();
                                buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
                            }
                            if camel_case {
                                write_camel_case(k, from_raw_parts_mut(buf, k_len));
                            } else {
                                copy_nonoverlapping(k.as_ptr(), buf, k_len);
                            }
                            buf = buf.add(k_len);
                            copy_nonoverlapping(b": ".as_ptr(), buf, 2);
                            buf = buf.add(2);
                            copy_nonoverlapping(v.as_ptr(), buf, v_len);
                            buf = buf.add(v_len);
                            copy_nonoverlapping(b"\r\n".as_ptr(), buf, 2);
                            buf = buf.add(2);
                        };
                        pos += len;
                        remaining -= len;
                    }
                }
            }
        }
        unsafe {
            dst.advance_mut(pos);
        }

        // optimized date header, set_date writes \r\n
        if !has_date {
            timer.set_date_header(dst);
        } else {
            // msg eof
            dst.extend_from_slice(b"\r\n");
        }

        Ok(())
    }
}

impl MessageType for Response<()> {
    fn status(&self) -> Option<StatusCode> {
        Some(self.head().status)
    }

    fn chunked(&self) -> bool {
        self.head().chunked()
    }

    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    fn extra_headers(&self) -> Option<&HeaderMap> {
        None
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        let head = self.head();
        let reason = head.reason().as_bytes();
        dst.reserve(256 + head.headers.len() * AVERAGE_HEADER_SIZE + reason.len());

        // status line
        write_status_line(head.version, head.status.as_u16(), dst);
        dst.put_slice(reason);
        Ok(())
    }
}

impl MessageType for RequestHeadType {
    fn status(&self) -> Option<StatusCode> {
        None
    }

    fn chunked(&self) -> bool {
        self.as_ref().chunked()
    }

    fn camel_case(&self) -> bool {
        self.as_ref().camel_case_headers()
    }

    fn headers(&self) -> &HeaderMap {
        self.as_ref().headers()
    }

    fn extra_headers(&self) -> Option<&HeaderMap> {
        self.extra_headers()
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        let head = self.as_ref();
        dst.reserve(256 + head.headers.len() * AVERAGE_HEADER_SIZE);
        write!(
            helpers::Writer(dst),
            "{} {} {}",
            head.method,
            head.uri.path_and_query().map(|u| u.as_str()).unwrap_or("/"),
            match head.version {
                Version::HTTP_09 => "HTTP/0.9",
                Version::HTTP_10 => "HTTP/1.0",
                Version::HTTP_11 => "HTTP/1.1",
                Version::HTTP_2 => "HTTP/2.0",
                Version::HTTP_3 => "HTTP/3.0",
                _ =>
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "unsupported version"
                    )),
            }
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl<T: MessageType> MessageEncoder<T> {
    /// Encode message
    pub(super) fn encode_chunk(
        &mut self,
        msg: &[u8],
        buf: &mut BytesMut,
    ) -> io::Result<bool> {
        self.te.encode(msg, buf)
    }

    /// Encode eof
    pub(super) fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        self.te.encode_eof(buf)
    }

    pub(super) fn encode(
        &mut self,
        dst: &mut BytesMut,
        message: &mut T,
        head: bool,
        stream: bool,
        version: Version,
        length: BodySize,
        ctype: ConnectionType,
        timer: &DateService,
    ) -> io::Result<()> {
        // transfer encoding
        if !head {
            self.te = match length {
                BodySize::Empty => TransferEncoding::empty(),
                BodySize::Sized(len) => TransferEncoding::length(len as u64),
                BodySize::Sized64(len) => TransferEncoding::length(len),
                BodySize::Stream => {
                    if message.chunked() && !stream {
                        TransferEncoding::chunked()
                    } else {
                        TransferEncoding::eof()
                    }
                }
                BodySize::None => TransferEncoding::empty(),
            };
        } else {
            self.te = TransferEncoding::empty();
        }

        message.encode_status(dst)?;
        message.encode_headers(dst, version, length, ctype, timer)
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug)]
pub(super) struct TransferEncoding {
    kind: TransferEncodingKind,
}

#[derive(Debug, PartialEq, Clone)]
enum TransferEncodingKind {
    /// An Encoder for when Transfer-Encoding includes `chunked`.
    Chunked(bool),
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Application decides when to stop writing.
    Eof,
}

impl TransferEncoding {
    #[inline]
    pub(super) fn empty() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(0),
        }
    }

    #[inline]
    pub(super) fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub(super) fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline]
    pub(super) fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
    pub(super) fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                buf.extend_from_slice(msg);
                Ok(eof)
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                } else {
                    writeln!(helpers::Writer(buf), "{:X}\r", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                    buf.reserve(msg.len() + 2);
                    buf.extend_from_slice(msg);
                    buf.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            }
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0);
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);

                    buf.extend_from_slice(&msg[..len as usize]);

                    *remaining -= len as u64;
                    Ok(*remaining == 0)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    #[inline]
    pub(super) fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        match self.kind {
            TransferEncodingKind::Eof => Ok(()),
            TransferEncodingKind::Length(rem) => {
                if rem != 0 {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
                } else {
                    Ok(())
                }
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                }
                Ok(())
            }
        }
    }
}

fn write_camel_case(value: &[u8], buffer: &mut [u8]) {
    let mut index = 0;
    let key = value;
    let mut key_iter = key.iter();

    if let Some(c) = key_iter.next() {
        if *c >= b'a' && *c <= b'z' {
            buffer[index] = *c ^ b' ';
            index += 1;
        }
    } else {
        return;
    }

    while let Some(c) = key_iter.next() {
        buffer[index] = *c;
        index += 1;
        if *c == b'-' {
            if let Some(c) = key_iter.next() {
                if *c >= b'a' && *c <= b'z' {
                    buffer[index] = *c ^ b' ';
                    index += 1;
                }
            }
        }
    }
}

const DEC_DIGITS_LUT: &[u8] = b"0001020304050607080910111213141516171819\
      2021222324252627282930313233343536373839\
      4041424344454647484950515253545556575859\
      6061626364656667686970717273747576777879\
      8081828384858687888990919293949596979899";

const STATUS_LINE_BUF_SIZE: usize = 13;

fn write_status_line(version: Version, mut n: u16, bytes: &mut BytesMut) {
    let mut buf: [u8; STATUS_LINE_BUF_SIZE] = match version {
        Version::HTTP_2 => *b"HTTP/2       ",
        Version::HTTP_10 => *b"HTTP/1.0     ",
        Version::HTTP_09 => *b"HTTP/0.9     ",
        _ => *b"HTTP/1.1     ",
    };

    let mut curr: isize = 12;
    let buf_ptr = buf.as_mut_ptr();
    let lut_ptr = DEC_DIGITS_LUT.as_ptr();
    let four = n > 999;

    // decode 2 more chars, if > 2 chars
    let d1 = (n % 100) << 1;
    n /= 100;
    curr -= 2;

    unsafe {
        ptr::copy_nonoverlapping(lut_ptr.offset(d1 as isize), buf_ptr.offset(curr), 2);

        // decode last 1 or 2 chars
        if n < 10 {
            curr -= 1;
            *buf_ptr.offset(curr) = (n as u8) + b'0';
        } else {
            let d1 = n << 1;
            curr -= 2;
            ptr::copy_nonoverlapping(
                lut_ptr.offset(d1 as isize),
                buf_ptr.offset(curr),
                2,
            );
        }
    }

    bytes.put_slice(&buf);
    if four {
        bytes.put_u8(b' ');
    }
}

/// NOTE: bytes object has to contain enough space
fn write_content_length(mut n: usize, bytes: &mut BytesMut) {
    if n < 10 {
        let mut buf: [u8; 21] = [
            b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l', b'e',
            b'n', b'g', b't', b'h', b':', b' ', b'0', b'\r', b'\n',
        ];
        buf[18] = (n as u8) + b'0';
        bytes.put_slice(&buf);
    } else if n < 100 {
        let mut buf: [u8; 22] = [
            b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l', b'e',
            b'n', b'g', b't', b'h', b':', b' ', b'0', b'0', b'\r', b'\n',
        ];
        let d1 = n << 1;
        unsafe {
            ptr::copy_nonoverlapping(
                DEC_DIGITS_LUT.as_ptr().add(d1),
                buf.as_mut_ptr().offset(18),
                2,
            );
        }
        bytes.put_slice(&buf);
    } else if n < 1000 {
        let mut buf: [u8; 23] = [
            b'\r', b'\n', b'c', b'o', b'n', b't', b'e', b'n', b't', b'-', b'l', b'e',
            b'n', b'g', b't', b'h', b':', b' ', b'0', b'0', b'0', b'\r', b'\n',
        ];
        // decode 2 more chars, if > 2 chars
        let d1 = (n % 100) << 1;
        n /= 100;
        unsafe {
            ptr::copy_nonoverlapping(
                DEC_DIGITS_LUT.as_ptr().add(d1),
                buf.as_mut_ptr().offset(19),
                2,
            )
        };

        // decode last 1
        buf[18] = (n as u8) + b'0';

        bytes.put_slice(&buf);
    } else {
        bytes.put_slice(b"\r\ncontent-length: ");
        unsafe { convert_usize(n, bytes) };
    }
}

unsafe fn convert_usize(mut n: usize, bytes: &mut BytesMut) {
    let mut curr: isize = 39;
    let mut buf: [u8; 41] = mem::MaybeUninit::uninit().assume_init();
    buf[39] = b'\r';
    buf[40] = b'\n';
    let buf_ptr = buf.as_mut_ptr();
    let lut_ptr = DEC_DIGITS_LUT.as_ptr();

    // eagerly decode 4 characters at a time
    while n >= 10_000 {
        let rem = (n % 10_000) as isize;
        n /= 10_000;

        let d1 = (rem / 100) << 1;
        let d2 = (rem % 100) << 1;
        curr -= 4;
        ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
        ptr::copy_nonoverlapping(lut_ptr.offset(d2), buf_ptr.offset(curr + 2), 2);
    }

    // if we reach here numbers are <= 9999, so at most 4 chars long
    let mut n = n as isize; // possibly reduce 64bit math

    // decode 2 more chars, if > 2 chars
    if n >= 100 {
        let d1 = (n % 100) << 1;
        n /= 100;
        curr -= 2;
        ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
    }

    // decode last 1 or 2 chars
    if n < 10 {
        curr -= 1;
        *buf_ptr.offset(curr) = (n as u8) + b'0';
    } else {
        let d1 = n << 1;
        curr -= 2;
        ptr::copy_nonoverlapping(lut_ptr.offset(d1), buf_ptr.offset(curr), 2);
    }

    bytes.extend_from_slice(slice::from_raw_parts(
        buf_ptr.offset(curr),
        41 - curr as usize,
    ));
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use bytes::Bytes;

    use super::*;
    use crate::http::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
    use crate::http::RequestHead;

    #[test]
    fn test_chunked_te() {
        let mut bytes = BytesMut::new();
        let mut enc = TransferEncoding::chunked();
        {
            assert!(!enc.encode(b"test", &mut bytes).ok().unwrap());
            assert!(enc.encode(b"", &mut bytes).ok().unwrap());
        }
        assert_eq!(
            bytes.split().freeze(),
            Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n")
        );
    }

    #[test]
    fn test_camel_case() {
        let mut bytes = BytesMut::with_capacity(2048);
        let mut head = RequestHead::default();
        head.set_camel_case_headers(true);
        head.headers.insert(DATE, HeaderValue::from_static("date"));
        head.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("plain/text"));

        let mut head = RequestHeadType::Owned(head);

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Empty,
            ConnectionType::Close,
            &DateService::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("Content-Length: 0\r\n"));
        assert!(data.contains("Connection: close\r\n"));
        assert!(data.contains("Content-Type: plain/text\r\n"));
        assert!(data.contains("Date: date\r\n"));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &DateService::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("Transfer-Encoding: chunked\r\n"));
        assert!(data.contains("Content-Type: plain/text\r\n"));
        assert!(data.contains("Date: date\r\n"));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Sized64(100),
            ConnectionType::KeepAlive,
            &DateService::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("Content-Length: 100\r\n"));
        assert!(data.contains("Content-Type: plain/text\r\n"));
        assert!(data.contains("Date: date\r\n"));

        let mut head = RequestHead::default();
        head.set_camel_case_headers(false);
        head.headers.insert(DATE, HeaderValue::from_static("date"));
        head.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("plain/text"));
        head.headers
            .append(CONTENT_TYPE, HeaderValue::from_static("xml"));

        let mut head = RequestHeadType::Owned(head);
        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &DateService::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("transfer-encoding: chunked\r\n"));
        assert!(data.contains("content-type: xml\r\n"));
        assert!(data.contains("content-type: plain/text\r\n"));
        assert!(data.contains("date: date\r\n"));
    }

    #[test]
    fn test_extra_headers() {
        let mut bytes = BytesMut::with_capacity(2048);

        let mut head = RequestHead::default();
        head.headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("some authorization"),
        );

        let mut extra_headers = HeaderMap::new();
        extra_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("another authorization"),
        );
        extra_headers.insert(DATE, HeaderValue::from_static("date"));

        let mut head = RequestHeadType::Rc(Rc::new(head), Some(extra_headers));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Empty,
            ConnectionType::Close,
            &DateService::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("content-length: 0\r\n"));
        assert!(data.contains("connection: close\r\n"));
        assert!(data.contains("authorization: another authorization\r\n"));
        assert!(data.contains("date: date\r\n"));
    }

    #[test]
    fn test_write_content_length() {
        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_content_length(0, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 0\r\n"[..]);
        bytes.reserve(50);
        write_content_length(9, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 9\r\n"[..]);
        bytes.reserve(50);
        write_content_length(10, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 10\r\n"[..]);
        bytes.reserve(50);
        write_content_length(99, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 99\r\n"[..]);
        bytes.reserve(50);
        write_content_length(100, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 100\r\n"[..]);
        bytes.reserve(50);
        write_content_length(101, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 101\r\n"[..]);
        bytes.reserve(50);
        write_content_length(998, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 998\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1000, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 1000\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1001, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 1001\r\n"[..]);
        bytes.reserve(50);
        write_content_length(5909, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 5909\r\n"[..]);
        write_content_length(25999, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 25999\r\n"[..]);
    }
}
