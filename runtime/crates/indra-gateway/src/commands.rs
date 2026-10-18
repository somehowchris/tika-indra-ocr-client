use crate::{configuration::ProbeKind, error::Error};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

pub fn probe(listen: SocketAddr, kind: ProbeKind) -> Result<(), Error> {
    exchange(
        listen,
        match kind {
            ProbeKind::Ready => {
                b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            }
            ProbeKind::Health => {
                b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            }
            ProbeKind::Startup => {
                b"GET /startupz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            }
        },
    )
}

pub fn drain(listen: SocketAddr) -> Result<(), Error> {
    exchange(listen, b"POST /drain HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

fn exchange(mut listen: SocketAddr, request: &[u8]) -> Result<(), Error> {
    if listen.ip().is_unspecified() {
        listen.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let due = Instant::now() + Duration::from_secs(2);
    let mut socket =
        TcpStream::connect_timeout(&listen, due.saturating_duration_since(Instant::now()))
            .map_err(|_| Error::Probe)?;
    socket
        .set_write_timeout(Some(due.saturating_duration_since(Instant::now())))
        .map_err(|_| Error::Probe)?;
    socket.write_all(request).map_err(|_| Error::Probe)?;
    let mut line = [0; 128];
    for end in 1..=line.len() {
        let remaining = due.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Probe);
        }
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|_| Error::Probe)?;
        socket
            .read_exact(&mut line[end - 1..end])
            .map_err(|_| Error::Probe)?;
        if line[end - 1] == b'\n' {
            return if line[..end].starts_with(b"HTTP/1.1 200 ")
                || line[..end].starts_with(b"HTTP/1.0 200 ")
            {
                Ok(())
            } else {
                Err(Error::Probe)
            };
        }
    }
    Err(Error::Probe)
}
