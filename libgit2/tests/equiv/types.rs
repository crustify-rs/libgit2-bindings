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

use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::core::types::{
    FileMode, NotAReferenceType, NotAnObjectType, ObjectType, ReferenceType, SubmoduleRecurse,
    SubmoduleUpdate, Time, UnnamedFileMode, UnnamedSubmoduleRecurse, UnnamedSubmoduleUpdate,
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
