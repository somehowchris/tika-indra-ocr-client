//! Diagnostic startup barrier; excluded from timed regions and production services.
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
    thread,
    time::{Duration, Instant},
};

pub fn wait(path: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 9 {
                    return Err(io::Error::other("invalid host admission marker"));
                }
                let mut marker = [0_u8; 10];
                let mut file = File::open(path)?;
                file.read_exact(&mut marker[..9])?;
                if &marker[..9] != b"admitted\n" || file.read(&mut marker[9..])? != 0 {
                    return Err(io::Error::other("host did not admit this experiment"));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "host admission deadline",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_requires_exact_regular_file() {
        let root = std::env::temp_dir().join(format!("indra-admission-{}", std::process::id()));
        fs::create_dir(&root).expect("fresh test directory");
        let marker = root.join("marker");
        fs::write(&marker, b"admitted\n").expect("marker");
        wait(&marker).expect("exact marker admitted");
        fs::write(&marker, b"rejected\n").expect("rejected marker");
        assert!(wait(&marker).is_err());
        fs::write(&marker, b"admitted\n!").expect("oversized marker");
        assert!(wait(&marker).is_err());
        let link = root.join("link");
        std::os::unix::fs::symlink(&marker, &link).expect("symlink");
        assert!(wait(&link).is_err());
        fs::remove_dir_all(root).expect("owned test cleanup");
    }
}
