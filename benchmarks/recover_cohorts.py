#!/usr/bin/env python3
"""Combine explicitly retained replacements for cohorts blocked before launch.

This does not retry requests or overwrite evidence. Only a cohort with no
observed application process and no successful request can be replaced.
"""

import argparse
import json
import shutil
import sys
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import urlsplit, urlunsplit

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from benchmarks.run import digest, read_json, save_json, summarize


def key(row):
    return row["engine"], row["repetition"]


def normalized_engine(engine):
    result = json.loads(json.dumps(engine))
    for name in ("url", "readiness_url"):
        if name in result:
            value = urlsplit(result[name])
            if value.hostname not in ("localhost", "127.0.0.1"):
                raise ValueError("port-only recovery requires a loopback HTTP endpoint")
            result[name] = urlunsplit((value.scheme, value.hostname, value.path, value.query, value.fragment))
    if "INDRA_PORT" in result.get("env", {}):
        result["env"]["INDRA_PORT"] = "<loopback-port>"
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--supplement", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reason", required=True)
    args = parser.parse_args()
    original, supplement = args.original.resolve(), args.supplement.resolve()
    environments = [read_json(path / "environment.json") for path in (original, supplement)]
    for field in ("manifest_sha256", "cases"):
        if environments[0][field] != environments[1][field]:
            raise ValueError(f"recovery changed {field}")
    for field in ("cpus", "driver_cpu", "concurrency", "seed", "whitespace", "warmup", "sample_interval"):
        if environments[0]["arguments"][field] != environments[1]["arguments"][field]:
            raise ValueError(f"recovery changed benchmark argument {field}")
    base_engines = {e["name"]: e for e in environments[0]["plan"]["engines"]}
    for engine in environments[1]["plan"]["engines"]:
        if normalized_engine(engine) != normalized_engine(base_engines[engine["name"]]):
            raise ValueError("only loopback port changes are permitted in a recovery plan")
    base_cohorts, new_cohorts = [read_json(path / "cohorts.json") for path in (original, supplement)]
    base_rows, new_rows = [[json.loads(line) for line in (path / "requests.jsonl").read_text().splitlines()]
                          for path in (original, supplement)]
    replacing = {key(c): c for c in new_cohorts}
    originals = {key(c): c for c in base_cohorts}
    for identity, replacement in replacing.items():
        previous = originals[identity]
        prior_rows = [r for r in base_rows if key(r) == identity]
        replacement_rows = [r for r in new_rows if key(r) == identity]
        if (not previous.get("startup_or_harness_error") or previous["resources"]["observed_process_count"]
                or any(r["status"] == "ok" for r in prior_rows)):
            raise ValueError("refusing to replace a cohort that launched application work")
        if (replacement.get("startup_or_harness_error") or replacement.get("validation_error")
                or replacement["resources"]["affinity_escapes"] or any(r["status"] != "ok" for r in replacement_rows)):
            raise ValueError("supplemental cohort did not pass")
        if previous["selected_cases"] != replacement["selected_cases"]:
            raise ValueError("supplement changed case selection/order")
        measured = [r["case_id"] for r in replacement_rows if r["phase"] == "measured"]
        if len(measured) != len(set(measured)) or set(measured) != set(previous["selected_cases"]):
            raise ValueError("supplement did not complete every selected case exactly once")
    if not replacing:
        raise ValueError("supplement contains no cohorts")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    for label, path in (("original", original), ("supplement", supplement)):
        shutil.copytree(path, output / "attempts" / label)
    cohorts = [replacing.get(key(c), c) for c in base_cohorts]
    rows = [r for r in base_rows if key(r) not in replacing] + new_rows
    for cohort in cohorts:
        directory = f"{cohort['engine']}-{cohort['repetition']}"
        source = supplement if key(cohort) in replacing else original
        shutil.copytree(source / directory, output / directory)
    provenance = {"reason": args.reason, "original": str(original), "supplement": str(supplement),
                  "original_summary_sha256": digest(original / "summary.json"),
                  "supplement_summary_sha256": digest(supplement / "summary.json"),
                  "replaced_cohorts": [{"engine": k[0], "repetition": k[1],
                                       "original_error": originals[k]["startup_or_harness_error"]}
                                      for k in replacing],
                  "original_blocked_request_count": sum(key(r) in replacing and r["phase"] == "measured" for r in base_rows),
                  "all_original_observations_retained": "attempts/original",
                  "all_supplemental_observations_retained": "attempts/supplement"}
    save_json(output / "recovery.json", provenance)
    metadata = dict(environments[0])
    metadata["recovery"] = provenance
    save_json(output / "environment.json", metadata)
    save_json(output / "cohorts.json", cohorts)
    (output / "requests.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    summary = summarize(rows, cohorts, list(base_engines), SimpleNamespace(**metadata["arguments"]))
    summary["recovery"] = provenance
    save_json(output / "summary.json", summary)
    print(json.dumps({"output": str(output), "recovered_cohorts": len(replacing),
                      "engines": {name: {k: v[k] for k in ("successes", "failures", "pages_per_second")}
                                  for name, v in summary["engines"].items()}}, indent=2))


if __name__ == "__main__":
    main()
