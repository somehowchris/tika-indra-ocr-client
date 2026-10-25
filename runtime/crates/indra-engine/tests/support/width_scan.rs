//! Prespecified shapes shared by the allocation gate and Criterion comparison.
use indra_engine::MAX_BATCH;

pub struct Case {
    pub name: &'static str,
    pub count: usize,
    pub width: usize,
    pub mixed: bool,
    pub height: usize,
    pub columns: usize,
}

impl Case {
    pub fn fill(&self, widths: &mut [usize; MAX_BATCH]) {
        for (index, width) in widths.iter_mut().enumerate() {
            *width = self.width + if self.mixed { index % 8 } else { 0 };
        }
    }
}

static TEXT_COLUMNS: usize = MAX_BATCH * (640 + 8);
static MATH_COLUMNS: usize = MAX_BATCH * 4096;

pub static CASES: [Case; 8] = [
    Case {
        name: "text-1x8",
        count: 1,
        width: 8,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "text-8x64",
        count: 8,
        width: 64,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "text-64x640",
        count: 64,
        width: 640,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "text-62x657",
        count: 62,
        width: 657,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "text-63x649",
        count: 63,
        width: 649,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "text-1x4096",
        count: 1,
        width: 4096,
        mixed: false,
        height: 32,
        columns: TEXT_COLUMNS,
    },
    Case {
        name: "math-64x4096",
        count: 64,
        width: 4096,
        mixed: false,
        height: 64,
        columns: MATH_COLUMNS,
    },
    Case {
        name: "text-8xmixed640-647",
        count: 8,
        width: 640,
        mixed: true,
        height: 32,
        columns: TEXT_COLUMNS,
    },
];

/// The pre-integration scan, retained only as a comparison arm.
pub fn legacy_max(
    widths: &[usize],
    height: usize,
    output: &mut [i64; MAX_BATCH],
) -> (usize, usize) {
    let mut widest = 0;
    for (slot, width) in widths.iter().enumerate() {
        output[slot] = *width as i64;
        widest = widest.max(*width);
    }
    let padded = widest.div_ceil(8) * 8;
    (padded, widths.len() * height * padded)
}
