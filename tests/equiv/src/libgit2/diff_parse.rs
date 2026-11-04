//! Equivalence tests for `src/libgit2/diff_parse.c`'s parse options: the patch parser run
//! with [`DiffParseOptions`] from Rust and with the same `git_diff_parse_options` from C,
//! over every object id type, versions libgit2 does not read, and options edited between
//! parses.

use core::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::diff::{Diff, DiffBox, DiffDeltaRef, DiffError, DiffFileRef};
use libgit2::libgit2::src::libgit2::diff_parse::DiffParseOptions;
use libgit2::libgit2::src::libgit2::oid::OidType;
use libgit2::libgit2::src::util::errors::last;

/// A git patch with abbreviated ids.
const PATCH: &[u8] = b"diff --git a/file b/file\nindex 1111111..2222222 100644\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
/// A pure rename.
const RENAME: &[u8] =
    b"diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";

type FileSnapshot = (Option<Vec<u8>>, [u8; 32], u8, u64, u32, u16, u16);
type DeltaSnapshot = (u32, u32, u16, u16, FileSnapshot, FileSnapshot);

fn safe_file(f: DiffFileRef<'_>) -> FileSnapshot {
    (
        f.path().map(|p| p.to_bytes().to_vec()),
        f.id().id(),
        f.id().raw_kind(),
        f.size(),
        f.flags(),
        f.mode(),
        f.id_abbrev(),
    )
}

fn safe_delta(d: DiffDeltaRef<'_>) -> DeltaSnapshot {
    (
        d.status(),
        d.flags(),
        d.similarity(),
        d.nfiles(),
        safe_file(d.old_file()),
        safe_file(d.new_file()),
    )
}

/// A patch whose ids are full SHA-256 ids: 64 hex digits, too long for SHA-1.
const PATCH_256: &[u8] = b"diff --git a/file b/file\nindex \
1111111111111111111111111111111111111111111111111111111111111111..\
2222222222222222222222222222222222222222222222222222222222222222 100644\n\
--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";

/// A patch whose ids are full SHA-1 ids: 40 hex digits.
const PATCH_1: &[u8] = b"diff --git a/file b/file\nindex \
1111111111111111111111111111111111111111..2222222222222222222222222222222222222222 100644\n\
--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";

fn inputs() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        PATCH.to_vec(),
        PATCH_1.to_vec(),
        PATCH_256.to_vec(),
        [PATCH, RENAME].concat(),
        b"garbage".to_vec(),
        PATCH_256[..PATCH_256.len() - 5].to_vec(),
    ]
}

fn safe_snapshot(diff: &DiffBox<'static>) -> Vec<DeltaSnapshot> {
    (0..diff.as_ref().len())
        .map(|i| safe_delta(diff.as_ref().get(i).unwrap()))
        .collect()
}

/// The C result of parsing `input` with `opts` (NULL for `None`): the return code, and on
/// success the deltas, read back through the safe handles of the adopted diff.
fn c_parse(
    input: &[u8],
    opts: Option<ffi::git_diff_parse_options>,
) -> (i32, Option<Vec<DeltaSnapshot>>) {
    let mut opts = opts;
    let opts_ptr = opts.as_mut().map_or(ptr::null_mut(), ptr::from_mut);
    let mut raw = ptr::null_mut();
    // SAFETY: `input` is readable for its length, `raw` is a writable slot, and `opts_ptr` is
    // NULL or a live options value C only reads; the runtime is pinned by the safe parse
    // every caller runs first.
    let rc = unsafe {
        ffi::git_diff_from_buffer_ext(&raw mut raw, input.as_ptr().cast(), input.len(), opts_ptr)
    };
    if rc != 0 {
        assert!(raw.is_null());
        return (rc, None);
    }
    // Read the C diff back through the test's own C calls.
    // SAFETY: a successful parse hands out one live diff, released below.
    let len = unsafe { ffi::git_diff_num_deltas(raw) };
    let mut out = Vec::new();
    for i in 0..len {
        // SAFETY: `i < len`, so the delta lives as long as the diff.
        let d = unsafe { ffi::git_diff_get_delta(raw, i) };
        // SAFETY: as above; the delta is copied out, its paths read before the release.
        let d = unsafe { d.read() };
        out.push(snapshot_of(&d));
    }
    // SAFETY: the diff is released once, after its last read.
    unsafe { ffi::git_diff_free(raw) };
    (rc, Some(out))
}

fn file_of(f: &ffi::git_diff_file) -> FileSnapshot {
    let path = (!f.path.is_null()).then(|| {
        // SAFETY: a non-null path of a live delta is its diff's NUL-terminated string.
        unsafe { core::ffi::CStr::from_ptr(f.path) }
            .to_bytes()
            .to_vec()
    });
    (
        path,
        f.id.id,
        f.id.type_,
        f.size,
        f.flags,
        f.mode,
        f.id_abbrev,
    )
}

fn snapshot_of(d: &ffi::git_diff_delta) -> DeltaSnapshot {
    (
        d.status,
        d.flags,
        d.similarity,
        d.nfiles,
        file_of(&d.old_file),
        file_of(&d.new_file),
    )
}

/// Compares one safe parse with C's, including the error's code, class and message.
fn assert_matches(
    safe: Result<DiffBox<'static>, DiffError>,
    input: &[u8],
    raw: ffi::git_diff_parse_options,
) {
    match safe {
        Ok(safe) => {
            let (rc, c) = c_parse(input, Some(raw));
            assert_eq!(rc, 0);
            assert_eq!(Some(safe_snapshot(&safe)), c);
        }
        Err(e) => {
            let (rc, c) = c_parse(input, Some(raw));
            assert!(c.is_none());
            assert_eq!(rc, e.raw_code());
            let ce = last();
            assert_eq!(e.raw_class(), ce.raw_class());
            assert_eq!(e.message(), ce.message());
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn options_parse_as_c_does_for_every_type_and_version() {
    for input in inputs() {
        for kind in [None, Some(OidType::Sha1), Some(OidType::Sha256)] {
            for version in [0, DiffParseOptions::VERSION, 2, u32::MAX] {
                let mut opts = DiffParseOptions::new();
                opts.as_mut().set_oid_type(kind);
                opts.as_mut().set_version(version);
                let raw = ffi::git_diff_parse_options {
                    version,
                    oid_type: kind.map_or(0, OidType::raw),
                };
                assert_eq!(
                    ffi::git_diff_parse_options::from(opts).oid_type,
                    raw.oid_type
                );
                assert_eq!(ffi::git_diff_parse_options::from(opts).version, raw.version);
                let safe_input = input.clone();
                let safe = Diff::from_buffer_with_options(&safe_input, opts);
                drop(safe_input);
                assert_matches(safe, &input, raw);
            }
        }
    }
}

/// Default options, NULL options and the SHA-1 type parse alike, in C and in Rust.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn default_options_match_null_options() {
    for input in inputs() {
        let safe = Diff::from_buffer_with_options(&input, DiffParseOptions::default());
        let plain = Diff::from_buffer(&input);
        let (rc_null, c_null) = c_parse(&input, None);
        match (safe, plain) {
            (Ok(safe), Ok(plain)) => {
                assert_eq!(rc_null, 0);
                assert_eq!(Some(safe_snapshot(&safe)), c_null);
                assert_eq!(safe_snapshot(&plain), safe_snapshot(&safe));
                let sha1 = Diff::from_buffer_with_oid_type(&input, OidType::Sha1).unwrap();
                assert_eq!(safe_snapshot(&sha1), safe_snapshot(&safe));
            }
            (Err(a), Err(b)) => {
                assert_eq!(a.raw_code(), rc_null);
                assert_eq!(b.raw_code(), rc_null);
            }
            _ => panic!("default options and from_buffer disagree"),
        }
    }
}

/// One options value edited between parses, in Rust and in a C struct alongside it,
/// compared after every step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn edited_options_sequence_matches_c() {
    let mut opts = DiffParseOptions::new();
    let mut raw = ffi::git_diff_parse_options {
        version: ffi::GIT_DIFF_PARSE_OPTIONS_VERSION,
        oid_type: 0,
    };
    let steps: [(Option<OidType>, u32, &[u8]); 6] = [
        (Some(OidType::Sha256), 1, PATCH_256),
        (Some(OidType::Sha1), 1, PATCH_256),
        (Some(OidType::Sha1), 0, PATCH_1),
        (None, 9, PATCH_1),
        (Some(OidType::Sha256), 9, PATCH_1),
        (None, 1, PATCH),
    ];
    for (kind, version, input) in steps {
        {
            let mut m = opts.as_mut();
            m.set_oid_type(kind);
            m.set_version(version);
        }
        raw.oid_type = kind.map_or(0, OidType::raw);
        raw.version = version;
        assert_eq!(opts.as_ref().oid_type(), OidType::from_raw(raw.oid_type));
        assert_eq!(opts.as_ref().version(), raw.version);
        assert_matches(Diff::from_buffer_with_options(input, opts), input, raw);
    }
}
