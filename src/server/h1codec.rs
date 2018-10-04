#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::h1decoder::{H1Decoder, Message};
use super::helpers;
use super::message::RequestPool;
use super::output::{ResponseInfo, ResponseLength};
use body::Body;
use error::ParseError;
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::Version;
use httpresponse::HttpResponse;

pub(crate) enum OutMessage {
    Response(HttpResponse),
    Payload(Bytes),
}

pub(crate) struct H1Codec {
    decoder: H1Decoder,
    encoder: H1Writer,
}

impl H1Codec {
    pub fn new(pool: &'static RequestPool) -> Self {
        H1Codec {
            decoder: H1Decoder::new(pool),
            encoder: H1Writer::new(),
        }
    }
}

impl Decoder for H1Codec {
    type Item = Message;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }
}

impl Encoder for H1Codec {
    type Item = OutMessage;
    type Error = io::Error;

    fn encode(
        &mut self, item: Self::Item, dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            OutMessage::Response(res) => {
                self.encoder.encode(res, dst)?;
            }
            OutMessage::Payload(bytes) => {
                dst.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }
}

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

const AVERAGE_HEADER_SIZE: usize = 30;

struct H1Writer {
    flags: Flags,
    written: u64,
    headers_size: u32,
}

impl H1Writer {
    fn new() -> H1Writer {
        H1Writer {
            flags: Flags::empty(),
            written: 0,
            headers_size: 0,
        }
    }

    fn written(&self) -> u64 {
        self.written
    }

    pub fn reset(&mut self) {
        self.written = 0;
        self.flags = Flags::KEEPALIVE;
    }

    pub fn upgrade(&self) -> bool {
        self.flags.contains(Flags::UPGRADE)
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    fn encode(
        &mut self, mut msg: HttpResponse, buffer: &mut BytesMut,
    ) -> io::Result<()> {
        // prepare task
        let info = ResponseInfo::new(false); // req.inner.method == Method::HEAD);

        //if msg.keep_alive().unwrap_or_else(|| req.keep_alive()) {
        //self.flags = Flags::STARTED | Flags::KEEPALIVE;
        //} else {
        self.flags = Flags::STARTED;
        //}

        // Connection upgrade
        let version = msg.version().unwrap_or_else(|| Version::HTTP_11); //req.inner.version);
        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if self.flags.contains(Flags::KEEPALIVE) {
            if version < Version::HTTP_11 {
                msg.headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if version >= Version::HTTP_11 {
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("close"));
        }
        let body = msg.replace_body(Body::Empty);

        // render message
        {
            let reason = msg.reason().as_bytes();
            if let Body::Binary(ref bytes) = body {
                buffer.reserve(
                    256 + msg.headers().len() * AVERAGE_HEADER_SIZE
                        + bytes.len()
                        + reason.len(),
                );
            } else {
                buffer.reserve(
                    256 + msg.headers().len() * AVERAGE_HEADER_SIZE + reason.len(),
                );
            }

            // status line
            helpers::write_status_line(version, msg.status().as_u16(), buffer);
            buffer.extend_from_slice(reason);

            // content length
            match info.length {
                ResponseLength::Chunked => {
                    buffer.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
                }
                ResponseLength::Zero => {
                    buffer.extend_from_slice(b"\r\ncontent-length: 0\r\n")
                }
                ResponseLength::Length(len) => {
                    helpers::write_content_length(len, buffer)
                }
                ResponseLength::Length64(len) => {
                    buffer.extend_from_slice(b"\r\ncontent-length: ");
                    write!(buffer.writer(), "{}", len)?;
                    buffer.extend_from_slice(b"\r\n");
                }
                ResponseLength::None => buffer.extend_from_slice(b"\r\n"),
            }
            if let Some(ce) = info.content_encoding {
                buffer.extend_from_slice(b"content-encoding: ");
                buffer.extend_from_slice(ce.as_ref());
                buffer.extend_from_slice(b"\r\n");
            }

            // write headers
            let mut pos = 0;
            let mut has_date = false;
            let mut remaining = buffer.remaining_mut();
            let mut buf = unsafe { &mut *(buffer.bytes_mut() as *mut [u8]) };
            for (key, value) in msg.headers() {
                match *key {
                    TRANSFER_ENCODING => continue,
                    CONTENT_LENGTH => match info.length {
                        ResponseLength::None => (),
                        _ => continue,
                    },
                    DATE => {
                        has_date = true;
                    }
                    _ => (),
                }

                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                let len = k.len() + v.len() + 4;
                if len > remaining {
                    unsafe {
                        buffer.advance_mut(pos);
                    }
                    pos = 0;
                    buffer.reserve(len);
                    remaining = buffer.remaining_mut();
                    unsafe {
                        buf = &mut *(buffer.bytes_mut() as *mut _);
                    }
                }

                buf[pos..pos + k.len()].copy_from_slice(k);
                pos += k.len();
                buf[pos..pos + 2].copy_from_slice(b": ");
                pos += 2;
                buf[pos..pos + v.len()].copy_from_slice(v);
                pos += v.len();
                buf[pos..pos + 2].copy_from_slice(b"\r\n");
                pos += 2;
                remaining -= len;
            }
            unsafe {
                buffer.advance_mut(pos);
            }

            // optimized date header, set_date writes \r\n
            if !has_date {
                // self.settings.set_date(&mut buffer, true);
                buffer.extend_from_slice(b"\r\n");
            } else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;
        }

        if let Body::Binary(bytes) = body {
            self.written = bytes.len() as u64;
            // buffer.write(bytes.as_ref())?;
            buffer.extend_from_slice(bytes.as_ref());
        } else {
            // capacity, makes sense only for streaming or actor
            // self.buffer_capacity = msg.write_buffer_capacity();

            msg.replace_body(body);
        }
        Ok(())
    }
}
