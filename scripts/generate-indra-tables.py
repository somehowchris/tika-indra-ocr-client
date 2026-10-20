#!/usr/bin/env python3
"""Regenerate the pinned Indra model lock and the Rust tables derived from it.

Inputs are a verified local copy of the Hugging Face snapshot (every file is
hash-checked here again) and the Hub tree listing of that revision. Outputs:

  backend/models.lock.json                        authoritative lock
  runtime/crates/indra-models/src/manifest.rs      generated artifact table
  runtime/crates/indra-engine/src/vocabulary.rs    recogniser vocabularies
  runtime/crates/indra-engine/src/unicode_tables.rs  CPython Unicode class ranges

The Unicode ranges reproduce CPython's unicodedata (the reference arm's
interpreter) so the Rust port classifies characters exactly like Python 3.13.
Run with the same CPython minor version as backend/Dockerfile (3.13):

  python3.13 -I scripts/generate-indra-tables.py --snapshot DIR --tree tree.json \
      --revision bd02167f... --tag v1.1.0 --package 1.1.2 --code-revision SHA

This tool is build-time only; production serving never runs Python.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import unicodedata
from pathlib import Path

REPOSITORY = "datalab-to/indra-ocr"
BASE_FILES = [
    "LICENSE", "config.json", "det.onnx", "meta.json", "order.onnx", "orient.onnx",
    "rec.onnx", "rec_cjk.onnx", "rec_rtl.onnx", "rec_world.onnx", "script.onnx",
    "lm.npz", "lm_cjk.npz", "lm_world.npz",
]
MATH_FILES = ["mathcls.onnx", "math_enc.onnx", "math_dec_init.onnx", "math_dec_step.onnx", "math_meta.json"]


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def rust_string(token: str) -> str:
    out = []
    for c in token:
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif (unicodedata.category(c)[0] in "CZ" and c != " ") or ord(c) < 0x20:
            out.append(f"\\u{{{ord(c):x}}}")
        else:
            out.append(c)
    return '"' + "".join(out) + '"'


def ranges(predicate) -> list[tuple[int, int]]:
    out: list[tuple[int, int]] = []
    start = None
    for code in range(0x110000):
        hit = predicate(chr(code))
        if hit and start is None:
            start = code
        elif not hit and start is not None:
            out.append((start, code - 1))
            start = None
    if start is not None:
        out.append((start, 0x10FFFF))
    return out


def emit_ranges(name: str, doc: str, items: list[tuple[int, int]]) -> str:
    lines = [f"/// {doc}", f"pub(crate) static {name}: [(u32, u32); {len(items)}] = ["]
    lines += [f"    (0x{a:X}, 0x{b:X})," for a, b in items]
    lines.append("];")
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshot", type=Path, required=True, help="directory holding the verified model files")
    parser.add_argument("--tree", type=Path, required=True, help="Hub tree listing JSON of the revision")
    parser.add_argument("--revision", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--package", required=True)
    parser.add_argument("--code-revision", required=True)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    if sys.version_info[:2] != (3, 13):
        raise SystemExit("run with CPython 3.13: the reference arm's Unicode database is 15.1")
    tree = {entry["path"]: entry for entry in json.loads(args.tree.read_text())}

    files = []
    for name in BASE_FILES + MATH_FILES:
        path = args.snapshot / name
        entry = tree[name]
        digest = sha256(path)
        expected = (entry.get("lfs") or {}).get("oid")
        if expected is not None and digest != expected:
            raise SystemExit(f"{name}: local sha256 {digest} differs from the Hub LFS object {expected}")
        if expected is None:
            blob = hashlib.sha1(b"blob %d\0" % path.stat().st_size + path.read_bytes()).hexdigest()
            if blob != entry["oid"]:
                raise SystemExit(f"{name}: local git blob {blob} differs from the Hub tree {entry['oid']}")
        if path.stat().st_size != entry["size"]:
            raise SystemExit(f"{name}: size differs from the Hub tree")
        files.append({"name": name, "size": entry["size"], "sha256": digest,
                      "profile": "math" if name in MATH_FILES else "base"})
    files.sort(key=lambda item: item["name"])
    lock = {
        "repository": REPOSITORY, "revision": args.revision, "reviewed_tag": args.tag,
        "code_revision": args.code_revision, "package_version": args.package, "files": files,
    }
    (args.repo / "backend/models.lock.json").write_text(json.dumps(lock, indent=2) + "\n")

    manifest = ["// Generated from backend/models.lock.json; startup verifies every selected file.",
                "use super::ModelArtifact;",
                f"pub(super) static FILES: [ModelArtifact; {len(files)}] = ["]
    for item in files:
        raw = bytes.fromhex(item["sha256"])
        manifest += ["    ModelArtifact {", f'        name: "{item["name"]}",', f"        size: {item['size']},", "        sha256: ["]
        for offset in range(0, 32, 14):
            manifest.append("            " + " ".join(f"0x{b:02x}," for b in raw[offset:offset + 14]))
        manifest += ["        ],", f"        math: {'true' if item['profile'] == 'math' else 'false'},", "    },"]
    manifest.append("];")
    (args.repo / "runtime/crates/indra-models/src/manifest.rs").write_text("\n".join(manifest) + "\n")

    meta = json.loads((args.snapshot / "meta.json").read_text())
    math_meta = json.loads((args.snapshot / "math_meta.json").read_text())
    vocabularies = {
        "TEXT": json.loads(meta["vocab"])["chars"],
        "CJK": json.loads(meta["vocab_cjk"])["chars"],
        "RTL": json.loads(meta["vocab_rtl"])["chars"],
        "WORLD": json.loads(meta["vocab_world"])["chars"],
        "MATH": math_meta["tokens"],
    }
    if meta["script_groups"] != ["base", "cjk", "rtl", "world"]:
        raise SystemExit("script group order changed; update ScriptGroup")
    out = [f"// Vocabulary data from datalab-to/indra-ocr snapshot",
           f"// {args.revision}. Model license applies; see THIRD_PARTY_NOTICES.md.",
           "// Metadata hashes are verified by indra-models before these tables are used.",
           "// Regenerate with scripts/generate-indra-tables.py; do not edit by hand."]
    for name, chars in vocabularies.items():
        out.append(f"pub(super) static {name}: [&str; {len(chars)}] = [")
        out += [f"    {rust_string(token)}," for token in chars]
        out.append("];")
    out += ["", "// Unicode Letter category (L*) of each token's first character. Python str.isalpha",
            "// uses L*, whereas Rust char::is_alphabetic also includes some combining marks.",
            f"pub(super) static TEXT_STARTS_LETTER: [bool; {len(vocabularies['TEXT'])}] = ["]
    out += [f"    {'true' if unicodedata.category(token[0]).startswith('L') else 'false'}," for token in vocabularies["TEXT"]]
    out.append("];")
    (args.repo / "runtime/crates/indra-engine/src/vocabulary.rs").write_text("\n".join(out) + "\n")

    version = unicodedata.unidata_version
    tables = [
        "//! Character class ranges reproduced from CPython's `unicodedata`, so the",
        f"//! port classifies text exactly like the Python reference (Unicode {version},",
        f"//! CPython {sys.version.split()[0]}). Generated by scripts/generate-indra-tables.py.",
        "//! Each table is sorted and non-overlapping; `contains` is a binary search.",
        "",
        "pub(crate) fn contains(table: &[(u32, u32)], c: char) -> bool {",
        "    let code = u32::from(c);",
        "    let index = table.partition_point(|&(_, last)| last < code);",
        "    table.get(index).is_some_and(|&(first, _)| first <= code)",
        "}",
        "",
        emit_ranges("BIDI_RTL", "Bidi_Class R or AL (`rtl.is_rtl`).",
                    ranges(lambda c: unicodedata.bidirectional(c) in ("R", "AL"))),
        emit_ranges("BIDI_L", "Bidi_Class L (`rtl.rtl_base` counts these against RTL letters).",
                    ranges(lambda c: unicodedata.bidirectional(c) == "L")),
        emit_ranges("BIDI_LTR", "Bidi_Class L, EN or AN (`rtl._is_ltr`).",
                    ranges(lambda c: unicodedata.bidirectional(c) in ("L", "EN", "AN"))),
        emit_ranges("MARK", "General_Category Mn or Me (`rtl.clusters` attaches these to their base).",
                    ranges(lambda c: unicodedata.category(c) in ("Mn", "Me"))),
        emit_ranges("LETTER", "General_Category L* (Python `str.isalpha` for one character).",
                    ranges(lambda c: unicodedata.category(c).startswith("L"))),
        emit_ranges("EAST_ASIAN_WIDE", "East_Asian_Width W or F (`order_post.display_len` counts two).",
                    ranges(lambda c: unicodedata.east_asian_width(c) in ("W", "F"))),
        emit_ranges("UPPER", "Python `str.isupper` for one character: cased and uppercase.",
                    ranges(lambda c: c.isupper())),
    ]
    (args.repo / "runtime/crates/indra-engine/src/unicode_tables.rs").write_text("\n".join(tables) + "\n")
    print(json.dumps({"files": len(files), "bytes": sum(item["size"] for item in files),
                      "vocabularies": {k: len(v) for k, v in vocabularies.items()}, "unicode": version}))


if __name__ == "__main__":
    main()
