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

from study import ROOT, VARIANTS, decode, require

NAMES = dict(zip(VARIANTS, ("fast PSM3", "best PSM3", "best PSM6", "best PSM11", "Python Indra", "Rust Indra"), strict=True))
TESSERACT, INDRA = VARIANTS[:4], VARIANTS[4:]
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


def plural(count, word):
    return f"{count} {word}{'s' * (count != 1)}"


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

