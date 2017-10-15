//! Basic http responses
#![allow(non_upper_case_globals)]
use std::rc::Rc;
use http::StatusCode;

use task::Task;
use route::RouteHandler;
use payload::Payload;
use httprequest::HttpRequest;
use httpresponse::{Body, HttpResponse, HttpResponseBuilder};

pub const HTTPOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HTTPCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HTTPNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
pub const HTTPBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HTTPNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HTTPForbidden: StaticResponse = StaticResponse(StatusCode::FORBIDDEN);
pub const HTTPMethodNotAllowed: StaticResponse = StaticResponse(StatusCode::METHOD_NOT_ALLOWED);
pub const HTTPInternalServerError: StaticResponse =
    StaticResponse(StatusCode::INTERNAL_SERVER_ERROR);


pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn builder(&self) -> HttpResponseBuilder {
        HttpResponse::builder(self.0)
    }
    pub fn response(&self) -> HttpResponse {
        HttpResponse::new(self.0, Body::Empty)
    }
    pub fn with_reason(self, reason: &'static str) -> HttpResponse {
        let mut resp = HttpResponse::new(self.0, Body::Empty);
        resp.set_reason(reason);
        resp
    }
}

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&self, _: HttpRequest, _: Payload, _: Rc<S>) -> Task {
        Task::reply(HttpResponse::new(self.0, Body::Empty))
    }
}

impl From<StaticResponse> for HttpResponse {
    fn from(st: StaticResponse) -> Self {
        st.response()
    }
}


#[cfg(test)]
mod tests {
    use http::StatusCode;
    use super::{HTTPOk, HTTPBadRequest, Body, HttpResponse};

    #[test]
    fn test_builder() {
        let resp = HTTPOk.builder().body(Body::Empty).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_response() {
        let resp = HTTPOk.response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_from() {
        let resp: HttpResponse = HTTPOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_with_reason() {
        let resp = HTTPOk.response();
        assert_eq!(resp.reason(), "");

        let resp = HTTPBadRequest.with_reason("test");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.reason(), "test");
    }
}
