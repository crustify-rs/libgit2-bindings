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
