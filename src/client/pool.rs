use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::Service;
use futures::future::{ok, Either, FutureResult};
use futures::sync::oneshot;
use futures::task::AtomicTask;
use futures::{Async, Future, Poll};
use http::uri::Authority;
use indexmap::IndexSet;
use slab::Slab;
use tokio_timer::{sleep, Delay};

use super::connect::Connect;
use super::connection::IoConnection;
use super::error::ConnectorError;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) struct Key {
    authority: Authority,
}

impl From<Authority> for Key {
    fn from(authority: Authority) -> Key {
        Key { authority }
    }
}

#[derive(Debug)]
struct AvailableConnection<T> {
    io: T,
    used: Instant,
    created: Instant,
}

/// Connections pool
pub(crate) struct ConnectionPool<T, Io: AsyncRead + AsyncWrite + 'static>(
    T,
    Rc<RefCell<Inner<Io>>>,
);

impl<T, Io> ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Connect, Response = (Connect, Io), Error = ConnectorError>,
{
    pub(crate) fn new(
        connector: T,
        conn_lifetime: Duration,
        conn_keep_alive: Duration,
        disconnect_timeout: Option<Duration>,
        limit: usize,
    ) -> Self {
        ConnectionPool(
            connector,
            Rc::new(RefCell::new(Inner {
                conn_lifetime,
                conn_keep_alive,
                disconnect_timeout,
                limit,
                acquired: 0,
                waiters: Slab::new(),
                waiters_queue: IndexSet::new(),
                available: HashMap::new(),
                task: AtomicTask::new(),
            })),
        )
    }
}

impl<T, Io> Clone for ConnectionPool<T, Io>
where
    T: Clone,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn clone(&self) -> Self {
        ConnectionPool(self.0.clone(), self.1.clone())
    }
}

impl<T, Io> Service<Connect> for ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Connect, Response = (Connect, Io), Error = ConnectorError>,
{
    type Response = IoConnection<Io>;
    type Error = ConnectorError;
    type Future = Either<
        FutureResult<IoConnection<Io>, ConnectorError>,
        Either<WaitForConnection<Io>, OpenConnection<T::Future, Io>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.0.poll_ready()
    }

    fn call(&mut self, req: Connect) -> Self::Future {
        let key = req.key();

        // acquire connection
        match self.1.as_ref().borrow_mut().acquire(&key) {
            Acquire::Acquired(io, created) => {
                // use existing connection
                Either::A(ok(IoConnection::new(
                    io,
                    created,
                    Acquired(key, Some(self.1.clone())),
                )))
            }
            Acquire::NotAvailable => {
                // connection is not available, wait
                let (rx, token) = self.1.as_ref().borrow_mut().wait_for(req);
                Either::B(Either::A(WaitForConnection {
                    rx,
                    key,
                    token,
                    inner: Some(self.1.clone()),
                }))
            }
            Acquire::Available => {
                // open new connection
                Either::B(Either::B(OpenConnection::new(
                    key,
                    self.1.clone(),
                    self.0.call(req),
                )))
            }
        }
    }
}

#[doc(hidden)]
pub struct WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    key: Key,
    token: usize,
    rx: oneshot::Receiver<Result<IoConnection<Io>, ConnectorError>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<Io> Drop for WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(i) = self.inner.take() {
            let mut inner = i.as_ref().borrow_mut();
            inner.release_waiter(&self.key, self.token);
            inner.check_availibility();
        }
    }
}

impl<Io> Future for WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite,
{
    type Item = IoConnection<Io>;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.rx.poll() {
            Ok(Async::Ready(item)) => match item {
                Err(err) => Err(err),
                Ok(conn) => {
                    let _ = self.inner.take();
                    Ok(Async::Ready(conn))
                }
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => {
                let _ = self.inner.take();
                Err(ConnectorError::Disconnected)
            }
        }
    }
}

#[doc(hidden)]
pub struct OpenConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fut: F,
    key: Key,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<F, Io> OpenConnection<F, Io>
where
    F: Future<Item = (Connect, Io), Error = ConnectorError>,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn new(key: Key, inner: Rc<RefCell<Inner<Io>>>, fut: F) -> Self {
        OpenConnection {
            key,
            fut,
            inner: Some(inner),
        }
    }
}

impl<F, Io> Drop for OpenConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut inner = inner.as_ref().borrow_mut();
            inner.release();
            inner.check_availibility();
        }
    }
}

impl<F, Io> Future for OpenConnection<F, Io>
where
    F: Future<Item = (Connect, Io), Error = ConnectorError>,
    Io: AsyncRead + AsyncWrite,
{
    type Item = IoConnection<Io>;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Err(err) => Err(err.into()),
            Ok(Async::Ready((_, io))) => {
                let _ = self.inner.take();
                Ok(Async::Ready(IoConnection::new(
                    io,
                    Instant::now(),
                    Acquired(self.key.clone(), self.inner.clone()),
                )))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
        }
    }
}

struct OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fut: F,
    key: Key,
    rx: Option<oneshot::Sender<Result<IoConnection<Io>, ConnectorError>>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<F, Io> OpenWaitingConnection<F, Io>
where
    F: Future<Item = (Connect, Io), Error = ConnectorError> + 'static,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn spawn(
        key: Key,
        rx: oneshot::Sender<Result<IoConnection<Io>, ConnectorError>>,
        inner: Rc<RefCell<Inner<Io>>>,
        fut: F,
    ) {
        tokio_current_thread::spawn(OpenWaitingConnection {
            key,
            fut,
            rx: Some(rx),
            inner: Some(inner),
        })
    }
}

impl<F, Io> Drop for OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut inner = inner.as_ref().borrow_mut();
            inner.release();
            inner.check_availibility();
        }
    }
}

impl<F, Io> Future for OpenWaitingConnection<F, Io>
where
    F: Future<Item = (Connect, Io), Error = ConnectorError>,
    Io: AsyncRead + AsyncWrite,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Err(err) => {
                let _ = self.inner.take();
                if let Some(rx) = self.rx.take() {
                    let _ = rx.send(Err(err));
                }
                Err(())
            }
            Ok(Async::Ready((_, io))) => {
                let _ = self.inner.take();
                if let Some(rx) = self.rx.take() {
                    let _ = rx.send(Ok(IoConnection::new(
                        io,
                        Instant::now(),
                        Acquired(self.key.clone(), self.inner.clone()),
                    )));
                }
                Ok(Async::Ready(()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
        }
    }
}

enum Acquire<T> {
    Acquired(T, Instant),
    Available,
    NotAvailable,
}

pub(crate) struct Inner<Io> {
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    disconnect_timeout: Option<Duration>,
    limit: usize,
    acquired: usize,
    available: HashMap<Key, VecDeque<AvailableConnection<Io>>>,
    waiters: Slab<(
        Connect,
        oneshot::Sender<Result<IoConnection<Io>, ConnectorError>>,
    )>,
    waiters_queue: IndexSet<(Key, usize)>,
    task: AtomicTask,
}

impl<Io> Inner<Io> {
    fn reserve(&mut self) {
        self.acquired += 1;
    }

    fn release(&mut self) {
        self.acquired -= 1;
    }

    fn release_waiter(&mut self, key: &Key, token: usize) {
        self.waiters.remove(token);
        self.waiters_queue.remove(&(key.clone(), token));
    }

    fn release_conn(&mut self, key: &Key, io: Io, created: Instant) {
        self.acquired -= 1;
        self.available
            .entry(key.clone())
            .or_insert_with(VecDeque::new)
            .push_back(AvailableConnection {
                io,
                created,
                used: Instant::now(),
            });
    }
}

impl<Io> Inner<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    /// connection is not available, wait
    fn wait_for(
        &mut self,
        connect: Connect,
    ) -> (
        oneshot::Receiver<Result<IoConnection<Io>, ConnectorError>>,
        usize,
    ) {
        let (tx, rx) = oneshot::channel();

        let key = connect.key();
        let entry = self.waiters.vacant_entry();
        let token = entry.key();
        entry.insert((connect, tx));
        assert!(!self.waiters_queue.insert((key, token)));
        (rx, token)
    }

    fn acquire(&mut self, key: &Key) -> Acquire<Io> {
        // check limits
        if self.limit > 0 && self.acquired >= self.limit {
            return Acquire::NotAvailable;
        }

        self.reserve();

        // check if open connection is available
        // cleanup stale connections at the same time
        if let Some(ref mut connections) = self.available.get_mut(key) {
            let now = Instant::now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.used) > self.conn_keep_alive
                    || (now - conn.created) > self.conn_lifetime
                {
                    if let Some(timeout) = self.disconnect_timeout {
                        tokio_current_thread::spawn(CloseConnection::new(
                            conn.io, timeout,
                        ))
                    }
                } else {
                    let mut io = conn.io;
                    let mut buf = [0; 2];
                    match io.read(&mut buf) {
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Ok(n) if n > 0 => {
                            if let Some(timeout) = self.disconnect_timeout {
                                tokio_current_thread::spawn(CloseConnection::new(
                                    io, timeout,
                                ))
                            }
                            continue;
                        }
                        Ok(_) | Err(_) => continue,
                    }
                    return Acquire::Acquired(io, conn.created);
                }
            }
        }
        Acquire::Available
    }

    fn release_close(&mut self, io: Io) {
        self.acquired -= 1;
        if let Some(timeout) = self.disconnect_timeout {
            tokio_current_thread::spawn(CloseConnection::new(io, timeout))
        }
    }

    fn check_availibility(&self) {
        if !self.waiters_queue.is_empty() && self.acquired < self.limit {
            self.task.notify()
        }
    }
}

struct ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    connector: T,
    inner: Rc<RefCell<Inner<Io>>>,
}

impl<T, Io> Future for ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Connect, Response = (Connect, Io), Error = ConnectorError>,
    T::Future: 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut inner = self.inner.as_ref().borrow_mut();
        inner.task.register();

        // check waiters
        loop {
            let (key, token) = {
                if let Some((key, token)) = inner.waiters_queue.get_index(0) {
                    (key.clone(), *token)
                } else {
                    break;
                }
            };
            match inner.acquire(&key) {
                Acquire::NotAvailable => break,
                Acquire::Acquired(io, created) => {
                    let (_, tx) = inner.waiters.remove(token);
                    if let Err(conn) = tx.send(Ok(IoConnection::new(
                        io,
                        created,
                        Acquired(key.clone(), Some(self.inner.clone())),
                    ))) {
                        let (io, created) = conn.unwrap().into_inner();
                        inner.release_conn(&key, io, created);
                    }
                }
                Acquire::Available => {
                    let (connect, tx) = inner.waiters.remove(token);
                    OpenWaitingConnection::spawn(
                        key.clone(),
                        tx,
                        self.inner.clone(),
                        self.connector.call(connect),
                    );
                }
            }
            let _ = inner.waiters_queue.swap_remove_index(0);
        }

        Ok(Async::NotReady)
    }
}

struct CloseConnection<T> {
    io: T,
    timeout: Delay,
}

impl<T> CloseConnection<T>
where
    T: AsyncWrite,
{
    fn new(io: T, timeout: Duration) -> Self {
        CloseConnection {
            io,
            timeout: sleep(timeout),
        }
    }
}

impl<T> Future for CloseConnection<T>
where
    T: AsyncWrite,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        match self.timeout.poll() {
            Ok(Async::Ready(_)) | Err(_) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => match self.io.shutdown() {
                Ok(Async::Ready(_)) | Err(_) => Ok(Async::Ready(())),
                Ok(Async::NotReady) => Ok(Async::NotReady),
            },
        }
    }
}

pub(crate) struct Acquired<T>(Key, Option<Rc<RefCell<Inner<T>>>>);

impl<T> Acquired<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    pub(crate) fn close(&mut self, conn: IoConnection<T>) {
        if let Some(inner) = self.1.take() {
            let (io, _) = conn.into_inner();
            inner.as_ref().borrow_mut().release_close(io);
        }
    }
    pub(crate) fn release(&mut self, conn: IoConnection<T>) {
        if let Some(inner) = self.1.take() {
            let (io, created) = conn.into_inner();
            inner
                .as_ref()
                .borrow_mut()
                .release_conn(&self.0, io, created);
        }
    }
}

impl<T> Drop for Acquired<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.1.take() {
            inner.as_ref().borrow_mut().release();
        }
    }
}
