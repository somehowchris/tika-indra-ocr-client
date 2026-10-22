//! Bounded HTTP/UTF-8 transport shared by the Tika bridge and its measurement
//! harness. There is no OCR implementation or network policy in this module.

use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, Write},
    path::Path,
};

pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
pub const MAX_HEADERS: usize = 16 * 1024;

/// Writes only validated request context. The caller owns environment Strings;
/// this operation uses fixed carrier storage and borrowed header text.
pub fn trace_headers(
    output: &mut impl Write,
    carrier: &indra_trace_context::Carrier,
) -> io::Result<()> {
    if let Some(parent) = carrier.parent() {
        write!(output, "traceparent: {parent}\r\n")?;
    }
    if let Some(state) = carrier.state() {
        write!(output, "tracestate: {state}\r\n")?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("configuration: {0}")]
    Argument(&'static str),
    #[error("backend protocol: {0}")]
    Protocol(&'static str),
    #[error("Indra returned HTTP {0}; inspect backend error metrics/logs")]
    Backend(u16),
}

pub struct Response {
    pub length: usize,
}

/// Read at most the supplied storage size. The caller checks CRLF and the
/// aggregate header budget; buffered bytes past this line remain unread.
pub fn bounded_line(reader: &mut impl BufRead, storage: &mut [u8]) -> io::Result<usize> {
    let mut used = 0;
    while used < storage.len() {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        let active = available.len().min(storage.len() - used);
        let count = available[..active]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(active, |position| position + 1);
        storage[used..used + count].copy_from_slice(&available[..count]);
        reader.consume(count);
        used += count;
        if storage[used - 1] == b'\n' {
            break;
        }
    }
    Ok(used)
}

fn parse_status(value: &str) -> Result<u16, Error> {
    let mut fields = value.split_whitespace();
    if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err(Error::Protocol("invalid HTTP version"));
    }
    fields
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|n| (100..600).contains(n))
        .ok_or(Error::Protocol("invalid HTTP status"))
}

pub fn ready_status(reader: &mut impl BufRead) -> Result<(), Error> {
    let mut line = [0; 1024];
    let length = bounded_line(reader, &mut line)?;
    if !line[..length].ends_with(b"\r\n") {
        return Err(Error::Protocol("truncated or malformed headers"));
    }
    let value = std::str::from_utf8(&line[..length - 2])
        .map_err(|_| Error::Protocol("non-ASCII/UTF-8 header"))?;
    match parse_status(value)? {
        200 => Ok(()),
        status => Err(Error::Backend(status)),
    }
}

pub fn headers(reader: &mut impl BufRead) -> Result<Response, Error> {
    let mut line = [0; MAX_HEADERS];
    let mut consumed = 0;
    let mut status = None;
    let mut length = None;
    let mut plain_text = false;
    loop {
        let remaining = MAX_HEADERS - consumed;
        if remaining == 0 {
            return Err(Error::Protocol("headers exceed 16 KiB"));
        }
        let read = bounded_line(reader, &mut line[..remaining])?;
        consumed += read;
        if read == 0 || !line[..read].ends_with(b"\r\n") {
            return Err(Error::Protocol("truncated or malformed headers"));
        }
        let value = std::str::from_utf8(&line[..read - 2])
            .map_err(|_| Error::Protocol("non-ASCII/UTF-8 header"))?;
        if status.is_none() {
            status = Some(parse_status(value)?);
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
        length: length.ok_or(Error::Protocol("missing Content-Length"))?,
    })
}

/// Carries only an incomplete codepoint between borrowed input chunks. Complete
/// UTF-8 spans are written directly from the reader's existing buffer.
pub struct Utf8Stream {
    pending: [u8; 4],
    used: usize,
}

impl Default for Utf8Stream {
    fn default() -> Self {
        Self::new()
    }
}

impl Utf8Stream {
    pub const fn new() -> Self {
        Self {
            pending: [0; 4],
            used: 0,
        }
    }

    pub fn write(&mut self, mut bytes: &[u8], output: &mut impl Write) -> Result<(), Error> {
        if self.used > 0 {
            let expected = match self.pending[0] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(Error::Protocol("response is not valid UTF-8")),
            };
            let count = (expected - self.used).min(bytes.len());
            self.pending[self.used..self.used + count].copy_from_slice(&bytes[..count]);
            self.used += count;
            bytes = &bytes[count..];
            match std::str::from_utf8(&self.pending[..self.used]) {
                Ok(_) => {
                    output.write_all(&self.pending[..self.used])?;
                    self.used = 0;
                }
                Err(error) if error.error_len().is_none() => return Ok(()),
                Err(_) => return Err(Error::Protocol("response is not valid UTF-8")),
            }
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => output.write_all(bytes)?,
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                output.write_all(&bytes[..valid])?;
                let tail = &bytes[valid..];
                debug_assert!(
                    tail.len() <= 3,
                    "a UTF-8 partial codepoint has at most three bytes"
                );
                self.pending[..tail.len()].copy_from_slice(tail);
                self.used = tail.len();
            }
            Err(_) => return Err(Error::Protocol("response is not valid UTF-8")),
        }
        Ok(())
    }

    pub const fn finish(&self) -> Result<(), Error> {
        if self.used == 0 {
            Ok(())
        } else {
            Err(Error::Protocol("response is not valid UTF-8"))
        }
    }
}

pub fn stream_utf8(
    reader: &mut impl BufRead,
    length: usize,
    output: &mut impl Write,
) -> Result<(), Error> {
    if length > MAX_OUTPUT {
        return Err(Error::Protocol("invalid/excessive Content-Length"));
    }
    let mut remaining = length;
    let mut text = Utf8Stream::new();
    while remaining > 0 {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Err(Error::Protocol("response ended before Content-Length"));
        }
        let count = bytes.len().min(remaining);
        text.write(&bytes[..count], output)?;
        reader.consume(count);
        remaining -= count;
    }
    text.finish()
}

/// Tika consumes OCR text through XHTML, where U+000C is not a valid XML 1.0
/// character. Omit only Indra's form-feed separator for the supported empty
/// `page_separator` setting at this boundary;
/// the native HTTP output and the byte-preserving UTF-8 reader stay unchanged.
pub fn stream_tika_text(
    reader: &mut impl BufRead,
    length: usize,
    output: &mut impl Write,
) -> Result<(), Error> {
    stream_utf8(reader, length, &mut TikaText(output))
}

struct TikaText<W>(W);

impl<W: Write> Write for TikaText<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match bytes.iter().position(|&byte| byte == 0x0c) {
            // Consume the empty page marker; surrounding line endings remain.
            // Forward every other span with the writer's short-write contract.
            Some(0) => Ok(1),
            Some(end) => self.0.write(&bytes[..end]),
            None => self.0.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Tika opens the result only after a successful process exit. Any returned
/// error removes this process's partial file. create_new preserves prior files.
pub fn save_response(reader: &mut impl BufRead, destination: &Path) -> Result<(), Error> {
    let parsed = headers(reader)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut output = Output {
        file: Some(file),
        destination,
        complete: false,
    };
    stream_tika_text(
        reader,
        parsed.length,
        output.file.as_mut().expect("owned new output"),
    )?;
    output.file.as_mut().expect("owned new output").flush()?;
    drop(output.file.take());
    output.complete = true;
    Ok(())
}

struct Output<'a> {
    file: Option<File>,
    destination: &'a Path,
    complete: bool,
}

impl Drop for Output<'_> {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.complete
            && let Err(error) = std::fs::remove_file(self.destination)
        {
            eprintln!("indra-tesseract: failed to remove partial output: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufReader, Cursor},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    #[test]
    fn every_short_chunk_boundary_preserves_multilingual_utf8_exactly() {
        let source = "Zürich — café. 日本語 🧪\n".as_bytes();
        for width in 1..=16 {
            let mut input = BufReader::with_capacity(width, Cursor::new(source));
            let mut result = [0; 128];
            stream_utf8(&mut input, source.len(), &mut Cursor::new(&mut result[..]))
                .expect("valid chunks");
            assert_eq!(&result[..source.len()], source, "chunk width {width}");
        }
    }

    #[test]
    fn tika_page_separators_are_xml_safe_at_every_utf8_chunk_boundary() {
        let source = "\u{c}Zürich — 日本語 🧪 �\n\u{c}\nSECOND\u{c}\u{c}".as_bytes();
        let expected = "Zürich — 日本語 🧪 �\n\nSECOND".as_bytes();
        for width in 1..=16 {
            let mut input = BufReader::with_capacity(width, Cursor::new(source));
            let mut result = [0; 128];
            stream_tika_text(&mut input, source.len(), &mut Cursor::new(&mut result[..]))
                .expect("valid Tika text");
            assert_eq!(&result[..expected.len()], expected, "chunk width {width}");
        }
        let mut small = [0; 1];
        assert!(matches!(
            stream_tika_text(
                &mut Cursor::new(b"a\x0cb"),
                3,
                &mut Cursor::new(&mut small[..])
            ),
            Err(Error::Io(_))
        ));
        assert_eq!(small, *b"a");
    }

    #[test]
    fn invalid_utf8_truncation_and_writer_failures_are_explicit() {
        for source in [
            &b"\xf0\x9f\xa7"[..],
            &b"\xc0\xaf"[..],
            &b"\xed\xa0\x80"[..],
            &b"\xf4\x90\x80\x80"[..],
            &b"\xe2A"[..],
            &b"\x80"[..],
        ] {
            for width in 1..=4 {
                let mut input = BufReader::with_capacity(width, Cursor::new(source));
                assert!(matches!(
                    stream_utf8(&mut input, source.len(), &mut io::sink()),
                    Err(Error::Protocol("response is not valid UTF-8"))
                ));
            }
        }
        assert!(matches!(
            stream_utf8(&mut Cursor::new(b"short"), 6, &mut io::sink()),
            Err(Error::Protocol("response ended before Content-Length"))
        ));
        let mut full = [0; 1];
        assert!(matches!(
            stream_utf8(&mut Cursor::new(b"abc"), 3, &mut Cursor::new(&mut full[..])),
            Err(Error::Io(_))
        ));
        let mut trailing = Cursor::new(b"textignored");
        stream_utf8(&mut trailing, 4, &mut io::sink()).expect("exact declared length");
        assert_eq!(trailing.position(), 4);
    }

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("remove owned test directory");
        }
    }

    fn directory() -> Directory {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "indra-output-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create isolated output directory");
        Directory(path)
    }

    #[test]
    fn transactional_stream_removes_partial_and_preserves_existing_files() {
        let directory = directory();
        let output = directory.0.join("result.txt");
        for response in [
            &b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\n\r\nshort"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\na\xf0\x9f\xa7x"[..],
        ] {
            let mut reader = BufReader::with_capacity(1, Cursor::new(response));
            assert!(save_response(&mut reader, &output).is_err());
            assert!(!output.exists(), "failure removes streamed partial output");
        }
        let good = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\n\r\ntext";
        std::fs::write(&output, b"keep existing").expect("existing result");
        assert!(save_response(&mut Cursor::new(good), &output).is_err());
        assert_eq!(
            std::fs::read_to_string(&output).expect("existing bytes"),
            "keep existing"
        );
        std::fs::remove_file(&output).expect("remove owned existing fixture");
        save_response(&mut Cursor::new(good), &output).expect("complete text");
        assert_eq!(std::fs::read_to_string(&output).expect("new bytes"), "text");
    }

    #[test]
    fn saved_tika_result_maps_only_the_native_page_separator() {
        let directory = directory();
        let output = directory.0.join("two-pages.txt");
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 33\r\n\r\nINVOICE 12345\n\x0c\nSECOND PAGE 67890";
        save_response(
            &mut BufReader::with_capacity(1, Cursor::new(response)),
            &output,
        )
        .expect("complete two-page Tika result");
        assert_eq!(
            std::fs::read_to_string(output).expect("saved bytes"),
            "INVOICE 12345\n\nSECOND PAGE 67890"
        );
    }

    #[test]
    fn readiness_accepts_json_status_without_applying_ocr_media_rules() {
        let json = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\n\r\n{\"phase\":\"ready\"}";
        ready_status(&mut Cursor::new(json)).expect("Rust lifecycle JSON response");
        assert!(matches!(
            ready_status(&mut Cursor::new(b"HTTP/1.1 503 Busy\r\n")),
            Err(Error::Backend(503))
        ));
    }
}
