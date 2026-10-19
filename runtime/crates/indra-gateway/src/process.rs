//! All descendants inherit the Java process group. Linux subreaping also makes
//! the gateway reap orphaned parser/OCR children when it is not container PID 1.
use crate::{
    configuration::Gateway,
    error::{self, Error},
};
use rustix::process::{self, Pid, Signal, WaitOptions};
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::time::Instant;

pub struct Java {
    group: Pid,
    pub main_exit: Option<i32>,
    armed: bool,
}

impl Java {
    pub fn spawn(settings: &Gateway) -> Result<Self, Error> {
        process::set_child_subreaper(Pid::from_raw(1))
            .map_err(|_| Error::Process("cannot enable Linux child subreaping"))?;
        let child = Command::new(&settings.java)
            .args([
                "-Xms32m",
                "-Xmx128m",
                "-XX:ActiveProcessorCount=1",
                "-XX:+UseSerialGC",
                "-Djava.awt.headless=true",
                "-Djava.io.tmpdir=/tmp",
            ])
            .arg("-jar")
            .arg(&settings.jar)
            .args([
                "--host",
                &settings.backend.ip().to_string(),
                "--port",
                &settings.backend.port().to_string(),
            ])
            .arg("--config")
            .arg(&settings.tika_config)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|source| error::io("starting the configured Java executable", source))?;
        let group =
            Pid::from_raw(child.id() as i32).ok_or(Error::Process("invalid Java process ID"))?;
        // std::process::Child has no implicit kill/reap on drop. This supervisor
        // is the sole wait owner, including adopted descendants.
        drop(child);
        Ok(Self {
            group,
            main_exit: None,
            armed: true,
        })
    }

    /// Nonblocking, bounded reap work per event-loop tick. true means ECHILD:
    /// there are no owned/adopted children left, not merely no ready zombies.
    pub fn reap(&mut self) -> Result<bool, Error> {
        for _ in 0..64 {
            match process::wait(WaitOptions::NOHANG) {
                Ok(Some((pid, status))) => {
                    if pid == self.group {
                        self.main_exit = Some(status.as_raw());
                    }
                    tracing::info!(
                        event = "child_reaped",
                        main = pid == self.group,
                        exit_status = status.exit_status(),
                        signal = status.terminating_signal()
                    );
                }
                Ok(None) => return Ok(false),
                Err(rustix::io::Errno::CHILD) => return Ok(true),
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return Err(Error::Process("wait failed while reaping owned children")),
            }
        }
        Ok(false)
    }

    fn signal(&self, signal: Signal) -> Result<(), Error> {
        match process::kill_process_group(self.group, signal) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(_) => Err(Error::Process(
                "could not signal the owned Java process group",
            )),
        }
    }

    pub async fn terminate(&mut self, seconds: u64) -> Result<(), Error> {
        if self.reap()? {
            self.armed = false;
            return Ok(());
        }
        for signal in [Signal::TERM, Signal::KILL] {
            self.signal(signal)?;
            let due = Instant::now() + Duration::from_secs(seconds);
            loop {
                if self.reap()? {
                    self.armed = false;
                    return Ok(());
                }
                if Instant::now() >= due {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
        Err(Error::Process(
            "children remain after bounded TERM/KILL and reap budgets",
        ))
    }
}

impl Drop for Java {
    fn drop(&mut self) {
        if self.armed {
            if self.signal(Signal::KILL).is_err() {
                eprintln!("gateway emergency child-group termination failed");
            }
            let _ = self.reap();
        }
    }
}
