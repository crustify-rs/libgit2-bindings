//! UB tests for `git2/sys/repository.h`.

/// The options `NewOptions` (`git_repository_new_options`) and its handles.
///
/// A `NewOptions` is a plain 8-byte value. Its handles project each field out of a
/// pointer, so these tests exercise every getter and setter on every holder Rust has
/// for one (a local, a heap slot, an element of a run that moves when it grows, a
/// copy), through reborrows, and across threads under the `Send` / `Sync` the crate
/// asserts (TSan). A misaligned projection or a write through the shared handle would
/// be flagged by Miri and ASan/UBSan. The only safe way in from a raw value is
/// `try_from`, which must refuse an object id type libgit2 does not name (libgit2
/// asserts on one), and the setter only takes `Option<OidType>`; handles that outlive
/// their value, aliased handles and setters on the shared handle do not compile
/// (`compile_fail/`, the type's doctests). `NewOptions::new` calls libgit2
/// (`git_repository_new_options_init`, which copies a template into the value): it runs
/// from many threads at once, and before anything initialised libgit2, for ASan and
/// TSan. libgit2 reads the options in `git_repository_new_ext` (the fixture, and
/// `Repository::with_oid_type`), keeping no pointer to them past the call, so there is
/// no foreign write or retained pointer to race (BSan). Everything else also runs
/// under Miri, from values built by conversion.
mod new_options {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::hint::black_box;
    use std::sync::Mutex;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::Repository;
    use libgit2::libgit2::repository::sys::repository::{NewOptions, NewOptionsMut, NewOptionsRef};

    use crate::fixture::repository_options::{new_ext, oid_type};
    use crate::fixture::{init, isolated};

    const OIDS: [Option<OidType>; 3] = [None, Some(OidType::Sha1), Some(OidType::Sha256)];

    /// Every value around the declared ones, each single bit, and a coarse stride
    /// across the whole type.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain((0..32).map(|k| 1u32 << k))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// The options as C would build them, without calling it: Miri runs these.
    fn options(version: u32, oid_type: Option<OidType>) -> NewOptions {
        let raw = ffi::git_repository_new_options {
            version,
            oid_type: oid_type.map_or(0, OidType::raw),
        };
        NewOptions::try_from(raw).unwrap()
    }

    fn read(r: NewOptionsRef<'_>) -> (u32, Option<OidType>) {
        (r.version(), r.oid_type())
    }

    fn write(mut m: NewOptionsMut<'_>, version: u32, oid_type: Option<OidType>) {
        m.set_version(version);
        m.set_oid_type(oid_type);
    }

    #[test]
    fn try_from_refuses_every_unnamed_oid_type() {
        for raw in probes() {
            let opts = black_box(ffi::git_repository_new_options {
                version: raw,
                oid_type: raw,
            });
            match NewOptions::try_from(opts) {
                Ok(o) => {
                    // Reading the type back validates what was stored.
                    let kind = o.as_ref().oid_type();
                    assert_eq!(kind.map_or(0, OidType::raw), raw);
                    assert_eq!(o.as_ref().version(), raw);
                }
                Err(e) => {
                    assert_eq!(e.raw(), raw);
                    assert!(raw != 0 && OidType::from_raw(raw).is_none());
                }
            }
        }
    }

    #[test]
    fn every_field_round_trips_on_every_holder() {
        let mut local = options(1, None);
        let mut boxed = Box::new(options(1, None));
        let mut run = vec![options(1, None)];
        for (i, version) in probes().step_by(7).enumerate() {
            let kind = OIDS[i % 3];
            write(local.as_mut(), version, kind);
            write(NewOptions::as_mut(&mut boxed), version, kind);
            // Growing the run moves its elements: every handle is taken afresh.
            run.push(options(1, None));
            let last = run.len() - 1;
            write(run[last].as_mut(), version, kind);
            for opts in [&local, &*boxed, &run[last]] {
                assert_eq!(read(opts.as_ref()), (version, kind));
                assert_eq!(read(opts.as_ref().to_options().as_ref()), (version, kind));
                let raw = ffi::git_repository_new_options::from(*opts);
                assert_eq!(
                    (raw.version, raw.oid_type),
                    (version, kind.map_or(0, OidType::raw))
                );
            }
        }
    }

    #[test]
    fn reborrows_keep_one_writer() {
        let mut opts = options(1, None);
        let mut m = opts.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_oid_type(Some(OidType::Sha256));
            assert_eq!(inner.as_ref().oid_type(), Some(OidType::Sha256));
        }
        let shared = m.as_ref();
        let copy = shared;
        assert_eq!(read(copy), (1, Some(OidType::Sha256)));
        assert!(!format!("{m:?}").is_empty());
        m.set_version(7);
        assert_eq!(read(opts.as_ref()), (7, Some(OidType::Sha256)));
    }

    #[test]
    fn copies_are_independent() {
        let c = {
            let a = options(1, Some(OidType::Sha1));
            let mut b = a;
            b.as_mut().set_oid_type(None);
            assert_eq!(read(a.as_ref()), (1, Some(OidType::Sha1)));
            assert_eq!(read(b.as_ref()), (1, None));
            let c = a.as_ref().to_options();
            assert_eq!(c, a);
            c
        };
        // The copy outlives the value it was read from.
        assert_eq!(read(c.as_ref()), (1, Some(OidType::Sha1)));
    }

    #[test]
    fn equality_and_hash_follow_the_fields() {
        let hash = |o: &NewOptions| {
            let mut h = DefaultHasher::new();
            o.hash(&mut h);
            h.finish()
        };
        let a = options(1, Some(OidType::Sha256));
        let b = options(1, Some(OidType::Sha256));
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));
        assert_eq!(format!("{a:?}"), format!("{:?}", b.as_ref()));
        assert_ne!(a, options(2, Some(OidType::Sha256)));
        assert_ne!(a, options(1, None));
        assert_ne!(options(1, None), options(1, Some(OidType::Sha1)));
    }

    #[test]
    #[cfg_attr(miri, ignore = "slow: many interleavings")]
    fn shared_handles_read_across_threads() {
        let opts = options(1, Some(OidType::Sha256));
        let r = opts.as_ref();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    for _ in 0..1000 {
                        assert_eq!(read(r), (1, Some(OidType::Sha256)));
                    }
                });
            }
        });
    }

    #[test]
    fn values_and_exclusive_handles_move_across_threads() {
        let mut opts = options(1, None);
        thread::scope(|s| {
            let m = opts.as_mut();
            s.spawn(move || write(m, 1, Some(OidType::Sha1)))
                .join()
                .unwrap();
        });
        assert_eq!(read(opts.as_ref()), (1, Some(OidType::Sha1)));
        let moved = thread::spawn(move || {
            let mut opts = opts;
            opts.as_mut().set_version(9);
            opts
        })
        .join()
        .unwrap();
        assert_eq!(read(moved.as_ref()), (9, Some(OidType::Sha1)));
    }

    #[test]
    fn locked_options_have_one_writer_at_a_time() {
        let opts = Mutex::new(options(0, None));
        let rounds = if cfg!(miri) { 10 } else { 500 };
        thread::scope(|s| {
            for t in 0..4 {
                let opts = &opts;
                s.spawn(move || {
                    for _ in 0..rounds {
                        let mut guard = opts.lock().unwrap();
                        let n = guard.as_ref().version();
                        let mut m = guard.as_mut();
                        m.set_version(n + 1);
                        m.set_oid_type(OIDS[t % 3]);
                    }
                });
            }
        });
        assert_eq!(opts.lock().unwrap().as_ref().version(), 4 * rounds);
    }

    /// `new` copies libgit2's template into a fresh value on each call: concurrent
    /// calls share nothing for TSan to flag, and each result is the default.
    fn new_from_many_threads() {
        thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        (0..100)
                            .map(|_| read(NewOptions::new().as_ref()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                assert!(
                    h.join()
                        .unwrap()
                        .into_iter()
                        .all(|o| o == (NewOptions::VERSION, None))
                );
            }
        });
        assert_eq!(NewOptions::default(), NewOptions::new());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_races_nothing_across_threads() {
        new_from_many_threads();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_needs_no_initialised_libgit2() {
        // Before `git_libgit2_init` libgit2's allocator and error state are unusable
        // (see `util/sys/errors.rs`): `new` touches neither.
        isolated(
            concat!(module_path!(), "::new_needs_no_initialised_libgit2"),
            new_from_many_threads,
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_ext_reads_the_options_on_any_thread() {
        init();
        thread::scope(|s| {
            for kind in OIDS {
                s.spawn(move || {
                    let mut opts = NewOptions::new();
                    opts.as_mut().set_oid_type(kind);
                    let (rc, repo) = new_ext(&opts);
                    assert_eq!(rc, 0);
                    assert_eq!(oid_type(&repo.unwrap()), kind.unwrap_or_default().raw());
                    // The options are untouched and reusable once the call is done.
                    assert_eq!(read(opts.as_ref()), (NewOptions::VERSION, kind));
                    let (rc, repo) = new_ext(&opts);
                    assert_eq!(rc, 0);
                    drop(repo);
                    for version in [0, 2, u32::MAX] {
                        opts.as_mut().set_version(version);
                        let (rc, repo) = new_ext(&opts);
                        assert_eq!((rc, repo.is_none()), (-1, true), "version {version}");
                    }
                });
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn with_oid_type_builds_its_options_safely() {
        init();
        for kind in [OidType::Sha1, OidType::Sha256] {
            let repo = Repository::with_oid_type(kind).unwrap();
            assert_eq!(oid_type(&repo), kind.raw());
        }
    }
}
