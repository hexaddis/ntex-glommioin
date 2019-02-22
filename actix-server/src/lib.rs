//! General purpose tcp server

mod accept;
mod builder;
mod config;
mod counter;
mod server;
mod services;
mod signals;
pub mod ssl;
mod worker;

pub use self::builder::ServerBuilder;
pub use self::config::{ServiceConfig, ServiceRuntime};
pub use self::server::Server;
pub use self::services::StreamServiceFactory;

/// Socket id token
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Token(usize);

impl Token {
    pub(crate) fn next(&mut self) -> Token {
        let token = Token(self.0 + 1);
        self.0 += 1;
        token
    }
}

/// Start server building process
pub fn build() -> ServerBuilder {
    ServerBuilder::default()
}
