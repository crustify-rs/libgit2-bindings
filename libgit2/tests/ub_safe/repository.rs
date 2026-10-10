//! UB tests for `git2/repository.h`, one section per batch.

/// The init flag enum `InitFlag`, its set `InitFlags`, and the mode `InitMode`.
///
/// `InitFlag` is a fieldless enum over C's `git_repository_init_flag_t`. The only safe
/// way in from a raw value is `from_raw` / `try_from`, which must reject every value
/// without a variant rather than produce an invalid discriminant (which Miri and UBSan
/// would flag on the next read); an `as` cast to it does not compile
/// (`compile_fail/`). `InitFlags` keeps any `uint32_t` C hands it, init_ext's internal
/// bits included, so its operations must stay in range for every bit pattern.
/// `InitMode` reads every `u32` as a mode: the shared modes' values become their
/// variants and every other value a `CustomMode`, which only `from_raw` builds (its
/// field is private: a `compile_fail` doctest on `InitMode`), so no two values alias
/// and every value reads back unchanged.
///
/// They are plain values: nothing allocates, points or is freed, so there is no
/// lifetime for ASan or BorrowSanitizer to catch, and sharing them across threads races
/// nothing (TSan). None of them calls libgit2, so they also run under Miri.
mod init_flags_and_modes {
    use std::collections::HashSet;
    use std::hint::black_box;
    use std::thread;

    use libgit2::libgit2::repository::repository::{InitFlag, InitFlags, InitMode};

    /// Every value around the declared ones, each single bit, and a coarse stride
    /// across the whole type; BorrowSanitizer instruments each comparison, so the probe
    /// set stays small.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain([0o2774, 0o2775, 0o2776, 0o2777, 0o3000])
            .chain((0..32).map(|k| 1u32 << k))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// Bit patterns for `InitFlags`: each single bit, its complement, and the extremes.
    fn bit_patterns() -> impl Iterator<Item = u32> {
        (0..32)
            .flat_map(|k| [1 << k, !(1 << k)])
            .chain([0, u32::MAX, 0x7_007b, 0x7_0000])
    }

    #[test]
    fn flag_from_raw_yields_only_valid_flags() {
        let mut seen = HashSet::new();
        for raw in probes() {
            // `black_box` keeps the conversion from being folded at compile time.
            if let Some(flag) = InitFlag::from_raw(black_box(raw)) {
                // Reading the discriminant back validates it.
                assert_eq!(flag.raw(), raw);
                assert!(!format!("{flag:?}").is_empty());
                seen.insert(flag);
            } else {
                assert_eq!(InitFlag::try_from(raw).map_err(|e| e.raw()), Err(raw));
            }
        }
        assert_eq!(seen.len(), InitFlag::ALL.len());
    }

    #[test]
    fn option_niche_holds_for_every_value() {
        // `Option<InitFlag>` stores `None` in a value no variant uses: a rejected value
        // must never alias a variant, nor a variant read as `None`.
        for raw in probes() {
            let flag = black_box(InitFlag::from_raw(raw));
            assert_eq!(flag.map(InitFlag::raw).unwrap_or(raw), raw);
        }
    }

    #[test]
    fn flags_stay_in_range_for_every_bit_pattern() {
        for bits in bit_patterns() {
            let mut set = InitFlags::from_bits_retain(black_box(bits));
            assert_eq!(set.bits(), bits);
            let named: Vec<_> = set.iter().collect();
            assert!(named.len() <= InitFlag::ALL.len());
            assert!(named.iter().all(|&f| set.contains(f)));
            assert!(!format!("{set:?}").is_empty());
            let truncated = InitFlags::from_bits_truncate(bits);
            assert_eq!(truncated.unnamed_bits(), 0);
            assert_eq!(truncated.iter().collect::<Vec<_>>(), named);
            for flag in InitFlag::ALL {
                set.insert(flag);
                assert!(set.contains(flag));
                set.remove(flag);
                assert!(!set.contains(flag));
            }
            assert_eq!(set.bits(), set.unnamed_bits());
            set |= InitFlags::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
        }
    }

    #[test]
    fn mode_from_raw_reads_back_every_value() {
        let mut named = HashSet::new();
        for raw in probes() {
            let mode = black_box(InitMode::from_raw(black_box(raw)));
            // Reading the variant and its payload back validates them.
            assert_eq!(mode.raw(), raw);
            assert!(!format!("{mode:?}").is_empty());
            match mode {
                InitMode::Custom(custom) => assert_eq!(custom.bits(), raw),
                other => {
                    named.insert(other);
                }
            }
        }
        assert_eq!(named.len(), 3);
    }

    #[test]
    fn values_cross_threads() {
        // Plain values: `Send` and `Sync` without an assertion, so sharing them races
        // nothing for TSan to report.
        let set = InitFlags::from_bits_retain(u32::MAX);
        let modes = [0, 0o2775, 0o2777, 0o750].map(InitMode::from_raw);
        thread::scope(|s| {
            let handles: Vec<_> = InitFlag::ALL
                .iter()
                .map(|flag| {
                    let (set, modes) = (&set, &modes);
                    s.spawn(move || (flag.raw(), set.contains(*flag), modes.map(InitMode::raw)))
                })
                .collect();
            for (flag, h) in InitFlag::ALL.iter().zip(handles) {
                assert_eq!(
                    h.join().unwrap(),
                    (flag.raw(), true, [0, 0o2775, 0o2777, 0o750])
                );
            }
        });
    }
}

/// The enums `Item` and `OpenFlag`, and the flag set `OpenFlags`.
///
/// `Item` and `OpenFlag` are fieldless enums over C's `git_repository_item_t` and
/// `git_repository_open_flag_t`. The only safe way in from a raw value is `from_raw` /
/// `try_from`, which must reject every value without a variant rather than produce an
/// invalid discriminant (which Miri and UBSan would flag on the next read); an `as` cast
/// to either does not compile (`compile_fail/`). For `Item` that also keeps libgit2 in
/// bounds: `git_repository_item_path` indexes a table of `GIT_REPOSITORY_ITEM__LAST`
/// entries with the value unchecked (UBSan reports `index 15 out of bounds` for the
/// sentinel), so every value an `Item` carries must resolve in bounds under ASan/UBSan.
/// `OpenFlags` keeps any `unsigned int`, so its operations must stay in range for every
/// bit pattern. They are plain values: nothing allocates, points or is freed, so there
/// is no lifetime for ASan or BorrowSanitizer to catch, and sharing them across threads
/// races nothing (TSan). Only the item paths call libgit2; the rest also runs under
/// Miri.
mod items_and_open_flags {
    use std::collections::HashSet;
    use std::hint::black_box;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{Item, OpenFlag, OpenFlags};

    use crate::fixture::{TempRepo, item_path};

    /// Every value around the declared ones, each single bit, and a coarse stride
    /// across the whole type; BorrowSanitizer instruments each comparison, so the probe
    /// set stays small.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain((0..32).map(|k| 1u32 << k))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// Bit patterns for `OpenFlags`: each single bit, its complement, and the extremes.
    fn bit_patterns() -> impl Iterator<Item = u32> {
        (0..32)
            .flat_map(|k| [1 << k, !(1 << k)])
            .chain([0, u32::MAX, 0x1f, !0x1f])
    }

    #[test]
    fn item_from_raw_yields_only_valid_items() {
        let mut seen = HashSet::new();
        for raw in probes() {
            // `black_box` keeps the conversion from being folded at compile time.
            if let Some(item) = Item::from_raw(black_box(raw)) {
                // Reading the discriminant back validates it.
                assert_eq!(item.raw(), raw);
                assert!(item.raw() < ffi::GIT_REPOSITORY_ITEM__LAST);
                assert!(!format!("{item:?}").is_empty());
                seen.insert(item);
            } else {
                assert_eq!(Item::try_from(raw).map_err(|e| e.raw()), Err(raw));
            }
        }
        assert_eq!(seen.len(), Item::ALL.len());
    }

    #[test]
    fn open_flag_from_raw_yields_only_valid_flags() {
        let mut seen = HashSet::new();
        for raw in probes() {
            if let Some(flag) = OpenFlag::from_raw(black_box(raw)) {
                assert_eq!(flag.raw(), raw);
                assert!(!format!("{flag:?}").is_empty());
                seen.insert(flag);
            } else {
                assert_eq!(OpenFlag::try_from(raw).map_err(|e| e.raw()), Err(raw));
            }
        }
        assert_eq!(seen.len(), OpenFlag::ALL.len());
    }

    #[test]
    fn option_niche_holds_for_every_value() {
        // `Option<Item>` / `Option<OpenFlag>` store `None` in a value no variant uses:
        // a rejected value must never alias a variant, nor a variant read as `None`.
        for raw in probes() {
            let item = black_box(Item::from_raw(raw));
            assert_eq!(item.map(Item::raw).unwrap_or(raw), raw);
            let flag = black_box(OpenFlag::from_raw(raw));
            assert_eq!(flag.map(OpenFlag::raw).unwrap_or(raw), raw);
        }
    }

    #[test]
    fn open_flags_stay_in_range_for_every_bit_pattern() {
        for bits in bit_patterns() {
            let mut set = OpenFlags::from_bits_retain(black_box(bits));
            assert_eq!(set.bits(), bits);
            let named: Vec<_> = set.iter().collect();
            assert!(named.len() <= OpenFlag::ALL.len());
            assert!(named.iter().all(|&f| set.contains(f)));
            assert_eq!(set.iter().count(), named.len());
            assert!(!format!("{set:?}").is_empty());
            for flag in OpenFlag::ALL {
                set.insert(flag);
                assert!(set.contains(flag));
                set.remove(flag);
                assert!(!set.contains(flag));
            }
            assert_eq!(set.bits(), set.unnamed_bits());
            set |= OpenFlags::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
        }
    }

    #[test]
    fn values_cross_threads() {
        // Plain values: `Send` and `Sync` without an assertion, so sharing them races
        // nothing for TSan to report.
        let set = OpenFlags::from_bits_retain(u32::MAX);
        thread::scope(|s| {
            let handles: Vec<_> = Item::ALL
                .iter()
                .map(|item| {
                    let set = &set;
                    s.spawn(move || (item.raw(), set.iter().map(OpenFlag::raw).sum::<u32>()))
                })
                .collect();
            for (item, h) in Item::ALL.iter().zip(handles) {
                assert_eq!(h.join().unwrap(), (item.raw(), 0x1f));
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_item_resolves_in_bounds() {
        let repo = TempRepo::new("items");
        // Every value `from_raw` lets through, which is every `Item` there is.
        for item in probes().filter_map(Item::from_raw) {
            match item_path(&repo, item) {
                Ok(path) => assert!(path.starts_with(repo.path.to_str().unwrap()), "{path}"),
                // The bare fixture has no working directory.
                Err(rc) => assert_eq!((item, rc), (Item::WorkDir, ffi::GIT_ENOTFOUND)),
            }
        }
    }
}

/// The enum `RepositoryState` (`git_repository_state_t`, unsigned, `0..=11`).
///
/// The only safe way in from a raw value is `from_raw` / `try_from`, which must
/// reject every value without a variant rather than produce an invalid discriminant
/// (which Miri and UBSan would flag on the next read); an `as` cast to it does not
/// compile (`compile_fail/`). It owns and points at nothing, and none of these tests
/// calls libgit2, so they also run under Miri; the sweep stays small for BSan.
mod state {
    use std::collections::HashSet;
    use std::hint::black_box;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{RepositoryState, UnnamedRepositoryState};

    /// Every value around the declared ones, and the extremes; `u32::MAX` is the `-1`
    /// `git_repository_state` returns on failure.
    fn probes() -> impl Iterator<Item = ffi::git_repository_state_t> {
        (0..=300).chain([1 << 31, u32::MAX - 1, u32::MAX])
    }

    #[test]
    fn from_raw_yields_only_valid_states() {
        let mut seen = HashSet::new();
        for raw in probes() {
            // `black_box` keeps the conversion from being folded at compile time.
            match RepositoryState::from_raw(black_box(raw)) {
                Some(state) => {
                    // Reading the discriminant back validates it.
                    assert_eq!(state.raw(), raw);
                    assert_eq!(state.is_clean(), raw == 0);
                    assert!(!format!("{state:?}").is_empty());
                    seen.insert(state);
                }
                None => {
                    let err: UnnamedRepositoryState = RepositoryState::try_from(raw).unwrap_err();
                    assert_eq!(err.raw(), raw);
                    assert!(!err.to_string().is_empty());
                }
            }
        }
        assert_eq!(seen.len(), 12);
    }

    #[test]
    fn option_niche_holds_for_every_value() {
        // `Option<RepositoryState>` stores `None` in a value no variant uses: a
        // rejected value must never alias a variant, nor a variant read as `None`.
        for raw in probes() {
            let state = black_box(RepositoryState::from_raw(raw));
            assert_eq!(state.map(RepositoryState::raw).unwrap_or(raw), raw);
            assert_eq!(state.is_some(), raw <= 11);
        }
    }

    #[test]
    fn states_cross_threads() {
        // A plain value: `Send` and `Sync` without an assertion, so sharing one races
        // nothing for TSan to report.
        let states: Vec<_> = (0..12).filter_map(RepositoryState::from_raw).collect();
        thread::scope(|s| {
            let handles: Vec<_> = states
                .iter()
                .map(|state| s.spawn(move || RepositoryState::from_raw(state.raw())))
                .collect();
            for (state, h) in states.iter().zip(handles) {
                assert_eq!(h.join().unwrap(), Some(*state));
            }
        });
    }
}

/// The `Repository` owner, its handles, its constructors and the
/// `git_repository_free` policy.
///
/// Every repository a constructor builds, or leaves behind when it fails, must be
/// released exactly once (ASan: double free, use after free; LSan: leak), whichever
/// thread builds it first (TSan: the first constructor initialises libgit2). Handles
/// outliving their box and boxes crossing threads are `compile_fail` doctests on
/// `Repository`. Every constructor calls libgit2, so nothing here runs under Miri.
mod repository_owner {
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::thread;

    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::{
        Repository, RepositoryBox, RepositoryMut, RepositoryRef,
    };
    use libgit2::libgit2::util::errors::ErrorCode;

    use crate::fixture::isolated;

    /// libgit2's bare test repository, only ever opened read-only.
    const TESTREPO: &std::ffi::CStr = {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git\0"
        );
        match std::ffi::CStr::from_bytes_with_nul(path.as_bytes()) {
            Ok(path) => path,
            Err(_) => panic!("interior NUL"),
        }
    };

    /// A scratch directory of its own under the temp dir, removed when dropped.
    struct Scratch {
        path: PathBuf,
        c_path: CString,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            Self { path, c_path }
        }

        /// `name` under the directory, NUL-terminated.
        fn child(&self, name: &str) -> CString {
            CString::new(self.path.join(name).as_os_str().as_bytes()).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Takes every handle a box hands out, in every order a caller may, then drops
    /// the box.
    fn exercise(mut repo: RepositoryBox) {
        let shared: RepositoryRef<'_> = repo.as_ref();
        let copy = shared;
        let _ = (shared, copy);
        {
            let mut exclusive: RepositoryMut<'_> = repo.as_mut();
            {
                let _reborrowed: RepositoryMut<'_> = exclusive.as_mut();
            }
            let _through: RepositoryRef<'_> = exclusive.as_ref();
        }
        let _ = repo.as_ref();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_constructor_builds_an_owner_freed_once() {
        exercise(Repository::new().unwrap());
        exercise(RepositoryBox::new().unwrap());
        for kind in [OidType::Sha1, OidType::Sha256] {
            exercise(Repository::with_oid_type(kind).unwrap());
        }
        exercise(Repository::open(TESTREPO).unwrap());
        exercise(Repository::open_bare(TESTREPO).unwrap());

        let scratch = Scratch::new("constructors");
        for (name, bare) in [("bare", true), ("work", false)] {
            let path = scratch.child(name);
            exercise(Repository::init(&path, bare).unwrap());
            // Reinitialised, then reopened, while the first one is still open.
            let first = Repository::init(&path, bare).unwrap();
            exercise(Repository::open(&path).unwrap());
            drop(first);
        }
        exercise(Repository::open_bare(&scratch.child("bare")).unwrap());
        exercise(Repository::open(&scratch.child("work/.git")).unwrap());
    }

    /// Each failure returns no owner and leaves nothing allocated, including the one
    /// where libgit2 fails after opening the repository it hands back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failures_release_everything() {
        let scratch = Scratch::new("failures");
        fs::create_dir_all(&scratch.path).unwrap();
        fs::write(scratch.path.join("file"), "").unwrap();
        let workdir = scratch.child("work");
        drop(Repository::init(&workdir, false).unwrap());

        let missing = scratch.child("missing");
        let not_found = Some(ErrorCode::NotFound);
        assert_eq!(Repository::open(c"").unwrap_err().code(), not_found);
        assert_eq!(Repository::open(&missing).unwrap_err().code(), not_found);
        assert_eq!(
            Repository::open(&scratch.c_path).unwrap_err().code(),
            not_found
        );
        assert_eq!(Repository::open_bare(c"").unwrap_err().code(), not_found);
        assert_eq!(
            Repository::open_bare(&workdir).unwrap_err().code(),
            not_found
        );
        let under_file = scratch.child("file/repo");
        assert_eq!(
            Repository::init(&under_file, true).unwrap_err().code(),
            Some(ErrorCode::Exists)
        );
        assert_eq!(
            Repository::init(c"", true).unwrap_err().code(),
            Some(ErrorCode::Generic)
        );

        // A reftable repository whose `reftable` directory cannot be made: libgit2
        // opens it, fails, and returns it anyway, which the wrapper frees.
        let reftable = scratch.path.join("reftable");
        fs::create_dir_all(reftable.join("objects")).unwrap();
        fs::create_dir_all(reftable.join("refs")).unwrap();
        fs::write(reftable.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            reftable.join("config"),
            "[core]\n\trepositoryformatversion = 1\n\tbare = true\n\
             [extensions]\n\trefstorage = reftable\n",
        )
        .unwrap();
        fs::write(reftable.join("reftable"), "").unwrap();
        let error = Repository::init(&scratch.child("reftable"), true).unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Generic));

        // `open` frees its half-built repository when the format is refused.
        let format = scratch.path.join("format");
        fs::create_dir_all(format.join("objects")).unwrap();
        fs::create_dir_all(format.join("refs")).unwrap();
        fs::write(format.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            format.join("config"),
            "[core]\n\trepositoryformatversion = 9\n",
        )
        .unwrap();
        let error = Repository::open(&scratch.child("format")).unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Generic));
    }

    /// Opening and freeing in a loop: every free releases what its open allocated.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn repeated_opens_free_everything() {
        let rounds = if cfg!(bsan) { 8 } else { 200 };
        let mut held = Vec::new();
        for round in 0..rounds {
            let repo = match round % 3 {
                0 => Repository::open(TESTREPO),
                1 => Repository::open_bare(TESTREPO),
                _ => Repository::new(),
            };
            held.push(repo.unwrap());
            if held.len() > 4 {
                drop(held.remove(round % held.len()));
            }
        }
    }

    /// Threads build, use and free their own repositories concurrently, over the
    /// same repository on disk and in directories of their own.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_build_their_own_repositories() {
        let scratch = Scratch::new("threads");
        let threads = if cfg!(bsan) { 2 } else { 4 };
        let rounds = if cfg!(bsan) { 2 } else { 20 };
        let barrier = Barrier::new(threads);
        thread::scope(|s| {
            for t in 0..threads {
                let (barrier, scratch) = (&barrier, &scratch);
                s.spawn(move || {
                    barrier.wait();
                    for round in 0..rounds {
                        exercise(Repository::open(TESTREPO).unwrap());
                        exercise(Repository::open_bare(TESTREPO).unwrap());
                        exercise(Repository::new().unwrap());
                        let path = scratch.child(&format!("{t}-{round}"));
                        exercise(Repository::init(&path, round % 2 == 0).unwrap());
                        assert!(Repository::open(&scratch.child("missing")).is_err());
                    }
                });
            }
        });
    }

    /// The first libgit2 calls in the process are repository constructors racing on
    /// several threads: each pins the runtime before it allocates.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn first_constructors_race_to_initialise() {
        isolated(
            concat!(module_path!(), "::first_constructors_race_to_initialise"),
            || {
                let barrier = Barrier::new(4);
                thread::scope(|s| {
                    for t in 0..4 {
                        let barrier = &barrier;
                        s.spawn(move || {
                            barrier.wait();
                            let repo = match t {
                                0 => Repository::new(),
                                1 => Repository::open(TESTREPO),
                                2 => Repository::open_bare(TESTREPO),
                                _ => Repository::with_oid_type(OidType::Sha256),
                            };
                            exercise(repo.unwrap());
                        });
                    }
                });
            },
        );
    }

    /// The first libgit2 call in the process is a constructor that fails: it pins the
    /// runtime, so its error is read from live thread state.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_failing_first_constructor_reports_its_error() {
        isolated(
            concat!(
                module_path!(),
                "::a_failing_first_constructor_reports_its_error"
            ),
            || {
                let error = Repository::open(c"").unwrap_err();
                assert_eq!(error.code(), Some(ErrorCode::NotFound));
                assert_eq!(
                    error.message().to_str(),
                    Ok("failed to resolve path '': No such file or directory")
                );
            },
        );
    }

    /// The owner is one pointer, and an absent one is the NULL niche.
    #[test]
    fn the_owner_is_a_pointer() {
        assert_eq!(size_of::<RepositoryBox>(), size_of::<*mut u8>());
        assert_eq!(
            size_of::<Option<RepositoryBox>>(),
            size_of::<RepositoryBox>()
        );
    }
}

/// The init options `InitOptions` (`git_repository_init_options`) and its handles.
///
/// The options store pointers to the strings their setters borrow and read them back
/// through the getters; every source here is a `CString`, an allocation of exactly its
/// length, so ASan flags a read past one, and each is released after the options, once
/// (ASan, LSan). The getters' strings outlive the handles and the options, since they
/// point at the sources. Every getter and setter runs on every holder Rust has for the
/// options (a local, a heap slot, an element of a run that moves when it grows, a
/// copy), through reborrows, and across threads under the `Send` / `Sync` the crate
/// asserts (TSan); `new` runs on many threads before anything initialised libgit2
/// (`git_repository_init_options_init` copies a template, touching no global state).
/// libgit2 reads the options in `git_repository_init_ext`, standing in for its wrapper
/// in `fixture`: every string is read in bounds (ASan), the flags it ORs in are written
/// through the exclusive borrow, and nothing is kept past the call, so the options may
/// be reused, moved to another thread or dropped right after (BSan: a foreign write
/// between Rust reborrows, no retained pointer). The object id type and reference
/// storage only take named values, and a string, the options' handles and borrows that
/// outlive their sources do not compile (`compile_fail/`, the type's doctests). Every
/// test builds its options through `git_repository_init_options_init`, so Miri skips
/// them; the inline unit tests run the accessors under Miri over a Rust-built value.
mod init_options {
    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Barrier, Mutex};
    use std::thread;

    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::refs::refdb::RefdbType;
    use libgit2::libgit2::repository::repository::{
        InitFlag, InitFlags, InitMode, InitOptions, InitOptionsMut, InitOptionsRef,
    };

    use crate::fixture::repository_options::{init_ext, oid_type};
    use crate::fixture::{init, isolated};

    /// `n` distinct strings of lengths `base + 1`, `base + 2`, ...
    fn strings(base: usize, n: usize) -> Vec<CString> {
        (1..=n)
            .map(|i| CString::new(vec![b'a' + ((base + i) % 26) as u8; base + i]).unwrap())
            .collect()
    }

    /// The five string getters, in field order.
    fn strs<'o>(o: InitOptionsRef<'_, 'o>) -> [Option<&'o CStr>; 5] {
        [
            o.workdir_path(),
            o.description(),
            o.template_path(),
            o.initial_head(),
            o.origin_url(),
        ]
    }

    /// Every value and byte the getters reach.
    fn read_all(o: InitOptionsRef<'_, '_>) -> (u32, u32, u32, Vec<Vec<u8>>, String) {
        let s = strs(o)
            .iter()
            .map(|s| s.map_or(b"-".to_vec(), |s| s.to_bytes().to_vec()))
            .collect();
        let typed = format!("{:?} {:?}", o.oid_type(), o.refdb_type());
        (
            o.version(),
            o.flags().bits(),
            o.mode().raw(),
            s,
            typed + &format!("{o:?}"),
        )
    }

    const OIDS: [Option<OidType>; 3] = [None, Some(OidType::Sha1), Some(OidType::Sha256)];
    const REFDBS: [Option<RefdbType>; 3] =
        [None, Some(RefdbType::Files), Some(RefdbType::Reftable)];

    /// Writes every field for `round`, the strings from `s` (or none on every fourth
    /// round).
    fn write<'o>(mut m: InitOptionsMut<'_, 'o>, s: &'o [CString], round: u32) {
        let pick = |i: usize| (round % 4 != 3).then(|| s[i].as_c_str());
        m.set_version(round % 3);
        m.set_flags(InitFlags::from_bits_retain(round.rotate_left(round)));
        m.set_mode(InitMode::from_raw(round.wrapping_mul(0o1234567)));
        m.set_workdir_path(pick(0));
        m.set_description(pick(1));
        m.set_template_path(pick(2));
        m.set_initial_head(pick(3));
        m.set_origin_url(pick(4));
        m.set_oid_type(OIDS[round as usize % 3]);
        m.set_refdb_type(REFDBS[round as usize / 3 % 3]);
    }

    /// What [`write`] stored, read back independently of the options.
    fn expected(s: &[CString], round: u32) -> (u32, u32, u32, Vec<Vec<u8>>) {
        let strs = (0..5)
            .map(|i| {
                if round % 4 == 3 {
                    b"-".to_vec()
                } else {
                    s[i].to_bytes().to_vec()
                }
            })
            .collect();
        (
            round % 3,
            round.rotate_left(round),
            round.wrapping_mul(0o1234567),
            strs,
        )
    }

    fn values(o: InitOptionsRef<'_, '_>) -> (u32, u32, u32, Vec<Vec<u8>>) {
        let (v, f, m, s, _) = read_all(o);
        (v, f, m, s)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn defaults_then_every_setter_on_every_holder() {
        let pool: Vec<Vec<CString>> = (0..64).map(|r| strings(r, 5)).collect();
        let mut local = InitOptions::new();
        let o = local.as_ref();
        assert_eq!(
            (o.version(), o.flags(), o.mode()),
            (1, InitFlags::empty(), InitMode::SharedUmask)
        );
        assert_eq!(strs(o), [None; 5]);
        assert_eq!((o.oid_type(), o.refdb_type()), (None, None));
        let mut boxed = Box::new(InitOptions::new());
        let mut run = vec![InitOptions::new()];
        for round in 0..64u32 {
            let s = &pool[round as usize];
            write(local.as_mut(), s, round);
            write(InitOptions::as_mut(&mut boxed), s, round);
            // Growing the run moves its elements: every handle is taken afresh.
            run.push(InitOptions::new());
            let last = run.len() - 1;
            write(run[last].as_mut(), s, round);
            for opts in [&local, &*boxed, &run[last]] {
                assert_eq!(values(opts.as_ref()), expected(s, round), "round {round}");
                assert_eq!(opts.as_ref().oid_type(), OIDS[round as usize % 3]);
                assert_eq!(opts.as_ref().refdb_type(), REFDBS[round as usize / 3 % 3]);
                let copy = opts.as_ref().to_options();
                assert_eq!(read_all(copy.as_ref()), read_all(opts.as_ref()));
            }
        }
        // Every element of the run still reads what it was given.
        for (i, opts) in run.iter().enumerate().skip(1) {
            let round = i as u32 - 1;
            assert_eq!(
                values(opts.as_ref()),
                expected(&pool[round as usize], round)
            );
        }
        drop((local, boxed, run));
        drop(pool);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_outlive_the_handles_and_the_options() {
        let s = strings(7, 5);
        let read = {
            let mut opts = InitOptions::new();
            write(opts.as_mut(), &s, 1);
            let r = opts.as_ref();
            strs(r)
        };
        // The options are gone; what they pointed at is not.
        let bytes: Vec<_> = read.iter().map(|s| s.unwrap().to_bytes()).collect();
        let want: Vec<_> = s.iter().map(|s| s.to_bytes()).collect();
        assert_eq!(bytes, want);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reborrows_keep_one_writer() {
        let s = strings(3, 5);
        let mut opts = InitOptions::new();
        let mut m = opts.as_mut();
        {
            let mut inner = m.as_mut();
            inner.set_initial_head(Some(&s[0]));
            inner.set_flags(InitFlag::Bare.into());
            assert_eq!(inner.as_ref().initial_head(), Some(s[0].as_c_str()));
        }
        let shared = m.as_ref();
        let copy = shared;
        assert_eq!(copy.flags(), InitFlag::Bare.into());
        assert!(!format!("{m:?}").is_empty());
        m.set_origin_url(Some(&s[1]));
        m.set_initial_head(None);
        let o = opts.as_ref();
        assert_eq!(
            (o.initial_head(), o.origin_url()),
            (None, Some(s[1].as_c_str()))
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_are_independent() {
        let s = strings(11, 5);
        let t = strings(13, 5);
        let mut a = InitOptions::new();
        write(a.as_mut(), &s, 5);
        let mut b = a;
        write(b.as_mut(), &t, 6);
        assert_eq!(values(a.as_ref()), expected(&s, 5));
        assert_eq!(values(b.as_ref()), expected(&t, 6));
        let c = a.as_ref().to_options();
        write(a.as_mut(), &t, 7);
        assert_eq!(values(c.as_ref()), expected(&s, 5));
        assert_eq!(values(a.as_ref()), expected(&t, 7));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_handles_read_across_threads() {
        let s = strings(2, 5);
        let mut opts = InitOptions::new();
        write(opts.as_mut(), &s, 9);
        let r = opts.as_ref();
        let (whole, s) = (&opts, &s);
        thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(move || {
                    for _ in 0..200 {
                        assert_eq!(values(r), expected(s, 9));
                        assert_eq!(values(whole.as_ref()), expected(s, 9));
                    }
                });
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn values_and_exclusive_handles_move_across_threads() {
        let s = strings(4, 5);
        let mut opts = InitOptions::new();
        thread::scope(|scope| {
            let m = opts.as_mut();
            let s = &s;
            scope.spawn(move || write(m, s, 10)).join().unwrap();
        });
        assert_eq!(values(opts.as_ref()), expected(&s, 10));
        let moved = thread::scope(|scope| {
            scope
                .spawn(move || {
                    let mut opts = opts;
                    opts.as_mut().set_version(9);
                    opts
                })
                .join()
                .unwrap()
        });
        assert_eq!(moved.as_ref().version(), 9);
        assert_eq!(moved.as_ref().initial_head(), Some(s[3].as_c_str()));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn locked_options_have_one_writer_at_a_time() {
        let pool: Vec<Vec<CString>> = (0..4).map(|t| strings(t * 5, 5)).collect();
        let opts = Mutex::new(InitOptions::new());
        thread::scope(|scope| {
            for (t, s) in pool.iter().enumerate() {
                let opts = &opts;
                scope.spawn(move || {
                    for _ in 0..200 {
                        let mut guard = opts.lock().unwrap();
                        let n = guard.as_ref().version();
                        let mut m = guard.as_mut();
                        write(m.as_mut(), s, t as u32);
                        m.set_version(n + 1);
                    }
                });
            }
        });
        assert_eq!(opts.lock().unwrap().as_ref().version(), 4 * 200 + 1);
    }

    /// `new` copies libgit2's template into a fresh value on each call: concurrent
    /// calls share nothing for TSan to flag, and each result is the default.
    fn new_from_many_threads() {
        let default = format!("{:?}", InitOptions::default());
        thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        (0..100)
                            .map(|_| format!("{:?}", InitOptions::new()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for h in handles {
                assert!(h.join().unwrap().iter().all(|o| *o == default));
            }
        });
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

    /// A scratch directory, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-init-options-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn c_path(&self, rel: &str) -> CString {
            CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_ext_reads_the_borrowed_strings_in_bounds() {
        init();
        let root = Root::new("strings");
        let tpl = root.0.join("tpl");
        fs::create_dir_all(tpl.join("hooks")).unwrap();
        fs::write(tpl.join("hooks/post-commit"), "#!/bin/sh\n").unwrap();
        // Each string an allocation of exactly its length, released after the options.
        let workdir = root.c_path("work");
        let description = CString::new("the description").unwrap();
        let template = root.c_path("tpl");
        let head = CString::new("trunk").unwrap();
        let url = CString::new("https://example.com/r.git").unwrap();
        let repo_path = root.c_path("repo.git");
        let mut opts = InitOptions::new();
        let mut m = opts.as_mut();
        m.set_flags(InitFlag::Mkpath.into());
        m.set_workdir_path(Some(&workdir));
        m.set_description(Some(&description));
        m.set_initial_head(Some(&head));
        m.set_origin_url(Some(&url));
        let (rc, repo) = init_ext(&repo_path, &mut opts);
        assert_eq!(rc, 0);
        drop(repo);
        assert_eq!(opts.as_ref().initial_head(), Some(head.as_c_str()));
        assert_eq!(
            read(&root.0.join("repo.git/description")),
            "the description"
        );
        assert_eq!(
            read(&root.0.join("repo.git/HEAD")),
            "ref: refs/heads/trunk\n"
        );
        assert!(read(&root.0.join("repo.git/config")).contains("https://example.com/r.git"));
        assert!(read(&root.0.join("work/.git")).starts_with("gitdir: "));

        // The same options again: a reinitialisation, reading the strings once more.
        // It fails adding `origin` a second time (`GIT_EEXISTS`), after opening the
        // repository, which it leaves in the slot: released here, once (LSan, ASan).
        let (rc, repo) = init_ext(&repo_path, &mut opts);
        assert_eq!(rc, -4);
        assert!(repo.is_some());
        drop(repo);
        // libgit2 wrote its reinitialisation bit through the exclusive borrow; the
        // handle taken afresh reads it, and the strings are untouched.
        assert_eq!(opts.as_ref().flags().unnamed_bits(), 1 << 18);
        assert_eq!(strs(opts.as_ref())[4], Some(url.as_c_str()));

        // A bare SHA-256 repository from an external template, on another thread.
        let bare = root.c_path("bare");
        let mut opts = InitOptions::new();
        let mut m = opts.as_mut();
        m.set_flags(InitFlag::Bare | InitFlag::Mkdir);
        m.set_mode(InitMode::SharedGroup);
        m.set_template_path(Some(&template));
        m.set_oid_type(Some(OidType::Sha256));
        let kind = thread::scope(|s| {
            s.spawn(|| {
                let (rc, repo) = init_ext(&bare, &mut opts);
                assert_eq!(rc, 0);
                oid_type(&repo.unwrap())
            })
            .join()
            .unwrap()
        });
        assert_eq!(kind, OidType::Sha256.raw());
        assert!(root.0.join("bare/hooks/post-commit").exists());
        assert_eq!(opts.as_ref().template_path(), Some(template.as_c_str()));
        // The options' last use is above; the strings they borrowed go only now.
        drop((workdir, description, template, head, url));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_ext_refuses_the_versions_it_does_not_know() {
        init();
        let root = Root::new("versions");
        for version in [0, 2, u32::MAX] {
            let mut opts = InitOptions::new();
            opts.as_mut().set_version(version);
            let (rc, repo) = init_ext(&root.c_path("never"), &mut opts);
            assert_eq!(rc, -1, "version {version}");
            assert!(repo.is_none());
            assert!(!root.0.join("never").exists());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_on_threads_while_libgit2_initialises() {
        // One thread initialises libgit2 for the first time while others build and
        // write options: `new` touches no state the initialisation sets up.
        isolated(
            concat!(
                module_path!(),
                "::options_on_threads_while_libgit2_initialises"
            ),
            || {
                let barrier = Barrier::new(5);
                let s = strings(1, 5);
                thread::scope(|scope| {
                    scope.spawn(|| {
                        barrier.wait();
                        init();
                    });
                    for t in 0..4 {
                        let (barrier, s) = (&barrier, &s);
                        scope.spawn(move || {
                            barrier.wait();
                            for _ in 0..50 {
                                let mut opts = InitOptions::new();
                                write(opts.as_mut(), s, t);
                                assert_eq!(values(opts.as_ref()), expected(s, t));
                            }
                        });
                    }
                });
            },
        );
    }
}

/// The repository routines of `git2/repository.h` this crate wraps beyond the
/// constructors: `Repository::wrap_odb`, `RepositoryRef::workdir` and
/// `RepositoryMut::cleanup_state`.
///
/// `wrap_odb` consumes the caller's odb reference once the repository holds its own:
/// the odb must stay live through the repository (ASan: use after free), be freed with
/// it (LSan) and never twice (ASan: double free), and only an odb no repository owns is
/// accepted (an odb a repository lent does not compile: doctest on `wrap_odb`). The
/// working directory `workdir` returns is the repository's string, borrowed for the
/// handle's lifetime (ASan; outliving the repository does not compile: doctest on
/// `workdir`). `cleanup_state` removes files through the exclusive handle, and refuses
/// a repository without a git directory, which libgit2 would resolve against the
/// process's working directory. A repository is neither `Send` nor `Sync`, so nothing
/// races (TSan); libgit2 keeps no pointer into Rust storage (BSan). Every test calls
/// libgit2, so none runs under Miri.
mod routines {
    use std::ffi::CString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use libgit2::libgit2::odb::odb::Odb;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::Repository;

    use crate::fixture::odb::{Repo, backends, write_blob};

    fn scratch(tag: &str) -> PathBuf {
        crate::fixture::init();
        let path = std::env::temp_dir().join(format!(
            "crustify-repository-routines-ub-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn c_path(path: &Path) -> CString {
        CString::new(path.to_str().unwrap()).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn wrapped_odb_lives_as_long_as_the_repository() {
        crate::fixture::init();
        for kind in [OidType::Sha1, OidType::Sha256] {
            let odb = Odb::with_oid_type(kind).unwrap();
            let repo = Repo::wrap(odb);
            // The caller's reference is gone; the repository's keeps the odb.
            let lent = repo.odb();
            // No backends: the odb answers, and refuses the write.
            assert_eq!(backends(&lent), 0);
            assert!(write_blob(&lent, b"x").is_err());
            drop(lent);
            drop(repo);
        }
        for _ in 0..16 {
            let repo = Repository::wrap_odb(Odb::new().unwrap()).unwrap();
            assert_eq!(repo.as_ref().workdir(), None);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn workdir_is_borrowed_from_the_repository() {
        let dir = scratch("workdir");
        let repo = Repository::init(&c_path(&dir), false).unwrap();
        let workdir = repo.as_ref().workdir().unwrap();
        let copy = repo.as_ref();
        assert_eq!(copy.workdir(), Some(workdir));
        assert_eq!(
            workdir.to_str().unwrap(),
            format!("{}/", dir.to_str().unwrap())
        );
        drop(repo);
        let bare = Repository::init(&c_path(&dir.join("bare")), true).unwrap();
        assert_eq!(bare.as_ref().workdir(), None);
        drop(bare);
        assert_eq!(Repository::new().unwrap().as_ref().workdir(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleanup_state_removes_through_the_exclusive_handle() {
        let dir = scratch("cleanup");
        let mut repo = Repository::init(&c_path(&dir), true).unwrap();
        for round in 0..4 {
            fs::write(dir.join("MERGE_HEAD"), "x\n").unwrap();
            fs::create_dir_all(dir.join("rebase-merge/sub")).unwrap();
            fs::write(dir.join("rebase-merge/sub/f"), "x\n").unwrap();
            fs::write(dir.join("REVERT_HEAD"), "x\n").unwrap();
            let mut m = repo.as_mut();
            m.as_mut().cleanup_state().unwrap();
            assert!(!dir.join("MERGE_HEAD").exists(), "round {round}");
            assert!(!dir.join("rebase-merge").exists());
            assert!(!dir.join("REVERT_HEAD").exists());
        }
        // Nothing left to remove succeeds too.
        repo.as_mut().cleanup_state().unwrap();
        drop(repo);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleanup_state_refuses_repositories_without_paths() {
        crate::fixture::init();
        let mut memory = Repository::new().unwrap();
        let mut wrapped = Repository::wrap_odb(Odb::new().unwrap()).unwrap();
        for _ in 0..4 {
            assert!(memory.as_mut().cleanup_state().is_err());
            assert!(wrapped.as_mut().cleanup_state().is_err());
        }
    }
}
