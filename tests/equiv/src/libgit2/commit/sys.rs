//! Equivalence of `git2/sys/commit.h`'s [`create_from_ids`] and
//! [`create_from_callback`] with `git_commit_create_from_ids` and
//! `git_commit_create_from_callback`.
//!
//! Each side writes into a fresh bare repository of its own, one script of steps in
//! order: root commits and children by id, through a slice of ids and through a parent
//! callback (C: one over a static array; Rust: a [`ParentCallback`] closure), a
//! reference moved from its first parent and refused from another, and the parents and
//! trees strict object creation refuses (missing, or of another object type). After
//! every step the sides compare the outcome (the new id, or the code, class and message
//! of the failure) and where `HEAD` points. A second script runs both routines on
//! repositories whose object database cannot be written once their root commit is
//! (every missing loose fan-out directory a regular file), where libgit2 fails the
//! write. Reference writes reach glibc's timezone state, which another test's would
//! race (`TSan`), so each script runs in an isolated child.
//!
//! Documented divergence, outside these comparisons: the wrappers refuse an id of
//! another type than the repository's, which libgit2 would format out of bounds, and a
//! signature it cannot write; and they ask a parent callback for every parent before
//! libgit2 runs, so a closure is asked for all of them even when libgit2 fails before
//! its first. The unit tests of `object/sys/commit.rs` hold those.

use std::ffi::{CStr, CString, c_int, c_void};
use std::path::PathBuf;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::commit::{
    ParentCallback, create_from_callback, create_from_ids,
};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryError};
use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const MISSING: &str = "dededededededededededededededededededede";

/// One step: through the callback (or the slice), the reference to move, the tree,
/// and the parents, each an index of an earlier commit or a fixed id.
#[derive(Clone, Copy, Debug)]
enum Id {
    Commit(usize),
    Fixed(&'static str),
}

type Step = (
    bool,
    Option<&'static CStr>,
    Id,
    &'static [Id],
    &'static CStr,
);

fn script() -> Vec<Step> {
    use Id::*;
    let head = Some(c"HEAD");
    vec![
        (false, head, Fixed(EMPTY_TREE), &[], c"root\n"),
        (true, head, Fixed(EMPTY_TREE), &[Commit(0)], c"child\n"),
        (false, head, Fixed(EMPTY_TREE), &[Commit(0)], c"stale\n"),
        (
            true,
            None,
            Fixed(EMPTY_TREE),
            &[Commit(1), Commit(0), Commit(1)],
            c"merge\n",
        ),
        (
            false,
            None,
            Fixed(EMPTY_TREE),
            &[Fixed(MISSING)],
            c"missing parent\n",
        ),
        (true, None, Fixed(MISSING), &[], c"missing tree\n"),
        (false, None, Commit(0), &[], c"commit as tree\n"),
        (
            true,
            None,
            Fixed(EMPTY_TREE),
            &[Fixed(EMPTY_TREE)],
            c"tree as parent\n",
        ),
        (
            true,
            head,
            Fixed(EMPTY_TREE),
            &[Commit(1)],
            c"from the tip\n",
        ),
    ]
}

type Outcome = Result<String, (c_int, c_int, String)>;

fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

fn scratch(side: &str) -> (PathBuf, CString) {
    init();
    let dir = std::env::temp_dir().join(format!(
        "crustify-equiv-sys-commit-{}-{side}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&dir));
    let path = CString::new(dir.to_str().unwrap()).unwrap();
    (dir, path)
}

fn head(dir: &std::path::Path) -> Option<String> {
    let target = std::fs::read_to_string(dir.join("HEAD")).unwrap();
    let branch = target.trim().strip_prefix("ref: ").unwrap().to_owned();
    std::fs::read_to_string(dir.join(branch))
        .ok()
        .map(|t| t.trim().to_owned())
}

fn hex(id: &Id, written: &[String]) -> String {
    match id {
        Id::Commit(i) => written[*i].clone(),
        Id::Fixed(h) => (*h).to_owned(),
    }
}

/// The parents a C callback hands out, with the count.
struct Parents(Vec<ffi::git_oid>);

unsafe extern "C" fn c_parent(idx: usize, payload: *mut c_void) -> *const ffi::git_oid {
    // SAFETY: the payload is the live `Parents` of the call, only read.
    let parents = unsafe { &*payload.cast::<Parents>() };
    parents.0.get(idx).map_or(ptr::null(), ptr::from_ref)
}

fn c_oid(hex: &str) -> ffi::git_oid {
    let mut id = ffi::git_oid::default();
    let h = CString::new(hex).unwrap();
    // SAFETY: a writable id and 40 hex digits.
    let rc = unsafe { ffi::git_oid_from_string(&raw mut id, h.as_ptr(), ffi::GIT_OID_SHA1) };
    assert_eq!(rc, 0);
    id
}

fn c_run(repo: *mut ffi::git_repository, written: &mut Vec<String>, step: &Step) -> Outcome {
    let (by_callback, update_ref, tree, parents, message) = *step;
    let sig = ffi::git_signature {
        name: c"A U Thor".as_ptr().cast_mut(),
        email: c"a@example.com".as_ptr().cast_mut(),
        when: ffi::git_time {
            time: 1_700_000_000,
            offset: 90,
            sign: b'+'.cast_signed(),
        },
    };
    let tree = c_oid(&hex(&tree, written));
    let ids: Vec<ffi::git_oid> = parents.iter().map(|p| c_oid(&hex(p, written))).collect();
    let mut out = ffi::git_oid::default();
    let update_ref = update_ref.map_or(ptr::null(), CStr::as_ptr);
    let rc = if by_callback {
        let mut payload = Parents(ids);
        // SAFETY: a writable id, the live repository, a signature over static strings,
        // NULL or NUL-terminated strings, and a callback over the live payload.
        unsafe {
            ffi::git_commit_create_from_callback(
                &raw mut out,
                repo,
                update_ref,
                &raw const sig,
                &raw const sig,
                ptr::null(),
                message.as_ptr(),
                &raw const tree,
                Some(c_parent),
                ptr::from_mut(&mut payload).cast(),
            )
        }
    } else {
        let mut ptrs: Vec<*const ffi::git_oid> = ids.iter().map(ptr::from_ref).collect();
        // SAFETY: as above, with `ptrs.len()` live ids.
        unsafe {
            ffi::git_commit_create_from_ids(
                &raw mut out,
                repo,
                update_ref,
                &raw const sig,
                &raw const sig,
                ptr::null(),
                message.as_ptr(),
                &raw const tree,
                ptrs.len(),
                ptrs.as_mut_ptr(),
            )
        }
    };
    if rc < 0 {
        let (class, msg) = {
            // SAFETY: after init `git_error_last` never returns NULL; its message is copied
            // at once.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let klass = unsafe { (*e).klass };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            unsafe {
                (
                    klass,
                    CStr::from_ptr(message2).to_string_lossy().into_owned(),
                )
            }
        };
        return Err((rc, class, msg));
    }
    let mut buf = [0u8; 41];
    // SAFETY: the id just written; 41 writable bytes, NUL included.
    unsafe { ffi::git_oid_tostr(buf.as_mut_ptr().cast(), buf.len(), &raw const out) };
    let h = CStr::from_bytes_until_nul(&buf)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    written.push(h.clone());
    Ok(h)
}

fn rust_run(
    repo: &libgit2::libgit2::src::libgit2::repository::RepositoryBox,
    written: &mut Vec<String>,
    step: &Step,
) -> Outcome {
    let (by_callback, update_ref, tree, parents, message) = *step;
    let sig = SignatureView::new(c"A U Thor", c"a@example.com", Time::new(1_700_000_000, 90));
    let oid = |h: String| Oid::from_string(&CString::new(h).unwrap(), OidType::Sha1).unwrap();
    let tree = oid(hex(&tree, written));
    let ids: Vec<Oid> = parents.iter().map(|p| oid(hex(p, written))).collect();
    let result = if by_callback {
        let mut next = |i: usize| ids.get(i).copied();
        let mut callback = ParentCallback::new(&mut next);
        create_from_callback(
            repo.as_ref(),
            update_ref,
            sig.as_ref(),
            sig.as_ref(),
            None,
            message,
            tree.as_ref(),
            &mut callback,
        )
    } else {
        let refs: Vec<_> = ids.iter().map(Oid::as_ref).collect();
        create_from_ids(
            repo.as_ref(),
            update_ref,
            sig.as_ref(),
            sig.as_ref(),
            None,
            message,
            tree.as_ref(),
            &refs,
        )
    };
    match result {
        Ok(id) => {
            let h = id.as_ref().to_hex_string().unwrap();
            written.push(h.clone());
            Ok(h)
        }
        Err(e) => Err(failure(e)),
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper consumes what it reports, as its callers hand it over"
)]
fn failure(e: RepositoryError) -> (c_int, c_int, String) {
    (
        e.raw_code(),
        e.raw_class(),
        e.message().to_string_lossy().into_owned(),
    )
}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process.
fn isolated(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn creation_by_id_scripts_match() {
    isolated(
        concat!(module_path!(), "::creation_by_id_scripts_match"),
        creation_by_id_scripts_match_body,
    );
}

fn creation_by_id_scripts_match_body() {
    let (c_dir, c_path) = scratch("c");
    let mut c_repo = ptr::null_mut();
    // SAFETY: a writable slot and a NUL-terminated path.
    let rc = unsafe { ffi::git_repository_init(&raw mut c_repo, c_path.as_ptr(), 1) };
    assert_eq!(rc, 0);
    let (dir, path) = scratch("rust");
    let rust = Repository::init(&path, true).unwrap();
    let (mut c_written, mut written) = (Vec::new(), Vec::new());
    let mut failures = 0;
    for (n, step) in script().iter().enumerate() {
        let expected = c_run(c_repo, &mut c_written, step);
        let got = rust_run(&rust, &mut written, step);
        assert_eq!(got, expected, "step {n}: {step:?}");
        assert_eq!(head(&dir), head(&c_dir), "step {n}");
        failures += usize::from(expected.is_err());
    }
    // The stale parent, the missing parent and tree, and the objects of the wrong type.
    assert_eq!(failures, 5);
    drop(rust);
    // SAFETY: the C side's repository, released once.
    unsafe { ffi::git_repository_free(c_repo) };
    drop(std::fs::remove_dir_all(dir));
    drop(std::fs::remove_dir_all(c_dir));
}

/// Both routines, on repositories whose object database cannot be written once the root
/// commit is: every loose fan-out directory the repository lacks is a regular file, so
/// `git_odb_write` fails after the parents were validated, before `HEAD` moves. The
/// sides compare the code, class and message (their directory masked) and `HEAD`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn unwritable_object_databases_fail_alike() {
    isolated(
        concat!(module_path!(), "::unwritable_object_databases_fail_alike"),
        unwritable_object_databases_fail_alike_body,
    );
}

fn unwritable_object_databases_fail_alike_body() {
    use Id::*;
    let update = Some(c"HEAD");
    let root: Step = (false, update, Fixed(EMPTY_TREE), &[], c"root\n");
    let steps: [Step; 3] = [
        (true, update, Fixed(EMPTY_TREE), &[Commit(0)], c"child\n"),
        (
            false,
            update,
            Fixed(EMPTY_TREE),
            &[Commit(0)],
            c"child by id\n",
        ),
        (true, None, Fixed(EMPTY_TREE), &[], c"another root\n"),
    ];
    let block = |dir: &std::path::Path| {
        for byte in 0..=u8::MAX {
            let fan_out = dir.join(format!("objects/{byte:02x}"));
            if !fan_out.exists() {
                std::fs::write(fan_out, "").unwrap();
            }
        }
    };
    let masked = |outcome: Outcome, dir: &std::path::Path| {
        let dir = dir.to_str().unwrap();
        outcome.map_err(|(rc, class, message)| (rc, class, message.replace(dir, "<repo>")))
    };
    let (c_dir, c_path) = scratch("c");
    let mut c_repo = ptr::null_mut();
    // SAFETY: a writable slot and a NUL-terminated path.
    let rc = unsafe { ffi::git_repository_init(&raw mut c_repo, c_path.as_ptr(), 1) };
    assert_eq!(rc, 0);
    let (dir, path) = scratch("rust");
    let rust = Repository::init(&path, true).unwrap();
    let (mut c_written, mut written) = (Vec::new(), Vec::new());
    let expected = c_run(c_repo, &mut c_written, &root);
    assert_eq!(rust_run(&rust, &mut written, &root), expected);
    assert!(expected.is_ok());
    block(&c_dir);
    block(&dir);
    for (n, step) in steps.iter().enumerate() {
        let expected = masked(c_run(c_repo, &mut c_written, step), &c_dir);
        let got = masked(rust_run(&rust, &mut written, step), &dir);
        assert_eq!(got, expected, "step {n}: {step:?}");
        assert!(expected.is_err(), "step {n}: {step:?}");
        assert_eq!(head(&dir), head(&c_dir), "step {n}");
    }
    drop(rust);
    // SAFETY: the C side's repository, released once.
    unsafe { ffi::git_repository_free(c_repo) };
    drop(std::fs::remove_dir_all(dir));
    drop(std::fs::remove_dir_all(c_dir));
}
