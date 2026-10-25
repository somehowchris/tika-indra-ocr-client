//! A standalone executable avoids process-wide libtest allocator noise.
use std::{alloc::System, hint::black_box};

use indra_engine::{EngineError, MAX_BATCH, MAX_LINES, Result};
use stats_alloc::{Region, Stats, StatsAlloc};

#[global_allocator]
static ALLOCATOR: StatsAlloc<System> = StatsAlloc::system();

#[path = "../src/batching.rs"]
#[allow(dead_code, unused_imports)]
mod batching;
#[path = "../src/continuous/plan.rs"]
#[allow(dead_code, unused_imports)]
mod continuous_plan;
#[path = "support/width_scan.rs"]
mod width_scan;

#[path = "support/language_model.rs"]
mod language_model;

static NFC: icu_normalizer::ComposingNormalizerBorrowed<'static> =
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc();

fn no_churn(stats: &Stats) {
    assert_eq!(stats.allocations, 0);
    assert_eq!(stats.reallocations, 0);
    assert_eq!(stats.deallocations, 0);
    assert_eq!(stats.bytes_allocated, 0);
    assert_eq!(stats.bytes_deallocated, 0);
    assert_eq!(stats.bytes_reallocated, 0);
}

/// The beam search and the right-to-left flip run entirely in startup scratch:
/// a warmed line costs zero Rust allocator operations on all six counters.
fn language_model_and_bidi_paths_have_no_rust_allocator_churn() {
    let model = language_model::synthetic_model(&ALLOCATOR);
    let mut scratch = indra_engine::gates::BeamScratch::new();
    let line = language_model::synthetic_line(96);
    let mut ids = [0_i32; indra_engine::gates::MAX_TOKENS];
    let mut steps = [0_u16; indra_engine::gates::MAX_TOKENS];
    let mut bidi = Box::new(indra_engine::gates::BidiScratch::new());
    let mut out = String::with_capacity(4096);
    let params = indra_engine::gates::LmParams {
        alpha: 0.5,
        beta: 1.5,
        gate: 0.9,
    };
    // Warm once: nothing lazy remains afterwards, but the gate measures the loop.
    indra_engine::gates::beam_search(
        &line.blank,
        &line.topv,
        &line.topi,
        &model,
        params,
        &mut scratch,
        &mut ids,
        &mut steps,
    )
    .expect("warm search");
    let region = Region::new(&ALLOCATOR);
    let mut emitted = 0_usize;
    for _ in 0..200 {
        emitted += black_box(
            indra_engine::gates::beam_search(
                black_box(&line.blank),
                black_box(&line.topv),
                black_box(&line.topi),
                &model,
                params,
                &mut scratch,
                &mut ids,
                &mut steps,
            )
            .expect("bounded beam search"),
        );
        for key in [0_u64, 0x1234, 0xabcde, u64::from(u16::MAX)] {
            black_box(model.logp(black_box(key)));
        }
        out.clear();
        indra_engine::gates::flip(
            black_box(language_model::RTL_LINE),
            Some(true),
            &mut bidi,
            &mut out,
        )
        .expect("bounded flip");
        out.clear();
        indra_engine::gates::flip(
            black_box(language_model::MIXED_LINE),
            None,
            &mut bidi,
            &mut out,
        )
        .expect("bounded flip");
        black_box(indra_engine::gates::rtl_base(language_model::MIXED_LINE));
        // NFC through the locked normaliser: seventeen pending marks stay inline.
        let mut composed = 0_usize;
        for ch in NFC.normalize_iter(black_box(language_model::MARKED_LINE).chars()) {
            composed += usize::from(ch != '\0');
        }
        black_box(composed);
    }
    let observed = region.change();
    no_churn(&observed);
    println!(
        "{{\"scope\":\"language model beam search, logp and bidi flip\",\"allocator\":\"System\",\"iterations\":200,\"tokens\":{emitted},\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_deallocated\":0,\"bytes_reallocated\":0}}"
    );
}

fn actual_width_planning_has_no_rust_allocator_churn() {
    let widths: [usize; MAX_LINES] = std::array::from_fn(|index| 8 + (index * 73) % 4089);
    let region = Region::new(&ALLOCATOR);
    for count in [16, 64, 256, 1024] {
        for cap in [1, 8, 64] {
            for math in [false, true] {
                for _ in 0..1000 {
                    let active = black_box(&widths[..count]);
                    let policy = if math {
                        batching::Policy::Math
                    } else {
                        batching::Policy::Text
                    };
                    let mut plan = batching::Plan::new(active, black_box(cap), policy, |index| {
                        !index.is_multiple_of(3)
                    })
                    .expect("fixed bounded input");
                    while let Some(range) = plan.next(active) {
                        black_box(&plan.indices[range]);
                    }
                }
            }
        }
    }
    let observed = region.change();
    no_churn(&observed);
    println!(
        "{{\"scope\":\"Rust width planner\",\"allocator\":\"System\",\"plans\":24000,\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_deallocated\":0,\"bytes_reallocated\":0}}"
    );
}

fn changed_tensor_scan_preserves_shape_and_zero_rust_allocator_churn() {
    let mut widths = [0; MAX_BATCH];
    let mut legacy = [0; MAX_BATCH];
    let mut proved = [0; MAX_BATCH];
    let region = Region::new(&ALLOCATOR);
    for case in &width_scan::CASES {
        black_box(case.name);
        case.fill(&mut widths);
        for _ in 0..1000 {
            let active = black_box(&widths[..case.count]);
            let previous = width_scan::legacy_max(active, case.height, &mut legacy);
            let plan = batching::tensor_widths(
                active.iter().copied().map(Ok),
                case.height,
                case.height * case.columns,
                &mut proved,
            )
            .expect("prespecified shape fits the production arena");
            assert_eq!(
                (plan.width() as usize, case.height * plan.work() as usize),
                previous
            );
            assert_eq!(proved, legacy);
            black_box(plan);
        }
    }
    let observed = region.change();
    no_churn(&observed);
    println!(
        "{{\"scope\":\"legacy and production tensor width scans\",\"allocator\":\"System\",\"comparison_pairs\":8000,\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_deallocated\":0,\"bytes_reallocated\":0}}"
    );
}

fn main() {
    language_model_and_bidi_paths_have_no_rust_allocator_churn();
    actual_width_planning_has_no_rust_allocator_churn();
    changed_tensor_scan_preserves_shape_and_zero_rust_allocator_churn();
    continuous_owner_selection_has_no_rust_allocator_churn();
}

fn continuous_owner_selection_has_no_rust_allocator_churn() {
    let widths: [[usize; MAX_LINES]; 2] = std::array::from_fn(|owner| {
        std::array::from_fn(|index| 8 + (index * 73 + owner * 19) % 4089)
    });
    let mut indices: [[usize; MAX_LINES]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|index| index));
    for (owner, sorted) in indices.iter_mut().enumerate() {
        sorted.sort_unstable_by_key(|i| (widths[owner][*i], *i));
    }
    let flags = [false; MAX_LINES];
    let mut batch = continuous_plan::Batch::new();
    let region = Region::new(&ALLOCATOR);
    for count in [64, 1024] {
        for cap in [1, 8, 64] {
            for _ in 0..100 {
                let mut cursor = [0; 2];
                let mut anchor = 0;
                let mut processed = [0; 2];
                while cursor.iter().any(|cursor| *cursor < count) {
                    if cursor[anchor] == count {
                        anchor = 1 - anchor;
                    }
                    let views = std::array::from_fn(|owner| {
                        (cursor[owner] < count).then_some(continuous_plan::View {
                            widths: &widths[owner],
                            indices: &indices[owner][..count],
                            flags: &flags,
                            route: None,
                            position: cursor[owner],
                        })
                    });
                    continuous_plan::select(black_box(views), anchor, black_box(cap), &mut batch)
                        .expect("bounded two-owner selection");
                    for owner in 0..2 {
                        if batch.owners[owner] != 0 {
                            cursor[owner] = batch.end[owner];
                            processed[owner] += batch.owners[owner];
                        }
                    }
                    black_box(&batch.rows[..batch.count]);
                    anchor = 1 - anchor;
                }
                assert_eq!(processed, [count; 2]);
            }
        }
    }
    let observed = region.change();
    no_churn(&observed);
    println!(
        "{{\"scope\":\"continuous fixed owner selection\",\"allocator\":\"System\",\"paired_pages\":600,\"allocations\":0,\"reallocations\":0,\"deallocations\":0,\"bytes_allocated\":0,\"bytes_deallocated\":0,\"bytes_reallocated\":0}}"
    );
}
