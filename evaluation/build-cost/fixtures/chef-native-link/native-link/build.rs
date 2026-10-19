fn main() {
    println!("cargo:rustc-link-search=native=/native-fixture");
    println!("cargo:rustc-link-lib=static=native_answer");
    println!("cargo:rerun-if-changed=/native-fixture/libnative_answer.a");
}
