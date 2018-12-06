use std::fmt;
use std::io::{self, Read};

use bytes::BytesMut;
use futures::{try_ready, Async, AsyncSink, Poll, Sink, StartSend, Stream};
use log::trace;
use tokio_codec::{Decoder, Encoder};
use tokio_io::{AsyncRead, AsyncWrite};

use super::framed::Fuse;

/// A `Sink` of frames encoded to an `AsyncWrite`.
pub struct FramedWrite<T, E> {
    inner: FramedWrite2<Fuse<T, E>>,
}

pub struct FramedWrite2<T> {
    inner: T,
    buffer: BytesMut,
    low_watermark: usize,
    high_watermark: usize,
}

impl<T, E> FramedWrite<T, E>
where
    T: AsyncWrite,
    E: Encoder,
{
    /// Creates a new `FramedWrite` with the given `encoder`.
    pub fn new(inner: T, encoder: E, lw: usize, hw: usize) -> FramedWrite<T, E> {
        FramedWrite {
            inner: framed_write2(Fuse(inner, encoder), lw, hw),
        }
    }
}

impl<T, E> FramedWrite<T, E> {
    /// Returns a reference to the underlying I/O stream wrapped by
    /// `FramedWrite`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_ref(&self) -> &T {
        &self.inner.inner.0
    }

    /// Returns a mutable reference to the underlying I/O stream wrapped by
    /// `FramedWrite`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.inner.0
    }

    /// Consumes the `FramedWrite`, returning its underlying I/O stream.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_inner(self) -> T {
        self.inner.inner.0
    }

    /// Returns a reference to the underlying decoder.
    pub fn encoder(&self) -> &E {
        &self.inner.inner.1
    }

    /// Returns a mutable reference to the underlying decoder.
    pub fn encoder_mut(&mut self) -> &mut E {
        &mut self.inner.inner.1
    }

    /// Check if write buffer is full
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    /// Check if write buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<T, E> FramedWrite<T, E>
where
    E: Encoder,
{
    /// Force send item
    pub fn force_send(&mut self, item: E::Item) -> Result<(), E::Error> {
        self.inner.force_send(item)
    }
}

impl<T, E> Sink for FramedWrite<T, E>
where
    T: AsyncWrite,
    E: Encoder,
{
    type SinkItem = E::Item;
    type SinkError = E::Error;

    fn start_send(&mut self, item: E::Item) -> StartSend<E::Item, E::Error> {
        self.inner.start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.inner.poll_complete()
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        Ok(self.inner.close()?)
    }
}

impl<T, D> Stream for FramedWrite<T, D>
where
    T: Stream,
{
    type Item = T::Item;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.inner.inner.0.poll()
    }
}

impl<T, U> fmt::Debug for FramedWrite<T, U>
where
    T: fmt::Debug,
    U: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FramedWrite")
            .field("inner", &self.inner.get_ref().0)
            .field("encoder", &self.inner.get_ref().1)
            .field("buffer", &self.inner.buffer)
            .finish()
    }
}

// ===== impl FramedWrite2 =====

pub fn framed_write2<T>(
    inner: T,
    low_watermark: usize,
    high_watermark: usize,
) -> FramedWrite2<T> {
    FramedWrite2 {
        inner,
        low_watermark,
        high_watermark,
        buffer: BytesMut::with_capacity(high_watermark),
    }
}

pub fn framed_write2_with_buffer<T>(
    inner: T,
    mut buffer: BytesMut,
    low_watermark: usize,
    high_watermark: usize,
) -> FramedWrite2<T> {
    if buffer.capacity() < high_watermark {
        let bytes_to_reserve = high_watermark - buffer.capacity();
        buffer.reserve(bytes_to_reserve);
    }
    FramedWrite2 {
        inner,
        buffer,
        low_watermark,
        high_watermark,
    }
}

impl<T> FramedWrite2<T> {
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn into_parts(self) -> (T, BytesMut, usize, usize) {
        (
            self.inner,
            self.buffer,
            self.low_watermark,
            self.high_watermark,
        )
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.high_watermark
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl<T> FramedWrite2<T>
where
    T: Encoder,
{
    pub fn force_send(&mut self, item: T::Item) -> Result<(), T::Error> {
        let len = self.buffer.len();
        if len < self.low_watermark {
            self.buffer.reserve(self.high_watermark - len)
        }
        self.inner.encode(item, &mut self.buffer)?;
        Ok(())
    }
}

impl<T> Sink for FramedWrite2<T>
where
    T: AsyncWrite + Encoder,
{
    type SinkItem = T::Item;
    type SinkError = T::Error;

    fn start_send(&mut self, item: T::Item) -> StartSend<T::Item, T::Error> {
        // Check the buffer capacity
        let len = self.buffer.len();
        if len >= self.high_watermark {
            return Ok(AsyncSink::NotReady(item));
        }
        if len < self.low_watermark {
            self.buffer.reserve(self.high_watermark - len)
        }

        self.inner.encode(item, &mut self.buffer)?;

        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        trace!("flushing framed transport");

        while !self.buffer.is_empty() {
            trace!("writing; remaining={}", self.buffer.len());

            let n = try_ready!(self.inner.poll_write(&self.buffer));

            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to \
                     write frame to transport",
                )
                .into());
            }

            // TODO: Add a way to `bytes` to do this w/o returning the drained
            // data.
            let _ = self.buffer.split_to(n);
        }

        // Try flushing the underlying IO
        try_ready!(self.inner.poll_flush());

        trace!("framed transport flushed");
        Ok(Async::Ready(()))
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        try_ready!(self.poll_complete());
        Ok(self.inner.shutdown()?)
    }
}

impl<T: Decoder> Decoder for FramedWrite2<T> {
    type Item = T::Item;
    type Error = T::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<T::Item>, T::Error> {
        self.inner.decode(src)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<T::Item>, T::Error> {
        self.inner.decode_eof(src)
    }
}

impl<T: Read> Read for FramedWrite2<T> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        self.inner.read(dst)
    }
}

impl<T: AsyncRead> AsyncRead for FramedWrite2<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }
}
