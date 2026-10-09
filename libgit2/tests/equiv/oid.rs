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
//!
//! [`Oid`]'s constructors must parse as `git_oid_from_string`, `git_oid_from_prefix`
//! and `git_oid_from_raw` do, its field accessors and copies must read and write the
//! `git_oid` bytes as C code and `git_oid_cpy` do, and [`OidShortener::new`] must
//! build what `git_oid_shorten_new` builds. Each side runs the same sequence of steps
//! on a fresh thread of its own, over independently owned inputs, recording after
//! every step the result (the oid bytes, or the error the call reported) and the
//! thread's last libgit2 error, read with `git_error_last`; the two records are
//! compared step by step.
//!
//! Documented divergences, outside these comparisons:
//!
//! - `Oid::from_raw` refuses a slice whose length is not the type's size before the
//!   call, where C would read past a shorter buffer; the module's `unit_tests` cover
//!   it.
//! - `OidShortener::new` refuses a `min_length` that does not survive C's round trip
//!   through an `int` before the call, with the error a release build of libgit2
//!   sets; a debug build's C assertion aborts instead, so the C side of those
//!   comparisons is skipped under the `coverage` variant, the one debug build.
//! - Both pin libgit2's runtime first, where C called before `git_libgit2_init`
//!   corrupts foreign thread-local state; every comparison here runs initialised.

use std::ffi::{CStr, CString, c_char, c_int};
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidShortener, OidType, UnnamedOidType};
use libgit2::libgit2::util::errors::ErrorBuf;

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

const SHA1_HEX: &str = "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3";
const SHA256_HEX: &str = "6ca13d52ca70c883e0f0bb101e425a89e8624de51db2d2392593af6a84118090";
const KINDS: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

/// The type byte and the 32 id bytes.
type Bytes = (u8, [u8; 32]);

/// One step's result: the oid, or the error class and message the call reported.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Oid(Bytes),
    Failed(c_int, CString),
}

/// One step's record: its result, and the thread's last error after it.
type Record = (Outcome, (c_int, CString));

/// The calling thread's last error, as C reads it.
fn c_last() -> (c_int, CString) {
    // SAFETY: the runtime is initialised; the result is a static error or this
    // thread's, live until the next libgit2 call on this thread.
    let e = unsafe { ffi::git_error_last() };
    assert!(!e.is_null());
    // SAFETY: `e` is a live `git_error` whose message is NULL or a C string.
    unsafe {
        let message = (*e).message;
        let message = if message.is_null() {
            c""
        } else {
            CStr::from_ptr(message)
        };
        ((*e).klass, message.to_owned())
    }
}

fn c_bytes(oid: &ffi::git_oid) -> Bytes {
    (oid.type_, oid.id)
}

fn rust_bytes(oid: &Oid) -> Bytes {
    (oid.as_ref().raw_kind(), oid.as_ref().id())
}

fn c_outcome(rc: c_int, out: &ffi::git_oid) -> Outcome {
    if rc == 0 {
        Outcome::Oid(c_bytes(out))
    } else {
        let (class, message) = c_last();
        Outcome::Failed(class, message)
    }
}

fn rust_outcome(r: Result<Oid, ErrorBuf>) -> Outcome {
    match r {
        Ok(oid) => Outcome::Oid(rust_bytes(&oid)),
        Err(e) => Outcome::Failed(e.raw_class(), e.message().to_owned()),
    }
}

/// Runs `steps` on a fresh, initialised thread, recording each.
fn run<T: Send + 'static>(steps: Vec<T>, step: fn(&T) -> Outcome) -> Vec<Record> {
    thread::spawn(move || {
        init();
        steps
            .iter()
            .map(|s| {
                let outcome = step(s);
                (outcome, c_last())
            })
            .collect()
    })
    .join()
    .unwrap()
}

fn assert_same(c: Vec<Record>, rust: Vec<Record>, what: &str) {
    assert_eq!(c.len(), rust.len());
    for (i, (c, r)) in c.iter().zip(&rust).enumerate() {
        assert_eq!(c, r, "{what}: step {i}");
    }
}

/// Hex strings around every boundary: both sizes, one either side, bad characters,
/// mixed case, and the empty string.
fn hex_inputs() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Vec::new();
    for hex in [SHA1_HEX, SHA256_HEX] {
        let b = hex.as_bytes();
        for len in [0, 1, 2, 3, 4, b.len() - 1, b.len()] {
            v.push(b[..len].to_vec());
        }
        v.push([b, b"0"].concat());
        v.push(hex.to_uppercase().into_bytes());
        let mut bad = b.to_vec();
        *bad.last_mut().unwrap() = b'g';
        v.push(bad);
        let mut bad = b.to_vec();
        bad[0] = b' ';
        v.push(bad);
    }
    v.push(vec![b'f'; 65]);
    v.push(b"0000".to_vec());
    v
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_string_parses_as_c() {
    let steps: Vec<(CString, OidType)> = hex_inputs()
        .into_iter()
        .flat_map(|s| KINDS.map(|k| (CString::new(s.clone()).unwrap(), k)))
        .collect();
    let c = run(steps.clone(), |(s, kind)| {
        let mut out = ffi::git_oid::default();
        // SAFETY: `out` is a live `git_oid`; `s` a C string read for the call.
        let rc = unsafe { ffi::git_oid_from_string(&mut out, s.as_ptr(), kind.raw()) };
        c_outcome(rc, &out)
    });
    let rust = run(steps, |(s, kind)| rust_outcome(Oid::from_string(s, *kind)));
    assert_same(c, rust, "from_string");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_prefix_parses_as_c() {
    let steps: Vec<(Vec<u8>, OidType)> = hex_inputs()
        .into_iter()
        .chain([b"a\0b".to_vec(), b"\xff".to_vec(), b"abc".to_vec()])
        .flat_map(|s| KINDS.map(|k| (s.clone(), k)))
        .collect();
    let c = run(steps.clone(), |(s, kind)| {
        let mut out = ffi::git_oid::default();
        // SAFETY: `out` is a live `git_oid`; `s` is valid for `s.len()` bytes.
        let rc =
            unsafe { ffi::git_oid_from_prefix(&mut out, s.as_ptr().cast(), s.len(), kind.raw()) };
        c_outcome(rc, &out)
    });
    let rust = run(steps, |(s, kind)| rust_outcome(Oid::from_prefix(s, *kind)));
    assert_same(c, rust, "from_prefix");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_raw_parses_as_c() {
    // Only inputs C can read: as long as the type needs.
    let mut steps: Vec<(Vec<u8>, OidType)> = Vec::new();
    for seed in [0u8, 1, 0x7f, 0xff] {
        let bytes: Vec<u8> = (0..32)
            .map(|i| seed.wrapping_mul(31).wrapping_add(i))
            .collect();
        steps.push((bytes[..20].to_vec(), OidType::Sha1));
        steps.push((bytes, OidType::Sha256));
    }
    let c = run(steps.clone(), |(raw, kind)| {
        let mut out = ffi::git_oid::default();
        // SAFETY: `out` is a live `git_oid`; `raw` holds the type's size in bytes.
        let rc = unsafe { ffi::git_oid_from_raw(&mut out, raw.as_ptr(), kind.raw()) };
        c_outcome(rc, &out)
    });
    let rust = run(steps, |(raw, kind)| rust_outcome(Oid::from_raw(raw, *kind)));
    assert_same(c, rust, "from_raw");
}

/// A run of field writes and copies, compared after each step: C writes the struct's
/// fields and calls `git_oid_cpy`; Rust uses the setters, `copy_from` and `to_oid`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn field_writes_and_copies_match_c() {
    init();
    let id = |seed: u8| std::array::from_fn::<u8, 32, _>(|i| seed ^ (i as u8).wrapping_mul(7));

    let mut c_a = ffi::git_oid::default();
    let mut c_b = ffi::git_oid::default();
    let mut r_a = Oid::zeroed();
    let mut r_b = Oid::zeroed();
    let check = |c_a: &ffi::git_oid, c_b: &ffi::git_oid, r_a: &Oid, r_b: &Oid, step| {
        assert_eq!(c_bytes(c_a), rust_bytes(r_a), "a, step {step}");
        assert_eq!(c_bytes(c_b), rust_bytes(r_b), "b, step {step}");
    };
    check(&c_a, &c_b, &r_a, &r_b, 0);

    c_a.type_ = 1;
    r_a.as_mut().set_kind(OidType::Sha1);
    check(&c_a, &c_b, &r_a, &r_b, 1);

    c_a.id = id(0x5a);
    r_a.as_mut().set_id(id(0x5a));
    check(&c_a, &c_b, &r_a, &r_b, 2);

    // SAFETY: two distinct live `git_oid`s.
    assert_eq!(unsafe { ffi::git_oid_cpy(&mut c_b, &c_a) }, 0);
    r_b.as_mut().copy_from(r_a.as_ref());
    check(&c_a, &c_b, &r_a, &r_b, 3);

    c_a.id[7] = 0xee;
    c_a.id[31] = 0x01;
    {
        let mut m = r_a.as_mut();
        let mut run = m.id_mut();
        assert!(run.set_elem(7, 0xee));
        assert!(run.set_elem(31, 0x01));
    }
    check(&c_a, &c_b, &r_a, &r_b, 4);

    c_b.type_ = 0xfe;
    r_b.as_mut().set_raw_kind(0xfe);
    check(&c_a, &c_b, &r_a, &r_b, 5);

    // SAFETY: two distinct live `git_oid`s.
    assert_eq!(unsafe { ffi::git_oid_cpy(&mut c_a, &c_b) }, 0);
    r_a = r_b.as_ref().to_oid();
    check(&c_a, &c_b, &r_a, &r_b, 6);

    // A parsed oid read back through the getters, against C's fields.
    let mut c_p = ffi::git_oid::default();
    let hex = CString::new(SHA256_HEX).unwrap();
    // SAFETY: `c_p` is a live `git_oid`; `hex` a C string.
    let rc = unsafe { ffi::git_oid_from_string(&mut c_p, hex.as_ptr(), OidType::Sha256.raw()) };
    assert_eq!(rc, 0);
    let r_p = Oid::from_string(&hex, OidType::Sha256).unwrap();
    assert_eq!(c_bytes(&c_p), rust_bytes(&r_p));
    let mut viewed = [0; 32];
    assert!(r_p.as_ref().id_bytes().copy_to_slice(&mut viewed));
    assert_eq!(viewed, c_p.id);
}

/// Whether the linked libgit2 is a debug build, whose argument assertions abort.
fn c_asserts_abort() -> bool {
    std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() == Ok("coverage")
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shortener_new_matches_c() {
    // Every length that survives C's `(size_t)((int)min_length)` round trip: up to
    // `i32::MAX`, and the sign extensions of negative `int`s.
    let mut lengths = vec![0usize, 1, 4, 40, 64, 1000, c_int::MAX as usize];
    lengths.extend([c_int::MIN as usize, -2i32 as usize, usize::MAX]);
    if !c_asserts_abort() {
        lengths.extend([
            c_int::MAX as usize + 1,
            u32::MAX as usize,
            usize::MAX / 2,
            c_int::MIN as usize - 1,
        ]);
    }
    let c = run(lengths.clone(), |&min_length| {
        // SAFETY: any length; the result is NULL or a fresh shortener we own.
        let os = unsafe { ffi::git_oid_shorten_new(min_length) };
        if os.is_null() {
            let (class, message) = c_last();
            return Outcome::Failed(class, message);
        }
        // SAFETY: `os` is live and owned here.
        unsafe { ffi::git_oid_shorten_free(os) };
        Outcome::Oid((1, [0; 32]))
    });
    let rust = run(lengths, |&min_length| match OidShortener::new(min_length) {
        Ok(shortener) => {
            drop(shortener);
            Outcome::Oid((1, [0; 32]))
        }
        Err(e) => Outcome::Failed(e.raw_class(), e.message().to_owned()),
    });
    assert_same(c, rust, "shorten_new");
}

/// Parses and shorteners interleaved, so each step starts from the error state the
/// previous left: a success keeps the previous error on both sides.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mixed_sequences_leave_the_same_error_state() {
    #[derive(Clone)]
    enum Step {
        Str(CString, OidType),
        Prefix(Vec<u8>, OidType),
        Raw(Vec<u8>, OidType),
        Shorten(usize),
    }
    let steps = vec![
        Step::Str(CString::new(SHA1_HEX).unwrap(), OidType::Sha1),
        Step::Prefix(b"zz".to_vec(), OidType::Sha1),
        Step::Shorten(4),
        Step::Raw(vec![9; 32], OidType::Sha256),
        Step::Str(CString::new("abc").unwrap(), OidType::Sha256),
        Step::Raw(vec![7; 20], OidType::Sha1),
        Step::Prefix(SHA256_HEX.as_bytes().to_vec(), OidType::Sha1),
        Step::Prefix(b"0".to_vec(), OidType::Sha256),
        Step::Shorten(0),
    ];
    let c = run(steps.clone(), |s| {
        let mut out = ffi::git_oid::default();
        let rc = match s {
            // SAFETY: `out` is a live `git_oid`; `s` a C string read for the call.
            Step::Str(s, k) => unsafe { ffi::git_oid_from_string(&mut out, s.as_ptr(), k.raw()) },
            // SAFETY: `out` is a live `git_oid`; `p` is valid for `p.len()` bytes.
            Step::Prefix(p, k) => unsafe {
                ffi::git_oid_from_prefix(&mut out, p.as_ptr().cast(), p.len(), k.raw())
            },
            // SAFETY: `out` is a live `git_oid`; `r` holds the type's size in bytes.
            Step::Raw(r, k) => unsafe { ffi::git_oid_from_raw(&mut out, r.as_ptr(), k.raw()) },
            Step::Shorten(n) => {
                // SAFETY: a length that round-trips through an `int`; the result is
                // NULL or a fresh shortener owned here.
                let os = unsafe { ffi::git_oid_shorten_new(*n) };
                assert!(!os.is_null());
                // SAFETY: `os` is live and owned here.
                unsafe { ffi::git_oid_shorten_free(os) };
                0
            }
        };
        c_outcome(rc, &out)
    });
    let rust = run(steps, |s| match s {
        Step::Str(s, k) => rust_outcome(Oid::from_string(s, *k)),
        Step::Prefix(p, k) => rust_outcome(Oid::from_prefix(p, *k)),
        Step::Raw(r, k) => rust_outcome(Oid::from_raw(r, *k)),
        Step::Shorten(n) => {
            drop(OidShortener::new(*n).unwrap());
            Outcome::Oid((0, [0; 32]))
        }
    });
    assert_same(c, rust, "mixed");
}
