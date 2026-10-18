//! Exercise real nonblocking FIFOs; no models or synthetic perf measurements.
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fd::OwnedFd,
    fs::{CWD, Mode, OFlags, mkfifoat, open},
    io::{Errno, read, write},
};

#[path = "../examples/ocr/perf.rs"]
mod perf;
use perf::{Control, ControlError};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    command_path: PathBuf,
    ack_path: PathBuf,
    command: Option<OwnedFd>,
    ack: Option<OwnedFd>,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "indra-perf-control-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("owned unique directory");
        let command_path = directory.join("control");
        let ack_path = directory.join("ack");
        mkfifoat(CWD, &command_path, Mode::RUSR | Mode::WUSR).expect("control FIFO");
        mkfifoat(CWD, &ack_path, Mode::RUSR | Mode::WUSR).expect("ack FIFO");
        let flags = OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let command = Some(open(&command_path, flags, Mode::empty()).expect("peer control"));
        let ack = Some(open(&ack_path, flags, Mode::empty()).expect("peer ack"));
        Self {
            directory,
            command_path,
            ack_path,
            command,
            ack,
        }
    }

    fn client(&self) -> Control {
        Control::open(&self.command_path, &self.ack_path).expect("FIFO pair")
    }

    fn responder(&mut self, replies: &'static [&'static [u8]]) -> thread::JoinHandle<()> {
        let command = self.command.take().expect("owned command fd");
        let ack = self.ack.take().expect("owned ack fd");
        thread::spawn(move || {
            for (index, reply) in replies.iter().enumerate() {
                expect_command(
                    &command,
                    if index == 0 {
                        b"enable\n"
                    } else {
                        b"disable\n"
                    },
                );
                assert_eq!(write(&ack, reply).expect("short atomic ack"), reply.len());
            }
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn expect_command(fd: &OwnedFd, expected: &[u8]) {
    let due = Instant::now() + Duration::from_secs(5);
    let mut data = [0; 8];
    let mut received = 0;
    while received < expected.len() {
        let remaining = due
            .checked_duration_since(Instant::now())
            .expect("bounded test peer");
        match read(fd, &mut data[received..expected.len()]) {
            Ok(0) => panic!("control closed before complete command"),
            Ok(count) => received += count,
            Err(Errno::INTR) => {}
            Err(Errno::AGAIN) => {
                let timeout = Timespec::try_from(remaining).expect("bounded poll timeout");
                let mut pollfd = [PollFd::new(fd, PollFlags::IN)];
                assert!(poll(&mut pollfd, Some(&timeout)).expect("peer poll") > 0);
            }
            Err(error) => panic!("peer read failed: {error}"),
        }
    }
    assert_eq!(&data[..received], expected);
}

#[test]
fn enable_disable_and_eof_after_complete_ack_succeed() {
    let mut fixture = Fixture::new();
    let mut client = fixture.client();
    let peer = fixture.responder(&[b"ack\n", b"ack\n"]);
    client.enable().expect("enable acknowledged");
    client
        .disable()
        .expect("disable acknowledged, final peer closes");
    peer.join().expect("peer");
}

#[test]
fn requires_existing_distinct_fifos_without_following_symlinks() {
    let fixture = Fixture::new();
    let regular = fixture.directory.join("regular");
    fs::write(&regular, b"").expect("regular file");
    assert!(matches!(
        Control::open(&regular, &fixture.ack_path),
        Err(ControlError::Fifo)
    ));
    let link = fixture.directory.join("link");
    std::os::unix::fs::symlink(&fixture.command_path, &link).expect("symlink");
    assert!(matches!(
        Control::open(&link, &fixture.ack_path),
        Err(ControlError::Io(_))
    ));
    assert!(matches!(
        Control::open(&fixture.command_path, &fixture.command_path),
        Err(ControlError::Fifo)
    ));
}

#[test]
fn missing_command_reader_cannot_block_open() {
    let mut fixture = Fixture::new();
    drop(fixture.command.take());
    drop(fixture.ack.take());
    let started = Instant::now();
    assert!(Control::open(&fixture.command_path, &fixture.ack_path).is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn malformed_ack_is_rejected() {
    let mut fixture = Fixture::new();
    let mut client = fixture.client();
    let peer = fixture.responder(&[b"bad\n"]);
    assert!(matches!(client.enable(), Err(ControlError::Protocol(_))));
    peer.join().expect("peer");
}

#[test]
fn stale_ack_is_rejected_before_sending_command() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    write(fixture.ack.as_ref().expect("ack"), b"ack\n").expect("queued stale ack");
    assert!(matches!(client.enable(), Err(ControlError::Protocol(_))));
    assert_eq!(
        read(fixture.command.as_ref().expect("command"), &mut [0; 8]),
        Err(Errno::AGAIN)
    );
}

#[test]
fn final_disable_ack_cannot_hide_queued_suffix() {
    let mut fixture = Fixture::new();
    let mut client = fixture.client();
    let peer = fixture.responder(&[b"ack\n", b"ack\nextra"]);
    client.enable().expect("enable");
    assert!(matches!(client.disable(), Err(ControlError::Protocol(_))));
    peer.join().expect("peer");
}

#[test]
fn duplicate_ack_is_rejected() {
    let mut fixture = Fixture::new();
    let mut client = fixture.client();
    let peer = fixture.responder(&[b"ack\nack\n"]);
    assert!(matches!(client.enable(), Err(ControlError::Protocol(_))));
    peer.join().expect("peer");
}

#[test]
fn live_peer_without_ack_has_a_finite_deadline() {
    let fixture = Fixture::new();
    let mut client = Control::open_with_timeout(
        &fixture.command_path,
        &fixture.ack_path,
        Duration::from_millis(40),
    )
    .expect("bounded FIFO pair");
    let started = Instant::now();
    assert!(matches!(client.enable(), Err(ControlError::Deadline)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn closed_or_truncated_ack_does_not_succeed() {
    static REPLIES: [&[&[u8]]; 2] = [&[b""], &[b"ac"]];
    for replies in REPLIES {
        let mut fixture = Fixture::new();
        let mut client = fixture.client();
        let peer = fixture.responder(replies);
        assert!(matches!(client.enable(), Err(ControlError::Closed)));
        peer.join().expect("peer");
    }
}
