//! Insert contracts and erasable proof guidance; restore every core source byte.
use clap::Parser;
use miette::Diagnostic;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use thiserror::Error;

// Import Creusot's contract-aware derive explicitly. Rust's built-in
// `PartialEq` derive produces discriminant MIR that has no logical contract.
static PRELUDE: &str = "use creusot_std::prelude::{bitwise_proof, ensures, logic, proof_assert, DeepModel, Invariant, PartialEq};\n\n";
static CONTRACTS: [(&str, &str); 7] = [
    (
        "    pub const fn new(max_count: u16, max_work: u32) -> Self {",
        "    #[ensures(result.count == 0u16 && result.width == 0u32)]\n    #[ensures(result.max_count == max_count && result.max_work == max_work)]\n",
    ),
    (
        "    pub const fn count(&self) -> u16 {",
        "    #[ensures(result == self.count)]\n",
    ),
    (
        "    pub const fn width(&self) -> u32 {",
        "    #[ensures(result == self.width)]\n",
    ),
    (
        "    pub const fn work(&self) -> u32 {",
        "    #[ensures(result@ == self.width@ * self.count@)]\n    #[ensures(result@ <= self.max_work@)]\n",
    ),
    (
        "    pub const fn aligned_width(width: u32) -> Option<u32> {",
        r#"    #[bitwise_proof]
    #[ensures(match result {
        Some(aligned) => width@ > 0 && width@ <= u32::MAX@ - 7
            && aligned@ == ((width@ + 7) / 8) * 8
            && width@ <= aligned@ && aligned@ - width@ < 8,
        None => width == 0u32 || width@ > u32::MAX@ - 7,
    })]
"#,
    ),
    (
        "    pub fn try_add(&mut self, width: u32) -> Result<(), BatchError> {",
        r#"    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]
    #[ensures(match result {
        Ok(()) => (^self).count@ == self.count@ + 1
            && (^self).width@ >= self.width@ && (^self).width@ >= width@,
        Err(_) => (^self).count == self.count && (^self).width == self.width,
    })]
"#,
    ),
    (
        "    pub fn clear(&mut self) {",
        "    #[ensures((^self).count == 0u16 && (^self).width == 0u32)]\n    #[ensures((^self).max_count == self.max_count && (^self).max_work == self.max_work)]\n",
    ),
];
static ALIGNMENT_FACT: &str = r#"        proof_assert! {
            forall<padded: u32> {
                mask_eight(padded);
                (padded & !7u32)@ == (padded@ / 8) * 8
                    && padded@ - 7 <= (padded & !7u32)@
                    && (padded & !7u32)@ <= padded@
            }
        };
"#;
static INVARIANT: &str = r#"// BEGIN proof-only lemmas and BatchPlan invariant. Erased from ordinary compilation.
// Prove the bounded arithmetic in integer mode, then import its contracts in
// bitvector mode. This avoids asking one solver goal to discover the complete
// bitvector-to-integer division identity at once.
#[logic]
#[creusot_std::prelude::requires(value@ <= u32::MAX@ / 8)]
#[ensures(result == value * 8u32)]
#[ensures(result@ == value@ * 8)]
fn multiply_by_eight(value: u32) -> u32 {
    value * 8u32
}

#[logic]
#[creusot_std::prelude::requires(left@ + right@ <= u32::MAX@)]
#[ensures(result == left + right)]
#[ensures(result@ == left@ + right@)]
fn sum_without_overflow(left: u32, right: u32) -> u32 {
    left + right
}

#[logic]
#[bitwise_proof]
#[ensures((value & !7u32)@ == (value@ / 8) * 8)]
#[ensures(value@ - 7 <= (value & !7u32)@ && (value & !7u32)@ <= value@)]
fn mask_eight(value: u32) {
    let quotient = value >> 3u32;
    let remainder = value & 7u32;
    proof_assert!(quotient@ <= u32::MAX@ / 8);
    proof_assert!(remainder@ <= 7);
    let rounded = multiply_by_eight(quotient);
    proof_assert!(rounded == value & !7u32);
    let recombined = sum_without_overflow(rounded, remainder);
    proof_assert!(recombined == value);
}

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
// END proof-only lemmas and BatchPlan invariant.

"#;

#[derive(Parser)]
#[command(about = "Synchronise Creusot contracts with the exact production BatchPlan source")]
struct Args {
    /// Fail if the checked-in projection differs; never rewrite it.
    #[arg(long)]
    check: bool,
    /// Optional JSON evidence destination; stdout always receives the record.
    #[arg(long)]
    evidence: Option<PathBuf>,
}

#[derive(Debug, Error, Diagnostic)]
enum Error {
    #[error("proof projection I/O failed")]
    Io(#[from] std::io::Error),
    #[error("proof evidence serialization failed")]
    Json(#[from] serde_json::Error),
    #[error("source boundary is missing or no longer unique: {0}")]
    Boundary(&'static str),
    #[error("checked-in projection differs; regenerate with the Rust projector")]
    Stale,
    #[error("erasure did not restore the exact production source")]
    Erasure,
}

struct Insertion {
    at: usize,
    text: &'static str,
}

fn position(source: &str, boundary: &'static str) -> Result<usize, Error> {
    let mut occurrences = source.match_indices(boundary);
    let (at, _) = occurrences.next().ok_or(Error::Boundary(boundary))?;
    if occurrences.next().is_some() {
        return Err(Error::Boundary(boundary));
    }
    Ok(at)
}

fn project(source: &str) -> Result<(String, String), Error> {
    let mut insertions: [Insertion; 10] = std::array::from_fn(|_| Insertion { at: 0, text: "" });
    insertions[0] = Insertion {
        at: position(source, "use thiserror::Error;")?,
        text: PRELUDE,
    };
    for (slot, (signature, contract)) in insertions[1..8].iter_mut().zip(&CONTRACTS) {
        *slot = Insertion {
            at: position(source, signature)?,
            text: contract,
        };
    }
    insertions[8] = Insertion {
        at: position(source, "#[cfg(test)]\n")?,
        text: INVARIANT,
    };
    insertions[9] = Insertion {
        at: position(source, "        match width.checked_add(7) {")?,
        text: ALIGNMENT_FACT,
    };
    insertions.sort_unstable_by_key(|insertion| insertion.at);
    let extra: usize = insertions
        .iter()
        .map(|insertion| insertion.text.len())
        .sum();
    let mut projected = String::with_capacity(source.len() + extra);
    let mut previous = 0;
    for insertion in &insertions {
        projected.push_str(&source[previous..insertion.at]);
        projected.push_str(insertion.text);
        previous = insertion.at;
    }
    projected.push_str(&source[previous..]);

    // Erase only the recorded inserted bytes. Every production byte, including
    // tests, comments and whitespace, must survive independently of the hash.
    let mut erased = String::with_capacity(source.len());
    let mut consumed = 0;
    let mut inserted = 0;
    for insertion in &insertions {
        let at = insertion.at + inserted;
        erased.push_str(&projected[consumed..at]);
        if !projected[at..].starts_with(insertion.text) {
            return Err(Error::Erasure);
        }
        inserted += insertion.text.len();
        consumed = at + insertion.text.len();
    }
    erased.push_str(&projected[consumed..]);
    if erased.as_bytes() != source.as_bytes() {
        return Err(Error::Erasure);
    }
    Ok((projected, erased))
}

#[derive(Serialize)]
struct Evidence {
    schema_version: u32,
    source: &'static str,
    source_sha256: String,
    projected: &'static str,
    projected_sha256: String,
    erased_sha256: String,
    reverse_projection_exact: bool,
    production_source_modified: bool,
    existing_function_body_bytes_modified: bool,
    proof_only_body_insertions: u32,
    existing_tests_preserved: bool,
    contracts_inserted: [&'static str; 7],
    trusted_assumptions_added: bool,
}

fn run(args: Args) -> Result<(), Error> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let source = fs::read_to_string(root.join("../crates/indra-core/src/batch.rs"))?;
    let (projected, erased) = project(&source)?;
    let output = root.join("batch-plan/src/lib.rs");
    if args.check {
        if fs::read(&output)? != projected.as_bytes() {
            return Err(Error::Stale);
        }
    } else {
        fs::write(output, &projected)?;
    }
    let evidence = Evidence {
        schema_version: 2,
        source: "runtime/crates/indra-core/src/batch.rs",
        source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        projected: "runtime/proofs/batch-plan/src/lib.rs",
        projected_sha256: format!("{:x}", Sha256::digest(projected.as_bytes())),
        erased_sha256: format!("{:x}", Sha256::digest(erased.as_bytes())),
        reverse_projection_exact: true,
        production_source_modified: false,
        existing_function_body_bytes_modified: false,
        proof_only_body_insertions: 1,
        existing_tests_preserved: true,
        contracts_inserted: std::array::from_fn(|index| CONTRACTS[index].0),
        trusted_assumptions_added: false,
    };
    let mut json = serde_json::to_string_pretty(&evidence)?;
    json.push('\n');
    if let Some(path) = args.evidence {
        fs::write(path, &json)?;
    }
    print!("{json}");
    Ok(())
}

fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}
