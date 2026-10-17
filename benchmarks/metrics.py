"""Explicit text metrics and paired, document-cluster bootstrap intervals."""

from __future__ import annotations

import math
import random
import unicodedata
from collections import defaultdict
from collections.abc import Sequence


def normalize(text: str, whitespace: str = "collapse") -> str:
    """NFC codepoints, case/punctuation preserved; never rewrite spelling.

    collapse: Unicode whitespace runs become one ASCII space; edges stripped.
    preserve: CRLF/CR become LF; all other whitespace, including edges, retained.
    """
    text = unicodedata.normalize("NFC", text.replace("\r\n", "\n").replace("\r", "\n"))
    if whitespace == "collapse":
        return " ".join(text.split())
    if whitespace != "preserve":
        raise ValueError("whitespace must be 'collapse' or 'preserve'")
    return text


def distance(reference: Sequence, prediction: Sequence) -> int:
    """Exact Levenshtein distance with bit-parallel integer states.

    Memory depends on the shorter length and its number of distinct tokens.
    Sequence elements must be hashable (strings/codepoints or word strings).
    """
    if len(reference) > len(prediction):
        reference, prediction = prediction, reference
    length = len(reference)
    if not length:
        return len(prediction)
    masks = {}
    for i, item in enumerate(reference):
        masks[item] = masks.get(item, 0) | (1 << i)
    positive = (1 << length) - 1
    negative = 0
    score = length
    high_bit = 1 << (length - 1)
    for item in prediction:
        equal = masks.get(item, 0)
        vertical = equal | negative
        horizontal = (((equal & positive) + positive) ^ positive) | equal
        plus = negative | ~(horizontal | positive)
        minus = positive & horizontal
        score += bool(plus & high_bit) - bool(minus & high_bit)
        plus = (plus << 1) | 1
        minus <<= 1
        positive = minus | ~(vertical | plus)
        negative = plus & vertical
    return score


def score(reference: str, prediction: str, whitespace: str = "collapse") -> dict:
    reference = normalize(reference, whitespace)
    prediction = normalize(prediction, whitespace)
    ref_words, pred_words = reference.split(), prediction.split()
    char_errors = distance(reference, prediction)
    word_errors = distance(ref_words, pred_words)
    return {
        "reference_codepoints": len(reference),
        "reference_words": len(ref_words),
        "character_errors": char_errors,
        "word_errors": word_errors,
        "cer": char_errors / len(reference) if reference else None,
        "wer": word_errors / len(ref_words) if ref_words else None,
        "blank_reference": not reference,
        "blank_false_positive": not reference and bool(prediction),
        "predicted_codepoints": len(prediction),
    }


def percentile(values: Sequence[float], quantile: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    location = (len(ordered) - 1) * quantile
    low, high = math.floor(location), math.ceil(location)
    return ordered[low] + (ordered[high] - ordered[low]) * (location - low)


def aggregate(scores: Sequence[dict]) -> dict:
    """Micro averages use total edits / total GT units; no page-average bias."""
    chars = sum(row["reference_codepoints"] for row in scores)
    words = sum(row["reference_words"] for row in scores)
    char_errors = sum(row["character_errors"] for row in scores)
    word_errors = sum(row["word_errors"] for row in scores)
    blanks = sum(row["blank_reference"] for row in scores)
    false_blanks = sum(row["blank_false_positive"] for row in scores)
    return {
        "unique_cases": len(scores),
        "reference_codepoints": chars,
        "reference_words": words,
        "character_errors": char_errors,
        "word_errors": word_errors,
        "micro_cer": char_errors / chars if chars else None,
        "micro_wer": word_errors / words if words else None,
        "blank_cases": blanks,
        "blank_false_positives": false_blanks,
    }


def paired_cer_interval(
    baseline: Sequence[dict], candidate: Sequence[dict], seed: int, resamples: int = 10000
) -> dict:
    """Percentile bootstrap of candidate minus baseline micro CER.

    Resample document clusters (all renditions/pages travel together). The caller
    must pass one observation per case, never repetitions of the same output.
    This interval concerns this corpus' document sampling, not all OCR domains.
    """
    left = {row["case_id"]: row for row in baseline}
    right = {row["case_id"]: row for row in candidate}
    common = sorted(left.keys() & right.keys())
    clusters = defaultdict(lambda: [0, 0, 0])
    for case_id in common:
        a, b = left[case_id], right[case_id]
        if a["reference_codepoints"] != b["reference_codepoints"]:
            raise ValueError("paired metrics have different reference lengths")
        cluster = clusters[a["document_id"]]
        cluster[0] += a["reference_codepoints"]
        cluster[1] += a["character_errors"]
        cluster[2] += b["character_errors"]
    rows = list(clusters.values())
    denominator = sum(row[0] for row in rows)
    result = {
        "paired_cases": len(common), "document_clusters": len(rows),
        "difference": sum(row[2] - row[1] for row in rows) / denominator if denominator else None,
        "method": "paired document-cluster percentile bootstrap, 95%",
        "seed": seed, "resamples": resamples, "low": None, "high": None,
    }
    if len(rows) < 2 or not denominator:
        result["warning"] = "insufficient independent nonempty document clusters"
        return result
    rng = random.Random(seed)
    samples = []
    for _ in range(resamples):
        selected = [rows[rng.randrange(len(rows))] for _ in rows]
        count = sum(row[0] for row in selected)
        if count:
            samples.append(sum(row[2] - row[1] for row in selected) / count)
    result.update(low=percentile(samples, 0.025), high=percentile(samples, 0.975))
    result["valid_resamples"] = len(samples)
    if len(rows) < 30:
        result["warning"] = "small document-cluster count; interval is exploratory"
    return result
