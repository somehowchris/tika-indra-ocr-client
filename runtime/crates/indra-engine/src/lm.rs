//! Token n-gram language model and CTC prefix beam search for unsure lines.
//!
//! Port of `indra_ocr/lm.py` at upstream commit
//! `0e96b93c6810f17cf8352acbb4487cc9e45b66d5` (Apache-2.0). The backoff tables
//! are read once at startup from the pinned `lm*.npz` archives into per-order
//! sorted key and value slices. `logp` follows the vectorised `logp_keys`
//! rule in `f64`; the beam search keeps every prefix, step and packed history
//! in fixed worker scratch and performs no heap allocation per line.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use crate::{EngineError, Result, recognition::MAX_TOKENS};

/// `lm.npz` is a 5-gram model; the CJK and world models are shorter.
pub const MAX_ORDER: usize = 5;
const DEFAULT_BITS: u32 = 11;
const TAG_SHIFT: u32 = 61;
pub const BEAM: usize = 8;
/// Classes the recogniser exports per step (`topv`/`topi`), the blank included.
pub const TOP_K: usize = 8;
const BLANK_SKIP: f64 = 0.999;
const CAND_FLOOR: f64 = 3e-3;
/// Prefixes under consideration after one step: every beam's own entry and
/// up to `TOP_K` extensions of each.
const CANDIDATES: usize = BEAM * (TOP_K + 1);
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Per-group weights: `alpha` scales the model, `beta` rewards each token,
/// lines at or above `gate` keep their greedy reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub alpha: f64,
    pub beta: f64,
    pub gate: f64,
}

impl Params {
    /// `pipeline.py` defaults for the base recogniser.
    pub const BASE: Self = Self {
        alpha: 0.5,
        beta: 1.5,
        gate: 0.9,
    };
    /// `meta.json` `lm_params` for the CJK and world recognisers.
    pub const GROUP: Self = Self {
        alpha: 0.2,
        beta: 1.5,
        gate: 0.95,
    };
}

pub struct Model {
    order: usize,
    bits: u32,
    vocab_size: u64,
    keys: [Box<[u64]>; MAX_ORDER],
    values: [Box<[f32]>; MAX_ORDER],
    history_keys: [Box<[u64]>; MAX_ORDER],
    backoffs: [Box<[f32]>; MAX_ORDER],
    max_logp: f64,
}

impl Model {
    /// Read one `.npz` archive. Startup only: the tables are the only heap
    /// allocations and they live as long as the engine.
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut archive = zip::ZipArchive::new(BufReader::new(file))
            .map_err(|_| EngineError::Shape("language model archive"))?;
        let order = scalar(&mut archive, "order.npy")?;
        let vocab_size = scalar(&mut archive, "vocab_size.npy")?;
        let bits = if archive.index_for_name("bits.npy").is_some() {
            u32::try_from(scalar(&mut archive, "bits.npy")?)
                .map_err(|_| EngineError::Shape("language model key bits"))?
        } else {
            DEFAULT_BITS
        };
        let order = usize::try_from(order)
            .ok()
            .filter(|order| (1..=MAX_ORDER).contains(order))
            .ok_or(EngineError::Shape("language model order"))?;
        if bits == 0 || order as u32 * bits > TAG_SHIFT || vocab_size <= 0 {
            return Err(EngineError::Shape("language model key layout"));
        }
        let mut keys: [Box<[u64]>; MAX_ORDER] = Default::default();
        let mut values: [Box<[f32]>; MAX_ORDER] = Default::default();
        let mut history_keys: [Box<[u64]>; MAX_ORDER] = Default::default();
        let mut backoffs: [Box<[f32]>; MAX_ORDER] = Default::default();
        for level in 0..order {
            keys[level] = sorted_keys(
                &mut archive,
                &format!("k{level}.npy"),
                (level as u32 + 1) * bits,
            )?;
            values[level] = floats(&mut archive, &format!("v{level}.npy"))?;
            history_keys[level] =
                sorted_keys(&mut archive, &format!("hk{level}.npy"), level as u32 * bits)?;
            backoffs[level] = floats(&mut archive, &format!("hb{level}.npy"))?;
            if keys[level].len() != values[level].len()
                || history_keys[level].len() != backoffs[level].len()
            {
                return Err(EngineError::Shape("language model table lengths"));
            }
        }
        let mut model = Self {
            order,
            bits,
            vocab_size: vocab_size as u64,
            keys,
            values,
            history_keys,
            backoffs,
            max_logp: 0.0,
        };
        model.max_logp = model.bound();
        Ok(model)
    }

    /// `max_logp`: the largest log-probability plus every positive backoff
    /// weight a back-off can add, an upper bound of `logp`.
    fn bound(&self) -> f64 {
        let mut top = -(self.vocab_size as f64).ln();
        for level in 0..self.order {
            for value in &self.values[level] {
                top = top.max(f64::from(*value));
            }
        }
        let mut backoff = 0.0_f64;
        for level in 0..self.order {
            if let Some(largest) = self.backoffs[level].iter().copied().reduce(f32::max) {
                backoff += f64::from(largest).max(0.0);
            }
        }
        top + backoff
    }

    pub const fn bits(&self) -> u32 {
        self.bits
    }

    pub const fn order(&self) -> usize {
        self.order
    }

    pub const fn max_logp(&self) -> f64 {
        self.max_logp
    }

    /// Whether `token + 1` fits the key layout of this model.
    pub const fn admits(&self, class: u64) -> bool {
        class + 1 < (1_u64 << self.bits)
    }

    /// `logp_keys` for one key: the last `order − 1` history tokens and then
    /// the token, packed `bits` each (`0` is the line start, `t + 1` token `t`).
    /// The value comes from the longest seen history that saw the token, plus
    /// the backoff weights of the longer seen histories; with no such history,
    /// all backoffs and a uniform floor.
    pub fn logp(&self, key: u64) -> f64 {
        let bits = self.bits;
        let token = key & ((1_u64 << bits) - 1);
        let mut history_seen = [false; MAX_ORDER];
        let mut backoff = [0.0_f64; MAX_ORDER];
        let mut pair_seen = [false; MAX_ORDER];
        let mut value = [0.0_f64; MAX_ORDER];
        for level in 0..self.order {
            let mask = if level == 0 {
                0
            } else {
                (1_u64 << (bits * level as u32)) - 1
            };
            let history = (key >> bits) & mask;
            if let Ok(index) = self.history_keys[level].binary_search(&history) {
                history_seen[level] = true;
                backoff[level] = f64::from(self.backoffs[level][index]);
            }
            if let Ok(index) = self.keys[level].binary_search(&((history << bits) | token)) {
                pair_seen[level] = true;
                value[level] = f64::from(self.values[level][index]);
            }
        }
        history_seen[0] = true;
        // The longest history length whose shorter histories were all seen.
        let longest = history_seen[1..self.order]
            .iter()
            .position(|seen| !*seen)
            .unwrap_or(self.order - 1);
        let mut chosen = None;
        for level in (0..=longest).rev() {
            if pair_seen[level] {
                chosen = Some(level);
                break;
            }
        }
        // np.cumsum of the reversed row: the sum runs from the longest history down.
        let mut suffix = [0.0_f64; MAX_ORDER + 1];
        let mut total = 0.0_f64;
        for level in (0..self.order).rev() {
            total += if level <= longest {
                backoff[level]
            } else {
                0.0
            };
            suffix[level] = total;
        }
        match chosen {
            Some(level) => suffix[level + 1] + value[level],
            None => suffix[0] - (self.vocab_size as f64).ln(),
        }
    }
}

/// One archive member, read in full into a startup-only boxed buffer.
fn entry(archive: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Result<Box<[u8]>> {
    let file = archive
        .by_name(name)
        .map_err(|_| EngineError::Shape("language model archive member"))?;
    let size = file.size();
    if size > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(EngineError::Limit("language model archive member"));
    }
    let mut bytes: Box<[u8]> = std::iter::repeat_n(0_u8, size as usize).collect();
    file.take(size).read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Parse a `.npy` v1/v2 header: descriptor, C order and element count.
fn npy<'a>(bytes: &'a [u8], descr: &str) -> Result<&'a [u8]> {
    let malformed = EngineError::Shape("language model array header");
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(malformed);
    }
    let (length, start) = match bytes[6] {
        1 => (usize::from(u16::from_le_bytes([bytes[8], bytes[9]])), 10),
        2 | 3 => (
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
            12,
        ),
        _ => return Err(malformed),
    };
    let header = bytes
        .get(start..start + length)
        .and_then(|header| std::str::from_utf8(header).ok())
        .ok_or(EngineError::Shape("language model array header"))?;
    let expected_descr = format!("'descr': '{descr}'");
    if !header.contains(&expected_descr) || !header.contains("'fortran_order': False") {
        return Err(EngineError::Shape("language model array layout"));
    }
    let shape = header
        .split("'shape': (")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .ok_or(EngineError::Shape("language model array shape"))?;
    let mut count = 1_usize;
    for dimension in shape.split(',') {
        let dimension = dimension.trim();
        if dimension.is_empty() {
            continue;
        }
        count = dimension
            .parse::<usize>()
            .ok()
            .and_then(|n| count.checked_mul(n))
            .ok_or(EngineError::Shape("language model array shape"))?;
    }
    let data = &bytes[start + length..];
    let width = match descr {
        "<i8" | "<u8" => 8,
        "<f4" => 4,
        _ => return Err(EngineError::Shape("language model array type")),
    };
    if data.len() != count * width {
        return Err(EngineError::Shape("language model array length"));
    }
    Ok(data)
}

fn scalar(archive: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Result<i64> {
    let bytes = entry(archive, name)?;
    let data = npy(&bytes, "<i8")?;
    let value: [u8; 8] = data
        .try_into()
        .map_err(|_| EngineError::Shape("language model scalar"))?;
    Ok(i64::from_le_bytes(value))
}

fn sorted_keys(
    archive: &mut zip::ZipArchive<BufReader<File>>,
    name: &str,
    key_bits: u32,
) -> Result<Box<[u64]>> {
    let bytes = entry(archive, name)?;
    let data = npy(&bytes, "<u8")?;
    let keys: Box<[u64]> = data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect();
    let limit = if key_bits >= 64 {
        u64::MAX
    } else {
        (1_u64 << key_bits) - 1
    };
    if keys.iter().any(|key| *key > limit) || keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(EngineError::Shape(
            "language model keys must be sorted and packed",
        ));
    }
    Ok(keys)
}

fn floats(archive: &mut zip::ZipArchive<BufReader<File>>, name: &str) -> Result<Box<[f32]>> {
    let bytes = entry(archive, name)?;
    let data = npy(&bytes, "<f4")?;
    let values: Box<[f32]> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    if !values.iter().all(|value| value.is_finite()) {
        return Err(EngineError::Shape("language model values must be finite"));
    }
    Ok(values)
}

/// One prefix under consideration: log-probabilities of ending in a blank and
/// in a non-blank, the model score, the token count and the packed history.
#[derive(Clone, Copy)]
struct Entry {
    blank: f64,
    non_blank: f64,
    score: f64,
    len: u16,
    history: u64,
    hash: u64,
}

impl Entry {
    const EMPTY: Self = Self {
        blank: 0.0,
        non_blank: f64::NEG_INFINITY,
        score: 0.0,
        len: 0,
        history: 0,
        hash: 0,
    };

    fn total(&self) -> f64 {
        lse(self.blank, self.non_blank) + self.score
    }
}

struct Pool<const N: usize> {
    entries: [Entry; N],
    tokens: [[u16; MAX_TOKENS]; N],
    steps: [[u16; MAX_TOKENS]; N],
    count: usize,
}

impl<const N: usize> Pool<N> {
    const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; N],
            tokens: [[0; MAX_TOKENS]; N],
            steps: [[0; MAX_TOKENS]; N],
            count: 0,
        }
    }
}

/// Fixed worker scratch for the beam search; allocated once at startup.
pub struct Scratch {
    beams: Pool<BEAM>,
    next: Pool<CANDIDATES>,
}

impl Scratch {
    pub fn new() -> Box<Self> {
        Box::new(Self {
            beams: Pool::new(),
            next: Pool::new(),
        })
    }
}

/// `_lse`: log(exp(a) + exp(b)) with the reference's special cases.
fn lse(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY {
        return b;
    }
    if b == f64::NEG_INFINITY {
        return a;
    }
    let m = if a >= b { a } else { b };
    m + ((a - m).exp() + (b - m).exp()).ln()
}

const fn extend_hash(hash: u64, class: u16) -> u64 {
    hash.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(class as u64 + 1)
}

/// `beam_search`: CTC prefix beam search over one line's steps.
///
/// `blank` holds `T` blank log-probabilities; `topv`/`topi` hold the `TOP_K`
/// best classes per step and their log-probabilities. A class under the
/// candidate floor at a step is not tried there; a step whose blank is above
/// the skip threshold only extends every beam by that blank. Writes the token
/// ids and the step at which each token was first emitted; returns the count.
#[allow(clippy::too_many_arguments)]
pub fn beam_search(
    blank: &[f32],
    topv: &[f32],
    topi: &[i64],
    model: &Model,
    params: Params,
    scratch: &mut Scratch,
    ids: &mut [i32],
    steps: &mut [u16],
) -> Result<usize> {
    let timesteps = blank.len();
    if timesteps > MAX_TOKENS
        || topv.len() != timesteps * TOP_K
        || topi.len() != timesteps * TOP_K
        || ids.len() < timesteps
        || steps.len() < timesteps
    {
        return Err(EngineError::Shape("beam search inputs"));
    }
    let bits = model.bits();
    let history_mask = if model.order() == 1 {
        0
    } else {
        (1_u64 << (bits * (model.order() as u32 - 1))) - 1
    };
    let log_skip = BLANK_SKIP.ln();
    let log_floor = CAND_FLOOR.ln();
    let reach = params.beta + params.alpha * model.max_logp();
    let beams = &mut scratch.beams;
    let next = &mut scratch.next;
    beams.count = 1;
    beams.entries[0] = Entry::EMPTY;
    for t in 0..timesteps {
        let b0 = f64::from(blank[t]);
        if b0 > log_skip {
            for entry in &mut beams.entries[..beams.count] {
                entry.blank = lse(entry.blank, entry.non_blank) + b0;
                entry.non_blank = f64::NEG_INFINITY;
            }
            continue;
        }
        let mut candidates = [(0_u16, 0.0_f64); TOP_K];
        let mut candidate_count = 0;
        for k in 0..TOP_K {
            let class = topi[t * TOP_K + k];
            let value = f64::from(topv[t * TOP_K + k]);
            if class == 0 || value < log_floor {
                continue;
            }
            let class = u64::try_from(class)
                .ok()
                .filter(|class| model.admits(*class))
                .and_then(|class| u16::try_from(class).ok())
                .ok_or(EngineError::Shape("beam search class id"))?;
            candidates[candidate_count] = (class, value);
            candidate_count += 1;
        }
        let candidates = &candidates[..candidate_count];
        // Every beam's own entries first: the blank, and a repeat that collapses.
        next.count = 0;
        for index in 0..beams.count {
            let source = beams.entries[index];
            let len = usize::from(source.len);
            let slot = next.count;
            next.tokens[slot][..len].copy_from_slice(&beams.tokens[index][..len]);
            next.steps[slot][..len].copy_from_slice(&beams.steps[index][..len]);
            let mut own = source;
            own.blank = lse(source.blank, source.non_blank) + b0;
            own.non_blank = f64::NEG_INFINITY;
            let last = (len != 0).then(|| beams.tokens[index][len - 1]);
            for (class, value) in candidates {
                if Some(*class) == last {
                    own.non_blank = lse(own.non_blank, source.non_blank + value);
                }
            }
            next.entries[slot] = own;
            next.count += 1;
        }
        let floor = if beams.count >= BEAM {
            next.entries[..next.count]
                .iter()
                .map(Entry::total)
                .fold(f64::INFINITY, f64::min)
        } else {
            f64::NEG_INFINITY
        };
        // Extensions that can reach the floor are scored by the model.
        let mut extensions = [(0_usize, 0_u16, 0.0_f64); BEAM * TOP_K];
        let mut extension_count = 0;
        for index in 0..beams.count {
            let source = beams.entries[index];
            let len = usize::from(source.len);
            let total = lse(source.blank, source.non_blank);
            let last = (len != 0).then(|| beams.tokens[index][len - 1]);
            for (class, value) in candidates {
                // After a blank, the same class is a new token.
                let new_from = if Some(*class) == last {
                    source.blank + value
                } else {
                    total + value
                };
                if new_from == f64::NEG_INFINITY {
                    continue;
                }
                let hash = extend_hash(source.hash, *class);
                let existing = (0..beams.count).find(|slot| {
                    let entry = &next.entries[*slot];
                    usize::from(entry.len) == len + 1
                        && entry.hash == hash
                        && next.tokens[*slot][..len] == beams.tokens[index][..len]
                        && next.tokens[*slot][len] == *class
                });
                if let Some(slot) = existing {
                    // An existing beam: merges, its model score stays.
                    next.entries[slot].non_blank = lse(next.entries[slot].non_blank, new_from);
                } else if new_from + source.score + reach >= floor - 1e-9 {
                    extensions[extension_count] = (index, *class, new_from);
                    extension_count += 1;
                }
            }
        }
        for (index, class, new_from) in &extensions[..extension_count] {
            let source = beams.entries[*index];
            let len = usize::from(source.len);
            let key = (source.history << bits) | (u64::from(*class) + 1);
            let probability = model.logp(key);
            let slot = next.count;
            if slot == CANDIDATES || len == MAX_TOKENS {
                return Err(EngineError::Limit("beam search candidates"));
            }
            next.tokens[slot][..len].copy_from_slice(&beams.tokens[*index][..len]);
            next.steps[slot][..len].copy_from_slice(&beams.steps[*index][..len]);
            next.tokens[slot][len] = *class;
            next.steps[slot][len] = t as u16;
            next.entries[slot] = Entry {
                blank: f64::NEG_INFINITY,
                non_blank: *new_from,
                score: source.score + params.alpha * probability + params.beta,
                len: (len + 1) as u16,
                history: key & history_mask,
                hash: extend_hash(source.hash, *class),
            };
            next.count += 1;
        }
        // Stable descending sort, as Python's sorted keeps insertion order for ties.
        let mut order = [0_usize; CANDIDATES];
        for slot in 0..next.count {
            let total = next.entries[slot].total();
            let mut position = slot;
            while position > 0 && next.entries[order[position - 1]].total() < total {
                order[position] = order[position - 1];
                position -= 1;
            }
            order[position] = slot;
        }
        beams.count = next.count.min(BEAM);
        for (target, slot) in order[..beams.count].iter().enumerate() {
            let len = usize::from(next.entries[*slot].len);
            beams.entries[target] = next.entries[*slot];
            beams.tokens[target][..len].copy_from_slice(&next.tokens[*slot][..len]);
            beams.steps[target][..len].copy_from_slice(&next.steps[*slot][..len]);
        }
    }
    // The beams are sorted by score; the first is the reference's `max`.
    let len = usize::from(beams.entries[0].len);
    for (target, token) in ids[..len].iter_mut().zip(&beams.tokens[0][..len]) {
        *target = i32::from(*token);
    }
    steps[..len].copy_from_slice(&beams.steps[0][..len]);
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn npy_bytes(descr: &str, shape: &str, data: &[u8]) -> Box<[u8]> {
        let mut header =
            format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
        while (10 + header.len() + 1) % 64 != 0 {
            header.push(' ');
        }
        header.push('\n');
        b"\x93NUMPY\x01\x00"
            .iter()
            .copied()
            .chain((header.len() as u16).to_le_bytes())
            .chain(header.bytes())
            .chain(data.iter().copied())
            .collect()
    }

    fn u64s(values: &[u64]) -> Box<[u8]> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn f32s(values: &[f32]) -> Box<[u8]> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A three-token, order-2 model with 4-bit keys written as a real npz.
    fn write_model(path: &Path, deflate: bool) {
        let file = File::create(path).expect("archive");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(if deflate {
            zip::CompressionMethod::Deflated
        } else {
            zip::CompressionMethod::Stored
        });
        let members: [(&str, Box<[u8]>); 11] = [
            ("order.npy", npy_bytes("<i8", "()", &2_i64.to_le_bytes())),
            (
                "vocab_size.npy",
                npy_bytes("<i8", "()", &4_i64.to_le_bytes()),
            ),
            ("bits.npy", npy_bytes("<i8", "()", &4_i64.to_le_bytes())),
            ("k0.npy", npy_bytes("<u8", "(3,)", &u64s(&[1, 2, 3]))),
            (
                "v0.npy",
                npy_bytes("<f4", "(3,)", &f32s(&[-1.0, -2.0, -3.0])),
            ),
            ("hk0.npy", npy_bytes("<u8", "(1,)", &u64s(&[0]))),
            ("hb0.npy", npy_bytes("<f4", "(1,)", &f32s(&[-0.5]))),
            // histories: start (0) and token 0 (1); pairs (0,t0) (t0,t1) (t0,t2)
            (
                "k1.npy",
                npy_bytes("<u8", "(3,)", &u64s(&[1, (1 << 4) | 2, (1 << 4) | 3])),
            ),
            (
                "v1.npy",
                npy_bytes("<f4", "(3,)", &f32s(&[-0.25, -0.125, -4.0])),
            ),
            ("hk1.npy", npy_bytes("<u8", "(2,)", &u64s(&[0, 1]))),
            ("hb1.npy", npy_bytes("<f4", "(2,)", &f32s(&[-0.75, 0.5]))),
        ];
        for (name, bytes) in members {
            writer.start_file(name, options).expect("member");
            writer.write_all(&bytes).expect("member bytes");
        }
        writer.finish().expect("archive end");
    }

    fn model(deflate: bool) -> Model {
        let path = std::env::temp_dir().join(format!(
            "indra-lm-{}-{}-{deflate}.npz",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "-")
        ));
        write_model(&path, deflate);
        let model = Model::load(&path).expect("synthetic model");
        std::fs::remove_file(path).expect("cleanup");
        model
    }

    #[test]
    fn logp_follows_the_vectorised_backoff_rule() {
        for deflate in [false, true] {
            let model = model(deflate);
            assert_eq!((model.order(), model.bits()), (2, 4));
            // Seen history (start) and seen pair: the bigram value.
            assert_eq!(model.logp(1), -0.25);
            // Seen history (start), unseen pair (start, t1): start backoff + unigram.
            assert_eq!(model.logp(2), -0.75 + -2.0);
            // Seen history t0 and seen pair (t0, t2).
            assert_eq!(model.logp((1 << 4) | 3), -4.0);
            // Seen history t0, unseen pair (t0, t0): backoff 0.5 + unigram of t0.
            assert_eq!(model.logp((1 << 4) | 1), 0.5 + -1.0);
            // Unseen history t1 (key 2): only the unigram, no backoff of the unseen history.
            assert_eq!(model.logp((2 << 4) | 3), -3.0);
            // Unseen everywhere: all valid backoffs (history length 0 only) and the uniform floor.
            assert_eq!(model.logp((2 << 4) | 4), -0.5 - 4.0_f64.ln());
            // max_logp: top value -0.125 plus the positive backoff 0.5.
            assert_eq!(model.max_logp(), -0.125 + 0.5);
            assert!(model.admits(14) && !model.admits(15));
        }
    }

    #[test]
    fn malformed_archives_are_rejected() {
        let bytes = npy_bytes("<u8", "(2,)", &u64s(&[5, 4]));
        assert!(npy(&bytes, "<u8").is_ok());
        assert!(npy(&bytes, "<f4").is_err());
        assert!(npy(&bytes[..20], "<u8").is_err());
        assert!(npy(&npy_bytes("<u8", "(3,)", &u64s(&[1, 2])), "<u8").is_err());
        let path = std::env::temp_dir().join(format!("indra-lm-bad-{}.npz", std::process::id()));
        std::fs::write(&path, b"not a zip").expect("fixture");
        assert!(Model::load(&path).is_err());
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn beam_search_matches_the_reference_on_a_synthetic_close_call() {
        let model = model(false);
        let mut scratch = Scratch::new();
        let params = Params {
            alpha: 1.0,
            beta: 0.0,
            gate: 0.9,
        };
        // Two steps; t0 and t1 are nearly tied at each step; the blank is unlikely.
        // Candidate classes: 1, 2 (t0, t1); the model favours t0 then t1.
        let p = |value: f64| value.ln() as f32;
        let blank = [p(0.02), p(0.02)];
        let mut topv = [f32::NEG_INFINITY; 2 * TOP_K];
        let mut topi = [0_i64; 2 * TOP_K];
        for t in 0..2 {
            topv[t * TOP_K] = p(0.49);
            topi[t * TOP_K] = 2;
            topv[t * TOP_K + 1] = p(0.47);
            topi[t * TOP_K + 1] = 1;
            topv[t * TOP_K + 2] = p(0.02);
            topi[t * TOP_K + 2] = 0;
        }
        let mut ids = [0_i32; MAX_TOKENS];
        let mut steps = [0_u16; MAX_TOKENS];
        let count = beam_search(
            &blank,
            &topv,
            &topi,
            &model,
            params,
            &mut scratch,
            &mut ids,
            &mut steps,
        )
        .expect("search");
        // Checked against lm.beam_search on the same npz: the model keeps the
        // repeat of t0 collapsed rather than extending with the likelier t1.
        assert_eq!((&ids[..count], &steps[..count]), (&[1][..], &[0][..]));
        // Without the model the greedy reading (t1, t1) collapses to one token.
        let count = beam_search(
            &blank,
            &topv,
            &topi,
            &model,
            Params {
                alpha: 0.0,
                beta: 0.0,
                gate: 0.9,
            },
            &mut scratch,
            &mut ids,
            &mut steps,
        )
        .expect("search");
        assert_eq!((&ids[..count], &steps[..count]), (&[2][..], &[0][..]));
        // A confidently blank step keeps every prefix and emits nothing.
        let count = beam_search(
            &[p(0.9995)],
            &topv[..TOP_K],
            &topi[..TOP_K],
            &model,
            params,
            &mut scratch,
            &mut ids,
            &mut steps,
        )
        .expect("blank step");
        assert_eq!(count, 0);
        assert!(
            beam_search(
                &blank,
                &topv[..TOP_K],
                &topi,
                &model,
                params,
                &mut scratch,
                &mut ids,
                &mut steps
            )
            .is_err()
        );
    }

    #[test]
    fn log_sum_exp_matches_the_reference_special_cases() {
        assert_eq!(lse(f64::NEG_INFINITY, -1.0), -1.0);
        assert_eq!(lse(-1.0, f64::NEG_INFINITY), -1.0);
        assert_eq!(lse(f64::NEG_INFINITY, f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert!((lse(0.0, 0.0) - 2.0_f64.ln()).abs() < 1e-15);
    }
}
