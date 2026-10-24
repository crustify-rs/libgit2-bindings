//! UB tests for `git2/oid.h`: first the enum `OidType`.
//!
//! `OidType` is a fieldless enum over C's `git_oid_t`. The only safe way in from a
//! raw value is `from_raw` / `try_from`, which must reject every value without a
//! variant rather than produce an invalid discriminant (which Miri and `UBSan` would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`). Its
//! sizes index a `git_oid`'s id, so they must stay within `GIT_OID_MAX_SIZE` and its
//! hex form. None of them calls libgit2, so they also run under Miri.
//!
//! Then the wrapped types: the `Oid` value and its handles, its constructors over
//! `git_oid_from_string` / `git_oid_from_prefix` / `git_oid_from_raw` and its copy over
//! `git_oid_cpy`, and the `OidShortener` owner over `git_oid_shorten_new` / `git_oid_shorten_free`.
//!
//! Then the symbols: comparisons (`git_oid_cmp`, `_equal`, `_ncmp`, `_is_zero`,
//! `_strcmp`, `_streq`), formatters (`git_oid_fmt`, `_nfmt`, `_pathfmt`, `_tostr`,
//! `_tostr_s`) and `OidShortenerMut::add` (`git_oid_shorten_add`), whose shortener keeps
//! pointers into the ids it is given.
//!
//! Inputs and outputs sit in exact-length heap buffers, so `ASan` flags any access
//! libgit2 makes past them.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::hint::black_box;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::oid::{
    Oid, OidShortener, OidShortenerBox, OidShortenerMut, OidType,
};
use libgit2::libgit2::src::util::errors::ErrorClass;

use crate::fixture::isolated;

/// Every value around the declared ones and `git_oid.type`'s byte boundary, then a
/// coarse stride across the whole type; `BorrowSanitizer` instruments each comparison,
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
            assert_ne!(format!("{ty:?}"), "");
            seen.insert(ty);
        } else {
            assert_eq!(
                OidType::try_from(raw)
                    .map_err(libgit2::libgit2::src::libgit2::oid::UnnamedOidType::raw),
                Err(raw)
            );
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
        assert_eq!(ty.map_or(raw, OidType::raw), raw);
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
        src.as_mut().set_id(std::array::from_fn(|i| {
            i.to_le_bytes()[0].wrapping_mul(kind)
        }));
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
            assert!(id.set_elem(i, u8::try_from(i).unwrap()));
        }
        assert!(!id.set_elem(id.len(), 0));
        let mut sub = id.slice_mut(30..).unwrap();
        assert!(sub.copy_from_slice(&[0xAA, 0xBB]));
        assert!(!sub.copy_from_slice(&[0; 3]));
        assert!(id.slice_mut(33..).is_none());
        // Reborrowed shared while the exclusive run is frozen.
        assert_eq!(id.as_ref().elem(31), Some(0xBB));
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
    assert_eq!(r.id()[30..], [0xAA, 0xBB]);
}

/// Hashing reads the 33 bytes in place, on values and handles alike, so a hashed
/// collection can key on either; the typed null ids are distinct keys (Miri checks the
/// reads, and the equality here needs no libgit2 call: the keys are byte-distinct).
#[test]
fn hashes_read_within_the_oid() {
    use std::hash::{BuildHasher, RandomState};
    let state = RandomState::new();
    let mut oids: Vec<Box<Oid>> = Vec::new();
    for kind in [OidType::Sha1, OidType::Sha256] {
        oids.push(Box::new(Oid::zero(kind)));
    }
    for raw in [0, 3, u8::MAX] {
        let mut oid = Oid::zeroed();
        oid.as_mut().set_raw_kind(raw);
        oid.as_mut()
            .set_id(std::array::from_fn(|i| u8::try_from(i).unwrap() ^ raw));
        oids.push(Box::new(oid));
    }
    let hashes: Vec<u64> = oids.iter().map(|o| state.hash_one(**o)).collect();
    for (o, h) in oids.iter().zip(&hashes) {
        assert_eq!(state.hash_one(o.as_ref().as_ref()), *h);
        let copy: Oid = **o;
        assert_eq!(state.hash_one(copy), *h);
    }
    let distinct: HashSet<u64> = hashes.iter().copied().collect();
    assert_eq!(distinct.len(), oids.len());
}

/// `Oid` and its handles are `Send` / `Sync`: readers on many threads, a writer on
/// one at a time (`TSan` checks the claims, and Miri the Rust-side accesses).
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

/// Every shortener is freed once, on whichever thread drops it; `LSan` (asan variant)
/// flags a leak, `ASan` a double free, `TSan` a race in the `Send` / `Sync` claims.
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

/// Oids of every kind a wrapper must accept: both types, their null ids, unknown
/// type bytes, the typeless zero id, each in a heap allocation of exactly its 33
/// bytes.
#[expect(
    clippy::vec_box,
    reason = "boxed on purpose: ASan then sees each oid's own bounds"
)]
fn sample_oids() -> Vec<Box<Oid>> {
    let mut v = Vec::new();
    for (kind, hex) in [(OidType::Sha1, SHA1_HEX), (OidType::Sha256, SHA256_HEX)] {
        v.push(Box::new(Oid::from_prefix(hex.as_bytes(), kind).unwrap()));
        v.push(Box::new(Oid::from_prefix(b"0", kind).unwrap()));
        v.push(Box::new(Oid::zero(kind)));
    }
    for raw in [0, 3, u8::MAX] {
        let mut oid = Oid::zeroed();
        oid.as_mut().set_raw_kind(raw);
        oid.as_mut().set_id([0xC5; 32]);
        v.push(Box::new(oid));
    }
    v.push(Box::new(Oid::zeroed()));
    v
}

/// Hex strings of every length up to past the longest id, odd ones included, and with
/// a bad digit at either end, each in an exact `len + 1` heap buffer.
fn sample_hex() -> Vec<CString> {
    let mut v: Vec<CString> = (0..=SHA256_HEX.len() + 2)
        .map(|n| CString::new(SHA256_HEX.repeat(2)[..n].to_owned()).unwrap())
        .collect();
    v.push(CString::new(SHA1_HEX).unwrap());
    v.push(CString::new(SHA1_HEX.to_uppercase()).unwrap());
    v.push(CString::new(format!("z{}", &SHA1_HEX[1..])).unwrap());
    v.push(CString::new(format!("{}z", &SHA1_HEX[..39])).unwrap());
    v.push(CString::new(format!("\u{e9}{}", &SHA1_HEX[1..])).unwrap());
    v.push(CString::new([&SHA1_HEX.as_bytes()[..39], &[0xFF][..]].concat()).unwrap());
    v
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn comparisons_read_within_their_operands() {
    let oids = sample_oids();
    let hexes = sample_hex();
    for a in &oids {
        let a = a.as_ref().as_ref();
        let _ = black_box(a.is_zero());
        for b in &oids {
            let b = b.as_ref().as_ref();
            assert_eq!(a == b, a.cmp(&b) == Ordering::Equal);
            assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
            for len in [0, 1, 2, 39, 40, 41, 63, 64, 65, 1000, usize::MAX] {
                let _ = black_box(a.eq_prefix(b, len));
            }
        }
        // An oid aliasing itself.
        assert!(a == a && a.eq_prefix(a, usize::MAX));
        for hex in &hexes {
            assert_eq!(a.eq_hex(hex), a.cmp_hex(hex) == Ordering::Equal);
        }
    }
}

/// Every formatter, for every kind of oid, into exact-length heap buffers of every
/// size around the hex forms: nothing is written past them, and what comes back is
/// within them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn formatters_write_within_exact_buffers() {
    for oid in sample_oids() {
        let r = oid.as_ref().as_ref();
        let hex_size = r.kind().map(OidType::hex_size);

        let mut full = Box::new([b'#'; 64]);
        match r.format(&mut full) {
            Ok(hex) => assert_eq!(Some(hex.len()), hex_size),
            Err(_) => assert_eq!(hex_size, None),
        }
        let mut path = Box::new([b'#'; 65]);
        match r.format_path(&mut path) {
            Ok(p) => assert_eq!(Some(p.len() - 1), hex_size),
            Err(_) => assert_eq!(hex_size, None),
        }
        for n in 0..=70 {
            let mut out = vec![b'#'; n].into_boxed_slice();
            if let Ok(hex) = r.format_prefix(&mut out) {
                let hex_size = hex_size.unwrap();
                assert_eq!(hex.len(), n.min(hex_size));
                assert!(out[n.min(hex_size)..].iter().all(|&b| b == 0));
            } else {
                assert_eq!(hex_size, None);
                assert!(out.iter().all(|&b| b == b'#'));
            }
            let mut out = vec![b'#'; n].into_boxed_slice();
            if let Ok(s) = r.format_cstr(&mut out) {
                assert_eq!(
                    s.to_bytes().len(),
                    n.saturating_sub(1).min(hex_size.unwrap_or(0))
                );
            } else {
                assert_eq!(hex_size, None);
                assert_eq!(out[0], 0);
            }
        }
    }
}

/// `git_oid_tostr_s`'s per-thread string is uninitialised on a thread's first call,
/// which an unknown type returns unwritten; each thread's copy is its own.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn thread_strings_are_copied_out_on_every_thread() {
    let oids = sample_oids();
    let refs: Vec<_> = oids.iter().map(|o| o.as_ref().as_ref()).collect();
    thread::scope(|s| {
        for t in 0..6 {
            let refs = &refs;
            s.spawn(move || {
                // Unknown types first, on a fresh thread string.
                for r in refs
                    .iter()
                    .rev()
                    .chain(refs.iter())
                    .cycle()
                    .skip(t)
                    .take(40)
                {
                    match r.to_hex_string() {
                        Ok(hex) => assert_eq!(Some(hex.len()), r.kind().map(OidType::hex_size)),
                        Err(e) => {
                            assert!(r.kind().is_none());
                            assert_eq!(e.message(), c"unable to parse OID - unknown type");
                        }
                    }
                    // Other libgit2 calls on the thread in between.
                    let mut buf = [0; 64];
                    drop(r.format(&mut buf));
                }
            });
        }
    });
}

/// The formatters as the first libgit2 call in the process: failures are reported on
/// initialised thread state.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn formatters_before_init_pin_the_runtime() {
    isolated(
        concat!(module_path!(), "::formatters_before_init_pin_the_runtime"),
        || {
            let unknown = Oid::zeroed();
            let mut buf = [0; 64];
            let err = unknown.as_ref().format(&mut buf).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert!(unknown.as_ref().format_prefix(&mut buf).is_err());
            assert!(unknown.as_ref().format_cstr(&mut buf).is_err());
            assert!(unknown.as_ref().format_path(&mut [0; 65]).is_err());
            assert!(unknown.as_ref().to_hex_string().is_err());
        },
    );
}

/// Deterministic, distinct-looking 40-digit hex ids.
fn random_ids(n: usize, mut seed: u64) -> Vec<Box<str>> {
    use std::fmt::Write as _;
    (0..n)
        .map(|_| {
            let mut id = String::with_capacity(40);
            while id.len() < 40 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let _ = write!(id, "{seed:016x}");
            }
            id.truncate(40);
            id.into_boxed_str()
        })
        .collect()
}

/// Ids in exact 40-byte heap buffers, no NUL after them: libgit2 reads, and keeps
/// pointers into, no more than that (`ASan`), and the result never shrinks.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn add_reads_forty_digits_of_ids_it_keeps() {
    let mut ids = random_ids(600, 0x9E37_79B9_7F4A_7C15);
    // Shared prefixes of every length, duplicates, and longer ids.
    for k in [1, 4, 20, 38, 39] {
        let mut id = ids[0].to_string();
        let flip = if &id[k..=k] == "0" { "1" } else { "0" };
        id.replace_range(k..=k, flip);
        ids.push(id.into_boxed_str());
    }
    ids.push(ids[1].clone());
    ids.push(SHA256_HEX.into());
    ids.push(SHA1_HEX.to_uppercase().into_boxed_str());
    let mut b = OidShortener::new(4).unwrap();
    let mut last = 0;
    for id in &ids {
        let len = b.as_mut().add(id).unwrap();
        assert!((last..=41).contains(&len), "{len} after {last}");
        last = len;
    }
    assert_eq!(last, 41);
    drop(b);
    drop(ids);
}

/// No id the wrapper refuses reaches C: a shortener that saw them answers as one that
/// did not.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn malformed_ids_leave_the_shortener_untouched() {
    let good = random_ids(64, 7);
    let mut malformed: Vec<String> = Vec::new();
    for i in 0..40 {
        let mut id = good[0].to_string();
        id.replace_range(i..=i, "x");
        malformed.push(id);
        malformed.push(good[1][..i].to_owned());
    }
    let mut clean = OidShortener::new(0).unwrap();
    let mut probed = OidShortener::new(0).unwrap();
    for id in &good {
        for bad in &malformed {
            assert!(probed.as_mut().add(bad).is_err());
        }
        assert_eq!(probed.as_mut().add(id), clean.as_mut().add(id));
    }
}

/// Filling the trie to libgit2's limit: the last additions fail, the shortener stays
/// usable for further (failing) additions and is freed once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[cfg_attr(
    bsan,
    ignore = "tens of thousands of additions stall BorrowSanitizer; the other variants run it"
)]
fn a_full_shortener_keeps_failing_safely() {
    let ids = random_ids(40_000, 0x2545_F491_4F6C_DD1D);
    let mut b = OidShortener::new(0).unwrap();
    let mut added = 0;
    let err = loop {
        match b.as_mut().add(&ids[added]) {
            Ok(_) => added += 1,
            Err(e) => break e,
        }
    };
    assert!(added > 10_000, "{added}");
    assert_eq!(err.class(), Some(ErrorClass::Invalid));
    assert_eq!(err.message(), c"unable to shorten OID - OID set full");
    for id in &ids[added..added + 8] {
        assert_eq!(b.as_mut().add(id).unwrap_err(), err);
    }
    assert!(b.as_mut().add("zz").is_err());
}

/// The shortener only reads the ids it keeps: Rust reads them between additions
/// (`BorrowSanitizer` checks the retained pointers against these reborrows), and an
/// owner moved to another thread keeps adding (`TSan` checks the `Send` claim).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn kept_ids_are_shared_with_rust_readers() {
    let ids = random_ids(64, 99);
    let mut b = OidShortener::new(0).unwrap();
    for (i, id) in ids.iter().enumerate().take(32) {
        b.as_mut().add(id).unwrap();
        let total: usize = ids[..=i].iter().map(|s| s.as_bytes()[39] as usize).sum();
        assert!(total > 0);
    }
    let ids = &ids;
    thread::scope(|s| {
        s.spawn(move || {
            let mut m: OidShortenerMut<'_, '_> = b.as_mut();
            for id in &ids[32..] {
                assert!(m.add(id).unwrap() >= 1);
            }
            drop(b);
        });
        s.spawn(move || ids.iter().map(|id| id.len()).sum::<usize>());
    });
}
