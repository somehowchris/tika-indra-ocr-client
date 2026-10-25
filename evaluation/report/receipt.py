"""Bind the actual rendered files and environment; visual approval stays explicit."""
from __future__ import annotations

import hashlib
from html import unescape
from html.parser import HTMLParser
from importlib import metadata
import json
import os
from pathlib import Path
import platform
import re

import study

DIRECTORY = Path(__file__).resolve().parent
# A figure or artist left open at the end of a cell prints as text, such as
# "<Figure size 1209.6x806.4 with 0 Axes>", in the rendered report.
OBJECT_REPR = re.compile(r"<(?:Figure size [^<>]* with \d+ Axes|(?:matplotlib|seaborn)\.[\w.]+ (?:object )?at 0x[0-9a-f]+)>")


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


class Resources(HTMLParser):
    """Inventory automatic HTML/CSS resource references, excluding citation links."""
    def __init__(self):
        super().__init__()
        self.references = []
        self.in_style = False

    def css(self, value):
        self.references.extend(re.findall(r"url\(\s*[\"']?([^\"')\s]+)", value, re.I))
        self.references.extend(re.findall(r"@import\s+[\"']([^\"']+)", value, re.I))

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        self.in_style = tag == "style" or self.in_style
        self.css(attrs.get("style", ""))
        keys = ("src", "poster", "data") if tag in {"script", "img", "iframe", "audio", "video", "source", "embed", "object", "input"} else ()
        if tag == "link" and set(attrs.get("rel", "").lower().split()) & {"stylesheet", "icon", "preload", "modulepreload", "prefetch"}:
            keys = ("href",)
        self.references.extend(attrs[k] for k in keys if k in attrs)
        study.require("srcset" not in attrs, "unembedded responsive resource references")
        study.require(not (tag == "meta" and attrs.get("http-equiv", "").lower() == "refresh"), "automatic refresh in report")

    def handle_endtag(self, tag):
        if tag == "style":
            self.in_style = False

    def handle_data(self, data):
        if self.in_style:
            self.css(data)


def verify_embedded_resources(html):
    resources = Resources()
    resources.feed(html)
    study.require(all(value and value.startswith(("data:", "#")) for value in resources.references), "report contains unembedded automatic HTML/CSS resource")
    return len(resources.references)


def verify_no_object_reprs(html):
    leaked = OBJECT_REPR.findall(unescape(html))
    study.require(not leaked, f"report prints plotting object reprs: {leaked[:3]}")


def verify_measurement_date(html, result):
    """An admitted study's report prints the index's aggregation time, so its date is the evidence's."""
    study.require(not result["measurements"] or result["aggregated_utc"] in html, "report does not print the admitted aggregation time")


def remaining_gates(result):
    """Scientific gates still open for this render; a complete admission closes the matrix and its interpretation."""
    gates = [] if len(result["measurements"]) == 24 else ["actual signed-image matrix with all 24 cells admitted"]
    gates += ["independent numerical replay of the sealed evidence", "reference-letter denominator reproduction from verified acquired references"]
    gates += [] if result["measurements"] else ["actual-result interpretation"]
    return gates + ["independent full visual review of the rendered HTML"]


def main():
    root = os.environ.get("OCR_EVALUATION_ROOT")
    source = os.environ.get("OCR_EVALUATION_SOURCE_SHA")
    study.require(bool(root) == bool(source), "set both artifact root and selected source")
    result = study.load(Path(root), source) if root else study.unsubmitted()
    html = DIRECTORY / "report.html"
    embedded_resources = verify_embedded_resources(html.read_text())
    verify_no_object_reprs(html.read_text())
    verify_measurement_date(html.read_text(), result)
    parser = Tables()
    parser.feed(html.read_text())
    for expected in result["statuses"]:
        count = sum(row[-5:-1] == [str(expected[k]) for k in ("cohort", "variant", "cpus", "status")] for row in parser.rows)
        study.require(count == 1, f"rendered status row missing/duplicated: {expected}")
    paths = [DIRECTORY / name for name in ("README.md", "report.qmd", "baseline.py", "baseline-csv-transport.json", "study.py", "test_study.py", "figures.py", "test_figures.py", "requirements.in", "requirements.lock", "tools.json", "render.sh", "receipt.py", "report.html")]
    paths += sorted((study.ROOT / "evaluation/figures").glob("signed-baseline-*"))
    generated = DIRECTORY / "study-data"
    validation = study.decode((generated / "plot-validation.json").read_bytes())
    study.require(validation["synthetic"] is False and validation["measurement_source_sha"] == result.get("source_sha") and
                  validation["study_index_sha256"] == result.get("index_sha256"), "figure source or synthetic scope mismatch")
    study.require({p.name for p in generated.iterdir()} == set(validation["files"]) | {"plot-validation.json"}, "unindexed study figure/data output")
    for name, identity in validation["files"].items():
        path = generated / name
        study.require(path.is_file() and not path.is_symlink() and path.stat().st_size == identity["bytes"] and
                      hashlib.sha256(path.read_bytes()).hexdigest() == identity["sha256"], "changed generated study data/figure")
    paths += sorted(generated.iterdir())
    paths = sorted(set(paths) | set(DIRECTORY.glob("*.py")))
    receipt = {
        "schema_version": 1,
        "scope": "historical baseline plus explicit frozen-study statuses; unavailable measurements never substituted",
        "python": platform.python_version(),
        "platform": platform.platform(),
        "packages": sorted(f"{d.metadata['Name']}=={d.version}" for d in metadata.distributions()),
        "source_sha": result.get("source_sha"), "index_sha256": result.get("index_sha256"),
        "planned_status_rows_verified_in_html": len(result["statuses"]),
        "comparison_status_records_exported": len(result["comparison_statuses"]),
        "automatic_resources_verified_embedded": embedded_resources,
        "admitted_cells": len(result["measurements"]), "admitted_comparisons": len(result["comparisons"]),
        "study_figures": {"plots": len(validation["plots"]), "checked_marks": sum(len(panel["marks"]) + len(panel.get("collapsed_marks", [])) for plot in validation["plots"] for panel in plot["verified_artists"]), "visual_review": "pending"},
        "files": {p.relative_to(study.ROOT).as_posix(): {"sha256": hashlib.sha256(p.read_bytes()).hexdigest(), "bytes": p.stat().st_size} for p in paths},
        "visual_review": {"status": "pending", "required": "Full-resolution PNG/SVG and the complete rendered HTML must be visually inspected; bind a separate review receipt to these exact hashes."},
        "remaining_scientific_gates": remaining_gates(result),
    }
    (DIRECTORY / "render-evidence").mkdir(exist_ok=True)
    (DIRECTORY / "render-evidence/receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"status_rows": len(result["statuses"]), "admitted_cells": len(result["measurements"]), "visual_review": "pending"}))


if __name__ == "__main__":
    main()
