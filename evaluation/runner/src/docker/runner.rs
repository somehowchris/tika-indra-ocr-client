//! Docker-only load generator: same bounded parent and signed Tika image,
//! static executable override, read-only views of only the two declared peers.
use super::*;

impl Session<'_> {
    pub(super) fn execute_docker_runner(
        &mut self,
        tika: &Container,
        ocr_pid: Option<u32>,
        maximum_seconds: u64,
    ) -> Result<(), DockerError> {
        let elf = self.checked(
            "client-static-elf",
            Command::new("readelf")
                .env("LC_ALL", "C")
                .arg("--program-headers")
                .arg(&self.paths.evaluator),
            COMMAND_TIMEOUT,
        )?;
        if read_text(&elf.stdout, 65536)?.contains("INTERP") {
            return Err(DockerError::Invalid(
                "Docker client requires a static executable without a dynamic interpreter",
            ));
        }
        let name = format!("indra-client-{}", self.token);
        self.runner = Some(OwnedContainer {
            name: String::from(name.as_str()),
            image: String::from(tika.image.as_str()),
            id: None,
        });
        let owner = fs::metadata(&self.paths.requests)?;
        let mut command = base_container(self.options, &name, &self.token);
        command.args([
            "--cgroupns=host",
            "--network",
            &format!("container:{}", tika.id),
            "--user",
            &format!("{}:{}", owner.uid(), owner.gid()),
            "--memory=1073741824",
            "--memory-swap=1073741824",
            "--stop-timeout=10",
            "--entrypoint=/indra-evaluate",
        ]);
        bind(&mut command, &self.paths.evaluator, "/indra-evaluate")?;
        bind(&mut command, &self.paths.corpus, "/corpus")?;
        bind(&mut command, &self.paths.manifest, "/manifest.json")?;
        bind(&mut command, Path::new("/sys/fs/cgroup"), "/sys/fs/cgroup")?;
        for pid in [Some(tika.state.pid), ocr_pid].into_iter().flatten() {
            bind(
                &mut command,
                &PathBuf::from(format!("/proc/{pid}")),
                &format!("/peers/{pid}"),
            )?;
        }
        command.args([
            "--mount",
            &format!(
                "type=bind,src={},dst=/output",
                path_text(&self.paths.requests)?
            ),
            &tika.image,
            "run",
            "--manifest",
            "/manifest.json",
            "--root",
            "/corpus",
            "--output",
            "/output",
            "--endpoint",
            "http://127.0.0.1:9998/tika",
            "--peer-proc-root",
            "/peers",
            "--label",
            self.options.label,
            "--cgroup",
            path_text(self.options.parent_cgroup)?,
            "--cpus",
            &self.options.cpus.to_string(),
            "--tika-pid",
            &tika.state.pid.to_string(),
            "--repetitions",
            &self.options.repetitions.to_string(),
        ]);
        if let Some(pid) = ocr_pid {
            command.args(["--ocr-pid", &pid.to_string()]);
        }
        if let Some(psm) = self.options.tesseract_psm {
            command.args(["--tesseract-psm", &psm.to_string()]);
        }
        self.checked("client-create", &mut command, COMMAND_TIMEOUT)?;
        let inspected = self.inspect(&name, "client-created-inspect")?;
        self.validate_runner(&inspected)?;
        self.runner
            .as_mut()
            .ok_or(DockerError::Invalid("missing client owner"))?
            .id = Some(String::from(inspected.id.as_str()));
        self.checked(
            "measured-run",
            docker(self.options.docker_host).args([
                "container",
                "start",
                "--attach",
                &inspected.id,
            ]),
            Duration::from_secs(maximum_seconds + 30),
        )?;
        let finished = self.inspect(&inspected.id, "client-finished-inspect")?;
        self.validate_runner(&finished)?;
        if finished.state.running || finished.state.exit_code != 0 || finished.state.oom_killed {
            return Err(DockerError::Invalid(
                "measured Docker client did not exit successfully",
            ));
        }
        Ok(())
    }

    fn validate_runner(&self, actual: &Container) -> Result<(), DockerError> {
        let owned = self
            .runner
            .as_ref()
            .ok_or(DockerError::Invalid("missing Docker client owner"))?;
        let host = &actual.host_config;
        if actual.config.labels.owner.as_deref() != Some(self.token.as_str())
            || actual.image != owned.image
            || owned.id.as_ref().is_some_and(|id| actual.id != *id)
            || host.cgroup_parent != self.options.parent_slice
            || host.nano_cpus != u64::from(self.options.cpus) * 1_000_000_000
            || host.cpuset_cpus != self.options.affinity
            || host.memory != 1024 * 1024 * 1024
            || host.memory_swap != host.memory
            || !host.readonly_rootfs
            || host.privileged
            || !host.cap_drop.iter().any(|cap| cap == "ALL")
            || !host
                .security_opt
                .iter()
                .any(|option| option == "no-new-privileges")
        {
            return Err(DockerError::Invalid(
                "Docker client ownership or resource identity differs",
            ));
        }
        Ok(())
    }

    pub(super) fn cleanup_docker_runner(&mut self) -> Result<(), DockerError> {
        let Some(owned) = &self.runner else {
            return Ok(());
        };
        let name = String::from(owned.name.as_str());
        let listed = self.checked(
            "client-cleanup-list",
            docker(self.options.docker_host).args([
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.ID}}",
            ]),
            COMMAND_TIMEOUT,
        )?;
        if read_text(&listed.stdout, 4096)?.trim().is_empty() {
            return Ok(());
        }
        let actual = self.inspect(&name, "client-cleanup-inspect")?;
        self.validate_runner(&actual)?;
        self.checked(
            "client-remove",
            docker(self.options.docker_host).args(["container", "rm", "--force", &actual.id]),
            COMMAND_TIMEOUT,
        )?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct SetupReceipt {
    schema_version: u8,
    slice: String,
    path: PathBuf,
    device: u64,
    inode: u64,
    setup_cpu_usage_usec: u64,
    setup_memory_peak_bytes: u64,
    sentinel_pid: u32,
    sentinel_cgroup: String,
    sentinel_start_ticks: u64,
}

pub(super) fn admit_docker_parent(
    options: &DockerOptions<'_>,
    parent: &ParentBudget,
) -> Result<ParentCounters, DockerError> {
    let receipt_path = options.docker_parent_receipt.ok_or(DockerError::Invalid(
        "Docker runner requires the exact owned parent setup receipt",
    ))?;
    let receipt: SetupReceipt = serde_json::from_str(&read_text(receipt_path, 65536)?)?;
    let baseline = parent_counters(&parent.path)?;
    let sentinel_path =
        Path::new("/sys/fs/cgroup").join(receipt.sentinel_cgroup.trim_start_matches('/'));
    if !sentinel_path.starts_with(&parent.path)
        || sentinel_path == parent.path
        || read_text(&sentinel_path.join("cgroup.procs"), 4096)?.trim()
            != receipt.sentinel_pid.to_string()
        || read_text(
            &PathBuf::from(format!("/proc/{}/cgroup", receipt.sentinel_pid)),
            4096,
        )?
        .trim()
            != format!("0::{}", receipt.sentinel_cgroup)
    {
        return Err(DockerError::Invalid(
            "owned sentinel process or cgroup identity differs",
        ));
    }
    let stat = read_text(
        &PathBuf::from(format!("/proc/{}/stat", receipt.sentinel_pid)),
        4096,
    )?;
    let start = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|value| value.parse::<u64>().ok());
    if start != Some(receipt.sentinel_start_ticks)
        || !receipt.matches(parent, options.parent_slice, &baseline)
    {
        return Err(DockerError::Invalid(
            "Docker setup receipt or empty-parent identity differs",
        ));
    }
    // The bounded sleeping sentinel remains charged to this parent. Setup and
    // lifetime memory are disclosed; neither peaks nor charges are subtracted.
    Ok(baseline)
}

impl SetupReceipt {
    fn matches(&self, parent: &ParentBudget, slice: &str, baseline: &ParentCounters) -> bool {
        self.schema_version == 1
            && self.slice == slice
            && self.path == parent.path
            && self.device == parent.device
            && self.inode == parent.inode
            && self.setup_cpu_usage_usec == baseline.cpu_usage_usec
            && self.setup_memory_peak_bytes == baseline.memory_peak_bytes
            && baseline.descendants == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_receipt_rejects_replaced_parent_extra_cpu_or_live_children() {
        let mut parent = ParentBudget {
            path: PathBuf::from("/sys/fs/cgroup/owned.slice"),
            device: 31,
            inode: 42,
            cpu_max: String::from("100000 100000"),
            memory_max: MEMORY_BYTES,
            memory_swap_max: 0,
            cpuset_effective: String::from("0"),
            ancestors: ArrayVec::new(),
        };
        let mut counters = ParentCounters {
            cpu_usage_usec: 123,
            cpu_stat: String::new(),
            memory_current_bytes: 1024,
            memory_peak_bytes: 4096,
            memory_events: String::new(),
            descendants: 1,
            dying_descendants: 1,
        };
        let receipt = SetupReceipt {
            schema_version: 1,
            slice: String::from("owned.slice"),
            path: PathBuf::from("/sys/fs/cgroup/owned.slice"),
            device: 31,
            inode: 42,
            setup_cpu_usage_usec: 123,
            setup_memory_peak_bytes: 4096,
            sentinel_pid: 123,
            sentinel_cgroup: String::from("/owned.slice/docker-test.scope"),
            sentinel_start_ticks: 99,
        };
        assert!(
            receipt.matches(&parent, "owned.slice", &counters),
            "disclosed dying memory charges are retained"
        );
        parent.inode += 1;
        assert!(
            !receipt.matches(&parent, "owned.slice", &counters),
            "path reuse is not identity"
        );
        parent.inode -= 1;
        counters.cpu_usage_usec += 1;
        assert!(
            !receipt.matches(&parent, "owned.slice", &counters),
            "unrecorded work cannot reuse the parent"
        );
        counters.cpu_usage_usec -= 1;
        counters.descendants = 2;
        assert!(
            !receipt.matches(&parent, "owned.slice", &counters),
            "a live child is never a clean start"
        );
        counters.descendants = 1;
        counters.memory_peak_bytes += 1;
        assert!(
            !receipt.matches(&parent, "owned.slice", &counters),
            "unexpected peak cannot be hidden"
        );
    }
}
