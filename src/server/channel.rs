use std::{ptr, mem, time, io};
use std::rc::Rc;
use std::net::{SocketAddr, Shutdown};

use bytes::{Bytes, BytesMut, Buf, BufMut};
use futures::{Future, Poll, Async};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::TcpStream;

use super::{h1, h2, utils, HttpHandler, IoStream};
use super::settings::WorkerSettings;

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";


enum HttpProtocol<T: IoStream, H: 'static> {
    H1(h1::Http1<T, H>),
    H2(h2::Http2<T, H>),
    Unknown(Rc<WorkerSettings<H>>, Option<SocketAddr>, T, BytesMut),
}
impl<T: IoStream, H: 'static> HttpProtocol<T, H> {
    fn is_unknown(&self) -> bool {
        match *self {
            HttpProtocol::Unknown(_, _, _, _) => true,
            _ => false
        }
    }
}

enum ProtocolKind {
    Http1,
    Http2,
}

#[doc(hidden)]
pub struct HttpChannel<T, H> where T: IoStream, H: HttpHandler + 'static {
    proto: Option<HttpProtocol<T, H>>,
    node: Option<Node<HttpChannel<T, H>>>,
}

impl<T, H> HttpChannel<T, H> where T: IoStream, H: HttpHandler + 'static
{
    pub(crate) fn new(settings: Rc<WorkerSettings<H>>,
                      io: T, peer: Option<SocketAddr>, http2: bool) -> HttpChannel<T, H>
    {
        settings.add_channel();
        if http2 {
            HttpChannel {
                node: None,
                proto: Some(HttpProtocol::H2(
                    h2::Http2::new(settings, io, peer, Bytes::new()))) }
        } else {
            HttpChannel {
                node: None,
                proto: Some(HttpProtocol::Unknown(
                    settings, peer, io, BytesMut::with_capacity(4096))) }
        }
    }

    fn shutdown(&mut self) {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                let io = h1.io();
                let _ = IoStream::set_linger(io, Some(time::Duration::new(0, 0)));
                let _ = IoStream::shutdown(io, Shutdown::Both);
            }
            Some(HttpProtocol::H2(ref mut h2)) => {
                h2.shutdown()
            }
            _ => (),
        }
    }
}

impl<T, H> Future for HttpChannel<T, H>
    where T: IoStream, H: HttpHandler + 'static
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if !self.proto.as_ref().map(|p| p.is_unknown()).unwrap_or(false) && self.node.is_none() {
            self.node = Some(Node::new(self));
            match self.proto {
                Some(HttpProtocol::H1(ref mut h1)) =>
                    h1.settings().head().insert(self.node.as_ref().unwrap()),
                Some(HttpProtocol::H2(ref mut h2)) =>
                    h2.settings().head().insert(self.node.as_ref().unwrap()),
                _ => (),
            }
        }

        let kind = match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                match h1.poll() {
                    Ok(Async::Ready(())) => {
                        h1.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                        return Ok(Async::Ready(()))
                    }
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) => {
                        h1.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                        return Err(())
                    }
                }
            },
            Some(HttpProtocol::H2(ref mut h2)) => {
                let result = h2.poll();
                match result {
                    Ok(Async::Ready(())) | Err(_) => {
                        h2.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                    }
                    _ => (),
                }
                return result
            },
            Some(HttpProtocol::Unknown(_, _, ref mut io, ref mut buf)) => {
                match utils::read_from_io(io, buf) {
                    Ok(Async::Ready(0)) => {
                        debug!("Ignored premature client disconnection");
                        return Err(())
                    },
                    Err(err) => {
                        debug!("Ignored premature client disconnection {}", err);
                        return Err(())
                    }
                    _ => (),
                }

                if buf.len() >= 14 {
                    if buf[..14] == HTTP2_PREFACE[..] {
                        ProtocolKind::Http2
                    } else {
                        ProtocolKind::Http1
                    }
                } else {
                    return Ok(Async::NotReady);
                }
            },
            None => unreachable!(),
        };

        // upgrade to h2
        let proto = self.proto.take().unwrap();
        match proto {
            HttpProtocol::Unknown(settings, addr, io, buf) => {
                match kind {
                    ProtocolKind::Http1 => {
                        self.proto = Some(
                            HttpProtocol::H1(h1::Http1::new(settings, io, addr, buf)));
                        self.poll()
                    },
                    ProtocolKind::Http2 => {
                        self.proto = Some(
                            HttpProtocol::H2(h2::Http2::new(settings, io, addr, buf.freeze())));
                        self.poll()
                    },
                }
            }
            _ => unreachable!()
        }
    }
}

pub(crate) struct Node<T>
{
    next: Option<*mut Node<()>>,
    prev: Option<*mut Node<()>>,
    element: *mut T,
}

impl<T> Node<T>
{
    fn new(el: &mut T) -> Self {
        Node {
            next: None,
            prev: None,
            element: el as *mut _,
        }
    }

    fn insert<I>(&self, next: &Node<I>) {
        #[allow(mutable_transmutes)]
        unsafe {
            if let Some(ref next2) = self.next {
                let n: &mut Node<()> = mem::transmute(next2.as_ref().unwrap());
                n.prev = Some(next as *const _ as *mut _);
            }
            let slf: &mut Node<T> = mem::transmute(self);
            slf.next = Some(next as *const _ as *mut _);

            let next: &mut Node<T> = mem::transmute(next);
            next.prev = Some(slf as *const _ as *mut _);
        }
    }

    fn remove(&self) {
        #[allow(mutable_transmutes)]
        unsafe {
            if let Some(ref prev) = self.prev {
                let p: &mut Node<()> = mem::transmute(prev.as_ref().unwrap());
                let slf: &mut Node<T> = mem::transmute(self);
                p.next = slf.next.take();
            }
        }
    }
}


impl Node<()> {

    pub(crate) fn head() -> Self {
        Node {
            next: None,
            prev: None,
            element: ptr::null_mut(),
        }
    }

    pub(crate) fn traverse<T, H>(&self) where T: IoStream, H: HttpHandler + 'static {
        let mut next = self.next.as_ref();
        loop {
            if let Some(n) = next {
                unsafe {
                    let n: &Node<()> = mem::transmute(n.as_ref().unwrap());
                    next = n.next.as_ref();

                    if !n.element.is_null() {
                        let ch: &mut HttpChannel<T, H> = mem::transmute(
                            &mut *(n.element as *mut _));
                        ch.shutdown();
                    }
                }
            } else {
                return
            }
        }
    }
}

impl IoStream for TcpStream {
    #[inline]
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, how)
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        TcpStream::set_nodelay(self, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_linger(self, dur)
    }
}


/// Wrapper for `AsyncRead + AsyncWrite` types
pub(crate) struct WrapperStream<T> where T: AsyncRead + AsyncWrite + 'static {
   io: T,
}

impl<T> WrapperStream<T> where T: AsyncRead + AsyncWrite + 'static
{
    pub fn new(io: T) -> Self {
        WrapperStream{io: io}
    }
}

impl<T> IoStream for WrapperStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    #[inline]
    fn shutdown(&mut self, _: Shutdown) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, _: bool) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_linger(&mut self, _: Option<time::Duration>) -> io::Result<()> {
        Ok(())
    }
}

impl<T> io::Read for WrapperStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.read(buf)
    }
}

impl<T> io::Write for WrapperStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.write(buf)
    }
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.io.flush()
    }
}

impl<T> AsyncRead for WrapperStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.io.read_buf(buf)
    }
}

impl<T> AsyncWrite for WrapperStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.shutdown()
    }
    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.io.write_buf(buf)
    }
}


#[cfg(feature="alpn")]
use tokio_openssl::SslStream;

#[cfg(feature="alpn")]
impl IoStream for SslStream<TcpStream> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = self.get_mut().shutdown();
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().get_mut().set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().get_mut().set_linger(dur)
    }
}

#[cfg(feature="tls")]
use tokio_tls::TlsStream;

#[cfg(feature="tls")]
impl IoStream for TlsStream<TcpStream> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = self.get_mut().shutdown();
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().get_mut().set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().get_mut().set_linger(dur)
    }
}
