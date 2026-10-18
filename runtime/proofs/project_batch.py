#!/usr/bin/env python3
"""Add proof-only contracts to the current, unmodified core BatchPlan source.

The inverse transformation must recover the complete original source byte for
byte. This is a synchronized proof projection, not a separately maintained
algorithm. No source function body is edited by this controller.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SOURCE = ROOT.parent / "crates/indra-core/src/batch.rs"
OUTPUT = ROOT / "batch-plan/src/lib.rs"
PRELUDE = "use creusot_std::prelude::{bitwise_proof, ensures, logic, DeepModel, Invariant};\n\n"

CONTRACTS = {
    "    pub const fn new(max_count: u16, max_work: u32) -> Self {": """    #[ensures(result.count == 0u16 && result.width == 0u32)]
    #[ensures(result.max_count == max_count && result.max_work == max_work)]
""",
    "    pub const fn count(&self) -> u16 {": """    #[ensures(result == self.count)]
""",
    "    pub const fn width(&self) -> u32 {": """    #[ensures(result == self.width)]
""",
    "    pub const fn work(&self) -> u32 {": """    #[ensures(result@ == self.width@ * self.count@)]
    #[ensures(result@ <= self.max_work@)]
""",
    "    pub const fn aligned_width(width: u32) -> Option<u32> {": """    #[bitwise_proof]
    #[ensures(match result {
        Some(aligned) => width@ > 0 && width@ <= u32::MAX@ - 7
            && aligned@ == ((width@ + 7) / 8) * 8
            && width@ <= aligned@ && aligned@ - width@ < 8,
        None => width == 0u32 || width@ > u32::MAX@ - 7,
    })]
""",
    "    pub fn try_add(&mut self, width: u32) -> Result<(), BatchError> {": """    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]
    #[ensures(match result {
        Ok(()) => (^self).count@ == self.count@ + 1
            && (^self).width@ >= self.width@ && (^self).width@ >= width@,
        Err(_) => (^self).count == self.count && (^self).width == self.width,
    })]
""",
    "    pub fn clear(&mut self) {": """    #[ensures((^self).count == 0u16 && (^self).width == 0u32)]
    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]
""",
}

INVARIANT = """// BEGIN proof-only BatchPlan invariant. Erased from ordinary compilation.
impl DeepModel for BatchError {
    type DeepModelTy = Self;

    #[logic(open)]
    fn deep_model(self) -> Self {
        self
    }
}

impl Invariant for BatchPlan {
    #[logic]
    fn invariant(self) -> bool {
        pearlite! {
            self.count@ <= self.max_count@
            && self.width@ * self.count@ <= self.max_work@
            && self.width@ % 8 == 0
            && (self.count == 0u16) == (self.width == 0u32)
        }
    }
}
// END proof-only BatchPlan invariant.

"""


def replace_once(source: str, old: str, new: str) -> str:
    if source.count(old) != 1:
        raise RuntimeError(f"Source boundary is no longer unique: {old!r}")
    return source.replace(old, new, 1)


def main() -> None:
    original = SOURCE.read_text()
    projected = replace_once(original, "use thiserror::Error;", PRELUDE + "use thiserror::Error;")
    for signature, contract in CONTRACTS.items():
        projected = replace_once(projected, signature, contract + signature)
    projected = replace_once(projected, "#[cfg(test)]\n", INVARIANT + "#[cfg(test)]\n")
    erased = replace_once(projected, PRELUDE, "")
    for signature, contract in CONTRACTS.items():
        erased = replace_once(erased, contract + signature, signature)
    erased = replace_once(erased, INVARIANT, "")
    if erased.encode() != SOURCE.read_bytes():
        raise RuntimeError("The proof projection changed core source bytes")
    OUTPUT.write_text(projected)
    digest = lambda value: hashlib.sha256(value).hexdigest()
    record = {
        "schema_version": 1,
        "source": "runtime/crates/indra-core/src/batch.rs",
        "source_sha256": digest(SOURCE.read_bytes()),
        "projected": "runtime/proofs/batch-plan/src/lib.rs",
        "projected_sha256": digest(OUTPUT.read_bytes()),
        "erased_sha256": digest(erased.encode()),
        "reverse_projection_exact": True,
        "function_bodies_modified": False,
        "existing_tests_preserved": True,
        "contracts_inserted": list(CONTRACTS),
        "trusted_assumptions_added": False,
    }
    (ROOT / "evidence/projection.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
