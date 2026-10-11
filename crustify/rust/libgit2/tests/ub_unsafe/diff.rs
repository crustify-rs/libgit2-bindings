#[path = "../support/diff.rs"]
mod support;
use libgit2::ffi;
use libgit2::libgit2::diff::diff::{Diff, DiffInputs};
use libgit2::libgit2::index::index::Index;
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
