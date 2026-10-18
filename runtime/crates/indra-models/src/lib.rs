//! Immutable model identities and explicit, bounded cache acquisition.
//!
//! Serving only calls [`verify`]. Network access is compiled behind `fetch`
//! and is never an implicit consequence of inference or a cache miss.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "fetch")]
mod acquisition;
mod manifest;
#[cfg(feature = "fetch")]
pub use acquisition::{FetchOptions, fetch, fetch_with};

pub static MODEL_REVISION: &str = "ed345c7f158eddbc7dee0c6a7269afb75a94f7e8";
pub static MODEL_REPOSITORY: &str = "datalab-to/indra-ocr";

#[derive(Debug)]
pub struct ModelArtifact {
    pub name: &'static str,
    pub size: u64,
    pub sha256: [u8; 32],
    pub math: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CacheReport {
    pub files: u8,
    pub bytes: u64,
    pub reused: u8,
    pub downloaded: u8,
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("model cache I/O failed during {operation} for {artifact}")]
    Io {
        operation: &'static str,
        artifact: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("model cache must be a real directory, with regular non-symlink artifacts")]
    FileType,
    #[error("model artifact {0} has a different size from the lock")]
    Size(&'static str),
    #[error("model artifact {0} failed SHA-256 verification")]
    Hash(&'static str),
    #[error("model cache contains an unlocked ONNX file")]
    UnlockedModel,
    #[error("model cache path length exceeds addressable capacity")]
    PathLength,
    #[error("another process is acquiring this model cache; retry after it finishes")]
    Busy,
    #[error(
        "model acquisition timeout must be positive and connect timeout cannot exceed total timeout"
    )]
    TimeoutConfig,
    #[cfg(feature = "fetch")]
    #[error("model download request failed")]
    Network(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, CacheError>;

pub fn model_artifacts(math: bool) -> impl Iterator<Item = &'static ModelArtifact> {
    manifest::FILES
        .iter()
        .filter(move |file| math || !file.math)
}

/// Verify selected artifacts every startup, including when the mount is read-only.
///
/// The cache is application-owned; mount it read-only for the serving process.
/// Hash validation is not a substitute for filesystem isolation against a writer
/// that deliberately races verification and the native runtime's later open.
pub fn verify(directory: &Path, math: bool) -> Result<CacheReport> {
    check_directory(directory)?;
    reject_unlocked_models(directory)?;
    let mut path = artifact_path(directory)?;
    let mut report = CacheReport::default();
    for expected in model_artifacts(math) {
        path.set_file_name(expected.name);
        verify_file(&path, expected)?;
        report.files += 1;
        report.bytes += expected.size;
        report.reused += 1;
    }
    Ok(report)
}

fn artifact_path(directory: &Path) -> Result<PathBuf> {
    let longest = manifest::FILES
        .iter()
        .map(|file| file.name.len())
        .max()
        .expect("non-empty pinned model manifest");
    let capacity = directory
        .as_os_str()
        .len()
        .checked_add(longest + 1)
        .ok_or(CacheError::PathLength)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(directory);
    path.push(manifest::FILES[0].name);
    Ok(path)
}

fn io_error(operation: &'static str, artifact: &'static str, source: io::Error) -> CacheError {
    CacheError::Io {
        operation,
        artifact,
        source,
    }
}

fn check_directory(directory: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(directory)
        .map_err(|error| io_error("inspect directory", "cache", error))?;
    if !metadata.is_dir() {
        return Err(CacheError::FileType);
    }
    Ok(())
}

fn reject_unlocked_models(directory: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).map_err(|error| io_error("list directory", "cache", error))?
    {
        let entry = entry.map_err(|error| io_error("read directory entry", "cache", error))?;
        let name = entry.file_name();
        if Path::new(&name)
            .extension()
            .is_some_and(|extension| extension == "onnx")
            && !manifest::FILES.iter().any(|file| name == file.name)
        {
            return Err(CacheError::UnlockedModel);
        }
    }
    Ok(())
}

fn open_regular(path: &Path, artifact: &'static str) -> Result<File> {
    // NOFOLLOW refuses a symlink atomically; NONBLOCK avoids hanging on a FIFO
    // before the descriptor metadata check. Both are harmless for regular files.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| io_error("open artifact", artifact, error))?;
    if !file
        .metadata()
        .map_err(|error| io_error("inspect artifact", artifact, error))?
        .is_file()
    {
        return Err(CacheError::FileType);
    }
    Ok(file)
}

fn verify_file(path: &Path, expected: &ModelArtifact) -> Result<()> {
    let mut file = open_regular(path, expected.name)?;
    if file
        .metadata()
        .map_err(|error| io_error("inspect size", expected.name, error))?
        .len()
        != expected.size
    {
        return Err(CacheError::Size(expected.name));
    }
    verify_stream(&mut file, expected)
}

fn verify_stream(reader: &mut impl Read, expected: &ModelArtifact) -> Result<()> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    let mut remaining = expected.size;
    loop {
        let length = reader
            .read(&mut buffer)
            .map_err(|error| io_error("read artifact", expected.name, error))?;
        if length == 0 {
            break;
        }
        remaining = remaining
            .checked_sub(length as u64)
            .ok_or(CacheError::Size(expected.name))?;
        digest.update(&buffer[..length]);
    }
    if remaining != 0 {
        return Err(CacheError::Size(expected.name));
    }
    if digest.finalize().as_slice() != expected.sha256 {
        return Err(CacheError::Hash(expected.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(super) static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
    pub(super) static ABC: ModelArtifact = ModelArtifact {
        name: "test.onnx",
        size: 3,
        sha256: [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ],
        math: false,
    };

    pub(super) fn directory() -> std::path::PathBuf {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("indra-models-{}-{number}", std::process::id()));
        std::fs::create_dir(&path).expect("unique test directory");
        path
    }

    #[test]
    fn fixed_manifest_matches_reference_totals() {
        assert_eq!(model_artifacts(false).count(), 9);
        assert_eq!(model_artifacts(true).count(), 14);
        assert_eq!(
            model_artifacts(true).map(|file| file.size).sum::<u64>(),
            38_187_650
        );
        for (index, file) in manifest::FILES.iter().enumerate() {
            assert!(!file.name.contains('/') && !file.name.contains(".."));
            assert!(file.size > 0);
            assert!(
                manifest::FILES[..index]
                    .iter()
                    .all(|earlier| earlier.name != file.name)
            );
        }
    }

    #[test]
    fn generated_manifest_matches_the_authoritative_lock() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../../../backend/models.lock.json"))
                .expect("valid authoritative model lock");
        assert_eq!(lock["repository"].as_str(), Some(MODEL_REPOSITORY));
        assert_eq!(lock["revision"].as_str(), Some(MODEL_REVISION));
        let artifacts = lock["files"].as_array().expect("lock artifact array");
        assert_eq!(artifacts.len(), manifest::FILES.len());
        for expected in &manifest::FILES {
            let actual = artifacts
                .iter()
                .find(|item| item["name"].as_str() == Some(expected.name))
                .expect("locked artifact");
            assert_eq!(actual["size"].as_u64(), Some(expected.size));
            assert_eq!(
                actual["profile"].as_str(),
                Some(if expected.math { "math" } else { "base" })
            );
            let hash = actual["sha256"].as_str().expect("locked hash");
            assert_eq!(hash.len(), 64);
            for (index, byte) in expected.sha256.iter().enumerate() {
                assert_eq!(
                    u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16).expect("hex hash"),
                    *byte
                );
            }
        }
    }

    #[test]
    #[ignore = "requires explicitly selected local model artifacts"]
    fn verify_real_locked_cache() {
        let path = std::env::var_os("INDRA_TEST_MODEL_DIR").expect("INDRA_TEST_MODEL_DIR");
        for math in [false, true] {
            let begin = std::time::Instant::now();
            let report = verify(Path::new(&path), math).expect("real locked artifacts");
            println!(
                "{{\"mode\":\"offline_verify\",\"math\":{math},\"files\":{},\"bytes\":{},\"elapsed_us\":{}}}",
                report.files,
                report.bytes,
                begin.elapsed().as_micros()
            );
        }
    }

    #[test]
    fn stream_rejects_truncation_trailing_bytes_and_wrong_hash() {
        assert!(verify_stream(&mut &b"abc"[..], &ABC).is_ok());
        assert!(matches!(
            verify_stream(&mut &b"ab"[..], &ABC),
            Err(CacheError::Size(_))
        ));
        assert!(matches!(
            verify_stream(&mut &b"abcd"[..], &ABC),
            Err(CacheError::Size(_))
        ));
        assert!(matches!(
            verify_stream(&mut &b"abd"[..], &ABC),
            Err(CacheError::Hash(_))
        ));
    }

    #[test]
    fn read_only_file_verifies_and_symlink_is_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = directory();
        let path = directory.join(ABC.name);
        std::fs::write(&path, b"abc").expect("write fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("read-only");
        verify_file(&path, &ABC).expect("read-only verification");
        let link = directory.join("link");
        symlink(&path, &link).expect("symlink fixture");
        assert!(verify_file(&link, &ABC).is_err());
        assert!(matches!(
            verify_file(&directory, &ABC),
            Err(CacheError::FileType)
        ));
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn verification_of_missing_cache_is_read_only_and_does_not_initialize_it() {
        let parent = directory();
        let missing = parent.join(MODEL_REVISION);
        assert!(matches!(
            verify(&missing, false),
            Err(CacheError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound
        ));
        assert!(!missing.exists(), "serving never acquires a missing cache");
        std::fs::remove_dir(parent).expect("cleanup");
    }

    #[test]
    fn unknown_onnx_never_silently_changes_the_engine() {
        let directory = directory();
        std::fs::write(directory.join("rec-int8.onnx"), b"unexpected").expect("write fixture");
        assert!(matches!(
            reject_unlocked_models(&directory),
            Err(CacheError::UnlockedModel)
        ));
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
