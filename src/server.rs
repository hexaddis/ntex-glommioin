use std::{io, net, thread};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::marker::PhantomData;

use actix::dev::*;
use futures::Stream;
use futures::sync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::TcpStream;
use num_cpus;
use socket2::{Socket, Domain, Type};

#[cfg(feature="tls")]
use futures::{future, Future};
#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

#[cfg(feature="alpn")]
use futures::{future, Future};
#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslAcceptor, SslAcceptorBuilder};
#[cfg(feature="alpn")]
use openssl::pkcs12::ParsedPkcs12;
#[cfg(feature="alpn")]
use tokio_openssl::{SslStream, SslAcceptorExt};

use utils;
use channel::{HttpChannel, HttpHandler, IntoHttpHandler};

/// Various server settings
#[derive(Debug, Clone)]
pub struct ServerSettings {
    addr: Option<net::SocketAddr>,
    secure: bool,
    host: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: None,
            secure: false,
            host: "localhost:8080".to_owned(),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    fn new(addr: Option<net::SocketAddr>, secure: bool) -> Self {
        let host = if let Some(ref addr) = addr {
            format!("{}", addr)
        } else {
            "unknown".to_owned()
        };
        ServerSettings {
            addr: addr,
            secure: secure,
            host: host,
        }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// An HTTP Server
///
/// `T` - async stream,  anything that implements `AsyncRead` + `AsyncWrite`.
///
/// `A` - peer address
///
/// `H` - request handler
pub struct HttpServer<T, A, H, U>
    where H: 'static
{
    h: Rc<Vec<H>>,
    io: PhantomData<T>,
    addr: PhantomData<A>,
    threads: usize,
    factory: Arc<Fn() -> U + Send + Sync>,
    workers: Vec<SyncAddress<Worker<H>>>,
}

impl<T: 'static, A: 'static, H, U: 'static> Actor for HttpServer<T, A, H, U> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<T: 'static, A: 'static, H, U: 'static>  HttpServer<T, A, H, U> {
    fn update_time(&self, ctx: &mut Context<Self>) {
        utils::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<T, A, H, U, V> HttpServer<T, A, H, U>
    where A: 'static,
          T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler,
          U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Create new http server with vec of http handlers
    pub fn new<F>(factory: F) -> Self
        where F: Sync + Send + 'static + Fn() -> U,
    {
        HttpServer{ h: Rc::new(Vec::new()),
                    io: PhantomData,
                    addr: PhantomData,
                    threads: num_cpus::get(),
                    factory: Arc::new(factory),
                    workers: Vec::new(),
        }
    }

    /// Set number of workers to start.
    ///
    /// By default http server uses number of available logical cpu as threads count.
    pub fn threads(mut self, num: usize) -> Self {
        self.threads = num;
        self
    }

    /// Start listening for incomming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn serve_incoming<S, Addr>(mut self, stream: S, secure: bool) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: Stream<Item=(T, A), Error=io::Error> + 'static
    {
        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(Some(addr), secure);
        let mut apps: Vec<_> = (*self.factory)().into_iter().map(|h| h.into_handler()).collect();
        for app in &mut apps {
            app.server_settings(settings.clone());
        }
        self.h = Rc::new(apps);

        // start server
        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(
                move |(t, _)| IoStream{io: t, peer: None, http2: false}));
            self
        }))
    }

    fn bind<S: net::ToSocketAddrs>(&self, addr: S)
                                   -> io::Result<Vec<(net::SocketAddr, Socket)>>
    {
        let mut err = None;
        let mut sockets = Vec::new();
        if let Ok(iter) = addr.to_socket_addrs() {
            for addr in iter {
                match addr {
                    net::SocketAddr::V4(a) => {
                        let socket = Socket::new(Domain::ipv4(), Type::stream(), None)?;
                        match socket.bind(&a.into()) {
                            Ok(_) => {
                                socket.listen(1024)
                                    .expect("failed to set socket backlog");
                                socket.set_reuse_address(true)
                                    .expect("failed to set socket reuse address");
                                sockets.push((addr, socket));
                            },
                            Err(e) => err = Some(e),
                        }
                    }
                    net::SocketAddr::V6(a) => {
                        let socket = Socket::new(Domain::ipv6(), Type::stream(), None)?;
                        match socket.bind(&a.into()) {
                            Ok(_) => {
                                socket.listen(1024)
                                    .expect("failed to set socket backlog");
                                socket.set_reuse_address(true)
                                    .expect("failed to set socket reuse address");
                                sockets.push((addr, socket))
                            }
                            Err(e) => err = Some(e),
                        }
                    }
                }
            }
        }

        if sockets.is_empty() {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "Can not bind to address."))
            }
        } else {
            Ok(sockets)
        }
    }

    fn start_workers(&mut self, settings: &ServerSettings, handler: &StreamHandlerType)
                     -> Vec<mpsc::UnboundedSender<IoStream<net::TcpStream>>>
    {
        // start workers
        let mut workers = Vec::new();
        for _ in 0..self.threads {
            let s = settings.clone();
            let (tx, rx) = mpsc::unbounded::<IoStream<net::TcpStream>>();

            let h = handler.clone();
            let factory = Arc::clone(&self.factory);
            let addr = Arbiter::start(move |ctx: &mut Context<_>| {
                let mut apps: Vec<_> = (*factory)()
                    .into_iter().map(|h| h.into_handler()).collect();
                for app in &mut apps {
                    app.server_settings(s.clone());
                }
                ctx.add_stream(rx);
                Worker{h: Rc::new(apps), handler: h}
            });
            workers.push(tx);
            self.workers.push(addr);
        }
        info!("Starting {} http workers", self.threads);
        workers
    }
}

impl<H: HttpHandler, U, V> HttpServer<TcpStream, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    /// It also starts number of http handler workers in seperate threads.
    /// For each address this method starts separate thread which does `accept()` in a loop.
    pub fn serve<S, Addr>(mut self, addr: S) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;
        let settings = ServerSettings::new(Some(addrs[0].0), false);
        let workers = self.start_workers(&settings, &StreamHandlerType::Normal);

        // start acceptors threads
        for (addr, sock) in addrs {
            info!("Starting http server on {}", addr);
            start_accept_thread(sock, addr, workers.clone());
        }

        // start http server actor
        Ok(HttpServer::create(|_| {self}))
    }
}

#[cfg(feature="tls")]
impl<H: HttpHandler, U, V> HttpServer<TlsStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    pub fn serve_tls<S, Addr>(mut self, addr: S, pkcs12: ::Pkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;
        let settings = ServerSettings::new(Some(addrs[0].0), false);
        let acceptor = match TlsAcceptor::builder(pkcs12) {
            Ok(builder) => {
                match builder.build() {
                    Ok(acceptor) => acceptor,
                    Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
                }
            }
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
        };
        let workers = self.start_workers(&settings, &StreamHandlerType::Tls(acceptor));

        // start acceptors threads
        for (addr, sock) in addrs {
            info!("Starting tls http server on {}", addr);
            start_accept_thread(sock, addr, workers.clone());
        }

        // start http server actor
        Ok(HttpServer::create(|_| {self}))
    }
}

#[cfg(feature="alpn")]
impl<H: HttpHandler, U, V> HttpServer<SslStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    pub fn serve_tls<S, Addr>(mut self, addr: S, identity: &ParsedPkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;
        let settings = ServerSettings::new(Some(addrs[0].0), false);
        let acceptor = match SslAcceptorBuilder::mozilla_intermediate(
            SslMethod::tls(), &identity.pkey, &identity.cert, &identity.chain)
        {
            Ok(mut builder) => {
                match builder.set_alpn_protocols(&[b"h2", b"http/1.1"]) {
                    Ok(_) => builder.build(),
                    Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err)),
                }
            },
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
        };
        let workers = self.start_workers(&settings, &StreamHandlerType::Alpn(acceptor));

        // start acceptors threads
        for (addr, sock) in addrs {
            info!("Starting tls http server on {}", addr);
            start_accept_thread(sock, addr, workers.clone());
        }

        // start http server actor
        Ok(HttpServer::create(|_| {self}))
    }
}

struct IoStream<T> {
    io: T,
    peer: Option<net::SocketAddr>,
    http2: bool,
}

impl<T> ResponseType for IoStream<T>
{
    type Item = ();
    type Error = ();
}

impl<T, A, H, U> StreamHandler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static {}

impl<T, A, H, U> Handler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(Rc::clone(&self.h), msg.io, msg.peer, msg.http2));
        Self::empty()
    }
}


/// Http workers
///
/// Worker accepts Socket objects via unbounded channel and start requests processing.
struct Worker<H> {
    h: Rc<Vec<H>>,
    handler: StreamHandlerType,
}

impl<H: 'static> Worker<H> {
    fn update_time(&self, ctx: &mut Context<Self>) {
        utils::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<H: 'static> Actor for Worker<H> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> StreamHandler<IoStream<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static {}

impl<H> Handler<IoStream<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: IoStream<net::TcpStream>, _: &mut Context<Self>)
              -> Response<Self, IoStream<net::TcpStream>>
    {
        self.handler.handle(Rc::clone(&self.h), msg);
        Self::empty()
    }
}

#[derive(Clone)]
enum StreamHandlerType {
    Normal,
    #[cfg(feature="tls")]
    Tls(TlsAcceptor),
    #[cfg(feature="alpn")]
    Alpn(SslAcceptor),
}

impl StreamHandlerType {
    fn handle<H: HttpHandler>(&mut self, h: Rc<Vec<H>>, msg: IoStream<net::TcpStream>) {
        match *self {
            StreamHandlerType::Normal => {
                let io = TcpStream::from_stream(msg.io, Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature="tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let IoStream { io, peer, http2 } = msg;
                let io = TcpStream::from_stream(io, Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(
                    TlsAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => Arbiter::handle().spawn(
                                HttpChannel::new(h, io, peer, http2)),
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
            #[cfg(feature="alpn")]
            StreamHandlerType::Alpn(ref acceptor) => {
                let IoStream { io, peer, .. } = msg;
                let io = TcpStream::from_stream(io, Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(
                    SslAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) = io.get_ref().ssl().selected_alpn_protocol()
                                {
                                    p.len() == 2 && &p == b"h2"
                                } else {
                                    false
                                };
                                Arbiter::handle().spawn(HttpChannel::new(h, io, peer, http2));
                            },
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
        }
    }
}

fn start_accept_thread(sock: Socket, addr: net::SocketAddr,
                       workers: Vec<mpsc::UnboundedSender<IoStream<net::TcpStream>>>) {
    // start acceptors thread
    let _ = thread::Builder::new().name(format!("Accept on {}", addr)).spawn(move || {
        let mut next = 0;
        loop {
            match sock.accept() {
                Ok((socket, addr)) => {
                    let addr = if let Some(addr) = addr.as_inet() {
                        net::SocketAddr::V4(addr)
                    } else {
                        net::SocketAddr::V6(addr.as_inet6().unwrap())
                    };
                    let msg = IoStream{
                        io: socket.into_tcp_stream(), peer: Some(addr), http2: false};
                    workers[next].unbounded_send(msg).expect("worker thread died");
                    next = (next + 1) % workers.len();
                }
                Err(err) => error!("Error accepting connection: {:?}", err),
            }
        }
    });
}
