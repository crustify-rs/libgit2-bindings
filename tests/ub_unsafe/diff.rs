#[path = "../support/diff.rs"]
mod support;
use std::ffi::CString;

use libgit2::ffi;
use libgit2::libgit2::diff::diff::{Diff, DiffInputs};
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::object::tree::Tree;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;
use libgit2::libgit2::util::errors::ErrorClass;
use support::Repo;

/// Suite for Diff::generate_with_options: each generator, empty inputs, callbacks,
/// retained payload lifetime, skip/cancel codes and reverse diffs.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generate_with_options_contract() {
    let fixture = Repo::new();
    let repo = fixture.get().as_ref();
    let old = fixture.tree(b"old\n");
    let new = fixture.tree(b"new\n");
    let index = Index::new().unwrap();
    let mut opts = ffi::git_diff_options::default();
    assert_eq!(
        // SAFETY: writable initialized storage of the right type, supported version.
        unsafe { ffi::git_diff_options_init(&mut opts, ffi::GIT_DIFF_OPTIONS_VERSION) },
        0
    );
    for input in [
        DiffInputs::Trees {
            old: Some(old.as_ref()),
            new: Some(new.as_ref()),
        },
        DiffInputs::TreeToIndex {
            old: Some(old.as_ref()),
            index: Some(index.as_ref()),
        },
        DiffInputs::IndexToWorkdir { index: None },
        DiffInputs::IndexToWorkdir {
            index: Some(index.as_ref()),
        },
        DiffInputs::TreeToWorkdir {
            old: Some(old.as_ref()),
        },
        DiffInputs::TreeToWorkdirWithIndex { old: None },
        DiffInputs::TreeToWorkdirWithIndex {
            old: Some(old.as_ref()),
        },
        DiffInputs::Indices {
            old: index.as_ref(),
            new: index.as_ref(),
        },
    ] {
        // SAFETY: initialized defaults, no nested pointers or callbacks. All inputs
        // use SHA1 and remain live; opts and repo outlive the result.
        let d = unsafe { Diff::generate_with_options(repo, input, &opts) }.unwrap();
        for i in 0..d.as_ref().len() {
            support::delta(d.as_ref().get(i).unwrap());
        }
    }
    unsafe extern "C" fn notify(
        _: *const ffi::git_diff,
        _: *const ffi::git_diff_delta,
        _: *const core::ffi::c_char,
        p: *mut core::ffi::c_void,
    ) -> i32 {
        // SAFETY: the test passes a live, exclusively accessed (calls, result) pair.
        let state = unsafe { &mut *p.cast::<(usize, i32)>() };
        state.0 += 1;
        state.1
    }
    for result in [0, 1, -123] {
        let mut state = (0usize, result);
        opts.notify_cb = Some(notify);
        opts.payload = (&raw mut state).cast();
        opts.flags = ffi::GIT_DIFF_REVERSE;
        // SAFETY: callback matches the payload, never unwinds/reenters/retains any
        // pointer. Payload and opts remain live until result is dropped; no
        // conflicting payload access occurs during the call.
        let diff = unsafe {
            Diff::generate_with_options(
                repo,
                DiffInputs::Trees {
                    old: None,
                    new: Some(new.as_ref()),
                },
                &opts,
            )
        };
        match result {
            0 => assert_eq!(
                diff.as_ref().unwrap().as_ref().get(0).unwrap().status(),
                ffi::GIT_DELTA_DELETED
            ),
            1 => assert!(diff.as_ref().unwrap().as_ref().is_empty()),
            _ => assert_eq!(diff.as_ref().err().unwrap().raw_code(), result),
        }
        drop(diff);
        assert_eq!(state.0, 1);
    }
    unsafe extern "C" fn cancel_progress(
        _: *const ffi::git_diff,
        _: *const core::ffi::c_char,
        _: *const core::ffi::c_char,
        _: *mut core::ffi::c_void,
    ) -> i32 {
        123
    }
    opts.notify_cb = None;
    opts.payload = core::ptr::null_mut();
    opts.progress_cb = Some(cancel_progress);
    // SAFETY: callback ignores its call-scoped inputs and null payload, never
    // unwinds or reenters, and remains valid forever. Other fields are defaults.
    let result = unsafe {
        Diff::generate_with_options(
            repo,
            DiffInputs::Trees {
                old: None,
                new: Some(new.as_ref()),
            },
            &opts,
        )
    };
    assert_eq!(result.err().unwrap().raw_code(), 123);
}

/// What libgit2 checks itself is no caller obligation: another options version and an
/// `oid_type` other than the repository's fail with `GIT_ERROR_INVALID` for every
/// generator, and inputs of the other object-id type are accepted. A borrowed pathspec
/// (the nested-pointer obligation) filters every generator, as patterns or, with
/// `GIT_DIFF_DISABLE_PATHSPEC_MATCH`, as literal paths.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generate_with_options_checked_fields() {
    let fixture = Repo::new();
    let repo = fixture.get().as_ref();
    let tree = fixture.tree(b"old\n");
    let index = Index::new().unwrap();
    let mut opts = ffi::git_diff_options::default();
    // SAFETY: writable initialized storage of the right type, supported version.
    let rc = unsafe { ffi::git_diff_options_init(&mut opts, ffi::GIT_DIFF_OPTIONS_VERSION) };
    assert_eq!(rc, 0);
    let inputs = || {
        [
            DiffInputs::Trees {
                old: None,
                new: Some(tree.as_ref()),
            },
            DiffInputs::TreeToIndex {
                old: Some(tree.as_ref()),
                index: Some(index.as_ref()),
            },
            DiffInputs::IndexToWorkdir { index: None },
            DiffInputs::TreeToWorkdir { old: None },
            DiffInputs::TreeToWorkdirWithIndex { old: None },
            DiffInputs::Indices {
                old: index.as_ref(),
                new: index.as_ref(),
            },
        ]
    };
    for (version, oid_type) in [
        (ffi::GIT_DIFF_OPTIONS_VERSION + 1, 0),
        (0, 0),
        (ffi::GIT_DIFF_OPTIONS_VERSION, ffi::GIT_OID_SHA256),
    ] {
        (opts.version, opts.oid_type) = (version, oid_type);
        for input in inputs() {
            // SAFETY: initialized defaults apart from the two checked fields: no
            // nested pointers or callbacks; opts and repo outlive any result.
            let e = unsafe { Diff::generate_with_options(repo, input, &opts) }.unwrap_err();
            assert_eq!(e.raw_code(), -1);
            assert_eq!(e.class(), Some(ErrorClass::Invalid));
        }
    }
    (opts.version, opts.oid_type) = (ffi::GIT_DIFF_OPTIONS_VERSION, ffi::GIT_OID_SHA1);
    // A pathspec, matched as patterns or (DISABLE_PATHSPEC_MATCH) as literal paths.
    let mut paths = [c"fil*".as_ptr().cast_mut()];
    opts.pathspec = ffi::git_strarray {
        strings: paths.as_mut_ptr(),
        count: paths.len(),
    };
    for (flags, expected) in [(0, 1), (ffi::GIT_DIFF_DISABLE_PATHSPEC_MATCH, 0)] {
        opts.flags = flags;
        for input in inputs() {
            // Only the tree side holds `file`; the working directory and indices are
            // empty.
            let tree_side = matches!(
                input,
                DiffInputs::Trees { .. } | DiffInputs::TreeToIndex { .. }
            );
            // SAFETY: a one-element array of a static NUL-terminated pattern, live with
            // `paths` and `opts` beyond every result; no callbacks.
            let diff = unsafe { Diff::generate_with_options(repo, input, &opts) }.unwrap();
            let len = if tree_side { expected } else { 0 };
            assert_eq!(diff.as_ref().len(), len, "flags {flags}");
        }
    }
    (opts.flags, opts.pathspec) = (0, ffi::git_strarray::default());
    let res = format!("{}/c/tests/resources", env!("CARGO_MANIFEST_DIR"));
    let sha256 =
        Repository::open(&CString::new(format!("{res}/testrepo_256.git")).unwrap()).unwrap();
    let flat = Oid::from_string(
        c"f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736",
        OidType::Sha256,
    )
    .unwrap();
    let foreign = Tree::lookup(sha256.as_ref(), flat.as_ref()).unwrap();
    let input = DiffInputs::Trees {
        old: Some(foreign.as_ref()),
        new: None,
    };
    // SAFETY: initialized defaults naming the repository's own object-id type, no
    // nested pointers or callbacks; the foreign tree is live for the call.
    let diff = unsafe { Diff::generate_with_options(repo, input, &opts) }.unwrap();
    let diff = diff.as_ref();
    assert_eq!(diff.len(), 3);
    for i in 0..diff.len() {
        let delta = diff.get(i).unwrap();
        assert_eq!(delta.status(), ffi::GIT_DELTA_DELETED);
        assert_eq!(delta.old_file().id().kind(), Some(OidType::Sha256));
    }
}

/// An empty pathspec pattern beside a non-empty one is refused before libgit2 compiles
/// the options' pathspec, which would read the byte after the empty pattern's
/// terminator (`parse_optimized_patterns`): both orders, for every generator, with
/// one-byte heap strings so the sanitizers see any read past them. Empty and
/// all-empty pathspecs still generate.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn generate_with_options_refuses_empty_pattern_beside_others() {
    let fixture = Repo::new();
    let repo = fixture.get().as_ref();
    let tree = fixture.tree(b"old\n");
    let index = Index::new().unwrap();
    let mut opts = ffi::git_diff_options::default();
    // SAFETY: writable initialized storage of the right type, supported version.
    let rc = unsafe { ffi::git_diff_options_init(&mut opts, ffi::GIT_DIFF_OPTIONS_VERSION) };
    assert_eq!(rc, 0);
    let inputs = || {
        [
            DiffInputs::Trees {
                old: None,
                new: Some(tree.as_ref()),
            },
            DiffInputs::TreeToIndex {
                old: Some(tree.as_ref()),
                index: Some(index.as_ref()),
            },
            DiffInputs::IndexToWorkdir { index: None },
            DiffInputs::TreeToWorkdir { old: None },
            DiffInputs::TreeToWorkdirWithIndex { old: None },
            DiffInputs::Indices {
                old: index.as_ref(),
                new: index.as_ref(),
            },
        ]
    };
    for (patterns, refused) in [
        (&["a", ""][..], true),
        (&["", "fil*"][..], true),
        (&[][..], false),
        (&[""][..], false),
        (&["", ""][..], false),
    ] {
        let owned: Vec<CString> = patterns.iter().map(|p| CString::new(*p).unwrap()).collect();
        let mut strings: Vec<_> = owned.iter().map(|p| p.as_ptr().cast_mut()).collect();
        opts.pathspec = ffi::git_strarray {
            strings: strings.as_mut_ptr(),
            count: strings.len(),
        };
        for input in inputs() {
            // SAFETY: a readable array of `count` NUL-terminated heap strings, live with
            // `owned`, `strings` and `opts` beyond the result; no callbacks.
            let result = unsafe { Diff::generate_with_options(repo, input, &opts) };
            match result {
                Ok(_) => assert!(!refused, "{patterns:?}"),
                Err(e) => {
                    assert!(refused, "{patterns:?}: {e}");
                    assert_eq!((e.raw_code(), e.class()), (-1, Some(ErrorClass::Invalid)));
                    assert_eq!(
                        e.message(),
                        c"invalid pathspec: an empty pattern beside a non-empty one"
                    );
                }
            }
        }
    }
}
