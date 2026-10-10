//! Equivalence of `git2/annotated_commit.h`'s [`AnnotatedCommit`] constructors with
//! libgit2's.
//!
//! [`AnnotatedCommit::lookup`], [`from_ref`](AnnotatedCommit::from_ref),
//! [`from_revspec`](AnnotatedCommit::from_revspec) and
//! [`from_fetchhead`](AnnotatedCommit::from_fetchhead) must build what
//! `git_annotated_commit_lookup`, `_from_ref`, `_from_revspec` and `_from_fetchhead`
//! build, and fail where they fail. Each side runs the same sequence of steps over a
//! repository of its own, built identically (so every object id agrees): two commits,
//! a tree, a blob, an annotated tag and references to each; once with SHA1 object ids
//! and once with SHA256. After every step the
//! result is recorded, as the commit id (`git_annotated_commit_id`) and reference name
//! (`git_annotated_commit_ref`) of what was built or the error class and message the
//! call reported, together with the thread's last libgit2 error; every annotated
//! commit built so far is read again, so a later step cannot disturb an earlier
//! result. Each side's references are freed before what was built from them is read.
//!
//! Documented divergences, outside these comparisons:
//!
//! - The Rust constructors return libgit2's error as an [`ErrorBuf`], without the
//!   return code (`GIT_ENOTFOUND` and `-1` read the same); the class and message are
//!   compared.
//! - libgit2 leaves the built commit in `out` when copying a reference name or remote
//!   URL runs out of memory while returning `-1`; the Rust side frees it. Only an
//!   allocation failure reaches it, which `tests/ub_unsafe/annotated_commit.rs`
//!   exercises against the Rust side alone (the C side leaks).
//! - The Rust constructors refuse an object id whose type is not the repository's
//!   before the call, where libgit2 formats it into an object path sized for the
//!   repository's type: a SHA256 id overflows a SHA1 repository's buffer (an ASan
//!   heap-buffer-overflow in `git_oid_pathfmt`), a SHA1 id leaves the tail of a SHA256
//!   repository's path unwritten, and an unknown type leaves all of it unwritten. The
//!   C side of those steps is not run; the record it is compared with is the
//!   `GIT_ERROR_INVALID` error libgit2 sets for a mismatched object id type elsewhere
//!   (`diff_generate.c`), which the Rust side must report and leave.
//! - An annotated commit cannot outlive its repository in Rust (its owner borrows the
//!   repository), where libgit2 frees one safely afterwards; only C does that here.
//! - Error messages naming the repository's path are compared with the path replaced,
//!   since each side has its own.

use std::ffi::{CStr, CString, c_int};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::refs::annotated_commit::{AnnotatedCommit, AnnotatedCommitBox};
use libgit2::libgit2::refs::refs::ReferenceBox;
use libgit2::libgit2::repository::repository::{
    InitFlag, InitOptions, Repository, RepositoryBox, RepositoryRef,
};
use libgit2::libgit2::util::errors::ErrorBuf;

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

/// The hex form of `id`, as `git_oid_tostr` writes it.
fn hex(id: *const ffi::git_oid) -> String {
    let mut buf = [0 as std::ffi::c_char; 65];
    // SAFETY: `buf` holds 65 bytes, which bounds the write; `id` is a live oid.
    unsafe { ffi::git_oid_tostr(buf.as_mut_ptr(), buf.len(), id) };
    // SAFETY: `git_oid_tostr` NUL-terminates within `buf`.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The objects every probe repository holds, by id.
#[derive(Clone, Copy)]
struct Ids {
    tree: ffi::git_oid,
    blob: ffi::git_oid,
    first: ffi::git_oid,
    second: ffi::git_oid,
    tag: ffi::git_oid,
}

/// A fresh bare repository whose `main` (and `HEAD`) is a second commit over a first,
/// `feature` the first, `refs/tags/v1` an annotated tag of the second, and
/// `refs/tags/tree` / `refs/tags/blob` a tree and a blob; freed and removed on drop.
///
/// Both sides hold their repository as a [`RepositoryBox`]; the C side calls libgit2
/// on its raw pointer.
struct Repo {
    path: PathBuf,
    repo: Option<RepositoryBox>,
    ids: Ids,
}

impl Repo {
    /// A repository of object id type `kind`.
    fn new(tag: &str, kind: ffi::git_oid_t) -> Self {
        init();
        let path = std::env::temp_dir().join(format!(
            "crustify-annotated-commit-equiv-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = InitOptions::new();
        let mut m = opts.as_mut();
        m.set_flags(InitFlag::Bare | InitFlag::Mkdir);
        m.set_initial_head(Some(c"main"));
        m.set_oid_type(Some(OidType::from_raw(kind).unwrap()));
        let repo = Repository::init_ext(&c_path, opts.as_mut()).unwrap();
        let (mut odb, raw) = (ptr::null_mut(), repo.as_c_ptr());
        // SAFETY: `odb` is a live slot and `raw` a live repository; the odb is a counted
        // reference, released below.
        assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, raw) }, 0);
        let write = |data: &[u8], kind: ffi::git_object_t| {
            let mut id = ffi::git_oid::default();
            let rc =
                // SAFETY: `id` is a live slot, `odb` a live odb and `data` readable for its
                // length.
                unsafe { ffi::git_odb_write(&mut id, odb, data.as_ptr().cast(), data.len(), kind) };
            assert_eq!(rc, 0, "{:?}", c_last());
            id
        };
        const WHO: &str = "A U Thor <author@example.com> 1700000000 +0130";
        let tree = write(b"", ffi::GIT_OBJECT_TREE);
        let blob = write(b"hello\n", ffi::GIT_OBJECT_BLOB);
        let first = write(
            format!(
                "tree {}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n",
                hex(&tree)
            )
            .as_bytes(),
            ffi::GIT_OBJECT_COMMIT,
        );
        let second = write(
            format!(
                "tree {}\nparent {}\nauthor {WHO}\ncommitter {WHO}\n\nsecond\n",
                hex(&tree),
                hex(&first)
            )
            .as_bytes(),
            ffi::GIT_OBJECT_COMMIT,
        );
        let tag = write(
            format!(
                "object {}\ntype commit\ntag v1\ntagger {WHO}\n\nv1\n",
                hex(&second)
            )
            .as_bytes(),
            ffi::GIT_OBJECT_TAG,
        );
        // SAFETY: the counted reference taken above, released once.
        unsafe { ffi::git_odb_free(odb) };
        let this = Self {
            path,
            repo: Some(repo),
            ids: Ids {
                tree,
                blob,
                first,
                second,
                tag,
            },
        };
        for (name, id) in [
            (c"refs/heads/main", &second),
            (c"refs/heads/feature", &first),
            (c"refs/tags/v1", &tag),
            (c"refs/tags/tree", &tree),
            (c"refs/tags/blob", &blob),
        ] {
            this.write_ref(name, id);
        }
        this
    }

    /// Points the loose reference `name` at `id` by writing its file, as
    /// `git_reference_create` would without a reflog signature: that takes the current
    /// time, whose glibc timezone lookup ThreadSanitizer reports against other tests'.
    fn write_ref(&self, name: &CStr, id: &ffi::git_oid) {
        let file = self.path.join(name.to_str().unwrap());
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("{}\n", hex(id))).unwrap();
    }

    fn kind(&self) -> u8 {
        self.ids.first.type_
    }

    /// The repository's handle, for the Rust side.
    fn handle(&self) -> RepositoryRef<'_> {
        self.repo.as_ref().unwrap().as_ref()
    }

    /// The repository's raw pointer, for the C side.
    fn raw(&self) -> *mut ffi::git_repository {
        self.repo.as_ref().unwrap().as_c_ptr()
    }

    /// The reference `name`, which the caller frees.
    fn reference(&self, name: &CStr) -> *mut ffi::git_reference {
        let mut reference = ptr::null_mut();
        // SAFETY: a live slot, the live repository and a NUL-terminated name.
        let rc = unsafe { ffi::git_reference_lookup(&mut reference, self.raw(), name.as_ptr()) };
        assert_eq!(rc, 0, "{:?}", c_last());
        reference
    }

    /// `message` with this repository's path replaced, so both sides read the same.
    fn normalise(&self, message: String) -> String {
        message.replace(self.path.to_str().unwrap(), "<repo>")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        drop(self.repo.take());
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// What one annotated commit reads as: its commit id and its reference name.
#[derive(Debug, PartialEq)]
struct Read {
    id: String,
    ref_name: Option<String>,
}

/// Reads the annotated commit at `commit` through libgit2's accessors.
fn read(commit: *const ffi::git_annotated_commit) -> Read {
    // SAFETY: `commit` is a live annotated commit, only read; the id and name point
    // into it and are copied before it can be freed.
    unsafe {
        let ref_name = ffi::git_annotated_commit_ref(commit);
        Read {
            id: hex(ffi::git_annotated_commit_id(commit)),
            ref_name: (!ref_name.is_null())
                .then(|| CStr::from_ptr(ref_name).to_string_lossy().into_owned()),
        }
    }
}

/// One step's record: what was built, or the error reported; then the thread's last
/// error.
#[derive(Debug, PartialEq)]
struct Outcome {
    result: Result<Read, (c_int, String)>,
    last: (c_int, String),
}

/// One constructor call.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// `lookup` of an id given as its type byte and its 32 id bytes.
    Lookup(u8, [u8; 32]),
    FromRef(&'static CStr),
    FromRevspec(&'static CStr),
    FromFetchhead(&'static CStr, &'static CStr, u8, [u8; 32]),
}

fn raw(id: &ffi::git_oid) -> (u8, [u8; 32]) {
    (id.type_, id.id)
}

fn c_oid((kind, id): (u8, [u8; 32])) -> ffi::git_oid {
    ffi::git_oid { type_: kind, id }
}

fn rust_oid((kind, id): (u8, [u8; 32])) -> Oid {
    let mut oid = Oid::zeroed();
    oid.as_mut().set_raw_kind(kind);
    oid.as_mut().set_id(id);
    oid
}

/// The steps both sides run, in order.
fn steps(ids: &Ids) -> Vec<Step> {
    let lookup = |(kind, id)| Step::Lookup(kind, id);
    let (own, first) = raw(&ids.first);
    let other = (ffi::GIT_OID_SHA1 + ffi::GIT_OID_SHA256) as u8 - own;
    let fetch = |b, u, (kind, id)| Step::FromFetchhead(b, u, kind, id);
    vec![
        lookup(raw(&ids.second)),
        lookup(raw(&ids.first)),
        lookup(raw(&ids.second)),
        lookup(raw(&ids.tree)),
        lookup(raw(&ids.blob)),
        lookup(raw(&ids.tag)),
        lookup((own, [0x11; 32])),
        lookup((own, [0; 32])),
        lookup((other, first)),
        lookup((0, first)),
        lookup((0xff, first)),
        Step::FromRef(c"HEAD"),
        Step::FromRef(c"refs/heads/main"),
        Step::FromRef(c"refs/heads/feature"),
        Step::FromRef(c"refs/tags/v1"),
        Step::FromRef(c"refs/tags/tree"),
        Step::FromRef(c"refs/tags/blob"),
        Step::FromRevspec(c"HEAD"),
        Step::FromRevspec(c"main"),
        Step::FromRevspec(c"feature"),
        Step::FromRevspec(c"v1"),
        Step::FromRevspec(c"refs/tags/v1"),
        Step::FromRevspec(c"main~1"),
        Step::FromRevspec(c"main^"),
        Step::FromRevspec(c"main~2"),
        Step::FromRevspec(c"v1^{commit}"),
        Step::FromRevspec(c"HEAD^{tree}"),
        Step::FromRevspec(c"tree"),
        Step::FromRevspec(c"blob"),
        Step::FromRevspec(c":/first"),
        Step::FromRevspec(c"nope"),
        Step::FromRevspec(c""),
        Step::FromRevspec(c"main..feature"),
        Step::FromRevspec(c"@{-1}"),
        fetch(c"main", c"https://example.invalid/r.git", raw(&ids.second)),
        fetch(c"refs/heads/feature", c"/srv/r", raw(&ids.first)),
        fetch(c"", c"", raw(&ids.first)),
        fetch(c"main", c"https://example.invalid/r.git", raw(&ids.tree)),
        fetch(c"main", c"https://example.invalid/r.git", (own, [0x11; 32])),
        fetch(c"main", c"https://example.invalid/r.git", (0, first)),
        fetch(c"main", c"https://example.invalid/r.git", (other, first)),
        Step::FromRef(c"HEAD"),
        lookup(raw(&ids.first)),
    ]
}

/// The error both sides record for an id of another type than the repository's.
fn refused() -> (c_int, String) {
    (
        ffi::GIT_ERROR_INVALID as c_int,
        "specified object ID type does not match repository object ID type".into(),
    )
}

/// Runs `step` through libgit2 on `repo`, keeping what it builds in `held`; an id of
/// another type than the repository's is not handed to C (see the module docs).
fn c_step(repo: &Repo, step: Step, held: &mut Vec<*mut ffi::git_annotated_commit>) -> Outcome {
    if let Step::Lookup(kind, _) | Step::FromFetchhead(_, _, kind, _) = step
        && kind != repo.kind()
    {
        return Outcome {
            result: Err(refused()),
            last: refused(),
        };
    }
    let raw = repo.raw();
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a live slot, `raw` a live repository used by this thread alone,
    // every id a live oid and every string NUL-terminated, all read for the call; a
    // reference is freed after the call, before anything built from it is read.
    let rc = unsafe {
        match step {
            Step::Lookup(kind, id) => {
                ffi::git_annotated_commit_lookup(&mut out, raw, &c_oid((kind, id)))
            }
            Step::FromRef(name) => {
                let reference = repo.reference(name);
                let rc = ffi::git_annotated_commit_from_ref(&mut out, raw, reference);
                ffi::git_reference_free(reference);
                rc
            }
            Step::FromRevspec(spec) => {
                ffi::git_annotated_commit_from_revspec(&mut out, raw, spec.as_ptr())
            }
            Step::FromFetchhead(branch, url, kind, id) => ffi::git_annotated_commit_from_fetchhead(
                &mut out,
                raw,
                branch.as_ptr(),
                url.as_ptr(),
                &c_oid((kind, id)),
            ),
        }
    };
    let last = c_last();
    let result = if rc < 0 {
        assert!(out.is_null(), "{step:?} left a commit behind a failure");
        Err((last.0, repo.normalise(last.1.clone())))
    } else {
        held.push(out);
        Ok(read(out))
    };
    Outcome {
        result,
        last: (last.0, repo.normalise(last.1)),
    }
}

/// Runs `step` through the Rust API on `repo`, keeping what it builds in `held`.
fn rust_step<'r>(repo: &'r Repo, step: Step, held: &mut Vec<AnnotatedCommitBox<'r>>) -> Outcome {
    let built: Result<AnnotatedCommitBox<'r>, ErrorBuf> = match step {
        Step::Lookup(kind, id) => {
            AnnotatedCommit::lookup(repo.handle(), rust_oid((kind, id)).as_ref())
        }
        Step::FromRef(name) => {
            // SAFETY: a fresh attached reference of `repo` this side owns, released by
            // `git_reference_free`; `repo` outlives it.
            let reference = unsafe { ReferenceBox::from_c(repo.reference(name)) }.unwrap();
            let built = AnnotatedCommit::from_ref(repo.handle(), reference.as_ref());
            // The result outlives the reference.
            drop(reference);
            built
        }
        Step::FromRevspec(spec) => AnnotatedCommit::from_revspec(repo.handle(), spec),
        Step::FromFetchhead(branch, url, kind, id) => AnnotatedCommit::from_fetchhead(
            repo.handle(),
            branch,
            url,
            rust_oid((kind, id)).as_ref(),
        ),
    };
    let last = c_last();
    let result = match built {
        Ok(commit) => {
            let read = read(commit.as_c_ptr());
            held.push(commit);
            Ok(read)
        }
        Err(e) => Err((
            e.raw_class(),
            repo.normalise(e.message().to_string_lossy().into_owned()),
        )),
    };
    Outcome {
        result,
        last: (last.0, repo.normalise(last.1)),
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_build_and_fail_as_c() {
    constructors_build_and_fail_as_c_with(ffi::GIT_OID_SHA1);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn constructors_build_and_fail_as_c_over_sha256() {
    constructors_build_and_fail_as_c_with(ffi::GIT_OID_SHA256);
}

fn constructors_build_and_fail_as_c_with(kind: ffi::git_oid_t) {
    let c_repo = Repo::new(&format!("c-{kind}"), kind);
    let rust_repo = Repo::new(&format!("rust-{kind}"), kind);
    assert_eq!(c_repo.kind(), kind as u8);
    let mut c_held = Vec::new();
    let mut rust_held = Vec::new();
    for (n, step) in steps(&c_repo.ids).into_iter().enumerate() {
        let c = c_step(&c_repo, step, &mut c_held);
        let rust = rust_step(&rust_repo, step, &mut rust_held);
        assert_eq!(rust, c, "step {n}: {step:?}");
        let c_reads: Vec<Read> = c_held.iter().map(|&p| read(p)).collect();
        let rust_reads: Vec<Read> = rust_held.iter().map(|b| read(b.as_c_ptr())).collect();
        assert_eq!(rust_reads, c_reads, "after step {n}: {step:?}");
    }
    // 21 of the 43 steps build (every constructor does); the rest fail.
    assert_eq!((c_held.len(), rust_held.len()), (21, 21));
    // Release in a different order than built, the repositories still open.
    for p in c_held.drain(..).rev() {
        // SAFETY: each a live annotated commit the C side owns, freed once.
        unsafe { ffi::git_annotated_commit_free(p) };
    }
    rust_held.reverse();
    drop(rust_held);
}
