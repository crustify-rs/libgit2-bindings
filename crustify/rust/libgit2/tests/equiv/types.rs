//! Equivalence of `git2/types.h`'s [`SubmoduleRecurse`] and [`SubmoduleUpdate`] with
//! the `git_submodule_recurse_t` / `git_submodule_update_t` values libgit2 declares,
//! reads, reports and writes.
//!
//! Each Rust type must name exactly the values of its C enum, each with its C value,
//! except `GIT_SUBMODULE_UPDATE_DEFAULT`, which libgit2 never produces and refuses to
//! write. libgit2 observes the types through a submodule: `git_submodule_lookup` reads
//! the `.gitmodules` setting into the value `git_submodule_update_strategy` /
//! `git_submodule_fetch_recurse_submodules` report, and
//! `git_submodule_set_update` / `git_submodule_set_fetch_recurse_submodules` write a
//! value back to `.gitmodules`, or refuse it; `git_submodule_init` copies the update
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

use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::core::types::{
    SubmoduleRecurse, SubmoduleUpdate, Time, UnnamedSubmoduleRecurse, UnnamedSubmoduleUpdate,
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
