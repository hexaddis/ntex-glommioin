#![warn(
    rust_2018_idioms,
    // missing_debug_implementations,
    // missing_docs,
    // unreachable_pub,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::borrow_interior_mutable_const
)]

#[macro_use]
extern crate log;

#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub use actix_macros::{main, test};

pub mod http;
pub mod server;
pub mod web;
pub mod ws;

pub mod service {
    pub use actix_service::*;
}
