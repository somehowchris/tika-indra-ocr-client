unsafe extern "C" {
    fn native_answer() -> u32;
}

pub fn answer() -> u32 {
    // Diagnostic-only fixture links the tiny C function compiled by the producer.
    unsafe { native_answer() }
}
