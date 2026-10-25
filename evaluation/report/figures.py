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

from study import COMPARISON_FIELDS, HOST_FIELDS, METADATA, RESOURCE_FIELDS, VARIANTS, require

LABELS = ("Fast PSM3", "Best PSM3", "Best PSM6", "Best PSM11",
          "Python math b64", "Rust math b64")
METRICS = {"cer": "Strict CER (%)", "cer_whitespace_collapsed": "Collapsed CER (%)"}
PAIRED_METRICS = ("cer", "cer_whitespace_collapsed", "wer")
COLOR = "#345995"
# One fixed colour per arm for the descriptive strata view; never a fitted scale.
ARM_COLORS = ("#345995", "#d9822b", "#2a9d8f", "#7b4fa3", "#b63a3a", "#5d6d7e")
STRATA_AXES = ("appearance", "script", "domain")
# Fixed vertical offset per arm inside a stratum row; the offset is part of the
# verified position, not a value. plot_strata sizes the figure so adjacent
# offsets sit at least one mark diameter apart in every panel, so equal values
# of six arms never hide one another.
STRATA_DODGE = .14
STRATA_MARK = 30
# Workload panels whose per-repetition values are drawn as min-max whiskers.
REPETITION_RANGES = ("throughput", "p50_ms")
STEMS = ("main-primary-quality", "main-primary-paired", "main-workload-performance",
         "main-challenge-quality", "laboratory-quality", "main-primary-strata")
TABLES = {
    "comparison-statuses": COMPARISON_FIELDS,
    "quality": ("study", "cohort", "cpus", "variant", "metric", "value", "scored", "selected", "failures", "metric_limits", "clusters", "reference_units", "errors"),
    "resources": ("study", "cpus", "variant", *RESOURCE_FIELDS),
    "performance": ("study", "cpus", "variant", "throughput", "p50_ms", "p95_ms", "p99_ms", "latency_quantile_method", "successes", "failures", "expected", "wall_us"),
    "performance-repetitions": ("study", "cpus", "variant", "repetition", "attempts", "successes", "failures", "wall_us", "sum_attempt_elapsed_us", "throughput", "p50_ms"),
    "hosts": HOST_FIELDS,
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


def successful(attempt):
    return attempt["status"] is not None and 200 <= attempt["status"] < 300 and attempt["error"] is None


def tables(admitted):
    result = {name: [] for name in TABLES}
    result["comparison-statuses"] = admitted["comparison_statuses"]
    result["hosts"] = admitted["hosts"]
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
        successes = [a["elapsed_us"] for a in attempts if successful(a)]
        performance = measured["performance"]
        require(len(attempts) == performance["expected_attempts"] and len(successes) == performance["successful_attempts"], "plot timing coverage differs")
        require(all(type(a["elapsed_us"]) is int and a["elapsed_us"] >= 0 for a in attempts), "invalid plot observation")
        result["performance"].append({**identity,
            "throughput": finite(performance["successful_case_responses_per_complete_workload_wall_second"]),
            **{f"p{p}_ms": None if not successes else quantile(successes, p / 100) / 1000 for p in (50, 95, 99)},
            "latency_quantile_method": "linear_type7_successful_nonwarmup_attempts",
            "successes": len(successes), "failures": performance["failed_attempts"],
            "expected": performance["expected_attempts"], "wall_us": performance["completed_repetition_wall_us"]})
        # Post-hoc descriptive spread: each technical repetition's own throughput
        # and successful-attempt median, from the admitted repetition records.
        repetitions = performance["completed_repetitions"]
        require(sum(r["successful_attempts"] for r in repetitions) == len(successes) and
                sum(r["wall_us"] for r in repetitions) == performance["completed_repetition_wall_us"], "repetition totals differ")
        for repetition in repetitions:
            elapsed = [a["elapsed_us"] for a in attempts if a["repetition"] == repetition["repetition"] and successful(a)]
            require(len(elapsed) == repetition["successful_attempts"] and repetition["wall_us"] > 0, "repetition timing coverage differs")
            result["performance-repetitions"].append({**identity, "repetition": repetition["repetition"], "attempts": repetition["attempts"],
                "successes": repetition["successful_attempts"], "failures": repetition["failed_attempts"], "wall_us": repetition["wall_us"],
                "sum_attempt_elapsed_us": repetition["sum_attempt_elapsed_us"],
                "throughput": repetition["successful_attempts"] * 1_000_000 / repetition["wall_us"],
                "p50_ms": None if not elapsed else quantile(elapsed, .5) / 1000})
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
    """Descriptive main-primary strict and collapsed CER by appearance, script and domain; no inference."""
    rows, collapsed = [], {}
    for summary in summaries:
        if summary["study"] != "main" or summary["cohort"] != "primary" or summary["axis"] not in STRATA_AXES:
            continue
        identity = (summary["cpus"], summary["axis"], summary["level"], summary["variant"])
        if summary["metric"] == "cer_whitespace_collapsed":
            require(collapsed.setdefault(identity, summary) is summary, "duplicate stratum row")
        elif summary["metric"] == "cer":
            rows.append({"study": summary["study"], "cpus": summary["cpus"], "variant": summary["variant"], "axis": summary["axis"], "level": summary["level"],
                         "value": percent(summary["micro_rate"]), "scored": summary["scored_cases"], "selected": summary["selected_cases"],
                         "failures": summary["service_failures"], "metric_limits": summary["metric_limits"], "clusters": summary["scored_document_clusters"],
                         "reference_units": summary["reference_units"], "errors": summary["errors"]})
    require(len({(r["cpus"], r["axis"], r["level"], r["variant"]) for r in rows}) == len(rows) == len(collapsed), "duplicate or unpaired stratum row")
    for row in rows:
        partner = collapsed[(row["cpus"], row["axis"], row["level"], row["variant"])]
        row.update(collapsed=percent(partner["micro_rate"]), collapsed_reference_units=partner["reference_units"], collapsed_errors=partner["errors"])
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


def host_note(hosts):
    """Footer lines naming each budget's CPU; budgets compare only when one host ran both."""
    lines = []
    for host in hosts:
        cores = "unknown physical cores" if host["physical_cores"] is None else f"{host['physical_cores']} physical core" + "s" * (host["physical_cores"] != 1)
        lines.append(f"{host['cpus']} CPU: {host['cpu_model']}, {'with' if host['avx512f'] else 'no'} AVX-512F · logical CPU{'s' * (host['cpus'] > 1)} "
                     f"{host['selected_cpus']} on {cores} (sibling lists {host['thread_siblings']}) · host {host['host']}, kernel {host['kernel']}")
    if len(hosts) > 1:
        lines.append("Both budgets ran on one host." if len({(h["host"], h["kernel"], h["cpu_model"]) for h in hosts}) == 1 else
                     "The budgets ran on different hosts, so a per-arm 1-versus-2 CPU difference mixes host and budget.")
    return lines


def segments(axis, rows, bounds, message):
    """One Seaborn line per row between its (low, high) bounds; endpoints checked against the rows."""
    import numpy as np
    import pandas as pd
    import seaborn as sns

    before = len(axis.lines)
    if rows:
        ends = [{"x": bound, "rank": VARIANTS.index(r["variant"]), "variant": r["variant"]} for r in rows for bound in bounds(r)]
        sns.lineplot(data=pd.DataFrame(ends), x="x", y="rank", units="variant", estimator=None,
                     sort=False, color=COLOR, linewidth=1.8, legend=False, ax=axis)
    lines = axis.lines[before:]
    require(len(lines) == len(rows), "missing/extra " + message)
    observed = sorted((tuple(map(float, line.get_xdata())), tuple(map(float, line.get_ydata()))) for line in lines)
    expected = sorted((tuple(bounds(r)), (float(VARIANTS.index(r["variant"])),) * 2) for r in rows)
    for seen, wanted in zip(observed, expected, strict=True):
        require(np.allclose(seen, wanted, rtol=1e-12, atol=1e-12), message + " endpoints differ")
    return [{"variant": r["variant"], "low": bounds(r)[0], "high": bounds(r)[1]} for r in rows]


def plot(rows, columns, title, note, stem, output, field, synthetic):
    import matplotlib.pyplot as plt
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
            intervals = []
            if field != "paired":
                if field == "performance" and key in REPETITION_RANGES:
                    intervals = segments(axis, [r for r in selected if r.get(key + "_range") is not None], lambda r: r[key + "_range"], "repetition range")
                bounds = [r[key + "_range"][1] for r in rows if r.get(key + "_range") is not None]
                maximum = max((r[k] for r in rows for k, _ in columns if r.get(k) is not None), default=1) if field == "quality" else max([r[key] for r in rows if r.get(key) is not None] + bounds, default=1)
                axis.set_xlim(0, max(maximum * 1.12, .001))
                require(axis.get_xlim()[0] == 0, "study axis lost zero baseline")
            else:
                intervals = segments(axis, [r for r in selected if r["low"] is not None], lambda r: (r["low"], r["high"]), "frozen interval")
                sns.lineplot(x=[0, 0], y=[-.45, len(VARIANTS) - .55], sort=False, estimator=None, color="#666666", linestyle="--", ax=axis)
                extent = [0] + [r[k] for r in rows for k in ("value", "low", "high") if r.get(k) is not None]
                margin = max(max(extent) - min(extent), .1) * .06
                axis.set_xlim(min(extent) - margin, max(extent) + margin)
            # Seaborn interval layers can restore the internal column name.
            axis.set_ylabel("")
            verified.append({"cpus": cpus, "panel": label, "y_axis_label": axis.get_ylabel(), "marks": [{"value": x, "rank": y} for x, y in actual],
                             "row_labels": labels, "missing_numeric_rows": [v for v in VARIANTS if not any(r["variant"] == v and r.get(key) is not None for r in selected)], "x_limits": list(axis.get_xlim()), "intervals": intervals})
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
    plt.close(graph.figure)
    return {"stem": stem, "title": prefix + title, "verified_artists": verified, "visual_review": "pending"}


def plot_strata(rows, stem, output, synthetic):
    """Strict (filled) and collapsed (open) CER per arm and stratum level, faceted by CPU budget and axis."""
    import matplotlib.colors
    import matplotlib.pyplot as plt
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
    def color(row):
        return tuple(round(c, 6) for c in matplotlib.colors.to_rgb(palette[arms[row["variant"]]]))
    # Mark diameter in points, open-mark edge included. The panel with the most
    # levels sets the axes height that keeps adjacent arm offsets this far apart.
    diameter = math.sqrt(STRATA_MARK) + 1.1
    span = max(len(levels[a]) for a in STRATA_AXES) + .3
    height = max(4.6, span * diameter / STRATA_DODGE / 72 + 1.4)
    points = [{**row, "panel": row["axis"], "arm": arms[row["variant"]], "x": finite(row["value"]), "rank": position(row)}
              for row in rows if row["value"] is not None]
    empty = [{"cpus": cpus, "panel": axis_name, "arm": LABELS[0], "x": float("nan"), "rank": float("nan")} for cpus in (1, 2) for axis_name in STRATA_AXES]
    frame = pd.DataFrame(points + empty)
    graph = sns.relplot(data=frame, kind="scatter", x="x", y="rank", hue="arm", hue_order=list(LABELS), palette=palette,
                        row="cpus", col="panel", row_order=[1, 2], col_order=list(STRATA_AXES),
                        s=STRATA_MARK, edgecolor="white", linewidth=.6, height=height, aspect=5.75 / height,
                        facet_kws={"sharex": False, "sharey": False, "legend_out": True})
    maximum = max((r[k] for r in rows for k in ("value", "collapsed") if r[k] is not None), default=1)
    verified = []
    for row_number, cpus in enumerate((1, 2)):
        for column_number, axis_name in enumerate(STRATA_AXES):
            axis = graph.axes[row_number, column_number]
            selected = [r for r in rows if r["cpus"] == cpus and r["axis"] == axis_name]
            # Each arm's collapsed value: an open mark on the arm's own offset,
            # joined to its strict mark, so the whitespace share reads per arm.
            joined = [r for r in selected if r["value"] is not None and r["collapsed"] is not None]
            before = len(axis.lines)
            if joined:
                ends = [{"x": r[k], "rank": position(r), "unit": r["variant"] + "|" + r["level"], "arm": arms[r["variant"]]} for r in joined for k in ("value", "collapsed")]
                sns.lineplot(data=pd.DataFrame(ends), x="x", y="rank", units="unit", hue="arm", palette=palette, estimator=None,
                             sort=False, linewidth=.9, legend=False, zorder=.9, ax=axis)
            connectors = axis.lines[before:]
            require(len(connectors) == len(joined), "missing/extra strict-collapsed connector")
            observed = sorted((tuple(map(float, line.get_xdata())), tuple(map(float, line.get_ydata())), tuple(round(c, 6) for c in matplotlib.colors.to_rgb(line.get_color()))) for line in connectors)
            expected = sorted(((r["value"], r["collapsed"]), (position(r),) * 2, color(r)) for r in joined)
            for seen, wanted in zip(observed, expected, strict=True):
                require(np.allclose(seen[0] + seen[1], wanted[0] + wanted[1], rtol=1e-12, atol=1e-12) and np.allclose(seen[2], wanted[2], atol=1e-6), "strict-collapsed connector differs")
            hollow = [r for r in selected if r["collapsed"] is not None]
            if hollow:
                sns.scatterplot(data=pd.DataFrame([{"x": finite(r["collapsed"]), "rank": position(r), "arm": arms[r["variant"]]} for r in hollow]),
                                x="x", y="rank", hue="arm", palette=palette, s=STRATA_MARK, linewidth=1.1, legend=False, ax=axis)
                ring = axis.collections[-1]
                ring.set_edgecolors(ring.get_facecolors())
                ring.set_facecolors("none")
                ring.set_gid("collapsed")
            ordered = levels[axis_name]
            labels = [f"{level} · n{next(r['selected'] for r in rows if r['axis'] == axis_name and r['level'] == level)}" for level in ordered]
            axis.set_yticks(range(len(ordered)), labels=labels)
            axis.set_ylim(len(ordered) - .35, -.65)
            axis.set_ylabel("")
            axis.set_xlabel("CER (%) · filled strict, open collapsed · lower is better")
            axis.set_title(f"{cpus} CPU · {axis_name}")
            axis.set_xlim(0, max(maximum * 1.12, .001))
            require(axis.get_xlim()[0] == 0, "strata axis lost zero baseline")
            for gid, field, colours in ((None, "value", "get_facecolors"), ("collapsed", "collapsed", "get_edgecolors")):
                observed = []
                for collection in (c for c in axis.collections if len(c.get_offsets()) and c.get_gid() == gid):
                    require(np.all(collection.get_sizes() > 0), "invisible stratum mark")
                    tints = getattr(collection, colours)()
                    require(len(tints) in (1, len(collection.get_offsets())), "unreadable stratum mark colours")
                    for index, point in enumerate(collection.get_offsets()):
                        tint = tints[index if len(tints) > 1 else 0][:3]
                        observed.append((float(point[0]), float(point[1]), tuple(round(float(c), 6) for c in tint)))
                expected = [(float(r[field]), float(position(r)), color(r)) for r in selected if r[field] is not None]
                require(len(observed) == len(expected), "missing/extra stratum mark")
                for seen, wanted in zip(sorted(observed), sorted(expected), strict=True):
                    require(np.allclose(seen[:2], wanted[:2], rtol=1e-12, atol=1e-12), "stratum mark value/position differs")
                    require(np.allclose(seen[2], wanted[2], atol=1e-6), "stratum mark colour differs from its arm")
            # Collapsed values near 0% would otherwise lose half a mark to the zero baseline.
            for artist in (*axis.collections, *connectors):
                artist.set_clip_on(False)
            axis.tick_params(axis="y", labelleft=True)
            require([v.get_text() for v in axis.get_yticklabels()] == labels, "stratum level labels differ")
            verified.append({"cpus": cpus, "panel": axis_name, "y_axis_label": axis.get_ylabel(), "row_labels": labels, "x_limits": list(axis.get_xlim()), "dodge": STRATA_DODGE,
                             "marks": [{"variant": r["variant"], "level": r["level"], "value": r["value"], "rank": ordered.index(r["level"]), "y": position(r)} for r in selected if r["value"] is not None],
                             "collapsed_marks": [{"variant": r["variant"], "level": r["level"], "value": r["collapsed"], "y": position(r)} for r in hollow],
                             "missing_numeric_rows": [{"variant": r["variant"], "level": r["level"]} for r in selected if r["value"] is None],
                             "coverage": [{k: r[k] for k in ("variant", "level", "scored", "selected", "failures", "metric_limits", "clusters")} for r in selected]})
    for row_number, cpus in enumerate((1, 2)):
        for column_number, axis_name in enumerate(STRATA_AXES):
            final_labels = [f"{level} · n{next(r['selected'] for r in rows if r['axis'] == axis_name and r['level'] == level)}" for level in levels[axis_name]]
            require([v.get_text() for v in graph.axes[row_number, column_number].get_yticklabels()] == final_labels, "final stratum labels were overwritten")
    legend = graph.legend
    require(legend is not None and [text.get_text() for text in legend.get_texts()] == list(LABELS), "arm legend differs")
    prefix = "SYNTHETIC SMOKE — NOT STUDY RESULTS · " if synthetic else ""
    title = "Main primary strata: strict and whitespace-collapsed CER by appearance, script and domain (descriptive)"
    graph.figure.suptitle(prefix + title, y=1.01, fontsize=14, fontweight="bold")
    graph.figure.text(.015, -.015,
        "Repetition 1 only · micro CER per stratum level: filled = strict, open = whitespace-collapsed, joined per arm; n = selected cases in the level\n"
        "(identical for every arm). Arms sit at fixed offsets at least one mark apart. A level without a scoreable first attempt for an arm has no mark;\n"
        "per-arm scored/selected, failures (F) and metric limits (M) are in plot-validation.json and summaries.csv. Small strata: descriptive only,\n"
        "no interval, no ranking or population claim.",
        fontsize=9, va="top", color="#404040")
    graph.figure.subplots_adjust(left=.2, wspace=.5)
    # About one inch between the rows for the x label and the next title;
    # hspace is a fraction of the (tall) axes height, not a length.
    graph.figure.subplots_adjust(hspace=1 / (graph.axes[0, 0].get_position().height * graph.figure.get_figheight()))
    def offsets():
        # Points between adjacent arm offsets, per panel, at the final layout.
        return [graph.axes[r, c].get_position().height * graph.figure.get_figheight() * 72 / (len(levels[a]) + .3) * STRATA_DODGE
                for r in range(2) for c, a in enumerate(STRATA_AXES)]
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
    for column_number in range(len(STRATA_AXES)):
        upper, lower = graph.axes[0, column_number], graph.axes[1, column_number]
        require(upper.xaxis.label.get_window_extent(renderer).y0 > lower.title.get_window_extent(renderer).y1, "strata axis label reaches into the next row's title")
    spacing = min(offsets())
    require(spacing >= diameter, "adjacent arm offsets are closer than one mark")
    gap = (graph.axes[0, 1].get_position().x0 - graph.axes[0, 0].get_position().x1) * graph.figure.get_figwidth()
    for suffix in ("png", "svg"):
        graph.figure.savefig(output / f"{stem}.{suffix}", dpi=160, bbox_inches="tight", facecolor="white",
                             metadata={"Date": None} if suffix == "svg" else None)
    plt.close(graph.figure)
    return {"stem": stem, "title": prefix + title, "verified_artists": verified, "visual_review": "pending",
            "level_label_gap_inches": round(gap, 3), "widest_level_label_inches": round(widest, 3),
            "arm_offset_points": round(spacing, 3), "mark_diameter_points": round(diameter, 3)}


def export(data, output):
    """Write every table as the CSV the report and analysis.py read back."""
    for name, fields in TABLES.items():
        with (Path(output) / (name + ".csv")).open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fields)
            writer.writeheader()
            writer.writerows(data[name])


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
    export(data, output)
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
    performance = [dict(r) for r in data["performance"] if r["study"] == "main"]
    for row in performance:
        repetitions = [r for r in data["performance-repetitions"] if (r["study"], r["cpus"], r["variant"]) == (row["study"], row["cpus"], row["variant"])]
        for key in REPETITION_RANGES:
            values = [r[key] for r in repetitions if r[key] is not None]
            row[key + "_range"] = (min(values), max(values)) if values else None
    if performance:
        plots.append(plot(performance, [("throughput", "Successful case responses / workload s"), ("p50_ms", "Successful complete-body p50 (ms)"), ("p95_ms", "Successful complete-body p95 (ms)")],
            "Main whole workload: throughput and finite latency observations", "\n".join([
            "All 41 primary+challenge cases ×3 technical repeats · n=successful/expected attempts; F=all-attempt failures.",
            "Higher throughput/lower latency is favorable. Wall includes failures/persistence; type7 latency quantiles condition on success.",
            "Whiskers: min–max of the three per-repetition throughput and p50 values (post-hoc descriptive addendum, no interval; none on p95).",
            "One warmup excluded · 1/2 CPU, shared 3GiB/zero swap, load generator included · no isolated CPU-scaling claim.",
            *host_note(admitted["hosts"])]),
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
