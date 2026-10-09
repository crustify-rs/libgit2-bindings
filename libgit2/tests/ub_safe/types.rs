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
//!
//! Then the enum `FileMode`.
//!
//! `FileMode` is a fieldless enum over C's `git_filemode_t`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value
//! without a variant rather than produce an invalid discriminant (which Miri and
//! UBSan would flag on the next read); an `as` cast to it does not compile
//! (`compile_fail/`). None of these calls libgit2, so they also run under Miri.
//!
//! Then `git_writestream`, in `write_streams`: Rust and C streams, their owners and
//! handles, lent to and freed by libgit2.
//!
//! Then the enum `SubmoduleIgnore` (`git_submodule_ignore_t`, signed, `-1` and
//! `1..=4`): converting C values is checked before any variant exists, so no value can
//! reach an invalid discriminant. Rust only, so Miri runs it; the sweep stays small
//! for BSan.
//!
//! Then `BranchType` (`git_branch_t`): its raw conversions yield only valid variants
//! for every input, so a C value can never make an invalid enum; pure Rust, so Miri runs
//! it.
//!
//! Then `git_signature`, in `signatures`: owned signatures, views over borrowed strings,
//! and the handles both lend, including to libgit2, which copies and writes them.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::sync::Mutex;
use std::thread;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::core::types::{
    BranchType, FileMode, NotAReferenceType, NotAnObjectType, ObjectType, ReferenceType,
    SubmoduleIgnore, SubmoduleRecurse, SubmoduleUpdate, Time, TimeMut, TimeRef,
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

/// Every type nibble with the permission bits git trees spell, the values around
/// the declared ones, a coarse stride across the whole type and its extremes;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn mode_probes() -> impl Iterator<Item = ffi::git_filemode_t> {
    let kinds = (0..16)
        .flat_map(|t| [0, 0o644, 0o755, 0o664, 0o777, 0o001, 0o100].map(move |p| (t << 12) | p));
    let named = [0o000000, 0o040000, 0o100644, 0o100755, 0o120000, 0o160000];
    let around = named
        .into_iter()
        .flat_map(|m: u32| [m.wrapping_sub(1), m, m + 1]);
    kinds
        .chain(around)
        .chain((0..=ffi::git_filemode_t::MAX).step_by(1 << 26))
        .chain([
            0o177777,
            0o200000,
            ffi::git_filemode_t::MAX - 1,
            ffi::git_filemode_t::MAX,
        ])
}

#[test]
fn from_raw_yields_only_valid_modes() {
    let mut seen = HashSet::new();
    for raw in mode_probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(mode) = FileMode::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(mode.raw(), raw);
            assert!(!format!("{mode:?}").is_empty());
            seen.insert(mode);
        } else {
            assert_eq!(FileMode::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 6);
}

#[test]
fn mode_option_niche_holds_for_every_value() {
    // `Option<FileMode>` stores `None` in a value no variant uses: a rejected value
    // must never alias a variant, nor a variant read as `None`.
    for raw in mode_probes() {
        let mode = black_box(FileMode::from_raw(raw));
        assert_eq!(mode.map(FileMode::raw).unwrap_or(raw), raw);
        // The unreadable mode is 0, the value a zeroed discriminant would hold.
        assert_eq!(mode == Some(FileMode::Unreadable), raw == 0);
    }
}

#[test]
fn modes_cross_threads() {
    // A plain value: `Send` and `Sync` without an assertion, so sharing one races
    // nothing for TSan to report.
    let modes = [
        FileMode::Unreadable,
        FileMode::Tree,
        FileMode::Blob,
        FileMode::BlobExecutable,
        FileMode::Link,
        FileMode::Commit,
    ];
    thread::scope(|s| {
        let handles: Vec<_> = modes
            .iter()
            .map(|mode| s.spawn(move || FileMode::from_raw(mode.raw())))
            .collect();
        for (mode, h) in modes.iter().zip(handles) {
            assert_eq!(h.join().unwrap(), Some(*mode));
        }
    });
}

mod write_streams {
    //! UB tests for `git2/types.h`'s `git_writestream`: `RustWriteStream` and its
    //! `StreamWriter`s, `WriteStreamBox` over Rust and C streams, and the handles both lend,
    //! including to libgit2, which writes, closes and frees them.

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::thread;

    use libgit2::libgit2::core::types::{RustWriteStream, StreamError, StreamWriter, WriteStream};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode, last};

    use crate::fixture::{
        Filters, TempRepo, blob_stream, commit_blob, init, isolated, stream_buffer, upper_streams,
    };

    /// Up to just past a page; whole-slice comparisons keep BSan's per-access cost low.
    const SIZES: [usize; 7] = [0, 1, 2, 7, 63, 4096, 4097];

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 + 7) as u8).collect()
    }

    /// What a writer saw, in order.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Write(Vec<u8>),
        Close,
        Drop,
    }

    type Log = Rc<RefCell<Vec<Call>>>;

    /// A writer logging into a log the test keeps, failing on request.
    struct Logged {
        log: Log,
        fail_write: Option<StreamError>,
        fail_close: Option<StreamError>,
    }

    fn logged() -> (Logged, Log) {
        let log = Log::default();
        let writer = Logged {
            log: log.clone(),
            fail_write: None,
            fail_close: None,
        };
        (writer, log)
    }

    impl StreamWriter for Logged {
        fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
            self.log.borrow_mut().push(Call::Write(data.to_vec()));
            self.fail_write.clone().map_or(Ok(()), Err)
        }

        fn close(&mut self) -> Result<(), StreamError> {
            self.log.borrow_mut().push(Call::Close);
            self.fail_close.clone().map_or(Ok(()), Err)
        }
    }

    impl Drop for Logged {
        fn drop(&mut self) {
            self.log.borrow_mut().push(Call::Drop);
        }
    }

    // --- Rust streams, Rust callers ----------------------------------------------------

    #[test]
    fn rust_stream_takes_writes_of_every_size() {
        let mut stream = RustWriteStream::new(Vec::new());
        let mut expected = Vec::new();
        for len in SIZES {
            let data = pattern(len);
            stream.as_mut().write(&data).unwrap();
            expected.extend_from_slice(&data);
        }
        assert_eq!(stream.writer(), &expected);
        let mut boxed = RustWriteStream::new(Vec::new()).into_box();
        for len in SIZES {
            boxed.as_mut().write(&pattern(len)).unwrap();
        }
        assert_eq!(WriteStream::close(boxed), Ok(()));
    }

    #[test]
    fn every_owner_drops_the_writer_exactly_once() {
        let (writer, log) = logged();
        drop(RustWriteStream::new(writer));
        assert_eq!(*log.borrow(), [Call::Drop]);

        let (writer, log) = logged();
        drop(RustWriteStream::new(writer).into_box());
        assert_eq!(*log.borrow(), [Call::Drop]);

        let (writer, log) = logged();
        let mut stream = RustWriteStream::new(writer).into_box();
        stream.as_mut().write(b"x").unwrap();
        assert_eq!(WriteStream::close(stream), Ok(()));
        assert_eq!(
            *log.borrow(),
            [Call::Write(b"x".to_vec()), Call::Close, Call::Drop]
        );

        let (writer, log) = logged();
        let writer = RustWriteStream::new(writer).into_inner();
        assert!(log.borrow().is_empty());
        drop(writer);
        assert_eq!(*log.borrow(), [Call::Drop]);
    }

    /// Handles and direct writer access alternate through reborrows of the owner: every
    /// write through the header sees the writer's latest state.
    #[test]
    fn handles_and_writer_access_alternate() {
        let mut stream = RustWriteStream::new(Vec::new());
        for round in 0..64u8 {
            let mut handle = stream.as_mut();
            handle.write(&[round]).unwrap();
            handle.as_mut().write(&[round, round]).unwrap();
            let writer = stream.writer_mut();
            assert_eq!(writer.len(), 3);
            writer.clear();
            let shared = stream.as_ref();
            let _ = (shared, stream.writer());
        }
        assert!(stream.writer().is_empty());
    }

    /// A borrowed writer outlives the stream; the stream's members reach it only while
    /// the stream lives.
    #[test]
    fn borrowed_writer_outlives_the_stream() {
        let mut out = Vec::new();
        let mut stream = RustWriteStream::new(&mut out);
        stream.as_mut().write(b"borrowed").unwrap();
        drop(stream);
        out.push(b'!');
        assert_eq!(out, b"borrowed!");
    }

    /// A writer's error comes back with its code, class and message; the error is the
    /// thread's last libgit2 error too.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn writer_errors_come_back_whole() {
        let (mut writer, _log) = logged();
        writer.fail_write = Some(
            StreamError::new(ErrorClass::Callback, c"refused %s %n").with_code(ErrorCode::User),
        );
        writer.fail_close = Some(StreamError::new(ErrorClass::Os, c"x"));
        let mut stream = RustWriteStream::new(writer).into_box();
        let error = stream.as_mut().write(b"data").unwrap_err();
        assert_eq!(
            (error.code(), error.class(), error.message()),
            (
                Some(ErrorCode::User),
                Some(ErrorClass::Callback),
                c"refused %s %n"
            )
        );
        assert_eq!(last().message(), c"refused %s %n");
        let error = WriteStream::close(stream).unwrap_err();
        assert_eq!(
            (error.class(), error.message()),
            (Some(ErrorClass::Os), c"x")
        );
    }

    /// The first contact with libgit2 in a process can be a writer's error, raised from
    /// the trampoline: it pins the runtime rather than reach uninitialised thread state.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn writer_error_before_init_pins_the_runtime() {
        isolated(
            concat!(
                module_path!(),
                "::writer_error_before_init_pins_the_runtime"
            ),
            || {
                let (mut writer, _log) = logged();
                writer.fail_write = Some(StreamError::new(ErrorClass::Filter, c"first"));
                let mut stream = RustWriteStream::new(writer);
                let error = stream.as_mut().write(b"x").unwrap_err();
                assert_eq!(
                    (error.class(), error.message()),
                    (Some(ErrorClass::Filter), c"first")
                );
            },
        );
    }

    // --- C streams, Rust callers -------------------------------------------------------

    /// A blob stream written past its 2 MiB buffer, so it spills to its temporary file,
    /// then committed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_stream_is_written_and_committed() {
        let repo = TempRepo::new("committed");
        let mut stream = blob_stream(&repo, Some(c"file.txt"));
        let chunk = vec![0x5a; 65_537];
        let mut total = 0;
        while total <= 2 * 1024 * 1024 {
            stream.as_mut().write(&chunk).unwrap();
            total += chunk.len();
        }
        stream.as_mut().write(&[]).unwrap();
        assert_eq!(repo.stream_files(), 1);
        let id = commit_blob(stream).unwrap();
        assert_eq!(id.len(), 20);
        assert_eq!(repo.stream_files(), 0);
    }

    /// Dropping a blob stream frees it, written or not, through its own `free` member:
    /// LSan flags a leak, ASan a double free.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_stream_drops_unwritten_and_written() {
        let repo = TempRepo::new("dropped");
        drop(blob_stream(&repo, None));
        let mut stream = blob_stream(&repo, Some(c"hint"));
        for len in SIZES {
            stream.as_mut().write(&pattern(len)).unwrap();
        }
        assert_eq!(repo.stream_files(), 1);
        drop(stream);
        assert_eq!(repo.stream_files(), 0);
    }

    /// Closing consumes the owner: the closed stream is freed at once, so it can never be
    /// written (which would call the NULL write function of its zeroed file buffer).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_stream_close_consumes_it() {
        let repo = TempRepo::new("closed");
        let mut stream = blob_stream(&repo, None);
        stream.as_mut().write(b"discarded").unwrap();
        assert_eq!(WriteStream::close(stream), Ok(()));
        assert_eq!(repo.stream_files(), 0);
        assert_eq!(WriteStream::close(blob_stream(&repo, None)), Ok(()));
    }

    // --- Rust streams, C callers -------------------------------------------------------

    /// libgit2 writes into the target and closes it, never frees it; the owner frees it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_target_is_written_and_closed_by_c() {
        let (writer, log) = logged();
        let mut target = RustWriteStream::new(writer).into_box();
        for len in SIZES {
            let data = pattern(len);
            assert_eq!(stream_buffer(Filters::None, &data, &mut target), 0);
            assert_eq!(
                log.borrow_mut().drain(..).collect::<Vec<_>>(),
                [Call::Write(data), Call::Close]
            );
        }
        drop(target);
        assert_eq!(*log.borrow(), [Call::Drop]);
    }

    /// Behind libgit2's buffered `ident` stream, the target gets the filtered data once
    /// the chain closes; a failing write skips its close.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_target_behind_a_c_filter_stream() {
        let (writer, log) = logged();
        let mut target = RustWriteStream::new(writer).into_box();
        let rc = stream_buffer(Filters::IdentClean, b"a $Id: 1234 $ b", &mut target);
        assert_eq!(rc, 0);
        assert_eq!(
            log.borrow_mut().drain(..).collect::<Vec<_>>(),
            [Call::Write(b"a $Id$ b".to_vec()), Call::Close]
        );
        // No empty data: `ident` then computes `NULL + 0` in `git_str_is_binary` (UBSan),
        // a libgit2 defect. Its NULL buffer would reach the target as empty data
        // (`unit_tests::null_buffer_is_empty_only_when_len_is_zero`).
        drop(target);

        let (mut writer, log) = logged();
        writer.fail_write = Some(StreamError::new(ErrorClass::Filter, c"full"));
        let mut target = RustWriteStream::new(writer).into_box();
        let rc = stream_buffer(Filters::IdentClean, b"plain", &mut target);
        assert_eq!(rc, ErrorCode::Generic.raw());
        assert_eq!(last().message(), c"full");
        assert_eq!(
            log.borrow_mut().drain(..).collect::<Vec<_>>(),
            [Call::Write(b"plain".to_vec())]
        );
    }

    /// A Rust stream handed to libgit2 by a filter's `stream` callback: libgit2 writes
    /// into it, closes it and frees it, through its members.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_filter_stream_is_freed_by_c() {
        let (writer, log) = logged();
        let mut target = RustWriteStream::new(writer).into_box();
        let (made, dropped) = upper_streams();
        for (i, len) in SIZES.into_iter().enumerate() {
            let data = pattern(len);
            assert_eq!(stream_buffer(Filters::Upper, &data, &mut target), 0);
            assert_eq!(
                log.borrow_mut().drain(..).collect::<Vec<_>>(),
                [Call::Write(data.to_ascii_uppercase()), Call::Close]
            );
            // One stream per chain, freed by libgit2 before the call returns.
            assert_eq!(upper_streams(), (made + i + 1, dropped + i + 1));
        }

        // The target's failure travels back up through the Rust filter stream.
        let (mut writer, log) = logged();
        writer.fail_close = Some(StreamError::new(ErrorClass::Os, c"disk gone"));
        let mut target = RustWriteStream::new(writer).into_box();
        let rc = stream_buffer(Filters::Upper, b"abc", &mut target);
        assert_eq!(rc, ErrorCode::Generic.raw());
        assert_eq!(last().message(), c"disk gone");
        assert_eq!(
            log.borrow_mut().drain(..).collect::<Vec<_>>(),
            [Call::Write(b"ABC".to_vec()), Call::Close]
        );
    }

    /// A filter that cannot start its stream makes libgit2 close the target unwritten.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_target_closed_when_a_filter_fails_to_start() {
        let (writer, log) = logged();
        let mut target = RustWriteStream::new(writer).into_box();
        let rc = stream_buffer(Filters::Refused, b"never written", &mut target);
        assert_eq!(rc, ErrorCode::User.raw());
        assert_eq!(last().message(), c"refused");
        assert_eq!(
            log.borrow_mut().drain(..).collect::<Vec<_>>(),
            [Call::Close]
        );
        drop(target);
        assert_eq!(*log.borrow(), [Call::Drop]);
    }

    /// libgit2 keeps the target's pointer only during the call: afterwards the owner
    /// reborrows it freely, and writes again (a Rust stream accepts writes after close).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_keeps_no_pointer_past_the_call() {
        let (writer, log) = logged();
        let mut target = RustWriteStream::new(writer).into_box();
        assert_eq!(stream_buffer(Filters::Upper, b"x", &mut target), 0);
        target.as_mut().write(b"after").unwrap();
        assert_eq!(stream_buffer(Filters::None, b"y", &mut target), 0);
        assert_eq!(
            *log.borrow(),
            [
                Call::Write(b"X".to_vec()),
                Call::Close,
                Call::Write(b"after".to_vec()),
                Call::Write(b"y".to_vec()),
                Call::Close,
            ]
        );
    }

    // --- Threads -------------------------------------------------------------------------

    /// `RustWriteStream<W>` is `Send` for a `Send` writer: written on one thread, then
    /// another, then read back on the first.
    #[test]
    fn rust_stream_moves_between_threads() {
        let mut stream = RustWriteStream::new(Vec::new());
        stream.as_mut().write(b"main ").unwrap();
        let stream = thread::spawn(move || {
            stream.as_mut().write(b"worker").unwrap();
            stream
        })
        .join()
        .unwrap();
        assert_eq!(stream.into_inner(), b"main worker");
    }

    /// `RustWriteStream<W>` is `Sync` for a `Sync` writer: shared readers race nothing.
    #[test]
    fn rust_stream_is_shared_read_only() {
        let mut stream = RustWriteStream::new(pattern(4097));
        stream.as_mut().write(b"tail").unwrap();
        let stream = &stream;
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    assert_eq!(stream.writer().len(), 4101);
                    let _ = stream.as_ref();
                });
            }
        });
    }

    /// A shared writer behind an `Arc`, written through streams on several threads at once.
    #[test]
    fn streams_on_many_threads_share_a_writer() {
        struct Counting(Arc<AtomicUsize>);
        impl StreamWriter for Counting {
            fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
                self.0.fetch_add(data.len(), SeqCst);
                Ok(())
            }
        }
        let total = Arc::new(AtomicUsize::new(0));
        thread::scope(|s| {
            for _ in 0..4 {
                let mut stream = RustWriteStream::new(Counting(total.clone()));
                s.spawn(move || {
                    for len in SIZES {
                        stream.as_mut().write(&pattern(len)).unwrap();
                    }
                });
            }
        });
        assert_eq!(total.load(SeqCst), 4 * SIZES.iter().sum::<usize>());
    }

    /// libgit2 streams on several threads at once, each on its own repository, and Rust
    /// streams lent to libgit2 on each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_streams_on_many_threads() {
        init();
        thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = TempRepo::new(&format!("thread-{t}"));
                    let mut stream = blob_stream(&repo, None);
                    stream.as_mut().write(&pattern(4097 + t)).unwrap();
                    commit_blob(stream).unwrap();
                    let (writer, log) = logged();
                    let mut target = RustWriteStream::new(writer).into_box();
                    assert_eq!(stream_buffer(Filters::Upper, b"t", &mut target), 0);
                    assert_eq!(log.borrow().len(), 2);
                });
            }
        });
    }
}

// --- `git_submodule_ignore_t` --------------------------------------------

/// Every value in and around the named range, and the extremes, converts to a valid
/// variant or to `None`, and a variant always converts back to its own value.
#[test]
fn submodule_ignore_conversions_never_build_an_invalid_variant() {
    let values = (-260..260).chain([i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX]);
    let mut named = 0;
    for raw in values {
        let raw: ffi::git_submodule_ignore_t = raw;
        if let Some(v) = SubmoduleIgnore::from_raw(raw) {
            named += 1;
            assert_eq!(v.raw(), raw);
            // A match over every variant reads the discriminant: Miri rejects an
            // invalid one.
            let rank = match v {
                SubmoduleIgnore::Unspecified => 0,
                SubmoduleIgnore::None => 1,
                SubmoduleIgnore::Untracked => 2,
                SubmoduleIgnore::Dirty => 3,
                SubmoduleIgnore::All => 4,
                _ => unreachable!("no other variant"),
            };
            assert_eq!(rank, raw.max(0));
        } else {
            assert_eq!(SubmoduleIgnore::try_from(raw).unwrap_err().raw(), raw);
        }
    }
    assert_eq!(named, 5);
    let none: Option<SubmoduleIgnore> = SubmoduleIgnore::from_raw(0);
    assert!(none.is_none());
}

// --- `BranchType` ------------------------------------------------------------

#[test]
fn every_raw_value_converts_to_a_variant_or_none() {
    let edges = [
        u32::MAX,
        u32::MAX - 1,
        0x8000_0000,
        0x8000_0001,
        0x8000_0003,
    ];
    for raw in (0..=300).chain(edges) {
        match BranchType::from_raw(raw) {
            Some(kind) => {
                assert_eq!(kind.raw(), raw);
                assert!(matches!(
                    kind,
                    BranchType::Local | BranchType::Remote | BranchType::All
                ));
            }
            None => assert!(!(1..=3).contains(&raw)),
        }
        assert_eq!(BranchType::try_from(raw).ok(), BranchType::from_raw(raw));
    }
    assert_eq!(BranchType::All.raw(), ffi::GIT_BRANCH_ALL);
}

#[test]
fn branch_types_cross_threads() {
    fn send_sync<T: Send + Sync + Copy>() {}
    send_sync::<BranchType>();
    let kind = std::thread::spawn(|| BranchType::Local | BranchType::Remote)
        .join()
        .unwrap();
    assert_eq!(kind, BranchType::All);
}

mod signatures {
    //! UB tests for `git2/types.h`'s `git_signature`: `SignatureBox` built by every
    //! constructor and copy, `SignatureView` over borrowed strings, the handles both lend,
    //! and the `TimeRef` / `TimeMut` they project out of `when`.
    //!
    //! Every getter and setter runs on every holder, through reborrows, over strings up
    //! to a page long; replaced strings, copies and views outlive what they came from
    //! (ASan flags a use after free, LSan a leaked string, Miri the view's projections);
    //! libgit2 reads a signature while Rust holds borrowed strings out of it, and C's
    //! copies are read after Rust's reborrows (BSan); owners and views cross threads
    //! under the `Send` / `Sync` the crate asserts, and race the process's first
    //! initialisation (TSan). Handles that outlive their owner or strings, strings
    //! held across a setter, aliased exclusive handles and setters on the shared handle
    //! do not compile (`compile_fail/`). Only the view tests avoid libgit2, so only they
    //! run under Miri.

    use std::ffi::{CStr, CString};
    use std::sync::Barrier;
    use std::thread;

    use libgit2::libgit2::core::types::{
        Signature, SignatureBox, SignatureMut, SignatureRef, SignatureView, Time,
    };
    use libgit2::libgit2::raw::{OdbDataStr, OdbDataStrFree};
    use libgit2::libgit2::util::errors::ErrorClass;

    use crate::fixture::{init, isolated};

    /// Lengths up to just past a page.
    const LENS: [usize; 6] = [1, 2, 7, 63, 4096, 4097];

    /// `len` printable bytes, none trimmed or refused by `git_signature_new`.
    fn text(len: usize, seed: u8) -> CString {
        let bytes = (0..len)
            .map(|i| b'a' + ((i as u8).wrapping_add(seed) % 26))
            .collect::<Vec<_>>();
        CString::new(bytes).unwrap()
    }

    fn dup(s: &CStr) -> OdbDataStr {
        OdbDataStrFree::dup(s).unwrap()
    }

    /// Reads every field and every byte of both strings, as owned values.
    fn read(sig: SignatureRef<'_>) -> (CString, CString, Time) {
        let name = sig.name().unwrap();
        let email = sig.email().unwrap();
        assert_eq!(name.to_bytes().len(), name.count_bytes());
        let when = sig.when();
        let _ = (when.seconds(), when.offset_minutes(), when.sign());
        assert!(!format!("{sig:?}").is_empty());
        (name.to_owned(), email.to_owned(), when.to_time())
    }

    /// Exercises every setter through `m` and its reborrows.
    fn write_all(mut m: SignatureMut<'_>, name: &CStr, email: &CStr) {
        m.set_name(dup(name));
        let old = m.as_mut().replace_email(dup(email)).unwrap();
        assert!(old.as_bytes().len() <= 4097);
        m.when_mut().set_seconds(-1);
        m.as_mut().when_mut().set_offset_minutes(-899);
        m.when_mut().set_sign(b'x');
        assert_eq!(m.as_ref().when().sign(), b'x');
        m.set_when(Time::new(1, 2));
        assert_eq!(m.as_ref().name(), Some(name));
        assert_eq!(m.as_ref().email(), Some(email));
        assert!(!format!("{m:?}").is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owned_signatures_stay_in_bounds() {
        init();
        for (i, &len) in LENS.iter().enumerate() {
            let (name, email) = (text(len, i as u8), text(len, 7));
            let mut sig = Signature::new(&name, &email, i as i64, -60).unwrap();
            assert_eq!(
                read(sig.as_ref()),
                (name.clone(), email.clone(), Time::new(i as i64, -60))
            );
            write_all(sig.as_mut(), &email, &name);
            assert_eq!(read(sig.as_ref()).0, email);

            let line =
                CString::new([name.as_bytes(), b" <", email.as_bytes(), b"> 12 +0130"].concat())
                    .unwrap();
            let parsed = Signature::parse(&line).unwrap();
            assert_eq!(read(parsed.as_ref()).0, name);
        }
    }

    /// What a signature hands out survives it: a replaced string, a copy, a clone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handed_out_values_outlive_the_signature() {
        init();
        let mut sig = Signature::new(c"First", c"first@x", 1, 1).unwrap();
        let old_name = sig.as_mut().replace_name(dup(c"Second")).unwrap();
        let clone = sig.clone();
        let copy = sig.as_ref().to_signature().unwrap();
        let when = sig.as_ref().when().to_time();
        drop(sig);
        assert_eq!(old_name.as_c_str(), c"First");
        assert_eq!(read(clone.as_ref()).0, c"Second".to_owned());
        assert_eq!(read(copy.as_ref()).1, c"first@x".to_owned());
        assert_eq!(clone.as_ref(), copy.as_ref());
        assert_eq!(when, Time::new(1, 1));
        // Clones of clones, dropped in either order.
        let a = clone.clone();
        drop(clone);
        let b = a.clone();
        drop(a);
        assert_eq!(read(b.as_ref()).0, c"Second".to_owned());
    }

    /// Repeated replacement releases each string once (ASan: double free; LSan: leak).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn repeated_replacement_releases_each_string_once() {
        init();
        let mut sig = Signature::new(c"N", c"e@x", 0, 0).unwrap();
        let mut kept = Vec::new();
        for i in 0..64 {
            let s = text(1 + i % 9, i as u8);
            if i % 2 == 0 {
                sig.as_mut().set_name(dup(&s));
                sig.as_mut().set_email(dup(&s));
            } else {
                kept.push(sig.as_mut().replace_name(dup(&s)).unwrap());
                kept.push(sig.as_mut().replace_email(dup(&s)).unwrap());
            }
            assert_eq!(sig.as_ref().name(), Some(s.as_c_str()));
        }
        drop(sig);
        assert_eq!(kept.len(), 64);
        assert!(kept.iter().all(|s| !s.as_bytes().is_empty()));
    }

    /// Views read borrowed strings in place, and copies of them own theirs: the copy
    /// outlives the strings the view borrowed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn view_copies_outlive_the_borrowed_strings() {
        init();
        for (i, &len) in LENS.iter().enumerate() {
            let (name, email) = (text(len, i as u8), text(len, 3));
            let copy = {
                let view = SignatureView::new(&name, &email, Time::new(-5, 90));
                let r = view.as_ref();
                assert_eq!(read(r), (name.clone(), email.clone(), Time::new(-5, 90)));
                r.to_signature().unwrap()
            };
            let expected = (name.clone(), email.clone(), Time::new(-5, 90));
            drop((name, email));
            assert_eq!(read(copy.as_ref()), expected);
        }
    }

    /// Views alone: no libgit2, so this runs under Miri too.
    #[test]
    fn views_read_in_place() {
        let name = c"View".to_owned();
        let email = c"view@x".to_owned();
        // The clone outlives the view it was made from.
        let clone = SignatureView::new(&name, &email, Time::new(3, -30)).clone();
        let r = clone.as_ref();
        assert_eq!(r.name(), Some(name.as_c_str()));
        assert_eq!(r.email(), Some(email.as_c_str()));
        assert_eq!(r.when().to_time(), Time::new(3, -30));
        assert_eq!(format!("{clone:?}"), format!("{r:?}"));
        let empty = SignatureView::new(c"", c"", Time::new(0, 0));
        assert_eq!(empty.as_ref().name(), Some(c""));
        assert_eq!(empty.as_ref().when().sign(), b'+');
    }

    /// libgit2 reads a signature (copying it, writing it into a commit through the
    /// copy) while Rust holds strings borrowed out of it, and Rust reads C's copies
    /// after reborrowing the original (BSan: foreign reads under Rust borrows).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2_reads_while_strings_are_borrowed() {
        init();
        let mut sig = Signature::parse(c"Held <held@x> 9 -0100").unwrap();
        {
            let r = sig.as_ref();
            let name = r.name().unwrap();
            let email = r.email().unwrap();
            let copies: Vec<SignatureBox> = (0..4).map(|_| sig.try_clone().unwrap()).collect();
            let more = r.to_signature().unwrap();
            assert_eq!(name, c"Held");
            assert_eq!(email, c"held@x");
            for c in &copies {
                assert_eq!(c.as_ref(), more.as_ref());
            }
        }
        let mut m = sig.as_mut();
        m.when_mut().set_seconds(10);
        let copy = m.as_ref().to_signature().unwrap();
        m.set_name(dup(c"After"));
        assert_eq!(copy.as_ref().name(), Some(c"Held"));
        assert_eq!(sig.as_ref().name(), Some(c"After"));
        assert_eq!(copy.as_ref().when().seconds(), 10);
    }

    /// Constructor failures hand out nothing, and leave no allocation behind (LSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_constructions_leave_nothing() {
        init();
        for (name, email) in [(c"a<", c"b"), (c"a", c">"), (c"", c"b"), (c"a", c" ; ")] {
            let error = Signature::new(name, email, 0, 0).unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
        }
        for line in [c"", c"A", c"A >a< 1", c"A <a@b> x"] {
            assert!(Signature::parse(line).is_err());
        }
    }

    /// `Signature::now`, alone in a child: libgit2 reads the local zone through glibc's
    /// `mktime` and `gmtime_r`, which serialise on a lock of glibc's own that TSan
    /// cannot see, so two concurrent calls (from any two tests) read as a race on
    /// glibc's copy of `TZ`. The calls are sound; they only run where nothing else does.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn now_stays_in_bounds() {
        isolated(concat!(module_path!(), "::now_stays_in_bounds"), || {
            for (i, &len) in LENS.iter().enumerate() {
                let (name, email) = (text(len, i as u8), text(len, 7));
                let mut now = Signature::now(&name, &email).unwrap();
                assert_eq!(read(now.as_ref()).1, email);
                write_all(now.as_mut(), &email, &name);
            }
            for (name, email) in [(c"a<", c"b"), (c"a", c">"), (c"", c"b"), (c"a", c" ; ")] {
                let error = Signature::now(name, email).unwrap_err();
                assert_eq!(error.class(), Some(ErrorClass::Invalid));
            }
        });
    }

    /// Owners move to other threads and drop there; shared handles and views are read
    /// from many at once, and copied concurrently (TSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_cross_threads() {
        init();
        let sig = Signature::new(c"Shared", c"shared@x", 1, 60).unwrap();
        let name = c"Viewed".to_owned();
        let view = SignatureView::new(&name, c"viewed@x", Time::new(2, -60));
        let owned: Vec<SignatureBox> = thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (sig, view) = (&sig, &view);
                    s.spawn(move || {
                        let r: SignatureRef<'_> = if i % 2 == 0 {
                            sig.as_ref()
                        } else {
                            view.as_ref()
                        };
                        read(r);
                        let copy = if i % 4 < 2 {
                            r.to_signature().unwrap()
                        } else {
                            sig.clone()
                        };
                        read(copy.as_ref());
                        copy
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        // Moved out, written and dropped on threads other than their makers'.
        let handles: Vec<_> = owned
            .into_iter()
            .map(|mut copy| {
                thread::spawn(move || {
                    write_all(copy.as_mut(), c"moved", c"moved@x");
                    drop(copy);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut sig = sig;
        let m = sig.as_mut();
        thread::scope(|s| {
            s.spawn(move || write_all(m, c"m", c"m@x"));
        });
        assert_eq!(sig.as_ref().name(), Some(c"m"));
    }

    /// Before `git_libgit2_init`, constructors and copies pin the runtime first, and
    /// views need nothing from libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_before_init() {
        isolated(concat!(module_path!(), "::signatures_before_init"), || {
            let view = SignatureView::new(c"Early", c"early@x", Time::new(0, 0));
            assert_eq!(view.as_ref().name(), Some(c"Early"));
            let copy = view.as_ref().to_signature().unwrap();
            assert_eq!(copy.as_ref(), view.as_ref());
        });
    }

    /// Views read and dropped, and copies made of them, while another thread's first
    /// constructor initialises libgit2: every path into libgit2 is ordered after the
    /// initialisation by the runtime pin (TSan). Alone in a child, so the
    /// initialisation is really the process's first.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_do_not_race_the_first_init() {
        isolated(
            concat!(module_path!(), "::signatures_do_not_race_the_first_init"),
            || {
                let start = Barrier::new(3);
                thread::scope(|s| {
                    s.spawn(|| {
                        start.wait();
                        for _ in 0..64 {
                            let view = SignatureView::new(c"v", c"v@x", Time::new(1, 1));
                            read(view.as_ref());
                        }
                    });
                    s.spawn(|| {
                        start.wait();
                        let view = SignatureView::new(c"c", c"c@x", Time::new(1, 1));
                        let copy = view.as_ref().to_signature().unwrap();
                        drop(copy);
                    });
                    s.spawn(|| {
                        start.wait();
                        let sig = Signature::new(c"first", c"first@x", 0, 0).unwrap();
                        drop(sig.clone());
                    });
                });
            },
        );
    }
}
