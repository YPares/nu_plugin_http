use std::str::FromStr;

use bytes::Bytes;

use http::response::Parts;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::Receiver;

use hyper::Error;
use hyper_util::rt::TokioIo;

use crate::bridge;

pub struct HTTPPlugin {
    pub runtime: Runtime,
}

impl HTTPPlugin {
    pub fn new() -> Self {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");
        HTTPPlugin { runtime }
    }
}

impl HTTPPlugin {
    pub async fn request(
        &self,
        method: String,
        url: String,
        body: bridge::Body,
    ) -> Result<(Parts, Receiver<Result<Bytes, Error>>), Box<dyn std::error::Error>> {
        // TODO: bring back TCP support (and TLS :/)
        eprintln!("hello world: {:?}", &url);

        let stream = tokio::net::UnixStream::connect(url)
            .await
            .expect("Failed to connect to server");
        let io = TokioIo::new(stream);

        use http_body_util::BodyExt;
        use hyper::client::conn;
        use hyper::Request;

        let (mut request_sender, connection) = conn::http1::handshake(io).await.unwrap();

        // spawn a task to poll the connection and drive the HTTP state
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("Error in connection: {}", e);
            }
        });

        let method = http::method::Method::from_str(&method.to_uppercase())?;
        let body = body.to_http_body();
        let req = Request::builder().method(method).body(body)?;

        let res = request_sender.send_request(req).await?;
        let (meta, mut body) = res.into_parts();

        let (tx, rx) = tokio::sync::mpsc::channel(32);

        tokio::spawn(async move {
            while let Some(next) = body.frame().await {
                match next {
                    Ok(frame) => {
                        if let Some(chunk) = frame.data_ref() {
                            if tx.send(Ok(chunk.clone())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        if tx.send(Err(e)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok((meta, rx))
    }
}
