use std::{
    fs::{File, OpenOptions, Permissions, TryLockError},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};

use sha2::{Digest, Sha256};

use crate::{
    CacheError, CacheReport, MODEL_REPOSITORY, MODEL_REVISION, ModelArtifact, Result,
    artifact_path, check_directory, io_error, model_artifacts, reject_unlocked_models, verify_file,
};

#[derive(Debug)]
pub struct FetchOptions {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(120),
        }
    }
}

pub fn fetch(directory: &Path, math: bool) -> Result<CacheReport> {
    fetch_with(directory, math, &FetchOptions::default())
}

/// Explicit acquisition from the immutable upstream revision only.
///
/// Fetch is synchronous setup work. Run it before the Tokio runtime or in a
/// separately scheduled init container; never in an async request handler.
pub fn fetch_with(directory: &Path, math: bool, options: &FetchOptions) -> Result<CacheReport> {
    if options.connect_timeout.is_zero() || options.request_timeout < options.connect_timeout {
        return Err(CacheError::TimeoutConfig);
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| io_error("create directory", "cache", error))?;
    check_directory(directory)?;
    let lock = lock(directory)?;
    reject_unlocked_models(directory)?;
    // Build at most one TLS connection pool for this explicit setup operation.
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(options.connect_timeout)
        .timeout(options.request_timeout)
        .user_agent(concat!("indra-models/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut report = CacheReport::default();
    let mut path = artifact_path(directory)?;
    for expected in model_artifacts(math) {
        path.set_file_name(expected.name);
        match verify_file(&path, expected) {
            Ok(()) => report.reused += 1,
            Err(CacheError::Size(_) | CacheError::Hash(_)) => {
                download(&client, directory, expected)?;
                report.downloaded += 1;
            }
            Err(CacheError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                download(&client, directory, expected)?;
                report.downloaded += 1;
            }
            Err(error) => return Err(error),
        }
        report.files += 1;
        report.bytes += expected.size;
    }
    lock.unlock()
        .map_err(|error| io_error("unlock cache", "cache", error))?;
    Ok(report)
}

fn lock(directory: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(".fetch.lock"))
        .map_err(|error| io_error("open cache lock", "cache", error))?;
    if !file
        .metadata()
        .map_err(|error| io_error("inspect cache lock", "cache", error))?
        .is_file()
    {
        return Err(CacheError::FileType);
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(CacheError::Busy),
        Err(TryLockError::Error(error)) => Err(io_error("lock cache", "cache", error)),
    }
}

fn download(
    client: &reqwest::blocking::Client,
    directory: &Path,
    expected: &ModelArtifact,
) -> Result<()> {
    let url = format!(
        "https://huggingface.co/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{}",
        expected.name
    );
    let mut response = client.get(url).send()?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length != expected.size)
    {
        return Err(CacheError::Size(expected.name));
    }
    install(directory, expected, &mut response)
}

fn install(directory: &Path, expected: &ModelArtifact, reader: &mut impl Read) -> Result<()> {
    // A single cache-wide OS lock makes this deterministic recovery filename
    // safe across cooperating processes. A crash leaves only this untrusted part.
    let temporary = directory.join(format!("{}.part", expected.name));
    match std::fs::remove_file(&temporary) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(io_error("remove stale partial", expected.name, error)),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| io_error("create partial", expected.name, error))?;
    let mut remaining = expected.size;
    let mut buffer = [0_u8; 65_536];
    let mut digest = Sha256::new();
    loop {
        let length = reader
            .read(&mut buffer)
            .map_err(|error| io_error("download artifact", expected.name, error))?;
        if length == 0 {
            break;
        }
        remaining = remaining
            .checked_sub(length as u64)
            .ok_or(CacheError::Size(expected.name))?;
        digest.update(&buffer[..length]);
        file.write_all(&buffer[..length])
            .map_err(|error| io_error("write partial", expected.name, error))?;
    }
    if remaining != 0 {
        return Err(CacheError::Size(expected.name));
    }
    if digest.finalize().as_slice() != expected.sha256 {
        return Err(CacheError::Hash(expected.name));
    }
    file.set_permissions(Permissions::from_mode(0o444))
        .map_err(|error| io_error("protect artifact", expected.name, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync artifact", expected.name, error))?;
    std::fs::rename(&temporary, directory.join(expected.name))
        .map_err(|error| io_error("publish artifact", expected.name, error))?;
    let directory = File::open(directory)
        .map_err(|error| io_error("open directory for sync", "cache", error))?;
    directory
        .sync_all()
        .map_err(|error| io_error("sync cache directory", "cache", error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ABC, directory};

    #[test]
    #[ignore = "explicitly downloads the immutable public model revision"]
    fn fetch_real_immutable_revision() {
        let path = std::env::var_os("INDRA_TEST_FETCH_DIR").expect("INDRA_TEST_FETCH_DIR");
        let directory = Path::new(&path);
        assert!(
            !directory.exists(),
            "select a new directory to prove a cold acquisition"
        );
        let started = std::time::Instant::now();
        let cold = fetch(directory, true).expect("cold immutable acquisition");
        println!(
            "{{\"mode\":\"cold_fetch\",\"files\":{},\"bytes\":{},\"downloaded\":{},\"elapsed_ms\":{}}}",
            cold.files,
            cold.bytes,
            cold.downloaded,
            started.elapsed().as_millis()
        );
        assert_eq!(cold.downloaded, 14);
        assert_eq!(cold.reused, 0);
        let started = std::time::Instant::now();
        let warm = fetch(directory, true).expect("reuse independently verified files");
        assert_eq!(warm.downloaded, 0);
        assert_eq!(warm.reused, 14);
        println!(
            "{{\"mode\":\"verified_reuse\",\"files\":{},\"bytes\":{},\"downloaded\":{},\"elapsed_us\":{}}}",
            warm.files,
            warm.bytes,
            warm.downloaded,
            started.elapsed().as_micros()
        );
        crate::verify(directory, true).expect("offline verify after fetch");
    }

    #[test]
    fn failed_download_cannot_replace_existing_good_artifact() {
        let directory = directory();
        let _lock = lock(&directory).expect("exclusive lock");
        install(&directory, &ABC, &mut &b"abc"[..]).expect("first install");
        assert!(matches!(
            install(&directory, &ABC, &mut &b"bad"[..]),
            Err(CacheError::Hash(_))
        ));
        verify_file(&directory.join(ABC.name), &ABC).expect("old good artifact remains");
        assert!(directory.join("test.onnx.part").exists());
        install(&directory, &ABC, &mut &b"abc"[..]).expect("recover partial");
        assert!(!directory.join("test.onnx.part").exists());
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn incomplete_or_oversized_download_never_becomes_visible() {
        let directory = directory();
        let _lock = lock(&directory).expect("exclusive lock");
        for bytes in [&b"ab"[..], &b"abcd"[..]] {
            assert!(matches!(
                install(&directory, &ABC, &mut &bytes[..]),
                Err(CacheError::Size(_))
            ));
            assert!(!directory.join(ABC.name).exists());
        }
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn competing_fetch_fails_fast_and_lock_recovers_on_drop() {
        let directory = directory();
        let first = lock(&directory).expect("first writer");
        assert!(matches!(lock(&directory), Err(CacheError::Busy)));
        drop(first);
        drop(lock(&directory).expect("OS releases lock on descriptor close"));
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn invalid_timeout_is_rejected_before_any_directory_or_network_work() {
        let path = std::env::temp_dir().join("indra-invalid-timeout-must-not-exist");
        let options = FetchOptions {
            connect_timeout: Duration::ZERO,
            request_timeout: Duration::from_secs(1),
        };
        assert!(matches!(
            fetch_with(&path, false, &options),
            Err(CacheError::TimeoutConfig)
        ));
        assert!(!path.exists());
    }
}
