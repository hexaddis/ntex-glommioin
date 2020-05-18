use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures::{ready, Sink, Stream};
use ntex_codec::{Decoder, Encoder};

use super::{Codec, Frame, Message, ProtocolError};

/// Stream error
#[derive(Debug, Display)]
pub enum StreamError<E: fmt::Debug> {
    #[display(fmt = "StreamError::Stream({:?})", _0)]
    Stream(E),
    Protocol(ProtocolError),
}

impl<E: fmt::Debug> std::error::Error for StreamError<E> {}

impl<E: fmt::Debug> From<ProtocolError> for StreamError<E> {
    fn from(err: ProtocolError) -> Self {
        StreamError::Protocol(err)
    }
}

pin_project_lite::pin_project! {
/// Stream ws protocol decoder.
pub struct StreamDecoder<S, E> {
    #[pin]
    stream: S,
    codec: Codec,
    buf: BytesMut,
    _t: PhantomData<E>,
}
}

impl<S, E> StreamDecoder<S, E> {
    pub fn new(stream: S) -> Self {
        StreamDecoder::with(stream, Codec::new())
    }

    pub fn with(stream: S, codec: Codec) -> Self {
        StreamDecoder {
            stream,
            codec,
            buf: BytesMut::new(),
            _t: PhantomData,
        }
    }
}

impl<S, E> Stream for StreamDecoder<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: fmt::Debug,
{
    type Item = Result<Frame, StreamError<E>>;

    #[inline]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        match ready!(this.stream.poll_next(cx)) {
            Some(Ok(buf)) => {
                this.buf.extend(&buf);
                match this.codec.decode(&mut this.buf) {
                    Ok(Some(item)) => Poll::Ready(Some(Ok(item))),
                    Ok(None) => Poll::Pending,
                    Err(err) => Poll::Ready(Some(Err(err.into()))),
                }
            }
            Some(Err(err)) => Poll::Ready(Some(Err(StreamError::Stream(err)))),
            None => Poll::Ready(None),
        }
    }
}

pin_project_lite::pin_project! {
/// Stream ws protocol decoder.
#[derive(Clone)]
pub struct StreamEncoder<S> {
    #[pin]
    sink: S,
    codec: Rc<RefCell<Codec>>,
}
}

impl<S> StreamEncoder<S> {
    pub fn new(sink: S) -> Self {
        StreamEncoder::with(sink, Codec::new())
    }

    pub fn with(sink: S, codec: Codec) -> Self {
        StreamEncoder {
            sink,
            codec: Rc::new(RefCell::new(codec)),
        }
    }
}

impl<S, E> Sink<Result<Message, E>> for StreamEncoder<S>
where
    S: Sink<Result<Bytes, E>>,
    S::Error: fmt::Debug,
{
    type Error = StreamError<S::Error>;

    #[inline]
    fn poll_ready(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(ready!(self
            .project()
            .sink
            .poll_ready(cx)
            .map_err(StreamError::Stream)))
    }

    fn start_send(
        self: Pin<&mut Self>,
        item: Result<Message, E>,
    ) -> Result<(), Self::Error> {
        let this = self.project();

        match item {
            Ok(item) => {
                let mut buf = BytesMut::new();
                this.codec.borrow_mut().encode(item, &mut buf)?;
                this.sink
                    .start_send(Ok(buf.freeze()))
                    .map_err(StreamError::Stream)
            }
            Err(e) => this.sink.start_send(Err(e)).map_err(StreamError::Stream),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(ready!(self
            .project()
            .sink
            .poll_flush(cx)
            .map_err(StreamError::Stream)))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(ready!(self
            .project()
            .sink
            .poll_close(cx)
            .map_err(StreamError::Stream)))
    }
}

#[cfg(test)]
mod tests {
    use futures::{SinkExt, StreamExt};

    use super::*;
    use crate::channel::mpsc;

    #[ntex_rt::test]
    async fn test_decoder() {
        let (tx, rx) = mpsc::channel();
        let mut decoder = StreamDecoder::new(rx);

        let mut buf = BytesMut::new();
        let mut codec = Codec::new().client_mode();
        codec
            .encode(Message::Text("test".to_string()), &mut buf)
            .unwrap();

        tx.send(Ok::<_, ()>(buf.split().freeze())).unwrap();
        let frame = StreamExt::next(&mut decoder).await.unwrap().unwrap();
        match frame {
            Frame::Text(data) => assert_eq!(data, b"test"[..]),
            _ => panic!(),
        }
    }

    #[ntex_rt::test]
    async fn test_encoder() {
        let (tx, mut rx) = mpsc::channel();
        let mut encoder = StreamEncoder::new(tx);

        encoder
            .send(Ok::<_, ()>(Message::Text("test".to_string())))
            .await
            .unwrap();
        encoder.flush().await.unwrap();
        encoder.close().await.unwrap();

        let data = rx.next().await.unwrap().unwrap();
        assert_eq!(data, b"\x81\x04test".as_ref());
        assert!(rx.next().await.is_none());
    }
}
