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

/// UB tests for `git2/worktree.h`: the `WorktreeBox` owner of a `git_worktree`, the
/// handles it lends, and the `git_worktree_free` policy.
///
/// Worktrees are adopted from `git_worktree_lookup` by the fixture, which also reads
/// and locks them through their boxes, standing in for the wrappers that do. Each box
/// must release its struct and six strings exactly once (ASan: double free, use after
/// free; LSan: leak), on whichever thread drops it, and shared boxes must race nothing
/// (TSan). libgit2 writes a worktree in place (`git_worktree_lock`) between Rust
/// reborrows of its handles (BSan). Handles that outlive their box and aliased exclusive
/// handles do not compile (doctests on `WorktreeBox`); the box is not `Clone` (doctest).
/// Every test calls libgit2, so none runs under Miri.
mod worktree_box {
    use std::sync::Barrier;
    use std::thread;

    use libgit2::libgit2::repository::worktree::{
        Worktree, WorktreeBox, WorktreeFree, WorktreeMut, WorktreeRef,
    };

    use crate::fixture::worktree::{WorktreeRepo, is_locked, lock, name, path, unlock};

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
            assert_eq!(name(&a), c"a");
            assert_eq!(name(&b), c"b");
            assert_ne!(path(&a), path(&b));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn replaced_box_is_released_once() {
        let repo = WorktreeRepo::new("replace");
        repo.add("a");
        repo.add("b");
        let mut w = repo.lookup(c"a");
        assert_eq!(name(&w), c"a");
        // The old worktree drops here, the new one at the end of the scope.
        w = repo.lookup(c"b");
        assert_eq!(name(&w), c"b");
        let mut slot = Some(w);
        assert_eq!(slot.take().map(|w| name(&w)), Some(c"b".to_owned()));
        assert!(slot.is_none());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn worktree_outlives_its_repository() {
        let repo = WorktreeRepo::new("outlive");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let expected = path(&w);
        // The repository is freed and its directory removed: the worktree holds copies.
        repo.close();
        assert_eq!(name(&w), c"wt");
        assert_eq!(path(&w), expected);
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
            .map(|w| thread::spawn(move || name(&w)))
            .collect();
        let read: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(read, names.map(|n| n.to_owned()));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_box_reads_race_nothing() {
        let repo = WorktreeRepo::new("sync");
        repo.add("wt");
        let w = repo.lookup(c"wt");
        let expected = (name(&w), path(&w));
        let start = Barrier::new(4);
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    start.wait();
                    // Shared handles and C reads through the same box, concurrently.
                    let r = w.as_ref();
                    for _ in 0..32 {
                        assert_eq!((name(&w), path(&w)), expected);
                        assert_eq!(is_locked(&w), 0);
                    }
                    let _ = r;
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
            assert_eq!(lock(&mut w, c"held"), 0);
            // A second lock is refused without writing.
            assert!(lock(&mut w, c"again") < 0);
            let r = w.as_ref();
            let _ = r;
            assert_eq!(is_locked(&w), 1);
            assert_eq!(unlock(&mut w), 0);
            // Unlocking an unlocked worktree reports 1 and writes nothing.
            assert_eq!(unlock(&mut w), 1);
            assert_eq!(is_locked(&w), 0, "round {round}");
            assert_eq!(name(&w), c"wt");
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
/// the options to `git_worktree_add` (through the fixture), which copies them, reads the
/// borrowed reference and the checkout options' borrowed strings, and keeps no pointer
/// to any of them. A misaligned projection, a stale reference pointer or a write through
/// the shared handle would be flagged by Miri and ASan/UBSan; the reference and the
/// options must be released exactly once, in either order allowed (LSan, ASan). Options
/// that outlive their reference or its repository, an exclusive handle to a borrowed
/// reference, a shrunk exclusive handle, `Send` and `Clone` do not compile (the type's
/// `compile_fail` doctests). The options are neither `Send` nor `Sync`, so each thread
/// builds its own: `AddOptions::new` (a template copy) runs from many threads at once
/// and before anything initialised libgit2 (ASan, TSan). libgit2 only reads the options
/// during a call and writes nothing through them, so Rust reborrows between calls race
/// no foreign write (BSan). Every test calls libgit2, so none runs under Miri; the
/// accessors run there in the crate's unit tests, on a template built in Rust.
mod add_options {
    use std::ffi::CString;
    use std::fs;
    use std::hint::black_box;
    use std::thread;

    use libgit2::libgit2::repository::worktree::{AddOptions, AddOptionsMut, AddOptionsRef};

    use crate::fixture::isolated;
    use crate::fixture::worktree::{is_locked, name};
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
            assert_eq!(read(opts.as_ref()).4, (i as u32) << 4);
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
            assert_eq!(name(&w), c"beta");
            assert_eq!(is_locked(&w), 1);
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
        assert_eq!(is_locked(&w), 0);
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
        assert_eq!(name(&w), c"wt");
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
                    assert_eq!(is_locked(&w), i32::from(t % 2 == 0));
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
        assert_eq!(name(&w), c"fresh");
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
