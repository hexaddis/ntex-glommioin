use std::rc::Rc;
use std::net::SocketAddr;

use bytes::Bytes;
use futures::{Future, Poll, Async};
use tokio_io::{AsyncRead, AsyncWrite};

use {h1, h2};
use error::Error;
use h1writer::Writer;
use httprequest::HttpRequest;
use server::ServerSettings;
use worker::WorkerSettings;

/// Low level http request handler
#[allow(unused_variables)]
pub trait HttpHandler: 'static {

    /// Handle request
    fn handle(&mut self, req: HttpRequest) -> Result<Box<HttpHandlerTask>, HttpRequest>;
}

pub trait HttpHandlerTask {

    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error>;

    fn poll(&mut self) -> Poll<(), Error>;

    fn disconnected(&mut self);
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self, settings: ServerSettings) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self, _: ServerSettings) -> Self::Handler {
        self
    }
}

enum HttpProtocol<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    H1(h1::Http1<T, H>),
    H2(h2::Http2<T, H>),
}

#[doc(hidden)]
pub struct HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    proto: Option<HttpProtocol<T, H>>,
}

impl<T, H> HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    pub(crate) fn new(h: Rc<WorkerSettings<H>>,
               io: T, peer: Option<SocketAddr>, http2: bool) -> HttpChannel<T, H>
    {
        h.add_channel();
        if http2 {
            HttpChannel {
                proto: Some(HttpProtocol::H2(
                    h2::Http2::new(h, io, peer, Bytes::new()))) }
        } else {
            HttpChannel {
                proto: Some(HttpProtocol::H1(
                    h1::Http1::new(h, io, peer))) }
        }
    }
}

/*impl<T, H> Drop for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}*/

impl<T, H> Future for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                match h1.poll() {
                    Ok(Async::Ready(h1::Http1Result::Done)) => {
                        h1.settings().remove_channel();
                        return Ok(Async::Ready(()))
                    }
                    Ok(Async::Ready(h1::Http1Result::Switch)) => (),
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) => {
                        h1.settings().remove_channel();
                        return Err(())
                    }
                }
            }
            Some(HttpProtocol::H2(ref mut h2)) => {
                let result = h2.poll();
                match result {
                    Ok(Async::Ready(())) | Err(_) => h2.settings().remove_channel(),
                    _ => (),
                }
                return result
            }
            None => unreachable!(),
        }

        // upgrade to h2
        let proto = self.proto.take().unwrap();
        match proto {
            HttpProtocol::H1(h1) => {
                let (h, io, addr, buf) = h1.into_inner();
                self.proto = Some(
                    HttpProtocol::H2(h2::Http2::new(h, io, addr, buf)));
                self.poll()
            }
            _ => unreachable!()
        }
    }
}
