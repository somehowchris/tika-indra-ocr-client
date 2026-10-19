//! Format bounds are checked before passing compressed bytes to native codecs.

use opencv::{core, imgcodecs, prelude::*};

use crate::{EngineError, MAX_FRAMES, MAX_INPUT_BYTES, MAX_PIXELS, MAX_SIDE, Result};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Png,
    Jpeg,
    Tiff,
}

pub(crate) struct Header {
    pub format: Format,
    pub dimensions: [[usize; 2]; MAX_FRAMES],
    pub frames: usize,
}

impl Header {
    fn new(format: Format) -> Self {
        Self {
            format,
            dimensions: [[0; 2]; MAX_FRAMES],
            frames: 0,
        }
    }

    fn push(&mut self, width: usize, height: usize) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(EngineError::InvalidImage("zero image dimension"));
        }
        if width > MAX_SIDE || height > MAX_SIDE || width > MAX_PIXELS / height {
            return Err(EngineError::Limit("image dimensions"));
        }
        if self.frames == MAX_FRAMES {
            return Err(EngineError::Limit("image frames"));
        }
        let earlier = self.dimensions[..self.frames]
            .iter()
            .map(|d| d[0] * d[1])
            .sum::<usize>();
        if earlier > MAX_PIXELS - width * height {
            return Err(EngineError::Limit("total decoded pixels"));
        }
        self.dimensions[self.frames] = [width, height];
        self.frames += 1;
        Ok(())
    }
}

fn bytes<const N: usize>(data: &[u8], at: usize) -> Result<[u8; N]> {
    let end = at
        .checked_add(N)
        .ok_or(EngineError::InvalidImage("offset overflow"))?;
    data.get(at..end)
        .and_then(|s| s.try_into().ok())
        .ok_or(EngineError::InvalidImage("truncated image structure"))
}

fn be32(data: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(bytes(data, at)?) as usize)
}

pub(crate) fn inspect(data: &[u8]) -> Result<Header> {
    if data.len() > MAX_INPUT_BYTES {
        return Err(EngineError::Limit("encoded image bytes"));
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(data);
    }
    if data.starts_with(b"\xff\xd8") {
        return jpeg(data);
    }
    if data.starts_with(b"II") || data.starts_with(b"MM") {
        return tiff(data);
    }
    Err(EngineError::UnsupportedImage(
        "only PNG, JPEG and classic TIFF are accepted; Tika rasterizes PDF",
    ))
}

fn png(data: &[u8]) -> Result<Header> {
    let mut header = Header::new(Format::Png);
    let mut pos: usize = 8;
    let mut image_data = false;
    loop {
        let length = be32(data, pos)?;
        let kind = bytes::<4>(data, pos + 4)?;
        let end = pos
            .checked_add(length)
            .and_then(|v| v.checked_add(12))
            .ok_or(EngineError::InvalidImage("PNG chunk overflow"))?;
        if end > data.len() {
            return Err(EngineError::InvalidImage("truncated PNG chunk"));
        }
        if header.frames == 0 && kind != *b"IHDR" {
            return Err(EngineError::InvalidImage("PNG must start with IHDR"));
        }
        match &kind {
            b"IHDR" => {
                if length != 13 || header.frames != 0 {
                    return Err(EngineError::InvalidImage("duplicate or invalid PNG IHDR"));
                }
                header.push(be32(data, pos + 8)?, be32(data, pos + 12)?)?;
            }
            b"IDAT" => image_data = true,
            b"acTL" | b"fcTL" | b"fdAT" => {
                return Err(EngineError::UnsupportedImage(
                    "animated PNG; use TIFF for multiple pages",
                ));
            }
            b"IEND" => {
                if length != 0 || !image_data || end != data.len() {
                    return Err(EngineError::InvalidImage("invalid PNG end"));
                }
                return Ok(header);
            }
            _ => (),
        }
        pos = end;
    }
}

fn jpeg(data: &[u8]) -> Result<Header> {
    let mut header = Header::new(Format::Jpeg);
    let mut pos = 2;
    let mut in_scan = false;
    while pos < data.len() {
        if data[pos] != 0xff {
            if !in_scan {
                return Err(EngineError::InvalidImage("invalid JPEG marker"));
            }
            pos += 1;
            continue;
        }
        while data.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *data
            .get(pos)
            .ok_or(EngineError::InvalidImage("truncated JPEG marker"))?;
        pos += 1;
        match marker {
            0x00 if in_scan => continue,
            0xd0..=0xd7 if in_scan => continue,
            0xd9 => {
                if header.frames == 0 || !in_scan || pos != data.len() {
                    return Err(EngineError::InvalidImage("invalid JPEG end"));
                }
                return Ok(header);
            }
            0x01 => continue,
            0xd8 | 0x00 => return Err(EngineError::InvalidImage("unexpected JPEG marker")),
            _ => (),
        }
        let length = u16::from_be_bytes(bytes(data, pos)?) as usize;
        if length < 2 || pos.checked_add(length).is_none_or(|end| end > data.len()) {
            return Err(EngineError::InvalidImage("truncated JPEG segment"));
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 || header.frames != 0 {
                return Err(EngineError::InvalidImage(
                    "duplicate or invalid JPEG dimensions",
                ));
            }
            header.push(
                u16::from_be_bytes(bytes(data, pos + 5)?) as usize,
                u16::from_be_bytes(bytes(data, pos + 3)?) as usize,
            )?;
        }
        if marker == 0xda {
            in_scan = true;
        }
        pos += length;
    }
    Err(EngineError::InvalidImage("JPEG has no complete end marker"))
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn short(&self, at: usize) -> Result<usize> {
        let b = bytes(self.data, at)?;
        Ok(if self.little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        } as usize)
    }
    fn long(&self, at: usize) -> Result<usize> {
        let b = bytes(self.data, at)?;
        Ok(if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    }
    fn value(&self, entry: usize, index: usize) -> Result<usize> {
        let kind = self.short(entry + 2)?;
        let count = self.long(entry + 4)?;
        if index >= count {
            return Err(EngineError::InvalidImage("TIFF value index"));
        }
        let width: usize = match kind {
            3 => 2,
            4 => 4,
            _ => {
                return Err(EngineError::InvalidImage(
                    "TIFF dimensions and offsets must be SHORT or LONG",
                ));
            }
        };
        let total = count
            .checked_mul(width)
            .ok_or(EngineError::InvalidImage("TIFF field overflow"))?;
        let base = if total <= 4 {
            entry + 8
        } else {
            self.long(entry + 8)?
        };
        if base
            .checked_add(total)
            .is_none_or(|end| end > self.data.len())
        {
            return Err(EngineError::InvalidImage("truncated TIFF value table"));
        }
        let at = base + index * width;
        if width == 2 {
            self.short(at)
        } else {
            self.long(at)
        }
    }
    fn segments(&self, offset: Option<usize>, count: Option<usize>) -> Result<bool> {
        let (Some(offset), Some(count)) = (offset, count) else {
            if offset.is_some() || count.is_some() {
                return Err(EngineError::InvalidImage("incomplete TIFF strip/tile pair"));
            }
            return Ok(false);
        };
        let n = self.long(offset + 4)?;
        if n == 0 || n > MAX_PIXELS || n != self.long(count + 4)? {
            return Err(EngineError::InvalidImage("invalid TIFF strip/tile count"));
        }
        for index in 0..n {
            let start = self.value(offset, index)?;
            let length = self.value(count, index)?;
            if length == 0
                || start
                    .checked_add(length)
                    .is_none_or(|end| end > self.data.len())
            {
                return Err(EngineError::InvalidImage("truncated TIFF strip/tile"));
            }
        }
        Ok(true)
    }
}

fn tiff(data: &[u8]) -> Result<Header> {
    let reader = Tiff {
        data,
        little: data.starts_with(b"II"),
    };
    if reader.short(2)? != 42 {
        return Err(EngineError::UnsupportedImage(
            "BigTIFF and non-classic TIFF are not accepted",
        ));
    }
    let mut header = Header::new(Format::Tiff);
    let mut seen = [0_usize; MAX_FRAMES];
    let mut offset = reader.long(4)?;
    while offset != 0 {
        if header.frames == MAX_FRAMES {
            return Err(EngineError::Limit("image frames"));
        }
        if seen[..header.frames].contains(&offset) {
            return Err(EngineError::InvalidImage("cyclic TIFF page directory"));
        }
        seen[header.frames] = offset;
        let count = reader.short(offset)?;
        let end = offset
            .checked_add(2)
            .and_then(|v| count.checked_mul(12).and_then(|n| v.checked_add(n)))
            .ok_or(EngineError::InvalidImage("TIFF directory overflow"))?;
        if end.checked_add(4).is_none_or(|n| n > data.len()) {
            return Err(EngineError::InvalidImage("truncated TIFF directory"));
        }
        let mut fields = [None; 6];
        for index in 0..count {
            let at = offset + 2 + 12 * index;
            let tag = reader.short(at)?;
            let selected = match tag {
                256 => Some(0),
                257 => Some(1),
                273 => Some(2),
                279 => Some(3),
                324 => Some(4),
                325 => Some(5),
                _ => None,
            };
            if let Some(slot) = selected
                && fields[slot].replace(at).is_some()
            {
                return Err(EngineError::InvalidImage(
                    "duplicate TIFF dimension or segment tag",
                ));
            }
        }
        let width = fields[0].ok_or(EngineError::InvalidImage("TIFF has no width"))?;
        let height = fields[1].ok_or(EngineError::InvalidImage("TIFF has no height"))?;
        if reader.long(width + 4)? != 1 || reader.long(height + 4)? != 1 {
            return Err(EngineError::InvalidImage("TIFF dimension must be a scalar"));
        }
        header.push(reader.value(width, 0)?, reader.value(height, 0)?)?;
        let strips = reader.segments(fields[2], fields[3])?;
        let tiles = reader.segments(fields[4], fields[5])?;
        if !strips && !tiles {
            return Err(EngineError::UnsupportedImage(
                "TIFF has no complete strips or tiles",
            ));
        }
        offset = reader.long(end)?;
    }
    if header.frames == 0 {
        return Err(EngineError::InvalidImage("TIFF has no pages"));
    }
    Ok(header)
}

pub(crate) fn decode(data: &[u8], header: &Header, frame: usize) -> Result<core::Mat> {
    let encoded = core::Mat::from_slice(data)?;
    let image = if header.format == Format::Tiff {
        // OpenCV exposes its multi-page decoder only through a native vector.
        // A one-frame range bounds it before decoding; no Rust growable list is used.
        let mut native_frame = core::Vector::<core::Mat>::with_capacity(1);
        let range = core::Range::new(frame as i32, frame as i32 + 1)?;
        if !imgcodecs::imdecodemulti(
            &encoded,
            imgcodecs::IMREAD_GRAYSCALE,
            &mut native_frame,
            &range,
        )
        .map_err(decode_error)?
            || native_frame.len() != 1
        {
            return Err(EngineError::InvalidImage(
                "TIFF page could not be fully decoded",
            ));
        }
        native_frame.get(0)?
    } else {
        imgcodecs::imdecode(&encoded, imgcodecs::IMREAD_GRAYSCALE).map_err(decode_error)?
    };
    if image.empty() || image.typ() != core::CV_8UC1 || !image.is_continuous() {
        return Err(EngineError::InvalidImage(
            "decoder produced no contiguous grayscale page",
        ));
    }
    let expected = header.dimensions[frame];
    let actual = [image.cols() as usize, image.rows() as usize];
    // EXIF/TIFF orientation may interchange width and height, with equal bounded area.
    if actual != expected && actual != [expected[1], expected[0]] {
        return Err(EngineError::InvalidImage(
            "decoder dimensions differ from validated header",
        ));
    }
    Ok(image)
}

fn decode_error(error: opencv::Error) -> EngineError {
    // These decoders are local to the request. In particular, TIFF nextPage and
    // EXIF orientation may throw outside OpenCV's readHeader/readData catches.
    // An invalid document must not terminate otherwise healthy model workers.
    if error.code == core::StsNoMem {
        EngineError::OpenCv(error)
    } else {
        EngineError::InvalidImage("native image decoder rejected payload")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static PNG: &[u8] = include_bytes!("../../../../backend/tests/data/smoke.png");
    static TIFF: &[u8] = include_bytes!("../../../../backend/tests/data/two-pages.tiff");
    #[test]
    fn fixture_headers_are_bounded() {
        assert_eq!(inspect(PNG).expect("PNG").frames, 1);
        assert_eq!(inspect(TIFF).expect("TIFF").frames, 2);
        assert!(inspect(&PNG[..PNG.len() - 1]).is_err());
        assert!(inspect(&TIFF[..TIFF.len() / 2]).is_err());
    }
    #[test]
    fn empty_unknown_and_oversized_dimensions_are_rejected() {
        assert!(inspect(b"").is_err());
        assert!(inspect(b"%PDF-1.7").is_err());
        let mut h = Header::new(Format::Png);
        assert!(h.push(0, 1).is_err());
        assert!(h.push(MAX_SIDE + 1, 1).is_err());
        h.push(5000, 5000).expect("at total limit");
        assert!(h.push(1, 1).is_err());
    }
    #[test]
    fn tiff_cycles_and_duplicate_dimensions_are_rejected() {
        let mut file = [0_u8; 80];
        file[..8].copy_from_slice(b"II*\0\x08\0\0\0");
        file[8..10].copy_from_slice(&2_u16.to_le_bytes());
        for at in [10, 22] {
            file[at..at + 2].copy_from_slice(&256_u16.to_le_bytes());
            file[at + 2..at + 4].copy_from_slice(&4_u16.to_le_bytes());
            file[at + 4..at + 8].copy_from_slice(&1_u32.to_le_bytes());
            file[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
        }
        assert!(matches!(
            inspect(&file),
            Err(EngineError::InvalidImage(
                "duplicate TIFF dimension or segment tag"
            ))
        ));
    }

    #[test]
    fn codec_errors_are_request_local_but_allocation_failure_is_terminal() {
        assert!(matches!(
            decode_error(opencv::Error::new(core::StsAssert, "invalid codec state")),
            EngineError::InvalidImage(_)
        ));
        assert!(matches!(
            decode_error(opencv::Error::new(core::StsNoMem, "native allocation")),
            EngineError::OpenCv(_)
        ));
    }
}
