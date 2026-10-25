"""Descriptive reading of the exported study tables; no OCR, rescoring or resampling.

Every number comes from the study-data CSVs that figures.render wrote from
study.load's admitted evidence, so the text cannot drift from the downloads.
Strata, length bins and traits are post-hoc descriptive views: no interval,
ranking or population claim, and small groups stay labelled with their size.
An absent value (failed or unscoreable) is reported as absent, never as zero.
"""
from __future__ import annotations

import csv
from pathlib import Path
import re

from figures import host_note
from study import ROOT, VARIANTS, decode, require

NAMES = dict(zip(VARIANTS, ("fast PSM3", "best PSM3", "best PSM6", "best PSM11", "Python Indra", "Rust Indra"), strict=True))
TESSERACT, INDRA = VARIANTS[:4], VARIANTS[4:]
STRATA = ("language", "script", "domain", "appearance", "reference_quality")
# Manifest traits that describe typography, layout or physical condition;
# provenance flags such as training_overlap_unknown are left out.
TRAITS = {"typography": ("italic_taxonomy", "superscript_notation", "mixed_type_sizes"),
          "layout": ("tables", "two_columns", "vertical_text", "footnotes", "running_headers", "printed_form", "handwritten_entries"),
          "condition": ("parchment", "crossouts", "torn_paper", "bleedthrough", "heavy_bleedthrough")}


def read(directory, name):
    with (Path(directory) / (name + ".csv")).open(newline="") as stream:
        return list(csv.DictReader(stream))


def number(text):
    """An exported numeric cell; an empty cell is an absent value, never zero."""
    return None if text == "" else float(text)


def percent(rate):
    return "no value" if rate is None else f"{100 * rate:.2f}%"


def span(values, unit="%"):
    values = [v for v in values if v is not None]
    if not values:
        return "no value"
    shown = (lambda v: f"{v:.2f}") if any(isinstance(v, float) for v in values) else str
    low, high = shown(min(values)), shown(max(values))
    return f"{low}{unit}" if low == high else f"{low}–{high}{unit}"


def code(text):
    """Inline Markdown code for an evidence string, so no $, * or _ in it is read as markup."""
    fence = "`" * (max(map(len, re.findall("`+", text)), default=0) + 1)
    return f"{fence} {text} {fence}"


def plural(count, word):
    return f"{count} {word}{'s' * (count != 1)}"


def budgets(sentences):
    """One sentence for both budgets when they read the same, else one per budget."""
    if len(sentences) == 2 and sentences[1] == sentences[2]:
        return "At both CPU budgets, " + sentences[1]
    return " ".join(f"At {cpus} CPU{'s' * (cpus > 1)}, {text}" for cpus, text in sorted(sentences.items()))


def indra(values, unit="%"):
    """'26.59% for either Indra arm' when both arms agree, else one value per arm."""
    shown = lambda value: "no value" if value is None else f"{value:.2f}{unit}" if isinstance(value, float) else f"{value}{unit}"
    if len({values.get(v) for v in INDRA}) == 1:
        return f"{shown(values.get(INDRA[0]))} for either Indra arm"
    return ", ".join(f"{shown(values.get(v))} for {NAMES[v]}" for v in INDRA)


def per_case(rows):
    """Micro rate of case rows: summed errors over summed reference units."""
    reference = sum(int(r["reference_units"]) for r in rows)
    return sum(int(r["errors"]) for r in rows) / reference if reference else None


def length_bins(directory, repository=ROOT):
    """Prespecified reference-length bins (corpus plan), main primary, per arm and budget."""
    edges = decode((repository / "evaluation/corpus-plan.json").read_bytes())["sampling"]["length_bins_reference_codepoints"]
    require(edges == sorted(edges) and all(type(e) is int and e > 0 for e in edges), "invalid length bins")
    names = [f"<{edges[0]}", *(f"{a}–{b - 1}" for a, b in zip(edges, edges[1:])), f"≥{edges[-1]}"]
    cases = [r for r in read(directory, "cases") if (r["study"], r["cohort"]) == ("main", "primary") and r["metric"] in ("cer", "cer_whitespace_collapsed")]
    # A page's length is its strict NFC reference length; it does not depend on the arm.
    length = {r["case_id"]: int(r["reference_units"]) for r in cases if r["metric"] == "cer" and r["reference_units"] != ""}
    domain = {r["case_id"]: r["domain"] for r in cases}
    bin_of = lambda case: names[sum(length[case] >= e for e in edges)]
    # A bin holding legal and other pages is also shown split, so the two read apart.
    mixed = {bin_of(c) for c in length if domain[c] == "modern_legal"} & {bin_of(c) for c in length if domain[c] != "modern_legal"}
    members = {}
    for case_id in length:
        name = bin_of(case_id)
        for label in (name, *([f"{name} · {'modern_legal' if domain[case_id] == 'modern_legal' else 'other domains'}"] if name in mixed else [])):
            members.setdefault(label, set()).add(case_id)
    order = lambda label: (names.index(label.split(" · ")[0]), label)
    clusters = {r["case_id"]: r["document_cluster"] for r in cases}
    rows = []
    for cpus in sorted({int(r["cpus"]) for r in cases}):
        for label in sorted(members, key=order):
            pages = members[label]
            row = {"cpus": cpus, "length bin": label, "pages": len(pages), "clusters": len({clusters[c] for c in pages}),
                   "reference codepoints": sum(length[c] for c in pages)}
            for variant in VARIANTS:
                chosen = [r for r in cases if r["case_id"] in pages and (int(r["cpus"]), r["variant"]) == (cpus, variant) and r["metric_available"] == "True"]
                scored = len({r["case_id"] for r in chosen if r["metric"] == "cer"})
                # An arm that scored fewer pages than the bin holds says so; its rate covers only those.
                row[NAMES[variant]] = (" · ".join(percent(per_case([r for r in chosen if r["metric"] == metric])) for metric in ("cer", "cer_whitespace_collapsed"))
                                       + ("" if scored == len(pages) else f" ({scored}/{len(pages)} scored)"))
            rows.append(row)
    if not length:
        return rows, ""
    longest = sorted(length, key=length.get, reverse=True)
    leading = next((i for i, c in enumerate(longest) if domain[c] != "modern_legal"), len(longest))
    legal = sum(length[c] for c in length if domain[c] == "modern_legal") / sum(length.values())
    head = ("The longest primary page is not modern_legal." if leading == 0 else
            f"The longest primary page ({length[longest[0]]} codepoints) is modern_legal." if leading == 1 else
            f"The {leading} longest primary pages ({length[longest[leading - 1]]}–{length[longest[0]]} codepoints) are all modern_legal.")
    note = (f"The bins are the corpus plan's prespecified edges {edges} applied to each page's strict NFC reference length; "
            + ", ".join(f"{n}: {plural(sum(bin_of(c) == n for c in length), 'page')}" for n in names) + f". {head} "
            f"Modern legal pages hold {legal:.0%} of the primary strict reference codepoints"
            + (", so the pooled bins mix reference length with the legal domain and its extracted-PDF references; the split rows show the two apart."
               if mixed else ".") + " Descriptive only, no interval.")
    return rows, note


def traits(directory, manifest):
    """Recorded typography, layout and condition traits, per cohort, arm and budget."""
    recorded = {case["id"]: set(case.get("traits") or ()) for case in manifest["cases"]}
    cases = [r for r in read(directory, "cases") if r["study"] == "main" and r["metric"] in ("cer", "cer_whitespace_collapsed") and r["metric_available"] == "True"]
    rows = []
    for group, names in TRAITS.items():
        for trait in names:
            for cohort in ("primary", "challenge"):
                members = [r for r in cases if r["cohort"] == cohort and trait in recorded[r["case_id"]]]
                for cpus in sorted({int(r["cpus"]) for r in members}):
                    selected = [r for r in members if int(r["cpus"]) == cpus]
                    strict = [r for r in selected if r["metric"] == "cer"]
                    row = {"group": group, "trait": trait, "cohort": cohort, "cpus": cpus, "cases": len({r["case_id"] for r in strict}),
                           "clusters": len({r["document_cluster"] for r in strict}), "domains": ", ".join(sorted({r["domain"] for r in strict}))}
                    for variant in VARIANTS:
                        row[NAMES[variant]] = " · ".join(percent(per_case([r for r in selected if r["variant"] == variant and r["metric"] == metric]))
                                                         for metric in ("cer", "cer_whitespace_collapsed"))
                    rows.append(row)
    return rows



def traits_text(rows):
    if not rows:
        return ""
    traits = {(r["trait"], r["cohort"]): r for r in rows}
    single = sum(r["clusters"] == 1 for r in traits.values())
    domain = sum("," not in r["domains"] for r in traits.values())
    return (f"Each arm cell is strict · collapsed CER over the trait's scored pages; primary pages and challenges stay apart. Of the "
            f"{len(traits)} trait and cohort groups, {single} come from a single document cluster and {domain} from a single domain, so a trait "
            "is not separable from its source, domain or cluster here. The manifest records the traits; this view is a post-hoc description.")


def reproduces_summaries(directory):
    """The case-level micro rates used above reproduce every exported domain summary."""
    totals = {}
    for r in read(directory, "cases"):
        if r["metric_available"] == "True":
            key = (r["study"], r["cpus"], r["variant"], r["cohort"], r["domain"], r["metric"])
            errors, reference = totals.get(key, (0, 0))
            totals[key] = (errors + int(r["errors"]), reference + int(r["reference_units"]))
    checked = 0
    for s in read(directory, "summaries"):
        if s["axis"] == "domain" and s["micro_rate"] != "":
            total = totals.get((s["study"], s["cpus"], s["variant"], s["cohort"], s["level"], s["metric"]))
            require(total == (int(s["errors"]), int(s["reference_units"])), "case aggregation differs from the exported summary")
            checked += 1
    return checked


def legal(directory):
    """Modern legal pages: strict, collapsed and WER, and the strict errors that collapsing removes."""
    summaries = read(directory, "summaries")
    domain = {(s["cpus"], s["variant"], s["metric"]): s for s in summaries
              if (s["study"], s["cohort"], s["axis"], s["level"]) == ("main", "primary", "domain", "modern_legal")}
    overall = {(s["cpus"], s["variant"]): s for s in summaries if (s["study"], s["cohort"], s["axis"], s["metric"]) == ("main", "primary", "overall", "cer")}
    rows = []
    for cpus in ("1", "2"):
        for variant in VARIANTS:
            strict, collapsed, wer = (domain.get((cpus, variant, m)) for m in ("cer", "cer_whitespace_collapsed", "wer"))
            if strict is None or strict["micro_rate"] == "" or collapsed["micro_rate"] == "":
                continue
            removed = int(strict["errors"]) - int(collapsed["errors"])
            rows.append({"cpus": int(cpus), "arm": NAMES[variant], "cases": int(strict["selected_cases"]), "clusters": int(strict["document_clusters"]),
                         "strict errors": int(strict["errors"]), "collapsed errors": int(collapsed["errors"]),
                         "strict CER": percent(number(strict["micro_rate"])), "collapsed CER": percent(number(collapsed["micro_rate"])),
                         "WER": percent(number(wer["micro_rate"])),
                         "strict errors removed by collapsing": None if not int(strict["errors"]) else round(100 * removed / int(strict["errors"]), 1),
                         "share of headline strict CER (pp)": round(100 * removed / int(overall[(cpus, variant)]["reference_units"]), 2)})
    by_language, references = {}, set()
    for r in read(directory, "cases"):
        if (r["study"], r["cohort"], r["domain"]) == ("main", "primary", "modern_legal"):
            references.add(r["reference_quality"])
            if (r["metric"], r["metric_available"]) == ("cer_whitespace_collapsed", "True"):
                key = (r["cpus"], r["language"], r["variant"])
                by_language[key] = by_language.get(key, 0) + int(r["errors"])
    return rows, by_language, sorted(references)


def legal_text(rows, by_language, references):
    if not rows:
        return ""
    cpus = min(r["cpus"] for r in rows)
    selected = [r for r in rows if r["cpus"] == cpus]
    shares = [round(r["strict errors removed by collapsing"]) for r in selected if r["strict errors removed by collapsing"] is not None]
    languages = sorted({language for c, language, _ in by_language if c == str(cpus)})
    errors = lambda language, variant: by_language.get((str(cpus), language, variant))
    per_language = "; ".join(f"{language} {indra({v: errors(language, v) for v in INDRA}, '')} against "
                             f"{span([errors(language, v) for v in TESSERACT], '')} for Tesseract" for language in languages)
    # The frozen legal references are audited Poppler -layout text of the PDF
    # layer (corpus-frozen.json transcription_policy), indentation included.
    return (f"The {plural(selected[0]['cases'], 'modern legal page')} ({plural(selected[0]['clusters'], 'cluster')}) have "
            f"{' and '.join(references)} references, the PDF text layer with its layout indentation and padding. Collapsing whitespace removes "
            f"{span(shares, '%')} of each arm's strict errors there, which is {span([r['share of headline strict CER (pp)'] for r in selected], ' pp')} "
            f"of each arm's headline primary strict CER. Whitespace-collapsed errors by language at {cpus} CPU: {per_language}. "
            "The strict view stays the prespecified primary metric; this breakdown is a post-hoc description.")


def quality(directory):
    """Main primary and challenge strict/collapsed CER per arm and budget."""
    rows = read(directory, "quality")
    text = []
    for cohort, label in (("primary", "main primary pages"), ("challenge", "challenge cases")):
        sentences = {}
        for cpus in (1, 2):
            selected = {(r["variant"], r["metric"]): r for r in rows if (r["study"], r["cohort"], r["cpus"]) == ("main", cohort, str(cpus))}
            if not selected:
                continue
            parts = []
            for metric in ("Strict CER (%)", "Collapsed CER (%)"):
                values = {v: number(selected[(v, metric)]["value"]) for v in VARIANTS if (v, metric) in selected}
                parts.append(f"{metric.split(' CER')[0].lower()} CER is {indra(values)} against {span([values.get(v) for v in TESSERACT])} for the Tesseract settings")
            first = selected[(VARIANTS[0], "Strict CER (%)")]
            failures = sum(int(r["failures"]) for (_, metric), r in selected.items() if metric == "Strict CER (%)")
            parts.append("every first attempt was scored" if failures == 0 else f"{plural(failures, 'first attempt')} failed across the arms")
            sentences[cpus] = f"on the {first['selected']} {label} ({plural(int(first['clusters']), 'document cluster')}): " + "; ".join(parts) + "."
        if sentences:
            text.append(budgets(sentences))
    return " ".join(text)


def paired(directory):
    """Indra minus each Tesseract setting, main primary strict CER, frozen intervals."""
    rows = [r for r in read(directory, "paired") if (r["study"], r["cohort"], r["metric"]) == ("main", "primary", "cer")
            and r["left"] in TESSERACT and r["variant"] in INDRA]
    excludes = lambda r: r["low"] != "" and (number(r["high"]) < 0 or number(r["low"]) > 0)
    sentences = {}
    for cpus in (1, 2):
        selected = {(r["left"], r["variant"]): r for r in rows if r["cpus"] == str(cpus)}
        if not selected:
            continue
        parts = []
        for left in TESSERACT:
            shown = {v: ("no paired estimate" if selected[(left, v)]["value"] == "" else
                         f"{number(selected[(left, v)]['value']):+.2f} pp (" + ("no interval" if selected[(left, v)]["low"] == "" else
                         f"{number(selected[(left, v)]['low']):+.2f} to {number(selected[(left, v)]['high']):+.2f}") + ")")
                     for v in INDRA if (left, v) in selected}
            parts.append(f"{indra(shown, '')} against {NAMES[left]}")
        first = next(iter(selected.values()))
        sentences[cpus] = (f"the paired strict-CER differences, Indra minus Tesseract over joint first-attempt successes ({first['clusters']} document "
                           f"clusters, {first['draws']} cluster bootstrap draws), are " + "; ".join(parts) + ".")
    if not sentences:
        return ""
    excluding = sorted({(NAMES[r["variant"]], NAMES[r["left"]]) for r in rows if excludes(r)})
    claim = (f"Of the {len(rows)} intervals ({len(INDRA)} Indra arms, {len(TESSERACT)} Tesseract settings, {plural(len(sentences), 'budget')}), "
             + ("none excludes zero." if not excluding else
                f"{sum(map(excludes, rows))} exclude zero: " + ", ".join(f"{a} against {b}" for a, b in excluding) + "."))
    return (budgets(sentences) + " Negative values favour Indra. " + claim +
            " The intervals are not adjusted for multiplicity, and one that crosses zero shows no difference in either direction.")


def strata_versus_tesseract(directory, worse=True):
    """Main primary strata where every Indra arm's strict CER is above (worse) or below every Tesseract setting's."""
    rows = [r for r in read(directory, "summaries") if (r["study"], r["cohort"]) == ("main", "primary") and r["axis"] in STRATA]
    index = {(r["cpus"], r["axis"], r["level"], r["variant"], r["metric"]): r for r in rows}
    result = []
    for cpus, axis, level in sorted({(r["cpus"], r["axis"], r["level"]) for r in rows}):
        rate = lambda v, m="cer": number(index[(cpus, axis, level, v, m)]["micro_rate"]) if (cpus, axis, level, v, m) in index else None
        tesseract = {v: rate(v) for v in TESSERACT if rate(v) is not None}
        indra_rates = {v: rate(v) for v in INDRA if rate(v) is not None}
        if not tesseract or len(indra_rates) != len(INDRA):
            continue
        best = min(tesseract, key=tesseract.get)
        if (min(indra_rates.values()) > tesseract[best]) if worse else (max(indra_rates.values()) < tesseract[best]):
            row = index[(cpus, axis, level, INDRA[-1], "cer")]
            result.append({"cpus": int(cpus), "axis": axis, "level": level, "cases": int(row["selected_cases"]), "clusters": int(row["document_clusters"]),
                           "Indra strict": " / ".join(dict.fromkeys(percent(indra_rates[v]) for v in INDRA)), "best Tesseract": NAMES[best],
                           "its strict": percent(tesseract[best]),
                           "Indra collapsed": " / ".join(dict.fromkeys(percent(rate(v, "cer_whitespace_collapsed")) for v in INDRA)),
                           "its collapsed": percent(rate(best, "cer_whitespace_collapsed"))})
    return result


def worse_strata_text(strata):
    if not strata:
        return "No main primary stratum has a higher strict CER for both Indra arms than for the best Tesseract setting."
    sentences = {}
    for cpus in (1, 2):
        selected = [r for r in strata if r["cpus"] == cpus]
        if selected:
            sentences[cpus] = (f"both Indra arms have a higher strict CER than the best Tesseract setting on {len(selected)} main primary "
                               f"{'stratum' if len(selected) == 1 else 'strata'}: "
                               + "; ".join(f"{r['axis']} {r['level']} ({plural(r['cases'], 'case')}, {plural(r['clusters'], 'cluster')}; Indra "
                                           f"{r['Indra strict']} against {r['best Tesseract']} {r['its strict']})" for r in selected) + ".")
    handwriting = any(r["axis"] == "appearance" and r["level"] == "handwritten" for r in strata)
    return (budgets(sentences) + (" Handwritten pages are among them." if handwriting else "") +
            " These strata are small and overlap (a page counts once per axis); they are descriptive, with no interval.")


def laboratory(directory):
    """The two laboratory crops: descriptive point estimates and the per-crop error difference."""
    rows = [r for r in read(directory, "quality") if (r["study"], r["cohort"]) == ("laboratory", "primary")]
    cases = [r for r in read(directory, "cases") if (r["study"], r["cohort"], r["metric"]) == ("laboratory", "primary", "cer") and r["metric_available"] == "True"]
    sentences = {}
    for cpus in (1, 2):
        values = {(r["variant"], r["metric"]): number(r["value"]) for r in rows if r["cpus"] == str(cpus)}
        if not values:
            continue
        parts = [f"{metric.split(' CER')[0].lower()} CER is {indra({v: values.get((v, metric)) for v in INDRA})} against "
                 f"{span([values.get((v, metric)) for v in TESSERACT])} for Tesseract" for metric in ("Strict CER (%)", "Collapsed CER (%)")]
        errors = {(r["case_id"], r["variant"]): int(r["errors"]) for r in cases if r["cpus"] == str(cpus)}
        complete = [c for c in sorted({c for c, _ in errors}) if all((c, v) in errors for v in VARIANTS)]
        if complete:
            case = max(complete, key=lambda c: min(errors[(c, v)] for v in INDRA) - min(errors[(c, v)] for v in TESSERACT))
            best = min(TESSERACT, key=lambda v: errors[(case, v)])
            parts.append(f"the largest per-crop difference is on {case}, strict errors {indra({v: errors[(case, v)] for v in INDRA}, '')} "
                         f"against {errors[(case, best)]} for {NAMES[best]}")
        sentences[cpus] = "on the two laboratory crops, " + "; ".join(parts) + "."
    return (budgets(sentences) + " The supplement has two document clusters and zero bootstrap resamples, so these are point descriptions only."
            if sentences else "")


def cpu_cost(directory):
    """Warmup-plus-workload parent CPU time per successful workload response, per arm and budget."""
    resources = {(r["cpus"], r["variant"]): r for r in read(directory, "resources") if r["study"] == "main"}
    performance = {(r["cpus"], r["variant"]): r for r in read(directory, "performance") if r["study"] == "main"}
    rows = []
    for cpus in ("1", "2"):
        for variant in VARIANTS:
            if (cpus, variant) in resources and (cpus, variant) in performance:
                seconds = int(resources[(cpus, variant)]["warmup_workload_cpu_usage_usec"]) / 1e6
                successes = int(performance[(cpus, variant)]["successes"])
                rows.append({"cpus": int(cpus), "arm": NAMES[variant], "CPU seconds": round(seconds, 1), "successes": successes,
                             "CPU s per success": None if not successes else round(seconds / successes, 3),
                             "throughput (responses/s)": round(float(performance[(cpus, variant)]["throughput"]), 3)})
    return rows


def cpu_cost_text(rows, hosts):
    per = {(r["cpus"], r["arm"]): r["CPU s per success"] for r in rows}
    ratios = {arm: per[(2, arm)] / per[(1, arm)] for (cpus, arm) in per if cpus == 1 and per.get((2, arm)) and per[(1, arm)]}
    if not ratios:
        return ""
    same = len({(h["host"], h["kernel"], h["cpu_model"]) for h in hosts}) == 1
    return ("Parent-cgroup CPU time from before warmup to after the workload, divided by successful workload responses: "
            + "; ".join(f"{arm} {per[(1, arm)]:.3f} s at 1 CPU and {per[(2, arm)]:.3f} s at 2 CPUs" for arm in ratios) + ". "
            f"Per response, the 2-CPU cell uses {span(list(ratios.values()), '×')} the CPU time of the 1-CPU cell. "
            + ("Both budgets ran on one host, so this compares budgets on the same hardware." if same else
               "The budgets ran on different hosts (" + "; ".join(line.split(" · ")[0] for line in host_note(hosts)[:len(hosts)]) +
               "), so the ratio mixes host and budget and is not a CPU-scaling measurement."))


def dispatch_text(hosts):
    """The Rust arm's logged startup dispatch on each budget's host; the other arms log none."""
    known = [h for h in hosts if h["rust_isa_avx512f"] is not None]
    if not known:
        return "No completed Rust run of an admitted budget logged a cpu_dispatch record."
    return ("The Rust arm's OCR process logs a cpu_dispatch record at startup: "
            + "; ".join(f"at {h['cpus']} CPU{'s' * (h['cpus'] > 1)} its CPUID check {'finds' if h['rust_isa_avx512f'] else 'does not find'} AVX-512F "
                        f"and OpenCV's feature line reads {code(h['rust_opencv_features'])}" for h in known)
            + ". In that line * marks an extension built for runtime dispatch and a trailing ? one this CPU lacks. ONNX Runtime does not log "
            "its kernel choice, and the Python and Tesseract arms log no dispatch record.")


def repetition_text(directory):
    """Per-repetition throughput spread (post-hoc, descriptive) and the Rust/Python ratio."""
    rows = [r for r in read(directory, "performance-repetitions") if r["study"] == "main"]
    pooled = {(r["cpus"], r["variant"]): float(r["throughput"]) for r in read(directory, "performance") if r["study"] == "main"}
    sentences = {}
    for cpus in ("1", "2"):
        values = {v: [float(r["throughput"]) for r in rows if (r["cpus"], r["variant"]) == (cpus, v)] for v in VARIANTS}
        if not all(values.values()):
            continue
        text = "per-repetition throughput spans " + "; ".join(f"{NAMES[v]} {span(values[v], '')}" for v in VARIANTS) + " responses/s"
        python, rust = values[INDRA[0]], values[INDRA[1]]
        if pooled.get((cpus, INDRA[0])) and all(python):
            ratio = [r / p for r, p in zip(rust, python, strict=True)]
            text += (f", and the Rust/Python throughput ratio is {pooled[(cpus, INDRA[1])] / pooled[(cpus, INDRA[0])]:.3f} pooled "
                     f"and {span(ratio, '')} per repetition")
        sentences[int(cpus)] = text + "."
    return ("Post-hoc descriptive addendum (three technical repetitions on one host per budget; no interval). " + budgets(sentences)) if sentences else ""


def readings(directory):
    """One or two sentences per study figure at 1 CPU, computed from the plotted rows."""
    quality = {(r["variant"], r["study"], r["cohort"], r["metric"]): r for r in read(directory, "quality") if r["cpus"] == "1"}
    performance = {r["variant"]: r for r in read(directory, "performance") if r["study"] == "main" and r["cpus"] == "1"}
    repetitions = [r for r in read(directory, "performance-repetitions") if r["study"] == "main" and r["cpus"] == "1"]
    strata = [r for r in read(directory, "summaries") if (r["study"], r["cohort"], r["cpus"]) == ("main", "primary", "1") and r["axis"] in ("appearance", "script", "domain")]
    paired = [r for r in read(directory, "paired") if (r["study"], r["cohort"], r["metric"], r["left"], r["cpus"]) == ("main", "primary", "cer", VARIANTS[0], "1")]
    def lowest(study, cohort, metric):
        values = {v: number(quality[(v, study, cohort, metric)]["value"]) for v in VARIANTS if (v, study, cohort, metric) in quality}
        values = {v: x for v, x in values.items() if x is not None}
        return " and ".join(NAMES[v] for v in values if values[v] == min(values.values())) if values else None
    def size(study, cohort):
        return next((int(r["selected"]) for (_, s, c, _), r in quality.items() if (s, c) == (study, cohort)), 0)
    result = {}
    if lowest("main", "primary", "Strict CER (%)"):
        same = all(quality[(INDRA[0], "main", "primary", m)]["value"] == quality[(INDRA[1], "main", "primary", m)]["value"]
                   for m in ("Strict CER (%)", "Collapsed CER (%)") if (INDRA[0], "main", "primary", m) in quality and (INDRA[1], "main", "primary", m) in quality)
        result["main-primary-quality"] = (f"At 1 CPU the lowest strict CER belongs to {lowest('main', 'primary', 'Strict CER (%)')} and the lowest collapsed "
                                          f"CER to {lowest('main', 'primary', 'Collapsed CER (%)')}" + ("; the two Indra marks coincide because their "
                                          "first-attempt outputs score identically." if same else "."))
    for stem, study, cohort, label in (("main-challenge-quality", "main", "challenge", "challenge cases"), ("laboratory-quality", "laboratory", "primary", "laboratory crops")):
        if lowest(study, cohort, "Strict CER (%)"):
            result[stem] = (f"On the {size(study, cohort)} {label} at 1 CPU the lowest strict CER belongs to {lowest(study, cohort, 'Strict CER (%)')}; "
                            "they are reported apart from the primary pages and carry no pooled claim.")
    with_interval = [r for r in paired if r["low"] != ""]
    if with_interval:
        crossing = [NAMES[r["variant"]] for r in with_interval if number(r["low"]) <= 0 <= number(r["high"])]
        result["main-primary-paired"] = ("Against fast PSM3 at 1 CPU, every drawn interval crosses zero." if len(crossing) == len(with_interval) else
                                         "Against fast PSM3 at 1 CPU, the intervals of " + (", ".join(crossing) or "no arm") + " cross zero.")
    if any(float(r["throughput"]) > 0 for r in performance.values()):
        fastest = max(performance, key=lambda v: float(performance[v]["throughput"]))
        spread = {v: [float(r["throughput"]) for r in repetitions if r["variant"] == v] for v in performance if float(performance[v]["throughput"]) > 0}
        widest = max(spread, key=lambda v: (max(spread[v]) - min(spread[v])) / float(performance[v]["throughput"]))
        result["main-workload-performance"] = (f"{NAMES[fastest]} has the highest throughput at 1 CPU ({float(performance[fastest]['throughput']):.3f} "
                                               f"responses/s, median {float(performance[fastest]['p50_ms']):.0f} ms); the widest per-repetition throughput "
                                               f"whisker is {NAMES[widest]}'s ({span(spread[widest], '')} responses/s).")
    gaps = {}
    for r in strata:
        if r["variant"] == INDRA[-1] and r["micro_rate"] != "" and r["metric"] in ("cer", "cer_whitespace_collapsed"):
            gaps.setdefault((r["axis"], r["level"], r["selected_cases"]), {})[r["metric"]] = number(r["micro_rate"])
    gaps = {k: v for k, v in gaps.items() if len(v) == 2}
    if gaps:
        axis, level, cases = max(gaps, key=lambda k: gaps[k]["cer"] - gaps[k]["cer_whitespace_collapsed"])
        result["main-primary-strata"] = (f"The widest strict-to-collapsed gap for Rust Indra is {axis} {level} ({plural(int(cases), 'case')}; "
                                         f"{percent(gaps[(axis, level, cases)]['cer'])} strict, {percent(gaps[(axis, level, cases)]['cer_whitespace_collapsed'])} "
                                         "collapsed): there most strict errors are whitespace.")
    return result


def discussion(directory, hosts):
    """Computed directions for the Discussion at 1 CPU; the report adds the fixed limitations."""
    quality = {(r["variant"], r["metric"]): number(r["value"]) for r in read(directory, "quality") if (r["study"], r["cohort"], r["cpus"]) == ("main", "primary", "1")}
    if not quality:
        return []
    paragraphs = []
    lower = all(quality.get((i, m)) is not None and all(quality.get((t, m)) is None or quality[(i, m)] < quality[(t, m)] for t in TESSERACT)
                for i in INDRA for m in ("Strict CER (%)", "Collapsed CER (%)"))
    rows = [r for r in read(directory, "paired") if (r["study"], r["cohort"], r["metric"], r["cpus"]) == ("main", "primary", "cer", "1")
            and r["left"] in TESSERACT and r["variant"] in INDRA and r["low"] != ""]
    def intervals(arm):
        selected = {r["left"]: r for r in rows if r["variant"] == arm}
        crossing = [NAMES[v] for v in TESSERACT if v in selected and number(selected[v]["low"]) <= 0 <= number(selected[v]["high"])]
        excluding = [NAMES[v] for v in TESSERACT if v in selected and NAMES[v] not in crossing]
        return (f"include zero against {', '.join(crossing)}, so the data do not show a difference from {'that setting' if len(crossing) == 1 else 'those settings'}"
                if crossing else "include zero against no setting") + (f", and exclude zero against {', '.join(excluding)} without a multiplicity adjustment"
                                                                        if excluding else "")
    readings = {arm: intervals(arm) for arm in INDRA}
    paired = (f"The paired intervals of either Indra arm {readings[INDRA[0]]}." if len(set(readings.values())) == 1 else
              " ".join(f"The paired intervals of {NAMES[arm]} {text}." for arm, text in readings.items()))
    paragraphs.append(("Both Indra arms have a lower strict and collapsed CER than every Tesseract setting on the main primary pages. " if lower else
                       "The Indra arms do not have a lower strict and collapsed CER than every Tesseract setting on the main primary pages. ") + paired)
    worse = [r for r in strata_versus_tesseract(directory, worse=True) if r["cpus"] == 1]
    better = [r for r in strata_versus_tesseract(directory, worse=False) if r["cpus"] == 1]
    describe = lambda selected: "; ".join(f"{r['axis']} {r['level']} ({plural(r['cases'], 'page')}, {plural(r['clusters'], 'cluster')})" for r in selected)
    small = sum(r["clusters"] <= 2 for r in worse + better)
    paragraphs.append("The aggregate hides strata in both directions. Indra is worse than the best Tesseract setting on "
                      + (describe(worse) or "no stratum") + ", and better than every Tesseract setting on " + (describe(better) or "no stratum")
                      + f". {small} of these {len(worse) + len(better)} strata come from one or two document clusters, so they describe these documents "
                      "rather than a script, language or domain in general.")
    legal_rows, _, _ = legal(directory)
    legal_rows = [r for r in legal_rows if r["cpus"] == 1]
    if legal_rows:
        arms = {r["arm"]: r for r in legal_rows}
        value = lambda metric, variants: ", ".join(f"{arms[NAMES[v]][metric]} for {NAMES[v]}" for v in variants if NAMES[v] in arms)
        paragraphs.append("On the modern legal pages strict CER mostly counts reference layout padding (strict "
                          f"{value('strict CER', INDRA)} against {value('strict CER', TESSERACT)}); without whitespace the arms read "
                          f"{value('collapsed CER', INDRA)} against {value('collapsed CER', TESSERACT)}.")
    laboratory_rows = {(r["variant"], r["metric"]): number(r["value"]) for r in read(directory, "quality") if (r["study"], r["cohort"], r["cpus"]) == ("laboratory", "primary", "1")}
    strict = {v: laboratory_rows.get((v, "Strict CER (%)")) for v in VARIANTS}
    if all(strict.get(v) is not None for v in INDRA) and any(strict.get(v) is not None for v in TESSERACT):
        higher = min(strict[v] for v in INDRA) > max(x for v, x in strict.items() if v in TESSERACT and x is not None)
        paragraphs.append(f"On the two laboratory crops Indra's strict CER is {'higher than every' if higher else 'not higher than every'} Tesseract "
                          "setting's; with two crops this is a description of those crops, not of laboratory documents.")
    performance = {r["variant"]: float(r["throughput"]) for r in read(directory, "performance") if r["study"] == "main" and r["cpus"] == "1"}
    if performance and any(performance.values()):
        fastest = max(performance, key=performance.get)
        same = len({(h["host"], h["kernel"], h["cpu_model"]) for h in hosts}) == 1
        differences = [name for name, key in (("CPU models", "cpu_model"), ("AVX-512F support", "avx512f")) if len({h[key] for h in hosts}) > 1]
        paragraphs.append(f"{NAMES[fastest]} answers the most workload responses per second at 1 CPU. "
                          + ("Both budgets ran on one host, so the 1- and 2-CPU cells compare budgets on the same hardware." if same else
                             "The two budgets ran on different hosts" + (f" with different {' and '.join(differences)}" if differences else "")
                             + ", so no 1- versus 2-CPU comparison in this study isolates the CPU budget."))
    return paragraphs
