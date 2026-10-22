//! Owned subprocesses and bounded cleanup, matching the real trace acceptance harness.
use super::{Failure, check, create_new};
use rustix::process::{self, Pid, Signal, WaitOptions};
use std::{
    fs, io,
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub(super) struct Process {
    pub(super) child: Child,
    group: Pid,
    armed: bool,
}

impl Process {
    pub(super) fn start(
        mut command: Command,
        name: &'static str,
        output: &Path,
    ) -> Result<Self, Failure> {
        let child = command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(create_new(&output.join(format!("{name}.stdout")))?)
            .stderr(create_new(&output.join(format!("{name}.stderr")))?)
            .spawn()?;
        let group = Pid::from_raw(child.id() as i32).ok_or(Failure::Gate("invalid child PID"))?;
        Ok(Self {
            child,
            group,
            armed: true,
        })
    }

    pub(super) async fn shutdown(&mut self, grace: Duration) -> Result<Option<i32>, Failure> {
        process::kill_process(self.group, Signal::TERM).map_err(io::Error::from)?;
        let deadline = Instant::now() + grace;
        loop {
            if let Some(status) = self.child.try_wait()? {
                check(
                    process::test_kill_process_group(self.group) == Err(rustix::io::Errno::SRCH),
                    "descendant remains after its watchdog exits",
                )?;
                self.armed = false;
                return Ok(status.code());
            }
            check(Instant::now() < deadline, "process shutdown deadline")?;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.armed {
            let _ = process::kill_process_group(self.group, Signal::KILL);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                match self.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        }
    }
}

pub(super) fn kill_owned_children() -> Result<(), Failure> {
    let pid = std::process::id();
    let children = format!("/proc/self/task/{pid}/children");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let input = fs::read_to_string(&children)?;
        check(input.len() <= 8192, "owned child PID inventory cap")?;
        if input.trim().is_empty() {
            return Ok(());
        }
        for raw in input.split_ascii_whitespace() {
            let raw: i32 = raw
                .parse()
                .map_err(|_| Failure::Gate("invalid adopted child PID"))?;
            let child = Pid::from_raw(raw).ok_or(Failure::Gate("zero adopted child PID"))?;
            match process::kill_process(child, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
        }
        while matches!(process::wait(WaitOptions::NOHANG), Ok(Some(_))) {}
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(Failure::Gate("owned child cleanup deadline"))
}

pub(super) async fn reap_all(grace: Duration) -> Result<bool, Failure> {
    let deadline = Instant::now() + grace;
    loop {
        match process::wait(WaitOptions::NOHANG) {
            Err(rustix::io::Errno::CHILD) => return Ok(true),
            Err(error) => return Err(io::Error::from(error).into()),
            Ok(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(2)).await
            }
            Ok(_) => return Ok(false),
        }
    }
}
