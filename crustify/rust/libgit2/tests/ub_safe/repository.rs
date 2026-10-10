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
