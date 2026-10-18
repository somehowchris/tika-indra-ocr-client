//! Bounded Linux perf control: every command requires exactly one `ack\n`.
use std::{
    path::Path,
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fd::OwnedFd,
    fs::{FileType, Mode, OFlags, fstat, open},
    io::{Errno, read, write},
};
use thiserror::Error;

static EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("perf control I/O failed")]
    Io(#[from] Errno),
    #[error("perf control requires two distinct existing FIFOs")]
    Fifo,
    #[error("perf acknowledgement deadline expired")]
    Deadline,
    #[error("perf control peer closed before a complete exchange")]
    Closed,
    #[error("perf acknowledgement protocol failed: {0}")]
    Protocol(&'static str),
}

pub struct Control {
    command: OwnedFd,
    acknowledgement: OwnedFd,
    timeout: Duration,
}

impl Control {
    pub fn open(command: &Path, acknowledgement: &Path) -> Result<Self, ControlError> {
        Self::open_with_timeout(command, acknowledgement, EXCHANGE_TIMEOUT)
    }

    pub(crate) fn open_with_timeout(
        command: &Path,
        acknowledgement: &Path,
        timeout: Duration,
    ) -> Result<Self, ControlError> {
        if timeout.is_zero() || timeout > EXCHANGE_TIMEOUT {
            return Err(ControlError::Protocol("invalid bounded exchange timeout"));
        }
        let flags = OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let acknowledgement = open(acknowledgement, flags | OFlags::RDONLY, Mode::empty())?;
        let command = open(command, flags | OFlags::WRONLY, Mode::empty())?;
        let acknowledgement_stat = fstat(&acknowledgement)?;
        let command_stat = fstat(&command)?;
        if FileType::from_raw_mode(acknowledgement_stat.st_mode) != FileType::Fifo
            || FileType::from_raw_mode(command_stat.st_mode) != FileType::Fifo
            || (acknowledgement_stat.st_dev == command_stat.st_dev
                && acknowledgement_stat.st_ino == command_stat.st_ino)
        {
            return Err(ControlError::Fifo);
        }
        Ok(Self {
            command,
            acknowledgement,
            timeout,
        })
    }

    pub fn enable(&mut self) -> Result<(), ControlError> {
        self.exchange(b"enable\n")
    }

    pub fn disable(&mut self) -> Result<(), ControlError> {
        self.exchange(b"disable\n")
    }

    fn exchange(&mut self, command: &[u8]) -> Result<(), ControlError> {
        let due = Instant::now() + self.timeout;
        self.no_queued_ack(due)?;
        let mut written = 0;
        while written < command.len() {
            remaining(due)?;
            match write(&self.command, &command[written..]) {
                Ok(0) => return Err(ControlError::Closed),
                Ok(count) => written += count,
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => wait(&self.command, PollFlags::OUT, due)?,
                Err(error) => return Err(error.into()),
            }
        }
        let mut ack = [0_u8; 4];
        let mut received = 0;
        while received < ack.len() {
            remaining(due)?;
            match read(&self.acknowledgement, &mut ack[received..]) {
                Ok(0) => return Err(ControlError::Closed),
                Ok(count) => received += count,
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => wait(&self.acknowledgement, PollFlags::IN, due)?,
                Err(error) => return Err(error.into()),
            }
        }
        if &ack != b"ack\n" {
            return Err(ControlError::Protocol(
                "expected exactly ack followed by newline",
            ));
        }
        // Reject a queued suffix or extra ACK, including after the final disable.
        // EOF after an already complete ACK is a completed exchange.
        self.no_queued_ack(due)
    }

    fn no_queued_ack(&self, due: Instant) -> Result<(), ControlError> {
        loop {
            remaining(due)?;
            match read(&self.acknowledgement, &mut [0_u8; 1]) {
                Ok(0) | Err(Errno::AGAIN) => return Ok(()),
                Ok(_) => {
                    return Err(ControlError::Protocol(
                        "unexpected queued acknowledgement bytes",
                    ));
                }
                Err(Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn remaining(due: Instant) -> Result<Duration, ControlError> {
    due.checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(ControlError::Deadline)
}

fn wait(fd: &OwnedFd, events: PollFlags, due: Instant) -> Result<(), ControlError> {
    loop {
        let timeout = Timespec::try_from(remaining(due)?)
            .map_err(|_| ControlError::Protocol("unrepresentable poll timeout"))?;
        let mut fds = [PollFd::new(fd, events)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Err(ControlError::Deadline),
            Ok(_) => {
                let flags = fds[0].revents();
                if flags.contains(events) {
                    return Ok(());
                }
                if flags.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
                    return Err(ControlError::Closed);
                }
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}
