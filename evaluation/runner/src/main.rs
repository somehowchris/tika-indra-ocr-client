use clap::{Parser, Subcommand};
use indra_evaluate::corpus::{self, Corpus, Downloads};
use miette::IntoDiagnostic;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(version, about = "Reproduce and measure the frozen Tika OCR corpus")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
#[allow(
    clippy::large_enum_variant,
    reason = "CLI settings are parsed once; fixed stack storage avoids an otherwise unnecessary heap indirection"
)]
enum Commands {
    /// Execute one frozen arm with Docker and a preconfigured aggregate systemd slice.
    Docker(DockerArgs),
    /// Score frozen first-attempt outputs after verifying their recorded hashes.
    Score {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Pair document clusters; use --resamples 0 for descriptive estimates without intervals.
    Compare {
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 20261017)]
        seed: u64,
        #[arg(long, default_value_t = 10_000)]
        /// Zero disables resampling; otherwise require 1000..=10000 draws.
        resamples: usize,
    },
    /// Reconstruct frozen inputs and references; every final byte must match.
    Prepare {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        locks: PathBuf,
    },
    /// Measure serial Tika requests inside an attested one/two-CPU parent cgroup.
    Run {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        label: String,
        #[arg(long)]
        cgroup: PathBuf,
        #[arg(long)]
        cpus: u32,
        #[arg(long)]
        tika_pid: u32,
        #[arg(long)]
        ocr_pid: Option<u32>,
        /// Read-only process views, restricted to measured peers in Docker mode.
        #[arg(long, default_value = "/proc")]
        peer_proc_root: PathBuf,
        #[arg(long, default_value_t = 3)]
        repetitions: u16,
        #[arg(long)]
        tesseract_psm: Option<u8>,
    },
    /// Verify every frozen input/reference and the conservative English limits.
    Verify {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        root: PathBuf,
    },
    /// Acquire hash-pinned source assets. No model output influences selection.
    Fetch {
        #[arg(long)]
        lock: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        include_metadata: bool,
    },
}

#[derive(clap::Args)]
struct DockerArgs {
    #[arg(long, value_parser = ["native", "python", "tesseract"])]
    arm: String,
    #[arg(long)]
    tika_image: String,
    #[arg(long)]
    backend_image: Option<String>,
    #[arg(long)]
    evaluator: Option<PathBuf>,
    #[arg(long)]
    tika_config: PathBuf,
    #[arg(long)]
    models: Option<PathBuf>,
    #[arg(long)]
    tesseract_models_lock: Option<PathBuf>,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    protocol: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    label: String,
    #[arg(long)]
    cpus: u32,
    #[arg(long)]
    parent_slice: String,
    #[arg(long)]
    parent_cgroup: PathBuf,
    /// Run the load generator in a sibling Docker container instead of systemd-run.
    #[arg(long)]
    docker_runner: bool,
    #[arg(long, requires = "docker_runner")]
    docker_parent_receipt: Option<PathBuf>,
    /// Daemon socket every docker command uses; DOCKER_* variables are ignored.
    #[arg(long, default_value = "unix:///var/run/docker.sock")]
    docker_host: String,
    #[arg(long)]
    affinity: String,
    #[arg(long, default_value_t = 3)]
    repetitions: u16,
    #[arg(long, default_value_t = 1)]
    threads: u8,
    #[arg(long, default_value_t = 64)]
    batch_size: u8,
    #[arg(long)]
    math: bool,
    #[arg(long)]
    tesseract_psm: Option<u8>,
}

impl DockerArgs {
    fn run(self) -> miette::Result<()> {
        use indra_evaluate::docker::{Arm, DockerOptions};
        let arm = match self.arm.as_str() {
            "native" => Arm::Native,
            "python" => Arm::Python,
            "tesseract" => Arm::Tesseract,
            _ => return Err(miette::miette!("unsupported OCR arm")),
        };
        let evaluator = match self.evaluator {
            Some(path) => path,
            None => std::env::current_exe().into_diagnostic()?,
        };
        indra_evaluate::docker::run_arm(&DockerOptions {
            arm,
            tika_image: &self.tika_image,
            backend_image: self.backend_image.as_deref(),
            evaluator: &evaluator,
            tika_config: &self.tika_config,
            models: self.models.as_deref(),
            tesseract_models_lock: self.tesseract_models_lock.as_deref(),
            corpus: &self.corpus,
            manifest: &self.manifest,
            protocol: &self.protocol,
            output: &self.output,
            label: &self.label,
            cpus: self.cpus,
            parent_slice: &self.parent_slice,
            parent_cgroup: &self.parent_cgroup,
            docker_runner: self.docker_runner,
            docker_parent_receipt: self.docker_parent_receipt.as_deref(),
            docker_host: &self.docker_host,
            affinity: &self.affinity,
            repetitions: self.repetitions,
            threads: self.threads,
            batch_size: self.batch_size,
            math: self.math,
            tesseract_psm: self.tesseract_psm,
        })
        .into_diagnostic()
    }
}
#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Corpus(#[from] corpus::CorpusError),
    #[error("evaluation I/O failed")]
    Io(#[from] std::io::Error),
    #[error("source HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("invalid acquisition contract: {0}")]
    Invalid(&'static str),
}

fn fetch(lock: &Path, root: &Path, include_metadata: bool) -> Result<(), Error> {
    let downloads: Downloads = corpus::read_json(lock)?;
    if downloads.schema_version != 1 {
        return Err(Error::Invalid("unknown schema"));
    }
    let total = downloads
        .assets
        .iter()
        .try_fold(0_u64, |a, d| a.checked_add(d.asset.bytes))
        .ok_or(Error::Invalid("size overflow"))?;
    if total > downloads.max_total_bytes || downloads.max_total_bytes > 1_200_000_000 {
        return Err(Error::Invalid(
            "source assets exceed frozen total byte ceiling",
        ));
    }
    fs::create_dir_all(root)?;
    let client = indra_evaluate::http::acquisition_client()?;
    for download in &downloads.assets {
        if !include_metadata && download.asset.path.starts_with("metadata") {
            continue;
        }
        let dest = corpus::safe_path(root, &download.asset.path)?;
        if dest.exists() {
            corpus::verify_asset(root, &download.asset)?;
            eprintln!("verified {}", download.asset.path.display());
            continue;
        }
        if download.asset.bytes > download.max_bytes {
            return Err(Error::Invalid("asset exceeds its byte ceiling"));
        }
        let parent = dest
            .parent()
            .ok_or(Error::Invalid("missing asset parent"))?;
        fs::create_dir_all(parent)?;
        let mut partial = PathBuf::from(&dest);
        partial.set_extension("download-partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        eprintln!("fetching {}", download.asset.path.display());
        let result = (|| -> Result<(), Error> {
            let mut response = client.get(&download.url).send()?.error_for_status()?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(Error::Invalid("source did not return HTTP200"));
            }
            if response
                .content_length()
                .is_some_and(|len| len != download.asset.bytes)
            {
                return Err(Error::Invalid(
                    "HTTP content length differs from frozen asset",
                ));
            }
            let mut buffer = [0_u8; 65_536];
            let mut hash = Sha256::new();
            let mut bytes = 0_u64;
            loop {
                let read = response.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
                if bytes > download.asset.bytes {
                    return Err(Error::Invalid("HTTP body exceeds frozen asset"));
                }
                file.write_all(&buffer[..read])?;
                hash.update(&buffer[..read]);
            }
            if bytes != download.asset.bytes
                || format!("{:x}", hash.finalize()) != download.asset.sha256
            {
                return Err(Error::Invalid(
                    "download SHA-256 or size differs; source must not be substituted",
                ));
            }
            file.sync_all()?;
            fs::rename(&partial, &dest)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err()
            && let Err(error) = fs::remove_file(&partial)
        {
            eprintln!("partial cleanup failed for {}: {error}", partial.display());
        }
        result?;
        eprintln!("acquired {}", download.asset.path.display());
    }
    Ok(())
}

fn main() -> miette::Result<()> {
    match Cli::parse().command {
        Commands::Docker(args) => args.run()?,
        Commands::Score {
            manifest,
            root,
            run,
            output,
        } => {
            indra_evaluate::score::score(&indra_evaluate::score::ScoreOptions {
                manifest: &manifest,
                root: &root,
                run: &run,
                output: &output,
            })
            .into_diagnostic()?;
        }
        Commands::Compare {
            left,
            right,
            output,
            seed,
            resamples,
        } => {
            indra_evaluate::score::compare(&indra_evaluate::score::CompareOptions {
                left: &left,
                right: &right,
                output: &output,
                seed,
                resamples,
            })
            .into_diagnostic()?;
        }
        Commands::Prepare {
            manifest,
            root,
            locks,
        } => {
            let corpus: Corpus = corpus::read_json(&manifest).into_diagnostic()?;
            let report =
                indra_evaluate::prepare::prepare(&corpus, &root, &locks).into_diagnostic()?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &report).into_diagnostic()?;
            println!();
            if !report.failures.is_empty() {
                return Err(miette::miette!(
                    "frozen preparation has mismatches; see the report and retained candidates"
                ));
            }
        }
        Commands::Run {
            manifest,
            root,
            output,
            endpoint,
            label,
            cgroup,
            cpus,
            tika_pid,
            ocr_pid,
            peer_proc_root,
            repetitions,
            tesseract_psm,
        } => {
            indra_evaluate::run::run(&indra_evaluate::run::RunOptions {
                manifest: &manifest,
                root: &root,
                output: &output,
                endpoint: &endpoint,
                label: &label,
                cgroup: &cgroup,
                cpus,
                tika_pid,
                ocr_pid,
                peer_proc_root: &peer_proc_root,
                repetitions,
                tesseract_psm,
            })
            .into_diagnostic()?;
        }
        Commands::Verify { manifest, root } => {
            let corpus: Corpus = corpus::read_json(&manifest).into_diagnostic()?;
            let admission = corpus.validate(&root).into_diagnostic()?;
            serde_json::to_writer_pretty(std::io::stdout().lock(), &admission).into_diagnostic()?;
            println!();
        }
        Commands::Fetch {
            lock,
            root,
            include_metadata,
        } => fetch(&lock, &root, include_metadata).into_diagnostic()?,
    }
    Ok(())
}
