use std::io;

use bytes::Bytes;
use futures::future::ok;
use futures::{SinkExt, StreamExt};

use ntex::codec::Framed;
use ntex::http::test::server as test_server;
use ntex::http::ws::handshake_response;
use ntex::http::{body::BodySize, h1, HttpService, Request, Response};
use ntex::util::framed::Dispatcher;
use ntex::ws;

async fn ws_service(req: ws::Frame) -> Result<Option<ws::Message>, io::Error> {
    let item = match req {
        ws::Frame::Ping(msg) => ws::Message::Pong(msg),
        ws::Frame::Text(text) => {
            ws::Message::Text(String::from_utf8(Vec::from(text.as_ref())).unwrap())
        }
        ws::Frame::Binary(bin) => ws::Message::Binary(bin),
        ws::Frame::Close(reason) => ws::Message::Close(reason),
        _ => ws::Message::Close(None),
    };
    Ok(Some(item))
}

#[ntex::test]
async fn test_simple() {
    let mut srv = test_server(|| {
        HttpService::build()
            .upgrade(|(req, mut framed): (Request, Framed<_, _>)| {
                async move {
                    let res = handshake_response(req.head()).finish();
                    // send handshake response
                    framed
                        .send(h1::Message::Item((res.drop_body(), BodySize::None)))
                        .await?;

                    // start websocket service
                    let framed = framed.into_framed(ws::Codec::new());
                    Dispatcher::new(framed, ws_service).await
                }
            })
            .finish(|_| ok::<_, io::Error>(Response::NotFound()))
            .tcp()
    });

    // client service
    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text("text".to_string()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Text(Bytes::from_static(b"text")));

    framed
        .send(ws::Message::Binary("text".into()))
        .await
        .unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Binary(Bytes::from_static(b"text")));

    framed.send(ws::Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Pong("text".to_string().into()));

    framed
        .send(ws::Message::Close(Some(ws::CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, ws::Frame::Close(Some(ws::CloseCode::Normal.into())));
}
