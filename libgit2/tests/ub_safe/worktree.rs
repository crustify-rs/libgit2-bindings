//! UB tests for `git2/worktree.h`.

/// The pruning options: the enum `PruneFlag`, the flag set `PruneFlags`, and the
/// `PruneOptions` value (`git_worktree_prune_options`) with its handles.
///
/// `PruneFlag` is a fieldless enum over C's `git_worktree_prune_t`. The only safe way
/// in from a raw value is `from_raw` / `try_from`, which must reject every value
/// without a variant rather than produce an invalid discriminant (which Miri and
/// UBSan would flag on the next read); an `as` cast to it does not compile
/// (`compile_fail/`). `PruneFlags` keeps any `u32`, so its operations must stay in
/// range for every bit pattern.
///
/// A `PruneOptions` is a plain 8-byte value. Its handles project each field out of a
/// pointer, so these tests exercise every getter and setter on every holder Rust has
/// for one (a local, a heap slot, an element of a run that moves when it grows, a
/// copy), through reborrows, and across threads under the `Send` / `Sync` the crate
/// asserts (TSan). A misaligned projection or a write through the shared handle would
/// be flagged by Miri and ASan/UBSan. Handles that outlive their value, aliased
/// handles and setters on the shared handle do not compile (the type's
/// `compile_fail` doctests). Only `PruneOptions::new` calls libgit2
/// (`git_worktree_prune_options_init`, which copies a template into the value): it
/// runs from many threads at once, and before anything initialised libgit2, for
/// ASan and TSan; everything else also runs under Miri, from values built by
/// conversion. libgit2 keeps no pointer to the options past a call, so there is no
/// foreign write or retained pointer to race (BSan).
mod prune_options {
    use std::collections::HashSet;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::hint::black_box;
    use std::sync::Mutex;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::repository::worktree::{
        PruneFlag, PruneFlags, PruneOptions, PruneOptionsMut, PruneOptionsRef,
    };

    use crate::fixture::isolated;

    /// Every value around the declared ones, each single bit, and a coarse stride
    /// across the whole type.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain((0..32).map(|k| 1u32 << k))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// Bit patterns for `PruneFlags`: each single bit, its complement, and the
    /// extremes.
    fn bit_patterns() -> impl Iterator<Item = u32> {
        (0..32)
            .flat_map(|k| [1 << k, !(1 << k)])
            .chain([0, u32::MAX, 0b111, 0x8000_0000])
    }

    /// The options as C would build them, without calling it: Miri runs these.
    fn options(version: u32, flags: u32) -> PruneOptions {
        PruneOptions::from(ffi::git_worktree_prune_options { version, flags })
    }

    fn read(r: PruneOptionsRef<'_>) -> (u32, u32) {
        (r.version(), r.flags().bits())
    }

    fn write(mut m: PruneOptionsMut<'_>, version: u32, flags: u32) {
        m.set_version(version);
        m.set_flags(PruneFlags::from_bits_retain(flags));
    }

    #[test]
    fn flag_from_raw_yields_only_valid_flags() {
        let mut seen = HashSet::new();
        for raw in probes() {
            // `black_box` keeps the conversion from being folded at compile time.
            if let Some(flag) = PruneFlag::from_raw(black_box(raw)) {
                // Reading the discriminant back validates it.
                assert_eq!(flag.raw(), raw);
                assert!(!format!("{flag:?}").is_empty());
                seen.insert(flag);
            } else {
                assert_eq!(PruneFlag::try_from(raw).map_err(|e| e.raw()), Err(raw));
            }
        }
        assert_eq!(seen.len(), PruneFlag::ALL.len());
    }

    #[test]
    fn option_niche_holds_for_every_value() {
        // `Option<PruneFlag>` stores `None` in a value no variant uses: a rejected
        // value must never alias a variant, nor a variant read as `None`.
        for raw in probes() {
            let flag = black_box(PruneFlag::from_raw(raw));
            assert_eq!(flag.map(PruneFlag::raw).unwrap_or(raw), raw);
        }
    }

    #[test]
    fn flags_stay_in_range_for_every_bit_pattern() {
        for bits in bit_patterns() {
            let mut set = PruneFlags::from_bits_retain(black_box(bits));
            assert_eq!(set.bits(), bits);
            let named: Vec<_> = set.iter().collect();
            assert!(named.len() <= PruneFlag::ALL.len());
            assert!(named.iter().all(|&f| set.contains(f)));
            assert_eq!(set.into_iter().count(), named.len());
            assert!(!format!("{set:?}").is_empty());
            for flag in PruneFlag::ALL {
                set.insert(flag);
                assert!(set.contains(flag));
                set.remove(flag);
                assert!(!set.contains(flag));
            }
            assert_eq!(set.bits(), set.unnamed_bits());
            set |= PruneFlags::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
        }
    }

    #[test]
    fn every_field_round_trips_on_every_holder() {
        let mut local = options(1, 0);
        let mut boxed = Box::new(options(1, 0));
        let mut run = vec![options(1, 0)];
        for (i, bits) in bit_patterns().enumerate() {
            let version = [0, 1, 2, u32::MAX][i % 4];
            write(local.as_mut(), version, bits);
            write(PruneOptions::as_mut(&mut boxed), version, bits);
            // Growing the run moves its elements: every handle is taken afresh.
            run.push(options(1, 0));
            let last = run.len() - 1;
            write(run[last].as_mut(), version, bits);
            for opts in [&local, &*boxed, &run[last]] {
                assert_eq!(read(opts.as_ref()), (version, bits));
                assert_eq!(read(opts.as_ref().to_options().as_ref()), (version, bits));
            }
        }
    }

    #[test]
    fn reborrows_keep_one_writer() {
        let mut opts = options(1, 0);
        let mut m = opts.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_flags(PruneFlag::Locked.into());
            assert_eq!(inner.as_ref().flags(), PruneFlag::Locked.into());
        }
        let shared = m.as_ref();
        let copy = shared;
        assert_eq!(read(copy), (1, 2));
        assert!(!format!("{m:?}").is_empty());
        m.set_version(7);
        assert_eq!(read(opts.as_ref()), (7, 2));
    }

    #[test]
    fn copies_are_independent() {
        let c = {
            let a = options(1, 4);
            let mut b = a;
            b.as_mut().set_flags(PruneFlags::empty());
            assert_eq!(read(a.as_ref()), (1, 4));
            assert_eq!(read(b.as_ref()), (1, 0));
            let c = a.as_ref().to_options();
            assert_eq!(c, a);
            c
        };
        // The copy outlives the value it was read from.
        assert_eq!(read(c.as_ref()), (1, 4));
    }

    #[test]
    fn raw_values_convert_both_ways() {
        for bits in bit_patterns() {
            let opts = options(u32::MAX, bits);
            assert_eq!(read(opts.as_ref()), (u32::MAX, bits));
            let back = ffi::git_worktree_prune_options::from(opts);
            assert_eq!((back.version, back.flags), (u32::MAX, bits));
        }
    }

    #[test]
    fn equality_and_hash_follow_the_fields() {
        let hash = |o: &PruneOptions| {
            let mut h = DefaultHasher::new();
            o.hash(&mut h);
            h.finish()
        };
        let a = options(1, 3);
        let b = options(1, (PruneFlag::Valid | PruneFlag::Locked).bits());
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));
        assert_eq!(format!("{a:?}"), format!("{:?}", b.as_ref()));
        assert_ne!(a, options(2, 3));
        assert_ne!(a, options(1, 3 | 1 << 31));
    }

    #[test]
    #[cfg_attr(miri, ignore = "slow: many interleavings")]
    fn shared_handles_read_across_threads() {
        let opts = options(1, 5);
        let r = opts.as_ref();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    for _ in 0..1000 {
                        assert_eq!(read(r), (1, 5));
                    }
                });
            }
        });
    }

    #[test]
    fn values_and_exclusive_handles_move_across_threads() {
        let mut opts = options(1, 0);
        thread::scope(|s| {
            let m = opts.as_mut();
            s.spawn(move || write(m, 1, 6)).join().unwrap();
        });
        assert_eq!(read(opts.as_ref()), (1, 6));
        let moved = thread::spawn(move || {
            let mut opts = opts;
            opts.as_mut().set_version(9);
            opts
        })
        .join()
        .unwrap();
        assert_eq!(read(moved.as_ref()), (9, 6));
    }

    #[test]
    fn locked_options_have_one_writer_at_a_time() {
        let opts = Mutex::new(options(0, 0));
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
                        m.set_flags(PruneFlags::from_bits_retain(t));
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
                            .map(|_| read(PruneOptions::new().as_ref()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                assert!(
                    h.join()
                        .unwrap()
                        .into_iter()
                        .all(|o| o == (PruneOptions::VERSION, 0))
                );
            }
        });
        assert_eq!(PruneOptions::default(), PruneOptions::new());
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
}
