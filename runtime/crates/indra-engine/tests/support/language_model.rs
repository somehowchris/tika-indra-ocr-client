//! A synthetic order-3 language model and recogniser outputs for gates and benches.
//! The archive mirrors lm.npz's layout; values are deterministic, not trained.
#![allow(dead_code)]
use std::{alloc::GlobalAlloc, io::Write, path::PathBuf};

use indra_engine::gates::{LanguageModel, TOP_K};

/// A right-to-left line in visual order (Hebrew with an embedded Latin run).
pub static RTL_LINE: &str = "٣٢١ )םולש( ABC םולש ,םולע lorem ipsum ,ןולח";
/// A left-to-right line with two right-to-left runs.
pub static MIXED_LINE: &str = "see םולש and ملاع here, then (a) again ב-א end";
/// Hangul jamo that compose, Latin with a combining acute, and a base letter
/// carrying seventeen combining marks: the normaliser's inline buffer limit.
pub static MARKED_LINE: &str = "\u{1112}\u{1161}\u{11ab} e\u{301} a\u{301}\u{302}\u{303}\u{304}\u{305}\u{306}\u{307}\u{308}\u{309}\u{30a}\u{30b}\u{30c}\u{30d}\u{30e}\u{30f}\u{310}\u{311}";
const BITS: u32 = 11;
const VOCAB: u64 = 1700;

fn npy(descr: &str, shape: &str, data: &[u8]) -> Vec<u8> {
    let mut header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut out = b"\x93NUMPY\x01\x00".to_vec();
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    out
}

fn keys(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Deterministic sorted keys: every token as a unigram, a sparse bigram and
/// trigram table with negative log-probabilities, and matching backoffs.
pub fn synthetic_model<A: GlobalAlloc>(_: &A) -> LanguageModel {
    let path: PathBuf =
        std::env::temp_dir().join(format!("indra-gate-lm-{}.npz", std::process::id()));
    let file = std::fs::File::create(&path).expect("archive");
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut members: Vec<(String, Vec<u8>)> = vec![
        ("order.npy".into(), npy("<i8", "()", &3_i64.to_le_bytes())),
        (
            "vocab_size.npy".into(),
            npy("<i8", "()", &(VOCAB as i64).to_le_bytes()),
        ),
        (
            "bits.npy".into(),
            npy("<i8", "()", &i64::from(BITS).to_le_bytes()),
        ),
    ];
    let unigrams: Vec<u64> = (1..=VOCAB).collect();
    let unigram_values: Vec<f32> = unigrams
        .iter()
        .map(|t| -(1.0 + (*t % 97) as f32 / 10.0))
        .collect();
    let mut bigrams = Vec::new();
    let mut bigram_values = Vec::new();
    let mut histories1 = Vec::new();
    let mut backoffs1 = Vec::new();
    for history in (0..VOCAB).step_by(3) {
        histories1.push(history);
        backoffs1.push(-0.5 - (history % 7) as f32 / 10.0);
        for token in (1..=VOCAB).step_by(11) {
            bigrams.push((history << BITS) | token);
            bigram_values.push(-(0.1 + ((history + token) % 13) as f32 / 10.0));
        }
    }
    let mut trigrams = Vec::new();
    let mut trigram_values = Vec::new();
    let mut histories2 = Vec::new();
    let mut backoffs2 = Vec::new();
    for a in (0..VOCAB).step_by(17) {
        for b in (1..=VOCAB).step_by(23) {
            let history = (a << BITS) | b;
            histories2.push(history);
            backoffs2.push(-0.25 - ((a + b) % 5) as f32 / 10.0);
            for token in (1..=VOCAB).step_by(97) {
                trigrams.push((history << BITS) | token);
                trigram_values.push(-(0.05 + ((a + b + token) % 11) as f32 / 10.0));
            }
        }
    }
    members.push((
        "k0.npy".into(),
        npy("<u8", &format!("({},)", unigrams.len()), &keys(&unigrams)),
    ));
    members.push((
        "v0.npy".into(),
        npy(
            "<f4",
            &format!("({},)", unigrams.len()),
            &floats(&unigram_values),
        ),
    ));
    members.push(("hk0.npy".into(), npy("<u8", "(1,)", &keys(&[0]))));
    members.push(("hb0.npy".into(), npy("<f4", "(1,)", &floats(&[-0.75]))));
    members.push((
        "k1.npy".into(),
        npy("<u8", &format!("({},)", bigrams.len()), &keys(&bigrams)),
    ));
    members.push((
        "v1.npy".into(),
        npy(
            "<f4",
            &format!("({},)", bigrams.len()),
            &floats(&bigram_values),
        ),
    ));
    members.push((
        "hk1.npy".into(),
        npy(
            "<u8",
            &format!("({},)", histories1.len()),
            &keys(&histories1),
        ),
    ));
    members.push((
        "hb1.npy".into(),
        npy(
            "<f4",
            &format!("({},)", histories1.len()),
            &floats(&backoffs1),
        ),
    ));
    members.push((
        "k2.npy".into(),
        npy("<u8", &format!("({},)", trigrams.len()), &keys(&trigrams)),
    ));
    members.push((
        "v2.npy".into(),
        npy(
            "<f4",
            &format!("({},)", trigrams.len()),
            &floats(&trigram_values),
        ),
    ));
    members.push((
        "hk2.npy".into(),
        npy(
            "<u8",
            &format!("({},)", histories2.len()),
            &keys(&histories2),
        ),
    ));
    members.push((
        "hb2.npy".into(),
        npy(
            "<f4",
            &format!("({},)", histories2.len()),
            &floats(&backoffs2),
        ),
    ));
    for (name, bytes) in &members {
        writer.start_file(name.as_str(), options).expect("member");
        writer.write_all(bytes).expect("member bytes");
    }
    writer.finish().expect("archive end");
    let model = LanguageModel::load(&path).expect("synthetic model");
    std::fs::remove_file(path).expect("cleanup");
    model
}

/// Recogniser outputs of one line: `steps` timesteps, each with a top-8 list
/// that mixes near-ties, confident classes and occasional confident blanks.
pub struct Line {
    pub blank: Vec<f32>,
    pub topv: Vec<f32>,
    pub topi: Vec<i64>,
}

pub fn synthetic_line(steps: usize) -> Line {
    let mut blank = Vec::with_capacity(steps);
    let mut topv = Vec::with_capacity(steps * TOP_K);
    let mut topi = Vec::with_capacity(steps * TOP_K);
    let mut state = 0x9E37_79B9_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for t in 0..steps {
        let confident_blank = t % 5 == 4;
        let blank_probability = if confident_blank {
            0.9995
        } else {
            0.05 + (next() % 30) as f64 / 100.0
        };
        blank.push((blank_probability as f32).ln());
        let mut remaining = 1.0 - blank_probability;
        for k in 0..TOP_K {
            let class = if k == 0 && confident_blank {
                0
            } else {
                1 + i64::from(next() % (VOCAB as u32 - 1))
            };
            let share = if k + 1 == TOP_K {
                remaining
            } else {
                remaining * (0.3 + (next() % 50) as f64 / 100.0)
            };
            remaining -= share;
            topv.push((share.max(1e-6) as f32).ln());
            topi.push(class);
        }
    }
    Line { blank, topv, topi }
}
