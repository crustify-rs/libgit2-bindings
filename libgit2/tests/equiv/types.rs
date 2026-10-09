//! Equivalence of `git2/types.h`'s enums with the values libgit2 declares, accepts,
//! reads, reports and writes, then of [`Time`] with the `git_time` values it builds.
//!
//! [`ObjectType`] must name exactly the values libgit2 treats as an object's type:
//! those `git_object_type_is_valid` accepts and `git_object_type2string` names, each
//! with the C value `git_object_string2type` reads its name back as. Through an object
//! database, a sequence of `git_odb_write` and `git_odb_read_header` calls over one
//! repository, value by value, must succeed exactly for the values
//! [`ObjectType::from_raw`] accepts and report each object back with the type it was
//! written as; every other value, [`ObjectType::raw_or_any`]'s `GIT_OBJECT_ANY`
//! included, must fail as an invalid object type. Each step is compared as it runs.
//!
//! [`ReferenceType`] must name what `git_reference_type` answers for the references
//! libgit2 builds: a fresh repository's symbolic `HEAD`, and a direct reference
//! created over an object, each read back after a lookup.
//!
//! Documented divergences for object and reference types, outside these comparisons:
//!
//! - `git_odb_write` accepts the unnamed type `0`: it hashes the data as a blob but
//!   stores it under an empty type name, so `git_odb_read_header` cannot read the
//!   object back, and a later write of the same blob is skipped as already present.
//!   [`ObjectType`] cannot carry `0`; the C behaviour is pinned here alone.
//! - `git_reference_type(NULL)` returns `-1` through the unsigned `git_reference_t`
//!   in a release build, which [`ReferenceType::from_raw`] refuses; a debug build's C
//!   assertion aborts instead, so that probe is skipped under the `coverage` variant,
//!   the one debug build.
//!
//! [`SubmoduleRecurse`] and [`SubmoduleUpdate`] must each name exactly the values of
//! its C enum, each with its C value, except `GIT_SUBMODULE_UPDATE_DEFAULT`, which
//! libgit2 never produces and refuses to write. libgit2 observes the types through a
//! submodule: `git_submodule_lookup` reads the `.gitmodules` setting into the value
//! `git_submodule_update_strategy` / `git_submodule_fetch_recurse_submodules` report,
//! and `git_submodule_set_update` / `git_submodule_set_fetch_recurse_submodules` write
//! a value back to `.gitmodules`, or refuse it; `git_submodule_init` copies the update
//! setting into the repository's config by name. Each probe runs the C routines on a
//! fresh repository and predicts their result from the Rust conversion alone; a
//! sequence of writes is compared after every step, both the value reported back and
//! the line `.gitmodules` records.
//!
//! Recorded C behaviour, outside the types: a setting libgit2 cannot read fails the
//! lookup (`GIT_EINVALID`), and `git_submodule_set_fetch_recurse_submodules` writes
//! `No` by deleting the setting, failing with `GIT_ENOTFOUND` when there is none.
//!
//! Then [`Time`], against the `git_time` values libgit2 builds, parses and copies.
//!
//! libgit2 has no routine of its own over a bare `git_time`: it builds one inside a
//! signature. [`Time::new`] must build what `git_signature_new` stores in `when` for
//! the same seconds and offset; the getters must read back exactly what
//! `git_signature_from_buffer` parses into `when`, sign bytes it copies from a
//! malformed zone included; and the setters must write what C code writing the same
//! fields writes, as `git_signature_dup` copies it. Each sequence of writes runs on a
//! Rust `Time` and on a C-allocated `git_signature` independently, and the two are
//! compared after every step.
//!
//! Documented divergences: none. A `Time` is a plain value, so the comparisons are
//! field by field (`git_signature__equal`'s rule) and never cover padding.
//!
//! Then [`FileMode`], against the `git_filemode_t` values libgit2 declares, accepts
//! and returns.
//!
//! [`FileMode`] must name exactly the `git_filemode_t` constants, each with its C
//! value. libgit2 observes a mode through its tree API, and each probe predicts the C
//! result from the Rust conversion alone:
//!
//! - `git_treebuilder_insert` accepts exactly the named modes but
//!   [`FileMode::Unreadable`], and refuses the rest with `GIT_ERROR_TREE` while
//!   leaving the builder as it was; an accepted entry reads back its mode through
//!   `git_tree_entry_filemode` and `git_tree_entry_filemode_raw`, in the builder and
//!   in the tree it writes. One builder takes a sequence of insertions, compared
//!   after every step. Every accepted mode but [`FileMode::Commit`] must also name an
//!   object of its kind.
//! - A parsed tree keeps whatever 16-bit mode its object spells:
//!   `git_tree_entry_filemode_raw` returns it, which [`FileMode::from_raw`] names
//!   exactly when the header does, and `git_tree_entry_filemode` normalises it to a
//!   mode `from_raw` names and the builder accepts; a mode past 16 bits fails the
//!   parse.
//!
//! Then `git_writestream`, in `write_streams`: C blob streams written through the
//! wrappers, and Rust streams driven by libgit2, against their C counterparts.
//!
//! Then [`SubmoduleIgnore`], against the `git_submodule_ignore_t` values libgit2's
//! submodule routines produce and consume.
//!
//! - Producer: `git_submodule_ignore` over a submodule whose `.gitmodules` entry spells
//!   each value C's config map accepts. [`SubmoduleIgnore::from_raw`] must name every
//!   value C returns and convert it back unchanged.
//! - Consumer: `git_submodule_set_ignore` with each raw value in and around the named
//!   range, on two independent repositories: C passes the raw value, Rust passes the
//!   variant's [`raw`](SubmoduleIgnore::raw) or, for a value without one, cannot call C
//!   at all. Both record the return, the error, `.gitmodules` and the value read back.
//!
//! Documented divergences, outside these comparisons: `git_submodule_set_ignore`
//! refuses `GIT_SUBMODULE_IGNORE_UNSPECIFIED` like any unnamed value, but `Unspecified`
//! is a variant (status queries take it), so a typed setter wrapper must refuse it
//! itself.
//!
//! Then [`BranchType`] against `git_branch_t` as libgit2 reads and writes it:
//! `git_branch_iterator_new` filters by it and `git_branch_next` reports it. On the
//! `testrepo.git` fixture, which has local branches and one remote-tracking branch, the C
//! side lists the branches for every raw selection; the Rust side predicts each listing
//! from the full one with [`BranchType::contains`] and converts every reported kind with
//! [`BranchType::from_raw`]. An iterator takes any bits, listing nothing for `0` and
//! ignoring bits beyond `GIT_BRANCH_ALL`; [`BranchType`] names only the three values every
//! branch routine accepts (`git_branch_lookup` asserts on the rest), so
//! [`BranchType::from_raw`] rejects those selections, which the test checks list nothing
//! new: a documented divergence.

use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::core::types::{
    BranchType, FileMode, NotAReferenceType, NotAnObjectType, ObjectType, ReferenceType,
    SubmoduleIgnore, SubmoduleRecurse, SubmoduleUpdate, Time, UnnamedFileMode,
    UnnamedSubmoduleRecurse, UnnamedSubmoduleUpdate,
};

/// Every `git_submodule_recurse_t` constant, beside the variant that must carry it.
const RECURSE: [(ffi::git_submodule_recurse_t, SubmoduleRecurse); 3] = [
    (ffi::GIT_SUBMODULE_RECURSE_NO, SubmoduleRecurse::No),
    (ffi::GIT_SUBMODULE_RECURSE_YES, SubmoduleRecurse::Yes),
    (
        ffi::GIT_SUBMODULE_RECURSE_ONDEMAND,
        SubmoduleRecurse::OnDemand,
    ),
];

/// Every `git_submodule_update_t` constant with a variant, beside it.
const UPDATE: [(ffi::git_submodule_update_t, SubmoduleUpdate); 4] = [
    (
        ffi::GIT_SUBMODULE_UPDATE_CHECKOUT,
        SubmoduleUpdate::Checkout,
    ),
    (ffi::GIT_SUBMODULE_UPDATE_REBASE, SubmoduleUpdate::Rebase),
    (ffi::GIT_SUBMODULE_UPDATE_MERGE, SubmoduleUpdate::Merge),
    (ffi::GIT_SUBMODULE_UPDATE_NONE, SubmoduleUpdate::None),
];

/// The submodule every probe repository declares.
const NAME: &CStr = c"sm";

/// Every value around the declared ones, and the extremes.
fn probes() -> impl Iterator<Item = u32> {
    (0..=16).chain([i32::MAX as u32, u32::MAX - 1, u32::MAX])
}

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL after init, and its message is a
    // NUL-terminated string, both live until the next libgit2 call on this thread,
    // after the copy.
    unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
    }
}

/// A fresh non-bare repository whose `.gitmodules` declares submodule `sm`, open
/// until dropped.
struct Superproject {
    path: PathBuf,
    repo: *mut ffi::git_repository,
}

impl Superproject {
    /// The repository, its `.gitmodules` declaring `sm` with the `extra` lines.
    fn new(tag: &str, extra: &str) -> Self {
        init();
        let path =
            std::env::temp_dir().join(format!("crustify-types-equiv-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_MKDIR;
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` a live
        // options struct, all read for the call only; the result is NULL or a fresh
        // repository, which `Drop` releases.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0, "{:?}", c_last());
        let this = Self { path, repo };
        this.write_gitmodules(extra);
        this
    }

    fn gitmodules(&self) -> PathBuf {
        self.path.join(".gitmodules")
    }

    fn write_gitmodules(&self, extra: &str) {
        fs::write(
            self.gitmodules(),
            format!(
                "[submodule \"sm\"]\n\tpath = sm\n\turl = https://example.invalid/sm.git\n{extra}"
            ),
        )
        .unwrap();
    }

    /// The `key` line of `.gitmodules`, trimmed, if any.
    fn setting(&self, key: &str) -> Option<String> {
        fs::read_to_string(self.gitmodules())
            .unwrap()
            .lines()
            .map(str::trim)
            .find(|l| l.split(' ').next() == Some(key))
            .map(str::to_owned)
    }

    /// What a fresh `git_submodule_lookup` of `sm` reports: its update strategy and
    /// fetch recursion, or the lookup's return and error.
    fn lookup(
        &self,
    ) -> Result<(ffi::git_submodule_update_t, ffi::git_submodule_recurse_t), (c_int, (c_int, String))>
    {
        let mut sm = ptr::null_mut();
        // SAFETY: `sm` is a live slot, `self.repo` a live repository and `NAME`
        // NUL-terminated, read for the call only; the result is NULL or a fresh
        // submodule.
        let rc = unsafe { ffi::git_submodule_lookup(&mut sm, self.repo, NAME.as_ptr()) };
        if rc != 0 {
            return Err((rc, c_last()));
        }
        // SAFETY: `sm` is the live submodule the lookup produced, released here
        // exactly once after both reads.
        unsafe {
            let read = (
                ffi::git_submodule_update_strategy(sm),
                ffi::git_submodule_fetch_recurse_submodules(sm),
            );
            ffi::git_submodule_free(sm);
            Ok(read)
        }
    }

    /// `git_submodule_init(sm, overwrite = 1)` on a fresh lookup of `sm`, which copies
    /// its settings into the repository's own config: its return, and the error on
    /// failure.
    fn init_submodule(&self) -> Result<(), (c_int, (c_int, String))> {
        let mut sm = ptr::null_mut();
        // SAFETY: as in `lookup`.
        let rc = unsafe { ffi::git_submodule_lookup(&mut sm, self.repo, NAME.as_ptr()) };
        if rc != 0 {
            return Err((rc, c_last()));
        }
        // SAFETY: `sm` is the live submodule the lookup produced, released here
        // exactly once after the call.
        let rc = unsafe {
            let rc = ffi::git_submodule_init(sm, 1);
            ffi::git_submodule_free(sm);
            rc
        };
        if rc == 0 { Ok(()) } else { Err((rc, c_last())) }
    }

    /// The `submodule.sm.<key>` line of the repository's own config, trimmed, if any.
    fn config_setting(&self, key: &str) -> Option<String> {
        fs::read_to_string(self.path.join(".git/config"))
            .unwrap()
            .lines()
            .map(str::trim)
            .find(|l| l.split(' ').next() == Some(key))
            .map(str::to_owned)
    }

    /// `git_submodule_set_update(repo, "sm", raw)`: its return, and the error on
    /// failure.
    fn set_update(&self, raw: ffi::git_submodule_update_t) -> Result<(), (c_int, (c_int, String))> {
        // SAFETY: `self.repo` is a live repository and `NAME` NUL-terminated, both
        // read for the call only; `raw` is any value, which C maps or refuses.
        let rc = unsafe { ffi::git_submodule_set_update(self.repo, NAME.as_ptr(), raw) };
        if rc == 0 { Ok(()) } else { Err((rc, c_last())) }
    }

    /// `git_submodule_set_fetch_recurse_submodules(repo, "sm", raw)`.
    fn set_recurse(
        &self,
        raw: ffi::git_submodule_recurse_t,
    ) -> Result<(), (c_int, (c_int, String))> {
        // SAFETY: as in `set_update`.
        let rc = unsafe {
            ffi::git_submodule_set_fetch_recurse_submodules(self.repo, NAME.as_ptr(), raw)
        };
        if rc == 0 { Ok(()) } else { Err((rc, c_last())) }
    }
}

impl Drop for Superproject {
    fn drop(&mut self) {
        // SAFETY: `self.repo` is the live repository `new` produced, released once.
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The error a setter reports for a value its map does not hold.
fn refused(var: &str) -> (c_int, (c_int, String)) {
    (
        -1,
        (
            ffi::GIT_ERROR_SUBMODULE as c_int,
            format!("invalid value for {var}"),
        ),
    )
}

#[test]
fn variants_carry_the_c_values() {
    for raw in probes() {
        let recurse = RECURSE.iter().find(|(c, _)| *c == raw).map(|(_, v)| *v);
        assert_eq!(SubmoduleRecurse::from_raw(raw), recurse, "recurse {raw}");
        assert_eq!(
            SubmoduleRecurse::try_from(raw).map_err(UnnamedSubmoduleRecurse::raw),
            recurse.ok_or(raw)
        );
        let update = UPDATE.iter().find(|(c, _)| *c == raw).map(|(_, v)| *v);
        assert_eq!(SubmoduleUpdate::from_raw(raw), update, "update {raw}");
        assert_eq!(
            SubmoduleUpdate::try_from(raw).map_err(UnnamedSubmoduleUpdate::raw),
            update.ok_or(raw)
        );
    }
    for (c, v) in RECURSE {
        assert_eq!((v.raw(), ffi::git_submodule_recurse_t::from(v)), (c, c));
    }
    for (c, v) in UPDATE {
        assert_eq!((v.raw(), ffi::git_submodule_update_t::from(v)), (c, c));
    }
    // The one named value without a variant.
    assert_eq!(
        SubmoduleUpdate::from_raw(ffi::GIT_SUBMODULE_UPDATE_DEFAULT),
        None
    );
}

/// A submodule that configures neither setting reports the Rust defaults.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unconfigured_settings_read_as_the_defaults() {
    let sp = Superproject::new("unconfigured", "");
    let (update, recurse) = sp.lookup().unwrap();
    assert_eq!(
        SubmoduleUpdate::from_raw(update),
        Some(SubmoduleUpdate::default())
    );
    assert_eq!(
        SubmoduleRecurse::from_raw(recurse),
        Some(SubmoduleRecurse::default())
    );
}

/// Every spelling libgit2 reads a setting from reports a value the Rust type names.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn configured_settings_read_as_the_rust_type() {
    let update = [
        ("checkout", SubmoduleUpdate::Checkout),
        ("rebase", SubmoduleUpdate::Rebase),
        ("merge", SubmoduleUpdate::Merge),
        ("none", SubmoduleUpdate::None),
        ("ReBaSe", SubmoduleUpdate::Rebase),
        ("true", SubmoduleUpdate::Checkout),
        ("false", SubmoduleUpdate::None),
        ("yes", SubmoduleUpdate::Checkout),
        ("off", SubmoduleUpdate::None),
    ];
    for (i, (text, want)) in update.into_iter().enumerate() {
        let sp = Superproject::new(&format!("update-{i}"), &format!("\tupdate = {text}\n"));
        let (raw, _) = sp.lookup().unwrap();
        assert_eq!(
            SubmoduleUpdate::from_raw(raw),
            Some(want),
            "update = {text}"
        );
    }
    let recurse = [
        ("on-demand", SubmoduleRecurse::OnDemand),
        ("On-Demand", SubmoduleRecurse::OnDemand),
        ("true", SubmoduleRecurse::Yes),
        ("false", SubmoduleRecurse::No),
        ("1", SubmoduleRecurse::Yes),
        ("0", SubmoduleRecurse::No),
        ("yes", SubmoduleRecurse::Yes),
        ("off", SubmoduleRecurse::No),
    ];
    for (i, (text, want)) in recurse.into_iter().enumerate() {
        let sp = Superproject::new(
            &format!("recurse-{i}"),
            &format!("\tfetchRecurseSubmodules = {text}\n"),
        );
        let (_, raw) = sp.lookup().unwrap();
        assert_eq!(
            SubmoduleRecurse::from_raw(raw),
            Some(want),
            "fetchRecurseSubmodules = {text}"
        );
    }
}

/// A sequence of writes: each variant is written and reported back as itself, each
/// value `from_raw` refuses is refused by C and leaves the setting as it was.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn update_writes_round_trip() {
    let sp = Superproject::new("update-writes", "");
    let mut current = SubmoduleUpdate::default();
    // Nothing written until the first variant: an unconfigured `update`.
    let mut written = None;
    let steps = [0, 2, 1, 0, 3, 4, 5, 1, 4, u32::MAX, 2, 3, 1];
    for (step, raw) in steps.into_iter().enumerate() {
        let result = sp.set_update(raw);
        match SubmoduleUpdate::from_raw(raw) {
            Some(v) => {
                assert_eq!(result, Ok(()), "step {step}");
                current = v;
                written = Some(match v {
                    SubmoduleUpdate::Checkout => "update = checkout",
                    SubmoduleUpdate::Rebase => "update = rebase",
                    SubmoduleUpdate::Merge => "update = merge",
                    SubmoduleUpdate::None => "update = none",
                    other => unreachable!("{other:?}"),
                });
            }
            None => assert_eq!(result, Err(refused("update")), "step {step}"),
        }
        let (update, recurse) = sp.lookup().unwrap();
        assert_eq!(
            SubmoduleUpdate::from_raw(update),
            Some(current),
            "step {step}"
        );
        assert_eq!(
            SubmoduleRecurse::from_raw(recurse),
            Some(SubmoduleRecurse::default())
        );
        assert_eq!(sp.setting("update").as_deref(), written, "step {step}");
    }
}

/// As [`update_writes_round_trip`], for the recursion setting. `No` is written by
/// deleting the setting, which then reads back as the default; with no setting to
/// delete, C fails with `GIT_ENOTFOUND` (a quirk of the setter, not of the value).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn recurse_writes_round_trip() {
    let sp = Superproject::new("recurse-writes", "");
    let mut current = SubmoduleRecurse::default();
    let steps = [3, 0, 1, 2, 3, 0, 1, 7, 2, u32::MAX, 0, 0, 1];
    for (step, raw) in steps.into_iter().enumerate() {
        let unset = sp.setting("fetchRecurseSubmodules").is_none();
        let result = sp.set_recurse(raw);
        match SubmoduleRecurse::from_raw(raw) {
            // C writes `No` by deleting the setting, and fails when there is none to
            // delete, though the submodule already reports `No`.
            Some(SubmoduleRecurse::No) if unset => {
                assert_eq!(current, SubmoduleRecurse::No);
                let missing = (
                    ffi::GIT_ERROR_CONFIG as c_int,
                    "could not find key 'submodule.sm.fetchRecurseSubmodules' to delete".to_owned(),
                );
                assert_eq!(result, Err((ffi::GIT_ENOTFOUND, missing)), "step {step}");
            }
            Some(v) => {
                assert_eq!(result, Ok(()), "step {step}");
                current = v;
            }
            None => assert_eq!(
                result,
                Err(refused("fetchRecurseSubmodules")),
                "step {step}"
            ),
        }
        let (update, recurse) = sp.lookup().unwrap();
        assert_eq!(
            SubmoduleRecurse::from_raw(recurse),
            Some(current),
            "step {step}"
        );
        assert_eq!(
            SubmoduleUpdate::from_raw(update),
            Some(SubmoduleUpdate::default())
        );
        let written = match current {
            SubmoduleRecurse::No => None,
            SubmoduleRecurse::Yes => Some("fetchRecurseSubmodules = true".to_owned()),
            SubmoduleRecurse::OnDemand => Some("fetchRecurseSubmodules = on-demand".to_owned()),
            other => unreachable!("{other:?}"),
        };
        assert_eq!(sp.setting("fetchRecurseSubmodules"), written, "step {step}");
    }
}

/// A spelling libgit2 does not read fails the lookup, so C never reports a value from
/// a setting it could not read.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unreadable_settings_fail_the_lookup() {
    for (i, (key, property)) in [("update", "update"), ("fetchRecurseSubmodules", "recurse")]
        .into_iter()
        .enumerate()
    {
        let sp = Superproject::new(&format!("unreadable-{i}"), &format!("\t{key} = bogus\n"));
        let message = format!("invalid value for submodule '{property}' property: 'bogus'");
        assert_eq!(
            sp.lookup(),
            Err((
                ffi::GIT_EINVALID,
                (ffi::GIT_ERROR_SUBMODULE as c_int, message)
            )),
            "{key}"
        );
    }
}

/// `git_submodule_init` copies the update setting into the repository's config by the
/// variant's name, and leaves the default (`Checkout`) unwritten.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn init_copies_the_update_setting_by_name() {
    let names = [
        (SubmoduleUpdate::Checkout, None),
        (SubmoduleUpdate::Rebase, Some("update = rebase")),
        (SubmoduleUpdate::Merge, Some("update = merge")),
        (SubmoduleUpdate::None, Some("update = none")),
    ];
    for (i, (v, written)) in names.into_iter().enumerate() {
        let sp = Superproject::new(&format!("init-{i}"), "");
        assert_eq!(sp.set_update(v.raw()), Ok(()));
        assert_eq!(sp.init_submodule(), Ok(()), "{v:?}");
        assert_eq!(sp.config_setting("update").as_deref(), written, "{v:?}");
    }
}

// --- `git_time` ------------------------------------------------------------

/// A C-allocated signature, freed by `git_signature_free` on drop.
struct Signature(*mut ffi::git_signature);

impl Signature {
    /// `git_signature_new(out, "n", "e@x", seconds, offset)`.
    fn new(seconds: i64, offset: i32) -> Signature {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the name and email are NUL-terminated,
        // without angle brackets, and not retained (libgit2 copies them).
        let rc = unsafe {
            ffi::git_signature_new(&mut out, c"n".as_ptr(), c"e@x".as_ptr(), seconds, offset)
        };
        assert_eq!(rc, 0);
        assert!(!out.is_null());
        Signature(out)
    }

    /// `git_signature_from_buffer(out, buf)`.
    fn parse(buf: &CStr) -> Signature {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `buf` NUL-terminated; neither is
        // retained.
        let rc = unsafe { ffi::git_signature_from_buffer(&mut out, buf.as_ptr()) };
        assert_eq!(rc, 0, "{buf:?}");
        assert!(!out.is_null());
        Signature(out)
    }

    /// `git_signature_dup(out, self)`.
    fn dup(&self) -> Signature {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `self.0` a live signature, only read.
        let rc = unsafe { ffi::git_signature_dup(&mut out, self.0) };
        assert_eq!(rc, 0);
        assert!(!out.is_null());
        Signature(out)
    }

    /// A copy of `when`.
    fn when(&self) -> ffi::git_time {
        // SAFETY: `self.0` is a live signature, read by value.
        unsafe { (*self.0).when }
    }

    /// Writes `when`, as C code holding the signature does.
    fn when_mut(&mut self) -> &mut ffi::git_time {
        // SAFETY: `self.0` is a live signature this value owns alone; the borrow ends
        // before any other access to it.
        unsafe { &mut (*self.0).when }
    }
}

impl Drop for Signature {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from a `git_signature_*` constructor and is freed once.
        unsafe { ffi::git_signature_free(self.0) }
    }
}

/// `time`'s fields, as the getters read them.
fn fields(time: &Time) -> (i64, i32, u8) {
    let r = time.as_ref();
    (r.seconds(), r.offset_minutes(), r.sign())
}

/// `raw`'s fields, as C reads them.
fn c_fields(raw: &ffi::git_time) -> (i64, i32, u8) {
    (raw.time, raw.offset, raw.sign as u8)
}

const SECONDS: [i64; 7] = [i64::MIN, -86_400, -1, 0, 1, 1_700_000_000, i64::MAX];
const OFFSETS: [i32; 11] = [i32::MIN, -899, -840, -60, -1, 0, 1, 60, 840, 899, i32::MAX];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_matches_git_signature_new() {
    init();
    for seconds in SECONDS {
        for offset in OFFSETS {
            let c = Signature::new(seconds, offset).when();
            let rust = Time::new(seconds, offset);
            assert_eq!(fields(&rust), c_fields(&c), "{seconds} {offset}");
            assert_eq!(Time::from(c), rust, "{seconds} {offset}");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn getters_read_what_git_signature_from_buffer_parses() {
    init();
    // The zone after the timestamp, and the fields libgit2 is expected to leave.
    let cases: [(&CStr, i64, i32, u8); 11] = [
        (c"A <a@b> 1234 +0130", 1234, 90, b'+'),
        (c"A <a@b> 1234 -0130", 1234, -90, b'-'),
        (c"A <a@b> 1234 +0000", 1234, 0, b'+'),
        // git's questionable `-0000`: offset 0, sign `-`.
        (c"A <a@b> 1234 -0000", 1234, 0, b'-'),
        (c"A <a@b> -5 +1459", -5, 899, b'+'),
        // Out of range: neither offset nor sign is stored.
        (c"A <a@b> 1234 +1500", 1234, 0, 0),
        (c"A <a@b> 1234 +0160", 1234, 0, 0),
        // Malformed: offset 0, the first character kept as the sign.
        (c"A <a@b> 1234 x0130", 1234, 0, b'x'),
        (c"A <a@b> 1234 \xff0130", 1234, 0, 0xff),
        // A signed number after the sign: the offset and sign disagree.
        (c"A <a@b> 1234 +-130", 1234, -90, b'+'),
        // No zone at all.
        (c"A <a@b> 1234", 1234, 0, 0),
    ];
    for (buf, seconds, offset, sign) in cases {
        let c = Signature::parse(buf).when();
        let time = Time::from(c);
        assert_eq!(fields(&time), c_fields(&c), "{buf:?}");
        assert_eq!(fields(&time), (seconds, offset, sign), "{buf:?}");
        assert_eq!(fields(&time.as_ref().to_time()), c_fields(&c), "{buf:?}");
    }
}

/// One write, applied to both sides.
#[derive(Clone, Copy, Debug)]
enum Step {
    Seconds(i64),
    Offset(i32),
    Sign(u8),
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn setters_write_what_git_signature_dup_copies() {
    init();
    let steps = [
        Step::Seconds(-1),
        Step::Offset(-60),
        Step::Sign(b'-'),
        Step::Offset(0),
        Step::Sign(b'x'),
        Step::Sign(0),
        Step::Sign(0xff),
        Step::Seconds(i64::MAX),
        Step::Offset(i32::MIN),
        Step::Sign(b'+'),
        Step::Seconds(i64::MIN),
        Step::Offset(i32::MAX),
    ];
    let mut c = Signature::new(1_700_000_000, 120);
    let mut rust = Time::new(1_700_000_000, 120);
    assert_eq!(fields(&rust), c_fields(&c.dup().when()));
    for step in steps {
        let when = c.when_mut();
        let mut handle = rust.as_mut();
        match step {
            Step::Seconds(v) => {
                when.time = v;
                handle.set_seconds(v);
            }
            Step::Offset(v) => {
                when.offset = v;
                handle.set_offset_minutes(v);
            }
            Step::Sign(v) => {
                when.sign = v as c_char;
                handle.set_sign(v);
            }
        }
        // What C copies of its own writes, and of the Rust value placed in a
        // signature of its own, are the Rust value.
        let copied = c.dup().when();
        assert_eq!(fields(&rust), c_fields(&copied), "{step:?}");
        let mut placed = Signature::new(0, 0);
        *placed.when_mut() = rust.into();
        assert_eq!(Time::from(placed.dup().when()), rust, "{step:?}");
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn zeroed_is_what_a_zoneless_parse_leaves() {
    init();
    let c = Signature::parse(c"A <a@b>").when();
    assert_eq!(Time::from(c), Time::zeroed());
}

/// Every `git_object_t` constant that names an object's type, beside its variant.
const OBJECT_TYPES: [(ffi::git_object_t, ObjectType); 4] = [
    (ffi::GIT_OBJECT_COMMIT, ObjectType::Commit),
    (ffi::GIT_OBJECT_TREE, ObjectType::Tree),
    (ffi::GIT_OBJECT_BLOB, ObjectType::Blob),
    (ffi::GIT_OBJECT_TAG, ObjectType::Tag),
];

/// Every value around the declared ones, past the packfile's delta types (6, 7), and
/// the extremes.
fn object_probes() -> impl Iterator<Item = ffi::git_object_t> {
    (-300..=300).chain([
        ffi::git_object_t::MIN,
        ffi::git_object_t::MIN + 1,
        ffi::git_object_t::MAX - 1,
        ffi::git_object_t::MAX,
    ])
}

/// `git_object_type2string(raw)`, copied.
fn type_name(raw: ffi::git_object_t) -> CString {
    // SAFETY: `git_object_type2string` takes any value and returns a static string,
    // "" for a value it does not name.
    unsafe { CStr::from_ptr(ffi::git_object_type2string(raw)) }.to_owned()
}

/// `git_object_string2type(name)`.
fn type_of_name(name: &CStr) -> ffi::git_object_t {
    // SAFETY: `name` is NUL-terminated and read for the call only.
    unsafe { ffi::git_object_string2type(name.as_ptr()) }
}

/// `git_object_type_is_valid` must accept exactly the values `from_raw` maps to a
/// variant, `git_object_type2string` must name exactly those, and each name must read
/// back as the variant's C value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn object_types_are_what_libgit2_accepts() {
    init();
    for raw in object_probes() {
        let kind = ObjectType::from_raw(raw);
        // SAFETY: `git_object_type_is_valid` takes any value.
        let valid = unsafe { ffi::git_object_type_is_valid(raw) };
        assert_eq!(valid != 0, kind.is_some(), "type_is_valid({raw})");
        assert_eq!(kind.map(ObjectType::raw), kind.is_some().then_some(raw));
        assert_eq!(
            ObjectType::try_from(raw).map_err(NotAnObjectType::raw),
            kind.ok_or(raw)
        );

        let name = type_name(raw);
        assert_eq!(
            name.is_empty(),
            kind.is_none(),
            "type2string({raw}) = {name:?}"
        );
        let back = type_of_name(&name);
        assert_eq!(ObjectType::from_raw(back), kind, "string2type({name:?})");
    }
    // The names, as C spells them, beside the variants.
    for ((raw, kind), name) in OBJECT_TYPES
        .into_iter()
        .zip([c"commit", c"tree", c"blob", c"tag"])
    {
        assert_eq!(kind.raw(), raw);
        assert_eq!(type_name(raw).as_c_str(), name);
        assert_eq!(type_of_name(name), raw);
    }
    // What C answers for a name it does not know, which no variant is.
    for name in [c"", c"any", c"ANY", c"invalid", c"ofs-delta"] {
        let raw = type_of_name(name);
        assert_eq!(raw, ffi::GIT_OBJECT_INVALID, "{name:?}");
        assert_eq!(ObjectType::from_raw(raw), None);
    }
    // SAFETY: `git_object_string2type` checks for NULL, answering `GIT_OBJECT_INVALID`.
    let raw = unsafe { ffi::git_object_string2type(ptr::null()) };
    assert_eq!(raw, ffi::GIT_OBJECT_INVALID);
    assert_eq!(ObjectType::from_raw(raw), None);
}

/// A scratch directory for one bare repository, removed on drop.
struct TempRepo {
    path: PathBuf,
    repo: *mut ffi::git_repository,
}

impl TempRepo {
    /// `git_repository_init_ext(path)`, bare, with default options.
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crustify-types-equiv-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` a live
        // options struct, all read for the call only; the result is a fresh
        // repository on success.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0, "{:?}", c_last());
        assert!(!repo.is_null());
        Self { path, repo }
    }

    /// The repository's object database, released with [`Odb`].
    fn odb(&self) -> Odb {
        let mut odb = ptr::null_mut();
        // SAFETY: `self.repo` is live; the result is a new reference to its odb.
        let rc = unsafe { ffi::git_repository_odb(&mut odb, self.repo) };
        assert_eq!(rc, 0);
        assert!(!odb.is_null());
        Odb(odb)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        // SAFETY: `self.repo` is the live repository `new` produced, freed once; every
        // `Odb` taken from it holds its own reference.
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// One reference to a repository's object database.
struct Odb(*mut ffi::git_odb);

impl Drop for Odb {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the reference `git_repository_odb` returned, released
        // once.
        unsafe { ffi::git_odb_free(self.0) };
    }
}

/// A `git_oid`'s type byte and id bytes.
type Id = (u8, [u8; 32]);

/// What one step reported: the id it wrote, or the type and length
/// `git_odb_read_header` read back, or the return and the thread's last error after
/// a failure.
#[derive(Debug, PartialEq)]
enum OdbStep {
    Written(Id),
    Header(ffi::git_object_t, usize),
    Failed(c_int, (c_int, String)),
}

impl Odb {
    /// `git_odb_write(data, raw)`.
    fn write(&self, data: &[u8], raw: ffi::git_object_t) -> OdbStep {
        let mut id = ffi::git_oid::default();
        // SAFETY: `id` is a writable `git_oid`; `data` is live for its length, read
        // for the call only.
        let rc =
            unsafe { ffi::git_odb_write(&mut id, self.0, data.as_ptr().cast(), data.len(), raw) };
        if rc == 0 {
            OdbStep::Written((id.type_, id.id))
        } else {
            OdbStep::Failed(rc, c_last())
        }
    }

    /// `git_odb_read_header(id)`.
    fn read_header(&self, &(type_, id): &Id) -> OdbStep {
        let id = ffi::git_oid { type_, id };
        let (mut len, mut raw) = (usize::MAX, ffi::git_object_t::MAX);
        // SAFETY: `len` and `raw` are writable slots; `id` is a live `git_oid`, read
        // for the call only.
        let rc = unsafe { ffi::git_odb_read_header(&mut len, &mut raw, self.0, &id) };
        if rc == 0 {
            OdbStep::Header(raw, len)
        } else {
            OdbStep::Failed(rc, c_last())
        }
    }
}

/// The failure libgit2 reports for a value that is no object's type.
fn invalid_object_type() -> OdbStep {
    OdbStep::Failed(
        -1,
        (
            ffi::GIT_ERROR_INVALID as c_int,
            "invalid object type".to_owned(),
        ),
    )
}

/// Writes one object per probed value to one object database, reading each back,
/// and compares every step with what the Rust conversion predicts.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn object_types_round_trip_through_the_odb() {
    init();
    let repo = TempRepo::new("object-odb");
    let odb = repo.odb();
    let probes = (-4..=8)
        .chain([ObjectType::raw_or_any(None)])
        .chain([ffi::git_object_t::MIN, ffi::git_object_t::MAX])
        .filter(|&raw| raw != 0);
    for raw in probes {
        let data = format!("probe {raw}");
        let written = odb.write(data.as_bytes(), raw);
        match ObjectType::from_raw(raw) {
            Some(kind) => {
                let OdbStep::Written(id) = written else {
                    panic!("write({raw}) = {written:?}")
                };
                let header = odb.read_header(&id);
                assert_eq!(
                    header,
                    OdbStep::Header(kind.raw(), data.len()),
                    "header({raw})"
                );
                let OdbStep::Header(read, _) = header else {
                    unreachable!()
                };
                assert_eq!(ObjectType::try_from(read), Ok(kind));
            }
            None => assert_eq!(written, invalid_object_type(), "write({raw})"),
        }
    }

    // A filter is not a type: an object cannot be written as "any".
    assert_eq!(
        odb.write(b"any", ObjectType::raw_or_any(None)),
        invalid_object_type()
    );
}

/// C alone: the unnamed `0`, which `ObjectType` cannot carry, is hashed as a blob but
/// stored under no type name, unreadable, and shadows a later blob of the same data.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn odb_write_of_type_zero_stores_an_unreadable_blob() {
    init();
    assert_eq!(ObjectType::from_raw(0), None);
    let repo = TempRepo::new("object-zero");
    let odb = repo.odb();
    let OdbStep::Written(id) = odb.write(b"zero", 0) else {
        panic!("C refused type 0")
    };
    let OdbStep::Failed(rc, _) = odb.read_header(&id) else {
        panic!("C read back an object of type 0")
    };
    assert!(rc < 0);
    assert_eq!(
        odb.write(b"zero", ObjectType::Blob.raw()),
        OdbStep::Written(id),
        "the blob's id"
    );
    assert!(matches!(odb.read_header(&id), OdbStep::Failed(..)));
}

/// `git_reference_lookup(name)`, released with [`Reference`].
fn lookup_reference(repo: &TempRepo, name: &CStr) -> Reference {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a live slot, `repo.repo` live, `name` NUL-terminated and read
    // for the call only; the result is a fresh reference on success.
    let rc = unsafe { ffi::git_reference_lookup(&mut out, repo.repo, name.as_ptr()) };
    assert_eq!(rc, 0, "lookup {name:?}: {:?}", c_last());
    Reference(out)
}

/// A reference libgit2 built.
struct Reference(*mut ffi::git_reference);

impl Reference {
    /// `git_reference_type`.
    fn raw_type(&self) -> ffi::git_reference_t {
        // SAFETY: `self.0` is a live reference.
        unsafe { ffi::git_reference_type(self.0) }
    }
}

impl Drop for Reference {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the live reference its constructor returned, freed once.
        unsafe { ffi::git_reference_free(self.0) };
    }
}

/// `git_reference_type` must answer, for the references libgit2 builds, the value of
/// the variant the reference's kind predicts.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reference_types_follow_c() {
    init();
    let repo = TempRepo::new("reference-kinds");

    // A fresh repository's HEAD names a branch: symbolic.
    let head = lookup_reference(&repo, c"HEAD");
    assert_eq!(head.raw_type(), ReferenceType::Symbolic.raw());
    assert_eq!(
        ReferenceType::from_raw(head.raw_type()),
        Some(ReferenceType::Symbolic)
    );

    // A reference created over an object id: direct, at creation and on lookup.
    let OdbStep::Written(id) = repo.odb().write(b"target", ObjectType::Blob.raw()) else {
        panic!("blob write failed")
    };
    let id = ffi::git_oid {
        type_: id.0,
        id: id.1,
    };
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a live slot, `repo.repo` live, the name NUL-terminated and `id`
    // a live `git_oid`, all read for the call only; no log message.
    let rc = unsafe {
        ffi::git_reference_create(
            &mut out,
            repo.repo,
            c"refs/tags/probe".as_ptr(),
            &id,
            0,
            ptr::null(),
        )
    };
    assert_eq!(rc, 0, "{:?}", c_last());
    let created = Reference(out);
    assert_eq!(
        ReferenceType::try_from(created.raw_type()),
        Ok(ReferenceType::Direct)
    );
    let found = lookup_reference(&repo, c"refs/tags/probe");
    assert_eq!(found.raw_type(), ReferenceType::Direct.raw());

    // HEAD again, after the repository changed: still symbolic.
    assert_eq!(
        ReferenceType::from_raw(lookup_reference(&repo, c"HEAD").raw_type()),
        Some(ReferenceType::Symbolic)
    );

    // No reference: a release build answers -1, which no variant is.
    if std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() != Ok("coverage") {
        // SAFETY: a release build's `git_reference_type` checks its argument and
        // returns -1 for NULL; the debug build, which asserts, is excluded above.
        let raw = unsafe { ffi::git_reference_type(ptr::null()) };
        assert_eq!(raw, -1i32 as ffi::git_reference_t);
        assert_eq!(
            ReferenceType::try_from(raw).map_err(NotAReferenceType::raw),
            Err(ffi::git_reference_t::MAX)
        );
    }
}

/// Every `git_filemode_t` constant, beside the variant that must carry it.
const MODES: [(ffi::git_filemode_t, FileMode); 6] = [
    (ffi::GIT_FILEMODE_UNREADABLE, FileMode::Unreadable),
    (ffi::GIT_FILEMODE_TREE, FileMode::Tree),
    (ffi::GIT_FILEMODE_BLOB, FileMode::Blob),
    (ffi::GIT_FILEMODE_BLOB_EXECUTABLE, FileMode::BlobExecutable),
    (ffi::GIT_FILEMODE_LINK, FileMode::Link),
    (ffi::GIT_FILEMODE_COMMIT, FileMode::Commit),
];

/// Whether the builder takes `mode`, as the Rust type predicts it.
fn insertable(mode: Option<FileMode>) -> bool {
    mode.is_some_and(|m| m != FileMode::Unreadable)
}

/// The class and message of the calling thread's last error.
fn last_error() -> (c_int, CString) {
    // SAFETY: the runtime is initialised; the result is a static error or this
    // thread's, live until the next libgit2 call on this thread.
    let e = unsafe { ffi::git_error_last() };
    assert!(!e.is_null());
    // SAFETY: `e` is a live `git_error` whose message is NULL or a C string.
    unsafe {
        let message = (*e).message;
        let message = if message.is_null() {
            c""
        } else {
            CStr::from_ptr(message)
        };
        ((*e).klass, message.to_owned())
    }
}

/// Clears the calling thread's last error, so a step reads only its own.
fn clear() {
    // SAFETY: the runtime is initialised.
    unsafe { ffi::git_error_clear() };
}

/// A bare repository of its own under the temp dir, with an empty tree and one blob
/// written to it; closed and removed when dropped.
struct TreeRepo {
    path: PathBuf,
    repo: *mut ffi::git_repository,
    blob: ffi::git_oid,
    empty_tree: ffi::git_oid,
}

impl TreeRepo {
    fn new(tag: &str) -> Self {
        init();
        let path =
            std::env::temp_dir().join(format!("crustify-types-equiv-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` a live
        // options struct, all read for the call only.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0, "{:?}", last_error());
        let mut repo = Self {
            path,
            repo,
            blob: Default::default(),
            empty_tree: Default::default(),
        };
        repo.blob = repo.write(b"blob\n", ffi::GIT_OBJECT_BLOB).unwrap();
        repo.empty_tree = repo.write(b"", ffi::GIT_OBJECT_TREE).unwrap();
        repo
    }

    /// `git_odb_write` of `data` as an object of `kind`.
    fn write(&self, data: &[u8], kind: ffi::git_object_t) -> Result<ffi::git_oid, c_int> {
        let mut odb = ptr::null_mut();
        // SAFETY: `odb` is a live slot and `self.repo` a live repository.
        assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, self.repo) }, 0);
        let mut id = ffi::git_oid::default();
        // SAFETY: `id` is a writable oid, `odb` the live database just taken, and
        // `data` readable for `data.len()` bytes; `odb` is released once, after.
        let rc = unsafe {
            let rc = ffi::git_odb_write(&mut id, odb, data.as_ptr().cast(), data.len(), kind);
            ffi::git_odb_free(odb);
            rc
        };
        if rc == 0 { Ok(id) } else { Err(rc) }
    }

    /// The id an entry of `mode` must name to pass the builder's object check.
    fn target(&self, mode: Option<FileMode>) -> ffi::git_oid {
        match mode {
            Some(FileMode::Tree) => self.empty_tree,
            _ => self.blob,
        }
    }

    /// `git_tree_lookup(id)`: the tree's entries' raw and normalised modes, or the
    /// lookup's return and error.
    fn tree_modes(&self, id: &ffi::git_oid) -> Result<Vec<(u32, u32)>, (c_int, c_int, CString)> {
        let mut tree = ptr::null_mut();
        clear();
        // SAFETY: `tree` is a live slot, `self.repo` a live repository and `id` a
        // live oid, read for the call only.
        let rc = unsafe { ffi::git_tree_lookup(&mut tree, self.repo, id) };
        if rc != 0 {
            let (class, message) = last_error();
            return Err((rc, class, message));
        }
        // SAFETY: `tree` is the live tree just looked up; every entry is read while
        // it lives, and it is released once, after.
        unsafe {
            let modes = (0..ffi::git_tree_entrycount(tree))
                .map(|i| {
                    let entry = ffi::git_tree_entry_byindex(tree, i);
                    assert!(!entry.is_null());
                    (
                        ffi::git_tree_entry_filemode_raw(entry),
                        ffi::git_tree_entry_filemode(entry),
                    )
                })
                .collect();
            ffi::git_tree_free(tree);
            Ok(modes)
        }
    }
}

impl Drop for TreeRepo {
    fn drop(&mut self) {
        // SAFETY: `self.repo` is the live repository this value owns, released once.
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn modes_carry_their_c_values() {
    for (raw, mode) in MODES {
        assert_eq!(mode.raw(), raw, "{mode:?}");
        assert_eq!(FileMode::from_raw(raw), Some(mode));
    }
}

#[test]
fn from_raw_names_exactly_the_c_modes() {
    // Every mode a tree entry can hold, and the extremes past it.
    for raw in (0..=0o177777).chain([0o200000, u32::MAX - 1, u32::MAX]) {
        let c = MODES.iter().find(|(r, _)| *r == raw).map(|&(_, m)| m);
        assert_eq!(FileMode::from_raw(raw), c, "{raw:#o}");
        assert_eq!(
            FileMode::try_from(raw).map_err(UnnamedFileMode::raw),
            c.ok_or(raw),
            "{raw:#o}"
        );
    }
}

/// What one insertion reported: its return, the failure's class and message (0 and
/// empty on success), and the entry's raw and normalised modes as the builder then
/// holds them, if it holds one.
type Inserted = (c_int, c_int, CString, Option<(u32, u32)>);

/// `git_treebuilder_insert(bld, name, target(mode), raw)`, then `git_treebuilder_get`.
fn insert(repo: &TreeRepo, bld: *mut ffi::git_treebuilder, name: &CStr, raw: u32) -> Inserted {
    insert_naming(repo.target(FileMode::from_raw(raw)), bld, name, raw)
}

/// `git_treebuilder_insert(bld, name, id, raw)`, then `git_treebuilder_get`, which
/// must return the entry the insertion handed out.
fn insert_naming(
    id: ffi::git_oid,
    bld: *mut ffi::git_treebuilder,
    name: &CStr,
    raw: u32,
) -> Inserted {
    let mut out = ptr::null();
    clear();
    // SAFETY: `out` is a live slot, `bld` a live builder, `name` NUL-terminated and
    // `id` a live oid, read for the call only; `out` is compared, never read through.
    let rc = unsafe { ffi::git_treebuilder_insert(&mut out, bld, name.as_ptr(), &id, raw) };
    let (class, message) = if rc == 0 {
        (0, CString::default())
    } else {
        last_error()
    };
    // SAFETY: `bld` is live; the entry it returns is read before the builder changes.
    let held = unsafe {
        let entry = ffi::git_treebuilder_get(bld, name.as_ptr());
        assert!(
            rc != 0 || entry == out,
            "the inserted entry is the one held"
        );
        (!entry.is_null()).then(|| {
            (
                ffi::git_tree_entry_filemode_raw(entry),
                ffi::git_tree_entry_filemode(entry),
            )
        })
    };
    (rc, class, message, held)
}

/// A builder over `repo` with no source tree; released by the caller.
fn builder(repo: &TreeRepo) -> *mut ffi::git_treebuilder {
    let mut bld = ptr::null_mut();
    // SAFETY: `bld` is a live slot and `repo.repo` a live repository; no source tree.
    let rc = unsafe { ffi::git_treebuilder_new(&mut bld, repo.repo, ptr::null()) };
    assert_eq!(rc, 0);
    bld
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn treebuilder_accepts_exactly_the_insertable_modes() {
    let repo = TreeRepo::new("insert");
    let bld = builder(&repo);

    // Each named mode under a name of its own, then unnamed modes, then each named
    // mode again over the first name, which a refused mode must leave alone.
    let unnamed = [
        0o100664,
        0o100600,
        0o040755,
        0o160755,
        0o120777,
        1,
        0o177777,
        u32::MAX,
    ];
    let mut steps: Vec<(CString, u32)> = MODES
        .iter()
        .enumerate()
        .map(|(i, &(raw, _))| (CString::new(format!("named-{i}")).unwrap(), raw))
        .collect();
    steps.extend(
        unnamed
            .iter()
            .enumerate()
            .map(|(i, &raw)| (CString::new(format!("unnamed-{i}")).unwrap(), raw)),
    );
    steps.extend(
        MODES
            .iter()
            .chain(MODES.iter().rev())
            .map(|&(raw, _)| (c"named-1".to_owned(), raw)),
    );

    // What the tree should end up holding, name by name, as the Rust type predicts.
    let mut held: Vec<(CString, u32)> = Vec::new();
    for (step, (name, raw)) in steps.iter().enumerate() {
        let mode = FileMode::from_raw(*raw);
        let before = held.iter().find(|(n, _)| n == name).map(|&(_, r)| r);
        let expected = if insertable(mode) {
            match held.iter_mut().find(|(n, _)| n == name) {
                Some(slot) => slot.1 = *raw,
                None => held.push((name.clone(), *raw)),
            }
            (0, 0, CString::default(), Some((*raw, *raw)))
        } else {
            let message = format!(
                "failed to insert entry: invalid filemode for file - {}",
                name.to_str().unwrap()
            );
            (
                -1,
                ffi::GIT_ERROR_TREE as c_int,
                CString::new(message).unwrap(),
                before.map(|r| (r, r)),
            )
        };
        assert_eq!(
            insert(&repo, bld, name, *raw),
            expected,
            "step {step}: {raw:#o}"
        );
    }

    let mut id = ffi::git_oid::default();
    // SAFETY: `id` is a writable oid and `bld` the live builder, released once, after.
    let rc = unsafe {
        let rc = ffi::git_treebuilder_write(&mut id, bld);
        ffi::git_treebuilder_free(bld);
        rc
    };
    assert_eq!(rc, 0, "{:?}", last_error());

    // The written tree sorts its entries by name; each holds the mode inserted last.
    held.sort_by(|a, b| a.0.cmp(&b.0));
    let expected: Vec<(u32, u32)> = held.iter().map(|&(_, r)| (r, r)).collect();
    assert_eq!(repo.tree_modes(&id), Ok(expected));
    for (_, raw) in &held {
        assert!(insertable(FileMode::from_raw(*raw)));
    }
}

/// With strict object creation on (the default), a mode also fixes the kind of
/// object the entry must name: a subtree a tree, a blob, executable or link a blob,
/// while a submodule's commit is not looked up. Each step names the other kind.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn treebuilder_checks_the_object_a_mode_names() {
    let repo = TreeRepo::new("object");
    let bld = builder(&repo);
    for (step, &(raw, mode)) in MODES.iter().enumerate() {
        if !insertable(Some(mode)) {
            continue;
        }
        let name = CString::new(format!("wrong-{step}")).unwrap();
        let wrong = match mode {
            FileMode::Tree => repo.blob,
            _ => repo.empty_tree,
        };
        let expected = if mode == FileMode::Commit {
            (0, 0, CString::default(), Some((raw, raw)))
        } else {
            let message = format!(
                "failed to insert entry: invalid object specified - {}",
                name.to_str().unwrap()
            );
            (
                -1,
                ffi::GIT_ERROR_TREE as c_int,
                CString::new(message).unwrap(),
                None,
            )
        };
        assert_eq!(insert_naming(wrong, bld, &name, raw), expected, "{mode:?}");
    }
    // SAFETY: `bld` is the live builder made above, released once.
    unsafe { ffi::git_treebuilder_free(bld) };
}

/// A tree object of one entry per mode, spelled in octal as git writes it, all
/// naming `target`.
fn raw_tree(modes: &[u32], target: &ffi::git_oid) -> Vec<u8> {
    let mut data = Vec::new();
    for (i, raw) in modes.iter().enumerate() {
        data.extend_from_slice(format!("{raw:o} e{i:06}\0").as_bytes());
        data.extend_from_slice(&target.id[..20]);
    }
    data
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parsed_trees_keep_any_16_bit_mode() {
    let repo = TreeRepo::new("parse");
    // Every 16-bit mode, at a stride under BorrowSanitizer, which instruments each
    // step; the named modes always.
    let stride = if cfg!(bsan) { 97 } else { 1 };
    let mut modes: Vec<u32> = (0..=0o177777).step_by(stride).collect();
    modes.extend(MODES.iter().map(|&(raw, _)| raw));

    let id = repo
        .write(&raw_tree(&modes, &repo.blob), ffi::GIT_OBJECT_TREE)
        .unwrap();
    let read = repo.tree_modes(&id).unwrap();
    assert_eq!(read.len(), modes.len());
    for (&written, &(raw, normalised)) in modes.iter().zip(&read) {
        // `_raw` returns the mode as spelled; `from_raw` names it iff the header does.
        assert_eq!(raw, written);
        let c = MODES.iter().find(|(r, _)| *r == raw).map(|&(_, m)| m);
        assert_eq!(FileMode::from_raw(raw), c, "{raw:#o}");
        // The normalised mode is one the builder would take.
        assert!(
            insertable(FileMode::from_raw(normalised)),
            "{raw:#o} -> {normalised:#o}"
        );
        // A named insertable mode normalises to itself.
        if insertable(c) {
            assert_eq!(normalised, raw);
        }
    }

    // A mode past 16 bits does not parse, nor one that is not octal digits.
    assert_eq!(FileMode::from_raw(0o200000), None);
    for spelling in [&b" 100644"[..], b"9", b"x", b"-1"] {
        let mut data = spelling.to_vec();
        data.extend_from_slice(b" e\0");
        data.extend_from_slice(&repo.blob.id[..20]);
        let id = repo.write(&data, ffi::GIT_OBJECT_TREE).unwrap();
        assert_eq!(
            repo.tree_modes(&id),
            Err((
                -1,
                ffi::GIT_ERROR_TREE as c_int,
                c"failed to parse tree: can't parse filemode".to_owned()
            )),
            "{}",
            String::from_utf8_lossy(spelling)
        );
    }
    let id = repo
        .write(&raw_tree(&[0o200000], &repo.blob), ffi::GIT_OBJECT_TREE)
        .unwrap();
    assert_eq!(
        repo.tree_modes(&id),
        Err((
            -1,
            ffi::GIT_ERROR_TREE as c_int,
            c"failed to parse tree: can't parse filemode".to_owned()
        ))
    );
}

mod write_streams {
    //! Equivalence of `git2/types.h`'s `git_writestream` wrappers with the C streams and
    //! C callers they stand in for.
    //!
    //! - C streams: a blob stream (`git_blob_create_from_stream`) written by raw member
    //!   calls, and one adopted into a `WriteStreamBox` and written through
    //!   `WriteStreamMut::write`, each on its own repository; the two are compared after
    //!   every step (the return, the thread's error after a failure, the stream's temporary
    //!   file), then on how they end: committed (the blob id), freed (`free` / drop), or
    //!   closed (`close` + `free` / `WriteStream::close`).
    //! - Rust streams: libgit2 drives a hand-written C-layout stream and a
    //!   `RustWriteStream` over the same recorder through `git_filter_list_stream_buffer`,
    //!   bare, behind libgit2's buffered `ident` stream, and behind a filter whose own
    //!   stream libgit2 frees; each side runs on a thread of its own, and the recorded
    //!   calls, returns and errors are compared after every call, failures injected.
    //!
    //! Each side runs on a fresh thread, so the error each reports is its own.

    use std::cell::RefCell;
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::path::PathBuf;
    use std::ptr;
    use std::rc::Rc;
    use std::sync::OnceLock;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{
        RustWriteStream, StreamError, StreamWriter, WriteStream, WriteStreamBox,
    };
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode, last};

    fn init() {
        // SAFETY: no arguments; the reference is kept for the rest of the process.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `len` bytes varying with `seed`; past a page, one repeated byte, since BSan is slow
    /// on per-element loops.
    fn pattern(len: usize, seed: usize) -> Vec<u8> {
        if len > 4097 {
            return vec![(seed * 7 + 3) as u8; len];
        }
        (0..len).map(|i| (i * 31 + seed * 7 + 3) as u8).collect()
    }

    /// The thread's last error, as `git_error_last` reports it.
    fn c_last() -> (c_int, CString) {
        // SAFETY: the runtime is initialised; the error is copied before any other call.
        unsafe {
            let e = ffi::git_error_last();
            ((*e).klass, CStr::from_ptr((*e).message).to_owned())
        }
    }

    /// What one call reported: its return (0 for success on the Rust side, the code
    /// otherwise) and, after a failure, the thread's last error.
    type Reported = (c_int, Option<(c_int, CString)>);

    fn c_reported(rc: c_int) -> Reported {
        (rc, (rc < 0).then(c_last))
    }

    fn rust_reported(result: Result<(), StreamError>) -> Reported {
        match result {
            Ok(()) => (0, None),
            Err(e) => {
                let last = last();
                // The error the wrapper copied is the thread's.
                assert_eq!(
                    (e.raw_class(), e.message()),
                    (last.raw_class(), last.message())
                );
                (e.raw_code(), Some((e.raw_class(), e.message().to_owned())))
            }
        }
    }

    // --- blob streams --------------------------------------------------------------------

    struct TempRepo {
        path: PathBuf,
        repo: *mut ffi::git_repository,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            init();
            let path = std::env::temp_dir().join(format!(
                "crustify-writestream-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot, a NUL-terminated path and live options, read for the call.
            let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
            assert_eq!(rc, 0);
            Self { path, repo }
        }

        /// The blob stream's temporary files under `objects/`.
        fn stream_files(&self) -> usize {
            std::fs::read_dir(self.path.join("objects"))
                .unwrap()
                .filter(|e| {
                    let name = e.as_ref().unwrap().file_name();
                    name.to_string_lossy().starts_with("streamed")
                })
                .count()
        }

        fn open(&self, hint: Option<&CStr>) -> *mut ffi::git_writestream {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot, the live repository and a NULL or live hint path.
            let rc = unsafe {
                ffi::git_blob_create_from_stream(
                    &mut out,
                    self.repo,
                    hint.map_or(ptr::null(), CStr::as_ptr),
                )
            };
            assert_eq!(rc, 0);
            out
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            // SAFETY: the repository opened above, freed once after its streams.
            unsafe { ffi::git_repository_free(self.repo) };
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// `git_blob_create_from_stream_commit`, consuming the stream: the blob id, or the
    /// failure.
    fn commit(stream: *mut ffi::git_writestream) -> Result<Vec<u8>, Reported> {
        let mut oid = ffi::git_oid::default();
        // SAFETY: an open, never-closed blob stream, given up to the call.
        let rc = unsafe { ffi::git_blob_create_from_stream_commit(&mut oid, stream) };
        if rc < 0 {
            return Err(c_reported(rc));
        }
        Ok(oid.id[..20].to_vec())
    }

    /// A raw `write` member call on a C stream.
    fn c_write(stream: *mut ffi::git_writestream, data: &[u8]) -> Reported {
        // SAFETY: an open stream and its own member; `data` is live for the call.
        c_reported(unsafe { (*stream).write.unwrap()(stream, data.as_ptr().cast(), data.len()) })
    }

    /// Chunk sizes written in order: empty, small, page-crossing, and enough to spill the
    /// blob stream's 2 MiB buffer into its temporary file twice.
    const CHUNKS: [usize; 8] = [0, 1, 7, 4097, 1 << 20, 1 << 20, 65_537, 0];

    /// One side's steps: the report and the temporary-file count after each write.
    type Trace = Vec<(Reported, usize)>;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_stream_writes_and_commit_match_c() {
        for (tag, hint) in [("none", None), ("hint", Some(c"dir/file.txt"))] {
            let c = thread::spawn(move || {
                let repo = TempRepo::new(&format!("c-commit-{tag}"));
                let stream = repo.open(hint);
                let mut trace = Trace::new();
                for (i, len) in CHUNKS.into_iter().enumerate() {
                    trace.push((c_write(stream, &pattern(len, i)), repo.stream_files()));
                }
                let id = commit(stream);
                (trace, id, repo.stream_files())
            })
            .join()
            .unwrap();
            let rust = thread::spawn(move || {
                let repo = TempRepo::new(&format!("rust-commit-{tag}"));
                // SAFETY: a fresh, open blob stream, owned here and released by its own
                // `free` member (stands in for the blob wrapper).
                let mut stream = unsafe { WriteStreamBox::from_c(repo.open(hint)) }.unwrap();
                let mut trace = Trace::new();
                for (i, len) in CHUNKS.into_iter().enumerate() {
                    let reported = rust_reported(stream.as_mut().write(&pattern(len, i)));
                    trace.push((reported, repo.stream_files()));
                }
                let id = commit(stream.into_c());
                (trace, id, repo.stream_files())
            })
            .join()
            .unwrap();
            assert_eq!(rust, c, "hint {hint:?}");
            assert!(c.1.is_ok());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_stream_free_and_close_match_c() {
        let c = thread::spawn(|| {
            let repo = TempRepo::new("c-end");
            let mut states = Vec::new();
            // Freed unwritten, freed written, closed written.
            let s = repo.open(None);
            states.push(repo.stream_files());
            // SAFETY: an open stream, its own member, never used again.
            unsafe { (*s).free.unwrap()(s) };
            states.push(repo.stream_files());
            let s = repo.open(Some(c"x"));
            c_write(s, b"discarded");
            // SAFETY: as above.
            unsafe { (*s).free.unwrap()(s) };
            states.push(repo.stream_files());
            let s = repo.open(None);
            c_write(s, b"closed");
            // SAFETY: an open stream and its own member; afterwards it is only freed.
            let closed = c_reported(unsafe { (*s).close.unwrap()(s) });
            states.push(repo.stream_files());
            // SAFETY: the closed stream, freed once.
            unsafe { (*s).free.unwrap()(s) };
            states.push(repo.stream_files());
            (states, closed)
        })
        .join()
        .unwrap();
        let rust = thread::spawn(|| {
            let repo = TempRepo::new("rust-end");
            // SAFETY: fresh, open blob streams, owned here (stands in for the blob wrapper).
            let open =
                |hint: Option<&CStr>| unsafe { WriteStreamBox::from_c(repo.open(hint)) }.unwrap();
            let mut states = Vec::new();
            let s = open(None);
            states.push(repo.stream_files());
            drop(s);
            states.push(repo.stream_files());
            let mut s = open(Some(c"x"));
            s.as_mut().write(b"discarded").unwrap();
            drop(s);
            states.push(repo.stream_files());
            let mut s = open(None);
            s.as_mut().write(b"closed").unwrap();
            let closed = rust_reported(WriteStream::close(s));
            // Close and free are one step on the Rust side.
            states.push(repo.stream_files());
            states.push(repo.stream_files());
            (states, closed)
        })
        .join()
        .unwrap();
        assert_eq!(rust, c);
        assert_eq!(c.0, [1, 0, 0, 0, 0]);
    }

    // --- Rust streams, driven by libgit2 ---------------------------------------------------

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Write(Vec<u8>),
        Close,
        Free,
    }

    /// The bytes a C caller passed. libgit2's buffered streams pass NULL for empty data
    /// (their input buffer is `git__calloc`ed and never grown).
    ///
    /// # Safety
    ///
    /// `buf` is NULL with `len` 0, or valid for reads of `len` bytes.
    unsafe fn bytes<'a>(buf: *const c_char, len: usize) -> &'a [u8] {
        if len == 0 {
            return &[];
        }
        // SAFETY: per this function's contract.
        unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len) }
    }

    /// A failure to inject: the code returned, and the class and message set.
    #[derive(Clone, Copy, Debug)]
    struct Fail(c_int, ErrorClass, &'static CStr);

    #[derive(Clone, Copy, Debug, Default)]
    struct Plan {
        write: Option<Fail>,
        close: Option<Fail>,
    }

    /// The recorder both sides share: the calls made, and the failures to report.
    struct Recorder {
        log: Rc<RefCell<Vec<Call>>>,
        plan: Plan,
    }

    // The C side: a hand-written C-layout stream, as a C caller would define one.

    #[repr(C)]
    struct RawStream {
        parent: ffi::git_writestream,
        rec: Recorder,
    }

    fn raw_fail(fail: Option<Fail>) -> c_int {
        match fail {
            None => 0,
            Some(Fail(code, class, message)) => {
                // SAFETY: a live message, which libgit2 copies; the runtime is initialised.
                unsafe { ffi::git_error_set_str(class.raw(), message.as_ptr()) };
                code
            }
        }
    }

    unsafe extern "C" fn raw_write(
        s: *mut ffi::git_writestream,
        buf: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: `s` heads a live `RawStream`; `buf` holds `len` bytes, or is empty.
        let (rec, data) = unsafe { (&(*s.cast::<RawStream>()).rec, bytes(buf, len).to_vec()) };
        rec.log.borrow_mut().push(Call::Write(data));
        raw_fail(rec.plan.write)
    }

    unsafe extern "C" fn raw_close(s: *mut ffi::git_writestream) -> c_int {
        // SAFETY: `s` heads a live `RawStream`.
        let rec = unsafe { &(*s.cast::<RawStream>()).rec };
        rec.log.borrow_mut().push(Call::Close);
        raw_fail(rec.plan.close)
    }

    unsafe extern "C" fn raw_free(s: *mut ffi::git_writestream) {
        // SAFETY: `s` heads a `RawStream` box, given up by its owner.
        let raw = unsafe { Box::from_raw(s.cast::<RawStream>()) };
        raw.rec.log.borrow_mut().push(Call::Free);
    }

    fn raw_stream(rec: Recorder) -> *mut ffi::git_writestream {
        let raw = Box::new(RawStream {
            parent: ffi::git_writestream {
                write: Some(raw_write),
                close: Some(raw_close),
                free: Some(raw_free),
            },
            rec,
        });
        Box::into_raw(raw).cast()
    }

    // The Rust side: the same recorder as a `StreamWriter`.

    fn rust_fail(fail: Option<Fail>) -> Result<(), StreamError> {
        match fail {
            None => Ok(()),
            Some(Fail(code, class, message)) => {
                let code = ErrorCode::from_raw(code).unwrap();
                Err(StreamError::new(class, message).with_code(code))
            }
        }
    }

    impl StreamWriter for Recorder {
        fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
            self.log.borrow_mut().push(Call::Write(data.to_vec()));
            rust_fail(self.plan.write)
        }

        fn close(&mut self) -> Result<(), StreamError> {
            self.log.borrow_mut().push(Call::Close);
            rust_fail(self.plan.close)
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            // The C side logs its free from `raw_free`; a raw stream's recorder drops there
            // too, so only the Rust side's drop is the free.
        }
    }

    /// A filter chain in front of the target.
    #[derive(Clone, Copy, Debug)]
    enum Filters {
        None,
        IdentClean,
        Upper,
        Refused,
    }

    /// `git_filter_list_stream_buffer(filters, data, target)`.
    fn stream_buffer(filters: Filters, data: &[u8], target: *mut ffi::git_writestream) -> Reported {
        let mut list = ptr::null_mut();
        let filter = match filters {
            Filters::None => None,
            // SAFETY: a NUL-terminated name; the registry outlives the call.
            Filters::IdentClean => Some(unsafe { ffi::git_filter_lookup(c"ident".as_ptr()) }),
            Filters::Upper => Some(upper_filter()),
            Filters::Refused => Some(refused_filter()),
        };
        if let Some(filter) = filter {
            // SAFETY: a live slot; these filters need no repository.
            let rc = unsafe {
                ffi::git_filter_list_new(&mut list, ptr::null_mut(), ffi::GIT_FILTER_TO_ODB, 0)
            };
            assert_eq!(rc, 0);
            // SAFETY: a fresh list and a registered filter.
            let rc = unsafe { ffi::git_filter_list_push(list, filter, ptr::null_mut()) };
            assert_eq!(rc, 0);
        }
        // SAFETY: `list` is NULL or live; `data` is live; `target` is an open stream the
        // caller owns, which the call writes and closes.
        let rc = unsafe {
            ffi::git_filter_list_stream_buffer(list, data.as_ptr().cast(), data.len(), target)
        };
        let reported = c_reported(rc);
        // SAFETY: the list above, or NULL.
        unsafe { ffi::git_filter_list_free(list) };
        reported
    }

    /// A C-written filter whose stream upper-cases into the next one: libgit2 makes it
    /// with the `stream` callback and frees it after the chain closes.
    #[repr(C)]
    struct UpperStream {
        parent: ffi::git_writestream,
        next: *mut ffi::git_writestream,
    }

    unsafe extern "C" fn upper_write(
        s: *mut ffi::git_writestream,
        buf: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: `s` heads a live `UpperStream`, whose `next` libgit2 keeps open.
        unsafe {
            let next = (*s.cast::<UpperStream>()).next;
            let data = bytes(buf, len).to_ascii_uppercase();
            (*next).write.unwrap()(next, data.as_ptr().cast(), data.len())
        }
    }

    unsafe extern "C" fn upper_close(s: *mut ffi::git_writestream) -> c_int {
        // SAFETY: as above.
        unsafe {
            let next = (*s.cast::<UpperStream>()).next;
            (*next).close.unwrap()(next)
        }
    }

    unsafe extern "C" fn upper_free(s: *mut ffi::git_writestream) {
        // SAFETY: `s` heads an `UpperStream` box libgit2 gives up.
        drop(unsafe { Box::from_raw(s.cast::<UpperStream>()) });
    }

    unsafe extern "C" fn upper_stream(
        out: *mut *mut ffi::git_writestream,
        _filter: *mut ffi::git_filter,
        _payload: *mut *mut c_void,
        _source: *const ffi::git_filter_source,
        next: *mut ffi::git_writestream,
    ) -> c_int {
        let stream = Box::new(UpperStream {
            parent: ffi::git_writestream {
                write: Some(upper_write),
                close: Some(upper_close),
                free: Some(upper_free),
            },
            next,
        });
        // SAFETY: libgit2's live out-slot, which takes the stream over.
        unsafe { out.write(Box::into_raw(stream).cast()) };
        0
    }

    /// `git_filter.stream` of a filter that cannot start: libgit2 then closes the chain's
    /// last stream, the target, unwritten.
    unsafe extern "C" fn refused_stream(
        _out: *mut *mut ffi::git_writestream,
        _filter: *mut ffi::git_filter,
        _payload: *mut *mut c_void,
        _source: *const ffi::git_filter_source,
        _next: *mut ffi::git_writestream,
    ) -> c_int {
        // SAFETY: a live message, which libgit2 copies; the runtime is initialised.
        unsafe { ffi::git_error_set_str(ffi::GIT_ERROR_FILTER as c_int, c"refused".as_ptr()) };
        ffi::GIT_EUSER
    }

    /// A filter registered once under `name` with `stream` as its callback.
    fn registered(
        slot: &'static OnceLock<Registered>,
        name: &CStr,
        stream: ffi::git_filter_stream_fn,
    ) -> *mut ffi::git_filter {
        slot.get_or_init(|| {
            let filter = Box::leak(Box::new(ffi::git_filter::default()));
            // SAFETY: a live filter struct.
            let rc = unsafe { ffi::git_filter_init(filter, ffi::GIT_FILTER_VERSION) };
            assert_eq!(rc, 0);
            filter.stream = stream;
            // Loaded only for paths that set `crustify-test`, which none do, so blobs with a
            // hint path stay unfiltered.
            filter.attributes = c"+crustify-test".as_ptr();
            // SAFETY: a NUL-terminated name and a filter that lives for the process.
            let rc = unsafe { ffi::git_filter_register(name.as_ptr(), filter, 0) };
            assert_eq!(rc, 0);
            Registered(filter)
        })
        .0
    }

    struct Registered(*mut ffi::git_filter);
    // SAFETY: leaked, immutable after registration.
    unsafe impl Send for Registered {}
    // SAFETY: as above.
    unsafe impl Sync for Registered {}

    fn upper_filter() -> *mut ffi::git_filter {
        static FILTER: OnceLock<Registered> = OnceLock::new();
        registered(&FILTER, c"crustify-upper-equiv", Some(upper_stream))
    }

    fn refused_filter() -> *mut ffi::git_filter {
        static FILTER: OnceLock<Registered> = OnceLock::new();
        registered(&FILTER, c"crustify-refused-equiv", Some(refused_stream))
    }

    /// Every call of a sequence: what libgit2 returned and what the target recorded.
    type Run = Vec<(Reported, Vec<Call>)>;

    const INPUTS: [&[u8]; 4] = [
        b"",
        b"plain text",
        b"a $Id: 0123abc $ b\n",
        b"\0binary $Id: 1 $",
    ];

    /// The inputs for a chain, each twice. `ident` gets no empty input: it then computes
    /// `NULL + 0` in `git_str_is_binary` (UBSan), a libgit2 defect outside these wrappers.
    fn inputs(filters: Filters) -> impl Iterator<Item = &'static [u8]> {
        let skip_empty = matches!(filters, Filters::IdentClean);
        INPUTS
            .into_iter()
            .filter(move |i| !(skip_empty && i.is_empty()))
            .flat_map(|i| [i, i])
    }

    /// Each input twice through one target (closed by the first call, written again by the
    /// second), then the target freed.
    fn run_c(filters: Filters, plan: Plan) -> Run {
        init();
        let log = Rc::new(RefCell::new(Vec::new()));
        let target = raw_stream(Recorder {
            log: log.clone(),
            plan,
        });
        let mut run = Run::new();
        for input in inputs(filters) {
            let reported = stream_buffer(filters, input, target);
            run.push((reported, log.borrow_mut().drain(..).collect()));
        }
        // SAFETY: the target made above, freed once, through its own member.
        unsafe { (*target).free.unwrap()(target) };
        run.push(((0, None), log.borrow_mut().drain(..).collect()));
        run
    }

    fn run_rust(filters: Filters, plan: Plan) -> Run {
        init();
        let log = Rc::new(RefCell::new(Vec::new()));
        let target = RustWriteStream::new(Recorder {
            log: log.clone(),
            plan,
        })
        .into_box();
        let mut run = Run::new();
        for input in inputs(filters) {
            let reported = stream_buffer(filters, input, target.as_c_ptr());
            run.push((reported, log.borrow_mut().drain(..).collect()));
        }
        drop(target);
        log.borrow_mut().push(Call::Free);
        run.push(((0, None), log.borrow_mut().drain(..).collect()));
        run
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn rust_targets_match_c_targets_under_libgit2() {
        let fail_user = Some(Fail(ffi::GIT_EUSER, ErrorClass::Callback, c"user stop"));
        let fail_os = Some(Fail(ffi::GIT_ERROR, ErrorClass::Os, c"disk full"));
        let plans = [
            Plan::default(),
            Plan {
                write: fail_user,
                close: None,
            },
            Plan {
                write: None,
                close: fail_os,
            },
            Plan {
                write: fail_os,
                close: fail_user,
            },
        ];
        let chains = [
            Filters::None,
            Filters::IdentClean,
            Filters::Upper,
            Filters::Refused,
        ];
        for filters in chains {
            for plan in plans {
                let c = thread::spawn(move || run_c(filters, plan)).join().unwrap();
                let rust = thread::spawn(move || run_rust(filters, plan))
                    .join()
                    .unwrap();
                assert_eq!(rust, c, "{filters:?} {plan:?}");
            }
        }
    }

    /// Rust-side calls through a handle match the same calls made on the raw stream's
    /// members, success and failure alike.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handle_calls_match_member_calls() {
        let plans = [
            Plan::default(),
            Plan {
                write: Some(Fail(ffi::GIT_EUSER, ErrorClass::Filter, c"no")),
                close: Some(Fail(ffi::GIT_PASSTHROUGH, ErrorClass::None, c"pass")),
            },
        ];
        for plan in plans {
            let c = thread::spawn(move || {
                init();
                let log = Rc::new(RefCell::new(Vec::new()));
                let s = raw_stream(Recorder {
                    log: log.clone(),
                    plan,
                });
                let mut steps = Vec::new();
                for (i, len) in [0, 1, 4097].into_iter().enumerate() {
                    steps.push(c_write(s, &pattern(len, i)));
                }
                // SAFETY: the stream's own members; closed, then freed once.
                unsafe {
                    steps.push(c_reported((*s).close.unwrap()(s)));
                    (*s).free.unwrap()(s);
                }
                (steps, log.take())
            })
            .join()
            .unwrap();
            let rust = thread::spawn(move || {
                init();
                let log = Rc::new(RefCell::new(Vec::new()));
                let mut s = RustWriteStream::new(Recorder {
                    log: log.clone(),
                    plan,
                })
                .into_box();
                let mut steps = Vec::new();
                for (i, len) in [0, 1, 4097].into_iter().enumerate() {
                    steps.push(rust_reported(s.as_mut().write(&pattern(len, i))));
                }
                steps.push(rust_reported(WriteStream::close(s)));
                log.borrow_mut().push(Call::Free);
                (steps, log.take())
            })
            .join()
            .unwrap();
            assert_eq!(rust, c, "{plan:?}");
        }
    }
}

// --- `git_submodule_ignore_t` --------------------------------------------

/// A working-tree repository of its own under the temp dir, removed when dropped, with
/// one submodule `sm` declared in `.gitmodules`.
struct IgnoreRepo {
    path: PathBuf,
    repo: *mut ffi::git_repository,
}

impl IgnoreRepo {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-types-equiv-ignore-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_MKDIR;
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` live, all
        // read for the call only.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0);
        let this = Self { path, repo };
        this.declare("none");
        this
    }

    /// Rewrites `.gitmodules` with `sm`'s `ignore` spelled `value`.
    fn declare(&self, value: &str) {
        fs::write(
            self.path.join(".gitmodules"),
            format!(
                "[submodule \"sm\"]\n\tpath = sm\n\turl = https://example.invalid/sm\n\tignore = {value}\n"
            ),
        )
        .unwrap();
    }

    fn gitmodules(&self) -> String {
        fs::read_to_string(self.path.join(".gitmodules")).unwrap()
    }

    /// `git_submodule_lookup("sm")` then `git_submodule_ignore`: the return of the
    /// lookup and, on success, the value.
    fn ignore(&self) -> (c_int, Option<ffi::git_submodule_ignore_t>) {
        let mut sm = ptr::null_mut();
        // SAFETY: `sm` is a live slot, `repo` open, the name NUL-terminated.
        let rc = unsafe { ffi::git_submodule_lookup(&mut sm, self.repo, c"sm".as_ptr()) };
        if rc != 0 {
            return (rc, None);
        }
        // SAFETY: `sm` is a live submodule, released once after the read.
        let value = unsafe {
            let value = ffi::git_submodule_ignore(sm);
            ffi::git_submodule_free(sm);
            value
        };
        (rc, Some(value))
    }

    /// `git_submodule_set_ignore("sm", raw)`.
    fn set_ignore(&self, raw: ffi::git_submodule_ignore_t) -> c_int {
        // SAFETY: `repo` is open and the name NUL-terminated.
        unsafe { ffi::git_submodule_set_ignore(self.repo, c"sm".as_ptr(), raw) }
    }
}

impl Drop for IgnoreRepo {
    fn drop(&mut self) {
        // SAFETY: `repo` is open and released once.
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The thread's last error, copied out: class and message.
fn ignore_error() -> (c_int, String) {
    // SAFETY: libgit2 is initialised (a repository is open); the error, when there is
    // one, stays valid until the next libgit2 call on this thread, and is copied out.
    let err = unsafe { ffi::git_error_last() };
    assert!(!err.is_null());
    // SAFETY: as above.
    let (class, message) = unsafe { ((*err).klass, CStr::from_ptr((*err).message)) };
    (class, message.to_string_lossy().into_owned())
}

/// What one `set_ignore` left behind: its return, its error on failure, the
/// `.gitmodules` text and the value read back.
type SetRecord = (
    c_int,
    Option<(c_int, String)>,
    String,
    (c_int, Option<ffi::git_submodule_ignore_t>),
);

fn set_record(repo: &IgnoreRepo, rc: c_int) -> SetRecord {
    let err = (rc != 0).then(ignore_error);
    (rc, err, repo.gitmodules(), repo.ignore())
}

/// Every spelling C's config map accepts yields a named value, which converts back
/// unchanged; a spelling it rejects fails the lookup in C, before any value exists.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn configured_ignore_values_are_named() {
    let repo = IgnoreRepo::new("configured");
    let cases = [
        ("none", SubmoduleIgnore::None),
        ("untracked", SubmoduleIgnore::Untracked),
        ("dirty", SubmoduleIgnore::Dirty),
        ("all", SubmoduleIgnore::All),
        ("true", SubmoduleIgnore::All),
        ("false", SubmoduleIgnore::None),
    ];
    for (text, expected) in cases {
        repo.declare(text);
        let (rc, raw) = repo.ignore();
        assert_eq!(rc, 0, "{text}");
        let raw = raw.unwrap();
        let named = SubmoduleIgnore::from_raw(raw);
        assert_eq!(named, Some(expected), "{text}: {raw}");
        assert_eq!(named.unwrap().raw(), raw);
    }
    repo.declare("bogus");
    assert!(repo.ignore().0 < 0);
}

/// `git_submodule_set_ignore` sees the same value from the variant as from the raw C
/// value, step after step on one repository per side; every value without a variant
/// is one C refuses too.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_ignore_takes_the_variants_values() {
    let c = IgnoreRepo::new("set-c");
    let rust = IgnoreRepo::new("set-rust");
    let sequence: [ffi::git_submodule_ignore_t; 12] = [3, 1, 4, 2, -1, 0, 5, -2, 4, 100, 1, 3];
    for (step, raw) in sequence.into_iter().enumerate() {
        let c_rec = set_record(&c, c.set_ignore(raw));
        match SubmoduleIgnore::from_raw(raw) {
            Some(variant) => {
                let rust_rec = set_record(&rust, rust.set_ignore(variant.raw()));
                assert_eq!(c_rec, rust_rec, "step {step}: {variant:?}");
                if variant == SubmoduleIgnore::Unspecified {
                    assert_eq!(c_rec.0, -1, "C refuses Unspecified");
                } else {
                    assert_eq!(c_rec.0, 0);
                    assert_eq!(c_rec.3, (0, Some(raw)), "read back as written");
                }
            }
            None => {
                // No variant, no call: C itself refuses the value and leaves the
                // repository as the other side's.
                assert_eq!(c_rec.0, -1, "step {step}: {raw}");
                assert_eq!(
                    c_rec.1,
                    Some((
                        ffi::GIT_ERROR_SUBMODULE as c_int,
                        "invalid value for ignore".into()
                    ))
                );
                assert_eq!(c_rec.2, rust.gitmodules(), "step {step}");
                assert_eq!(c_rec.3, rust.ignore(), "step {step}");
            }
        }
    }
}

// --- `BranchType` ------------------------------------------------------------

fn open_fixture() -> *mut ffi::git_repository {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    let mut repo = ptr::null_mut();
    // SAFETY: no arguments; the extra reference dies with the process.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
    // directories.
    let rc = unsafe {
        ffi::git_repository_open_ext(
            &mut repo,
            path.as_ptr().cast(),
            ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
            ptr::null(),
        )
    };
    assert_eq!(rc, 0);
    repo
}

/// The branches `git_branch_next` lists for `flags`, with the kind it reports for each.
fn c_list(
    repo: *mut ffi::git_repository,
    flags: ffi::git_branch_t,
) -> Vec<(String, ffi::git_branch_t)> {
    let mut iter = ptr::null_mut();
    // SAFETY: `iter` receives the iterator; `repo` is open; any flags are accepted.
    let rc = unsafe { ffi::git_branch_iterator_new(&mut iter, repo, flags) };
    assert_eq!(rc, 0);
    let mut out = Vec::new();
    loop {
        let mut reference = ptr::null_mut();
        let mut kind: ffi::git_branch_t = 0;
        // SAFETY: both out-parameters are writable; `iter` is live.
        let rc = unsafe { ffi::git_branch_next(&mut reference, &mut kind, iter) };
        if rc == ffi::GIT_ITEROVER {
            break;
        }
        assert_eq!(rc, 0);
        // SAFETY: `reference` is live until freed; its name is copied out first.
        let name = unsafe { CStr::from_ptr(ffi::git_reference_name(reference)) };
        out.push((name.to_string_lossy().into_owned(), kind));
        // SAFETY: `reference` is the caller's, released once.
        unsafe { ffi::git_reference_free(reference) };
    }
    // SAFETY: `iter` is live, released once.
    unsafe { ffi::git_branch_iterator_free(iter) };
    out.sort();
    out
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn branch_iterators_filter_and_report_as_branch_type_says() {
    let repo = open_fixture();
    let all = c_list(repo, ffi::GIT_BRANCH_ALL);
    assert!(all.iter().any(|(n, _)| n.starts_with("refs/heads/")));
    assert!(all.iter().any(|(n, _)| n.starts_with("refs/remotes/")));

    // Every kind libgit2 reports is a named one, and one that names a single kind.
    for (name, kind) in &all {
        let kind = BranchType::from_raw(*kind).expect("a named kind");
        let expected = if name.starts_with("refs/heads/") {
            BranchType::Local
        } else {
            BranchType::Remote
        };
        assert_eq!(kind, expected, "{name}");
        assert_eq!(kind.raw(), expected.raw());
    }

    // Each selection lists exactly the branches whose kind it contains.
    for selection in [BranchType::Local, BranchType::Remote, BranchType::All] {
        let predicted: Vec<_> = all
            .iter()
            .filter(|(_, kind)| selection.contains(BranchType::from_raw(*kind).unwrap()))
            .cloned()
            .collect();
        assert_eq!(c_list(repo, selection.raw()), predicted, "{selection:?}");
    }
    let union = BranchType::Local | BranchType::Remote;
    assert_eq!(c_list(repo, union.raw()), all);

    // Selections the type does not name list nothing, or what their named bits do.
    for raw in [0, 4, 5, 6, 7, 8, 0x8000_0003] {
        assert!(BranchType::from_raw(raw).is_none());
        let named = raw & ffi::GIT_BRANCH_ALL;
        let expected = BranchType::from_raw(named).map_or(Vec::new(), |k| c_list(repo, k.raw()));
        assert_eq!(c_list(repo, raw), expected, "raw {raw:#x}");
    }

    // SAFETY: `repo` is open, released once.
    unsafe { ffi::git_repository_free(repo) };
}
