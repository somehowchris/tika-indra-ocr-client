// Standalone compiler-contract probe; imports no evaluator code or metric crate.
fn main() {
    let (a, b, c) = char::UNICODE_VERSION;
    print!("{{\"rust_unicode_version\":[{a},{b},{c}],\"white_space_scalars\":[");
    let mut comma = false;
    for scalar in 0..=0x10ffff {
        if char::from_u32(scalar).is_some_and(char::is_whitespace) {
            if comma {
                print!(",");
            }
            print!("{scalar}");
            comma = true;
        }
    }
    println!("]}}");
}
