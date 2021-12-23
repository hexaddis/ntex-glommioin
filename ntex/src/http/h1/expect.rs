use std::io;
use std::task::{Context, Poll};

use crate::http::request::Request;
use crate::{util::Ready, Service, ServiceFactory};

pub struct ExpectHandler;

impl ServiceFactory<Request> for ExpectHandler {
    type Response = Request;
    type Error = io::Error;
    type Service = ExpectHandler;
    type InitError = io::Error;
    type Future = Ready<Self::Service, Self::InitError>;

    #[inline]
    fn new_service(&self, _: ()) -> Self::Future {
        Ready::Ok(ExpectHandler)
    }
}

impl Service<Request> for ExpectHandler {
    type Response = Request;
    type Error = io::Error;
    type Future = Ready<Self::Response, Self::Error>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&self, req: Request) -> Self::Future {
        Ready::Ok(req)
    }
}
