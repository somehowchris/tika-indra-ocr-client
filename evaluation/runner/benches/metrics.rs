use criterion::{Criterion, criterion_group, criterion_main};
use indra_evaluate::metrics::Workspace;
use std::hint::black_box;
fn measure(c: &mut Criterion) {
    let mut workspace = Workspace::default();
    c.bench_function("unicode_cer_reused_workspace", |b| b.iter(|| workspace.characters(black_box("Français: coût 42. Zürich, β, 中文, हिन्दी।"), black_box("Francais: cout 42. Zurich, β, 中文, हिन्दी।"), false).unwrap()));
}
criterion_group!(benches, measure);
criterion_main!(benches);
