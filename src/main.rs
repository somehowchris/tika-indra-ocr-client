//! Deliberately narrow Tika 3.3.2 contract, not a general Tesseract emulator.
//! Input is streamed; a bounded response is buffered once to validate UTF-8
//! before creating the output file. Transport and buffer copies are explicit.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const MAX_INPUT: u64 = 20 * 1024 * 1024;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_HEADERS: usize = 16 * 1024;
const BANNER: &str = "indra-tesseract 0.1.0 (Tika 3.3.2 CLI adapter)";

#[derive(Debug)]
enum Error {
    Io(io::Error),
    Argument(&'static str),
    Protocol(&'static str),
    Backend(u16),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O: {error}"),
            Self::Argument(message) => write!(f, "configuration: {message}"),
            Self::Protocol(message) => write!(f, "backend protocol: {message}"),
            Self::Backend(status) => write!(
                f,
                "Indra returned HTTP {status}; inspect backend error metrics/logs"
            ),
        }
    }
}

enum Command {
    Probe,
    Ready { tika: bool },
    Ocr { input: PathBuf, output: PathBuf },
}

fn parse_args(mut args: impl Iterator<Item = OsString>) -> Result<Command, Error> {
    let Some(input) = args.next() else {
        return Ok(Command::Probe);
    };
    if input == "--version" {
        return if args.next().is_none() {
            Ok(Command::Probe)
        } else {
            Err(Error::Argument("--version takes no arguments"))
        };
    }
    if input == "--ready" || input == "--tika-ready" {
        return if args.next().is_none() {
            Ok(Command::Ready {
                tika: input == "--tika-ready",
            })
        } else {
            Err(Error::Argument("readiness probes take no arguments"))
        };
    }
    if input.as_encoded_bytes().starts_with(b"-") {
        return Err(Error::Argument(
            "supported commands: no arguments, --version, --ready, --tika-ready, INPUT OUTPUT --psm 3 ... txt",
        ));
    }
    let mut output = args.next().ok_or(Error::Argument("missing output base"))?;
    if output == "stdout" || output == "-" {
        return Err(Error::Argument("an output file base is required"));
    }
    let mut psm = false;
    let mut text = false;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--psm") if !psm && !text => {
                if args.next().as_deref() != Some(std::ffi::OsStr::new("3")) {
                    return Err(Error::Argument(
                        "only --psm 3 is supported; Indra uses its own automatic layout and orientation",
                    ));
                }
                psm = true;
            }
            Some("-c") if !text => {
                let value = args.next().ok_or(Error::Argument("missing -c value"))?;
                if value != "page_separator=" && value != "preserve_interword_spaces=0" {
                    return Err(Error::Argument(
                        "supported -c values: page_separator= and preserve_interword_spaces=0",
                    ));
                }
            }
            Some("txt") if !text => text = true,
            Some("-l") => {
                return Err(Error::Argument(
                    "set Tika language to empty: Indra selects script models automatically, without Tesseract language hints",
                ));
            }
            _ => {
                return Err(Error::Argument(
                    "unsupported OCR flag/output format; only the documented Tika plain-text contract is implemented",
                ));
            }
        }
    }
    if !psm || !text {
        return Err(Error::Argument(
            "explicit --psm 3 and trailing txt are required",
        ));
    }
    output.push(".txt");
    Ok(Command::Ocr {
        input: PathBuf::from(input),
        output: PathBuf::from(output),
    })
}

fn env_number(name: &str, default: u64, maximum: u64) -> Result<u64, Error> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= maximum)
            .ok_or(Error::Argument(
                "invalid timeout: use a positive bounded integer",
            )),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(Error::Argument("timeout must be Unicode")),
    }
}

/// Every underlying read/write uses the remaining total deadline. A peer
/// cannot extend it indefinitely by sending one byte before each timeout.
struct Connection {
    socket: TcpStream,
    deadline: Instant,
}

impl Connection {
    fn open(ready: bool) -> Result<Self, Error> {
        let address = match env::var("INDRA_ADDR") {
            Ok(value) => value
                .parse::<SocketAddr>()
                .map_err(|_| Error::Argument("INDRA_ADDR must be a literal IP:port; no DNS"))?,
            Err(env::VarError::NotPresent) => SocketAddr::from(([127, 0, 0, 1], 8081)),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(Error::Argument("INDRA_ADDR must be Unicode"));
            }
        };
        if !address.ip().is_loopback() {
            return Err(Error::Argument(
                "INDRA_ADDR must be loopback; deploy Tika and Indra in one network namespace",
            ));
        }
        let total = if ready {
            2
        } else {
            env_number("INDRA_TIMEOUT_SECONDS", 70, 300)?
        };
        let connect = env_number("INDRA_CONNECT_TIMEOUT_MS", 1000, 10_000)?;
        Self::connect(address, total, connect)
    }

    fn connect(address: SocketAddr, total: u64, connect: u64) -> Result<Self, Error> {
        let deadline = Instant::now() + Duration::from_secs(total);
        let socket = TcpStream::connect_timeout(
            &address,
            Duration::from_millis(connect).min(Duration::from_secs(total)),
        )?;
        socket.set_nodelay(true)?;
        Ok(Self { socket, deadline })
    }

    fn tika() -> Result<Self, Error> {
        let address = match env::var("TIKA_ADDR") {
            Ok(value) => value
                .parse::<SocketAddr>()
                .map_err(|_| Error::Argument("TIKA_ADDR must be a literal IP:port"))?,
            Err(env::VarError::NotPresent) => SocketAddr::from(([127, 0, 0, 1], 9998)),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(Error::Argument("TIKA_ADDR must be Unicode"));
            }
        };
        if !address.ip().is_loopback() {
            return Err(Error::Argument("TIKA_ADDR must be loopback"));
        }
        Self::connect(address, 2, 1000)
    }

    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "total Indra deadline exceeded"))
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        self.socket.read(buffer)
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        self.socket.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

struct Response {
    status: u16,
    length: usize,
}

fn headers(reader: &mut impl BufRead) -> Result<Response, Error> {
    let mut line = Vec::with_capacity(256);
    let mut consumed = 0;
    let mut status = None;
    let mut length = None;
    let mut plain_text = false;
    loop {
        line.clear();
        let remaining = MAX_HEADERS - consumed;
        if remaining == 0 {
            return Err(Error::Protocol("headers exceed 16 KiB"));
        }
        let read = reader.take(remaining as u64).read_until(b'\n', &mut line)?;
        consumed += read;
        if read == 0 || !line.ends_with(b"\r\n") {
            return Err(Error::Protocol("truncated or malformed headers"));
        }
        let value = std::str::from_utf8(&line[..line.len() - 2])
            .map_err(|_| Error::Protocol("non-ASCII/UTF-8 header"))?;
        if status.is_none() {
            let mut fields = value.split_whitespace();
            if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
                return Err(Error::Protocol("invalid HTTP version"));
            }
            status = Some(
                fields
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|n| (100..600).contains(n))
                    .ok_or(Error::Protocol("invalid HTTP status"))?,
            );
        } else if value.is_empty() {
            break;
        } else {
            let (key, value) = value
                .split_once(':')
                .ok_or(Error::Protocol("malformed header"))?;
            if key.eq_ignore_ascii_case("content-length") {
                if length.is_some() {
                    return Err(Error::Protocol("duplicate Content-Length"));
                }
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|n| *n <= MAX_OUTPUT)
                        .ok_or(Error::Protocol("invalid/excessive Content-Length"))?,
                );
            } else if key.eq_ignore_ascii_case("transfer-encoding") {
                return Err(Error::Protocol(
                    "Transfer-Encoding is unsupported; backend must send Content-Length",
                ));
            } else if key.eq_ignore_ascii_case("content-type") {
                plain_text = value
                    .trim()
                    .split(';')
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("text/plain"));
            }
        }
    }
    let status = status.ok_or(Error::Protocol("missing HTTP status"))?;
    if status != 200 {
        return Err(Error::Backend(status));
    }
    if !plain_text {
        return Err(Error::Protocol("expected text/plain"));
    }
    Ok(Response {
        status,
        length: length.ok_or(Error::Protocol("missing Content-Length"))?,
    })
}

fn response(connection: Connection) -> Result<Vec<u8>, Error> {
    let mut reader = BufReader::with_capacity(8192, connection);
    let parsed = headers(&mut reader)?;
    debug_assert_eq!(parsed.status, 200);
    let mut body = Vec::with_capacity(parsed.length);
    reader.take(parsed.length as u64).read_to_end(&mut body)?;
    if body.len() != parsed.length {
        return Err(Error::Protocol("response ended before Content-Length"));
    }
    std::str::from_utf8(&body).map_err(|_| Error::Protocol("response is not valid UTF-8"))?;
    Ok(body)
}

fn ocr(input: PathBuf, output: PathBuf) -> Result<(), Error> {
    let mut file = File::open(input)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_INPUT {
        return Err(Error::Argument(
            "input must be a nonempty regular file no larger than 20 MiB",
        ));
    }
    let mut connection = Connection::open(false)?;
    write!(
        connection,
        "POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        metadata.len()
    )?;
    let sent = io::copy(&mut (&mut file).take(metadata.len()), &mut connection)?;
    if sent != metadata.len() {
        return Err(Error::Argument("input was truncated during upload"));
    }
    let text = response(connection)?;
    // Tika reads OUTPUT.txt only after this process exits successfully.
    // Do not overwrite an existing file or leave a partial result on failure.
    let mut result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    if let Err(error) = result.write_all(&text) {
        drop(result);
        if let Err(cleanup) = std::fs::remove_file(&output) {
            eprintln!("indra-tesseract: failed to remove partial output: {cleanup}");
        }
        return Err(Error::Io(error));
    }
    Ok(())
}

fn tika_ready() -> Result<(), Error> {
    let mut connection = Connection::tika()?;
    connection
        .write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    // A health probe needs only the bounded HTTP status line. Tika may use
    // chunked encoding, whereas the image-service contract requires a length.
    let mut line = Vec::with_capacity(64);
    BufReader::new(connection)
        .take(1024)
        .read_until(b'\n', &mut line)?;
    if !line.ends_with(b"\r\n") {
        return Err(Error::Protocol("invalid Tika health response"));
    }
    let line =
        std::str::from_utf8(&line).map_err(|_| Error::Protocol("invalid Tika status encoding"))?;
    let mut fields = line.split_whitespace();
    if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) || fields.next() != Some("200") {
        return Err(Error::Protocol("Tika /version is not ready"));
    }
    Ok(())
}

fn run() -> Result<(), Error> {
    match parse_args(env::args_os().skip(1))? {
        Command::Probe => println!("{BANNER}"),
        Command::Ready { tika } => {
            let mut connection = Connection::open(true)?;
            connection.write_all(
                b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )?;
            response(connection)?;
            if tika {
                tika_ready()?;
            }
        }
        Command::Ocr { input, output } => ocr(input, output)?,
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("indra-tesseract: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_tika_contract_and_appends_extension() {
        let args = [
            "/tmp/a.png",
            "/tmp/b",
            "--psm",
            "3",
            "-c",
            "page_separator=",
            "-c",
            "preserve_interword_spaces=0",
            "txt",
        ];
        match parse_args(args.into_iter().map(OsString::from)) {
            Ok(Command::Ocr { input, output }) => {
                assert_eq!(input, PathBuf::from("/tmp/a.png"));
                assert_eq!(output, PathBuf::from("/tmp/b.txt"));
            }
            _ => panic!("valid Tika command rejected"),
        }
    }

    #[test]
    fn rejects_semantically_unsupported_options() {
        for args in [
            vec!["a", "b", "--psm", "0", "txt"],
            vec!["a", "b", "--psm", "3", "hocr"],
            vec!["a", "b", "--psm", "3", "-l", "eng", "txt"],
            vec![
                "a",
                "b",
                "--psm",
                "3",
                "-c",
                "preserve_interword_spaces=1",
                "txt",
            ],
        ] {
            assert!(parse_args(args.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn rejects_unbounded_ambiguous_or_wrong_http_responses() {
        for value in [
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 999999999\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\n",
        ] {
            assert!(headers(&mut io::Cursor::new(value.as_bytes())).is_err());
        }
        let mut valid = io::Cursor::new(b"HTTP/1.0 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 42\r\n\r\n");
        assert_eq!(headers(&mut valid).expect("valid headers").length, 42);
    }

    #[test]
    fn rejects_oversized_header_without_consuming_unbounded_input() {
        let mut huge = b"HTTP/1.1 200 OK\r\nX-Large: ".to_vec();
        huge.extend(std::iter::repeat_n(b'a', MAX_HEADERS * 2));
        let mut reader = io::Cursor::new(huge);
        assert!(headers(&mut reader).is_err());
        assert!(reader.position() <= MAX_HEADERS as u64);
    }
}
