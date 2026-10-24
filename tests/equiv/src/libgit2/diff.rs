#[path = "../../../support/diff.rs"]
mod support;
use core::ffi::CStr;
use core::ptr;
use std::ffi::CString;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::blob;
use libgit2::libgit2::src::libgit2::diff::{Diff, DiffBox, DiffError};
use libgit2::libgit2::src::libgit2::index::Index;
use libgit2::libgit2::src::libgit2::oid::OidType;
use libgit2::libgit2::src::libgit2::repository::Repository;
use libgit2::libgit2::src::libgit2::tree::{FileMode, Tree, TreeArc, TreeBuilder};
use libgit2::libgit2::src::util::errors::last;
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
    {
        // SAFETY: this fixture holds one live initialized diff throughout all reads;
        // each index is in range and all returned path contents are copied out.
        let len = unsafe { ffi::git_diff_num_deltas(diff.0) };
        // SAFETY: as above.
        unsafe { assert!(ffi::git_diff_get_delta(diff.0, len).is_null()) };
        (0..len)
            .map(|i| {
                // SAFETY: as above; `i < len`, so the delta is live.
                let d = unsafe { ffi::git_diff_get_delta(diff.0, i) };
                // SAFETY: as above; the delta is copied.
                let d = unsafe { d.read() };
                // SAFETY: as above, its files' paths live with the diff.
                let old = unsafe { raw_file(d.old_file) };
                // SAFETY: as above.
                let new = unsafe { raw_file(d.new_file) };
                (d.status, d.flags, d.similarity, d.nfiles, old, new)
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
        let rc =
            unsafe { ffi::git_diff_from_buffer(&raw mut raw, input.as_ptr().cast(), input.len()) };
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
            ffi::git_diff_from_buffer_ext(
                &raw mut raw,
                PATCH.as_ptr().cast(),
                PATCH.len(),
                &raw mut opts,
            )
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
        let rc = {
            let repo = c.get().as_c_ptr();
            // SAFETY: independently owned repositories, trees and indices are live for
            // each synchronous default-options call and outlive the resulting raw diff.

            match step {
                // SAFETY: as above.
                0 => unsafe {
                    ffi::git_diff_tree_to_tree(
                        &raw mut out,
                        repo,
                        ptr::null_mut(),
                        c_new.as_c_ptr(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                1 => unsafe {
                    ffi::git_diff_tree_to_tree(
                        &raw mut out,
                        repo,
                        c_old.as_c_ptr(),
                        c_new.as_c_ptr(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                2 => unsafe {
                    ffi::git_diff_tree_to_tree(
                        &raw mut out,
                        repo,
                        c_new.as_c_ptr(),
                        ptr::null_mut(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                3 => unsafe {
                    ffi::git_diff_tree_to_index(
                        &raw mut out,
                        repo,
                        c_old.as_c_ptr(),
                        ci.as_c_ptr(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                4 => unsafe {
                    ffi::git_diff_index_to_workdir(&raw mut out, repo, ptr::null_mut(), ptr::null())
                },
                // SAFETY: as above.
                5 => unsafe {
                    ffi::git_diff_tree_to_workdir(&raw mut out, repo, c_old.as_c_ptr(), ptr::null())
                },
                // SAFETY: as above.
                6 => unsafe {
                    ffi::git_diff_tree_to_workdir_with_index(
                        &raw mut out,
                        repo,
                        c_old.as_c_ptr(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                _ => unsafe {
                    ffi::git_diff_index_to_index(
                        &raw mut out,
                        repo,
                        ci.as_c_ptr(),
                        ci.as_c_ptr(),
                        ptr::null(),
                    )
                },
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
#[expect(
    clippy::items_after_statements,
    reason = "each helper item sits beside the steps that use it"
)]
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
                ffi::git_diff_options_init(&raw mut rust_opts, ffi::GIT_DIFF_OPTIONS_VERSION),
                0
            );
        };
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                ffi::git_diff_options_init(&raw mut c_opts, ffi::GIT_DIFF_OPTIONS_VERSION),
                0
            );
        };
        rust_opts.progress_cb = Some(cancel);
        rust_opts.payload = (&raw const rust_code).cast_mut().cast();
        c_opts.progress_cb = Some(cancel);
        c_opts.payload = (&raw const c_code).cast_mut().cast();
        libgit2::libgit2::src::util::errors::clear();
        // SAFETY: initialized defaults with a static non-unwinding callback and
        // matching live read-only payload; repo, options and referents outlive result.
        let safe = unsafe {
            Diff::generate_with_options(
                rust.get().as_ref(),
                libgit2::libgit2::src::libgit2::diff::DiffInputs::Trees {
                    old: None,
                    new: Some(rust_tree.as_ref()),
                },
                &rust_opts,
            )
        }
        .err()
        .unwrap();
        libgit2::libgit2::src::util::errors::clear();
        let mut out = ptr::null_mut();
        // SAFETY: independent valid inputs; callback payload is live and read only.
        let rc = unsafe {
            ffi::git_diff_tree_to_tree(
                &raw mut out,
                c.get().as_c_ptr(),
                ptr::null_mut(),
                c_tree.as_c_ptr(),
                &raw const c_opts,
            )
        };
        assert_eq!(rc, code);
        assert!(out.is_null());
        assert_eq!(safe.raw_code(), rc);
        let error = last();
        assert_eq!(safe.error(), &error);
    }
}

/// Compares a safe generator's result with the C call's `rc`/`out` on independent
/// inputs: the deltas on success, the code and the thread's error on failure.
fn assert_generated_like_c(
    safe: Result<DiffBox<'_>, DiffError>,
    rc: i32,
    out: *mut ffi::git_diff,
    step: &str,
) {
    match safe {
        Ok(safe) => {
            assert_eq!(rc, 0, "{step}");
            let expected = raw_snapshot(&RawDiff(out));
            let actual: Vec<_> = (0..safe.as_ref().len())
                .map(|i| support::delta(safe.as_ref().get(i).unwrap()))
                .collect();
            assert_eq!(actual, expected, "{step}");
        }
        Err(e) => {
            let ce = last();
            assert_eq!((e.raw_code(), out.is_null()), (rc, true), "{step}");
            assert_eq!(e.error(), &ce, "{step}");
        }
    }
}

/// A tree of `files` written to `fixture`'s object database.
fn tree_of<'r>(fixture: &'r Repo, files: &[(&CStr, &[u8])]) -> TreeArc<'r> {
    let repo = fixture.get().as_ref();
    let mut builder = TreeBuilder::new(repo).unwrap();
    for (name, text) in files {
        let id = blob::create_from_buffer(repo, text).unwrap();
        builder
            .as_mut()
            .insert(name, id.as_ref(), FileMode::Blob)
            .unwrap();
    }
    let id = builder.as_mut().write().unwrap();
    Tree::lookup(repo, id.as_ref()).unwrap()
}

struct RawIndex(*mut ffi::git_index);
impl Drop for RawIndex {
    fn drop(&mut self) {
        // SAFETY: releases the one counted reference git_repository_index handed out.
        unsafe { ffi::git_index_free(self.0) }
    }
}

/// Generators over a populated working directory and a case-insensitive repository
/// index: C re-sorts the deltas case-insensitively for tree/index and index/index
/// diffs, whether the index is passed or loaded from the repository, and ignores a
/// failed reload of the repository's index. Workdir diffs on a bare repository fail
/// with the same code and error.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario, its steps run and compared in order"
)]
fn case_insensitive_populated_and_bare_generators_match_c() {
    let rust = Repo::new();
    let c = Repo::new();
    let mut indices = Vec::new();
    for fixture in [&rust, &c] {
        for (name, text) in [("B", "b\n"), ("a", "a\n"), ("c", "untracked\n")] {
            std::fs::write(fixture.path.join(name), text).unwrap();
        }
        let mut index = ptr::null_mut();
        // SAFETY: writable slot and a live repository; the counted reference is
        // released by RawIndex, and both are used on this thread only.
        let rc = unsafe { ffi::git_repository_index(&raw mut index, fixture.get().as_c_ptr()) };
        assert_eq!(rc, 0);
        let index = RawIndex(index);
        {
            let caps = ffi::GIT_INDEX_CAPABILITY_IGNORE_CASE;
            // SAFETY: the live repository index, used on this thread only; C copies the
            // paths and reads the staged files from the working directory.
            unsafe { assert_eq!(ffi::git_index_set_caps(index.0, caps), 0) };
            // SAFETY: as above.
            unsafe {
                for path in [c"B", c"a"] {
                    assert_eq!(ffi::git_index_add_bypath(index.0, path.as_ptr()), 0);
                }
            }
        }
        std::fs::write(fixture.path.join("a"), "changed a\n").unwrap();
        indices.push(index);
    }
    let (rust_raw_index, c_index) = (&indices[0], &indices[1]);
    let rust_index = rust.get().as_ref().index().unwrap();
    assert_eq!(rust_index.as_c_ptr(), rust_raw_index.0);
    let files: [(&CStr, &[u8]); 2] = [(c"B", b"old b\n"), (c"a", b"a\n")];
    let (rust_tree, c_tree) = (tree_of(&rust, &files), tree_of(&c, &files));
    let (rust_empty, c_empty) = (Index::new().unwrap(), Index::new().unwrap());
    for step in 0..5 {
        let r = rust.get().as_ref();
        let safe = match step {
            0 => Diff::tree_to_index(r, Some(rust_tree.as_ref()), Some(rust_index.as_ref())),
            1 => Diff::tree_to_index(r, Some(rust_tree.as_ref()), None),
            2 => Diff::index_to_index(r, rust_index.as_ref(), rust_empty.as_ref()),
            3 => Diff::index_to_workdir(r, Some(rust_index.as_ref())),
            _ => Diff::tree_to_workdir_with_index(r, Some(rust_tree.as_ref())),
        };
        if step == 2 {
            let diff = safe.as_ref().unwrap().as_ref();
            let paths: Vec<_> = (0..diff.len())
                .map(|i| diff.get(i).unwrap().old_file().path().unwrap())
                .collect();
            assert_eq!(paths, [c"a", c"B"]);
        }
        let mut out = ptr::null_mut();
        let rc = {
            let repo = c.get().as_c_ptr();
            // SAFETY: independently owned repository, tree and indices, live for each
            // synchronous default-options call and outliving the raw diff it returns.

            match step {
                // SAFETY: as above.
                0 => unsafe {
                    ffi::git_diff_tree_to_index(
                        &raw mut out,
                        repo,
                        c_tree.as_c_ptr(),
                        c_index.0,
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                1 => unsafe {
                    ffi::git_diff_tree_to_index(
                        &raw mut out,
                        repo,
                        c_tree.as_c_ptr(),
                        ptr::null_mut(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                2 => unsafe {
                    ffi::git_diff_index_to_index(
                        &raw mut out,
                        repo,
                        c_index.0,
                        c_empty.as_c_ptr(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                3 => unsafe {
                    ffi::git_diff_index_to_workdir(&raw mut out, repo, c_index.0, ptr::null())
                },
                // SAFETY: as above.
                _ => unsafe {
                    ffi::git_diff_tree_to_workdir_with_index(
                        &raw mut out,
                        repo,
                        c_tree.as_c_ptr(),
                        ptr::null(),
                    )
                },
            }
        };
        assert_generated_like_c(safe, rc, out, &format!("populated step {step}"));
    }
    // An index file that no longer parses: reloading the repository's index fails,
    // and C clears that error and diffs whatever the failed reload left in memory.
    for fixture in [&rust, &c] {
        std::fs::write(fixture.path.join(".git/index"), b"not an index").unwrap();
    }
    for step in 0..2 {
        let r = rust.get().as_ref();
        let safe = match step {
            0 => Diff::tree_to_index(r, Some(rust_tree.as_ref()), None),
            _ => Diff::index_to_workdir(r, None),
        };
        assert!(safe.is_ok());
        let mut out = ptr::null_mut();
        let rc = {
            let repo = c.get().as_c_ptr();
            // SAFETY: an independently owned live repository and tree; NULL loads the
            // repository's own index, as on the Rust side.

            match step {
                // SAFETY: as above.
                0 => unsafe {
                    ffi::git_diff_tree_to_index(
                        &raw mut out,
                        repo,
                        c_tree.as_c_ptr(),
                        ptr::null_mut(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                _ => unsafe {
                    ffi::git_diff_index_to_workdir(&raw mut out, repo, ptr::null_mut(), ptr::null())
                },
            }
        };
        assert_generated_like_c(safe, rc, out, &format!("unparsable index step {step}"));
    }
    drop(rust_index);
    let bare = |tag: &str| {
        let path =
            std::env::temp_dir().join(format!("crustify-diff-bare-{tag}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        let repo = Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), true);
        Repo {
            repo: Some(repo.unwrap()),
            path,
        }
    };
    let (rust, c) = (bare("rust"), bare("c"));
    for step in 0..4 {
        let r = rust.get().as_ref();
        let safe = match step {
            0 => Diff::index_to_workdir(r, None),
            1 => Diff::tree_to_workdir(r, None),
            2 => Diff::tree_to_workdir_with_index(r, None),
            _ => Diff::tree_to_index(r, None, None),
        };
        // Only the working-directory generators need a working directory.
        assert_eq!(
            safe.as_ref().err().map(DiffError::raw_code),
            (step < 3).then_some(ffi::GIT_EBAREREPO)
        );
        let mut out = ptr::null_mut();
        let rc = {
            let repo = c.get().as_c_ptr();
            // SAFETY: an independently owned live repository and NULL optional inputs.

            match step {
                // SAFETY: as above.
                0 => unsafe {
                    ffi::git_diff_index_to_workdir(&raw mut out, repo, ptr::null_mut(), ptr::null())
                },
                // SAFETY: as above.
                1 => unsafe {
                    ffi::git_diff_tree_to_workdir(&raw mut out, repo, ptr::null_mut(), ptr::null())
                },
                // SAFETY: as above.
                2 => unsafe {
                    ffi::git_diff_tree_to_workdir_with_index(
                        &raw mut out,
                        repo,
                        ptr::null_mut(),
                        ptr::null(),
                    )
                },
                // SAFETY: as above.
                _ => unsafe {
                    ffi::git_diff_tree_to_index(
                        &raw mut out,
                        repo,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null(),
                    )
                },
            }
        };
        assert_generated_like_c(safe, rc, out, &format!("bare step {step}"));
    }
}
