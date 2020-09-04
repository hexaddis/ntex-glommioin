use std::{fmt, io};

use bitflags::bitflags;
use bytes::BytesMut;
use http::{Method, Version};

use crate::codec::{Decoder, Encoder};
use crate::http::body::BodySize;
use crate::http::config::DateService;
use crate::http::error::ParseError;
use crate::http::message::ConnectionType;
use crate::http::request::Request;
use crate::http::response::Response;

use super::decoder::{PayloadDecoder, PayloadItem, PayloadType};
use super::{decoder, encoder};
use super::{Message, MessageType};

bitflags! {
    struct Flags: u8 {
        const HEAD              = 0b0000_0001;
        const KEEPALIVE_ENABLED = 0b0000_0010;
        const STREAM            = 0b0000_0100;
    }
}

/// HTTP/1 Codec
pub struct Codec {
    timer: DateService,
    decoder: decoder::MessageDecoder<Request>,
    payload: Option<PayloadDecoder>,
    version: Version,
    ctype: ConnectionType,

    // encoder part
    flags: Flags,
    encoder: encoder::MessageEncoder<Response<()>>,
}

impl Default for Codec {
    fn default() -> Self {
        Codec::new(DateService::default(), false)
    }
}

impl fmt::Debug for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "h1::Codec({:?})", self.flags)
    }
}

impl Codec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(timer: DateService, keep_alive: bool) -> Self {
        let flags = if keep_alive {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };

        Codec {
            flags,
            timer,
            decoder: decoder::MessageDecoder::default(),
            payload: None,
            version: Version::HTTP_11,
            ctype: ConnectionType::Close,
            encoder: encoder::MessageEncoder::default(),
        }
    }

    #[inline]
    /// Check if request is upgrade
    pub fn upgrade(&self) -> bool {
        self.ctype == ConnectionType::Upgrade
    }

    #[inline]
    /// Check if last response is keep-alive
    pub fn keepalive(&self) -> bool {
        self.ctype == ConnectionType::KeepAlive
    }

    #[inline]
    /// Check if keep-alive enabled on server level
    pub fn keepalive_enabled(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE_ENABLED)
    }

    #[inline]
    /// Check last request's message type
    pub fn message_type(&self) -> MessageType {
        if self.flags.contains(Flags::STREAM) {
            MessageType::Stream
        } else if self.payload.is_none() {
            MessageType::None
        } else {
            MessageType::Payload
        }
    }

    #[inline]
    #[doc(hidden)]
    pub fn set_date_header(&self, dst: &mut BytesMut) {
        self.timer.set_date_header(dst)
    }
}

impl Decoder for Codec {
    type Item = Message<Request>;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(ref mut payload) = self.payload {
            Ok(match payload.decode(src)? {
                Some(PayloadItem::Chunk(chunk)) => Some(Message::Chunk(Some(chunk))),
                Some(PayloadItem::Eof) => {
                    self.payload.take();
                    Some(Message::Chunk(None))
                }
                None => None,
            })
        } else if let Some((req, payload)) = self.decoder.decode(src)? {
            let head = req.head();
            self.flags.set(Flags::HEAD, head.method == Method::HEAD);
            self.version = head.version;
            self.ctype = head.connection_type();
            if self.ctype == ConnectionType::KeepAlive
                && !self.flags.contains(Flags::KEEPALIVE_ENABLED)
            {
                self.ctype = ConnectionType::Close
            }
            match payload {
                PayloadType::None => self.payload = None,
                PayloadType::Payload(pl) => self.payload = Some(pl),
                PayloadType::Stream(pl) => {
                    self.payload = Some(pl);
                    self.flags.insert(Flags::STREAM);
                }
            }
            Ok(Some(Message::Item(req)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for Codec {
    type Item = Message<(Response<()>, BodySize)>;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((mut res, length)) => {
                // set response version
                res.head_mut().version = self.version;

                // connection status
                self.ctype = if let Some(ct) = res.head().ctype() {
                    if ct == ConnectionType::KeepAlive {
                        self.ctype
                    } else {
                        ct
                    }
                } else {
                    self.ctype
                };

                // encode message
                self.encoder.encode(
                    dst,
                    &mut res,
                    self.flags.contains(Flags::HEAD),
                    self.flags.contains(Flags::STREAM),
                    self.version,
                    length,
                    self.ctype,
                    &self.timer,
                )?;
                // self.headers_size = (dst.len() - len) as u32;
            }
            Message::Chunk(Some(bytes)) => {
                self.encoder.encode_chunk(bytes.as_ref(), dst)?;
            }
            Message::Chunk(None) => {
                self.encoder.encode_eof(dst)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;
    use crate::http::{HttpMessage, Method};

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut codec = Codec::default();
        assert!(format!("{:?}", codec).contains("h1::Codec"));

        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let item = codec.decode(&mut buf).unwrap().unwrap();
        let req = item.message();

        assert_eq!(req.method(), Method::GET);
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
               POST /test2 HTTP/1.1\r\n\
               transfer-encoding: chunked\r\n\r\n"
                .iter(),
        );

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"line");

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert!(msg.eof());

        // decode next message
        let item = codec.decode(&mut buf).unwrap().unwrap();
        let req = item.message();
        assert_eq!(*req.method(), Method::POST);
        assert!(req.chunked().unwrap());

        let mut codec = Codec::default();
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\r\n",
        );
        let _item = codec.decode(&mut buf).unwrap().unwrap();
        assert!(codec.upgrade());
        assert!(!codec.keepalive_enabled());
    }
}
