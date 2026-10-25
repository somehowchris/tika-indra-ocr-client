//! A failed legal render must not leave a partial file that blocks the next
//! `prepare`. A stub `pdftoppm`, the only program on the evaluator's `PATH`,
//! stands in for Poppler; the frozen acquisition locks are the real ones.
#![cfg(unix)]

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::Command,
};

fn write_new(path: &Path, mode: u32, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

fn asset(path: &str, bytes: &[u8]) -> Value {
    json!({"path": path, "bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(bytes))})
}

fn prepare(manifest: &Path, root: &Path, tools: &Path) -> (bool, Value) {
    let locks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../acquisition/locks");
    let output = Command::new(env!("CARGO_BIN_EXE_indra-evaluate"))
        .arg("prepare")
        .arg("--manifest")
        .arg(manifest)
        .arg("--root")
        .arg(root)
        .arg("--locks")
        .arg(locks)
        .env("PATH", tools)
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no preparation report ({error}): {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), report)
}

#[test]
fn a_failed_render_leaves_no_partial_that_blocks_the_next_prepare() {
    let base = std::env::temp_dir().join(format!("prepare-partial-test-{}", std::process::id()));
    fs::create_dir(&base).unwrap();
    let root = base.join("corpus");
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&[0, 64, 128, 192, 255, 32], 3, 2, ExtendedColorType::L8)
        .unwrap();
    let pdf = b"SYNTHETIC PDF STAND-IN; ONLY ITS HASH IS CHECKED";
    let reference = b"Synthetic reference";
    write_new(&root.join("source/synthetic-legal.pdf"), 0o644, pdf);
    write_new(
        &root.join("prepared/synthetic-legal/page-0002.txt"),
        0o644,
        reference,
    );
    let manifest = base.join("corpus.json");
    let corpus = json!({
        "schema_version": 1, "corpus_id": "synthetic-prepare-test", "status": "frozen",
        "selection_seed": 1, "expected_primary_pages": 1, "sources": {},
        "cases": [{
            "id": "synthetic-legal-p0002", "source_id": "eurlex",
            "original_page_id": "synthetic-legal-p0002", "document_cluster": "synthetic-legal",
            "writer_cluster": null, "cohort": "primary", "task": "full_page",
            "language": "deu", "script": "Latn", "tesseract_language": "deu",
            "word_tokenizer": "whitespace", "english": {"status": "absent", "reference_letters": 0},
            "domain": "test-fixture", "appearance": "synthetic", "reference_quality": "fixture",
            "input": asset("prepared/synthetic-legal/page-0002.png", &png),
            "reference": asset("prepared/synthetic-legal/page-0002.txt", reference),
            "annotation": asset("source/synthetic-legal.pdf", pdf),
            "width": 3, "height": 2, "input_mime": "image/png",
            "audit": {"status": "reviewed", "input_reference_alignment": true,
                "reference_covers_task": true, "reading_order_reviewed": true,
                "english_reviewed": true, "transcription_policy_reviewed": true}
        }]
    });
    write_new(&manifest, 0o644, corpus.to_string().as_bytes());

    let failing = base.join("failing-tools");
    write_new(&failing.join("pdftoppm"), 0o755, b"#!/bin/sh\nexit 3\n");
    let (succeeded, report) = prepare(&manifest, &root, &failing);
    assert!(!succeeded, "{report}");
    let failure = &report["failures"][0];
    assert_eq!(failure["kind"], "image", "{report}");
    assert!(
        failure["error"]
            .as_str()
            .unwrap()
            .contains("pdftoppm exited unsuccessfully"),
        "{report}"
    );
    let partial = root.join("prepared/synthetic-legal/page-0002.png.prepare-partial.png");
    assert!(
        partial.is_file(),
        "the failed render leaves its partial file"
    );

    let rendered = base.join("rendered.png");
    write_new(&rendered, 0o644, &png);
    let working = base.join("working-tools");
    let script = format!("#!/bin/sh\nexec /bin/cat '{}'\n", rendered.display());
    write_new(&working.join("pdftoppm"), 0o755, script.as_bytes());
    let (succeeded, report) = prepare(&manifest, &root, &working);
    assert!(succeeded, "{report}");
    assert_eq!(report["failures"], json!([]), "{report}");
    assert_eq!(report["generated_files"], 1, "{report}");
    assert_eq!(report["reused_files"], 1, "{report}");
    assert_eq!(
        fs::read(root.join("prepared/synthetic-legal/page-0002.png")).unwrap(),
        png
    );
    assert!(!partial.exists());
    fs::remove_dir_all(base).unwrap();
}
