use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use crate::channel::mpsc::Receiver;
use crate::codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed};

pub struct Handshake<Io, Codec>
where
    Codec: Encoder + Decoder,
{
    io: Io,
    _t: PhantomData<Codec>,
}

impl<Io, Codec> Handshake<Io, Codec>
where
    Io: AsyncRead + AsyncWrite + Unpin,
    Codec: Encoder + Decoder,
{
    pub(crate) fn new(io: Io) -> Self {
        Self {
            io,
            _t: PhantomData,
        }
    }

    pub fn codec(
        self,
        codec: Codec,
    ) -> HandshakeResult<Io, (), Codec, Receiver<<Codec as Encoder>::Item>> {
        HandshakeResult {
            state: (),
            out: None,
            framed: Framed::new(self.io, codec),
        }
    }
}

#[pin_project::pin_project]
pub struct HandshakeResult<Io, St, Codec: Encoder + Decoder, Out> {
    pub(crate) state: St,
    pub(crate) out: Option<Out>,
    pub(crate) framed: Framed<Io, Codec>,
}

impl<Io, St, Codec: Encoder + Decoder, Out: Unpin> HandshakeResult<Io, St, Codec, Out> {
    #[inline]
    pub fn get_ref(&self) -> &Io {
        self.framed.get_ref()
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut Io {
        self.framed.get_mut()
    }

    pub fn out<U>(self, out: U) -> HandshakeResult<Io, St, Codec, U>
    where
        U: Stream<Item = <Codec as Encoder>::Item> + Unpin,
    {
        HandshakeResult {
            state: self.state,
            framed: self.framed,
            out: Some(out),
        }
    }

    #[inline]
    pub fn state<S>(self, state: S) -> HandshakeResult<Io, S, Codec, Out> {
        HandshakeResult {
            state,
            framed: self.framed,
            out: self.out,
        }
    }
}

impl<Io, St, Codec, Out> Stream for HandshakeResult<Io, St, Codec, Out>
where
    Io: AsyncRead + AsyncWrite + Unpin,
    Codec: Encoder + Decoder,
{
    type Item = Result<<Codec as Decoder>::Item, <Codec as Decoder>::Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.project().framed.next_item(cx)
    }
}

impl<Io, St, Codec, Out> futures::Sink<<Codec as Encoder>::Item>
    for HandshakeResult<Io, St, Codec, Out>
where
    Io: AsyncRead + AsyncWrite + Unpin,
    Codec: Encoder + Decoder,
{
    type Error = <Codec as Encoder>::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if self.framed.is_write_ready() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn start_send(
        self: Pin<&mut Self>,
        item: <Codec as Encoder>::Item,
    ) -> Result<(), Self::Error> {
        self.project().framed.write(item)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.get_mut().framed.flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.get_mut().framed.close(cx)
    }
}
