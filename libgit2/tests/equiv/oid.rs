//! Equivalence of `git2/oid.h`'s [`OidType`] with the `git_oid_t` values libgit2
//! declares and accepts.
//!
//! [`OidType`] must name exactly the values of `git_oid_t`, each with its C value, and
//! its default must be `GIT_OID_DEFAULT`. libgit2 observes a type through the parsers
//! that take one: `git_oid_from_raw` accepts exactly the named types, copies
//! [`OidType::size`] bytes and records the type in `git_oid.type`,
//! `git_oid_from_prefix` accepts at most [`OidType::hex_size`] digits, and
//! `git_oid_from_string` exactly that many. Each probe runs
//! the C parser and predicts its result from the Rust conversion alone; a sequence of
//! parses into one `git_oid` is compared after every step.

use std::ffi::{CStr, CString, c_char, c_int};

use libgit2::ffi;
use libgit2::libgit2::oid::oid::{OidType, UnnamedOidType};

/// Every `git_oid_t` constant, beside the variant that must carry it.
const TYPES: [(ffi::git_oid_t, OidType); 2] = [
    (ffi::GIT_OID_SHA1, OidType::Sha1),
    (ffi::GIT_OID_SHA256, OidType::Sha256),
];

/// Raw id bytes no parse of either type reads past: `GIT_OID_MAX_SIZE` of them.
const RAW: [u8; 32] = [0xa5; 32];

/// More hex digits than any type takes, NUL-terminated.
const HEX: &CStr = c"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0";

/// Every value around the declared ones, `git_oid.type`'s byte boundary, and the
/// extremes.
fn probes() -> impl Iterator<Item = ffi::git_oid_t> {
    (0..=300).chain([ffi::git_oid_t::MAX - 1, ffi::git_oid_t::MAX])
}

/// Initialises libgit2, whose error state a rejected parse writes; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The message of the calling thread's last error.
fn last_message() -> String {
    // SAFETY: `git_error_last` never returns NULL, and its message is a NUL-terminated
    // string, both live until the next libgit2 call on this thread, after the copy.
    unsafe { CStr::from_ptr((*ffi::git_error_last()).message) }
        .to_string_lossy()
        .into_owned()
}

/// `git_oid_from_raw(out, RAW, raw)`.
fn from_raw(out: &mut ffi::git_oid, raw: ffi::git_oid_t) -> c_int {
    // SAFETY: `out` is a writable `git_oid`, and `RAW` holds the most bytes any type
    // reads.
    unsafe { ffi::git_oid_from_raw(out, RAW.as_ptr(), raw) }
}

/// `git_oid_from_prefix(out, HEX, len, raw)`, with `len` at most `HEX`'s length.
fn from_prefix(out: &mut ffi::git_oid, len: usize, raw: ffi::git_oid_t) -> c_int {
    assert!(len <= HEX.to_bytes().len());
    // SAFETY: `out` is a writable `git_oid`; libgit2 reads at most `len` digits of
    // `HEX`, which holds as many.
    unsafe { ffi::git_oid_from_prefix(out, HEX.as_ptr().cast::<c_char>(), len, raw) }
}

/// `git_oid_from_string(out, str, raw)`.
fn from_string(out: &mut ffi::git_oid, str: &CStr, raw: ffi::git_oid_t) -> c_int {
    // SAFETY: `out` is a writable `git_oid`, and `str` is NUL-terminated; libgit2
    // reads at most its hex size, plus one, of it, and stops at the NUL.
    unsafe { ffi::git_oid_from_string(out, str.as_ptr(), raw) }
}

/// The `git_oid` that parsing `RAW` as `ty` produces, as the Rust type predicts it.
fn expected_from_raw(ty: OidType) -> ffi::git_oid {
    let mut oid = ffi::git_oid {
        type_: u8::try_from(ty.raw()).unwrap(),
        ..Default::default()
    };
    oid.id[..ty.size()].copy_from_slice(&RAW[..ty.size()]);
    oid
}

fn same(a: &ffi::git_oid, b: &ffi::git_oid) -> bool {
    a.type_ == b.type_ && a.id == b.id
}

#[test]
fn types_carry_their_c_values() {
    for (c, ty) in TYPES {
        assert_eq!(ty.raw(), c, "{ty:?}");
        assert_eq!(ffi::git_oid_t::from(ty), c, "{ty:?}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn default_is_git_oid_default() {
    // SAFETY: no arguments; returns a constant.
    let c = unsafe { ffi::crustify_GIT_OID_DEFAULT() };
    assert_eq!(OidType::default().raw(), c);
    assert_eq!(OidType::from_raw(c), Some(OidType::default()));
}

#[test]
fn from_raw_names_exactly_the_c_types() {
    for raw in probes() {
        let c = TYPES.iter().find(|(c, _)| *c == raw).map(|&(_, ty)| ty);
        assert_eq!(OidType::from_raw(raw), c, "{raw}");
        assert_eq!(
            OidType::try_from(raw).map_err(UnnamedOidType::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_oid_from_raw_accepts_exactly_the_named_types() {
    init();
    for raw in probes() {
        let mut out = ffi::git_oid::default();
        let ret = from_raw(&mut out, raw);
        match OidType::from_raw(raw) {
            Some(ty) => {
                assert_eq!(ret, 0, "{raw}");
                assert!(same(&out, &expected_from_raw(ty)), "{raw}: {out:?}");
            }
            None => {
                assert_eq!(ret, -1, "{raw}");
                assert_eq!(last_message(), "unable to parse OID - unknown type");
                assert!(same(&out, &ffi::git_oid::default()), "{raw}: {out:?}");
            }
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_oid_from_prefix_takes_hex_size_digits() {
    init();
    for raw in probes() {
        let mut out = ffi::git_oid::default();
        match OidType::from_raw(raw) {
            Some(ty) => {
                assert_eq!(from_prefix(&mut out, ty.hex_size(), raw), 0, "{raw}");
                assert_eq!(out.type_ as ffi::git_oid_t, ty.raw());
                // Every digit landed: the bytes past the id stay zero.
                assert!(out.id[..ty.size()].iter().any(|&b| b != 0), "{ty:?}");
                assert!(out.id[ty.size()..].iter().all(|&b| b == 0), "{ty:?}");

                assert_eq!(from_prefix(&mut out, ty.hex_size() + 1, raw), -1);
                assert_eq!(last_message(), "unable to parse OID - too long");
            }
            None => {
                assert_eq!(from_prefix(&mut out, 1, raw), -1, "{raw}");
                assert_eq!(last_message(), "unable to parse OID - unknown type");
            }
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_oid_from_string_takes_exactly_hex_size_digits() {
    init();
    let digits = HEX.to_bytes();
    for raw in probes() {
        let mut out = ffi::git_oid::default();
        match OidType::from_raw(raw) {
            Some(ty) => {
                for (len, ret, message) in [
                    (ty.hex_size(), 0, None),
                    (ty.hex_size() + 1, -1, Some("too long")),
                ] {
                    let str = CString::new(&digits[..len]).unwrap();
                    assert_eq!(from_string(&mut out, &str, raw), ret, "{ty:?} {len}");
                    if let Some(message) = message {
                        assert_eq!(last_message(), format!("unable to parse OID - {message}"));
                    }
                }
                // The NUL ends a shorter string inside the hex size: an invalid digit.
                let short = CString::new(&digits[..ty.hex_size() - 1]).unwrap();
                assert_eq!(from_string(&mut out, &short, raw), -1, "{ty:?}");
                assert_eq!(
                    last_message(),
                    "unable to parse OID - contains invalid characters"
                );
            }
            None => {
                assert_eq!(from_string(&mut out, HEX, raw), -1, "{raw}");
                assert_eq!(last_message(), "unable to parse OID - unknown type");
            }
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parses_into_one_oid_follow_the_rust_type() {
    init();
    // One `git_oid` reused across types: a rejected type leaves it as the previous
    // parse did, an accepted one rewrites all of it, the longer id's tail included.
    let steps: [ffi::git_oid_t; 7] = [
        ffi::GIT_OID_SHA256,
        ffi::GIT_OID_SHA1,
        0,
        ffi::GIT_OID_SHA256,
        3,
        ffi::GIT_OID_SHA1,
        ffi::GIT_OID_SHA1 | 0x100,
    ];
    let mut out = ffi::git_oid::default();
    let mut expected = ffi::git_oid::default();
    for (step, raw) in steps.into_iter().enumerate() {
        let ret = from_raw(&mut out, raw);
        let ty = OidType::try_from(raw);
        assert_eq!(ret, if ty.is_ok() { 0 } else { -1 }, "step {step}");
        if let Ok(ty) = ty {
            expected = expected_from_raw(ty);
        }
        assert!(same(&out, &expected), "step {step}: {out:?}");
    }
}
