//! UB tests for `git2/errors.h`'s enums, `ErrorCode` and `ErrorClass`.
//!
//! Both are fieldless enums over C `int`s. The only safe way in from an `int` is
//! `from_raw` / `try_from`, which must reject every value without a variant rather
//! than produce an invalid discriminant (which Miri and UBSan would flag on the next
//! read); an `as` cast to either enum does not compile (`compile_fail/`). Neither
//! calls libgit2, so every test here also runs under Miri.

use std::collections::HashSet;
use std::ffi::c_int;
use std::hint::black_box;
use std::thread;

use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

/// Every `int` around the declared ranges, then a coarse stride across the whole type;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = c_int> {
    (-64..=64)
        .chain((c_int::MIN..=c_int::MAX).step_by(1 << 26))
        .chain([c_int::MIN, c_int::MIN + 1, c_int::MAX - 1, c_int::MAX])
}

#[test]
fn from_raw_yields_only_valid_codes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(code) = ErrorCode::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(code.raw(), raw);
            assert!(!format!("{code:?}").is_empty());
            seen.insert(code);
        } else {
            assert_eq!(ErrorCode::try_from(raw), Err(raw));
        }
    }
    assert_eq!(seen.len(), 35);
}

#[test]
fn from_raw_yields_only_valid_classes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        if let Some(class) = ErrorClass::from_raw(black_box(raw)) {
            assert_eq!(class.raw(), raw);
            assert!(!format!("{class:?}").is_empty());
            seen.insert(class);
        } else {
            assert_eq!(ErrorClass::try_from(raw), Err(raw));
        }
    }
    assert_eq!(seen.len(), 37);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<ErrorCode>` stores `None` in a value no variant uses: a rejected value
    // must never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let code = black_box(ErrorCode::from_raw(raw));
        let class = black_box(ErrorClass::from_raw(raw));
        assert_eq!(code.map(ErrorCode::raw).unwrap_or(raw), raw);
        assert_eq!(class.map(ErrorClass::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn enums_cross_threads() {
    // Plain `Copy` data: shared and sent between threads without synchronisation.
    let codes: Vec<ErrorCode> = (-40..=0).filter_map(ErrorCode::from_raw).collect();
    let classes: Vec<ErrorClass> = (0..=36).filter_map(ErrorClass::from_raw).collect();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for code in &codes {
                    assert_eq!(ErrorCode::from_raw(code.raw()), Some(*code));
                }
                for class in &classes {
                    assert_eq!(ErrorClass::from_raw(class.raw()), Some(*class));
                }
            });
        }
    });
    let moved = thread::spawn(move || (codes.len(), classes.len()))
        .join()
        .unwrap();
    assert_eq!(moved, (35, 37));
}
