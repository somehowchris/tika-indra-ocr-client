use super::{Failure, check};
use axum::http::{Method, Request, header};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use std::{net::SocketAddr, time::Duration};

pub(super) struct Reply {
    pub status: u16,
    pub bytes: [u8; 16_384],
    pub used: usize,
}

/// The connection future is owned by this bounded exchange; it is never
/// detached. The response has a fixed cap and must reach an actual clean EOF.
pub(super) async fn request(
    address: SocketAddr,
    path: &'static str,
    data: &'static [u8],
    headers: &[(&'static str, &'static str)],
) -> Result<Reply, Failure> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let socket = tokio::net::TcpStream::connect(address).await?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(socket)).await?;
        let mut request = Request::builder()
            .method(if path == "/tika" {
                Method::PUT
            } else {
                Method::GET
            })
            .uri(path)
            .header(header::HOST, address.to_string())
            .header(header::ACCEPT, "text/plain")
            .header(header::CONNECTION, "close")
            .header(header::CONTENT_TYPE, "image/tiff");
        for &(key, value) in headers {
            request = request.header(key, value);
        }
        let request = request.body(Full::new(Bytes::from_static(data)))?;
        let exchange = async {
            let response = sender.send_request(request).await?;
            drop(sender);
            let mut reply = Reply {
                status: response.status().as_u16(),
                bytes: [0; 16_384],
                used: 0,
            };
            let mut body = response.into_body();
            while let Some(frame) = body.frame().await {
                let frame = frame?;
                if let Ok(data) = frame.into_data() {
                    let end = reply
                        .used
                        .checked_add(data.len())
                        .ok_or(Failure::Gate("response length overflow"))?;
                    check(end <= reply.bytes.len(), "response exceeds fixed test cap")?;
                    reply.bytes[reply.used..end].copy_from_slice(&data);
                    reply.used = end;
                }
            }
            Ok::<_, Failure>(reply)
        };
        let (response, transport) = tokio::join!(exchange, connection);
        transport?;
        response
    })
    .await
    .map_err(|_| Failure::Gate("HTTP exchange deadline"))?
}
