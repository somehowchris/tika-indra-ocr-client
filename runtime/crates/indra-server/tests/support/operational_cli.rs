//! Release subprocess checks exercise CLI parsing before any model/runtime work.
use std::{path::PathBuf, process::Command};

struct WorkingDirectory(PathBuf);

impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove isolated CLI directory");
    }
}

#[test]
#[ignore = "requires a release indra-server binary; included in the static native service gate"]
fn release_cli_ignores_dotenv_and_validates_offline_configuration_and_api() {
    let binary = std::env::var_os("INDRA_TEST_SERVER_BIN")
        .map_or_else(|| PathBuf::from(super::DEFAULT_BINARY), PathBuf::from);
    let directory = WorkingDirectory(
        std::env::temp_dir().join(format!("indra-release-cli-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).expect("isolated CLI working directory");
    // A debug binary would fail before even Clap parses the command.
    std::fs::write(directory.0.join(".env"), "this is not valid dotenv\n")
        .expect("malformed dotenv fixture");
    let command = || {
        let mut command = Command::new(&binary);
        command.env_clear().current_dir(&directory.0);
        command
    };
    let defaults = command()
        .arg("check-config")
        .output()
        .expect("offline defaults");
    assert!(
        defaults.status.success(),
        "release must ignore working-directory .env"
    );
    assert!(
        !directory.0.join("models").exists(),
        "checking configuration never downloads"
    );

    let config = directory.0.join("indra.toml");
    std::fs::write(&config, "[engine]\nthreads = 2\nmath = true\n").expect("explicit config file");
    let selected = command()
        .arg("--config")
        .arg(&config)
        .env("INDRA__ENGINE__THREADS", "invalid-overridden-secret")
        .args(["check-config", "--threads", "1", "--math", "false"])
        .output()
        .expect("file/environment/flag composition");
    assert!(
        selected.status.success(),
        "flags must override before typed decoding"
    );
    let stdout = std::str::from_utf8(&selected.stdout).expect("UTF-8 summary");
    assert!(stdout.contains("threads=1") && stdout.contains("math=false"));
    assert!(!stdout.contains("invalid-overridden-secret"));

    let invalid = command()
        .env("INDRA__ENGINE__THREADS", "private-error-value")
        .arg("check-config")
        .output()
        .expect("invalid typed environment");
    assert!(!invalid.status.success());
    let stderr = std::str::from_utf8(&invalid.stderr).expect("UTF-8 diagnostics");
    assert!(
        stderr.contains("engine.threads"),
        "diagnostic retains field location"
    );
    assert!(
        !stderr.contains("private-error-value"),
        "diagnostic redacts source value"
    );

    std::fs::write(
        &config,
        "[telemetry]\nprivate_unknown_key = 'private-file-value'\n",
    )
    .expect("unknown config field");
    let unknown = command()
        .arg("--config")
        .arg(&config)
        .arg("check-config")
        .output()
        .expect("unknown field rejection");
    assert!(!unknown.status.success());
    assert!(
        !std::str::from_utf8(&unknown.stderr)
            .expect("UTF-8 diagnostics")
            .contains("private-file-value")
    );
    assert!(
        !command()
            .args(["--config", "missing.toml", "check-config"])
            .status()
            .expect("explicit missing config")
            .success()
    );
    assert!(
        !command()
            .args(["check-config", "--unrecognized"])
            .status()
            .expect("unknown option")
            .success()
    );

    // A bare boolean flag must not swallow the subcommand that follows it.
    assert!(
        command()
            .args(["--math", "check-config"])
            .status()
            .expect("boolean flag before subcommand")
            .success()
    );
    // Neither command requires valid settings, a model cache, a server or CDN.
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let completion = command()
            .args(["completions", shell])
            .output()
            .expect("completion generation");
        assert!(completion.status.success());
        assert!(
            std::str::from_utf8(&completion.stdout)
                .expect("UTF-8 completion")
                .contains("indra-server")
        );
    }
    let api = command()
        .env("INDRA__SERVER__WORKERS", "invalid")
        .arg("openapi")
        .output()
        .expect("offline OpenAPI");
    assert!(
        api.status.success(),
        "API export does not initialize models or settings"
    );
    let api: serde_json::Value = serde_json::from_slice(&api.stdout).expect("generated JSON API");
    assert_eq!(
        api["paths"]["/ocr"]["post"]["requestBody"]["required"],
        true
    );
    let content = &api["paths"]["/ocr"]["post"]["requestBody"]["content"];
    for media in [
        "application/octet-stream",
        "image/png",
        "image/jpeg",
        "image/tiff",
    ] {
        assert!(content.get(media).is_some(), "raw request media {media}");
    }
    assert!(
        api["paths"]["/readyz"]["get"]["responses"]
            .get("503")
            .is_some()
    );
}
