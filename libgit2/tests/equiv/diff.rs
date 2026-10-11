#[path = "../support/diff.rs"]
mod support;
use core::{ffi::CStr, ptr};
use libgit2::ffi;
use libgit2::libgit2::diff::diff::Diff;
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::oid::oid::OidType;
use libgit2::libgit2::util::errors::last;
use support::{DeltaSnapshot, FileSnapshot, PATCH, RENAME, Repo};

struct RawDiff(*mut ffi::git_diff);
impl Drop for RawDiff {
    fn drop(&mut self) {
        // SAFETY: each fixture adopts a fresh successful C output once; no borrow
        // escapes its snapshots, and the repository (if any) outlives this owner.
        unsafe { ffi::git_diff_free(self.0) }
    }
}
unsafe fn raw_file(f: ffi::git_diff_file) -> FileSnapshot {
    let path = if f.path.is_null() {
        None
    } else {
        // SAFETY: caller keeps the parent alive; this non-null path points to
        // its immutable NUL-terminated parent-owned string.
        Some(unsafe { CStr::from_ptr(f.path) }.to_bytes().to_vec())
    };
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
fn raw_snapshot(diff: &RawDiff) -> Vec<DeltaSnapshot> {
    // SAFETY: this fixture holds one live initialized diff throughout all reads;
    // each index is in range and all returned path contents are copied out.
    unsafe {
        let len = ffi::git_diff_num_deltas(diff.0);
        assert!(ffi::git_diff_get_delta(diff.0, len).is_null());
        (0..len)
            .map(|i| {
                let d = ffi::git_diff_get_delta(diff.0, i).read();
                (
                    d.status,
                    d.flags,
                    d.similarity,
                    d.nfiles,
                    raw_file(d.old_file),
                    raw_file(d.new_file),
                )
            })
            .collect()
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn parse_sequences_and_errors_match_c() {
    for input in [
        Vec::new(),
        PATCH.to_vec(),
        RENAME.to_vec(),
        [PATCH, RENAME].concat(),
        [PATCH, b"trailing non-patch text\n"].concat(),
        b"garbage".to_vec(),
        PATCH[..PATCH.len() - 5].to_vec(),
    ] {
        let safe_input = input.clone();
        let safe = Diff::from_buffer(&safe_input);
        drop(safe_input);
        let mut raw = ptr::null_mut();
        // SAFETY: valid byte range and writable output; runtime pinned by safe parse.
        let rc = unsafe { ffi::git_diff_from_buffer(&mut raw, input.as_ptr().cast(), input.len()) };
        match safe {
            Ok(safe) => {
                assert_eq!(rc, 0);
                let raw = RawDiff(raw);
                for _ in 0..3 {
                    let actual: Vec<_> = (0..safe.as_ref().len())
                        .map(|i| support::delta(safe.as_ref().get(i).unwrap()))
                        .collect();
                    assert_eq!(actual, raw_snapshot(&raw));
                }
                drop(raw);
                assert!(safe.as_ref().get(usize::MAX).is_none());
            }
            Err(e) => {
                assert_eq!(rc, e.raw_code());
                assert!(raw.is_null());
                let ce = last();
                assert_eq!(e.raw_class(), ce.raw_class());
                assert_eq!(e.message(), ce.message());
            }
        }
    }
    let safe = Diff::from_buffer_with_oid_type(PATCH, OidType::Sha1).unwrap();
    let mut opts = ffi::git_diff_parse_options {
        version: ffi::GIT_DIFF_PARSE_OPTIONS_VERSION,
        oid_type: ffi::GIT_OID_SHA1,
    };
    let mut raw = ptr::null_mut();
    assert_eq!(
        // SAFETY: initialized options, readable input, fresh output slot, adopted below.
        unsafe {
            ffi::git_diff_from_buffer_ext(&mut raw, PATCH.as_ptr().cast(), PATCH.len(), &mut opts)
        },
        0
    );
    assert_eq!(
        vec![support::delta(safe.as_ref().get(0).unwrap())],
        raw_snapshot(&RawDiff(raw))
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generated_multi_call_sequences_match_c() {
    let rust = Repo::new();
    let c = Repo::new();
    let rust_old = rust.tree(b"old\n");
    let rust_new = rust.tree(b"new\n");
    let c_old = c.tree(b"old\n");
    let c_new = c.tree(b"new\n");
    let ri = Index::new().unwrap();
    let ci = Index::new().unwrap();
    for step in 0..8 {
        let r = rust.get().as_ref();
        let safe = match step {
            0 => Diff::tree_to_tree(r, None, Some(rust_new.as_ref())),
            1 => Diff::tree_to_tree(r, Some(rust_old.as_ref()), Some(rust_new.as_ref())),
            2 => Diff::tree_to_tree(r, Some(rust_new.as_ref()), None),
            3 => Diff::tree_to_index(r, Some(rust_old.as_ref()), Some(ri.as_ref())),
            4 => Diff::index_to_workdir(r, None),
            5 => Diff::tree_to_workdir(r, Some(rust_old.as_ref())),
            6 => Diff::tree_to_workdir_with_index(r, Some(rust_old.as_ref())),
            _ => Diff::index_to_index(r, ri.as_ref(), ri.as_ref()),
        }
        .unwrap();
        let mut out = ptr::null_mut();
        // SAFETY: independently owned repositories, trees and indices are live for
        // each synchronous default-options call and outlive the resulting raw diff.
        let rc = unsafe {
            let repo = c.get().as_c_ptr();
            match step {
                0 => ffi::git_diff_tree_to_tree(
                    &mut out,
                    repo,
                    ptr::null_mut(),
                    c_new.as_c_ptr(),
                    ptr::null(),
                ),
                1 => ffi::git_diff_tree_to_tree(
                    &mut out,
                    repo,
                    c_old.as_c_ptr(),
                    c_new.as_c_ptr(),
                    ptr::null(),
                ),
                2 => ffi::git_diff_tree_to_tree(
                    &mut out,
                    repo,
                    c_new.as_c_ptr(),
                    ptr::null_mut(),
                    ptr::null(),
                ),
                3 => ffi::git_diff_tree_to_index(
                    &mut out,
                    repo,
                    c_old.as_c_ptr(),
                    ci.as_c_ptr(),
                    ptr::null(),
                ),
                4 => ffi::git_diff_index_to_workdir(&mut out, repo, ptr::null_mut(), ptr::null()),
                5 => ffi::git_diff_tree_to_workdir(&mut out, repo, c_old.as_c_ptr(), ptr::null()),
                6 => ffi::git_diff_tree_to_workdir_with_index(
                    &mut out,
                    repo,
                    c_old.as_c_ptr(),
                    ptr::null(),
                ),
                _ => ffi::git_diff_index_to_index(
                    &mut out,
                    repo,
                    ci.as_c_ptr(),
                    ci.as_c_ptr(),
                    ptr::null(),
                ),
            }
        };
        assert_eq!(rc, 0);
        let raw = RawDiff(out);
        let expected = raw_snapshot(&raw);
        drop(raw);
        assert_eq!(
            (0..safe.as_ref().len())
                .map(|i| support::delta(safe.as_ref().get(i).unwrap()))
                .collect::<Vec<_>>(),
            expected,
            "step {step}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn progress_cancellation_codes_match_c() {
    let rust = Repo::new();
    let c = Repo::new();
    let rust_tree = rust.tree(b"content\n");
    let c_tree = c.tree(b"content\n");
    unsafe extern "C" fn cancel(
        _: *const ffi::git_diff,
        _: *const core::ffi::c_char,
        _: *const core::ffi::c_char,
        payload: *mut core::ffi::c_void,
    ) -> i32 {
        // SAFETY: each caller supplies a live immutable i32 for this call only.
        unsafe { payload.cast::<i32>().read() }
    }
    for code in [-123, 123] {
        let rust_code = code;
        let c_code = code;
        let mut rust_opts = ffi::git_diff_options::default();
        let mut c_opts = ffi::git_diff_options::default();
        // SAFETY: two writable option structs, supported version, pinned runtime.
        unsafe {
            assert_eq!(
                ffi::git_diff_options_init(&mut rust_opts, ffi::GIT_DIFF_OPTIONS_VERSION),
                0
            );
            assert_eq!(
                ffi::git_diff_options_init(&mut c_opts, ffi::GIT_DIFF_OPTIONS_VERSION),
                0
            );
        }
        rust_opts.progress_cb = Some(cancel);
        rust_opts.payload = (&raw const rust_code).cast_mut().cast();
        c_opts.progress_cb = Some(cancel);
        c_opts.payload = (&raw const c_code).cast_mut().cast();
        libgit2::libgit2::util::sys::errors::clear();
        // SAFETY: initialized defaults with a static non-unwinding callback and
        // matching live read-only payload; repo, options and referents outlive result.
        let safe = unsafe {
            Diff::generate_with_options(
                rust.get().as_ref(),
                libgit2::libgit2::diff::diff::DiffInputs::Trees {
                    old: None,
                    new: Some(rust_tree.as_ref()),
                },
                &rust_opts,
            )
        }
        .err()
        .unwrap();
        libgit2::libgit2::util::sys::errors::clear();
        let mut out = ptr::null_mut();
        // SAFETY: independent valid inputs; callback payload is live and read only.
        let rc = unsafe {
            ffi::git_diff_tree_to_tree(
                &mut out,
                c.get().as_c_ptr(),
                ptr::null_mut(),
                c_tree.as_c_ptr(),
                &c_opts,
            )
        };
        assert_eq!(rc, code);
        assert!(out.is_null());
        assert_eq!(safe.raw_code(), rc);
        let error = last();
        assert_eq!(safe.error(), &error);
    }
}
