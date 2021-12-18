//! A runtime implementation that runs everything on the current thread.
use std::{future::Future, pin::Pin};

mod arbiter;
mod builder;
mod system;

pub use self::arbiter::Arbiter;
pub use self::builder::{Builder, SystemRunner};
pub use self::system::System;

#[cfg(feature = "tokio")]
mod tokio;
#[cfg(feature = "tokio")]
pub use self::tokio::*;

/// Asynchronous signal handling
pub mod signal {
    #[cfg(unix)]
    pub mod unix {
        pub use tok_io::signal::unix::*;
    }
    pub use tok_io::signal::ctrl_c;
}

/// Task management.
pub mod task {
    pub use tok_io::task::{spawn_blocking, yield_now, JoinError, JoinHandle};
}

pub trait Runtime {
    /// Spawn a future onto the single-threaded runtime.
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()>>>);

    /// Runs the provided future, blocking the current thread until the future
    /// completes.
    fn block_on(&self, f: Pin<Box<dyn Future<Output = ()>>>);
}
