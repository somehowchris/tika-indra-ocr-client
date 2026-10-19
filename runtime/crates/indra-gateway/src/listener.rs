//! One task per accepted connection is bounded by the static semaphore. The
//! transport enforces first-header and total-connection deadlines without an
//! auxiliary task. HTTP/1 responses always close the connection.

use crate::{configuration::MAX_HEADERS, service::Shared};
use axum::{
    extract::connect_info::Connected,
    serve::{IncomingStream, Listener},
};
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::SemaphorePermit,
    time::{Instant, Interval},
};

pub struct LimitedListener {
    pub socket: TcpListener,
    pub shared: &'static Shared,
}

// Axum's Connected contract requires Clone for this fixed-size peer record.
#[derive(Clone)]
pub struct Peer(pub SocketAddr);
impl Connected<IncomingStream<'_, LimitedListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, LimitedListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl Listener for LimitedListener {
    type Io = TimedIo;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.socket.accept().await {
                Ok((socket, address)) => {
                    let Ok(permit) = self.shared.connections.try_acquire() else {
                        drop(socket);
                        continue;
                    };
                    if let Err(error) = socket.set_nodelay(true) {
                        tracing::warn!(event="socket_option", kind=?error.kind());
                    }
                    return (TimedIo::new(socket, permit, self.shared), address);
                }
                Err(error) => {
                    tracing::warn!(event="accept_failed", kind=?error.kind());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

pub struct TimedIo {
    socket: TcpStream,
    _permit: SemaphorePermit<'static>,
    timer: Interval,
    total_due: Instant,
    expired: bool,
    header_done: bool,
    header_bytes: usize,
    matched: usize,
}

impl TimedIo {
    fn new(socket: TcpStream, permit: SemaphorePermit<'static>, shared: &'static Shared) -> Self {
        let now = Instant::now();
        Self {
            socket,
            _permit: permit,
            timer: tokio::time::interval_at(
                now + Duration::from_secs(shared.settings.header_timeout_seconds),
                Duration::from_secs(3600),
            ),
            total_due: now
                + Duration::from_secs(
                    shared.settings.header_timeout_seconds
                        + shared.settings.request_timeout_seconds
                        + shared.settings.client_stall_seconds,
                ),
            expired: false,
            header_done: false,
            header_bytes: 0,
            matched: 0,
        }
    }
    fn deadline(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if self.expired || self.timer.poll_tick(cx).is_ready() {
            self.expired = true;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "gateway connection deadline",
            ))
        } else {
            Ok(())
        }
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Err(error) = this.deadline(cx) {
            return Poll::Ready(Err(error));
        }
        let previous = buf.filled().len();
        match Pin::new(&mut this.socket).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if !this.header_done {
                    for byte in &buf.filled()[previous..] {
                        this.header_bytes += 1;
                        if this.header_bytes > MAX_HEADERS {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "gateway header bound",
                            )));
                        }
                        this.matched = if *byte == b"\r\n\r\n"[this.matched] {
                            this.matched + 1
                        } else if *byte == b'\r' {
                            1
                        } else {
                            0
                        };
                        if this.matched == 4 {
                            this.header_done = true;
                            this.timer.reset_at(this.total_due);
                            break;
                        }
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl AsyncWrite for TimedIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Err(error) = this.deadline(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut this.socket).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Err(error) = this.deadline(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut this.socket).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_shutdown(cx)
    }
}
