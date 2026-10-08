//! UB tests that target the safe public API. Each tested TU gets a module here,
//! homed in `tests/ub_safe/<tu>.rs`.
#![forbid(unsafe_code)]

/// Illegal safe paths that must fail to compile, one file each under `compile_fail/`.
#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ub_safe/compile_fail/*.rs");
}
