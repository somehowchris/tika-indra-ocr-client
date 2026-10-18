"""Bind the actual rendered files and environment; visual approval stays explicit."""
from __future__ import annotations

import hashlib
from html.parser import HTMLParser
from importlib import metadata
import json
import os
from pathlib import Path
import platform

import study

DIRECTORY = Path(__file__).resolve().parent


class Tables(HTMLParser):
    def __init__(self):
        super().__init__()
        self.rows = []
        self.row = None
        self.cell = None

    def handle_starttag(self, tag, attrs):
        if tag == "tr":
            self.row = []
        elif tag in {"td", "th"} and self.row is not None:
            self.cell = ""

    def handle_data(self, data):
        if self.cell is not None:
            self.cell += data

    def handle_endtag(self, tag):
        if tag in {"td", "th"} and self.cell is not None:
            self.row.append(self.cell.strip())
            self.cell = None
        elif tag == "tr" and self.row is not None:
            self.rows.append(self.row)
            self.row = None


def main():
    root = os.environ.get("OCR_EVALUATION_ROOT")
    source = os.environ.get("OCR_EVALUATION_SOURCE_SHA")
    study.require(bool(root) == bool(source), "set both artifact root and selected source")
    result = study.load(Path(root), source) if root else {"statuses": study.missing_cells(), "measurements": [], "comparisons": []}
    html = DIRECTORY / "report.html"
    parser = Tables()
    parser.feed(html.read_text())
    for expected in result["statuses"]:
        count = sum(row[-5:-1] == [str(expected[k]) for k in ("cohort", "variant", "cpus", "status")] for row in parser.rows)
        study.require(count == 1, f"rendered status row missing/duplicated: {expected}")
    paths = [DIRECTORY / name for name in ("README.md", "report.qmd", "baseline.py", "baseline-csv-transport.json", "study.py", "test_study.py", "requirements.in", "requirements.lock", "tools.json", "render.sh", "receipt.py", "report.html")]
    paths += sorted((study.ROOT / "evaluation/figures").glob("signed-baseline-*"))
    receipt = {
        "schema_version": 1,
        "scope": "historical baseline plus explicit frozen-study statuses; unavailable measurements never substituted",
        "python": platform.python_version(),
        "platform": platform.platform(),
        "packages": sorted(f"{d.metadata['Name']}=={d.version}" for d in metadata.distributions()),
        "source_sha": result.get("source_sha"), "index_sha256": result.get("index_sha256"),
        "planned_status_rows_verified_in_html": len(result["statuses"]),
        "admitted_cells": len(result["measurements"]), "admitted_comparisons": len(result["comparisons"]),
        "files": {p.relative_to(study.ROOT).as_posix(): {"sha256": hashlib.sha256(p.read_bytes()).hexdigest(), "bytes": p.stat().st_size} for p in paths},
        "visual_review": {"status": "pending", "required": "Full-resolution PNG/SVG and the complete rendered HTML must be visually inspected; bind a separate review receipt to these exact hashes."},
        "remaining_scientific_gates": ["actual signed-image matrix and independent numerical admission", "reference-letter denominator reproduction from verified acquired references", "measured-study plots and paired interpretation", "independent full visual review"],
    }
    (DIRECTORY / "render-evidence").mkdir(exist_ok=True)
    (DIRECTORY / "render-evidence/receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"status_rows": len(result["statuses"]), "admitted_cells": len(result["measurements"]), "visual_review": "pending"}))


if __name__ == "__main__":
    main()
