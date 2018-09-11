//! Actix net - framework for the compisible network services for Rust.
//!
//! ## Package feature
//!
//! * `tls` - enables ssl support via `native-tls` crate
//! * `ssl` - enables ssl support via `openssl` crate
//! * `rust-tls` - enables ssl support via `rustls` crate
//!
// #![warn(missing_docs)]

#![cfg_attr(
    feature = "cargo-clippy",
    allow(
        declare_interior_mutable_const,
        borrow_interior_mutable_const
    )
)]

#[macro_use]
extern crate log;
extern crate bytes;
// #[macro_use]
extern crate failure;
#[macro_use]
extern crate futures;
extern crate mio;
extern crate net2;
extern crate num_cpus;
extern crate slab;
extern crate tokio;
extern crate tokio_current_thread;
extern crate tokio_io;
extern crate tokio_reactor;
extern crate tokio_tcp;
extern crate tokio_timer;
extern crate tower_service;
extern crate trust_dns_resolver;

#[allow(unused_imports)]
#[macro_use]
extern crate actix;

#[cfg(feature = "tls")]
extern crate native_tls;

#[cfg(feature = "ssl")]
extern crate openssl;
#[cfg(feature = "ssl")]
extern crate tokio_openssl;

#[cfg(feature = "rust-tls")]
extern crate rustls;
#[cfg(feature = "rust-tls")]
extern crate tokio_rustls;
#[cfg(feature = "rust-tls")]
extern crate webpki;
#[cfg(feature = "rust-tls")]
extern crate webpki_roots;

use actix::Message;

/// re-export for convinience
pub use tower_service::{NewService, Service};

pub(crate) mod accept;
pub mod connector;
pub mod resolver;
pub mod server;
mod server_service;
pub mod service;
pub mod ssl;
pub mod stream;
mod worker;

pub use server::Server;
pub use service::{IntoNewService, IntoService, NewServiceExt, ServiceExt};

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
pub struct StopServer {
    /// Whether to try and shut down gracefully
    pub graceful: bool,
}

impl Message for StopServer {
    type Result = Result<(), ()>;
}

/// Socket id token
#[derive(Clone, Copy)]
pub(crate) struct Token(usize);
