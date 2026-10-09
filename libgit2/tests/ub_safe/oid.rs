//! UB tests for `git2/oid.h`: first the enum `OidType`.
//!
//! `OidType` is a fieldless enum over C's `git_oid_t`. The only safe way in from a
//! raw value is `from_raw` / `try_from`, which must reject every value without a
//! variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). Its
//! sizes index a `git_oid`'s id, so they must stay within `GIT_OID_MAX_SIZE` and its
//! hex form. None of them calls libgit2, so they also run under Miri.
//!
//! Then the wrapped types: the `Oid` value and its handles, its constructors over `git_oid_from_string` / `git_oid_from_prefix` /
//! `git_oid_from_raw` and its copy over `git_oid_cpy`, and the `OidShortener` owner
//! over `git_oid_shorten_new` / `git_oid_shorten_free`.
//!
//! Inputs sit in exact-length heap buffers, so ASan flags any read libgit2 makes past
//! them.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::hint::black_box;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidShortener, OidShortenerBox, OidType};
use libgit2::libgit2::util::errors::ErrorClass;

use crate::fixture::isolated;

/// Every value around the declared ones and `git_oid.type`'s byte boundary, then a
/// coarse stride across the whole type; BorrowSanitizer instruments each comparison,
/// so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::git_oid_t> {
    (0..=300)
        .chain((0..=ffi::git_oid_t::MAX).step_by(1 << 26))
        .chain([ffi::git_oid_t::MAX - 1, ffi::git_oid_t::MAX])
}

#[test]
fn from_raw_yields_only_valid_types() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(ty) = OidType::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(ty.raw(), raw);
            assert!(!format!("{ty:?}").is_empty());
            seen.insert(ty);
        } else {
            assert_eq!(OidType::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<OidType>` stores `None` in a value no variant uses: a rejected value
    // must never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let ty = black_box(OidType::from_raw(raw));
        assert_eq!(ty.map(OidType::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn sizes_stay_within_a_git_oid() {
    let oid = ffi::git_oid::default();
    let mut hex = [0u8; 2 * 32];
    for raw in probes() {
        let Some(ty) = OidType::from_raw(black_box(raw)) else {
            continue;
        };
        // Slicing panics, rather than reading out of bounds, if a size overshoots.
        let id = &oid.id[..black_box(ty).size()];
        let digits = &mut hex[..black_box(ty).hex_size()];
        assert_eq!(digits.len(), 2 * id.len());
        digits.fill(b'0');
    }
}

#[test]
fn types_cross_threads() {
    // A plain value: `Send` and `Sync` without an assertion, so sharing one races
    // nothing for TSan to report.
    let types = [OidType::Sha1, OidType::Sha256, OidType::default()];
    thread::scope(|s| {
        let handles: Vec<_> = types
            .iter()
            .map(|ty| s.spawn(move || (ty.raw(), ty.size(), ty.hex_size())))
            .collect();
        for (ty, h) in types.iter().zip(handles) {
            assert_eq!(h.join().unwrap(), (ty.raw(), ty.size(), ty.hex_size()));
        }
    });
}

const SHA1_HEX: &str = "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3";
const SHA256_HEX: &str = "6ca13d52ca70c883e0f0bb101e425a89e8624de51db2d2392593af6a84118090";

/// `s` in a heap buffer of exactly its length: no NUL, nothing after it.
fn exact(s: &[u8]) -> Box<[u8]> {
    s.to_vec().into_boxed_slice()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_before_init_pin_the_runtime() {
    // Each failure sets libgit2's thread error, which before initialisation would be
    // written through a TLS key that does not exist.
    isolated(
        concat!(module_path!(), "::constructors_before_init_pin_the_runtime"),
        || {
            let err = Oid::from_string(c"zz", OidType::Sha1).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert!(Oid::from_prefix(b"", OidType::Sha1).is_err());
            assert!(Oid::from_raw(&[0; 19], OidType::Sha1).is_err());
            assert!(OidShortener::new(u32::MAX as usize).is_err());
            drop(OidShortener::new(0).unwrap());
        },
    );
}

/// The shortener's allocation is libgit2's first one in the process.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shortener_first_before_init() {
    isolated(
        concat!(module_path!(), "::shortener_first_before_init"),
        || {
            let shortener = OidShortener::new(7).unwrap();
            drop(shortener);
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_prefix_reads_only_its_slice() {
    for kind in [OidType::Sha1, OidType::Sha256] {
        let hex = if kind == OidType::Sha1 {
            SHA1_HEX
        } else {
            SHA256_HEX
        };
        for len in [1, 2, 3, 39, 40, 41, 63, 64] {
            let prefix = exact(&hex.as_bytes()[..len.min(hex.len())]);
            let oid = Oid::from_prefix(&prefix, kind).unwrap();
            assert_eq!(oid.as_ref().kind(), Some(kind));
        }
        // Too long: refused before any character is read.
        let long = exact(&[b'a'; 65]);
        assert!(Oid::from_prefix(&long, kind).is_err());
        // Empty: never read.
        assert!(Oid::from_prefix(&[], kind).is_err());
        // Bad characters, last one included.
        for bad in [&b"g"[..], b"a\0", b"abcz", b"\xff"] {
            assert!(Oid::from_prefix(&exact(bad), kind).is_err());
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_string_reads_within_the_string() {
    let cases: [(&str, OidType, bool); 7] = [
        (SHA1_HEX, OidType::Sha1, true),
        (SHA256_HEX, OidType::Sha256, true),
        (&SHA1_HEX[..39], OidType::Sha1, false),
        ("", OidType::Sha1, false),
        ("a", OidType::Sha256, false),
        (SHA256_HEX, OidType::Sha1, false),
        (SHA1_HEX, OidType::Sha256, false),
    ];
    for (hex, kind, ok) in cases {
        // A `CString` is an exact `len + 1` heap buffer.
        let s = CString::new(hex).unwrap();
        assert_eq!(
            Oid::from_string(&s, kind).is_ok(),
            ok,
            "{hex:?} as {kind:?}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_raw_reads_exactly_the_type_size() {
    let raw20 = exact(&[0x11; 20]);
    let raw32 = exact(&[0x22; 32]);
    let oid = Oid::from_raw(&raw20, OidType::Sha1).unwrap();
    assert_eq!(
        oid.as_ref().id()[..21],
        [[0x11; 20].as_slice(), &[0]].concat()[..]
    );
    let oid = Oid::from_raw(&raw32, OidType::Sha256).unwrap();
    assert_eq!(oid.as_ref().id(), [0x22; 32]);
    // A slice shorter than the type needs never reaches C.
    assert!(Oid::from_raw(&raw20, OidType::Sha256).is_err());
    assert!(Oid::from_raw(&raw20[..19], OidType::Sha1).is_err());
    assert!(Oid::from_raw(&[], OidType::Sha1).is_err());
    // A longer slice too, though C would read only its prefix.
    assert!(Oid::from_raw(&raw32, OidType::Sha1).is_err());
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_of_any_byte_pattern() {
    for kind in [0, 1, 2, 3, u8::MAX] {
        let mut src = Oid::zeroed();
        src.as_mut().set_raw_kind(kind);
        src.as_mut()
            .set_id(std::array::from_fn(|i| (i as u8).wrapping_mul(kind)));
        let mut dst = Oid::zeroed();
        dst.as_mut().copy_from(src.as_ref());
        assert_eq!(dst.as_ref().raw_kind(), kind);
        assert_eq!(dst.as_ref().id(), src.as_ref().id());
        let copy = dst.as_ref().to_oid();
        dst.as_mut().set_id([0; 32]);
        assert_eq!(copy.as_ref().id(), src.as_ref().id());
    }
}

/// The handles and runs over Rust storage, without libgit2; Miri checks the
/// projections.
#[test]
fn views_stay_in_bounds() {
    let mut oid = Oid::zeroed();
    {
        let mut m = oid.as_mut();
        let mut id = m.id_mut();
        for i in 0..id.len() {
            assert!(id.set_elem(i, i as u8));
        }
        assert!(!id.set_elem(id.len(), 0));
        let mut sub = id.slice_mut(30..).unwrap();
        assert!(sub.copy_from_slice(&[0xaa, 0xbb]));
        assert!(!sub.copy_from_slice(&[0; 3]));
        assert!(id.slice_mut(33..).is_none());
        // Reborrowed shared while the exclusive run is frozen.
        assert_eq!(id.as_ref().elem(31), Some(0xbb));
        m.set_raw_kind(2);
        assert_eq!(m.as_ref().raw_kind(), 2);
    }
    let r = oid.as_ref();
    let bytes = r.id_bytes();
    assert_eq!(bytes.elems().count(), 32);
    assert_eq!(bytes.elem(29), Some(29));
    assert_eq!(bytes.elem(32), None);
    assert!(bytes.slice(..33).is_none());
    let (head, tail) = bytes.split_at(20).unwrap();
    assert_eq!((head.len(), tail.len()), (20, 12));
    assert_eq!(r.id()[30..], [0xaa, 0xbb]);
}

/// `Oid` and its handles are `Send` / `Sync`: readers on many threads, a writer on
/// one at a time (TSan checks the claims, and Miri the Rust-side accesses).
#[test]
fn oids_cross_threads() {
    let mut oid = Oid::zeroed();
    oid.as_mut().set_kind(OidType::Sha1);
    oid.as_mut().set_id([0x33; 32]);
    thread::scope(|s| {
        let r = oid.as_ref();
        for _ in 0..4 {
            s.spawn(move || {
                assert_eq!(r.id(), [0x33; 32]);
                assert_eq!(r.id_bytes().elem(31), Some(0x33));
                assert_eq!(r.kind(), Some(OidType::Sha1));
            });
        }
    });
    thread::scope(|s| {
        let mut m = oid.as_mut();
        s.spawn(move || {
            m.set_id([0x44; 32]);
            assert!(m.id_mut().set_elem(0, 0x45));
        });
    });
    let moved = thread::spawn(move || oid).join().unwrap();
    assert_eq!(moved.as_ref().id()[..2], [0x45, 0x44]);
}

/// libgit2 copies one shared oid on many threads at once (`git_oid_cpy` only reads
/// its source).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn oid_copies_cross_threads() {
    let oid = Oid::from_raw(&[0x33; 20], OidType::Sha1).unwrap();
    let r = oid.as_ref();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                let copy = r.to_oid();
                assert_eq!(copy.as_ref().id()[..20], [0x33; 20]);
                let mut dst = Oid::zeroed();
                dst.as_mut().copy_from(r);
                assert_eq!(dst.as_ref().raw_kind(), 1);
            });
        }
    });
}

/// Every shortener is freed once, on whichever thread drops it; LSan (asan variant)
/// flags a leak, ASan a double free, TSan a race in the `Send` / `Sync` claims.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shorteners_are_freed_once_on_any_thread() {
    let boxes: Vec<OidShortenerBox<'static>> =
        (0..8).map(|i| OidShortener::new(i).unwrap()).collect();
    thread::scope(|s| {
        for b in &boxes {
            s.spawn(move || format!("{:?}", b.as_ref()));
        }
    });
    let handles: Vec<_> = boxes
        .into_iter()
        .map(|mut b| {
            thread::spawn(move || {
                let mut m = b.as_mut();
                let _ = m.as_mut().as_ref();
                drop(b);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let threads: Vec<_> = (0..4)
        .map(|_| thread::spawn(|| (0..64).for_each(|i| drop(OidShortener::new(i).unwrap()))))
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

/// A shortener borrowing short-lived strings is dropped before them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shortener_lifetime_follows_its_strings() {
    let text = CString::new(SHA1_HEX).unwrap();
    let borrowed: &CStr = &text;
    let shortener: OidShortenerBox<'_> = OidShortener::new(borrowed.to_bytes().len()).unwrap();
    drop(shortener);
    drop(text);
}
