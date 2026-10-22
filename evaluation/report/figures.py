"""Seaborn views of already admitted study evidence; no OCR or resampling.

Every exported mark/interval is checked against its source row. Synthetic smoke
inputs are explicitly labelled and never loaded by the report notebook.
"""
from __future__ import annotations

import csv
import hashlib
import json
import math
from pathlib import Path

from study import COMPARISON_FIELDS, METADATA, RESOURCE_FIELDS, VARIANTS, require

LABELS = ("Fast PSM3", "Best PSM3", "Best PSM6", "Best PSM11",
          "Python math b64", "Rust math b64")
METRICS = {"cer": "Strict CER (%)", "cer_whitespace_collapsed": "Collapsed CER (%)"}
PAIRED_METRICS = ("cer", "cer_whitespace_collapsed", "wer")
COLOR = "#345995"
# One fixed colour per arm for the descriptive strata view; never a fitted scale.
ARM_COLORS = ("#345995", "#d9822b", "#2a9d8f", "#7b4fa3", "#b63a3a", "#5d6d7e")
STRATA_AXES = ("appearance", "script", "domain")
# Fixed vertical offset per arm inside a stratum row, so six marks never hide
# one another; the offset is part of the verified position, not a value.
STRATA_DODGE = .085
STEMS = ("main-primary-quality", "main-primary-paired", "main-workload-performance",
         "main-challenge-quality", "laboratory-quality", "main-primary-strata")
TABLES = {
    "comparison-statuses": COMPARISON_FIELDS,
    "quality": ("study", "cohort", "cpus", "variant", "metric", "value", "scored", "selected", "failures", "metric_limits", "clusters", "reference_units", "errors"),
    "resources": ("study", "cpus", "variant", *RESOURCE_FIELDS),
    "performance": ("study", "cpus", "variant", "throughput", "p50_ms", "p95_ms", "p99_ms", "latency_quantile_method", "successes", "failures", "expected", "wall_us"),
    "paired": ("study", "cohort", "cpus", "left", "variant", "metric", "value", "low", "high", "joint", "selected", "clusters", "paired_cases", "reference_units", "left_failures", "right_failures", "left_unscoreable", "right_unscoreable", "draws", "undefined_draws"),
    "attempts": ("study", "cpus", "variant", "case_id", "cohort", "repetition", "elapsed_us", "status", "error"),
    "cases": ("study", "cpus", "variant", "case_id", *METADATA, "word_tokenizer", "outcome", "metric", "metric_eligible", "metric_available", "reference_units", "hypothesis_units", "errors", "rate", "first_http_status", "first_elapsed_us", "first_output_bytes", "first_output_sha256", "input_sha256", "reference_sha256"),
    "summaries": ("study", "cpus", "variant", "cohort", "axis", "level", "metric", "selected_cases", "scored_cases", "service_failures", "metric_limits", "missing_first_attempts", "document_clusters", "scored_document_clusters", "metric_cases", "reference_units", "hypothesis_units", "errors", "empty_reference_cases", "micro_rate", "macro_case_rate"),
}


def finite(value):
    require(value is None or (type(value) in (int, float) and math.isfinite(value)), "nonfinite plot value")
    return value


def percent(value):
    return None if value is None else finite(value) * 100


def quantile(values, probability):
    """Descriptive type7 quantile of admitted observations; never an interval."""
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lower = math.floor(position)
    return ordered[lower] + (ordered[math.ceil(position)] - ordered[lower]) * (position - lower)


def tables(admitted):
    result = {name: [] for name in TABLES}
    result["comparison-statuses"] = admitted["comparison_statuses"]
    for measured in admitted["measurements"]:
        identity = {"study": measured["cohort"], "cpus": measured["cpus"], "variant": measured["variant"]}
        require(identity["variant"] in VARIANTS and identity["cpus"] in (1, 2), "unknown plot identity")
        for case in measured["score"]["cases"]:
            for metric in ("cer", "cer_whitespace_collapsed", "wer"):
                distance = (case["metrics"] or {}).get(metric)
                result["cases"].append({**identity, "case_id": case["case_id"], **{k: case[k] for k in METADATA},
                    **{k: case[k] for k in ("word_tokenizer", "outcome", "first_http_status", "first_elapsed_us", "first_output_bytes", "first_output_sha256", "input_sha256", "reference_sha256")},
                    "metric": metric, "metric_eligible": metric != "wer" or case["word_tokenizer"] == "whitespace",
                    "metric_available": distance is not None,
                    **{k: None if distance is None else distance[k] for k in ("reference_units", "hypothesis_units", "errors")},
                    "rate": None if distance is None or distance["reference_units"] == 0 else distance["errors"] / distance["reference_units"]})
        for summary in measured["score"]["summaries"]:
            for metric in ("cer", "cer_whitespace_collapsed", "wer"):
                aggregate = summary[metric]
                result["summaries"].append({**identity,
                    **{k: summary[k] for k in ("cohort", "axis", "level", "selected_cases", "scored_cases", "service_failures", "metric_limits", "missing_first_attempts", "document_clusters", "scored_document_clusters")},
                    "metric": metric, "metric_cases": aggregate["cases"],
                    **{k: aggregate[k] for k in ("reference_units", "hypothesis_units", "errors", "empty_reference_cases", "micro_rate", "macro_case_rate")}})
            if summary["axis"] != "overall":
                continue
            for metric in METRICS:
                aggregate = summary[metric]
                result["quality"].append({**identity, "cohort": summary["cohort"], "metric": METRICS[metric],
                    "value": percent(aggregate["micro_rate"]), "scored": summary["scored_cases"],
                    "selected": summary["selected_cases"], "failures": summary["service_failures"],
                    "metric_limits": summary["metric_limits"], "clusters": summary["scored_document_clusters"],
                    "reference_units": aggregate["reference_units"], "errors": aggregate["errors"]})
        result["resources"].append({**identity, **measured["resources"]})
        attempts = measured["timings"]
        successes = [a["elapsed_us"] for a in attempts if a["status"] is not None and 200 <= a["status"] < 300 and a["error"] is None]
        performance = measured["performance"]
        require(len(attempts) == performance["expected_attempts"] and len(successes) == performance["successful_attempts"], "plot timing coverage differs")
        require(all(type(a["elapsed_us"]) is int and a["elapsed_us"] >= 0 for a in attempts), "invalid plot observation")
        result["performance"].append({**identity,
            "throughput": finite(performance["successful_case_responses_per_complete_workload_wall_second"]),
            **{f"p{p}_ms": None if not successes else quantile(successes, p / 100) / 1000 for p in (50, 95, 99)},
            "latency_quantile_method": "linear_type7_successful_nonwarmup_attempts",
            "successes": len(successes), "failures": performance["failed_attempts"],
            "expected": performance["expected_attempts"], "wall_us": performance["completed_repetition_wall_us"]})
        for attempt in attempts:
            result["attempts"].append({**identity, **{key: attempt[key] for key in ("case_id", "cohort", "repetition", "elapsed_us", "status", "error")}})
    # Every planned pair and all three frozen metrics reach paired.csv; the
    # plot below shows only the prespecified fast-PSM3 strict-CER view.
    for comparison in admitted["comparisons"]:
        report = comparison["report"]
        require(report["left_variant"] in VARIANTS and report["right_variant"] in VARIANTS and
                VARIANTS.index(report["left_variant"]) < VARIANTS.index(report["right_variant"]), "unknown or unordered pair")
        for summary in report["strata"]:
            if summary["axis"] != "overall":
                continue
            for metric in PAIRED_METRICS:
                estimate = summary[metric]
                low, high = percent(estimate["ci_95_low"]), percent(estimate["ci_95_high"])
                require((low is None) == (high is None) and (low is None or low <= high), "invalid frozen interval")
                require(comparison["cohort"] != "laboratory" or low is None, "laboratory interval forbidden")
                require(estimate["right_minus_left"] is not None or low is None, "interval without a point estimate")
                result["paired"].append({"study": comparison["cohort"], "cohort": summary["cohort"],
                    "cpus": comparison["cpus"], "left": report["left_variant"], "variant": report["right_variant"], "metric": metric,
                    "value": percent(estimate["right_minus_left"]), "low": low, "high": high,
                    "joint": summary["joint_success_cases"], "selected": summary["selected_cases"],
                    "clusters": estimate["document_clusters"], "paired_cases": estimate["paired_cases"], "reference_units": estimate["reference_units"], "left_failures": summary["left_service_failures"],
                    "right_failures": summary["right_service_failures"], "left_unscoreable": summary["left_unscoreable"],
                    "right_unscoreable": summary["right_unscoreable"], "draws": estimate["bootstrap_samples"],
                    "undefined_draws": estimate["undefined_bootstrap_samples"]})
    return result


def baseline_pairs(paired):
    """The prespecified plotted subset: main primary, fast PSM3 reference, strict CER."""
    return [r for r in paired if r["study"] == "main" and r["cohort"] == "primary" and r["left"] == VARIANTS[0] and r["metric"] == "cer"]


def strata_rows(summaries):
    """Descriptive main-primary strict CER by appearance, script and domain; no inference."""
    rows = []
    for summary in summaries:
        if summary["study"] != "main" or summary["cohort"] != "primary" or summary["axis"] not in STRATA_AXES or summary["metric"] != "cer":
            continue
        rows.append({"study": summary["study"], "cpus": summary["cpus"], "variant": summary["variant"], "axis": summary["axis"], "level": summary["level"],
                     "value": percent(summary["micro_rate"]), "scored": summary["scored_cases"], "selected": summary["selected_cases"],
                     "failures": summary["service_failures"], "metric_limits": summary["metric_limits"], "clusters": summary["scored_document_clusters"],
                     "reference_units": summary["reference_units"], "errors": summary["errors"]})
    require(len({(r["cpus"], r["axis"], r["level"], r["variant"]) for r in rows}) == len(rows), "duplicate stratum row")
    return rows


def row_labels(rows, cpus, field):
    labels = []
    for variant, label in zip(VARIANTS, LABELS, strict=True):
        found = [r for r in rows if r["cpus"] == cpus and r["variant"] == variant]
        require(len(found) <= 1, "duplicate plotted row")
        if not found:
            labels.append(label + (" · reference" if field == "paired" and variant == VARIANTS[0] else " · not admitted"))
        else:
            row = found[0]
            if field == "quality":
                labels.append(f"{label} · {row['scored']}/{row['selected']}; F{row['failures']}; M{row['metric_limits']}")
            elif field == "paired":
                labels.append(f"{label} · J{row['joint']}/{row['selected']}; N{row['paired_cases']}; k{row['clusters']}\nR{row['reference_units']}; F{row['left_failures']}/{row['right_failures']}; U{row['left_unscoreable']}/{row['right_unscoreable']}")
            else:
                labels.append(f"{label} · n{row['successes']}/{row['expected']}; F{row['failures']}")
    return labels


def plot(rows, columns, title, note, stem, output, field, synthetic):
    import numpy as np
    import pandas as pd
    import seaborn as sns

    sns.set_theme(style="whitegrid", context="notebook", font="DejaVu Sans",
                  rc={"svg.hashsalt": "tika-indra-measured-v1"})
    points = []
    for row in rows:
        for key, label in columns:
            if row.get(key) is not None:
                points.append({**row, "panel": label, "x": finite(row[key]), "rank": VARIANTS.index(row["variant"])})
    # Keep absent CPU facets visible without inventing an observed mark.
    empty = [{"cpus": cpus, "panel": label, "x": float("nan"), "rank": float("nan")} for cpus in (1, 2) for _, label in columns]
    frame = pd.DataFrame(points + empty)
    graph = sns.relplot(data=frame, kind="scatter", x="x", y="rank", row="cpus", col="panel",
                        row_order=[1, 2], col_order=[label for _, label in columns],
                        color=COLOR, s=62, edgecolor="white", linewidth=.6,
                        height=5.2 if field == "paired" else 4.2, aspect=1.5, facet_kws={"sharex": False, "sharey": False})
    verified = []
    for row_number, cpus in enumerate((1, 2)):
        for column_number, (key, label) in enumerate(columns):
            axis = graph.axes[row_number, column_number]
            selected = [r for r in rows if r["cpus"] == cpus]
            labels = row_labels(selected, cpus, field)
            axis.set_yticks(range(len(VARIANTS)), labels=labels)
            axis.set_ylim(len(VARIANTS) - .35, -.65)
            axis.set_ylabel("")
            axis.set_xlabel(label)
            axis.set_title(f"{cpus} CPU · {label}")
            collections = [c for c in axis.collections if len(c.get_offsets())]
            actual = [tuple(map(float, point)) for collection in collections for point in collection.get_offsets()]
            expected = [(float(r[key]), float(VARIANTS.index(r["variant"]))) for r in selected if r.get(key) is not None]
            require(len(actual) == len(expected), "missing/extra Seaborn mark")
            for observed, wanted in zip(sorted(actual), sorted(expected), strict=True):
                require(np.allclose(observed, wanted, rtol=1e-12, atol=1e-12), "Seaborn mark value/position differs")
            for collection in collections:
                require(np.all(collection.get_sizes() > 0), "invisible study mark")
                require(np.allclose(collection.get_facecolors()[0][:3], (52/255, 89/255, 149/255)), "study mark color differs")
            axis.tick_params(axis="y", labelleft=column_number == 0)
            if column_number == 0:
                require([v.get_text() for v in axis.get_yticklabels()] == labels, "failure/coverage row labels differ")
            for rank, variant in enumerate(VARIANTS):
                found = [r for r in selected if r["variant"] == variant]
                if not found or found[0].get(key) is None:
                    text = "0 identity; no estimated interval" if field == "paired" and variant == VARIANTS[0] else "no numeric value"
                    axis.text(.02, rank, text, transform=axis.get_yaxis_transform(), fontsize=8, color="#666666")
            if field != "paired":
                maximum = max((r[k] for r in rows for k, _ in columns if r.get(k) is not None), default=1) if field == "quality" else max((r[key] for r in rows if r.get(key) is not None), default=1)
                axis.set_xlim(0, max(maximum * 1.12, .001))
                require(axis.get_xlim()[0] == 0, "study axis lost zero baseline")
            else:
                ends = [{"x": r[bound], "rank": VARIANTS.index(r["variant"]), "variant": r["variant"]}
                        for r in selected if r["low"] is not None for bound in ("low", "high")]
                before = len(axis.lines)
                if ends:
                    sns.lineplot(data=pd.DataFrame(ends), x="x", y="rank", units="variant", estimator=None,
                                 sort=False, color=COLOR, linewidth=1.8, legend=False, ax=axis)
                lines = axis.lines[before:]
                intervals = [r for r in selected if r["low"] is not None]
                require(len(lines) == len(intervals), "missing/extra frozen interval")
                observed_intervals = sorted((tuple(map(float, line.get_xdata())), tuple(map(float, line.get_ydata()))) for line in lines)
                expected_intervals = sorted(((r["low"], r["high"]), (float(VARIANTS.index(r["variant"])),) * 2) for r in intervals)
                for observed, wanted in zip(observed_intervals, expected_intervals, strict=True):
                    require(np.allclose(observed, wanted, rtol=1e-12, atol=1e-12), "frozen interval endpoints differ")
                sns.lineplot(x=[0, 0], y=[-.45, len(VARIANTS) - .55], sort=False, estimator=None, color="#666666", linestyle="--", ax=axis)
                extent = [0] + [r[k] for r in rows for k in ("value", "low", "high") if r.get(k) is not None]
                margin = max(max(extent) - min(extent), .1) * .06
                axis.set_xlim(min(extent) - margin, max(extent) + margin)
            # Seaborn interval layers can restore the internal column name.
            axis.set_ylabel("")
            verified.append({"cpus": cpus, "panel": label, "y_axis_label": axis.get_ylabel(), "marks": [{"value": x, "rank": y} for x, y in actual],
                             "row_labels": labels, "missing_numeric_rows": [v for v in VARIANTS if not any(r["variant"] == v and r.get(key) is not None for r in selected)], "x_limits": list(axis.get_xlim()), "intervals": [{k: r[k] for k in ("variant", "low", "high")} for r in selected if field == "paired" and r["low"] is not None]})
    # Shared tick formatters can overwrite another CPU facet after its first check.
    for row_number, cpus in enumerate((1, 2)):
        final_labels = row_labels([r for r in rows if r["cpus"] == cpus], cpus, field)
        require([v.get_text() for v in graph.axes[row_number, 0].get_yticklabels()] == final_labels, "final facet count labels were overwritten")
    prefix = "SYNTHETIC SMOKE — NOT STUDY RESULTS · " if synthetic else ""
    graph.figure.suptitle(prefix + title, y=1.025, fontsize=14, fontweight="bold")
    graph.figure.text(.015, -.025, note, fontsize=9, va="top", color="#404040")
    graph.figure.subplots_adjust(left=.41 if len(columns) == 1 else .24, hspace=.3, wspace=.12)
    for suffix in ("png", "svg"):
        graph.figure.savefig(output / f"{stem}.{suffix}", dpi=160, bbox_inches="tight", facecolor="white",
                             metadata={"Date": None} if suffix == "svg" else None)
    graph.figure.clear()
    return {"stem": stem, "title": prefix + title, "verified_artists": verified, "visual_review": "pending"}


def plot_strata(rows, stem, output, synthetic):
    """One mark per arm and stratum level, faceted by CPU budget and axis; colours identify arms."""
    import numpy as np
    import pandas as pd
    import seaborn as sns

    sns.set_theme(style="whitegrid", context="notebook", font="DejaVu Sans",
                  rc={"svg.hashsalt": "tika-indra-measured-v1"})
    palette = dict(zip(LABELS, ARM_COLORS, strict=True))
    arms = dict(zip(VARIANTS, LABELS, strict=True))
    levels = {}
    for axis_name in STRATA_AXES:
        selected = {}
        for row in rows:
            if row["axis"] == axis_name:
                require(selected.setdefault(row["level"], row["selected"]) == row["selected"], "stratum selection differs between arms")
        # Larger strata first; ties alphabetical. Selection counts never depend on an arm.
        levels[axis_name] = sorted(selected, key=lambda level: (-selected[level], level))
    def position(row):
        return levels[row["axis"]].index(row["level"]) + (VARIANTS.index(row["variant"]) - (len(VARIANTS) - 1) / 2) * STRATA_DODGE
    points = [{**row, "panel": row["axis"], "arm": arms[row["variant"]], "x": finite(row["value"]), "rank": position(row)}
              for row in rows if row["value"] is not None]
    empty = [{"cpus": cpus, "panel": axis_name, "arm": LABELS[0], "x": float("nan"), "rank": float("nan")} for cpus in (1, 2) for axis_name in STRATA_AXES]
    frame = pd.DataFrame(points + empty)
    graph = sns.relplot(data=frame, kind="scatter", x="x", y="rank", hue="arm", hue_order=list(LABELS), palette=palette,
                        row="cpus", col="panel", row_order=[1, 2], col_order=list(STRATA_AXES),
                        s=58, edgecolor="white", linewidth=.6, height=4.6, aspect=1.25,
                        facet_kws={"sharex": False, "sharey": False, "legend_out": True})
    maximum = max((r["value"] for r in rows if r["value"] is not None), default=1)
    verified = []
    for row_number, cpus in enumerate((1, 2)):
        for column_number, axis_name in enumerate(STRATA_AXES):
            axis = graph.axes[row_number, column_number]
            selected = [r for r in rows if r["cpus"] == cpus and r["axis"] == axis_name]
            ordered = levels[axis_name]
            labels = [f"{level} · n{next(r['selected'] for r in rows if r['axis'] == axis_name and r['level'] == level)}" for level in ordered]
            axis.set_yticks(range(len(ordered)), labels=labels)
            axis.set_ylim(len(ordered) - .35, -.65)
            axis.set_ylabel("")
            axis.set_xlabel("Strict CER (%) · lower is better")
            axis.set_title(f"{cpus} CPU · {axis_name}")
            axis.set_xlim(0, max(maximum * 1.12, .001))
            require(axis.get_xlim()[0] == 0, "strata axis lost zero baseline")
            collections = [c for c in axis.collections if len(c.get_offsets())]
            observed = []
            for collection in collections:
                require(np.all(collection.get_sizes() > 0), "invisible stratum mark")
                colors = collection.get_facecolors()
                require(len(colors) in (1, len(collection.get_offsets())), "unreadable stratum mark colours")
                for index, point in enumerate(collection.get_offsets()):
                    color = colors[index if len(colors) > 1 else 0][:3]
                    observed.append((float(point[0]), float(point[1]), tuple(round(float(c), 6) for c in color)))
            expected = []
            for row in selected:
                if row["value"] is not None:
                    color = tuple(round(int(palette[arms[row["variant"]]][i:i + 2], 16) / 255, 6) for i in (1, 3, 5))
                    expected.append((float(row["value"]), float(position(row)), color))
            require(len(observed) == len(expected), "missing/extra stratum mark")
            for seen, wanted in zip(sorted(observed), sorted(expected), strict=True):
                require(np.allclose(seen[:2], wanted[:2], rtol=1e-12, atol=1e-12), "stratum mark value/position differs")
                require(np.allclose(seen[2], wanted[2], atol=1e-6), "stratum mark colour differs from its arm")
            axis.tick_params(axis="y", labelleft=True)
            require([v.get_text() for v in axis.get_yticklabels()] == labels, "stratum level labels differ")
            verified.append({"cpus": cpus, "panel": axis_name, "y_axis_label": axis.get_ylabel(), "row_labels": labels, "x_limits": list(axis.get_xlim()), "dodge": STRATA_DODGE,
                             "marks": [{"variant": r["variant"], "level": r["level"], "value": r["value"], "rank": ordered.index(r["level"]), "y": position(r)} for r in selected if r["value"] is not None],
                             "missing_numeric_rows": [{"variant": r["variant"], "level": r["level"]} for r in selected if r["value"] is None],
                             "coverage": [{k: r[k] for k in ("variant", "level", "scored", "selected", "failures", "metric_limits", "clusters")} for r in selected]})
    for row_number, cpus in enumerate((1, 2)):
        for column_number, axis_name in enumerate(STRATA_AXES):
            final_labels = [f"{level} · n{next(r['selected'] for r in rows if r['axis'] == axis_name and r['level'] == level)}" for level in levels[axis_name]]
            require([v.get_text() for v in graph.axes[row_number, column_number].get_yticklabels()] == final_labels, "final stratum labels were overwritten")
    legend = graph.legend
    require(legend is not None and [text.get_text() for text in legend.get_texts()] == list(LABELS), "arm legend differs")
    prefix = "SYNTHETIC SMOKE — NOT STUDY RESULTS · " if synthetic else ""
    title = "Main primary strata: strict CER by appearance, script and domain (descriptive)"
    graph.figure.suptitle(prefix + title, y=1.02, fontsize=14, fontweight="bold")
    graph.figure.text(.015, -.03,
        "Repetition 1 only · micro strict CER per stratum level; n = selected cases in the level (identical for every arm).\n"
        "A level without a scoreable first attempt for an arm has no mark; per-arm scored/selected, failures (F) and metric limits (M)\n"
        "are in plot-validation.json and summaries.csv. Small strata: descriptive only, no interval, no ranking or population claim.",
        fontsize=9, va="top", color="#404040")
    graph.figure.subplots_adjust(left=.2, hspace=.32, wspace=.5)
    # Level labels sit in the gap before their panel; the widest one (long
    # domain names) must not reach into the previous panel. Widen the figure so
    # every panel keeps its width and every gap holds that label.
    graph.figure.canvas.draw()
    renderer = graph.figure.canvas.get_renderer()
    width = graph.figure.get_figwidth()
    widest = max(label.get_window_extent(renderer).width for axis in graph.axes.flat for label in axis.get_yticklabels()) / graph.figure.dpi + .3
    first, second = graph.axes[0, 0].get_position(), graph.axes[0, 1].get_position()
    panel, gap = first.width * width, (second.x0 - first.x1) * width
    if widest > gap:
        parameters = graph.figure.subplotpars
        graph.figure.set_figwidth(width + (widest - gap) * (len(STRATA_AXES) - 1) / (parameters.right - parameters.left))
        graph.figure.subplots_adjust(wspace=widest / panel)
        graph.figure.canvas.draw()
        renderer = graph.figure.canvas.get_renderer()
    for row_number in range(2):
        for column_number in range(1, len(STRATA_AXES)):
            previous = graph.axes[row_number, column_number - 1].get_window_extent(renderer)
            for label in graph.axes[row_number, column_number].get_yticklabels():
                require(label.get_window_extent(renderer).x0 >= previous.x1, "stratum level label reaches into the previous panel")
    gap = (graph.axes[0, 1].get_position().x0 - graph.axes[0, 0].get_position().x1) * graph.figure.get_figwidth()
    for suffix in ("png", "svg"):
        graph.figure.savefig(output / f"{stem}.{suffix}", dpi=160, bbox_inches="tight", facecolor="white",
                             metadata={"Date": None} if suffix == "svg" else None)
    graph.figure.clear()
    return {"stem": stem, "title": prefix + title, "verified_artists": verified, "visual_review": "pending",
            "level_label_gap_inches": round(gap, 3), "widest_level_label_inches": round(widest, 3)}


def render(admitted, output, *, synthetic=False):
    """Caller supplies study.load's result; notebook never supplies synthetic data."""
    output = Path(output)
    require(not output.is_symlink(), "figure output must not be a symlink")
    output.mkdir(parents=True, exist_ok=True)
    allowed = {name + ".csv" for name in TABLES} | {"plot-validation.json"} | {stem + "." + suffix for stem in STEMS for suffix in ("png", "svg")}
    for path in output.iterdir():
        require(path.is_file() and not path.is_symlink() and path.name in allowed, "unexpected generated figure entry")
        path.unlink()
    data = tables(admitted)
    for name, fields in TABLES.items():
        with (output / (name + ".csv")).open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fields)
            writer.writeheader()
            writer.writerows(data[name])
    plots = []
    quality = data["quality"]
    for cohort, group, title, stem in [
        ("main", "primary", "Main primary: strict and collapsed CER", STEMS[0]),
        ("main", "challenge", "Main challenges: descriptive quality", STEMS[3]),
        ("laboratory", "primary", "Laboratory crops: descriptive quality, two articles", STEMS[4]),
    ]:
        rows = [r for r in quality if r["study"] == cohort and r["cohort"] == group]
        if rows:
            # Quality metrics become panels; no aggregation or Seaborn-fitted interval.
            wide = []
            for cpus in (1, 2):
                for variant in VARIANTS:
                    selected = [r for r in rows if r["cpus"] == cpus and r["variant"] == variant]
                    if selected:
                        require(len(selected) == 2, "quality metric panel incomplete")
                        wide.append({**selected[0], **{r["metric"]: r["value"] for r in selected}})
            plots.append(plot(wide, [(v, v) for v in METRICS.values()], title,
                "Repetition 1 only · scored/selected; F=HTTP/service failures; M=metric limits · lower is better.\n"
                "Missing/unscoreable cells have no mark. Conditional selected-corpus CER; editorial/crop qualifications remain.",
                stem, output, "quality", synthetic))
    paired = baseline_pairs(data["paired"])
    if paired:
        plots.append(plot(paired, [("value", "Strict CER difference (percentage points)")],
            "Main primary: paired difference versus fast PSM3", "Arm minus fast PSM3; negative favors the arm · baseline is the reference, not an estimated self-comparison.\n"
            "Retained frozen document-cluster percentile intervals; joint first-attempt successes only; absent intervals stay absent.\n"
            "J=joint/selected; N=metric-eligible cases; k=clusters; R=reference scalars; F/U=baseline/arm failures/unscoreable.\n"
            "Conditional subsets differ by pair; no whole-arm benefit, multiplicity-adjusted or population superiority claim.",
            STEMS[1], output, "paired", synthetic))
    performance = [r for r in data["performance"] if r["study"] == "main"]
    if performance:
        plots.append(plot(performance, [("throughput", "Successful case responses / workload s"), ("p50_ms", "Successful complete-body p50 (ms)"), ("p95_ms", "Successful complete-body p95 (ms)")],
            "Main whole workload: throughput and finite latency observations", "All 41 primary+challenge cases ×3 technical repeats · n=successful/expected attempts; F=all-attempt failures.\n"
            "Higher throughput/lower latency is favorable. Wall includes failures/persistence; type7 latency quantiles condition on success.\n"
            "One warmup excluded · 1/2 CPU, shared 3GiB/zero swap, load generator included · no isolated CPU-scaling claim.",
            STEMS[2], output, "performance", synthetic))
    strata = strata_rows(data["summaries"])
    if strata:
        plots.append(plot_strata(strata, STEMS[5], output, synthetic))
    files = {p.name: {"sha256": hashlib.sha256(p.read_bytes()).hexdigest(), "bytes": p.stat().st_size} for p in sorted(output.iterdir())}
    receipt = {"schema_version": 1, "synthetic": synthetic, "measurement_source_sha": admitted.get("source_sha"),
               "study_index_sha256": admitted.get("index_sha256"), "admitted_cells": len(admitted["measurements"]),
               "plots": plots, "files": files, "visual_review": "pending",
               "scope": "Seaborn marks/intervals numerically checked against admitted source rows. No OCR, rescoring or bootstrap performed. Raw attempt export retains all non-warmup failures."}
    (output / "plot-validation.json").write_text(json.dumps(receipt, indent=2, allow_nan=False) + "\n")
    return receipt
