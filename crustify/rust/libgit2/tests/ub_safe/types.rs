//! UB tests for `git2/types.h`: first the enums `ObjectType`, `ReferenceType`,
//! `SubmoduleRecurse` and `SubmoduleUpdate`.
//!
//! Each is a fieldless enum over its C enum: `git_object_t` (signed),
//! `git_reference_t`, `git_submodule_recurse_t` and `git_submodule_update_t`
//! (unsigned). The only safe way in from a raw value is `from_raw` / `try_from`, which
//! must reject every value without a variant rather than produce an invalid
//! discriminant (which Miri and UBSan would flag on the next read); an `as` cast to
//! any of them, or an infallible conversion to `ObjectType` or `ReferenceType`, does
//! not compile (`compile_fail/`). None of them owns or points at anything, and none of
//! these tests calls libgit2, so they also run under Miri.
//!
//! Then the `Time` value (`git_time`) and its handles.
//!
//! A `Time` is a plain 16-byte value with 3 bytes of padding. Its handles project each
//! field out of a pointer, so these tests exercise every getter and setter on every
//! holder Rust has for one (a local, a heap slot, an element of a run that moves when
//! it grows, a copy), through reborrows, and across threads under the `Send` / `Sync`
//! the crate asserts (TSan). Reading padding through a getter, a misaligned
//! projection or a write through the shared handle would be flagged by Miri and
//! ASan/UBSan. Handles that outlive their value, aliased exclusive handles and setters
//! on the shared handle do not compile (`compile_fail/`). None of them calls libgit2,
//! so all run under Miri; no C routine keeps a pointer to a bare `git_time`, so
//! there is no foreign write or retained pointer to race (BSan).

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::sync::Mutex;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::core::types::{
    NotAReferenceType, NotAnObjectType, ObjectType, ReferenceType, SubmoduleRecurse,
    SubmoduleUpdate, Time, TimeMut, TimeRef,
};

/// Every value around the declared ones, then a coarse stride across the whole
/// type; BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = u32> {
    (0..=300).chain((0..=u32::MAX).step_by(1 << 26)).chain([
        i32::MAX as u32,
        u32::MAX - 1,
        u32::MAX,
    ])
}

#[test]
fn from_raw_yields_only_valid_variants() {
    let mut recurse = HashSet::new();
    let mut update = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        match SubmoduleRecurse::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            Some(v) => {
                assert_eq!(v.raw(), raw);
                assert!(!format!("{v:?}").is_empty());
                recurse.insert(v);
            }
            None => assert_eq!(
                SubmoduleRecurse::try_from(raw).map_err(|e| e.raw()),
                Err(raw)
            ),
        }
        match SubmoduleUpdate::from_raw(black_box(raw)) {
            Some(v) => {
                assert_eq!(v.raw(), raw);
                assert!(!format!("{v:?}").is_empty());
                update.insert(v);
            }
            None => assert_eq!(
                SubmoduleUpdate::try_from(raw).map_err(|e| e.raw()),
                Err(raw)
            ),
        }
    }
    assert_eq!(recurse.len(), 3);
    assert_eq!(update.len(), 4);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<_>` stores `None` in a value no variant uses: a rejected value must
    // never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let r = black_box(SubmoduleRecurse::from_raw(raw));
        assert_eq!(r.map(SubmoduleRecurse::raw).unwrap_or(raw), raw);
        let u = black_box(SubmoduleUpdate::from_raw(raw));
        assert_eq!(u.map(SubmoduleUpdate::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn defaults_are_valid_variants() {
    let r = black_box(SubmoduleRecurse::default());
    assert_eq!(SubmoduleRecurse::from_raw(r.raw()), Some(r));
    let u = black_box(SubmoduleUpdate::default());
    assert_eq!(SubmoduleUpdate::from_raw(u.raw()), Some(u));
    // The header's "default" update value has no variant; resolving it lands on one.
    assert_eq!(
        SubmoduleUpdate::from_raw(black_box(0)).unwrap_or_default(),
        SubmoduleUpdate::Checkout
    );
}

#[test]
fn values_cross_threads() {
    // Plain values: `Send` and `Sync` without an assertion, so sharing them races
    // nothing for TSan to report.
    let recurse = [
        SubmoduleRecurse::No,
        SubmoduleRecurse::Yes,
        SubmoduleRecurse::OnDemand,
    ];
    let update = [
        SubmoduleUpdate::Checkout,
        SubmoduleUpdate::Rebase,
        SubmoduleUpdate::Merge,
        SubmoduleUpdate::None,
    ];
    thread::scope(|s| {
        let r: Vec<_> = recurse.iter().map(|v| s.spawn(move || v.raw())).collect();
        let u: Vec<_> = update.iter().map(|v| s.spawn(move || v.raw())).collect();
        for (v, h) in recurse.iter().zip(r) {
            assert_eq!(h.join().unwrap(), v.raw());
        }
        for (v, h) in update.iter().zip(u) {
            assert_eq!(h.join().unwrap(), v.raw());
        }
    });
}

/// Every value around the declared ones, then a coarse stride across the whole type;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn object_probes() -> impl Iterator<Item = ffi::git_object_t> {
    (-300..=300)
        .chain((ffi::git_object_t::MIN..=ffi::git_object_t::MAX).step_by(1 << 26))
        .chain([
            ffi::git_object_t::MIN + 1,
            ffi::git_object_t::MAX - 1,
            ffi::git_object_t::MAX,
        ])
}

/// As [`object_probes`], over the unsigned `git_reference_t`.
fn reference_probes() -> impl Iterator<Item = ffi::git_reference_t> {
    (0..=300)
        .chain((0..=ffi::git_reference_t::MAX).step_by(1 << 26))
        .chain([ffi::git_reference_t::MAX - 1, ffi::git_reference_t::MAX])
}

#[test]
fn object_from_raw_yields_only_valid_types() {
    let mut seen = HashSet::new();
    for raw in object_probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(kind) = ObjectType::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(kind.raw(), raw);
            assert_eq!(ObjectType::raw_or_any(Some(kind)), raw);
            assert!(!format!("{kind:?}").is_empty());
            seen.insert(kind);
        } else {
            assert_eq!(
                ObjectType::try_from(raw).map_err(NotAnObjectType::raw),
                Err(raw)
            );
        }
    }
    assert_eq!(seen.len(), 4);
}

#[test]
fn reference_from_raw_yields_only_valid_types() {
    let mut seen = HashSet::new();
    for raw in reference_probes() {
        if let Some(kind) = ReferenceType::from_raw(black_box(raw)) {
            assert_eq!(kind.raw(), raw);
            assert!(!format!("{kind:?}").is_empty());
            seen.insert(kind);
        } else {
            assert_eq!(
                ReferenceType::try_from(raw).map_err(NotAReferenceType::raw),
                Err(raw)
            );
        }
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn object_and_reference_option_niches_hold() {
    // `Option<_>` stores `None` in a value no variant uses: a rejected value must
    // never alias a variant, nor a variant read as `None`.
    for raw in object_probes() {
        let kind = black_box(ObjectType::from_raw(raw));
        assert_eq!(kind.map(ObjectType::raw).unwrap_or(raw), raw);
        // The "any" filter is the one value `None` maps back to.
        let filter = black_box(ObjectType::raw_or_any(kind));
        assert_eq!(filter, kind.map_or(ffi::GIT_OBJECT_ANY, ObjectType::raw));
    }
    for raw in reference_probes() {
        let kind = black_box(ReferenceType::from_raw(raw));
        assert_eq!(kind.map(ReferenceType::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn errors_format_every_extreme() {
    // Formatting the signed and unsigned extremes overflows nothing.
    for raw in [ffi::git_object_t::MIN, -1, 0, ffi::git_object_t::MAX] {
        let e = black_box(ObjectType::try_from(raw)).unwrap_err();
        assert!(e.to_string().contains(&raw.to_string()));
    }
    for raw in [0, 3, ffi::git_reference_t::MAX] {
        let e = black_box(ReferenceType::try_from(raw)).unwrap_err();
        assert!(e.to_string().contains(&raw.to_string()));
    }
}

#[test]
fn types_cross_threads() {
    // Plain values: `Send` and `Sync` without an assertion, so sharing them races
    // nothing for TSan to report.
    let objects = [
        ObjectType::Commit,
        ObjectType::Tree,
        ObjectType::Blob,
        ObjectType::Tag,
    ];
    let references = [ReferenceType::Direct, ReferenceType::Symbolic];
    thread::scope(|s| {
        let objects: Vec<_> = objects
            .iter()
            .map(|kind| s.spawn(move || ObjectType::from_raw(kind.raw())))
            .collect();
        let references: Vec<_> = references
            .iter()
            .map(|kind| s.spawn(move || ReferenceType::from_raw(kind.raw())))
            .collect();
        for (kind, h) in [
            ObjectType::Commit,
            ObjectType::Tree,
            ObjectType::Blob,
            ObjectType::Tag,
        ]
        .into_iter()
        .zip(objects)
        {
            assert_eq!(h.join().unwrap(), Some(kind));
        }
        for (kind, h) in [ReferenceType::Direct, ReferenceType::Symbolic]
            .into_iter()
            .zip(references)
        {
            assert_eq!(h.join().unwrap(), Some(kind));
        }
    });
}

// --- `git_time` ------------------------------------------------------------

const SIGNS: [u8; 6] = [0, b'+', b'-', b'x', 0x80, 0xff];

fn read(r: TimeRef<'_>) -> (i64, i32, u8) {
    (r.seconds(), r.offset_minutes(), r.sign())
}

fn write(mut m: TimeMut<'_>, seconds: i64, offset: i32, sign: u8) {
    m.set_seconds(seconds);
    m.set_offset_minutes(offset);
    m.set_sign(sign);
}

#[test]
fn every_field_round_trips_on_every_holder() {
    let mut local = Time::zeroed();
    let mut boxed = Box::new(Time::zeroed());
    let mut run = vec![Time::zeroed()];
    for (i, sign) in SIGNS.into_iter().enumerate() {
        let seconds = [i64::MIN, -1, 0, 1, i64::MAX][i % 5];
        let offset = [i32::MIN, -1, 0, 1, i32::MAX][i % 5];
        write(local.as_mut(), seconds, offset, sign);
        write(Time::as_mut(&mut boxed), seconds, offset, sign);
        // Growing the run moves its elements: every handle is taken afresh.
        run.push(Time::zeroed());
        let last = run.len() - 1;
        write(run[last].as_mut(), seconds, offset, sign);
        for time in [&local, &*boxed, &run[last]] {
            assert_eq!(read(time.as_ref()), (seconds, offset, sign));
            assert_eq!(
                read(time.as_ref().to_time().as_ref()),
                (seconds, offset, sign)
            );
        }
    }
}

#[test]
fn reborrows_keep_one_writer() {
    let mut time = Time::new(5, -30);
    let mut m = time.as_mut();
    {
        let mut inner = m.as_mut();
        inner.set_sign(b'x');
        assert_eq!(inner.as_ref().sign(), b'x');
    }
    let shared = m.as_ref();
    let copy = shared;
    assert_eq!(read(copy), (5, -30, b'x'));
    m.set_offset_minutes(0);
    assert_eq!(read(time.as_ref()), (5, 0, b'x'));
}

#[test]
fn copies_are_independent() {
    let c = {
        let a = Time::new(1, 60);
        let mut b = a;
        b.as_mut().set_seconds(2);
        assert_eq!(read(a.as_ref()), (1, 60, b'+'));
        assert_eq!(read(b.as_ref()), (2, 60, b'+'));
        let c = a.as_ref().to_time();
        assert_eq!(c, a);
        c
    };
    // The copy outlives the value it was read from.
    assert_eq!(read(c.as_ref()), (1, 60, b'+'));
}

#[test]
fn raw_values_convert_both_ways() {
    for sign in SIGNS {
        let raw = ffi::git_time {
            time: -7,
            offset: 330,
            sign: sign as _,
        };
        let time = Time::from(raw);
        assert_eq!(read(time.as_ref()), (-7, 330, sign));
        let back = ffi::git_time::from(time);
        assert_eq!((back.time, back.offset, back.sign as u8), (-7, 330, sign));
    }
}

#[test]
fn equality_and_hash_never_read_padding() {
    // `ffi::git_time` built by value leaves its padding unspecified; equal fields must
    // compare and hash equal anyway, and Miri checks no padding byte is read.
    let a = Time::from(ffi::git_time {
        time: 9,
        offset: -9,
        sign: b'-' as _,
    });
    let b = Time::new(9, -9);
    assert_eq!(a, b);
    let hash = |t: &Time| {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    };
    assert_eq!(hash(&a), hash(&b));
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    assert_eq!(format!("{:?}", a.as_ref()), format!("{b:?}"));
}

#[test]
#[cfg_attr(miri, ignore = "slow: many interleavings")]
fn shared_handles_read_across_threads() {
    let time = Time::new(1_700_000_000, 120);
    let r = time.as_ref();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                for _ in 0..1000 {
                    assert_eq!(read(r), (1_700_000_000, 120, b'+'));
                }
            });
        }
    });
}

#[test]
fn values_and_exclusive_handles_move_across_threads() {
    let mut time = Time::zeroed();
    thread::scope(|s| {
        let m = time.as_mut();
        s.spawn(move || write(m, 3, -3, b'-')).join().unwrap();
    });
    assert_eq!(read(time.as_ref()), (3, -3, b'-'));
    let moved = thread::spawn(move || {
        let mut time = time;
        time.as_mut().set_sign(b'x');
        time
    })
    .join()
    .unwrap();
    assert_eq!(read(moved.as_ref()), (3, -3, b'x'));
}

#[test]
fn a_locked_time_has_one_writer_at_a_time() {
    let time = Mutex::new(Time::zeroed());
    let rounds = if cfg!(miri) { 10 } else { 500 };
    thread::scope(|s| {
        for t in 0..4 {
            let time = &time;
            s.spawn(move || {
                for _ in 0..rounds {
                    let mut guard = time.lock().unwrap();
                    let n = guard.as_ref().seconds();
                    let mut m = guard.as_mut();
                    m.set_seconds(n + 1);
                    m.set_offset_minutes(t);
                }
            });
        }
    });
    assert_eq!(time.lock().unwrap().as_ref().seconds(), 4 * rounds as i64);
}
