//! Http server
use std::{time, io};
use std::net::Shutdown;

use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};

mod srv;
mod worker;
mod channel;
mod encoding;
mod h1;
mod h2;
mod h1writer;
mod h2writer;
mod settings;
mod utils;

pub use self::srv::HttpServer;
pub use self::settings::ServerSettings;

use error::Error;
use httprequest::{HttpMessage, HttpRequest};
use httpresponse::HttpResponse;

/// max buffer size 64k
pub(crate) const MAX_WRITE_BUFFER_SIZE: usize = 65_536;

/// Pause accepting incoming connections
///
/// If socket contains some pending connection, they might be dropped.
/// All opened connection remains active.
#[derive(Message)]
pub struct PauseServer;

/// Resume accepting incoming connections
#[derive(Message)]
pub struct ResumeServer;

/// Stop incoming connection processing, stop all workers and exit.
///
/// If server starts with `spawn()` method, then spawned thread get terminated.
#[derive(Message)]
pub struct StopServer {
    pub graceful: bool
}

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

/// Low-level io stream operations
pub trait IoStream: AsyncRead + AsyncWrite + 'static {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()>;

    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;

    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()>;
}

#[derive(Debug)]
pub enum WriterState {
    Done,
    Pause,
}

/// Stream writer
pub trait Writer {
    fn written(&self) -> u64;

    fn start(&mut self, req: &mut HttpMessage, resp: &mut HttpResponse)
             -> Result<WriterState, io::Error>;

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error>;

    fn write_eof(&mut self) -> Result<WriterState, io::Error>;

    fn flush(&mut self) -> Poll<(), io::Error>;

    fn poll_completed(&mut self, shutdown: bool) -> Poll<(), io::Error>;
}
