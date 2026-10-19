//! Bounded image transport compatible with the static Tika executable bridge.

use crate::error::ErrorCode;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, header},
};
use bytes::BytesMut;
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
static HEX: &[u8; 16] = b"0123456789abcdef";

/// Correlation only; never use this value as an authentication secret.
#[derive(Debug, Copy, Clone)]
pub struct RequestId([u8; 32]);

impl RequestId {
    pub fn next() -> Self {
        let sequence = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64);
        let mut id = [b'0'; 32];
        write_hex(timestamp, &mut id[..16]);
        write_hex(sequence, &mut id[16..]);
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("generated request ID is lowercase ASCII hex")
    }
}

fn write_hex(mut value: u64, target: &mut [u8]) {
    for byte in target.iter_mut().rev() {
        *byte = HEX[(value & 15) as usize];
        value >>= 4;
    }
}

pub fn set_common_headers(headers: &mut HeaderMap, request_id: &RequestId) {
    headers.insert(
        "x-request-id",
        HeaderValue::from_bytes(&request_id.0)
            .expect("generated request ID is a valid header value"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    // The bridge uses Connection: close. Early admission rejection must never
    // leave unread upload bytes available to another request on the connection.
    headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
}

pub fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    first.to_str().ok()
}

pub fn expected_length(headers: &HeaderMap, limit: usize) -> Result<usize, ErrorCode> {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return Err(ErrorCode::InvalidRequest);
    }
    let length = one_header(headers, "content-length").ok_or(ErrorCode::LengthRequired)?;
    if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ErrorCode::InvalidRequest);
    }
    let length = length.parse::<usize>().map_err(|_| ErrorCode::TooLarge)?;
    if length == 0 {
        return Err(ErrorCode::InvalidRequest);
    }
    if length > limit {
        return Err(ErrorCode::TooLarge);
    }
    let kind = one_header(headers, "content-type").ok_or(ErrorCode::UnsupportedMediaType)?;
    if ![
        "application/octet-stream",
        "image/png",
        "image/jpeg",
        "image/tiff",
    ]
    .iter()
    .any(|expected| kind.eq_ignore_ascii_case(expected))
    {
        return Err(ErrorCode::UnsupportedMediaType);
    }
    Ok(length)
}

/// Copies every received frame exactly once into a preallocated reusable buffer.
/// Trailers, overlong data and truncation are explicit framing failures.
pub async fn read_body(
    mut body: Body,
    expected: usize,
    target: &mut BytesMut,
) -> Result<(), ErrorCode> {
    target.clear();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ErrorCode::InvalidRequest)?;
        let data = frame.into_data().map_err(|_| ErrorCode::InvalidRequest)?;
        if data.len() > expected.saturating_sub(target.len()) {
            return Err(ErrorCode::InvalidRequest);
        }
        target.extend_from_slice(&data);
    }
    if target.len() != expected {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(length: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static(length));
        headers
    }

    #[test]
    fn request_ids_are_fixed_ascii_and_not_reused() {
        let first = RequestId::next();
        let second = RequestId::next();
        assert_eq!(first.as_str().len(), 32);
        assert!(first.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn ambiguous_or_unbounded_framing_is_rejected() {
        assert_eq!(expected_length(&headers("1"), 1), Ok(1));
        assert_eq!(expected_length(&headers("2"), 1), Err(ErrorCode::TooLarge));
        for length in ["", "+1", "-1", "0", "1,1"] {
            assert!(expected_length(&headers(length), 1).is_err());
        }
        let mut duplicate = headers("1");
        duplicate.append(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
        assert!(expected_length(&duplicate, 1).is_err());
        let mut chunked = headers("1");
        chunked.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        assert!(expected_length(&chunked, 1).is_err());
    }

    #[tokio::test]
    async fn upload_bound_and_storage_reuse_are_explicit() {
        let mut storage = BytesMut::with_capacity(16);
        let pointer = storage.as_ptr();
        read_body(Body::from("image"), 5, &mut storage)
            .await
            .expect("exact upload");
        assert_eq!(&storage[..], b"image");
        assert_eq!(pointer, storage.as_ptr());
        assert!(
            read_body(Body::from("too long"), 2, &mut storage)
                .await
                .is_err()
        );
        assert!(
            read_body(Body::from("short"), 8, &mut storage)
                .await
                .is_err()
        );
    }
}
