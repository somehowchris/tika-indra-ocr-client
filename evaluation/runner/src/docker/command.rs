//! Shell-free, bounded subprocess capture. Every invocation retains its argv,
//! stdout, stderr and termination result before its caller interprets success.
use super::DockerError;
use arrayvec::ArrayVec;
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static MAX_COMMANDS: u16 = 768;
static MAX_CAPTURE_BYTES: u64 = 32 * 1024 * 1024;
static MAX_TOTAL_CAPTURE_BYTES: u64 = 256 * 1024 * 1024;
static CLEANUP_CAPTURE_RESERVE: u64 = 128 * 1024 * 1024;

pub(super) struct Control {
    directory: PathBuf,
    sequence: u16,
    captured_bytes: u64,
    cleanup: bool,
}

pub(super) struct Completed {
    pub status: ExitStatus,
    pub stdout: PathBuf,
    pub timed_out: bool,
    pub output_limit: bool,
}

impl Completed {
    pub fn success(&self) -> bool {
        self.status.success() && !self.timed_out && !self.output_limit
    }

    pub fn require_success(&self, step: &'static str) -> Result<(), DockerError> {
        if self.success() {
            Ok(())
        } else {
            Err(DockerError::Command {
                step,
                code: self.status.code(),
                timed_out: self.timed_out,
                output_limit: self.output_limit,
            })
        }
    }
}

#[derive(Serialize)]
struct Invocation<'a> {
    step: &'a str,
    program: &'a str,
    arguments: ArrayVec<&'a str, 128>,
    environment: ArrayVec<(&'a str, Option<&'a str>), 32>,
    started_unix_ms: u128,
    deadline_seconds: u64,
    capture_limit_bytes_per_stream: u64,
}

#[derive(Serialize)]
struct Termination {
    code: Option<i32>,
    signal: Option<i32>,
    elapsed_ms: u128,
    timed_out: bool,
    output_limit: bool,
    stdout_bytes: u64,
    stderr_bytes: u64,
}

pub(super) fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), DockerError> {
    let mut file = create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn read_text(path: &Path, limit: u64) -> Result<String, DockerError> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(DockerError::Invalid(
            "command output exceeds its parsing bound",
        ));
    }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(DockerError::Invalid(
            "command output grew beyond its parsing bound",
        ));
    }
    Ok(text)
}

fn capture<R: Read>(
    mut pipe: R,
    mut file: File,
    stop: &AtomicBool,
    exceeded: &AtomicBool,
    finished: &AtomicBool,
    limit: u64,
) -> io::Result<u64> {
    let result = (|| {
        let mut buffer = [0_u8; 8192];
        let mut written = 0_u64;
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let count = match pipe.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if finished.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if count == 0 {
                break;
            }
            let available = (limit - written).min(count as u64) as usize;
            file.write_all(&buffer[..available])?;
            written += available as u64;
            if available != count {
                exceeded.store(true, Ordering::Release);
                stop.store(true, Ordering::Release);
                break;
            }
        }
        file.sync_all()?;
        Ok(written)
    })();
    if result.is_err() {
        stop.store(true, Ordering::Release);
    }
    result
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            if let Err(error) = child.kill() {
                eprintln!("evaluation command emergency termination: {error}");
            }
            if let Err(error) = child.wait() {
                eprintln!("evaluation command emergency reap: {error}");
            }
        }
    }
}

impl Control {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            sequence: 0,
            captured_bytes: 0,
            cleanup: false,
        }
    }

    pub fn begin_cleanup(&mut self) {
        self.cleanup = true;
    }

    pub fn execute(
        &mut self,
        step: &'static str,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<Completed, DockerError> {
        if self.sequence >= MAX_COMMANDS + if self.cleanup { 32 } else { 0 } {
            return Err(DockerError::Invalid(
                "command count exceeded the fixed driver bound",
            ));
        }
        let total_limit = MAX_TOTAL_CAPTURE_BYTES
            + if self.cleanup {
                CLEANUP_CAPTURE_RESERVE
            } else {
                0
            };
        let capture_limit = MAX_CAPTURE_BYTES.min((total_limit - self.captured_bytes) / 2);
        if capture_limit < 8192 {
            return Err(DockerError::Invalid(
                "total retained command streams exceed their fixed bound",
            ));
        }
        self.sequence += 1;
        let stem = format!("{:04}-{step}", self.sequence);
        let stdout_path = self.directory.join(format!("{stem}.stdout"));
        let stderr_path = self.directory.join(format!("{stem}.stderr"));
        let mut arguments = ArrayVec::new();
        for value in command.get_args() {
            arguments
                .try_push(
                    value
                        .to_str()
                        .ok_or(DockerError::Invalid("non-UTF-8 command argument"))?,
                )
                .map_err(|_| DockerError::Invalid("too many command arguments"))?;
        }
        let mut environment = ArrayVec::new();
        for (key, value) in command.get_envs() {
            let key = key
                .to_str()
                .ok_or(DockerError::Invalid("non-UTF-8 environment key"))?;
            let value = value
                .map(|v| {
                    v.to_str()
                        .ok_or(DockerError::Invalid("non-UTF-8 environment value"))
                })
                .transpose()?;
            environment
                .try_push((key, value))
                .map_err(|_| DockerError::Invalid("too many command environment entries"))?;
        }
        write_json(
            &self.directory.join(format!("{stem}.invocation.json")),
            &Invocation {
                step,
                program: command
                    .get_program()
                    .to_str()
                    .ok_or(DockerError::Invalid("non-UTF-8 executable"))?,
                arguments,
                environment,
                started_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| DockerError::Invalid("clock precedes Unix epoch"))?
                    .as_millis(),
                deadline_seconds: timeout.as_secs(),
                capture_limit_bytes_per_stream: capture_limit,
            },
        )?;
        let stdout_file = create(&stdout_path)?;
        let stderr_file = create(&stderr_path)?;
        let started = Instant::now();
        let mut guard = ChildGuard(Some(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        ));
        let stop = AtomicBool::new(false);
        let exceeded = AtomicBool::new(false);
        let finished = AtomicBool::new(false);
        let child = guard
            .0
            .as_mut()
            .ok_or(DockerError::Invalid("missing child"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(DockerError::Invalid("missing stdout pipe"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(DockerError::Invalid("missing stderr pipe"))?;
        for pipe in [
            std::os::fd::AsFd::as_fd(&stdout),
            std::os::fd::AsFd::as_fd(&stderr),
        ] {
            let flags = fcntl_getfl(pipe).map_err(std::io::Error::from)?;
            fcntl_setfl(pipe, flags | OFlags::NONBLOCK).map_err(std::io::Error::from)?;
        }
        let (status, timed_out, stdout_bytes, stderr_bytes) =
            thread::scope(|scope| -> Result<_, DockerError> {
                let stdout_thread = thread::Builder::new()
                    .name(String::from("eval-stdout"))
                    .spawn_scoped(scope, || {
                        capture(
                            stdout,
                            stdout_file,
                            &stop,
                            &exceeded,
                            &finished,
                            capture_limit,
                        )
                    })?;
                let stderr_thread = match thread::Builder::new()
                    .name(String::from("eval-stderr"))
                    .spawn_scoped(scope, || {
                        capture(
                            stderr,
                            stderr_file,
                            &stop,
                            &exceeded,
                            &finished,
                            capture_limit,
                        )
                    }) {
                    Ok(handle) => handle,
                    Err(error) => {
                        stop.store(true, Ordering::Release);
                        child.kill()?;
                        child.wait()?;
                        return Err(DockerError::Io(error));
                    }
                };
                let mut timed_out = false;
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break status,
                        Ok(None) => {}
                        Err(error) => {
                            stop.store(true, Ordering::Release);
                            child.kill()?;
                            child.wait()?;
                            return Err(DockerError::Io(error));
                        }
                    }
                    if started.elapsed() >= timeout || stop.load(Ordering::Acquire) {
                        timed_out = started.elapsed() >= timeout;
                        stop.store(true, Ordering::Release);
                        child.kill()?;
                        break child.wait()?;
                    }
                    thread::sleep(Duration::from_millis(10));
                };
                finished.store(true, Ordering::Release);
                let stdout_bytes = stdout_thread
                    .join()
                    .map_err(|_| DockerError::Invalid("stdout capture panicked"))??;
                let stderr_bytes = stderr_thread
                    .join()
                    .map_err(|_| DockerError::Invalid("stderr capture panicked"))??;
                Ok((status, timed_out, stdout_bytes, stderr_bytes))
            })?;
        guard.0 = None;
        self.captured_bytes += stdout_bytes + stderr_bytes;
        let output_limit = exceeded.load(Ordering::Acquire);
        write_json(
            &self.directory.join(format!("{stem}.termination.json")),
            &Termination {
                code: status.code(),
                signal: status.signal(),
                elapsed_ms: started.elapsed().as_millis(),
                timed_out,
                output_limit,
                stdout_bytes,
                stderr_bytes,
            },
        )?;
        Ok(Completed {
            status,
            stdout: stdout_path,
            timed_out,
            output_limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_capture_keeps_exact_prefix_and_signals_termination() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = PathBuf::from(format!("target/capture-{nonce}"));
        let stop = AtomicBool::new(false);
        let exceeded = AtomicBool::new(false);
        let finished = AtomicBool::new(false);
        assert_eq!(
            capture(
                &b"a\xffbcdef"[..],
                create(&path).expect("new file"),
                &stop,
                &exceeded,
                &finished,
                4
            )
            .expect("capture"),
            4
        );
        assert!(stop.load(Ordering::Relaxed));
        assert!(exceeded.load(Ordering::Relaxed));
        let mut bytes = [0_u8; 4];
        File::open(&path)
            .expect("captured")
            .read_exact(&mut bytes)
            .expect("read");
        assert_eq!(&bytes, b"a\xffbc");
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn capture_finishes_without_eof_when_a_descendant_retains_the_pipe() {
        struct HeldOpen(std::io::Cursor<&'static [u8]>);
        impl Read for HeldOpen {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                match self.0.read(out)? {
                    0 => Err(io::ErrorKind::WouldBlock.into()),
                    count => Ok(count),
                }
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = PathBuf::from(format!("target/inherited-pipe-{nonce}"));
        let reader = HeldOpen(std::io::Cursor::new(b"retained bytes"));
        // A nonblocking pipe whose writer remains alive never returns EOF.
        let stop = AtomicBool::new(false);
        let exceeded = AtomicBool::new(false);
        let finished = AtomicBool::new(true);
        let before = Instant::now();
        assert_eq!(
            capture(
                reader,
                create(&path).expect("file"),
                &stop,
                &exceeded,
                &finished,
                1024
            )
            .expect("capture"),
            14
        );
        assert!(before.elapsed() < Duration::from_secs(1));
        assert_eq!(read_text(&path, 1024).expect("text"), "retained bytes");
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn deadline_kills_and_reaps_child_and_shell_metacharacters_stay_literal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = PathBuf::from(format!("target/command-deadline-{nonce}"));
        std::fs::create_dir(&directory).expect("directory");
        let mut control = Control::new(PathBuf::from(&directory));
        let started = Instant::now();
        let result = control
            .execute(
                "deadline-fixture",
                Command::new("/bin/sleep").arg("2"),
                Duration::from_millis(30),
            )
            .expect("bounded command");
        assert!(result.timed_out);
        assert!(!result.success());
        assert_eq!(result.status.signal(), Some(9));
        assert!(started.elapsed() < Duration::from_secs(1));
        let literal = "literal $(no-shell) `no-shell` $HOME";
        let result = control
            .execute(
                "literal-fixture",
                Command::new("/usr/bin/printf").args(["%s", literal]),
                Duration::from_secs(2),
            )
            .expect("literal command");
        assert!(result.success());
        assert_eq!(read_text(&result.stdout, 1024).expect("text"), literal);
        #[derive(serde::Deserialize)]
        struct End {
            timed_out: bool,
            signal: Option<i32>,
        }
        let record: End = serde_json::from_str(
            &read_text(
                &directory.join("0001-deadline-fixture.termination.json"),
                4096,
            )
            .expect("termination"),
        )
        .expect("JSON");
        assert!(record.timed_out);
        assert_eq!(record.signal, Some(9));
        std::fs::remove_dir_all(directory).expect("cleanup owned test directory");
    }

    #[test]
    fn cleanup_has_separate_command_and_capture_reserves() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = PathBuf::from(format!("target/command-reserve-{nonce}"));
        std::fs::create_dir(&directory).expect("directory");
        let mut control = Control::new(PathBuf::from(&directory));
        control.sequence = MAX_COMMANDS;
        control.captured_bytes = MAX_TOTAL_CAPTURE_BYTES;
        assert!(
            control
                .execute(
                    "refused",
                    &mut Command::new("/usr/bin/true"),
                    Duration::from_secs(1)
                )
                .is_err()
        );
        control.begin_cleanup();
        assert!(
            control
                .execute(
                    "cleanup-fixture",
                    &mut Command::new("/usr/bin/true"),
                    Duration::from_secs(1)
                )
                .expect("reserved cleanup executes")
                .success()
        );
        assert!(
            directory
                .join("0769-cleanup-fixture.termination.json")
                .is_file()
        );
        std::fs::remove_dir_all(directory).expect("cleanup owned directory");
    }
}
