use std::rc::Rc;
use std::sync::{atomic::AtomicUsize, atomic::Ordering, Arc};
use std::{net, time};

use futures::sync::mpsc::{unbounded, SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::Future;
use net2::TcpStreamExt;
use slab::Slab;
use tokio::executor::current_thread;
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

#[cfg(any(feature = "tls", feature = "alpn", feature = "rust-tls"))]
use futures::future;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;
#[cfg(feature = "tls")]
use tokio_tls::TlsAcceptorExt;

#[cfg(feature = "alpn")]
use openssl::ssl::SslAcceptor;
#[cfg(feature = "alpn")]
use tokio_openssl::SslAcceptorExt;

#[cfg(feature = "rust-tls")]
use rustls::{ServerConfig, Session};
#[cfg(feature = "rust-tls")]
use tokio_rustls::ServerConfigExt;

use actix::msgs::StopArbiter;
use actix::{Actor, Addr, Arbiter, AsyncContext, Context, Handler, Message, Response};

use super::accept::AcceptNotify;
use super::channel::HttpChannel;
use super::settings::{ServerSettings, WorkerSettings};
use super::{HttpHandler, IntoHttpHandler, KeepAlive};

#[derive(Message)]
pub(crate) struct Conn<T> {
    pub io: T,
    pub token: usize,
    pub peer: Option<net::SocketAddr>,
    pub http2: bool,
}

#[derive(Clone)]
pub(crate) struct SocketInfo {
    pub addr: net::SocketAddr,
    pub htype: StreamHandlerType,
}

pub(crate) struct WorkersPool<H: IntoHttpHandler + 'static> {
    sockets: Slab<SocketInfo>,
    pub factory: Arc<Fn() -> Vec<H> + Send + Sync>,
    pub host: Option<String>,
    pub keep_alive: KeepAlive,
}

impl<H: IntoHttpHandler + 'static> WorkersPool<H> {
    pub fn new<F>(factory: F) -> Self
    where
        F: Fn() -> Vec<H> + Send + Sync + 'static,
    {
        WorkersPool {
            factory: Arc::new(factory),
            host: None,
            keep_alive: KeepAlive::Os,
            sockets: Slab::new(),
        }
    }

    pub fn insert(&mut self, addr: net::SocketAddr, htype: StreamHandlerType) -> usize {
        let entry = self.sockets.vacant_entry();
        let token = entry.key();
        entry.insert(SocketInfo { addr, htype });
        token
    }

    pub fn start(
        &mut self, idx: usize, notify: AcceptNotify,
    ) -> (WorkerClient, Addr<Worker<H::Handler>>) {
        let host = self.host.clone();
        let addr = self.sockets[0].addr;
        let factory = Arc::clone(&self.factory);
        let socks = self.sockets.clone();
        let ka = self.keep_alive;
        let (tx, rx) = unbounded::<Conn<net::TcpStream>>();
        let client = WorkerClient::new(idx, tx, self.sockets.clone());
        let conn = client.conn.clone();
        let sslrate = client.sslrate.clone();

        let addr = Arbiter::start(move |ctx: &mut Context<_>| {
            let s = ServerSettings::new(Some(addr), &host, false);
            let apps: Vec<_> =
                (*factory)().into_iter().map(|h| h.into_handler()).collect();
            ctx.add_message_stream(rx);
            Worker::new(apps, socks, ka, s, conn, sslrate, notify)
        });

        (client, addr)
    }
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    pub idx: usize,
    tx: UnboundedSender<Conn<net::TcpStream>>,
    info: Slab<SocketInfo>,
    pub conn: Arc<AtomicUsize>,
    pub sslrate: Arc<AtomicUsize>,
}

impl WorkerClient {
    fn new(
        idx: usize, tx: UnboundedSender<Conn<net::TcpStream>>, info: Slab<SocketInfo>,
    ) -> Self {
        WorkerClient {
            idx,
            tx,
            info,
            conn: Arc::new(AtomicUsize::new(0)),
            sslrate: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn send(
        &self, msg: Conn<net::TcpStream>,
    ) -> Result<(), SendError<Conn<net::TcpStream>>> {
        self.tx.unbounded_send(msg)
    }

    pub fn available(&self, maxconn: usize, maxsslrate: usize) -> bool {
        if maxsslrate <= self.sslrate.load(Ordering::Relaxed) {
            false
        } else {
            maxconn > self.conn.load(Ordering::Relaxed)
        }
    }
}

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
pub(crate) struct StopWorker {
    pub graceful: Option<time::Duration>,
}

impl Message for StopWorker {
    type Result = Result<bool, ()>;
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests
/// processing.
pub(crate) struct Worker<H>
where
    H: HttpHandler + 'static,
{
    settings: Rc<WorkerSettings<H>>,
    socks: Slab<SocketInfo>,
    tcp_ka: Option<time::Duration>,
}

impl<H: HttpHandler + 'static> Worker<H> {
    pub(crate) fn new(
        h: Vec<H>, socks: Slab<SocketInfo>, keep_alive: KeepAlive,
        settings: ServerSettings, conn: Arc<AtomicUsize>, sslrate: Arc<AtomicUsize>,
        notify: AcceptNotify,
    ) -> Worker<H> {
        let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
            Some(time::Duration::new(val as u64, 0))
        } else {
            None
        };

        Worker {
            settings: Rc::new(WorkerSettings::new(
                h, keep_alive, settings, notify, conn, sslrate,
            )),
            socks,
            tcp_ka,
        }
    }

    fn update_time(&self, ctx: &mut Context<Self>) {
        self.settings.update_date();
        ctx.run_later(time::Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }

    fn shutdown_timeout(
        &self, ctx: &mut Context<Self>, tx: oneshot::Sender<bool>, dur: time::Duration,
    ) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = slf.settings.num_channels();
            if num == 0 {
                let _ = tx.send(true);
                Arbiter::current().do_send(StopArbiter(0));
            } else if let Some(d) = dur.checked_sub(time::Duration::new(1, 0)) {
                slf.shutdown_timeout(ctx, tx, d);
            } else {
                info!("Force shutdown http worker, {} connections", num);
                slf.settings.head().traverse::<TcpStream, H>();
                let _ = tx.send(false);
                Arbiter::current().do_send(StopArbiter(0));
            }
        });
    }
}

impl<H: 'static> Actor for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> Handler<Conn<net::TcpStream>> for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Result = ();

    fn handle(&mut self, msg: Conn<net::TcpStream>, _: &mut Context<Self>) {
        if self.tcp_ka.is_some() && msg.io.set_keepalive(self.tcp_ka).is_err() {
            error!("Can not set socket keep-alive option");
        }
        self.socks
            .get_mut(msg.token)
            .unwrap()
            .htype
            .handle(Rc::clone(&self.settings), msg);
    }
}

/// `StopWorker` message handler
impl<H> Handler<StopWorker> for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = self.settings.num_channels();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Response::reply(Ok(true))
        } else if let Some(dur) = msg.graceful {
            info!("Graceful http worker shutdown, {} connections", num);
            let (tx, rx) = oneshot::channel();
            self.shutdown_timeout(ctx, tx, dur);
            Response::async(rx.map_err(|_| ()))
        } else {
            info!("Force shutdown http worker, {} connections", num);
            self.settings.head().traverse::<TcpStream, H>();
            Response::reply(Ok(false))
        }
    }
}

#[derive(Clone)]
pub(crate) enum StreamHandlerType {
    Normal,
    #[cfg(feature = "tls")]
    Tls(TlsAcceptor),
    #[cfg(feature = "alpn")]
    Alpn(SslAcceptor),
    #[cfg(feature = "rust-tls")]
    Rustls(Arc<ServerConfig>),
}

impl StreamHandlerType {
    pub fn is_ssl(&self) -> bool {
        match *self {
            StreamHandlerType::Normal => false,
            #[cfg(feature = "tls")]
            StreamHandlerType::Tls(_) => true,
            #[cfg(feature = "alpn")]
            StreamHandlerType::Alpn(_) => true,
            #[cfg(feature = "rust-tls")]
            StreamHandlerType::Rustls(_) => true,
        }
    }

    fn handle<H: HttpHandler>(
        &mut self, h: Rc<WorkerSettings<H>>, msg: Conn<net::TcpStream>,
    ) {
        match *self {
            StreamHandlerType::Normal => {
                let _ = msg.io.set_nodelay(true);
                let io = TcpStream::from_std(msg.io, &Handle::default())
                    .expect("failed to associate TCP stream");

                current_thread::spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature = "tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let Conn {
                    io, peer, http2, ..
                } = msg;
                let _ = io.set_nodelay(true);
                let io = TcpStream::from_std(io, &Handle::default())
                    .expect("failed to associate TCP stream");
                self.settings.ssl_conn_add();

                current_thread::spawn(TlsAcceptorExt::accept_async(acceptor, io).then(
                    move |res| {
                        self.settings.ssl_conn_del();
                        match res {
                            Ok(io) => current_thread::spawn(HttpChannel::new(
                                h, io, peer, http2,
                            )),
                            Err(err) => {
                                trace!("Error during handling tls connection: {}", err)
                            }
                        };
                        future::result(Ok(()))
                    },
                ));
            }
            #[cfg(feature = "alpn")]
            StreamHandlerType::Alpn(ref acceptor) => {
                let Conn { io, peer, .. } = msg;
                let _ = io.set_nodelay(true);
                let io = TcpStream::from_std(io, &Handle::default())
                    .expect("failed to associate TCP stream");
                self.settings.ssl_conn_add();

                current_thread::spawn(SslAcceptorExt::accept_async(acceptor, io).then(
                    move |res| {
                        self.settings.ssl_conn_del();
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) =
                                    io.get_ref().ssl().selected_alpn_protocol()
                                {
                                    p.len() == 2 && &p == b"h2"
                                } else {
                                    false
                                };
                                current_thread::spawn(HttpChannel::new(
                                    h, io, peer, http2,
                                ));
                            }
                            Err(err) => {
                                trace!("Error during handling tls connection: {}", err)
                            }
                        };
                        future::result(Ok(()))
                    },
                ));
            }
            #[cfg(feature = "rust-tls")]
            StreamHandlerType::Rustls(ref acceptor) => {
                let Conn { io, peer, .. } = msg;
                let _ = io.set_nodelay(true);
                let io = TcpStream::from_std(io, &Handle::default())
                    .expect("failed to associate TCP stream");
                self.settings.ssl_conn_add();

                current_thread::spawn(ServerConfigExt::accept_async(acceptor, io).then(
                    move |res| {
                        self.settings.ssl_conn_del();
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) =
                                    io.get_ref().1.get_alpn_protocol()
                                {
                                    p.len() == 2 && &p == &"h2"
                                } else {
                                    false
                                };
                                current_thread::spawn(HttpChannel::new(
                                    h, io, peer, http2,
                                ));
                            }
                            Err(err) => {
                                trace!("Error during handling tls connection: {}", err)
                            }
                        };
                        future::result(Ok(()))
                    },
                ));
            }
        }
    }

    pub(crate) fn scheme(&self) -> &'static str {
        match *self {
            StreamHandlerType::Normal => "http",
            #[cfg(feature = "tls")]
            StreamHandlerType::Tls(_) => "https",
            #[cfg(feature = "alpn")]
            StreamHandlerType::Alpn(_) => "https",
            #[cfg(feature = "rust-tls")]
            StreamHandlerType::Rustls(_) => "https",
        }
    }
}
