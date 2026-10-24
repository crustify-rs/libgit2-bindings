//! UB tests for `git2/worktree.h`.

/// The pruning options: the enum `PruneFlag`, the flag set `PruneFlags`, and the
/// `PruneOptions` value (`git_worktree_prune_options`) with its handles.
///
/// `PruneFlag` is a fieldless enum over C's `git_worktree_prune_t`. The only safe way
/// in from a raw value is `from_raw` / `try_from`, which must reject every value
/// without a variant rather than produce an invalid discriminant (which Miri and
/// `UBSan` would flag on the next read); an `as` cast to it does not compile
/// (`compile_fail/`). `PruneFlags` keeps any `u32`, so its operations must stay in
/// range for every bit pattern.
///
/// A `PruneOptions` is a plain 8-byte value. Its handles project each field out of a
/// pointer, so these tests exercise every getter and setter on every holder Rust has
/// for one (a local, a heap slot, an element of a run that moves when it grows, a
/// copy), through reborrows, and across threads under the `Send` / `Sync` the crate
/// asserts (`TSan`). A misaligned projection or a write through the shared handle would
/// be flagged by Miri and ASan/UBSan. Handles that outlive their value, aliased
/// handles and setters on the shared handle do not compile (the type's
/// `compile_fail` doctests). Only `PruneOptions::new` calls libgit2
/// (`git_worktree_prune_options_init`, which copies a template into the value): it
/// runs from many threads at once, and before anything initialised libgit2, for
/// `ASan` and `TSan`; everything else also runs under Miri, from values built by
/// conversion. libgit2 keeps no pointer to the options past a call, so there is no
/// foreign write or retained pointer to race (`BSan`).
mod prune_options {
    use std::collections::HashSet;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::hint::black_box;
    use std::sync::Mutex;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::worktree::{
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
                assert_ne!(format!("{flag:?}"), "");
                seen.insert(flag);
            } else {
                assert_eq!(
                    PruneFlag::try_from(raw)
                        .map_err(libgit2::libgit2::src::libgit2::worktree::UnnamedPruneFlag::raw),
                    Err(raw)
                );
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
            assert_eq!(flag.map_or(raw, PruneFlag::raw), raw);
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
            assert_ne!(format!("{set:?}"), "");
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
        assert_ne!(format!("{m:?}"), "");
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
    /// calls share nothing for `TSan` to flag, and each result is the default.
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

/// UB tests for `git2/worktree.h`: the `WorktreeBox` owner of a `git_worktree`, the
/// handles it lends, and the `git_worktree_free` policy.
///
/// Worktrees come from `Worktree::lookup`, and are read and locked through their
/// handles. Each box must release its struct and six strings exactly once (`ASan`: double
/// free, use after free; `LSan`: leak), on whichever thread drops it, and shared boxes
/// must race nothing (`TSan`). libgit2 writes a worktree in place (`git_worktree_lock`)
/// between Rust reborrows of its handles (`BSan`). Handles that outlive their box and
/// aliased exclusive handles do not compile (doctests on `WorktreeBox`); the box is not
/// `Clone` (doctest). Every test calls libgit2, so none runs under Miri.
mod worktree_box {
    use std::sync::Barrier;
    use std::thread;

    use libgit2::libgit2::src::libgit2::worktree::{
        Worktree, WorktreeBox, WorktreeFree, WorktreeMut, WorktreeRef,
    };

    use crate::fixture::worktree::WorktreeRepo;

    #[test]
    fn worktrees_cross_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Worktree>();
        send_sync::<WorktreeBox>();
        send_sync::<WorktreeRef<'_>>();
        send_sync::<WorktreeMut<'_>>();
        send_sync::<WorktreeFree>();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_release_everything() {
        let repo = WorktreeRepo::new("release");
        repo.add("a");
        repo.add("b");
        for _ in 0..16 {
            let a = repo.lookup(c"a");
            let b = repo.lookup(c"b");
            assert_eq!(a.as_ref().name(), c"a");
            assert_eq!(b.as_ref().name(), c"b");
            assert_ne!(a.as_ref().path(), b.as_ref().path());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn replaced_box_is_released_once() {
        let repo = WorktreeRepo::new("replace");
        repo.add("a");
        repo.add("b");
        let mut w = repo.lookup(c"a");
        assert_eq!(w.as_ref().name(), c"a");
        // The old worktree drops here, the new one at the end of the scope.
        w = repo.lookup(c"b");
        assert_eq!(w.as_ref().name(), c"b");
        let mut slot = Some(w);
        assert_eq!(
            slot.take().map(|w| w.as_ref().name().to_owned()),
            Some(c"b".to_owned())
        );
        assert!(slot.is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::no_effect_underscore_binding,
        reason = "the copy itself is what the test exercises"
    )]
    fn worktree_outlives_its_repository() {
        let repo = WorktreeRepo::new("outlive");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let expected = w.as_ref().path().to_owned();
        // The repository is freed and its directory removed: the worktree holds copies.
        repo.close();
        assert_eq!(w.as_ref().name(), c"wt");
        assert_eq!(w.as_ref().path(), &*expected);
        let r = w.as_ref();
        let _copy = r;
        drop(w);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn box_moves_to_another_thread_and_drops_there() {
        let repo = WorktreeRepo::new("send");
        let names = [c"w0", c"w1", c"w2", c"w3"];
        for n in names {
            repo.add(n.to_str().unwrap());
        }
        let boxes: Vec<WorktreeBox> = names.iter().map(|n| repo.lookup(n)).collect();
        let handles: Vec<_> = boxes
            .into_iter()
            .map(|w| thread::spawn(move || w.as_ref().name().to_owned()))
            .collect();
        let read: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(read, names.map(ToOwned::to_owned));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_box_reads_race_nothing() {
        let repo = WorktreeRepo::new("sync");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let expected = (w.as_ref().name().to_owned(), w.as_ref().path().to_owned());
        let start = Barrier::new(4);
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    start.wait();
                    // Shared handles and C reads through the same box, concurrently.
                    let r = w.as_ref();
                    for _ in 0..32 {
                        assert_eq!((r.name(), r.path()), (&*expected.0, &*expected.1));
                        assert_eq!(r.is_locked(), Ok(false));
                    }
                });
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_writes_between_reborrows() {
        let repo = WorktreeRepo::new("reborrow");
        repo.add("wt");
        let mut w = repo.lookup(c"wt");
        for round in 0..4 {
            {
                // An exclusive handle and its reborrows, then a C write through the box.
                let mut m = w.as_mut();
                let inner = m.as_mut();
                let _shared = inner.as_ref();
            }
            w.as_mut().lock(Some(c"held")).unwrap();
            // A second lock is refused without writing.
            assert!(w.as_mut().lock(Some(c"again")).is_err());
            let r = w.as_ref();
            assert_eq!(r.is_locked(), Ok(true));
            assert_eq!(w.as_mut().unlock(), Ok(true));
            // Unlocking an unlocked worktree reports `false` and writes nothing.
            assert_eq!(w.as_mut().unlock(), Ok(false));
            assert_eq!(w.as_ref().is_locked(), Ok(false), "round {round}");
            assert_eq!(w.as_ref().name(), c"wt");
        }
    }
}

/// The worktree creation options: `AddOptions` (`git_worktree_add_options`) with its
/// handles, the borrowed branch reference and the embedded checkout options.
///
/// An `AddOptions<'o>` is a value Rust holds, never allocated or freed by libgit2; its
/// handles project each field out of a pointer, and the checkout options are reached in
/// place. These tests exercise every getter and setter on every holder (a local, a heap
/// slot, an element of a run that moves when it grows), through reborrows, then hand
/// the options to `git_worktree_add` (through `Worktree::add`), which copies them, reads the
/// borrowed reference and the checkout options' borrowed strings, and keeps no pointer
/// to any of them. A misaligned projection, a stale reference pointer or a write through
/// the shared handle would be flagged by Miri and ASan/UBSan; the reference and the
/// options must be released exactly once, in either order allowed (`LSan`, `ASan`). Options
/// that outlive their reference or its repository, an exclusive handle to a borrowed
/// reference, a shrunk exclusive handle, `Send` and `Clone` do not compile (the type's
/// `compile_fail` doctests). The options are neither `Send` nor `Sync`, so each thread
/// builds its own: `AddOptions::new` (a template copy) runs from many threads at once
/// and before anything initialised libgit2 (`ASan`, `TSan`). libgit2 only reads the options
/// during a call and writes nothing through them, so Rust reborrows between calls race
/// no foreign write (`BSan`). Every test calls libgit2, so none runs under Miri; the
/// accessors run there in the crate's unit tests, on a template built in Rust.
mod add_options {
    use std::ffi::CString;
    use std::hint::black_box;
    use std::{fs, thread};

    use libgit2::libgit2::src::libgit2::worktree::{AddOptions, AddOptionsMut, AddOptionsRef};

    use crate::fixture::isolated;
    use crate::fixture::worktree_add::AddRepo;

    /// The plain fields, as the getters read them.
    fn read(r: AddOptionsRef<'_, '_>) -> (u32, bool, bool, bool, u32, u32) {
        let checkout = r.checkout_options();
        (
            r.version(),
            r.lock(),
            r.checkout_existing(),
            r.reference().is_some(),
            checkout.strategy(),
            checkout.dir_mode(),
        )
    }

    fn write(mut m: AddOptionsMut<'_, '_>, lock: bool, existing: bool, strategy: u32) {
        m.set_lock(lock);
        m.as_mut().set_checkout_existing(existing);
        let mut checkout = m.checkout_options();
        checkout.set_strategy(strategy);
        checkout.as_mut().set_dir_mode(0o755);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_field_round_trips_on_every_holder() {
        let mut local = AddOptions::new();
        let mut boxed = Box::new(AddOptions::new());
        let mut run: Vec<AddOptions<'_>> = Vec::new();
        for round in 0..64u32 {
            let (lock, existing, strategy) = (round % 2 == 1, round % 3 == 0, round << 4);
            write(local.as_mut(), lock, existing, strategy);
            write(boxed.as_mut().as_mut(), lock, existing, strategy);
            // The run reallocates as it grows, moving every value already in it.
            run.push(AddOptions::new());
            write(run.last_mut().unwrap().as_mut(), lock, existing, strategy);
            let expected = (AddOptions::VERSION, lock, existing, false, strategy, 0o755);
            assert_eq!(read(local.as_ref()), expected);
            assert_eq!(read((*boxed).as_ref()), expected);
            assert_eq!(read(black_box(&run[round as usize]).as_ref()), expected);
        }
        for (i, opts) in run.iter().enumerate() {
            assert_eq!(read(opts.as_ref()).4, u32::try_from(i).unwrap() << 4);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reborrows_keep_one_writer() {
        let mut opts = AddOptions::new();
        let mut m = opts.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_lock(true);
            let mut checkout = inner.checkout_options();
            let mut deeper = checkout.as_mut();
            deeper.set_strategy(0x10);
            assert_eq!(checkout.as_ref().strategy(), 0x10);
        }
        let shared = m.as_ref();
        assert!(shared.lock());
        let copy = shared;
        assert_eq!(copy.checkout_options().strategy(), 0x10);
        m.set_checkout_existing(true);
        assert!(opts.as_ref().checkout_existing());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn borrowed_reference_is_read_by_c_and_released_once() {
        let repo = AddRepo::new("ref");
        let feature = repo.reference(c"refs/heads/feature");
        {
            let mut opts = AddOptions::new();
            opts.as_mut().set_reference(Some(feature.as_ref()));
            opts.as_mut().set_lock(true);
            let w = repo.add("beta", Some(&opts)).unwrap();
            assert_eq!(w.as_ref().name(), c"beta");
            assert_eq!(w.as_ref().is_locked(), Ok(true));
            // The reference is now checked out there: a second worktree is refused.
            let (rc, message) = repo.add("gamma", Some(&opts)).err().unwrap();
            assert_eq!(rc, -1);
            assert_eq!(
                message,
                "reference refs/heads/feature is already checked out"
            );
            // The options never owned the reference.
            assert!(opts.as_ref().reference().is_some());
        }
        assert_eq!(
            fs::read_to_string(repo.admin("beta").join("HEAD")).unwrap(),
            "ref: refs/heads/feature\n"
        );
        // Dropped after the options, before the repository: freed once (ASan, LSan).
        drop(feature);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_reference_leaves_nothing_behind() {
        let repo = AddRepo::new("tag");
        let tag = repo.reference(c"refs/tags/v1");
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(tag.as_ref()));
        // Not a branch: refused before anything is created or copied.
        let refused = repo.add("delta", Some(&opts)).err();
        assert_eq!(refused, Some((-1, "reference is not a branch".to_owned())));
        assert!(!repo.admin("delta").exists());
        // The options borrow the reference until their last use, cleared or not.
        opts.as_mut().set_reference(None);
        drop(tag);
        // Without one, an existing branch named after the worktree is checked out.
        let mut opts = AddOptions::new();
        opts.as_mut().set_checkout_existing(true);
        let w = repo.add("alpha", Some(&opts)).unwrap();
        assert_eq!(w.as_ref().is_locked(), Ok(false));
        drop(w);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reference_from_another_repository_is_only_read() {
        let other = AddRepo::new("other-a");
        let repo = AddRepo::new("other-b");
        let feature = other.reference(c"refs/heads/feature");
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(feature.as_ref()));
        // libgit2 takes the name only; `repo` has its own `feature`.
        let w = repo.add("wt", Some(&opts)).unwrap();
        assert_eq!(w.as_ref().name(), c"wt");
        // The options are done with the reference; it goes before its repository.
        drop(w);
        drop(feature);
        drop(other);
        drop(repo);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn checkout_options_borrows_are_read_by_c() {
        let repo = AddRepo::new("checkout");
        let target = CString::new(repo.base.join("target").as_os_str().as_encoded_bytes()).unwrap();
        let label = CString::new("mine").unwrap();
        let mut opts = AddOptions::new();
        {
            let mut m = opts.as_mut();
            let mut checkout = m.checkout_options();
            checkout.set_target_directory(Some(&target));
            checkout.set_our_label(Some(&label));
        }
        let main = repo.reference(c"refs/heads/main");
        opts.as_mut().set_reference(Some(main.as_ref()));
        // `main` is the repository's own HEAD, so it is checked out already.
        let (_, message) = repo.add("refused", Some(&opts)).err().unwrap();
        assert_eq!(message, "reference refs/heads/main is already checked out");
        let feature = repo.reference(c"refs/heads/feature");
        opts.as_mut().set_reference(Some(feature.as_ref()));
        let w = repo.add("wt", Some(&opts)).unwrap();
        // The checkout went to the borrowed target directory, which it created.
        assert!(repo.base.join("target").is_dir());
        drop(w);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_reads_between_reborrows() {
        let repo = AddRepo::new("reborrow");
        let feature = repo.reference(c"refs/heads/feature");
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(feature.as_ref()));
        // The branch is free, then checked out by the first worktree; then no reference
        // and the existing `alpha`. (No round creates a branch, which would take a
        // reflog signature at the current time; see the fixture.)
        for (round, name) in ["one", "two", "alpha"].into_iter().enumerate() {
            {
                let mut m = opts.as_mut();
                let mut inner = m.as_mut();
                inner.set_lock(round % 2 == 0);
                let _shared = inner.as_ref();
            }
            if round == 2 {
                let mut m = opts.as_mut();
                m.set_reference(None);
                m.set_checkout_existing(true);
            }
            let added = repo.add(name, Some(&opts));
            let _ = opts.as_ref().reference();
            assert_eq!(
                added.is_ok(),
                round != 1,
                "round {round}: {:?}",
                added.err()
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn many_threads_build_and_use_their_own() {
        thread::scope(|s| {
            for t in 0..8 {
                s.spawn(move || {
                    let repo = AddRepo::new(&format!("thread-{t}"));
                    let feature = repo.reference(c"refs/heads/feature");
                    let mut opts = AddOptions::new();
                    opts.as_mut().set_reference(Some(feature.as_ref()));
                    opts.as_mut().set_lock(t % 2 == 0);
                    let w = repo.add("wt", Some(&opts)).unwrap();
                    assert_eq!(w.as_ref().is_locked(), Ok(t % 2 == 0));
                    for _ in 0..64 {
                        assert_eq!(read(AddOptions::new().as_ref()).0, AddOptions::VERSION);
                    }
                });
            }
        });
    }

    fn create_branches() {
        let repo = AddRepo::new("create");
        let heads = repo.base.join("repo/.git/refs/heads");
        // No reference, no existing branch: a new one named after the worktree, at
        // `HEAD`.
        let w = repo.add("fresh", Some(&AddOptions::new())).unwrap();
        assert_eq!(w.as_ref().name(), c"fresh");
        assert!(heads.join("fresh").is_file());
        // `checkout_existing` with no such branch falls back to creating it.
        let mut opts = AddOptions::new();
        opts.as_mut().set_checkout_existing(true);
        assert!(repo.add("other", Some(&opts)).is_ok());
        assert!(heads.join("other").is_file());
        // Nothing to branch from: `HEAD` is unborn.
        fs::write(repo.base.join("repo/.git/HEAD"), "ref: refs/heads/unborn\n").unwrap();
        let (rc, _) = repo.add("orphan", Some(&opts)).err().unwrap();
        assert_eq!(rc, -9, "GIT_EUNBORNBRANCH");
        assert!(!repo.admin("orphan").exists());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn default_options_create_a_branch_at_head() {
        // Creating a branch takes a reflog signature at the current time, whose glibc
        // timezone lookup TSan reports against concurrent tests: alone in a child.
        isolated(
            concat!(module_path!(), "::default_options_create_a_branch_at_head"),
            create_branches,
        );
    }

    fn new_before_init() {
        // Nothing initialised libgit2 in this child: the template copy needs no runtime.
        let opts = AddOptions::new();
        assert_eq!(
            read(opts.as_ref()),
            (AddOptions::VERSION, false, false, false, 0, 0)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_needs_no_initialised_libgit2() {
        isolated(
            concat!(module_path!(), "::new_needs_no_initialised_libgit2"),
            new_before_init,
        );
    }
}

/// UB tests for the worktree routines: `Worktree::lookup` / `open_from_repository`,
/// `list`, the readers on `WorktreeRef` and the lock routines on `WorktreeMut`.
///
/// The strings `name` and `path` return borrow the worktree and must stay readable
/// for as long as its handle (`ASan`: use after free; a string outliving its box does not
/// compile, doctest on `WorktreeRef::name`). The arrays and buffers the routines fill
/// (`list`, whose filtering moves strings within the array and frees the dropped ones,
/// and `lock_reason`) must release every allocation once, failed calls included (`ASan`,
/// `LSan`). The readers run on many threads at once through one shared box, as the
/// worktree's `Sync` claims, while only the exclusive handle locks (`TSan`; locking
/// through a shared handle does not compile, doctest on `WorktreeMut::lock`). A
/// repository without paths, whose NULL common directory libgit2 would pass to
/// `strlen`, must be refused before C (ASan/UBSan: NULL dereference). C writes the
/// `locked` bit between Rust reborrows (`BSan`). Every test calls libgit2, so none runs
/// under Miri.
mod routines {
    use std::sync::Barrier;
    use std::{fs, thread};

    use libgit2::libgit2::src::libgit2::repository::Repository;
    use libgit2::libgit2::src::libgit2::strarray::StrArrayVal;
    use libgit2::libgit2::src::libgit2::worktree::{
        PruneFlag, PruneOptions, Worktree, WorktreeError, list,
    };

    use crate::fixture::worktree::WorktreeRepo;

    fn names(array: &StrArrayVal) -> Vec<String> {
        let mut names: Vec<_> = array
            .as_ref()
            .iter()
            .map(|n| n.to_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn strings_live_as_long_as_the_handle() {
        let repo = WorktreeRepo::new("strings");
        repo.add("wt");
        let mut w = repo.lookup(c"wt");
        let (name, path) = {
            let r = w.as_ref();
            (r.name(), r.path())
        };
        // The strings outlive the handle they came from, as long as the box is shared.
        let expected = repo.workdir("wt");
        assert_eq!(path.to_str().unwrap(), expected.to_str().unwrap());
        assert_eq!(w.as_ref().name(), name);
        let copy = w.as_ref();
        assert_eq!(copy.path(), path);
        // Then an exclusive reborrow reads the same strings, and C writes in between.
        let mut m = w.as_mut();
        m.lock(None).unwrap();
        assert_eq!(m.as_ref().name(), c"wt");
        assert_eq!(m.as_mut().unlock(), Ok(true));
        assert_eq!(w.as_ref().name().to_bytes(), b"wt");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lists_release_every_string() {
        let repo = WorktreeRepo::new("list");
        for name in ["a", "b", "c"] {
            repo.add(name);
        }
        // Entries libgit2 lets through and the wrapper drops: directories without a
        // worktree's files, and plain files.
        for junk in ["j1", "j2", "j3", "j4", "j5"] {
            fs::create_dir_all(repo.admin(junk)).unwrap();
        }
        fs::write(repo.admin("file"), "").unwrap();
        for _ in 0..8 {
            let listed = list(repo.repo()).unwrap();
            assert_eq!(names(&listed), ["a", "b", "c"]);
        }
        // Every worktree pruned by hand: only junk is left, and dropped.
        for name in ["a", "b", "c"] {
            fs::remove_dir_all(repo.admin(name)).unwrap();
        }
        assert!(list(repo.repo()).unwrap().as_ref().is_empty());
        fs::remove_dir_all(repo.base.join("repo/.git/worktrees")).unwrap();
        assert!(list(repo.repo()).unwrap().as_ref().is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lists_cross_threads_and_outlive_the_repository() {
        let repo = WorktreeRepo::new("list-send");
        repo.add("wt");
        let listed = list(repo.repo()).unwrap();
        repo.close();
        let read = thread::spawn(move || names(&listed)).join().unwrap();
        assert_eq!(read, ["wt"]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn repositories_without_paths_are_refused_before_c() {
        crate::fixture::init();
        let odb = libgit2::libgit2::src::libgit2::odb::Odb::new().unwrap();
        for repo in [
            Repository::new().unwrap(),
            Repository::wrap_odb(odb).unwrap(),
        ] {
            assert!(list(repo.as_ref()).unwrap().as_ref().is_empty());
            let e = Worktree::lookup(repo.as_ref(), c"wt").unwrap_err();
            assert_eq!(e.raw_code(), libgit2::ffi::GIT_ENOTFOUND);
            assert!(Worktree::open_from_repository(repo.as_ref()).is_err());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failed_lookups_release_everything() {
        let repo = WorktreeRepo::new("failed");
        repo.add("wt");
        fs::create_dir_all(repo.admin("broken")).unwrap();
        fs::write(repo.admin("broken").join("commondir"), "../..\n").unwrap();
        fs::write(repo.admin("broken").join("HEAD"), "x\n").unwrap();
        fs::create_dir(repo.admin("broken").join("gitdir")).unwrap();
        for _ in 0..8 {
            assert!(Worktree::lookup(repo.repo(), c"absent").is_err());
            assert!(Worktree::lookup(repo.repo(), c"broken").is_err());
            assert!(Worktree::lookup(repo.repo(), c"").is_err());
            assert!(Worktree::open_from_repository(repo.repo()).is_err());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn worktree_opened_from_its_repository_outlives_it() {
        let repo = WorktreeRepo::new("open-from");
        repo.add("wt");
        let workdir = repo.workdir("wt");
        let c_dir = std::ffi::CString::new(workdir.to_str().unwrap()).unwrap();
        let opened = Repository::open(&c_dir).unwrap();
        let w = Worktree::open_from_repository(opened.as_ref()).unwrap();
        drop(opened);
        repo.close();
        assert_eq!(w.as_ref().name(), c"wt");
        assert!(w.as_ref().validate().is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lock_reasons_release_their_buffers() {
        let repo = WorktreeRepo::new("reason");
        repo.add("wt");
        let mut w = repo.lookup(c"wt");
        assert!(w.as_ref().lock_reason().unwrap().is_none());
        for reason in [Some(c"because"), None, Some(c"")] {
            w.as_mut().lock(reason).unwrap();
            let held = w.as_ref().lock_reason().unwrap().unwrap();
            let bytes = held.as_ref().as_bytes();
            assert_eq!(bytes, reason.map_or(&b""[..], |r| r.to_bytes()));
            assert_eq!(w.as_mut().unlock(), Ok(true));
            // The buffer is the caller's: the unlock frees nothing it holds.
            assert_eq!(held.as_ref().as_bytes(), bytes);
        }
        // An unreadable `locked` entry fails, with nothing to release.
        fs::create_dir(repo.admin("wt").join("locked")).unwrap();
        for _ in 0..4 {
            assert!(w.as_ref().lock_reason().is_err());
        }
        assert!(w.as_mut().unlock().is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_readers_race_nothing() {
        let repo = WorktreeRepo::new("readers");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let start = Barrier::new(4);
        let opts = PruneOptions::new();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    start.wait();
                    let r = w.as_ref();
                    for _ in 0..16 {
                        assert_eq!(r.name(), c"wt");
                        assert_eq!(r.validate(), Ok(()));
                        assert_eq!(r.is_locked(), Ok(false));
                        assert!(r.lock_reason().unwrap().is_none());
                        // Valid, so not prunable: the reason stays on this thread.
                        assert_eq!(r.is_prunable(opts.as_ref()), Ok(false));
                        assert!(r.prune(opts.as_ref()).is_err());
                    }
                });
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn exclusive_handle_locks_on_another_thread() {
        let repo = WorktreeRepo::new("lock-thread");
        repo.add("wt");
        let mut w = repo.lookup(c"wt");
        let m = w.as_mut();
        thread::scope(|s| {
            s.spawn(move || {
                let mut m = m;
                m.lock(Some(c"from a thread")).unwrap();
                assert_eq!(m.as_ref().is_locked(), Ok(true));
            });
        });
        let error: WorktreeError = thread::spawn({
            let mut w = repo.lookup(c"wt");
            move || w.as_mut().lock(None).unwrap_err()
        })
        .join()
        .unwrap();
        assert_eq!(error.raw_code(), libgit2::ffi::GIT_ELOCKED);
        assert_eq!(w.as_mut().unlock(), Ok(true));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn pruned_worktree_stays_readable() {
        let repo = WorktreeRepo::new("prune");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let mut opts = PruneOptions::new();
        opts.as_mut()
            .set_flags(PruneFlag::Valid | PruneFlag::WorkingTree);
        w.as_ref().prune(opts.as_ref()).unwrap();
        assert!(!repo.admin("wt").exists() && !repo.workdir("wt").exists());
        // The struct and its strings are untouched; the directories are gone.
        assert_eq!(w.as_ref().name(), c"wt");
        assert!(w.as_ref().validate().is_err());
        assert!(w.as_ref().prune(opts.as_ref()).is_err());
        assert_eq!(w.as_ref().is_locked(), Ok(false));
    }
}

/// `Worktree::add`: the safe wrapper of `git_worktree_add`, which builds the worktree's
/// directories and files, checks it out, and hands back a `WorktreeBox` it looked up.
///
/// The box copies every string it holds, so it outlives the repository it was added to
/// and the options and reference the add read (`ASan`: no use after free). Every failing
/// add releases what libgit2 allocated for it, and the repository the inner checkout
/// opened, and returns no worktree (`LSan`, ASan): a missing parent directory, an existing
/// working or administrative directory, a reference that is not a branch or that is
/// checked out, a failed checkout. A repository without paths, whose NULL common
/// directory libgit2 would join and then assert on, is refused before the call
/// (UBSan/ASan would flag the NULL read; the unit tests check the working directory
/// is untouched). Repositories are not `Send`, so each thread adds to its own (`TSan`).
/// Adds that create a branch take the time through glibc's `tzset`, which `TSan` reports
/// against other threads: those run alone in a child. Every test calls libgit2, so
/// none runs under Miri.
mod adding {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::{fs, thread};

    use libgit2::libgit2::src::libgit2::repository::Repository;
    use libgit2::libgit2::src::libgit2::worktree::{AddOptions, Worktree};
    use libgit2::libgit2::src::util::errors::ErrorCode;

    use crate::fixture::isolated;
    use crate::fixture::worktree_add::AddRepo;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn added_worktree_outlives_its_repository_and_options() {
        let repo = AddRepo::new("outlives");
        let feature = repo.reference(c"refs/heads/feature");
        let name = CString::new("wt").unwrap();
        let path = CString::new(repo.base.join("wt/wt").as_os_str().as_bytes()).unwrap();
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(feature.as_ref()));
        opts.as_mut().set_lock(true);
        let w = Worktree::add(repo.repo().as_ref(), &name, &path, Some(opts.as_ref())).unwrap();
        // The strings passed in and the reference go first (the options' last use is
        // above).
        drop((name, path));
        drop(feature);
        let (base, admin) = (repo.base.clone(), repo.admin("wt"));
        assert_eq!(w.as_ref().name(), c"wt");
        let wt_path = w.as_ref().path().to_str().unwrap().to_owned();
        assert_eq!(wt_path, base.join("wt/wt").to_str().unwrap());
        assert_eq!(w.as_ref().is_locked(), Ok(true));
        // Then the repository, and its directories with the fixture; the box still
        // reads its copies.
        drop(repo);
        assert!(!admin.exists());
        assert_eq!(w.as_ref().name(), c"wt");
        assert!(w.as_ref().validate().is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failing_adds_release_everything() {
        let repo = AddRepo::new("failing");
        fs::write(repo.base.join("blocked"), "a file\n").unwrap();
        let feature = repo.reference(c"refs/heads/feature");
        let tag = repo.reference(c"refs/tags/v1");
        let alpha = repo.reference(c"refs/heads/alpha");
        let at = |rel: &str| CString::new(repo.base.join(rel).as_os_str().as_bytes()).unwrap();
        let add = |name: &std::ffi::CStr, rel: &str, opts: &AddOptions<'_>| {
            Worktree::add(repo.repo().as_ref(), name, &at(rel), Some(opts.as_ref()))
        };
        let mut opts = AddOptions::new();
        // A tag is not a branch.
        opts.as_mut().set_reference(Some(tag.as_ref()));
        let error = add(c"e", "wt/e", &opts).unwrap_err();
        assert_eq!(error.message(), c"reference is not a branch");
        opts.as_mut().set_reference(Some(feature.as_ref()));
        // The checkout fails (its target is a file), after the worktree is on disk.
        let target = at("blocked");
        opts.as_mut()
            .checkout_options()
            .set_target_directory(Some(&target));
        assert!(add(c"c", "wt/c", &opts).is_err());
        opts.as_mut().checkout_options().set_target_directory(None);
        // What it left holds the branch checked out. (libgit2's check gives up once a
        // directory under `worktrees/` is not a worktree's, as the failures below
        // leave: so this comes first.)
        let error = add(c"d", "wt/d", &opts).unwrap_err();
        assert_eq!(
            error.message(),
            c"reference refs/heads/feature is already checked out"
        );
        opts.as_mut().set_reference(None);
        opts.as_mut().set_checkout_existing(true);
        // The working directory's parent is missing: the administrative directory stays.
        let error = add(c"alpha", "missing/alpha", &opts).unwrap_err();
        assert_eq!(
            error.class(),
            Some(libgit2::libgit2::src::util::errors::ErrorClass::Os)
        );
        assert!(repo.admin("alpha").is_dir());
        // That administrative directory exists now.
        let error = add(c"alpha", "wt/alpha", &opts).unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Exists));
        // The working directory exists.
        fs::create_dir(repo.base.join("wt/b")).unwrap();
        opts.as_mut().set_reference(Some(alpha.as_ref()));
        let error = add(c"b", "wt/b", &opts).unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Exists));
        assert!(repo.admin("b").is_dir());
        drop((feature, tag, alpha));
    }

    /// A reference of another repository lends its name: the add reads the lent
    /// reference and the repository's own of that name for the call only (`ASan`: the
    /// lender and its reference go first, the worktree still reads its copies), and
    /// releases the reference it looked up itself whether the add succeeds, fails after
    /// the lookup (the branch checked out already), or finds no branch of that name in
    /// the repository (`LSan`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lent_references_are_read_for_the_call_only() {
        let repo = AddRepo::new("lent-to");
        let lender = AddRepo::new("lender");
        fs::write(lender.base.join("repo/.git/refs/heads/only"), {
            fs::read_to_string(lender.base.join("repo/.git/refs/heads/main")).unwrap()
        })
        .unwrap();
        let feature = lender.reference(c"refs/heads/feature");
        let only = lender.reference(c"refs/heads/only");
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(feature.as_ref()));
        let w = repo.add("lent", Some(&opts)).unwrap();
        let error = repo.add("twice", Some(&opts)).unwrap_err();
        assert_eq!(
            error.1,
            "reference refs/heads/feature is already checked out"
        );
        opts.as_mut().set_reference(Some(only.as_ref()));
        let error = repo.add("none", Some(&opts)).unwrap_err();
        assert_eq!(error.0, ErrorCode::UnbornBranch.raw());
        drop((feature, only));
        drop(lender);
        assert_eq!(w.as_ref().name(), c"lent");
        assert_eq!(w.as_ref().validate(), Ok(()));
        let head = fs::read_to_string(repo.admin("lent").join("HEAD")).unwrap();
        assert_eq!(head, "ref: refs/heads/feature\n");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_repository_without_paths_is_refused() {
        let alpha = AddRepo::new("pathless");
        let reference = alpha.reference(c"refs/heads/alpha");
        let pathless = Repository::new().unwrap();
        let mut opts = AddOptions::new();
        opts.as_mut().set_reference(Some(reference.as_ref()));
        let path = CString::new(alpha.base.join("wt/never").as_os_str().as_bytes()).unwrap();
        for opts in [None, Some(opts.as_ref())] {
            let error = Worktree::add(pathless.as_ref(), c"never", &path, opts).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
        }
        assert!(!alpha.base.join("wt/never").exists());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_add_to_their_own_repositories() {
        thread::scope(|s| {
            for t in 0..6 {
                s.spawn(move || {
                    let repo = AddRepo::new(&format!("adding-thread-{t}"));
                    let alpha = repo.reference(c"refs/heads/alpha");
                    let mut opts = AddOptions::new();
                    opts.as_mut().set_reference(Some(alpha.as_ref()));
                    let path = repo.base.join("wt/wt");
                    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
                    let w = Worktree::add(repo.repo().as_ref(), c"wt", &path, Some(opts.as_ref()));
                    let w = w.unwrap();
                    drop(alpha);
                    // The worktree moves to the thread that drops it last.
                    thread::spawn(move || assert_eq!(w.as_ref().name(), c"wt"))
                        .join()
                        .unwrap();
                });
            }
        });
    }

    /// Without options, or with `checkout_existing`: the branch is created (or reused),
    /// and created branches stay when the add fails later.
    fn default_adds() {
        let repo = AddRepo::new("adding-default");
        let path = |rel: &str| CString::new(repo.base.join(rel).as_os_str().as_bytes()).unwrap();
        let w = Worktree::add(repo.repo().as_ref(), c"fresh", &path("wt/fresh"), None).unwrap();
        assert_eq!(w.as_ref().name(), c"fresh");
        let error = Worktree::add(repo.repo().as_ref(), c"fresh", &path("wt/x"), None).unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Exists));
        let error = Worktree::add(repo.repo().as_ref(), c"bad..name", &path("wt/y"), None);
        assert_eq!(error.unwrap_err().code(), Some(ErrorCode::InvalidSpec));
        let error = Worktree::add(repo.repo().as_ref(), c"lost", &path("none/lost"), None);
        assert!(error.is_err());
        assert!(repo.base.join("repo/.git/refs/heads/lost").is_file());
        drop(w);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adds_without_options_create_their_branch() {
        isolated(
            concat!(module_path!(), "::adds_without_options_create_their_branch"),
            default_adds,
        );
    }
}
