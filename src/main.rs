//! Deliberately narrow Tika 3.3.2 contract, not a general Tesseract emulator.
//! Input and output are streamed with fixed UTF-8 carry storage. Tika observes
//! a result only after successful exit; returned errors remove partial output.

use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use tika_indra_ocr_client::{Error, bounded_line, ready_status, save_response};

const MAX_INPUT: u64 = 20 * 1024 * 1024;
static BANNER: &str = "indra-tesseract 0.1.0 (Tika 3.3.2 CLI adapter)";

enum Command {
    Probe,
    Ready { tika: bool },
    TikaServerReady,
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
    if input == "--ready" || input == "--tika-ready" || input == "--tika-server-ready" {
        return if args.next().is_none() {
            if input == "--tika-server-ready" {
                Ok(Command::TikaServerReady)
            } else {
                Ok(Command::Ready {
                    tika: input == "--tika-ready",
                })
            }
        } else {
            Err(Error::Argument("readiness probes take no arguments"))
        };
    }
    if input.as_encoded_bytes().starts_with(b"-") {
        return Err(Error::Argument(
            "supported commands: no arguments, --version, --ready, --tika-ready, --tika-server-ready, INPUT OUTPUT --psm 3 ... txt",
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
    save_response(&mut BufReader::with_capacity(8192, connection), &output)
}

fn tika_ready() -> Result<(), Error> {
    let mut connection = Connection::tika()?;
    connection
        .write_all(b"GET /version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    // A health probe needs only the bounded HTTP status line. Tika may use
    // chunked encoding, whereas the image-service contract requires a length.
    let mut line = [0; 1024];
    let length = bounded_line(&mut BufReader::new(connection), &mut line)?;
    let line = &line[..length];
    if !line.ends_with(b"\r\n") {
        return Err(Error::Protocol("invalid Tika health response"));
    }
    let line =
        std::str::from_utf8(line).map_err(|_| Error::Protocol("invalid Tika status encoding"))?;
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
            // Readiness is the endpoint's status, independent of whether the
            // backend describes it as plain text or structured lifecycle JSON.
            ready_status(&mut BufReader::new(connection))?;
            if tika {
                tika_ready()?;
            }
        }
        Command::TikaServerReady => tika_ready()?,
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
    use tika_indra_ocr_client::{MAX_HEADERS, headers};

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
            &["a", "b", "--psm", "0", "txt"][..],
            &["a", "b", "--psm", "3", "hocr"][..],
            &["a", "b", "--psm", "3", "-l", "eng", "txt"][..],
            &[
                "a",
                "b",
                "--psm",
                "3",
                "-c",
                "preserve_interword_spaces=1",
                "txt",
            ][..],
        ] {
            assert!(parse_args(args.iter().map(OsString::from)).is_err());
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
        let mut huge = [b'a'; MAX_HEADERS * 2];
        let prefix = b"HTTP/1.1 200 OK\r\nX-Large: ";
        huge[..prefix.len()].copy_from_slice(prefix);
        let mut reader = io::Cursor::new(&huge[..]);
        assert!(headers(&mut reader).is_err());
        assert!(reader.position() <= MAX_HEADERS as u64);
    }
}
