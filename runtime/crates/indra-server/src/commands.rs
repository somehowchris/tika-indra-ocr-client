//! Operational commands use the executable itself; final images need no shell.

use crate::{configuration::ProbeKind, error::StartupError};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

pub fn probe(listen: SocketAddr, kind: ProbeKind) -> Result<(), StartupError> {
    let request: &[u8] = match kind {
        ProbeKind::Ready => b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        ProbeKind::Health => {
            b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        }
        ProbeKind::Startup => {
            b"GET /startupz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        }
    };
    exchange(listen, request)
}

pub fn drain(listen: SocketAddr) -> Result<(), StartupError> {
    exchange(listen, b"POST /drain HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

fn exchange(mut listen: SocketAddr, request: &[u8]) -> Result<(), StartupError> {
    if listen.ip().is_unspecified() {
        listen.set_ip(if listen.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        });
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut socket =
        TcpStream::connect_timeout(&listen, deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| StartupError::Probe)?;
    socket
        .set_write_timeout(Some(deadline.saturating_duration_since(Instant::now())))
        .map_err(|_| StartupError::Probe)?;
    socket.write_all(request).map_err(|_| StartupError::Probe)?;
    let mut line = [0_u8; 128];
    let mut length = 0;
    while length < line.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StartupError::Probe);
        }
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|_| StartupError::Probe)?;
        socket
            .read_exact(&mut line[length..length + 1])
            .map_err(|_| StartupError::Probe)?;
        length += 1;
        if line[length - 1] == b'\n' {
            return if line[..length].starts_with(b"HTTP/1.1 200 ")
                || line[..length].starts_with(b"HTTP/1.0 200 ")
            {
                Ok(())
            } else {
                Err(StartupError::Probe)
            };
        }
    }
    Err(StartupError::Probe)
}
