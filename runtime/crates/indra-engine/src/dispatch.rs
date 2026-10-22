//! Runtime CPU dispatch facts, recorded once per worker at startup.
//!
//! Published images are portable builds (no `target-cpu=native`); the kernels
//! that actually execute are chosen at runtime by ONNX Runtime, OpenCV and the
//! Rust standard library from CPUID. The facts are logged, not assumed, so a
//! measurement can state which instruction sets its host dispatched.

use crate::Result;
use opencv::core;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CpuDispatch {
    pub sse4_2: bool,
    pub avx: bool,
    pub fma: bool,
    pub avx2: bool,
    pub avx512f: bool,
    pub opencv_sse4_2: bool,
    pub opencv_avx2: bool,
    pub opencv_fma3: bool,
    pub opencv_avx512f: bool,
}

#[cfg(target_arch = "x86_64")]
fn rust_features() -> [bool; 5] {
    [
        std::arch::is_x86_feature_detected!("sse4.2"),
        std::arch::is_x86_feature_detected!("avx"),
        std::arch::is_x86_feature_detected!("fma"),
        std::arch::is_x86_feature_detected!("avx2"),
        std::arch::is_x86_feature_detected!("avx512f"),
    ]
}

#[cfg(not(target_arch = "x86_64"))]
fn rust_features() -> [bool; 5] {
    [false; 5]
}

impl CpuDispatch {
    /// Standard-library CPUID detection beside OpenCV's own dispatcher answer.
    pub fn detect() -> Result<Self> {
        let [sse4_2, avx, fma, avx2, avx512f] = rust_features();
        Ok(Self {
            sse4_2,
            avx,
            fma,
            avx2,
            avx512f,
            opencv_sse4_2: core::check_hardware_support(core::CPU_SSE4_2)?,
            opencv_avx2: core::check_hardware_support(core::CPU_AVX2)?,
            opencv_fma3: core::check_hardware_support(core::CPU_FMA3)?,
            opencv_avx512f: core::check_hardware_support(core::CPU_AVX_512F)?,
        })
    }
}

/// OpenCV's compiled baseline plus the dispatched extensions, as one startup
/// string (for example `SSE SSE2 SSE3 *SSE4.1 *SSE4.2 *FP16 *AVX *AVX2 ...`).
pub fn opencv_cpu_features() -> Result<String> {
    Ok(core::get_cpu_features_line()?)
}
