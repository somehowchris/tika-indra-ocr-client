//! Opt-in scientific evidence, separate from production logs: bounded NumPy tensor files.

use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use crate::{EngineError, Result};

const MAX_TRACE_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) enum Recorder {
    Disabled,
    /// Observe real encoder input shapes in native unit gates without tensor I/O.
    #[cfg(test)]
    MathBatches {
        histogram: [u16; crate::MAX_BATCH],
    },
    Directory {
        path: PathBuf,
        index: u64,
        bytes: u64,
    },
}

impl Recorder {
    pub fn directory(path: &Path) -> Result<Self> {
        fs::create_dir_all(path)?;
        if fs::read_dir(path)?.next().is_some() {
            return Err(EngineError::Config("tensor trace directory must be empty"));
        }
        Ok(Self::Directory {
            path: PathBuf::from(path),
            index: 0,
            bytes: 0,
        })
    }

    pub fn tensor<T: Element>(&mut self, name: &str, shape: &[i64], data: &[T]) -> Result<()> {
        #[cfg(test)]
        if let Self::MathBatches { histogram } = self {
            if name == "math-input" {
                let row = shape
                    .first()
                    .and_then(|value| usize::try_from(*value).ok())
                    .and_then(|batch| batch.checked_sub(1))
                    .and_then(|index| histogram.get_mut(index))
                    .ok_or(EngineError::Shape("observed math batch size"))?;
                *row = row
                    .checked_add(1)
                    .ok_or(EngineError::Limit("observed math batch count"))?;
            }
            return Ok(());
        }
        let Self::Directory { path, index, bytes } = self else {
            return Ok(());
        };
        let elements = shape
            .iter()
            .try_fold(1_usize, |product, dimension| {
                usize::try_from(*dimension)
                    .ok()
                    .and_then(|n| product.checked_mul(n))
            })
            .ok_or(EngineError::Shape("trace tensor dimensions"))?;
        if elements != data.len() {
            return Err(EngineError::Shape("trace tensor element count"));
        }
        let byte_count = (data.len() as u64)
            .checked_mul(size_of::<T>() as u64)
            .ok_or(EngineError::Limit("trace byte count"))?;
        if bytes
            .checked_add(byte_count)
            .is_none_or(|n| n > MAX_TRACE_BYTES)
        {
            return Err(EngineError::Limit(
                "diagnostic tensor trace exceeds 512 MiB",
            ));
        }
        let file = path.join(format!("{index:06}-{name}.npy"));
        let mut header = String::with_capacity(128);
        write!(
            &mut header,
            "{{'descr': '{}', 'fortran_order': False, 'shape': (",
            T::DESCRIPTOR
        )
        .map_err(|_| EngineError::Shape("trace header"))?;
        for dimension in shape {
            write!(&mut header, "{dimension}, ").map_err(|_| EngineError::Shape("trace header"))?;
        }
        header.push_str(") }");
        while !(10 + header.len() + 1).is_multiple_of(64) {
            header.push(' ');
        }
        header.push('\n');
        let length =
            u16::try_from(header.len()).map_err(|_| EngineError::Limit("trace header length"))?;
        let mut writer = BufWriter::new(File::create_new(file)?);
        writer.write_all(b"\x93NUMPY\x01\x00")?;
        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(header.as_bytes())?;
        for value in data {
            value.write(&mut writer)?;
        }
        writer.flush()?;
        *index += 1;
        *bytes += byte_count;
        Ok(())
    }
}

pub(crate) trait Element {
    const DESCRIPTOR: &'static str;
    fn write(&self, writer: &mut impl Write) -> std::io::Result<()>;
}

macro_rules! element {
    ($ty:ty,$descriptor:literal) => {
        impl Element for $ty {
            const DESCRIPTOR: &'static str = $descriptor;
            fn write(&self, writer: &mut impl Write) -> std::io::Result<()> {
                writer.write_all(&self.to_le_bytes())
            }
        }
    };
}
element!(f32, "<f4");
element!(f64, "<f8");
element!(i64, "<i8");
element!(u64, "<u8");
element!(i32, "<i4");
element!(u8, "|u1");
impl Element for bool {
    const DESCRIPTOR: &'static str = "|b1";
    fn write(&self, writer: &mut impl Write) -> std::io::Result<()> {
        writer.write_all(&[u8::from(*self)])
    }
}
