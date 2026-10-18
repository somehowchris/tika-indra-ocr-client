//! Diagnostic HTTP replay; production parsing/UTF-8 code is reused unchanged.
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tika_indra_ocr_client::{Error, MAX_HEADERS, MAX_OUTPUT, headers, ready_status, stream_utf8};

const REQUEST_SECONDS: u64 = 420;
struct Connection {
    socket: TcpStream,
    wire: File,
    due: Instant,
    received: usize,
}
impl Connection {
    fn remaining(&self) -> io::Result<Duration> {
        self.due
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "reference HTTP deadline"))
    }
}
impl Read for Connection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        let n = self.socket.read(bytes)?;
        self.received = self
            .received
            .checked_add(n)
            .ok_or_else(|| io::Error::other("wire length overflow"))?;
        if self.received > MAX_HEADERS + MAX_OUTPUT {
            return Err(io::Error::other("wire capture cap"));
        }
        self.wire.write_all(&bytes[..n])?;
        Ok(n)
    }
}
impl Write for Connection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        self.socket.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}
fn receive(reader: &mut impl BufRead, output: &mut impl Write) -> Result<(), Error> {
    let response = headers(reader)?;
    stream_utf8(reader, response.length, output)?;
    if !reader.fill_buf()?.is_empty() {
        return Err(Error::Protocol("bytes after complete response"));
    }
    Ok(())
}
fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
fn address(value: &std::ffi::OsStr) -> Result<SocketAddr, Error> {
    let address = value
        .to_str()
        .and_then(|s| s.parse::<SocketAddr>().ok())
        .ok_or(Error::Argument("literal address required"))?;
    if !address.ip().is_loopback() {
        return Err(Error::Argument("loopback required"));
    }
    Ok(address)
}
fn main() -> Result<(), Error> {
    let mut arguments = env::args_os().skip(1);
    let operation = arguments
        .next()
        .ok_or(Error::Argument("request|ready|hold|copy required"))?;
    match operation.to_str() {
        Some("hold") => {
            let marker = arguments
                .next()
                .ok_or(Error::Argument("admission marker required"))?;
            let binary = arguments.next().ok_or(Error::Argument("server required"))?;
            let due = Instant::now() + Duration::from_secs(30);
            loop {
                match fs::symlink_metadata(&marker) {
                    Ok(meta)
                        if meta.is_file() && !meta.file_type().is_symlink() && meta.len() == 9 =>
                    {
                        let mut bytes = [0_u8; 10];
                        let mut file = File::open(&marker)?;
                        file.read_exact(&mut bytes[..9])?;
                        if &bytes[..9] != b"admitted\n" || file.read(&mut bytes[9..])? != 0 {
                            return Err(Error::Protocol("invalid admission"));
                        }
                        break;
                    }
                    Ok(_) => return Err(Error::Protocol("invalid admission file")),
                    Err(e) if e.kind() == io::ErrorKind::NotFound && Instant::now() < due => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(Command::new(binary)
                .args(arguments)
                .env_clear()
                .env("LANG", "C.UTF-8")
                .exec()
                .into())
        }
        Some("copy") => {
            let path = arguments.next().ok_or(Error::Argument("file required"))?;
            if arguments.next().is_some() {
                return Err(Error::Argument("extra argument"));
            }
            let mut file = File::open(path)?;
            let size = file.metadata()?.len();
            if !file.metadata()?.is_file() || size > 1024 * 1024 * 1024 {
                return Err(Error::Argument("copy file cap"));
            }
            if io::copy(&mut (&mut file).take(size), &mut io::stdout().lock())? != size {
                return Err(Error::Protocol("truncated copy"));
            }
            Ok(())
        }
        Some("ready") => {
            let target = address(
                &arguments
                    .next()
                    .ok_or(Error::Argument("address required"))?,
            )?;
            if arguments.next().is_some() {
                return Err(Error::Argument("extra argument"));
            }
            let mut socket = TcpStream::connect_timeout(&target, Duration::from_secs(1))?;
            socket.set_read_timeout(Some(Duration::from_secs(1)))?;
            socket.set_write_timeout(Some(Duration::from_secs(1)))?;
            socket.write_all(
                b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )?;
            ready_status(&mut BufReader::new(socket))
        }
        Some("request") => {
            let [target, input, body, wire] = std::array::from_fn(|_| arguments.next());
            let target = address(&target.ok_or(Error::Argument("address required"))?)?;
            let input = input.ok_or(Error::Argument("input required"))?;
            let body = body.ok_or(Error::Argument("output required"))?;
            let wire = wire.ok_or(Error::Argument("wire output required"))?;
            if arguments.next().is_some() {
                return Err(Error::Argument("extra argument"));
            }
            let mut file = File::open(input)?;
            let size = file.metadata()?.len();
            if !file.metadata()?.is_file() || size == 0 || size > 20 * 1024 * 1024 {
                return Err(Error::Argument("input cap"));
            }
            let mut output = create(Path::new(&body))?;
            let mut connection = Connection {
                socket: TcpStream::connect_timeout(&target, Duration::from_secs(2))?,
                wire: create(Path::new(&wire))?,
                due: Instant::now() + Duration::from_secs(REQUEST_SECONDS),
                received: 0,
            };
            write!(
                connection,
                "POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
            )?;
            if io::copy(&mut (&mut file).take(size), &mut connection)? != size {
                return Err(Error::Protocol("truncated input"));
            }
            let mut reader = BufReader::new(connection);
            let result = receive(&mut reader, &mut output);
            // Retain bounded error responses too; primary and drain errors remain distinct.
            let drained = io::copy(&mut reader, &mut io::sink());
            if let Err(error) = &drained {
                eprintln!("HTTP drain failed: {error}");
            }
            reader.get_mut().wire.sync_all()?;
            output.sync_all()?;
            result?;
            drained?;
            Ok(())
        }
        _ => Err(Error::Argument("request|ready|hold|copy required")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_preserves_utf8_form_feed_and_rejects_incomplete_or_ambiguous_wire() {
        let mut output = [0_u8; 5];
        receive(&mut io::Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\na\n\x0c\nb"),&mut io::Cursor::new(&mut output[..])).unwrap();
        assert_eq!(&output, b"a\n\x0c\nb");
        for wire in [b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\na".as_slice(),b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1\r\n\r\naextra",b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\na"] {
            assert!(receive(&mut io::Cursor::new(wire),&mut io::sink()).is_err());
        }
    }
}
