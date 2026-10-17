#!/usr/bin/env python3
"""Collect completed numeric evidence without redistributing public OCR text."""

import argparse
import hashlib
import json
import shutil
from pathlib import Path


RUNS = [
    ("raw-1", "Synthetic images", 1, 1),
    ("raw-2-verified", "Synthetic images", 2, 2),
    ("tika-stock-1", "Tika OCR-only, distro baseline, 300 DPI", 1, 1),
    ("tika-stock-2", "Tika OCR-only, distro baseline, 300 DPI", 2, 2),
    ("tika-1", "Tika OCR-only, tessdata_best baseline, 300 DPI", 1, 1),
    ("tika-2", "Tika OCR-only, tessdata_best baseline, 300 DPI", 2, 2),
    ("native-1", "Native PDF AUTO control", 1, 1),
    ("native-2", "Native PDF AUTO control", 2, 2),
    ("batch-1", "Matched batch-size ablation", 1, 1),
    ("threads-2-completed", "Matched thread-count ablation", 2, 1),
    ("math-1", "Math toggle on ordinary Latin text", 1, 1),
    ("public-1", "Public legacy regression diagnostics", 1, 1),
    ("public-2", "Public legacy regression diagnostics", 2, 2),
]


def number(value, places=3):
    return "—" if value is None else f"{value:.{places}f}"


def batch_totals(cohorts):
    hist = {}
    padded = valid = 0
    for cohort in cohorts:
        metrics = cohort["backend_metrics"]
        padded += metrics["padded_pixels"]
        valid += metrics["valid_pixels"]
        for key, value in metrics["actual_tensor_batch_histogram"].items():
            hist[key] = hist.get(key, 0) + value
    sizes = [int(key.rsplit(":B", 1)[1]) for key in hist]
    return hist, (min(sizes), max(sizes)) if sizes else None, 1 - valid / padded if padded else None


def copy_numeric(source, out):
    out.mkdir(parents=True, exist_ok=True)
    for filename in ("summary.json", "cohorts.json", "environment.json", "requests.jsonl", "recovery.json"):
        if (source / filename).is_file():
            shutil.copy2(source / filename, out / filename)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results-root", type=Path, required=True)
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    destination = args.repository / "benchmarks/results"
    destination.mkdir(parents=True, exist_ok=True)
    report = [
        "# Measured OCR results", "",
        "Measured locally on 2026-10-17 using native processes and the verified static GNU/glibc bridge.",
        "These are small reproducible experiments, not production accuracy or p99 SLO guarantees.",
        "All application processes shared CPU 2 for the one-CPU runs or CPUs 2 and 3 for the two-CPU runs;",
        "the load generator/monitor used CPU 8. No GPU, external OCR API, Docker, or Kubernetes runtime was used.", "",
        "The guest exposes CPUs 2 and 3 as different core IDs, with no exposed sibling on either logical CPU.",
        "This virtualized/shared environment still does not establish exclusive physical cores or portable scaling.",
        "Two-CPU throughput runs use concurrency 2; one-CPU runs use concurrency 1. Compare latency in that context.", "",
        "Actual batch-shape instrumentation was enabled, and its unisolated overhead is included in Indra timings.",
        "The frozen measured backend is archived; later production recovery/health checks are separate validation.", "",
        "## Primary synthetic comparisons", "",
        "Raw service comparisons use 12 images and five source-document clusters. Tika OCR-only uses those",
        "12 images plus three PDFs, 15 cases total. Several renditions reuse one source text. Accuracy is scored",
        "once per case; three timing repetitions do not increase the number of independent documents.",
        "`Stock`/`distro` denotes this host's Tesseract 5.3.4 and hashed installed English model. The separately",
        "pinned `tessdata_best` model is a different baseline. Native Tika used the recorded Java 17 executable.",
        "Use each paired stock-model run to discuss replacement of that baseline; label best-model ratios explicitly.",
        "The large error-rate difference is concentrated in the two-column and multilingual challenge cases;",
        "the explicitly configured Tesseract language is `eng`. It is not a claim of a global language-agnostic optimum.", "",
        "CER/WER below use NFC codepoints and collapsed whitespace while retaining case/punctuation/reading order.",
        "Strict whitespace scores are in the JSON evidence. Compare accuracy only within a matched corpus.", "",
        "A post hoc sensitivity split excluding the multilingual challenge, two-column reading-order case, and blank",
        "shows that ordinary-text accuracy was close. This is a diagnostic split, not a revised acceptance corpus:", "",
        "| Path | Reference codepoints | Distro Tesseract errors | Tesseract best errors | Indra errors |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for name, title, baseline_name in (("raw-1", "Raw images", "tesseract-best"),
                                       ("tika-1", "Tika OCR-only, best baseline", "tika-tesseract-best"),
                                       ("tika-stock-1", "Tika OCR-only, distro baseline", "tika-tesseract-stock")):
        source = args.results_root / f"ocr-results-{name}/requests.jsonl"
        if source.is_file():
            rows = [json.loads(line) for line in source.read_text().splitlines()]
            totals = {}
            for row in rows:
                if row["phase"] != "measured" or row["repetition"] != 0 or row["kind"] in (
                        "multilingual_latin_challenge", "two_column_reading_order", "blank"):
                    continue
                value = totals.setdefault(row["engine"], [0, 0])
                value[0] += row["score"]["reference_codepoints"]
                value[1] += row["score"]["character_errors"]
            prefix = "tika-" if name.startswith("tika") else ""
            baseline = totals[baseline_name]
            distro = totals.get(prefix + "tesseract-distro", totals.get(prefix + "tesseract-stock", [0, "—"]))[1]
            best = totals.get(prefix + "tesseract-best", [0, "—"])[1]
            report.append(f"| {title} | {baseline[0]} | {distro} | {best} | {totals[prefix + 'indra'][1]} |")
    report += ["", "These slice scores are identical across the valid one/two-CPU runs. The full-corpus scores remain below.", ""]
    structured = {"runs": [], "pending": [], "excluded_runs": ["raw-2", "threads-2"]}
    for name, title, budget, concurrency in RUNS:
        source = args.results_root / f"ocr-results-{name}"
        if not (source / "summary.json").exists():
            structured["pending"].append(name)
            continue
        summary = json.loads((source / "summary.json").read_text())
        cohorts = json.loads((source / "cohorts.json").read_text())
        environment = json.loads((source / "environment.json").read_text())
        request_rows = [json.loads(line) for line in (source / "requests.jsonl").read_text().splitlines()]
        errors = [c.get("validation_error") for c in cohorts if c.get("validation_error")]
        valid = not summary["affinity_escapes"] and not errors and all(
            c["resources"]["proc_available"] and c["resources"]["thread_affinity_checks"] > 0 for c in cohorts)
        out = destination / name
        copy_numeric(source, out)
        configuration = out / "configuration"
        configuration.mkdir(exist_ok=True)
        (configuration / "plan.snapshot.json").write_text(json.dumps(environment["plan"], indent=2) + "\n")
        # Plans are embedded at launch. The generated Tika configuration files
        # are immutable per run and retained separately from deployment files.
        plan_directory = Path(environment["arguments"]["plan"]).parent
        for pattern in ("*.xml", "*-stack.json"):
            for path in plan_directory.glob(pattern):
                shutil.copy2(path, configuration / path.name)
        if name.startswith("tika-stock-") and (plan_directory / "tesseract-installation.txt").is_file():
            shutil.copy2(plan_directory / "tesseract-installation.txt", configuration / "tesseract-installation.txt")
        public = name.startswith("public-")
        if not public:
            for directory in source.iterdir():
                if directory.is_dir():
                    shutil.copytree(directory, out / directory.name, dirs_exist_ok=True)
        # No public scans, ground-truth text, predicted text, or raw service logs
        # are copied. Numeric observations and content hashes are sufficient.
        structured["runs"].append({"name": name, "title": title, "budget": budget,
                                    "concurrency": concurrency, "valid_affinity": valid, "summary": summary})
        if name == "native-1":
            report += ["## Controls and targeted ablations", ""]
        if name == "public-1":
            report += ["## Public regression diagnostics", "",
                       "Reported separately. Three pinned Tesseract regression scans include legacy UNLV",
                       "transcription markers and historical reading-order conventions. These references are not",
                       "adjudicated modern production ground truth. The source corpus was not used to tune models.",
                       "Only numeric results, hashes, manifest, and retrieval code are redistributed; no scan,",
                       "source article transcription, or predicted article text is included.", ""]
        report += [f"### {name}: {title}", "",
                   f"Total application budget: **{budget} logical CPU(s)**; request concurrency: **{concurrency}**.",
                   f"Affinity validation: **{'passed' if valid else 'FAILED — exclude from performance claims'}**.", "",
                   "| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |",
                   "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for engine, result in summary["engines"].items():
            accuracy = result["accuracy"]
            report.append(f"| {engine} | {accuracy['unique_cases']} | {result['successes']} | {result['failures']} | "
                          f"{number(accuracy['micro_cer'] * 100 if accuracy['micro_cer'] is not None else None, 4)} | "
                          f"{number(accuracy['micro_wer'] * 100 if accuracy['micro_wer'] is not None else None, 4)} | "
                          f"{number(result['pages_per_second'])} | {number(result['latency_success_only_ms']['p50'], 1)} |")
        report += ["", f"[Summary](../benchmarks/results/{name}/summary.json) · "
                   f"[Every request](../benchmarks/results/{name}/requests.jsonl) · "
                   f"[Resources and actual batch shapes](../benchmarks/results/{name}/cohorts.json)", "",
                   "| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |",
                   "| --- | --- | --- | --- | --- |"]
        for engine in summary["engines"]:
            selected = [c for c in cohorts if c["engine"] == engine]
            rates = []
            for cohort in selected:
                pages = sum(row["pages"] for row in request_rows if row["engine"] == engine
                            and row["repetition"] == cohort["repetition"] and row["phase"] == "measured"
                            and row["status"] == "ok")
                rates.append(pages / cohort["measured_wall_seconds"] if cohort.get("measured_wall_seconds") else None)
            rss = [c["resources"]["peak_sum_rss_bytes"] / 1048576 for c in selected]
            cpu = [c["resources"]["observed_cpu_seconds_lower_bound"] for c in selected]
            startup = [c.get("startup_ms") for c in selected]
            report.append(f"| {engine} | {', '.join(number(n) for n in rates)} | "
                          f"{', '.join(number(n, 1) for n in rss)} | {', '.join(number(n, 2) for n in cpu)} | "
                          f"{', '.join(number(n, 1) for n in startup)} |")
        report += ["", "RSS is sampled across the whole process tree and double-counts shared mappings.",
                   "CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.", ""]
        observed_batches = []
        for engine in summary["engines"]:
            hist, sizes, padding = batch_totals([c for c in cohorts if c["engine"] == engine])
            if sizes:
                observed_batches.append(f"{engine}: B{sizes[0]}–B{sizes[1]}, {padding * 100:.2f}% padding")
        if observed_batches:
            report += ["Observed request tensors, including client warmups: " + "; ".join(observed_batches) + ".", ""]
        if name.startswith("native-"):
            calls = sum(c["backend_metrics"]["request_count_including_warmup"] for c in cohorts)
            report += [f"Observed backend OCR requests including warmups: **{calls}**. This is a bypass correctness control,",
                       "not a useful throughput benchmark with only one distinct document.", ""]
        if name == "batch-1":
            hist = {}
            for cohort in cohorts:
                for key, value in cohort["backend_metrics"]["actual_tensor_batch_histogram"].items():
                    group = cohort["engine"]
                    hist.setdefault(group, {})[key] = hist.setdefault(group, {}).get(key, 0) + value
            report += ["Actual tensor batch-size histograms, including warmups:", "", "```json",
                       json.dumps(hist, indent=2), "```", "",
                       "The configured ceiling is 64 lines. It does not mean any request formed a 64-line tensor.", ""]
        if name == "math-1":
            report += ["This toggle was tested on ordinary Latin text. It measures overhead/output stability there;",
                       "it does not establish equation recognition accuracy or the best math profile.", ""]
        if name == "threads-2-completed":
            report += ["Two Indra cohorts were blocked before launch by an unrelated server on the selected port.",
                       "The harness refused to use that server. Only those blocked cohorts were completed on a new loopback port;",
                       "the original 12 blocked attempts and all replacement observations remain under `attempts/`, with",
                       "[explicit recovery provenance](../benchmarks/results/threads-2-completed/recovery.json).", "",
                       "At concurrency 1, one intra-op thread was faster here than two. The practical two-CPU throughput",
                       "default remains two workers with one intra-op thread each, subject to the higher resident-memory cost.",
                       "That judgment combines the C1 thread ablation with the separate C2 primary run; it is not a matched",
                       "proof that one worker/two threads loses at every arrival rate or on other CPUs.", ""]
    report += ["## Remaining limits", "",
               "- The initial `raw-2` run recorded a launcher affinity observation on the measurement CPU and was rejected.",
               "  It was rerun after waiting for `taskset` to apply the workload mask before process sampling. The valid run is `raw-2-verified`.",
               "- The initial `threads-2` experiment had only two-thread candidates. Use `threads-2-completed` for the matched one/two-thread controls and its disclosed recovery.",
               "- No unmeasured 200-DPI-versus-300-DPI rendering conclusion is claimed. Source images contain both DPI variants; Tika's primary OCR rendering was 300 DPI.",
               "- Native process affinity was tested. OCI root-filesystem correctness checks are separate evidence; the read-only cgroup hierarchy prevented actual application containers. No Docker/Kubernetes resource-enforcement or signed-image runtime performance result is claimed.",
               "- Local Tika plans used 256 MiB parent / 512 MiB forked JVM heap ceilings and `ActiveProcessorCount` equal to the CPU budget, identically for both engines. They differ from deployment defaults (128 MiB parent and processor-count 1); these are not exact deployment timings.",
               "- Raw Tesseract's startup column is zero because there is no separately started resident service; each request latency includes process/model initialization. Restarted services do not imply cold filesystem caches.",
               "- Each cohort received one client warmup request. A format first encountered during the measured corpus may still trigger lazy library initialization; these short runs do not prove a fully saturated steady state.",
               "- All p95/p99 numbers have small-sample warnings. No open-loop overload or long soak result is claimed.",
               "- Use a held-out, manually adjudicated corpus and reserved hardware for a production acceptance gate.", "",
               "See [methodology and primary references](benchmark-methodology.md) and [reproduction instructions](../benchmarks/README.md).", ""]
    for name in structured["excluded_runs"]:
        source = args.results_root / f"ocr-results-{name}"
        if source.is_dir():
            copy_numeric(source, destination / "excluded" / name)
    fixtures = args.results_root / "ocr-benchmark-fixtures"
    if fixtures.is_dir():
        shutil.copytree(fixtures, destination / "synthetic-fixtures", dirs_exist_ok=True)
    assets = args.results_root / "ocr-benchmark-assets"
    frozen = destination / "frozen-backend"
    frozen.mkdir(exist_ok=True)
    for name in ("server.py", "model_files.py", "models.lock.json"):
        if (assets / "backend" / name).is_file():
            shutil.copy2(assets / "backend" / name, frozen / name)
    hashes = assets / "source-hashes.json"
    if hashes.is_file():
        recorded = json.loads(hashes.read_text())
        recorded = {key: value for key, value in recorded.items() if "__pycache__" not in key}
        (destination / "frozen-source-hashes.json").write_text(json.dumps(recorded, indent=2) + "\n")
    topology = args.results_root / "ocr-host-topology.json"
    if topology.is_file():
        shutil.copy2(topology, destination / topology.name)
    report += ["## Evidence package", "",
               "The repository includes the exact synthetic fixtures and their hash manifest, frozen measured backend",
               "source/model manifest, the initial harness snapshot, current harness, raw synthetic outputs, numeric public",
               "observations, excluded-run numeric evidence, and the thread-ablation recovery provenance.",
               "[Evidence index](../benchmarks/results/index.json) · [Synthetic manifest](../benchmarks/results/synthetic-fixtures/manifest.json) ·",
               "[Frozen source hashes](../benchmarks/results/frozen-source-hashes.json) · [CPU topology](../benchmarks/results/ocr-host-topology.json)", ""]
    if structured["pending"]:
        report += ["## Still running", "", ", ".join(structured["pending"]), ""]
    (destination / "index.json").write_text(json.dumps(structured, ensure_ascii=False, indent=2) + "\n")
    target = args.repository / "research/benchmark-results.md"
    target.write_text("\n".join(report), encoding="utf-8")
    evidence_hashes = {str(path.relative_to(destination)): hashlib.sha256(path.read_bytes()).hexdigest()
                       for path in destination.rglob("*") if path.is_file() and path.name != "evidence-sha256.json"}
    (destination / "evidence-sha256.json").write_text(json.dumps(evidence_hashes, indent=2) + "\n")
    print(json.dumps({"report": str(target), "completed": len(structured["runs"]), "pending": structured["pending"]}))


if __name__ == "__main__":
    main()
