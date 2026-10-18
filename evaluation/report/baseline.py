"""Render only the verified historical signed-container study with Seaborn.

Reporting code is deliberately outside the Rust serving/evaluation path.
The eight source rows must reconcile with their retained summary JSON files.
"""

from __future__ import annotations

import csv
import hashlib
import json
from pathlib import Path

from study import require

import numpy as np
import pandas as pd
import seaborn as sns

ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "evaluation/results/signed-baseline-20261018"
FIGURES = ROOT / "evaluation/figures"
ENGINES = ["Tesseract best", "Python Indra"]
POLICIES = ["Shared CPU quota", "Deployment partitions"]
PALETTE = {"Tesseract best": "#345995", "Python Indra": "#B65D20"}


def load() -> pd.DataFrame:
    receipt = json.loads((DATA / "provenance.json").read_text())
    rows = list(csv.DictReader((DATA / "summary.csv").open(newline="")))
    require(len(rows) == 8, "historical baseline or plotted mark did not reconcile")
    csv_bytes = (DATA / "summary.csv").read_bytes()
    transport = json.loads(Path(__file__).with_name("baseline-csv-transport.json").read_text())
    require(receipt["csv_sha256"] == transport["frozen_provenance_sha256"], "historical CSV provenance changed")
    require(b"\r" not in csv_bytes and hashlib.sha256(csv_bytes).hexdigest() == transport["committed_lf_sha256"], "committed CSV transport differs")
    require(hashlib.sha256(csv_bytes.replace(b"\n", b"\r\n")).hexdigest() == receipt["csv_sha256"], "CSV CRLF recovery differs from frozen bytes")
    identities = set()
    for row in rows:
        key = (row["policy"], int(row["cpus"]), row["engine"])
        require(key not in identities, "historical baseline or plotted mark did not reconcile")
        identities.add(key)
        relative = row["source"]
        path = DATA / relative
        require(path.resolve().is_relative_to(DATA.resolve()), "historical baseline or plotted mark did not reconcile")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == receipt["sources"][relative]["sha256"], "historical baseline or plotted mark did not reconcile")
        source = json.loads(path.read_text())["engines"][row["source_engine"]]
        require(source["attempts"] == source["successes"] == 45, "historical baseline or plotted mark did not reconcile")
        require(source["failures"] == 0, "historical baseline or plotted mark did not reconcile")
        require(source["accuracy"]["unique_cases"] == 15, "historical baseline or plotted mark did not reconcile")
        for field, actual in [
            ("throughput", source["pages_per_second"]),
            ("p50_ms", source["latency_success_only_ms"]["p50"]),
            ("p95_ms", source["latency_success_only_ms"]["p95"]),
            ("strict_cer", source["strict_accuracy"]["micro_cer"]),
            ("collapsed_cer", source["accuracy"]["micro_cer"]),
        ]:
            require(float(row[field]) == actual, "historical baseline or plotted mark did not reconcile")
            require(np.isfinite(actual) and actual >= 0, "historical baseline or plotted mark did not reconcile")
        require(float(row["throughput"]) == source["successful_pages"] / source["measured_wall_seconds"], "historical baseline or plotted mark did not reconcile")
    require(identities == {(p, c, e) for p in POLICIES for c in [1, 2] for e in ENGINES}, "historical baseline or plotted mark did not reconcile")
    frame = pd.DataFrame(rows)
    frame["cpus"] = frame["cpus"].astype(int)
    for field in ["throughput", "p50_ms", "p95_ms", "strict_cer", "collapsed_cer"]:
        frame[field] = frame[field].astype(float)
    return frame


def plot(frame: pd.DataFrame, field: str, ylabel: str, title: str, stem: str) -> dict:
    sns.set_theme(style="whitegrid", context="notebook", font="DejaVu Sans", rc={"svg.hashsalt": "tika-indra-historical-v1"})
    graph = sns.catplot(
        data=frame, kind="bar", x="cpus", y=field, hue="engine", col="policy",
        order=[1, 2], hue_order=ENGINES, col_order=POLICIES,
        palette=PALETTE, saturation=1, errorbar=None, height=4.6, aspect=1.15,
        legend_out=True, sharey=True,
    )
    graph.set_axis_labels("Total application CPU ceiling", ylabel)
    graph.set_titles("{col_name}")
    graph.legend.set_title("")
    graph.figure.suptitle(title, x=0.42, y=1.07, fontsize=15, fontweight="bold")
    upper = float(frame[field].max()) * 1.24
    checked = []
    for policy, axis in zip(POLICIES, graph.axes.flat, strict=True):
        axis.set_ylim(0, upper)
        require(axis.get_ylim()[0] == 0, "historical baseline or plotted mark did not reconcile")
        containers = [c for c in axis.containers if len(c) == 2]
        require(len(containers) == 2, "historical baseline or plotted mark did not reconcile")
        for engine, container in zip(ENGINES, containers, strict=True):
            for position, bar in enumerate(container):
                cpus = position + 1
                selected = frame[(frame.policy == policy) & (frame.engine == engine) & (frame.cpus == cpus)]
                require(len(selected) == 1, "historical baseline or plotted mark did not reconcile")
                expected = float(selected[field].iloc[0])
                require(np.isclose(bar.get_height(), expected, rtol=1e-12, atol=1e-12), "historical baseline or plotted mark did not reconcile")
                expected_center = position + (-0.2 if engine == ENGINES[0] else 0.2)
                require(np.isclose(bar.get_x() + bar.get_width() / 2, expected_center), "historical baseline or plotted mark did not reconcile")
                hex_color = PALETTE[engine].lstrip("#")
                expected_color = tuple(int(hex_color[i:i+2], 16) / 255 for i in [0, 2, 4])
                require(np.allclose(bar.get_facecolor()[:3], expected_color), "historical baseline or plotted mark did not reconcile")
                checked.append({"policy": policy, "cpus": cpus, "engine": engine, "field": field, "value": expected})
            axis.bar_label(container, fmt="%.2f" if field == "throughput" else "%.0f", padding=3, fontsize=10)
    graph.figure.text(
        0.02, -0.04,
        "Historical study • 15 mostly English synthetic fixtures • 3 repeats • 45 successes per bar\n"
        "Concurrency: 1 at 1 CPU; 2 at 2 CPUs. 3 GiB application cap; old load generator outside the cap.\n"
        "No native Rust OCR arm. No multilingual/general accuracy claim. No confidence intervals estimated here.",
        fontsize=9, color="#404040", va="top",
    )
    FIGURES.mkdir(parents=True, exist_ok=True)
    for suffix in ["png", "svg"]:
        graph.figure.savefig(FIGURES / f"{stem}.{suffix}", dpi=180, bbox_inches="tight", facecolor="white", metadata={"Date": None} if suffix == "svg" else None)
    graph.figure.clear()
    return {"file_stem": stem, "checked_bars": checked, "zero_baseline": True}


def render() -> None:
    frame = load()
    checks = [
        plot(frame, "throughput", "Successful fixture pages / workload second",
             "Historical Tika 3.3.2 throughput", "signed-baseline-throughput"),
        plot(frame, "p95_ms", "Successful complete-response p95 (ms)",
             "Historical Tika 3.3.2 tail latency", "signed-baseline-p95"),
    ]
    result = {"schema_version": 1, "scope": "historical signed baseline only", "plots": checks,
              "visual_review": "pending", "no_new_native_measurement": True}
    (FIGURES / "signed-baseline-plot-validation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"rows_verified": len(frame), "bars_verified": sum(len(x["checked_bars"]) for x in checks)}))


if __name__ == "__main__":
    render()
