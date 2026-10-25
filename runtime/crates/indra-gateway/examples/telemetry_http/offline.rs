//! Offline replay of the same bounded wire, privacy and parent-graph verifier.
//! No runtime, receiver, child process, cgroup write or model load is started.
use super::{Failure, INPUTS, NAMES, audit, check, create_new, sentry};
use clap::{Parser, Subcommand};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
struct Command {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Recheck a captured run without inference or network access.
    Verify {
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        goldens: Option<PathBuf>,
        #[arg(long)]
        identity: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

pub(super) fn run() -> Result<(), Failure> {
    let Action::Verify {
        evidence,
        goldens,
        identity,
        output,
    } = Command::parse().action;
    let receipt = verify(&evidence, goldens.as_deref(), &identity)?;
    let mut file = create_new(&output)?;
    serde_json::to_writer_pretty(&mut file, &receipt)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn regular(path: &Path, maximum: u64) -> Result<(), Failure> {
    let metadata = fs::symlink_metadata(path)?;
    check(
        metadata.is_file() && metadata.len() <= maximum,
        "offline evidence file type/size",
    )
}
fn json(path: &Path, maximum: u64) -> Result<Value, Failure> {
    regular(path, maximum)?;
    Ok(serde_json::from_reader(File::open(path)?)?)
}
fn file_sha(path: &Path) -> Result<[u8; 32], Failure> {
    regular(path, 64 * 1024 * 1024)?;
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 65536];
    loop {
        let used = file.read(&mut bytes)?;
        if used == 0 {
            break;
        }
        hash.update(&bytes[..used]);
    }
    Ok(hash.finalize().into())
}
fn assets(path: &Path, expected: &Value) -> Result<(), Failure> {
    regular(path, 16384)?;
    let text = fs::read_to_string(path)?;
    static ROLES: [&str; 5] = ["bridge", "gateway", "java", "native", "tika"];
    let dependencies = expected["trace_dependencies_sha256"]
        .as_object()
        .ok_or(Failure::Gate("offline dependency identity absent"))?;
    check(dependencies.len() == 16, "offline dependency count")?;
    let mut seen = [false; 21];
    let mut used = 0;
    for line in text.lines() {
        let row: Value = serde_json::from_str(line)?;
        let role = row["role"]
            .as_str()
            .ok_or(Failure::Gate("offline asset role"))?;
        let (index, digest) = if role == "trace_dependency" {
            let name = row["path"]
                .as_str()
                .and_then(|p| p.rsplit('/').next())
                .ok_or(Failure::Gate("offline dependency name"))?;
            let (i, (_, digest)) = dependencies
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key.as_str() == name)
                .ok_or(Failure::Gate("offline unexpected dependency"))?;
            (i + 5, digest)
        } else {
            let i = ROLES
                .iter()
                .position(|name| *name == role)
                .ok_or(Failure::Gate("offline unexpected role"))?;
            (i, &expected["role_sha256"][role])
        };
        check(
            !seen[index]
                && digest.as_str().is_some_and(|s| s.len() == 64)
                && row["sha256"] == *digest,
            "offline duplicate/mismatched asset",
        )?;
        seen[index] = true;
        used += 1;
    }
    check(
        used == 21 && seen.into_iter().all(|v| v),
        "offline incomplete actual assets",
    )
}
fn outcomes(path: &Path, goldens: &Path) -> Result<(), Failure> {
    regular(path, 65536)?;
    let text = fs::read_to_string(path)?;
    let mut expected = [([0_u8; 32], 0_u64); 3];
    for (i, name) in NAMES.iter().enumerate() {
        let reference = goldens.join(format!("golden-{name}.txt"));
        regular(&reference, 16384)?;
        expected[i] = (file_sha(&reference)?, fs::metadata(reference)?.len());
    }
    let mut used = 0;
    for (index, line) in text.lines().enumerate() {
        check(index < 12, "offline excess request outcomes")?;
        let row: Value = serde_json::from_str(line)?;
        let (digest, size) = expected[INPUTS[index]];
        check(
            row["sequence"].as_u64() == Some(index as u64)
                && row["fixture"] == NAMES[INPUTS[index]]
                && row["status"] == "success"
                && row["http_status"] == 200
                && row["output_bytes"].as_u64() == Some(size)
                && row["failure"].is_null(),
            "offline unsuccessful/wrong request",
        )?;
        let actual = row["output_sha256"]
            .as_array()
            .ok_or(Failure::Gate("offline request digest absent"))?;
        check(
            actual.len() == 32
                && actual
                    .iter()
                    .zip(digest)
                    .all(|(a, b)| a.as_u64() == Some(u64::from(b))),
            "offline golden response hash differs",
        )?;
        used += 1;
    }
    check(used == 12, "offline missing request outcomes")
}
fn verify(evidence: &Path, goldens: Option<&Path>, identity: &Path) -> Result<Value, Failure> {
    check(
        fs::symlink_metadata(evidence)?.is_dir(),
        "offline run directory",
    )?;
    let summary = json(&evidence.join("summary.json"), 65536)?;
    let identity = json(identity, 65536)?;
    let protocol = format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../../../../validation/telemetry-http-protocol.json"
        ))
    );
    check(
        summary["complete"] == true
            && summary["child_cleanup_complete"] == true
            && summary["protocol_sha256"] == protocol
            && identity["protocol_sha256"] == protocol,
        "offline incomplete or wrong protocol",
    )?;
    let arm = summary["arm"]
        .as_str()
        .ok_or(Failure::Gate("offline arm absent"))?;
    check(
        matches!(arm, "reference" | "off" | "enabled"),
        "offline unknown arm",
    )?;
    assets(&evidence.join("assets.jsonl"), &identity)?;
    regular(&evidence.join("spans.json"), 2 * 1024 * 1024)?;
    let mut captured = sentry::Captured::default();
    let mut exports = 0;
    let mut bytes = 0_u64;
    for entry in fs::read_dir(evidence)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with("sentry-") {
            regular(&entry.path(), 65536)?;
            let data = fs::read(entry.path())?;
            sentry::verify_envelope(&data)?;
            captured.envelopes += 1;
            captured.gateway_events += 1;
            captured.bytes += data.len();
            check(
                captured.envelopes <= 8 && captured.bytes <= 262144,
                "offline Sentry aggregate cap",
            )?;
        } else if name.starts_with("otlp-") {
            regular(&entry.path(), 262144)?;
            exports += 1;
            bytes += entry.metadata()?.len();
            check(
                exports <= 64 && bytes <= 4194304,
                "offline OTLP aggregate cap",
            )?;
        }
    }
    if arm == "reference" {
        check(
            summary["planned_requests"] == 0
                && summary["successful_requests"] == 0
                && fs::metadata(evidence.join("requests.jsonl"))?.len() == 0,
            "offline reference outcomes",
        )?;
    } else {
        check(
            summary["planned_requests"] == 12 && summary["successful_requests"] == 12,
            "offline success denominator",
        )?;
        outcomes(
            &evidence.join("requests.jsonl"),
            goldens.ok_or(Failure::Gate("offline goldens required"))?,
        )?;
    }
    let cohort = summary["cohort_exports_observed"].as_u64().unwrap_or(0);
    let audit = audit::verify(
        evidence,
        arm == "enabled",
        captured,
        usize::try_from(cohort).map_err(|_| Failure::Gate("offline cohort count overflow"))?,
        goldens,
    )?;
    let audit = serde_json::to_value(audit)?;
    check(
        audit == summary["export_audit"],
        "offline actual audit differs from retained summary",
    )?;
    Ok(
        serde_json::json!({"schema_version":1,"scope":"Offline replay of existing wire/privacy/graph verifier; no inference or timing. HTTP exporter header gates were runtime checks; original headers are not retained for offline replay.",
        "producer_source":identity["source_sha"],"producer_binary_sha256":identity["binary_sha256"],
        "verifier_binary_sha256":format!("{:x}", Sha256::digest(fs::read(std::env::current_exe()?)?)),
        "protocol_sha256":protocol,"arm":arm,"cpus":summary["cpus"],
        "successful_requests":summary["successful_requests"],"accepted":true,"actual_export_audit":audit}),
    )
}
