use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::{
    hint::black_box,
    io::{self, BufRead, Read},
    time::Duration,
};
use tika_indra_ocr_client::{headers, stream_utf8};

static ASCII: [u8; 16 * 1024] = [b'a'; 16 * 1024];
static UNIT: &[u8] = "Δοκιμή 日本語 🧪\n".as_bytes();
const UNICODE_LENGTH: usize = "Δοκιμή 日本語 🧪\n".len() * 512;
static UNICODE: [u8; UNICODE_LENGTH] = repeated();
static HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 16384\r\nConnection: close\r\n\r\n";

const fn repeated() -> [u8; UNICODE_LENGTH] {
    let unit = "Δοκιμή 日本語 🧪\n".as_bytes();
    let mut data = [0; UNICODE_LENGTH];
    let mut index = 0;
    while index < data.len() {
        data[index] = unit[index % unit.len()];
        index += 1;
    }
    data
}

/// A borrowed source makes chunk boundaries deterministic without measuring
/// the standard library's one-time BufReader allocation or network/file I/O.
struct Chunks<'a> {
    bytes: &'a [u8],
    width: usize,
}
impl BufRead for Chunks<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Ok(&self.bytes[..self.bytes.len().min(self.width)])
    }
    fn consume(&mut self, count: usize) {
        self.bytes = &self.bytes[count..];
    }
}
impl Read for Chunks<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let data = self.fill_buf()?;
        let count = data.len().min(out.len());
        out[..count].copy_from_slice(&data[..count]);
        self.consume(count);
        Ok(count)
    }
}

fn streaming(criterion: &mut Criterion) {
    assert!(std::str::from_utf8(&UNICODE).is_ok());
    assert_eq!(UNICODE.len(), UNIT.len() * 512);
    criterion.bench_function("http_headers/fixed_storage", |bench| {
        bench.iter(|| headers(&mut io::Cursor::new(black_box(HEADERS))).expect("valid headers"))
    });
    let mut group = criterion.benchmark_group("utf8_stream");
    for (name, source) in [("ascii", &ASCII[..]), ("multilingual", &UNICODE[..])] {
        group.throughput(Throughput::Bytes(source.len() as u64));
        for width in [7, 8192] {
            group.bench_with_input(BenchmarkId::new(name, width), &width, |bench, width| {
                bench.iter(|| {
                    stream_utf8(
                        &mut Chunks {
                            bytes: black_box(source),
                            width: *width,
                        },
                        source.len(),
                        &mut io::sink(),
                    )
                    .expect("valid UTF-8")
                });
            });
        }
    }
    group.finish();
}

criterion_group! { name = benches; config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3)); targets = streaming }
criterion_main!(benches);
