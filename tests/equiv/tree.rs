//! Equivalence of `git2/tree.h`'s [`WalkMode`] with the `git_treewalk_mode` values
//! libgit2 declares and `git_tree_walk` accepts.
//!
//! [`WalkMode`] must name exactly the values `git_tree_walk` walks with, each with its
//! C value, and every other value must be one `git_tree_walk` refuses before calling
//! back. Each variant must also order a walk as it documents: pre-order calls back
//! for an entry before its subtree and lets a positive result skip the subtree;
//! post-order calls back after the subtree and ignores a positive result. `expected`
//! below is that documentation, compared with `git_tree_walk` over one tree of nested
//! subtrees, step after step.
//!
//! The tree object's lifecycle is compared in [`tree_object`], the tree builder's,
//! `UpdateAction` and `TreeUpdate` in [`builder`], the tree entry's lifecycle in
//! [`tree_entry`].

use std::collections::HashSet;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::{fs, ptr};

use libgit2::ffi;
use libgit2::libgit2::object::tree::{UnnamedWalkMode, WalkMode};

/// Every `git_treewalk_mode` constant, beside the variant that must carry it.
const MODES: [(ffi::git_treewalk_mode, WalkMode); 2] = [
    (ffi::GIT_TREEWALK_PRE, WalkMode::PreOrder),
    (ffi::GIT_TREEWALK_POST, WalkMode::PostOrder),
];

/// Every value around the declared ones, a stride across the rest, and the extremes.
fn probes() -> impl Iterator<Item = ffi::git_treewalk_mode> {
    (0..=64)
        .chain((0..=ffi::git_treewalk_mode::MAX).step_by(1 << 28))
        .chain([ffi::git_treewalk_mode::MAX - 1, ffi::git_treewalk_mode::MAX])
}

/// One entry of the walked tree: a blob, or a subtree with its entries, in the order
/// libgit2 sorts them.
enum Node {
    Blob(&'static str),
    Tree(&'static str, &'static [Node]),
}

impl Node {
    fn name(&self) -> &'static str {
        match self {
            Node::Blob(name) | Node::Tree(name, _) => name,
        }
    }
}

/// The walked tree: subtrees at two depths, between blobs on either side.
const TREE: &[Node] = &[
    Node::Blob("a.txt"),
    Node::Tree(
        "dir",
        &[
            Node::Blob("b.txt"),
            Node::Blob("d.txt"),
            Node::Tree("sub", &[Node::Blob("c.txt")]),
        ],
    ),
    Node::Tree("nest", &[Node::Tree("leaf", &[Node::Blob("e.txt")])]),
    Node::Blob("z.txt"),
];

/// A callback's result for the entry `name` under `root`.
type Reply = fn(root: &str, name: &str) -> c_int;

/// One call back: the entry's root and name.
type Visit = (String, String);

/// What a walk returned, called back for, and left as the thread's error.
type Walked = (c_int, Vec<Visit>, (c_int, String));

/// The walk `mode` documents over `nodes` under `root`, appending its visits to `out`;
/// its return, as `git_tree_walk` should return it.
fn expected(
    mode: WalkMode,
    nodes: &[Node],
    root: &str,
    reply: Reply,
    out: &mut Vec<Visit>,
) -> c_int {
    let pre = match mode {
        WalkMode::PreOrder => true,
        WalkMode::PostOrder => false,
        _ => unreachable!("a mode this suite does not know"),
    };
    for node in nodes {
        let name = node.name();
        if pre {
            out.push((root.to_owned(), name.to_owned()));
            match reply(root, name) {
                r if r < 0 => return r,
                r if r > 0 => continue,
                _ => {}
            }
        }
        if let Node::Tree(_, children) = node {
            let rc = expected(mode, children, &format!("{root}{name}/"), reply, out);
            if rc != 0 {
                return rc;
            }
        }
        if !pre {
            out.push((root.to_owned(), name.to_owned()));
            let r = reply(root, name);
            if r < 0 {
                return r;
            }
        }
    }
    0
}

/// [`expected`] from the top of the tree, with the error a stop leaves.
fn model(mode: WalkMode, reply: Reply) -> Walked {
    let mut visits = Vec::new();
    let rc = expected(mode, TREE, "", reply, &mut visits);
    let error = if rc < 0 {
        (
            ffi::GIT_ERROR_CALLBACK.cast_signed(),
            format!("git_tree_walk callback returned {rc}"),
        )
    } else {
        (ffi::GIT_ERROR_NONE.cast_signed(), "no error".to_owned())
    };
    (rc, visits, error)
}

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error class and message.
fn last_error() -> (c_int, String) {
    {
        // SAFETY: the runtime is initialised; `git_error_last` never returns NULL then,
        // and its message is a NUL-terminated string live until the next libgit2 call on
        // this thread, after the copy.
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
    }
}

/// A bare repository of its own under the temp dir holding [`TREE`]; closed and
/// removed when dropped.
struct TreeRepo {
    path: PathBuf,
    repo: *mut ffi::git_repository,
    tree: *mut ffi::git_tree,
}

impl TreeRepo {
    fn new(tag: &str) -> Self {
        init();
        let path =
            std::env::temp_dir().join(format!("crustify-tree-equiv-{}-{tag}", std::process::id()));
        drop(fs::remove_dir_all(&path));
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot and `c_path` NUL-terminated; `1` asks for a
        // bare repository.
        let rc = unsafe { ffi::git_repository_init(&raw mut repo, c_path.as_ptr(), 1) };
        assert_eq!(rc, 0);
        let id = write_tree(repo, TREE);
        let mut tree = ptr::null_mut();
        // SAFETY: `tree` is a live slot, `repo` a live repository and `id` a live oid.
        let rc = unsafe { ffi::git_tree_lookup(&raw mut tree, repo, &raw const id) };
        assert_eq!(rc, 0);
        Self { path, repo, tree }
    }

    /// `git_tree_walk(tree, raw, ...)`, with each call back recorded and answered by
    /// `reply`.
    fn walk(&self, raw: ffi::git_treewalk_mode, reply: Reply) -> Walked {
        let mut walk = Walk {
            visits: Vec::new(),
            reply,
        };
        let payload = (&raw mut walk).cast();
        // SAFETY: the runtime is initialised.
        unsafe { ffi::git_error_clear() };
        // SAFETY: `self.tree` is a live tree; `record` reads `payload` as the `Walk`
        // above, which outlives the call and is reached only through it meanwhile.
        let rc = unsafe { ffi::git_tree_walk(self.tree, raw, Some(record), payload) };
        (rc, walk.visits, last_error())
    }
}

impl Drop for TreeRepo {
    fn drop(&mut self) {
        // SAFETY: the tree and repository are live and released once, the tree first.
        unsafe { ffi::git_tree_free(self.tree) };
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(self.repo) };
        drop(fs::remove_dir_all(&self.path));
    }
}

/// Writes `nodes` to `repo` as a tree, its subtrees first; its id.
fn write_tree(repo: *mut ffi::git_repository, nodes: &[Node]) -> ffi::git_oid {
    let mut builder = ptr::null_mut();
    // SAFETY: `builder` is a live slot and `repo` a live repository; no source tree.
    let rc = unsafe { ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()) };
    assert_eq!(rc, 0);
    for node in nodes {
        let (id, mode) = match node {
            Node::Blob(name) => {
                let mut id = ffi::git_oid::default();
                // SAFETY: `id` is a writable oid and `name` readable for its length.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &raw mut id,
                        repo,
                        name.as_ptr().cast(),
                        name.len(),
                    )
                };
                assert_eq!(rc, 0);
                (id, ffi::GIT_FILEMODE_BLOB)
            }
            Node::Tree(_, children) => (write_tree(repo, children), ffi::GIT_FILEMODE_TREE),
        };
        let name = CString::new(node.name()).unwrap();
        // SAFETY: `builder` is live, `name` NUL-terminated and `id` a live oid, read
        // for the call only; no entry is asked back.
        let rc = unsafe {
            ffi::git_treebuilder_insert(
                ptr::null_mut(),
                builder,
                name.as_ptr(),
                &raw const id,
                mode,
            )
        };
        assert_eq!(rc, 0);
    }
    let mut id = ffi::git_oid::default();
    // SAFETY: `id` is a writable oid and `builder` live; it is freed once, after.
    unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut id, builder), 0) };
    // SAFETY: as above.
    unsafe { ffi::git_treebuilder_free(builder) };
    id
}

/// What [`record`] keeps of a walk.
struct Walk {
    visits: Vec<Visit>,
    reply: Reply,
}

/// `git_treewalk_cb`: records the entry's root and name in the `Walk` at `payload`,
/// then answers as its `reply` does.
unsafe extern "C" fn record(
    root: *const c_char,
    entry: *const ffi::git_tree_entry,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: `TreeRepo::walk` passes a live, otherwise unreached `Walk`.
    let walk = unsafe { &mut *payload.cast::<Walk>() };
    // SAFETY: libgit2 passes a NUL-terminated root, live for the call; it is copied.
    let root = unsafe { CStr::from_ptr(root) }.to_str().unwrap().to_owned();
    // SAFETY: libgit2 passes a live entry for the call.
    let name = unsafe { ffi::git_tree_entry_name(entry) };
    // SAFETY: the entry's name is NUL-terminated, live with it; it is copied.
    let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap().to_owned();
    let rc = (walk.reply)(&root, &name);
    walk.visits.push((root, name));
    rc
}

fn walk_on(_: &str, _: &str) -> c_int {
    0
}

fn skip_dir(_: &str, name: &str) -> c_int {
    i32::from(name == "dir")
}

fn skip_nested_trees(root: &str, name: &str) -> c_int {
    if root.is_empty() {
        0
    } else {
        i32::from(!name.contains('.')) * 7
    }
}

fn skip_everything(_: &str, _: &str) -> c_int {
    1
}

fn stop_at_sub(_: &str, name: &str) -> c_int {
    if name == "sub" { -123 } else { 0 }
}

fn stop_at_first(_: &str, _: &str) -> c_int {
    -1
}

fn stop_at_last(_: &str, name: &str) -> c_int {
    if name == "z.txt" { -42 } else { 0 }
}

fn stop_at_leaf_blob(root: &str, _: &str) -> c_int {
    if root == "nest/leaf/" { i32::MIN } else { 0 }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn modes_are_the_values_git_tree_walk_accepts() {
    let repo = TreeRepo::new("accepts");
    let mut walked = HashSet::new();
    for raw in probes() {
        let c = repo.walk(raw, walk_on);
        if let Some(mode) = WalkMode::from_raw(raw) {
            assert_eq!(c, model(mode, walk_on), "mode {raw}");
            assert_eq!(mode.raw(), raw);
            assert_eq!(WalkMode::try_from(raw), Ok(mode));
            walked.insert(mode);
        } else {
            let refused = (
                -1,
                Vec::new(),
                (
                    ffi::GIT_ERROR_INVALID.cast_signed(),
                    "invalid walking mode for tree walk".to_owned(),
                ),
            );
            assert_eq!(c, refused, "mode {raw}");
            assert_eq!(
                WalkMode::try_from(raw).map_err(UnnamedWalkMode::raw),
                Err(raw)
            );
        }
    }
    assert_eq!(walked.len(), MODES.len());
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn each_variant_carries_its_constant_and_order() {
    let repo = TreeRepo::new("order");
    for (raw, mode) in MODES {
        assert_eq!(mode.raw(), raw);
        assert_eq!(ffi::git_treewalk_mode::from(mode), raw);
        assert_eq!(
            repo.walk(mode.raw(), walk_on),
            model(mode, walk_on),
            "{mode:?}"
        );
    }

    // Both orders visit every entry once, under the same roots, in different orders.
    let (_, mut pre, _) = repo.walk(WalkMode::PreOrder.raw(), walk_on);
    let (_, mut post, _) = repo.walk(WalkMode::PostOrder.raw(), walk_on);
    assert_ne!(pre, post);
    assert_eq!(pre.first(), Some(&(String::new(), "a.txt".to_owned())));
    assert_eq!(pre.get(1), Some(&(String::new(), "dir".to_owned())));
    assert_eq!(post.get(1), Some(&("dir/".to_owned(), "b.txt".to_owned())));
    pre.sort();
    post.sort();
    assert_eq!(pre, post);
    assert_eq!(pre.len(), 10);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn callback_results_act_as_each_mode_documents() {
    let repo = TreeRepo::new("results");
    let replies: [(&str, Reply); 8] = [
        ("walk on", walk_on),
        ("skip dir", skip_dir),
        ("skip nested trees", skip_nested_trees),
        ("skip everything", skip_everything),
        ("stop at sub", stop_at_sub),
        ("stop at first", stop_at_first),
        ("stop at last", stop_at_last),
        ("stop at leaf blob", stop_at_leaf_blob),
    ];
    // Every reply under each mode in turn, on one tree, with a refused mode between
    // the steps, which must leave the next walk unaffected.
    for (what, reply) in replies {
        for (_, mode) in MODES.iter().chain(MODES.iter().rev()) {
            assert_eq!(
                repo.walk(mode.raw(), reply),
                model(*mode, reply),
                "{what}, {mode:?}"
            );
            let refused = repo.walk(u32::try_from(MODES.len()).unwrap(), reply);
            assert_eq!(refused.0, -1);
            assert!(refused.1.is_empty(), "{:?}", refused.1);
        }
    }
}

mod builder {
    //! Equivalence of `git2/tree.h`'s [`TreeBuilder`] lifecycle, [`UpdateAction`] and
    //! [`TreeUpdate`] with libgit2's.
    //!
    //! - Builders: [`TreeBuilder::new`] and [`TreeBuilder::from_tree`] must build what
    //!   `git_treebuilder_new` builds (with a NULL source and with a tree), and
    //!   [`clear`](libgit2::libgit2::object::tree::TreeBuilderMut::clear) must leave what
    //!   `git_treebuilder_clear` leaves. Each side runs the same steps over a bare repository of
    //!   its own, built identically, with source trees looked up in a fixture repository it opened
    //!   itself; after every step the builder's state is recorded (its entry count, the file mode
    //!   of every name either side ever held, and the id `git_treebuilder_write` gives, or the
    //!   error it reports) and compared. The C side runs `git_treebuilder_insert`, `_get`,
    //!   `_write`, `_entrycount` and `git_tree_entry_filemode_raw` raw, the Rust side their
    //!   wrappers ([`TreeBuilderRef::len`] for the count, `TreeEntryRef::filemode_raw` for an
    //!   entry's mode). The builder routines' failures and the walk callback are compared in
    //!   [`routines`].
    //!
    //!   [`TreeBuilderRef::len`]: libgit2::libgit2::object::tree::TreeBuilderRef::len
    //! - Allocation failures: libgit2's allocator is made to fail its `k`-th allocation inside the
    //!   constructor, for each `k` until the constructor succeeds; both sides must fail at the same
    //!   `k` with the same error class and message, and then build the same builder. This runs
    //!   alone in a child process, since the allocator is process-global.
    //! - Update actions: `git_tree_create_updated` accepts exactly the values
    //!   [`UpdateAction::from_raw`] names, and does with each what its variant says.
    //! - Updates: a [`TreeUpdate`] holds the fields C code fills in a `git_tree_update` for the
    //!   same change, and an array of them makes `git_tree_create_updated` write the same trees, or
    //!   fail with the same errors, step after step.
    //!
    //! Documented divergences, outside these comparisons:
    //!
    //! - The Rust constructors return libgit2's error as an [`ErrorBuf`], without the return code
    //!   (always `-1`); the class and message are compared.
    //! - A builder cannot outlive its repository in Rust (its owner borrows the repository), where
    //!   libgit2 frees one safely afterwards; only C does that here.
    //! - `git_treebuilder_clear` returns `0` and fails only on a NULL builder, which the Rust
    //!   handle cannot be; `clear` returns nothing.

    use std::collections::BTreeSet;
    use std::ffi::{CStr, CString, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::ptr::{self, NonNull};
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering::SeqCst;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::tree::{
        Tree, TreeArc, TreeBuilder, TreeBuilderBox, TreeEntryBox, TreeRef, TreeUpdate, UpdateAction,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::util::errors::ErrorBuf;

    const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/c/tests/resources");
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    /// `testrepo.git` trees: its `HEAD`'s (three blobs), and one holding three blobs and
    /// the subtree `ab`.
    const FLAT: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const NESTED: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
    /// `testrepo_256.git`'s `HEAD` tree, with SHA256 ids.
    const FLAT_256: &str = "f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736";

    /// The calling thread's last error class and message.
    fn c_last() -> (c_int, String) {
        {
            // SAFETY: `git_error_last` never returns NULL after init, and its message is a
            // NUL-terminated string, both live until the next libgit2 call on this thread,
            // after the copy.
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
        }
    }

    fn rust_error(e: &ErrorBuf) -> (c_int, String) {
        (e.raw_class(), e.message().to_string_lossy().into_owned())
    }

    /// The hex form of `id`.
    fn hex(id: &ffi::git_oid) -> String {
        let mut buf = [0u8; 65];
        // SAFETY: 65 writable bytes bound the write; `id` is live.
        unsafe { ffi::git_oid_tostr(buf.as_mut_ptr().cast(), buf.len(), id) };
        CStr::from_bytes_until_nul(&buf)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// A SHA1 object id of `byte` repeated.
    fn id(byte: u8) -> ffi::git_oid {
        let mut id = [0; 32];
        id[..20].fill(byte);
        ffi::git_oid {
            type_: u8::try_from(ffi::GIT_OID_SHA1).unwrap(),
            id,
        }
    }

    /// A fresh bare repository in a temporary directory, removed on drop after the
    /// repository it holds. Both sides hold theirs as a [`RepositoryBox`]; the C side
    /// calls libgit2 on its raw pointer.
    struct TempRepo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl TempRepo {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("crustify-tree-equiv-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, true).unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    fn open(name: &str) -> RepositoryBox {
        Repository::open(&CString::new(format!("{RESOURCES}/{name}")).unwrap()).unwrap()
    }

    /// The tree `hex` names in `repo`, one counted reference the caller frees.
    fn lookup(repo: &RepositoryBox, hex: &str, kind: OidType) -> NonNull<ffi::git_tree> {
        let oid = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
        let id = ffi::git_oid {
            type_: oid.as_ref().raw_kind(),
            id: oid.as_ref().id(),
        };
        let mut tree = ptr::null_mut();
        // SAFETY: a writable slot, the box's live repository and a live id of its type.
        let rc = unsafe { ffi::git_tree_lookup(&raw mut tree, repo.as_c_ptr(), &raw const id) };
        assert_eq!(rc, 0);
        NonNull::new(tree).unwrap()
    }

    fn free_tree(tree: NonNull<ffi::git_tree>) {
        // SAFETY: a tree `lookup` handed out, released once.
        unsafe { ffi::git_tree_free(tree.as_ptr()) }
    }

    /// The tree `hex` names in `repo`, as the Rust side holds it.
    fn lookup_object<'r>(repo: &'r RepositoryBox, hex: &str, kind: OidType) -> TreeArc<'r> {
        let oid = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
        Tree::lookup(repo.as_ref(), oid.as_ref()).unwrap()
    }

    /// The names `tree` holds.
    fn names(tree: NonNull<ffi::git_tree>) -> Vec<CString> {
        // SAFETY: a live tree, only read.
        let count = unsafe { ffi::git_tree_entrycount(tree.as_ptr()) };
        (0..count)
            .map(|i| {
                // SAFETY: as above; `i` is below the entry count.
                let entry = unsafe { ffi::git_tree_entry_byindex(tree.as_ptr(), i) };
                // SAFETY: a live entry of the tree.
                let name = unsafe { ffi::git_tree_entry_name(entry) };
                // SAFETY: its name is NUL-terminated, copied out while it lives.
                unsafe { CStr::from_ptr(name) }.to_owned()
            })
            .collect()
    }

    /// What a builder holds, as its routines report it.
    #[derive(Debug, PartialEq, Eq)]
    struct State {
        count: usize,
        modes: Vec<(CString, Option<ffi::git_filemode_t>)>,
        written: Result<String, (c_int, String)>,
    }

    /// Reads `builder`'s state, `names` being every name either side ever held.
    fn state(builder: *mut ffi::git_treebuilder, names: &BTreeSet<CString>) -> State {
        {
            // SAFETY: a live builder, unaliased during the reads; each entry `get` returns lives
            // until the builder next changes, after its read; `out` is writable, and the
            // builder's repository outlives it.
            let count = unsafe { ffi::git_treebuilder_entrycount(builder) };
            let modes = names
                .iter()
                .map(|name| {
                    // SAFETY: as above.
                    let entry = unsafe { ffi::git_treebuilder_get(builder, name.as_ptr()) };
                    (
                        name.clone(),
                        (!entry.is_null())
                            // SAFETY: as above.
                            .then(|| unsafe { ffi::git_tree_entry_filemode_raw(entry) }),
                    )
                })
                .collect();
            let mut out = ffi::git_oid::default();
            // SAFETY: as above.
            let written = unsafe {
                if ffi::git_treebuilder_write(&raw mut out, builder) < 0 {
                    Err(c_last())
                } else {
                    Ok(hex(&out))
                }
            };
            State {
                count,
                modes,
                written,
            }
        }
    }

    /// The Rust side of [`state`]: the file modes through
    /// [`get`](libgit2::libgit2::object::tree::TreeBuilderRef::get) (read from a copy of
    /// each entry, standing in for its file-mode getter), and the id through
    /// [`write`](libgit2::libgit2::object::tree::TreeBuilderMut::write); the entry count
    /// raw.
    fn rust_state(builder: &mut TreeBuilderBox<'_>, names: &BTreeSet<CString>) -> State {
        let count = builder.as_ref().len();
        let modes = names
            .iter()
            .map(|name| {
                let entry = builder.as_ref().get(name);
                (name.clone(), entry.map(|e| e.filemode_raw()))
            })
            .collect();
        let written = match builder.as_mut().write() {
            Ok(id) => Ok(id.as_ref().to_hex_string().unwrap()),
            Err(e) => Err(rust_error(e.error())),
        };
        State {
            count,
            modes,
            written,
        }
    }

    /// `git_treebuilder_insert` of a submodule entry `name` naming `id(byte)`; its return
    /// code and, on failure, the error.
    fn insert(
        builder: *mut ffi::git_treebuilder,
        name: &CStr,
        byte: u8,
    ) -> Result<(), (c_int, String)> {
        // SAFETY: a live builder, unaliased; a NUL-terminated name and a live id, copied.
        let rc = unsafe {
            ffi::git_treebuilder_insert(
                ptr::null_mut(),
                builder,
                name.as_ptr(),
                &id(byte),
                ffi::GIT_FILEMODE_COMMIT,
            )
        };
        if rc < 0 { Err(c_last()) } else { Ok(()) }
    }

    /// One step of a builder's life, run on either side.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        Insert(&'static CStr, u8),
        Clear,
    }

    /// The C side: builds through `git_treebuilder_new` (seeded with `source` unless it
    /// is `None`), runs `steps` and records the state after construction and each step.
    fn c_run(
        repo: &RepositoryBox,
        source: Option<NonNull<ffi::git_tree>>,
        steps: &[Step],
        names: &BTreeSet<CString>,
    ) -> Vec<State> {
        let mut builder = ptr::null_mut();
        let source = source.map_or(ptr::null(), |t| t.as_ptr().cast_const());
        // SAFETY: a writable slot, the box's live repository, which outlives the builder,
        // and NULL or a live tree.
        let rc = unsafe { ffi::git_treebuilder_new(&raw mut builder, repo.as_c_ptr(), source) };
        assert_eq!(rc, 0, "{:?}", c_last());
        let mut states = vec![state(builder, names)];
        for step in steps {
            match *step {
                Step::Insert(name, byte) => insert(builder, name, byte).unwrap(),
                // SAFETY: a live builder, unaliased.
                Step::Clear => assert_eq!(unsafe { ffi::git_treebuilder_clear(builder) }, 0),
            }
            states.push(state(builder, names));
        }
        // SAFETY: the builder `git_treebuilder_new` handed out, released once.
        unsafe { ffi::git_treebuilder_free(builder) };
        states
    }

    /// The Rust side of [`c_run`]: [`TreeBuilder::new`] or [`TreeBuilder::from_tree`],
    /// [`insert`](libgit2::libgit2::object::tree::TreeBuilderMut::insert) and
    /// [`clear`](libgit2::libgit2::object::tree::TreeBuilderMut::clear), read through
    /// [`rust_state`].
    fn rust_run(
        repo: &RepositoryBox,
        source: Option<TreeRef<'_, '_>>,
        steps: &[Step],
        names: &BTreeSet<CString>,
    ) -> Vec<State> {
        let mut builder: TreeBuilderBox<'_> = match source {
            None => TreeBuilder::new(repo.as_ref()).unwrap(),
            Some(tree) => TreeBuilder::from_tree(repo.as_ref(), tree).unwrap(),
        };
        let mut states = vec![rust_state(&mut builder, names)];
        for step in steps {
            match *step {
                Step::Insert(name, byte) => {
                    let id = Oid::from_raw(&id(byte).id[..20], OidType::Sha1).unwrap();
                    builder
                        .as_mut()
                        .insert(name, id.as_ref(), FileMode::Commit)
                        .unwrap();
                }
                Step::Clear => builder.as_mut().clear(),
            }
            states.push(rust_state(&mut builder, names));
        }
        states
    }

    /// Runs `steps` on both sides, from a builder seeded with the tree `seed` of the
    /// fixture repository `fixture` (or an empty one), and compares every state.
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the closure lets each handle reborrow for its own lifetime, which the method path cannot"
    )]
    fn compare(tag: &str, seed: Option<(&str, &str, OidType)>, steps: &[Step]) -> Vec<State> {
        let c_repo = TempRepo::new(&format!("{tag}-c"));
        let rust_repo = TempRepo::new(&format!("{tag}-rust"));
        let c_fixture = seed.map(|(name, _, _)| open(name));
        let rust_fixture = seed.map(|(name, _, _)| open(name));
        let c_tree = seed.map(|(_, hex, kind)| lookup(c_fixture.as_ref().unwrap(), hex, kind));
        let rust_tree =
            seed.map(|(_, hex, kind)| lookup_object(rust_fixture.as_ref().unwrap(), hex, kind));

        let mut all: BTreeSet<CString> =
            c_tree.map(names).unwrap_or_default().into_iter().collect();
        all.extend(steps.iter().filter_map(|s| match s {
            Step::Insert(name, _) => Some((*name).to_owned()),
            Step::Clear => None,
        }));
        all.insert(c"absent".to_owned());

        let c_states = c_run(c_repo.repo(), c_tree, steps, &all);
        let rust_states = rust_run(
            rust_repo.repo(),
            rust_tree.as_ref().map(|x| x.as_ref()),
            steps,
            &all,
        );
        c_tree.into_iter().for_each(free_tree);
        drop(rust_tree);
        for (i, (c, rust)) in c_states.iter().zip(&rust_states).enumerate() {
            assert_eq!(c, rust, "{tag}: state after step {i}");
        }
        assert_eq!(c_states.len(), rust_states.len());
        c_states
    }

    const STEPS: &[Step] = &[
        Step::Insert(c"e1", 1),
        Step::Insert(c"e2", 2),
        Step::Insert(c"e1", 3),
        Step::Clear,
        Step::Clear,
        Step::Insert(c"e2", 2),
        Step::Insert(c"README", 4),
        Step::Clear,
    ];

    /// Empty builders: the empty tree first, then the same entries, clears and writes.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_builders_match() {
        let states = compare("empty", None, STEPS);
        assert_eq!(states[0].written.as_deref(), Ok(EMPTY_TREE));
        assert_eq!(states[0].count, 0);
        assert_eq!(states[3].count, 2);
        assert_eq!(states[4].written.as_deref(), Ok(EMPTY_TREE));
    }

    /// Builders seeded with flat and nested trees, the empty tree, and a SHA256 tree for a
    /// SHA1 repository, before and after the same steps.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn seeded_builders_match() {
        for (tag, seed) in [
            ("flat", ("testrepo.git", FLAT, OidType::Sha1)),
            ("nested", ("testrepo.git", NESTED, OidType::Sha1)),
            ("sha256", ("testrepo_256.git", FLAT_256, OidType::Sha256)),
        ] {
            let states = compare(tag, Some(seed), STEPS);
            if seed.2 == OidType::Sha1 {
                assert_eq!(states[0].written.as_deref(), Ok(seed.1), "{tag}");
            }
            assert!(states[0].count >= 3, "{tag}");
        }
        let empty = TempRepo::new("seed-empty");
        let mut builder = TreeBuilder::new(empty.repo().as_ref()).unwrap();
        assert_eq!(
            state(builder.as_c_ptr(), &BTreeSet::new())
                .written
                .as_deref(),
            Ok(EMPTY_TREE)
        );
        builder.as_mut().clear();
        drop(builder);
        // The empty tree, now in a repository of its own on each side.
        let c_repo = TempRepo::new("seed-empty-c");
        let rust_repo = TempRepo::new("seed-empty-rust");
        let c_tree = lookup(empty.repo(), EMPTY_TREE, OidType::Sha1);
        let rust_tree = lookup_object(empty.repo(), EMPTY_TREE, OidType::Sha1);
        let none = BTreeSet::new();
        assert_eq!(
            c_run(c_repo.repo(), Some(c_tree), STEPS, &none),
            rust_run(rust_repo.repo(), Some(rust_tree.as_ref()), STEPS, &none)
        );
        free_tree(c_tree);
        drop(rust_tree);
    }

    mod routines {
        //! Equivalence of the tree-builder routines' wrappers with libgit2's, and of the
        //! walk callback's answers with a C callback's.
        //!
        //! - Steps: each side runs the same inserts (wanting the entry back or not), removals and
        //!   clears over a bare repository of its own holding the same blob and empty tree, the
        //!   inserts including every kind libgit2 refuses (bad names, the unreadable mode, the zero
        //!   id, a missing object, an object of the wrong type) and replacements in place. After
        //!   each step, the step's outcome (the entry handed back, or the error) and the builder's
        //!   state (its count, every name's file mode through `get`, and the tree `write` gives, or
        //!   its code and error) are compared.
        //! - Writing and inserting without an object database (a repository without a git
        //!   directory) fail alike, code included.
        //! - Allocation failures: libgit2's allocator fails its `k`-th allocation inside an insert
        //!   and a write, for each `k` until they succeed; both sides must fail alike and leave
        //!   their builders alike. This runs alone in a child process.
        //! - Walk callbacks: `git_tree_walk` over one nested tree, with a C callback, and with a C
        //!   callback handing each entry (copied) and root to [`WalkCallback::call`] and returning
        //!   its answer, visit the same entries under the same roots and return and leave the same,
        //!   in both orders, for answers that continue, skip and stop with `GIT_EUSER`, and for a
        //!   subtree the object database lacks. The public walk, `TreeRef::walk`, over a tree of
        //!   the Rust side's own repository, must do the same as `git_tree_walk` over the C side's,
        //!   its `Ok(Continue)` standing for 0 and `Ok(Break)` for `GIT_EUSER`.
        //! - Filters: `TreeBuilderMut::remove_if` and `git_treebuilder_filter`, over builders
        //!   filled alike, lend the same entries in the same order, remove the same ones and leave
        //!   the same builders, run after run.
        //!
        //! Documented divergences, outside these comparisons:
        //!
        //! - `insert`, `insert_entry` and `remove` return libgit2's error without its code, which
        //!   is always `-1`; the C side checks that.
        //! - A name of more than 65535 bytes, which libgit2 inserts while reporting nothing and
        //!   then writes as a corrupt tree, is refused in Rust (the unit test
        //!   `overlong_names_are_refused` keeps the corrected behaviour).
        //! - An id of another type than the repository's, for a mode libgit2 looks up, is refused
        //!   in Rust (`GIT_ERROR_INVALID`), where libgit2 formats it out of bounds while looking it
        //!   up (`tests/ub_safe/tree.rs`,
        //!   `routines::ids_of_another_type_are_refused_before_the_lookup`).
        //! - A closure stops a walk with `GIT_EUSER` alone, where a C callback may return any
        //!   negative value.

        use std::ffi::c_char;
        use std::ops::ControlFlow;

        use libgit2::libgit2::object::tree::{
            TreeBuilderFilter, TreeEntryRef, WalkAction, WalkCallback, WalkMode,
        };
        use libgit2::libgit2::util::errors::ErrorCode;

        use super::*;

        /// An id each side spells alike: `id(byte)`, the zero id, or an object written
        /// into each side's repository.
        #[derive(Clone, Copy, Debug)]
        enum Id {
            Byte(u8),
            Zero,
            Blob,
            EmptyTree,
        }

        #[derive(Clone, Copy, Debug)]
        enum Op<'a> {
            Insert(&'a CStr, Id, FileMode),
            InsertEntry(&'a CStr, Id, FileMode),
            Remove(&'a CStr),
            Clear,
        }

        const OPS: &[Op<'static>] = &[
            Op::Insert(c"blob", Id::Blob, FileMode::Blob),
            Op::InsertEntry(c"exec", Id::Blob, FileMode::BlobExecutable),
            Op::InsertEntry(c"link", Id::Blob, FileMode::Link),
            Op::Insert(c"tree", Id::EmptyTree, FileMode::Tree),
            Op::InsertEntry(c"sub", Id::Byte(7), FileMode::Commit),
            // Replacements in place.
            Op::InsertEntry(c"blob", Id::Blob, FileMode::BlobExecutable),
            Op::Insert(c"sub", Id::Byte(8), FileMode::Commit),
            Op::InsertEntry(c"tree", Id::Byte(9), FileMode::Commit),
            // Refused.
            Op::Insert(c"", Id::Blob, FileMode::Blob),
            Op::InsertEntry(c".", Id::Blob, FileMode::Blob),
            Op::Insert(c"..", Id::Blob, FileMode::Blob),
            Op::Insert(c".git", Id::Blob, FileMode::Blob),
            Op::InsertEntry(c".GIT", Id::Blob, FileMode::Blob),
            Op::Insert(c"a/b", Id::Blob, FileMode::Blob),
            Op::Insert(c"bad", Id::Blob, FileMode::Unreadable),
            Op::InsertEntry(c"bad", Id::Zero, FileMode::Commit),
            Op::Insert(c"bad", Id::Byte(5), FileMode::Blob),
            Op::InsertEntry(c"bad", Id::Byte(5), FileMode::Tree),
            Op::Insert(c"bad", Id::Blob, FileMode::Tree),
            Op::InsertEntry(c"blob", Id::EmptyTree, FileMode::Blob),
            // Removals.
            Op::Remove(c"exec"),
            Op::Remove(c"exec"),
            Op::Remove(c"absent"),
            Op::Remove(c""),
            Op::Clear,
            Op::Remove(c"blob"),
            Op::InsertEntry(c"blob", Id::Blob, FileMode::Blob),
            Op::Remove(c"blob"),
            Op::Clear,
        ];

        /// An entry as C reads it: name, hex id and raw mode.
        type View = (CString, String, ffi::git_filemode_t);

        /// One step's outcome: the entry handed back (`insert_entry`), or the error.
        type Outcome = Result<Option<View>, (c_int, String)>;

        /// The builder's state: count, file modes and the written tree, or the failure's
        /// code and error.
        #[derive(Debug, PartialEq, Eq)]
        struct Snapshot {
            count: usize,
            modes: Vec<(CString, Option<ffi::git_filemode_t>)>,
            written: Result<String, (c_int, c_int, String)>,
        }

        fn view(entry: *const ffi::git_tree_entry) -> View {
            // SAFETY: a live entry, only read.
            let name = unsafe { ffi::git_tree_entry_name(entry) };
            // SAFETY: as above.
            let id = unsafe { ffi::git_tree_entry_id(entry) };
            // SAFETY: as above.
            let mode = unsafe { ffi::git_tree_entry_filemode_raw(entry) };
            // SAFETY: the name is NUL-terminated and lives with the entry; it is copied.
            let name = unsafe { CStr::from_ptr(name) }.to_owned();
            // SAFETY: the id lives with the entry; it is copied.
            (name, hex(unsafe { &*id }), mode)
        }

        /// A bare repository holding a blob of `content\n` and the empty tree, written raw
        /// alike on both sides; their raw ids.
        fn setup(tag: &str) -> (TempRepo, ffi::git_oid, ffi::git_oid) {
            let temp = TempRepo::new(tag);
            let repo = temp.repo().as_c_ptr();
            let mut blob = ffi::git_oid::default();
            let mut tree = ffi::git_oid::default();
            let mut builder = ptr::null_mut();
            {
                let data = b"content\n";
                // SAFETY: writable ids and slot, the live repository and 8 readable bytes; the
                // builder is freed once.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &raw mut blob,
                        repo,
                        data.as_ptr().cast(),
                        data.len(),
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: as above.
                unsafe {
                    assert_eq!(
                        ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()),
                        0
                    );
                };
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut tree, builder), 0) };
                // SAFETY: as above.
                unsafe { ffi::git_treebuilder_free(builder) };
            }
            (temp, blob, tree)
        }

        fn raw_id(spec: Id, blob: &ffi::git_oid, tree: &ffi::git_oid) -> ffi::git_oid {
            match spec {
                Id::Byte(b) => id(b),
                Id::Zero => ffi::git_oid {
                    type_: u8::try_from(ffi::GIT_OID_SHA1).unwrap(),
                    id: [0; 32],
                },
                Id::Blob => *blob,
                Id::EmptyTree => *tree,
            }
        }

        fn names() -> BTreeSet<CString> {
            let mut all: BTreeSet<CString> = OPS
                .iter()
                .filter_map(|op| match *op {
                    Op::Insert(n, ..) | Op::InsertEntry(n, ..) | Op::Remove(n) => {
                        Some(n.to_owned())
                    }
                    Op::Clear => None,
                })
                .collect();
            all.insert(c"absent".to_owned());
            all
        }

        fn c_snapshot(builder: *mut ffi::git_treebuilder, names: &BTreeSet<CString>) -> Snapshot {
            {
                // SAFETY: a live builder, unaliased during the reads; each entry `get`
                // returns is read before the builder next changes; `out` is writable.
                let modes = unsafe {
                    names
                        .iter()
                        .map(|name| {
                            let entry = ffi::git_treebuilder_get(builder, name.as_ptr());
                            (name.clone(), (!entry.is_null()).then(|| view(entry).2))
                        })
                        .collect()
                };
                let mut out = ffi::git_oid::default();
                // SAFETY: as above.
                let rc = unsafe { ffi::git_treebuilder_write(&raw mut out, builder) };
                let written = if rc < 0 {
                    let (class, message) = c_last();
                    Err((rc, class, message))
                } else {
                    Ok(hex(&out))
                };
                // SAFETY: as above.
                unsafe {
                    Snapshot {
                        count: ffi::git_treebuilder_entrycount(builder),
                        modes,
                        written,
                    }
                }
            }
        }

        fn rust_snapshot(builder: &mut TreeBuilderBox<'_>, names: &BTreeSet<CString>) -> Snapshot {
            let modes = names
                .iter()
                .map(|name| {
                    let entry = builder.as_ref().get(name);
                    (name.clone(), entry.map(|e| e.filemode_raw()))
                })
                .collect();
            let written = match builder.as_mut().write() {
                Ok(id) => Ok(id.as_ref().to_hex_string().unwrap()),
                Err(e) => {
                    let (class, message) = rust_error(e.error());
                    Err((e.raw_code(), class, message))
                }
            };
            Snapshot {
                count: builder.as_ref().len(),
                modes,
                written,
            }
        }

        /// The C side of one step.
        fn c_step(
            builder: *mut ffi::git_treebuilder,
            op: Op<'_>,
            blob: &ffi::git_oid,
            tree: &ffi::git_oid,
        ) -> Outcome {
            let failed = |rc: c_int| {
                assert_eq!(rc, -1, "{op:?}");
                Err(c_last())
            };
            match op {
                Op::Insert(name, spec, mode) | Op::InsertEntry(name, spec, mode) => {
                    let want = matches!(op, Op::InsertEntry(..));
                    let mut out = ptr::null();
                    let slot = if want { &raw mut out } else { ptr::null_mut() };
                    let id = raw_id(spec, blob, tree);
                    // SAFETY: NULL or a writable slot, a live builder, unaliased, a
                    // NUL-terminated name and a live id, both copied.
                    let rc = unsafe {
                        ffi::git_treebuilder_insert(
                            slot,
                            builder,
                            name.as_ptr(),
                            &raw const id,
                            mode.raw(),
                        )
                    };
                    if rc < 0 {
                        return failed(rc);
                    }
                    assert_eq!(rc, 0);
                    // The entry is read before the builder next changes.
                    Ok(want.then(|| view(out)))
                }
                Op::Remove(name) => {
                    // SAFETY: a live builder, unaliased, and a NUL-terminated name.
                    let rc = unsafe { ffi::git_treebuilder_remove(builder, name.as_ptr()) };
                    if rc < 0 { failed(rc) } else { Ok(None) }
                }
                Op::Clear => {
                    // SAFETY: a live builder, unaliased.
                    assert_eq!(unsafe { ffi::git_treebuilder_clear(builder) }, 0);
                    Ok(None)
                }
            }
        }

        /// The Rust side of one step.
        fn rust_step(
            builder: &mut TreeBuilderBox<'_>,
            op: Op<'_>,
            blob: &ffi::git_oid,
            tree: &ffi::git_oid,
        ) -> Outcome {
            let oid = |spec| {
                let raw = raw_id(spec, blob, tree);
                Oid::from_raw(&raw.id[..20], OidType::Sha1).unwrap()
            };
            let error = |e: ErrorBuf| rust_error(&e);
            match op {
                Op::Insert(name, spec, mode) => builder
                    .as_mut()
                    .insert(name, oid(spec).as_ref(), mode)
                    .map(|()| None)
                    .map_err(error),
                Op::InsertEntry(name, spec, mode) => {
                    let mut m = builder.as_mut();
                    let id = oid(spec);
                    let entry = m.insert_entry(name, id.as_ref(), mode);
                    entry.map(|e| Some(entry_view(e))).map_err(error)
                }
                Op::Remove(name) => builder.as_mut().remove(name).map(|()| None).map_err(error),
                Op::Clear => {
                    builder.as_mut().clear();
                    Ok(None)
                }
            }
        }

        /// A lent entry as C reads it, from a copy.
        fn entry_view(entry: TreeEntryRef<'_>) -> View {
            let copy = entry.to_tree_entry().unwrap();
            view(copy.as_c_ptr())
        }

        /// Every step's outcome and the state after it match, from an empty builder.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn steps_match() {
            super::super::init();
            let all = names();
            let (c_temp, c_blob, c_tree) = setup("routines-c");
            let (rust_temp, rust_blob, rust_tree) = setup("routines-rust");
            assert_eq!(hex(&c_blob), hex(&rust_blob));

            let mut c_builder = ptr::null_mut();
            // SAFETY: a writable slot and the live repository, which outlives the builder.
            let rc = unsafe {
                ffi::git_treebuilder_new(&raw mut c_builder, c_temp.repo().as_c_ptr(), ptr::null())
            };
            assert_eq!(rc, 0);
            let mut rust_builder = TreeBuilder::new(rust_temp.repo().as_ref()).unwrap();
            assert_eq!(
                c_snapshot(c_builder, &all),
                rust_snapshot(&mut rust_builder, &all)
            );
            let mut refused = 0;
            for (i, op) in OPS.iter().enumerate() {
                let c = c_step(c_builder, *op, &c_blob, &c_tree);
                let rust = rust_step(&mut rust_builder, *op, &rust_blob, &rust_tree);
                assert_eq!(c, rust, "step {i}: {op:?}");
                refused += usize::from(c.is_err());
                assert_eq!(
                    c_snapshot(c_builder, &all),
                    rust_snapshot(&mut rust_builder, &all),
                    "state after step {i}: {op:?}"
                );
            }
            assert_eq!(refused, 16);
            drop(rust_builder);
            // SAFETY: the builder made above, released once.
            unsafe { ffi::git_treebuilder_free(c_builder) };
        }

        /// Without an object database, writing fails with the same code and error, and so
        /// does an insert libgit2 looks up; a submodule entry goes in on both sides.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn missing_object_databases_match() {
            super::super::init();
            let c_repo = Repository::new().unwrap();
            let rust_repo = Repository::new().unwrap();
            let all = names();
            let (blob, tree) = (id(1), id(2));
            let mut c_builder = ptr::null_mut();
            // SAFETY: a writable slot and the live repository, which outlives the builder.
            let rc = unsafe {
                ffi::git_treebuilder_new(&raw mut c_builder, c_repo.as_c_ptr(), ptr::null())
            };
            assert_eq!(rc, 0);
            let mut rust_builder = TreeBuilder::new(rust_repo.as_ref()).unwrap();
            for op in [
                Op::Insert(c"sub", Id::Byte(3), FileMode::Commit),
                Op::Insert(c"blob", Id::Blob, FileMode::Blob),
                Op::InsertEntry(c"tree", Id::EmptyTree, FileMode::Tree),
                Op::Remove(c"sub"),
            ] {
                assert_eq!(
                    c_step(c_builder, op, &blob, &tree),
                    rust_step(&mut rust_builder, op, &blob, &tree),
                    "{op:?}"
                );
                let c = c_snapshot(c_builder, &all);
                assert!(c.written.is_err());
                assert_eq!(c, rust_snapshot(&mut rust_builder, &all), "{op:?}");
            }
            drop(rust_builder);
            // SAFETY: the builder made above, released once.
            unsafe { ffi::git_treebuilder_free(c_builder) };
        }

        /// A walk's return, its visits as (root, entry view), and the thread's error.
        type Walked = (c_int, Vec<(CString, View)>, (c_int, String));

        /// A rule answering for the entry `name` under `root`, as a C value.
        type Rule = fn(&CStr, &CStr) -> c_int;

        const RULES: [Rule; 5] = [
            |_, _| 0,
            |_, name| c_int::from(name == c"dir"),
            |root, _| c_int::from(!root.is_empty()),
            |_, name| {
                if name == c"c" {
                    ErrorCode::User.raw()
                } else {
                    0
                }
            },
            |root, name| {
                if root == c"nest/leaf/" && name == c"e" {
                    ErrorCode::User.raw()
                } else {
                    1
                }
            },
        ];

        struct Walk<'a> {
            rule: Rule,
            visits: Vec<(CString, View)>,
            callback: Option<WalkCallback<'a>>,
        }

        unsafe extern "C" fn c_callback(
            root: *const c_char,
            entry: *const ffi::git_tree_entry,
            payload: *mut c_void,
        ) -> c_int {
            // SAFETY: `payload` is the walk's state, reached only here for the call.
            let walk = unsafe { &mut *payload.cast::<Walk<'_>>() };
            // SAFETY: `root` is NUL-terminated and lives for the call.
            let root = unsafe { CStr::from_ptr(root) };
            let seen = view(entry);
            let rc = (walk.rule)(root, &seen.0);
            walk.visits.push((root.to_owned(), seen));
            rc
        }

        /// Hands each entry, copied, and root to the walk's [`WalkCallback`], and returns
        /// its answer as libgit2 reads one.
        unsafe extern "C" fn bridge(
            root: *const c_char,
            entry: *const ffi::git_tree_entry,
            payload: *mut c_void,
        ) -> c_int {
            let (walk, root, copy) = {
                let mut out = ptr::null_mut();
                // SAFETY: as in `c_callback`; the copy is a fresh entry the bridge owns.
                unsafe { assert_eq!(ffi::git_tree_entry_dup(&raw mut out, entry), 0) };
                // SAFETY: as above.
                let walk = unsafe { &mut *payload.cast::<Walk<'_>>() };
                // SAFETY: as above.
                let root = unsafe { CStr::from_ptr(root) };
                // SAFETY: `out` is the duplicate just made, owned here.
                let entry = unsafe { TreeEntryBox::from_c(out) }.unwrap();
                (walk, root, entry)
            };
            match walk.callback.as_mut().unwrap().call(root, copy.as_ref()) {
                WalkAction::Continue => 0,
                WalkAction::Skip => 1,
                WalkAction::Stop => ErrorCode::User.raw(),
            }
        }

        fn walk(
            tree: NonNull<ffi::git_tree>,
            mode: ffi::git_treewalk_mode,
            rule: Rule,
            through_rust: bool,
        ) -> Walked {
            let mut visits = Vec::new();
            let mut answered_stop = false;
            let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
                let seen = entry_view(entry);
                let rc = rule(root, &seen.0);
                visits.push((root.to_owned(), seen));
                answered_stop = rc < 0;
                match rc {
                    0 => WalkAction::Continue,
                    1.. => WalkAction::Skip,
                    _ => WalkAction::Stop,
                }
            };
            let mut state = Walk {
                rule,
                visits: Vec::new(),
                callback: through_rust.then(|| WalkCallback::new(&mut f)),
            };
            let callback: ffi::git_treewalk_cb =
                Some(if through_rust { bridge } else { c_callback });
            // SAFETY: no arguments.
            unsafe { ffi::git_error_clear() };
            // SAFETY: a live tree, a callback and its payload, live for the walk.
            let rc = unsafe {
                ffi::git_tree_walk(tree.as_ptr(), mode, callback, (&raw mut state).cast())
            };
            let stopped = state.callback.as_ref().map(WalkCallback::stopped);
            let c_visits = state.visits;
            drop(state.callback);
            if through_rust {
                // `stopped` tells the closure's stop from libgit2's own failures.
                assert_eq!(stopped, Some(answered_stop));
                assert_eq!(answered_stop, rc == ErrorCode::User.raw());
                assert!(c_visits.is_empty(), "{c_visits:?}");
                (rc, visits, c_last())
            } else {
                (rc, c_visits, c_last())
            }
        }

        /// The walked tree: submodule leaves `a`, `z`, `dir/b`, `dir/sub/c`, and the
        /// subtrees `dir`, `dir/sub` and `nest/leaf` (holding `e`), written raw.
        fn nested_tree(repo: *mut ffi::git_repository) -> ffi::git_oid {
            let subtree = |entries: &[(&CStr, ffi::git_oid, ffi::git_filemode_t)]| {
                let mut builder = ptr::null_mut();
                let mut out = ffi::git_oid::default();
                // SAFETY: a writable slot and id, the live repository, NUL-terminated
                // names and live ids, copied; the builder is freed once.
                unsafe {
                    assert_eq!(
                        ffi::git_treebuilder_new(&raw mut builder, repo, ptr::null()),
                        0
                    );
                };
                // SAFETY: as above.
                unsafe {
                    for (name, id, mode) in entries {
                        let rc = ffi::git_treebuilder_insert(
                            ptr::null_mut(),
                            builder,
                            name.as_ptr(),
                            id,
                            *mode,
                        );
                        assert_eq!(rc, 0);
                    }
                }
                // SAFETY: as above.
                unsafe { assert_eq!(ffi::git_treebuilder_write(&raw mut out, builder), 0) };
                // SAFETY: as above.
                unsafe { ffi::git_treebuilder_free(builder) };
                out
            };
            let (commit, tree) = (ffi::GIT_FILEMODE_COMMIT, ffi::GIT_FILEMODE_TREE);
            let sub = subtree(&[(c"c", id(3), commit)]);
            let dir = subtree(&[(c"b", id(2), commit), (c"sub", sub, tree)]);
            let leaf = subtree(&[(c"e", id(5), commit)]);
            let nest = subtree(&[(c"leaf", leaf, tree)]);
            subtree(&[
                (c"a", id(1), commit),
                (c"dir", dir, tree),
                (c"nest", nest, tree),
                (c"z", id(26), commit),
            ])
        }

        /// Walks through the Rust callback match walks through a C callback answering
        /// alike, in both orders.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walk_callbacks_match_c_callbacks() {
            super::super::init();
            let temp = TempRepo::new("walk-callback");
            let root = nested_tree(temp.repo().as_c_ptr());
            let tree = lookup(temp.repo(), &hex(&root), OidType::Sha1);
            for mode in [ffi::GIT_TREEWALK_PRE, ffi::GIT_TREEWALK_POST] {
                for (i, rule) in RULES.into_iter().enumerate() {
                    let c = walk(tree, mode, rule, false);
                    let rust = walk(tree, mode, rule, true);
                    assert_eq!(c, rust, "mode {mode} rule {i}");
                    assert!(!c.1.is_empty(), "{:?}", c.1);
                }
            }
            let (_, visits, _) = walk(tree, ffi::GIT_TREEWALK_PRE, RULES[0], true);
            assert_eq!(visits.len(), 9);
            free_tree(tree);
        }

        /// A tree holding the submodule `a` and the subtree `broken`, whose tree the object
        /// database lacks, written raw; its id.
        fn broken_tree(repo: *mut ffi::git_repository) -> ffi::git_oid {
            let mut data = b"160000 a\0".to_vec();
            data.extend_from_slice(&[0x11; 20]);
            data.extend_from_slice(b"40000 broken\0");
            data.extend_from_slice(&[0x77; 20]);
            let mut odb = ptr::null_mut();
            let mut out = ffi::git_oid::default();
            {
                // SAFETY: a writable slot and id, the live repository, `data` read for the
                // call; the odb reference is released once.
                unsafe { assert_eq!(ffi::git_repository_odb(&raw mut odb, repo), 0) };
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_odb_write(
                        &raw mut out,
                        odb,
                        data.as_ptr().cast(),
                        data.len(),
                        ffi::GIT_OBJECT_TREE,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
            }
            out
        }

        /// A walk reaching a subtree the object database lacks fails alike through either
        /// callback, after the same visits, with libgit2's lookup error, in both orders.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn walks_into_missing_subtrees_match() {
            super::super::init();
            let temp = TempRepo::new("walk-broken");
            let root = broken_tree(temp.repo().as_c_ptr());
            let tree = lookup(temp.repo(), &hex(&root), OidType::Sha1);
            for mode in [ffi::GIT_TREEWALK_PRE, ffi::GIT_TREEWALK_POST] {
                let c = walk(tree, mode, RULES[0], false);
                assert_eq!(c, walk(tree, mode, RULES[0], true), "mode {mode}");
                assert_eq!(c.0, ffi::GIT_ENOTFOUND);
                assert_eq!(c.1.len(), if mode == ffi::GIT_TREEWALK_PRE { 2 } else { 1 });
            }
            free_tree(tree);
        }

        /// The public walk: [`TreeRef::walk`] over the Rust side's own tree, with a closure
        /// answering as `rule` does, its result read back as the return it stands for:
        /// `Continue` as 0, `Break` as `GIT_EUSER`, a failure as its code, whose error
        /// must be the thread's.
        ///
        /// [`TreeRef::walk`]: libgit2::libgit2::object::tree::TreeRef::walk
        fn public_walk(tree: TreeRef<'_, '_>, mode: ffi::git_treewalk_mode, rule: Rule) -> Walked {
            let mut visits = Vec::new();
            let mut f = |root: &CStr, entry: TreeEntryRef<'_>| {
                let seen = entry_view(entry);
                let rc = rule(root, &seen.0);
                visits.push((root.to_owned(), seen));
                match rc {
                    0 => WalkAction::Continue,
                    1.. => WalkAction::Skip,
                    _ => WalkAction::Stop,
                }
            };
            let mut callback = WalkCallback::new(&mut f);
            // SAFETY: no arguments.
            unsafe { ffi::git_error_clear() };
            let walked = tree.walk(WalkMode::from_raw(mode).unwrap(), &mut callback);
            let rc = match walked {
                Ok(ControlFlow::Continue(())) => 0,
                Ok(ControlFlow::Break(())) => {
                    assert!(callback.stopped());
                    ErrorCode::User.raw()
                }
                Err(e) => {
                    assert!(!callback.stopped());
                    assert_eq!(rust_error(e.error()), c_last());
                    e.raw_code()
                }
            };
            drop(callback);
            (rc, visits, c_last())
        }

        /// [`TreeRef::walk`] over the Rust side's tree matches `git_tree_walk` with a C
        /// callback over the C side's, each in a repository of its own: the same visits
        /// under the same roots, return and error, in both orders, for answers that
        /// continue, skip and stop, twice over; and the same failure for a subtree the
        /// object database lacks.
        ///
        /// [`TreeRef::walk`]: libgit2::libgit2::object::tree::TreeRef::walk
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn public_walks_match_c_walks() {
            super::super::init();
            let c_temp = TempRepo::new("public-walk-c");
            let rust_temp = TempRepo::new("public-walk-rust");
            let c_root = hex(&nested_tree(c_temp.repo().as_c_ptr()));
            let rust_root = hex(&nested_tree(rust_temp.repo().as_c_ptr()));
            assert_eq!(c_root, rust_root);
            let c_tree = lookup(c_temp.repo(), &c_root, OidType::Sha1);
            let rust_tree = lookup_object(rust_temp.repo(), &rust_root, OidType::Sha1);
            for round in 0..2 {
                for mode in [ffi::GIT_TREEWALK_PRE, ffi::GIT_TREEWALK_POST] {
                    for (i, rule) in RULES.into_iter().enumerate() {
                        let c = walk(c_tree, mode, rule, false);
                        let rust = public_walk(rust_tree.as_ref(), mode, rule);
                        assert_eq!(c, rust, "round {round} mode {mode} rule {i}");
                    }
                }
            }
            free_tree(c_tree);
            drop(rust_tree);

            let c_broken = hex(&broken_tree(c_temp.repo().as_c_ptr()));
            let rust_broken = hex(&broken_tree(rust_temp.repo().as_c_ptr()));
            let c_tree = lookup(c_temp.repo(), &c_broken, OidType::Sha1);
            let rust_tree = lookup_object(rust_temp.repo(), &rust_broken, OidType::Sha1);
            for mode in [ffi::GIT_TREEWALK_PRE, ffi::GIT_TREEWALK_POST] {
                for rule in [RULES[0], RULES[1]] {
                    let c = walk(c_tree, mode, rule, false);
                    assert_eq!(
                        c,
                        public_walk(rust_tree.as_ref(), mode, rule),
                        "mode {mode}"
                    );
                }
            }
            free_tree(c_tree);
        }

        /// What a filter picks, from an entry as C reads it.
        type Pick = fn(&View) -> bool;

        /// A filter run's visits, in the order the builder lent its entries.
        struct Filtering {
            pick: Pick,
            visits: Vec<View>,
        }

        unsafe extern "C" fn c_filter(
            entry: *const ffi::git_tree_entry,
            payload: *mut c_void,
        ) -> c_int {
            // SAFETY: `payload` is the run's state; `entry` is live for the call.
            let state = unsafe { &mut *payload.cast::<Filtering>() };
            let seen = view(entry);
            let remove = (state.pick)(&seen);
            state.visits.push(seen);
            c_int::from(remove)
        }

        const PICKS: [Pick; 6] = [
            |_| false,
            |(name, ..)| name.to_bytes().ends_with(b"3") || name.as_bytes() == b"link",
            |(.., mode)| *mode == ffi::GIT_FILEMODE_TREE,
            |(_, id, _)| id.as_bytes()[0] % 2 == 0,
            |(name, ..)| name.as_bytes().first() == Some(&b's'),
            |_| true,
        ];

        /// The builder of each side, filled alike: a blob, an executable, a link, the
        /// empty tree, and submodules `s00` to `s19`.
        fn fill(builder: &mut dyn FnMut(Op<'_>) -> Outcome, names: &[CString]) {
            for op in [
                Op::Insert(c"blob", Id::Blob, FileMode::Blob),
                Op::Insert(c"exec", Id::Blob, FileMode::BlobExecutable),
                Op::Insert(c"link", Id::Blob, FileMode::Link),
                Op::Insert(c"tree", Id::EmptyTree, FileMode::Tree),
            ] {
                builder(op).unwrap();
            }
            for (i, name) in names.iter().enumerate() {
                builder(Op::Insert(
                    name,
                    Id::Byte(u8::try_from(i).unwrap() * 7 + 1),
                    FileMode::Commit,
                ))
                .unwrap();
            }
        }

        /// [`TreeBuilderMut::remove_if`] matches `git_treebuilder_filter`, each over a
        /// builder of its own filled alike: the closure and the C callback are lent the
        /// same entries in the same order, the same ones go, and the builders' states
        /// (count, every name's mode, the tree written) match after each run. The runs
        /// keep nothing, pick by name, mode and id, and finally remove everything; the
        /// builders are filled again in between.
        ///
        /// [`TreeBuilderMut::remove_if`]: libgit2::libgit2::object::tree::TreeBuilderMut::remove_if
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn filters_match_c() {
            super::super::init();
            let (c_temp, c_blob, c_tree) = setup("filter-c");
            let (rust_temp, rust_blob, rust_tree) = setup("filter-rust");
            let submodules: Vec<CString> = (0..20)
                .map(|i| CString::new(format!("s{i:02}")).unwrap())
                .collect();
            let mut all = names();
            all.extend(submodules.iter().cloned());

            let mut c_builder = ptr::null_mut();
            // SAFETY: a writable slot and the live repository, which outlives the builder.
            let rc = unsafe {
                ffi::git_treebuilder_new(&raw mut c_builder, c_temp.repo().as_c_ptr(), ptr::null())
            };
            assert_eq!(rc, 0);
            let mut rust_builder = TreeBuilder::new(rust_temp.repo().as_ref()).unwrap();
            for round in 0..2 {
                fill(
                    &mut |op| c_step(c_builder, op, &c_blob, &c_tree),
                    &submodules,
                );
                fill(
                    &mut |op| rust_step(&mut rust_builder, op, &rust_blob, &rust_tree),
                    &submodules,
                );
                for (i, pick) in PICKS.into_iter().enumerate() {
                    let mut state = Filtering {
                        pick,
                        visits: Vec::new(),
                    };
                    // SAFETY: a live builder, unaliased; the callback and its state are
                    // live for the call.
                    let rc = unsafe {
                        ffi::git_treebuilder_filter(
                            c_builder,
                            Some(c_filter),
                            (&raw mut state).cast(),
                        )
                    };
                    assert_eq!(rc, 0);

                    let mut visits = Vec::new();
                    let mut f = |entry: TreeEntryRef<'_>| {
                        let seen = entry_view(entry);
                        let remove = pick(&seen);
                        visits.push(seen);
                        remove
                    };
                    let mut filter = TreeBuilderFilter::new(&mut f);
                    rust_builder.as_mut().remove_if(&mut filter);
                    drop(filter);
                    assert_eq!(state.visits, visits, "round {round} pick {i}");
                    assert_eq!(
                        c_snapshot(c_builder, &all),
                        rust_snapshot(&mut rust_builder, &all),
                        "round {round} pick {i}"
                    );
                }
                assert_eq!(rust_builder.as_ref().len(), 0);
            }
            drop(rust_builder);
            // SAFETY: the builder made above, released once.
            unsafe { ffi::git_treebuilder_free(c_builder) };
        }

        /// Runs `c` and then `rust` with the `k`-th libgit2 allocation of each failing, for
        /// each `k` until both succeed; their outcomes, and both builders' states after
        /// each attempt, must match. The number of failing attempts.
        fn sweep<T: PartialEq + std::fmt::Debug>(
            what: &str,
            mut c: impl FnMut() -> Result<T, (c_int, c_int, String)>,
            mut rust: impl FnMut() -> Result<T, (c_int, c_int, String)>,
            mut states: impl FnMut() -> (Snapshot, Snapshot),
        ) -> usize {
            for k in 1.. {
                COUNTDOWN.store(k, SeqCst);
                let c_out = c();
                COUNTDOWN.store(k, SeqCst);
                let rust_out = rust();
                COUNTDOWN.store(0, SeqCst);
                assert_eq!(c_out, rust_out, "{what}: failing allocation {k}");
                let (c_state, rust_state) = states();
                assert_eq!(c_state, rust_state, "{what}: state after allocation {k}");
                if c_out.is_ok() {
                    return k - 1;
                }
            }
            unreachable!()
        }

        /// Inserting (a new entry, and the map's growth) and writing (the sorted entries,
        /// the write buffer and its growth, the object) fail alike at every allocation,
        /// leaving the builders alike; the error comes back with C's class and message.
        /// Runs alone in a child process, since the allocator is process-global.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        #[expect(
            clippy::too_many_lines,
            reason = "one scenario, its steps run and compared in order"
        )]
        fn routine_allocation_failures_match() {
            isolated(
                concat!(module_path!(), "::routine_allocation_failures_match"),
                || {
                    let allocator = ffi::git_allocator {
                        gmalloc: Some(failing_malloc),
                        grealloc: Some(failing_realloc),
                        gfree: Some(plain_free),
                    };
                    // SAFETY: a fully set, libc-backed allocator, which libgit2 copies.
                    let rc = unsafe {
                        ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &allocator)
                    };
                    assert_eq!(rc, 0);
                    let c_temp = TempRepo::new("routines-oom-c");
                    let rust_temp = TempRepo::new("routines-oom-rust");
                    let mut c_builder = ptr::null_mut();
                    // SAFETY: a writable slot and the live repository, which outlives the
                    // builder.
                    let rc = unsafe {
                        ffi::git_treebuilder_new(
                            &raw mut c_builder,
                            c_temp.repo().as_c_ptr(),
                            ptr::null(),
                        )
                    };
                    assert_eq!(rc, 0);
                    let rust_builder = std::cell::RefCell::new(
                        TreeBuilder::new(rust_temp.repo().as_ref()).unwrap(),
                    );
                    // Names long enough to outgrow the write buffer's first guess.
                    let names: Vec<CString> = (0..12u8)
                        .map(|i| CString::new(vec![b'a' + i; 200]).unwrap())
                        .collect();
                    let all: BTreeSet<CString> = names.iter().cloned().collect();
                    let states = || {
                        (
                            c_snapshot_unwritten(c_builder, &all),
                            rust_snapshot_unwritten(&mut rust_builder.borrow_mut(), &all),
                        )
                    };
                    let no_code = |(class, message): (c_int, String)| (-1, class, message);

                    let mut failures = 0;
                    for (i, name) in (1u8..).zip(&names) {
                        // No entry back: the Rust side reads one from a copy, which
                        // allocates.
                        let op = Op::Insert(name, Id::Byte(i), FileMode::Commit);
                        let (blob, tree) = (id(0), id(0));
                        failures += sweep(
                            "insert",
                            || c_step(c_builder, op, &blob, &tree).map_err(no_code),
                            || {
                                rust_step(&mut rust_builder.borrow_mut(), op, &blob, &tree)
                                    .map_err(no_code)
                            },
                            states,
                        );
                    }
                    assert!(failures >= names.len(), "{failures}");

                    drop(rust_builder);
                    // SAFETY: the builder made above, released once.
                    unsafe { ffi::git_treebuilder_free(c_builder) };

                    // Writes, each attempt from a fresh pair of builders holding the same
                    // entries: a builder whose write buffer ran out of memory keeps it
                    // marked so (libgit2's defect, see `TreeBuilderMut::write`), and every
                    // later write of it fails. Each side's second write must match too.
                    let mut failures = 0;
                    let mut stuck = 0;
                    for k in 1.. {
                        let mut c_builder = ptr::null_mut();
                        // SAFETY: a writable slot and the live repository, which outlives
                        // the builder.
                        let rc = unsafe {
                            ffi::git_treebuilder_new(
                                &raw mut c_builder,
                                c_temp.repo().as_c_ptr(),
                                ptr::null(),
                            )
                        };
                        assert_eq!(rc, 0);
                        let mut rust_builder = TreeBuilder::new(rust_temp.repo().as_ref()).unwrap();
                        for (i, name) in (1u8..).zip(&names) {
                            let op = Op::Insert(name, Id::Byte(i), FileMode::Commit);
                            let (blob, tree) = (id(0), id(0));
                            assert_eq!(c_step(c_builder, op, &blob, &tree), Ok(None));
                            assert_eq!(rust_step(&mut rust_builder, op, &blob, &tree), Ok(None));
                        }
                        let c_write = || {
                            let mut out = ffi::git_oid::default();
                            // SAFETY: a writable id and a live builder, unaliased.
                            let rc = unsafe { ffi::git_treebuilder_write(&raw mut out, c_builder) };
                            if rc < 0 {
                                let (class, message) = c_last();
                                Err((rc, class, message))
                            } else {
                                Ok(hex(&out))
                            }
                        };
                        let mut rust_write = || match rust_builder.as_mut().write() {
                            Ok(id) => Ok(id.as_ref().to_hex_string().unwrap()),
                            Err(e) => {
                                let (class, message) = rust_error(e.error());
                                Err((e.raw_code(), class, message))
                            }
                        };
                        COUNTDOWN.store(k, SeqCst);
                        let c = c_write();
                        COUNTDOWN.store(k, SeqCst);
                        let rust = rust_write();
                        COUNTDOWN.store(0, SeqCst);
                        assert_eq!(c, rust, "write: failing allocation {k}");
                        let succeeded = c.is_ok();
                        let (c_again, rust_again) = (c_write(), rust_write());
                        assert_eq!(c_again, rust_again, "write again after allocation {k}");
                        stuck += usize::from(!succeeded && c_again.is_err());
                        drop(rust_builder);
                        // SAFETY: the builder made above, released once.
                        unsafe { ffi::git_treebuilder_free(c_builder) };
                        if succeeded {
                            break;
                        }
                        failures += 1;
                    }
                    // The entries' vector, the buffer, its growth, then the object's write.
                    assert!(failures >= 3, "{failures}");
                    assert!(stuck >= 1, "{stuck}");

                    // SAFETY: NULL restores libgit2's default, libc-backed allocator.
                    let rc = unsafe {
                        ffi::git_libgit2_opts(
                            ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                            ptr::null::<ffi::git_allocator>(),
                        )
                    };
                    assert_eq!(rc, 0);
                },
            );
        }

        /// [`c_snapshot`] without the write, which would allocate.
        fn c_snapshot_unwritten(
            builder: *mut ffi::git_treebuilder,
            names: &BTreeSet<CString>,
        ) -> Snapshot {
            {
                // SAFETY: a live builder, unaliased during the reads; each entry `get` returns
                // is read before the builder next changes.
                let modes = unsafe {
                    names
                        .iter()
                        .map(|name| {
                            let entry = ffi::git_treebuilder_get(builder, name.as_ptr());
                            (name.clone(), (!entry.is_null()).then(|| view(entry).2))
                        })
                        .collect()
                };
                // SAFETY: as above.
                unsafe {
                    Snapshot {
                        count: ffi::git_treebuilder_entrycount(builder),
                        modes,
                        written: Ok(String::new()),
                    }
                }
            }
        }

        /// [`rust_snapshot`] without the write, which would allocate.
        fn rust_snapshot_unwritten(
            builder: &mut TreeBuilderBox<'_>,
            names: &BTreeSet<CString>,
        ) -> Snapshot {
            let modes = names
                .iter()
                .map(|name| {
                    let entry = builder.as_ref().get(name);
                    (name.clone(), entry.map(|e| e.filemode_raw()))
                })
                .collect();
            Snapshot {
                count: builder.as_ref().len(),
                modes,
                written: Ok(String::new()),
            }
        }
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process, so no other test shares libgit2's globals.
    fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// Allocations left before the failing one; `0` fails none.
    static COUNTDOWN: AtomicUsize = AtomicUsize::new(0);

    /// Whether this allocation is the one to fail; the child allocates on one thread.
    fn fails() -> bool {
        let left = COUNTDOWN.load(SeqCst);
        if left == 0 {
            return false;
        }
        COUNTDOWN.store(left - 1, SeqCst);
        left == 1
    }

    unsafe extern "C" fn failing_malloc(
        n: usize,
        _: *const std::ffi::c_char,
        _: c_int,
    ) -> *mut c_void {
        if fails() {
            return ptr::null_mut();
        }
        // SAFETY: libc's `malloc`, for any size.
        unsafe { malloc(n) }
    }

    unsafe extern "C" fn failing_realloc(
        p: *mut c_void,
        n: usize,
        _: *const std::ffi::c_char,
        _: c_int,
    ) -> *mut c_void {
        if fails() {
            return ptr::null_mut();
        }
        // SAFETY: `p` is NULL or a live block of this libc-backed family.
        unsafe { realloc(p, n) }
    }

    unsafe extern "C" fn plain_free(p: *mut c_void) {
        // SAFETY: `p` is NULL or a live block of this libc-backed family.
        unsafe { free(p) }
    }

    /// Each side's result under the failing allocator: the builder's state, or the error.
    type Outcome = Result<State, (c_int, String)>;

    /// A constructor fails its `k`-th allocation on both sides alike, for every `k` until
    /// it succeeds, empty and seeded with a nested tree; the half-built builder is freed
    /// (`ASan`: leak, double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocation_failures_match() {
        isolated(
            concat!(module_path!(), "::allocation_failures_match"),
            || {
                let allocator = ffi::git_allocator {
                    gmalloc: Some(failing_malloc),
                    grealloc: Some(failing_realloc),
                    gfree: Some(plain_free),
                };
                // SAFETY: a fully set allocator, which libgit2 copies; it is libc-backed, as
                // libgit2's default is, so blocks allocated before stay valid.
                let rc = unsafe {
                    ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &allocator)
                };
                assert_eq!(rc, 0);

                let c_repo = TempRepo::new("oom-c");
                let rust_repo = TempRepo::new("oom-rust");
                let fixture = open("testrepo.git");
                let tree = lookup(&fixture, NESTED, OidType::Sha1);
                let rust_fixture = open("testrepo.git");
                let rust_tree = lookup_object(&rust_fixture, NESTED, OidType::Sha1);
                let all: BTreeSet<CString> = names(tree).into_iter().collect();

                for source in [None, Some(tree)] {
                    let mut failures = 0;
                    for k in 1.. {
                        COUNTDOWN.store(k, SeqCst);
                        let mut raw = ptr::null_mut();
                        let src = source.map_or(ptr::null(), |t: NonNull<ffi::git_tree>| {
                            t.as_ptr().cast_const()
                        });
                        // SAFETY: a writable slot, a live repository outliving the builder, and
                        // NULL or a live tree.
                        let rc = unsafe {
                            ffi::git_treebuilder_new(&raw mut raw, c_repo.repo().as_c_ptr(), src)
                        };
                        COUNTDOWN.store(0, SeqCst);
                        let c: Outcome = if rc < 0 {
                            assert!(raw.is_null());
                            Err(c_last())
                        } else {
                            let s = state(raw, &all);
                            // SAFETY: the builder handed out above, released once.
                            unsafe { ffi::git_treebuilder_free(raw) };
                            Ok(s)
                        };

                        COUNTDOWN.store(k, SeqCst);
                        let built = match source {
                            None => TreeBuilder::new(rust_repo.repo().as_ref()),
                            Some(_) => TreeBuilder::from_tree(
                                rust_repo.repo().as_ref(),
                                rust_tree.as_ref(),
                            ),
                        };
                        COUNTDOWN.store(0, SeqCst);
                        let rust: Outcome = match built {
                            Err(e) => Err(rust_error(&e)),
                            Ok(b) => Ok(state(b.as_c_ptr(), &all)),
                        };

                        assert_eq!(
                            c,
                            rust,
                            "failing allocation {k}, source {:?}",
                            source.is_some()
                        );
                        match c {
                            Err(_) => failures += 1,
                            Ok(_) => break,
                        }
                    }
                    assert!(failures >= 1);
                    if source.is_some() {
                        // The calloc, then each of the four entries and the map's growth.
                        assert!(failures >= 5, "{failures}");
                    }
                }
                free_tree(tree);
                drop(rust_tree);
                // SAFETY: NULL restores libgit2's default, libc-backed allocator.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                        ptr::null::<ffi::git_allocator>(),
                    )
                };
                assert_eq!(rc, 0);
            },
        );
    }

    /// `git_tree_create_updated` over `baseline` with one update of `action` at `path`:
    /// the new tree's id, or the error.
    fn create_updated(
        repo: &RepositoryBox,
        baseline: NonNull<ffi::git_tree>,
        action: ffi::git_tree_update_t,
        path: &CStr,
        byte: u8,
    ) -> Result<String, (c_int, String)> {
        let update = ffi::git_tree_update {
            action,
            id: id(byte),
            filemode: ffi::GIT_FILEMODE_COMMIT,
            path: path.as_ptr(),
        };
        let mut out = ffi::git_oid::default();
        // SAFETY: a writable id, the box's live repository, a live tree of it, and one live
        // update whose path is NUL-terminated.
        let rc = unsafe {
            ffi::git_tree_create_updated(
                &raw mut out,
                repo.as_c_ptr(),
                baseline.as_ptr(),
                1,
                &raw const update,
            )
        };
        if rc < 0 { Err(c_last()) } else { Ok(hex(&out)) }
    }

    /// The values `git_tree_create_updated` accepts as an action are exactly those
    /// [`UpdateAction::from_raw`] names; [`UpdateAction::Upsert`] adds the entry and
    /// [`UpdateAction::Remove`] takes it out again; the C side passes the header's
    /// constants, the Rust side its variants' values.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn update_actions_match_what_libgit2_accepts() {
        let temp = TempRepo::new("actions");
        let repo = temp.repo();
        let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
        let base = state(builder.as_c_ptr(), &BTreeSet::new()).written.unwrap();
        builder.as_mut().clear();
        drop(builder);
        let baseline = lookup(repo, &base, OidType::Sha1);

        let c_added = create_updated(repo, baseline, ffi::GIT_TREE_UPDATE_UPSERT, c"sub", 7);
        let rust_added = create_updated(repo, baseline, UpdateAction::Upsert.raw(), c"sub", 7);
        assert_eq!(c_added, rust_added);
        let added = c_added.unwrap();
        assert_ne!(added, base);

        let with_entry = lookup(repo, &added, OidType::Sha1);
        let c_removed = create_updated(repo, with_entry, ffi::GIT_TREE_UPDATE_REMOVE, c"sub", 7);
        let rust_removed = create_updated(repo, with_entry, UpdateAction::Remove.raw(), c"sub", 7);
        assert_eq!(c_removed, rust_removed);
        assert_eq!(c_removed.as_deref(), Ok(base.as_str()));

        for raw in [0, 1, 2, 3, 0x100, u32::MAX] {
            let accepted = create_updated(repo, with_entry, raw, c"sub", 7);
            assert_eq!(
                accepted.is_ok(),
                UpdateAction::from_raw(raw).is_some(),
                "{raw}"
            );
            if let Err(e) = accepted {
                assert_eq!(
                    e,
                    (
                        ffi::GIT_ERROR_TREE.cast_signed(),
                        "unknown action for update".into()
                    )
                );
            }
        }
        free_tree(with_entry);
        free_tree(baseline);
    }

    /// One change of a [`tree_updates_match_c_structs`] step: the path, and for an upsert
    /// the entry's object (a blob by index, or a submodule commit `id(byte)`) and mode.
    #[derive(Clone, Copy, Debug)]
    enum Change {
        Upsert(&'static CStr, Target, FileMode),
        Remove(&'static CStr),
    }

    #[derive(Clone, Copy, Debug)]
    enum Target {
        Blob(usize),
        Id(u8),
    }

    /// The blobs both sides write, whose ids the upserts name.
    const BLOBS: [&[u8]; 3] = [b"zero\n", b"one\n", b"two\n"];

    /// Writes [`BLOBS`] into `repo`, returning their ids.
    fn write_blobs(repo: &RepositoryBox) -> Vec<ffi::git_oid> {
        BLOBS
            .iter()
            .map(|content| {
                let mut out = ffi::git_oid::default();
                // SAFETY: a writable id, the box's live repository and `content.len()`
                // readable bytes.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &raw mut out,
                        repo.as_c_ptr(),
                        content.as_ptr().cast(),
                        content.len(),
                    )
                };
                assert_eq!(rc, 0, "{:?}", c_last());
                out
            })
            .collect()
    }

    /// What C reads out of one update: its action, id (type and bytes), mode and path.
    type Fields = (
        ffi::git_tree_update_t,
        u8,
        [u8; 32],
        ffi::git_filemode_t,
        CString,
    );

    fn c_fields(update: &ffi::git_tree_update) -> Fields {
        // SAFETY: every path of these updates is a live NUL-terminated `&'static CStr`.
        let path = unsafe { CStr::from_ptr(update.path) };
        (
            update.action,
            update.id.type_,
            update.id.id,
            update.filemode,
            path.to_owned(),
        )
    }

    fn rust_fields(update: &TreeUpdate<'_>) -> Fields {
        let r = update.as_ref();
        (
            r.action().raw(),
            r.id().raw_kind(),
            r.id().id(),
            r.filemode().raw(),
            r.path().to_owned(),
        )
    }

    /// `git_tree_create_updated` over `baseline` (the empty tree when `None`) with
    /// `count` updates at `updates`, in `repo`: the new tree's id, or the error.
    fn apply(
        repo: &RepositoryBox,
        baseline: Option<&str>,
        updates: *const ffi::git_tree_update,
        count: usize,
    ) -> Result<String, (c_int, String)> {
        let tree = baseline.map(|hex| lookup(repo, hex, OidType::Sha1));
        let mut out = ffi::git_oid::default();
        // SAFETY: a writable id, the box's live repository, NULL or a live tree of it, and
        // `count` live updates whose paths are NUL-terminated, only read.
        let rc = unsafe {
            ffi::git_tree_create_updated(
                &raw mut out,
                repo.as_c_ptr(),
                tree.map_or(ptr::null_mut(), NonNull::as_ptr),
                count,
                updates,
            )
        };
        let result = if rc < 0 { Err(c_last()) } else { Ok(hex(&out)) };
        if let Some(tree) = tree {
            free_tree(tree);
        }
        result
    }

    /// The steps of [`tree_updates_match_c_structs`]: each a call of
    /// `git_tree_create_updated` on the tree the last successful step wrote.
    const UPDATE_STEPS: &[&[Change]] = &[
        &[
            Change::Upsert(c"a.txt", Target::Blob(0), FileMode::Blob),
            Change::Upsert(c"dir/b.txt", Target::Blob(1), FileMode::BlobExecutable),
            Change::Upsert(c"dir/sub/c.lnk", Target::Blob(2), FileMode::Link),
            Change::Upsert(c"mod", Target::Id(7), FileMode::Commit),
        ],
        // A blob descended into as a tree.
        &[Change::Upsert(
            c"a.txt/inner",
            Target::Blob(0),
            FileMode::Blob,
        )],
        &[
            Change::Remove(c"a.txt"),
            Change::Upsert(c"dir/b.txt", Target::Blob(2), FileMode::Blob),
            Change::Upsert(c"new/deep/x", Target::Blob(0), FileMode::Blob),
        ],
        // A tree replaced with a blob.
        &[Change::Upsert(c"dir", Target::Blob(0), FileMode::Blob)],
        // A blob the repository lacks.
        &[Change::Upsert(
            c"missing.txt",
            Target::Id(0x42),
            FileMode::Blob,
        )],
        // A mode no entry can have.
        &[Change::Upsert(c"u", Target::Blob(0), FileMode::Unreadable)],
        // The subtree left empty goes too.
        &[Change::Remove(c"dir/sub/c.lnk")],
        // Nothing to remove.
        &[Change::Remove(c"nothing-here")],
        // The same path twice.
        &[
            Change::Upsert(c"z", Target::Blob(0), FileMode::Blob),
            Change::Remove(c"z"),
        ],
        &[
            Change::Upsert(c"a.txt", Target::Blob(1), FileMode::Blob),
            Change::Remove(c"mod"),
            Change::Upsert(c"new/deep/y", Target::Id(9), FileMode::Commit),
        ],
    ];

    /// A [`TreeUpdate`] must be the `git_tree_update` C code would build for the same
    /// change: the same fields as C reads them, and the same trees, or errors, when
    /// `git_tree_create_updated` applies an array of them, step after step. Each side
    /// writes the same blobs into a bare repository of its own; the C side fills
    /// `git_tree_update` structs as the header documents them, the Rust side builds
    /// updates with [`TreeUpdate::upsert`] and [`TreeUpdate::remove`], or (every other
    /// change) a removal rewritten into the change by the setters, and passes its
    /// `[TreeUpdate]` to C in place.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn tree_updates_match_c_structs() {
        let (c_temp, rust_temp) = (TempRepo::new("updates-c"), TempRepo::new("updates-rust"));
        let (c_repo, rust_repo) = (c_temp.repo(), rust_temp.repo());
        let c_blobs = write_blobs(c_repo);
        let rust_blobs: Vec<Oid> = write_blobs(rust_repo)
            .iter()
            .map(|id| Oid::from_raw(&id.id[..20], OidType::Sha1).unwrap())
            .collect();

        let (mut c_tip, mut rust_tip): (Option<String>, Option<String>) = (None, None);
        let mut errors = 0;
        for (n, step) in UPDATE_STEPS.iter().enumerate() {
            let c_updates: Vec<ffi::git_tree_update> = step
                .iter()
                .map(|change| match *change {
                    Change::Upsert(path, target, mode) => ffi::git_tree_update {
                        action: ffi::GIT_TREE_UPDATE_UPSERT,
                        id: match target {
                            Target::Blob(i) => c_blobs[i],
                            Target::Id(byte) => id(byte),
                        },
                        filemode: mode.raw(),
                        path: path.as_ptr(),
                    },
                    Change::Remove(path) => ffi::git_tree_update {
                        action: ffi::GIT_TREE_UPDATE_REMOVE,
                        id: ffi::git_oid::default(),
                        filemode: ffi::GIT_FILEMODE_UNREADABLE,
                        path: path.as_ptr(),
                    },
                })
                .collect();
            let rust_updates: Vec<TreeUpdate<'static>> = step
                .iter()
                .enumerate()
                .map(|(i, change)| match *change {
                    Change::Upsert(path, target, mode) => {
                        let target = match target {
                            Target::Blob(i) => rust_blobs[i],
                            Target::Id(byte) => {
                                Oid::from_raw(&id(byte).id[..20], OidType::Sha1).unwrap()
                            }
                        };
                        if i % 2 == 0 {
                            TreeUpdate::upsert(path, target.as_ref(), mode)
                        } else {
                            let mut update = TreeUpdate::remove(c"placeholder");
                            let mut m = update.as_mut();
                            m.set_path(path);
                            m.set_filemode(mode);
                            m.id_mut().copy_from(target.as_ref());
                            m.set_action(UpdateAction::Upsert);
                            update
                        }
                    }
                    Change::Remove(path) => TreeUpdate::remove(path),
                })
                .collect();
            let c_view: Vec<Fields> = c_updates.iter().map(c_fields).collect();
            let rust_view: Vec<Fields> = rust_updates.iter().map(rust_fields).collect();
            assert_eq!(c_view, rust_view, "step {n}");

            let c = apply(
                c_repo,
                c_tip.as_deref(),
                c_updates.as_ptr(),
                c_updates.len(),
            );
            let rust = apply(
                rust_repo,
                rust_tip.as_deref(),
                rust_updates.as_ptr().cast::<ffi::git_tree_update>(),
                rust_updates.len(),
            );
            assert_eq!(c, rust, "step {n}: {step:?}");
            match (c, rust) {
                (Ok(c), Ok(rust)) => (c_tip, rust_tip) = (Some(c), Some(rust)),
                _ => errors += 1,
            }
        }
        assert_eq!(errors, 6);
        let tip = lookup(c_repo, c_tip.as_deref().unwrap(), OidType::Sha1);
        assert_eq!(
            names(tip),
            [c"a.txt", c"dir", c"new"].map(CStr::to_owned).to_vec()
        );
        free_tree(tip);
    }
}

mod tree_entry {
    //! Equivalence of `git2/tree.h`'s tree entry owner with the C lifecycle it wraps.
    //!
    //! [`TreeEntryBox`]'s `clone` / `try_clone` and [`TreeEntryRef::to_tree_entry`] must
    //! copy as `git_tree_entry_dup` does, and its drop release as `git_tree_entry_free`
    //! does. Each side opens `testrepo.git` on its own and takes an owned entry for each
    //! probe path through `git_tree_entry_bypath` ([`TreeEntry::bypath`] on the Rust
    //! side) from commit `763d71a`'s tree: files and directories at every depth.
    //! Then each side copies the entry, copies the copy, releases the original, the
    //! tree and the repository, and copies once more; after every step the sides are
    //! compared: what C reads out of each live entry (name, id, raw and normalised mode,
    //! type) and `git_tree_entry_cmp` between the side's live entries.
    //!
    //! [`path_lookups_match_c`] compares [`TreeEntry::bypath`] with
    //! `git_tree_entry_bypath` on every probe path, the ones that fail included: the
    //! entry each finds (as above), or the code, error class and message. Documented
    //! divergence, outside that comparison: an object that is not a tree has no
    //! [`TreeRef`] to look into, where the C routine reads it as a tree.
    //!
    //! [`getters_match_c`] compares the getters ([`TreeEntryRef::name`], `id`,
    //! `filemode`, `filemode_raw`, `kind`), [`TreeEntryRef::cmp_in_tree`] and
    //! [`TreeEntryRef::to_object`] with `git_tree_entry_name`, `_id`, `_filemode`,
    //! `_filemode_raw`, `_type`, `_cmp` and `_to_object`: on every entry of three trees
    //! (C's borrowed from its tree, Rust's owned through [`TreeEntry::bypath`]), each
    //! entry against every other, and each object looked up in a fixture repository
    //! and in a bare one lacking it; then again after each side released its trees and
    //! repositories, the lookups going to a repository opened afresh. Documented
    //! divergence, outside that comparison: an entry whose id is of another type than
    //! the repository's is refused by `to_object`, where C formats the id out of
    //! bounds; `tree.rs`'s unit tests hold the refusal.
    //!
    //! [`TreeEntryRef::to_tree_entry`]: libgit2::libgit2::object::tree::TreeEntryRef::to_tree_entry
    //! [`TreeEntryRef::name`]: libgit2::libgit2::object::tree::TreeEntryRef::name
    //! [`TreeEntryRef::cmp_in_tree`]: libgit2::libgit2::object::tree::TreeEntryRef::cmp_in_tree
    //! [`TreeEntryRef::to_object`]: libgit2::libgit2::object::tree::TreeEntryRef::to_object

    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tree::{Tree, TreeEntry, TreeEntryBox, TreeRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError, RepositoryRef};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
    );
    const TREE: &CStr = c"ae90f12eea699729ed24555e40b9fd669da12a12";
    const PATHS: [&CStr; 7] = [
        c"README",
        c"new.txt",
        c"ab",
        c"ab/de/",
        c"ab/c/3.txt",
        c"ab/de/fgh",
        c"ab/de/fgh/1.txt",
    ];

    /// What C reads out of a live entry.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct View {
        name: CString,
        id: String,
        raw_mode: u32,
        mode: u32,
        kind: ffi::git_object_t,
    }

    fn view(p: *const ffi::git_tree_entry) -> View {
        let mut out = [0u8; 65];
        // SAFETY: `p` is a live entry; its name and id live as long as it does and are
        // copied at once; `out` has 65 writable bytes.
        let tree_entry_id = unsafe { ffi::git_tree_entry_id(p) };
        // SAFETY: as above.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), tree_entry_id) };
        // SAFETY: as above.
        let tree_entry_name = unsafe { ffi::git_tree_entry_name(p) };
        // SAFETY: as above.
        let cstr = unsafe { CStr::from_ptr(tree_entry_name) };
        // SAFETY: as above.
        let tree_entry_filemode_raw = unsafe { ffi::git_tree_entry_filemode_raw(p) };
        // SAFETY: as above.
        let tree_entry_filemode = unsafe { ffi::git_tree_entry_filemode(p) };
        // SAFETY: as above.
        unsafe {
            View {
                name: cstr.to_owned(),
                id: CStr::from_bytes_until_nul(&out)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                raw_mode: tree_entry_filemode_raw,
                mode: tree_entry_filemode,
                kind: ffi::git_tree_entry_type(p),
            }
        }
    }

    fn cmp(a: *const ffi::git_tree_entry, b: *const ffi::git_tree_entry) -> c_int {
        // SAFETY: two live entries, read for the call only.
        unsafe { ffi::git_tree_entry_cmp(a, b) }.signum()
    }

    /// `git_tree_entry_bypath(tree, path)`: a fresh entry the caller frees.
    fn bypath(tree: *const ffi::git_tree, path: &CStr) -> *mut ffi::git_tree_entry {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live tree and a NUL-terminated path.
        let rc = unsafe { ffi::git_tree_entry_bypath(&raw mut out, tree, path.as_ptr()) };
        assert_eq!(rc, 0, "{path:?}");
        out
    }

    /// `git_tree_entry_dup(entry)`.
    fn dup(entry: *const ffi::git_tree_entry) -> *mut ffi::git_tree_entry {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a live entry, read for the call.
        assert_eq!(unsafe { ffi::git_tree_entry_dup(&raw mut out, entry) }, 0);
        out
    }

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    #[test]
    fn copies_match_c() {
        init();
        let path = CString::new(TESTREPO).unwrap();
        let tree_id = Oid::from_string(TREE, OidType::Sha1).unwrap();

        // The C side: its own repository and tree.
        let mut c_repo = ptr::null_mut();
        let mut c_tree = ptr::null_mut();
        let mut c_id = ffi::git_oid::default();
        {
            // SAFETY: writable slots, a NUL-terminated path and hex id; the repository is open
            // before the tree is looked up in it.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &raw mut c_repo,
                    path.as_ptr(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_oid_from_string(&raw mut c_id, TREE.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
            };
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_tree_lookup(&raw mut c_tree, c_repo, &raw const c_id),
                    0
                );
            };
        }
        // The Rust side: its own repository, and the tree.
        let rust_repo = Repository::open(&path).unwrap();
        let rust_tree = Tree::lookup(rust_repo.as_ref(), tree_id.as_ref()).unwrap();

        // Step 1: an owned entry per path on each side.
        let c_entries: Vec<_> = PATHS.iter().map(|p| bypath(c_tree, p)).collect();
        let rust_entries: Vec<TreeEntryBox> = PATHS
            .iter()
            .map(|p| TreeEntry::bypath(rust_tree.as_ref(), p).unwrap())
            .collect();
        let views = |c: &[*mut ffi::git_tree_entry], r: &[TreeEntryBox]| {
            let c: Vec<_> = c.iter().map(|&e| view(e)).collect();
            let r: Vec<_> = r.iter().map(|e| view(e.as_c_ptr())).collect();
            assert_eq!(c, r);
            c
        };
        let original = views(&c_entries, &rust_entries);

        // Step 2: a copy of each, and a copy of the copy.
        let c_copies: Vec<_> = c_entries.iter().map(|&e| dup(e)).collect();
        let rust_copies: Vec<_> = rust_entries.iter().map(TreeEntryBox::clone).collect();
        let c_copies2: Vec<_> = c_copies.iter().map(|&e| dup(e)).collect();
        let rust_copies2: Vec<_> = rust_copies
            .iter()
            .map(|e| e.as_ref().to_tree_entry().unwrap())
            .collect();
        assert_eq!(views(&c_copies, &rust_copies), original);
        assert_eq!(views(&c_copies2, &rust_copies2), original);

        // Step 3: the originals, the trees and the repositories go.
        for e in c_entries {
            // SAFETY: each original is released once; its copies remain.
            unsafe { ffi::git_tree_entry_free(e) };
        }
        drop(rust_entries);
        // SAFETY: the tree and repository are released once, the tree first.
        unsafe { ffi::git_tree_free(c_tree) };
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_repo) };
        drop(rust_tree);
        drop(rust_repo);
        assert_eq!(views(&c_copies, &rust_copies), original);
        assert_eq!(views(&c_copies2, &rust_copies2), original);

        // Step 4: a third generation, and the order the entries sort in.
        let c_copies3: Vec<_> = c_copies2.iter().map(|&e| dup(e)).collect();
        let rust_copies3: Vec<_> = rust_copies2
            .iter()
            .map(|e| e.try_clone().unwrap())
            .collect();
        assert_eq!(views(&c_copies3, &rust_copies3), original);
        for i in 0..PATHS.len() {
            for j in 0..PATHS.len() {
                let c = cmp(c_copies3[i], c_copies[j]);
                let r = cmp(rust_copies3[i].as_c_ptr(), rust_copies[j].as_c_ptr());
                assert_eq!(c, r, "{:?} vs {:?}", PATHS[i], PATHS[j]);
            }
        }

        for e in c_copies.into_iter().chain(c_copies2).chain(c_copies3) {
            // SAFETY: each copy is released once.
            unsafe { ffi::git_tree_entry_free(e) };
        }
    }

    /// Every probe path's lookup, successes and failures alike.
    const PROBES: [&CStr; 17] = [
        c"README",
        c"ab",
        c"ab/",
        c"ab/de/",
        c"ab/c/3.txt",
        c"ab/de/fgh",
        c"ab/de/fgh/1.txt",
        c"ab/de/fgh/1.txt/",
        c"",
        c"/",
        c"ab//de",
        c"README/",
        c"README/x",
        c"nope",
        c"ab/nope",
        c"./README",
        c"ab/../README",
    ];

    /// What a path lookup gave: the entry as C reads it, or the failure.
    #[derive(Debug, PartialEq, Eq)]
    enum Found {
        Entry(View),
        Failed(c_int, c_int, String),
    }

    fn c_last() -> (c_int, String) {
        {
            // SAFETY: `git_error_last` never returns NULL after init, and its message is a
            // NUL-terminated string, both live until the next libgit2 call on this thread.
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
        }
    }

    #[test]
    fn path_lookups_match_c() {
        init();
        let path = CString::new(TESTREPO).unwrap();
        let tree_id = Oid::from_string(TREE, OidType::Sha1).unwrap();
        let mut c_repo = ptr::null_mut();
        let mut c_tree = ptr::null_mut();
        let mut c_id = ffi::git_oid::default();
        {
            // SAFETY: as in `copies_match_c`.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &raw mut c_repo,
                    path.as_ptr(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_oid_from_string(&raw mut c_id, TREE.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
            };
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_tree_lookup(&raw mut c_tree, c_repo, &raw const c_id),
                    0
                );
            };
        }
        let rust_repo = Repository::open(&path).unwrap();
        let rust_tree = Tree::lookup(rust_repo.as_ref(), tree_id.as_ref()).unwrap();

        let mut c_entries = Vec::new();
        let mut rust_entries = Vec::new();
        for probe in PROBES {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, a live tree and a NUL-terminated path.
            let rc = unsafe { ffi::git_tree_entry_bypath(&raw mut out, c_tree, probe.as_ptr()) };
            let c = if rc < 0 {
                let (class, message) = c_last();
                Found::Failed(rc, class, message)
            } else {
                c_entries.push(out);
                Found::Entry(view(out))
            };
            let rust = match TreeEntry::bypath(rust_tree.as_ref(), probe) {
                Ok(entry) => {
                    let seen = Found::Entry(view(entry.as_c_ptr()));
                    rust_entries.push(entry);
                    seen
                }
                Err(e) => Found::Failed(
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                ),
            };
            assert_eq!(c, rust, "{probe:?}");
        }
        assert_eq!(c_entries.len(), 7);

        // The documented divergence: an object that is not a tree has no tree handle.
        let commit_id =
            Oid::from_string(c"763d71aadf09a7951596c9746c024e7eece7c7af", OidType::Sha1).unwrap();
        let commit = Object::lookup(rust_repo.as_ref(), commit_id.as_ref(), None).unwrap();
        assert!(TreeRef::from_object(commit.as_ref()).is_none());

        // The tree and the repositories go; the entries are still read alike.
        // SAFETY: the tree and repository are released once, the tree first.
        unsafe { ffi::git_tree_free(c_tree) };
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_repo) };
        drop(commit);
        drop(rust_tree);
        drop(rust_repo);
        let c_views: Vec<_> = c_entries.iter().map(|&e| view(e)).collect();
        let rust_views: Vec<_> = rust_entries.iter().map(|e| view(e.as_c_ptr())).collect();
        assert_eq!(c_views, rust_views);
        for e in c_entries {
            // SAFETY: each entry is released once.
            unsafe { ffi::git_tree_entry_free(e) };
        }
    }

    /// What a lookup of an entry's object gave: the object's id and type, or the failure.
    #[derive(Debug, PartialEq, Eq)]
    enum Looked {
        Object(String, ffi::git_object_t),
        Failed(c_int, c_int, String),
    }

    fn looked(p: *const ffi::git_object) -> Looked {
        let mut out = [0u8; 65];
        // SAFETY: a live object; its id is formatted into 65 writable bytes.
        let object_id = unsafe { ffi::git_object_id(p) };
        // SAFETY: as above.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), object_id) };
        // SAFETY: as above.
        unsafe {
            Looked::Object(
                CStr::from_bytes_until_nul(&out)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                ffi::git_object_type(p),
            )
        }
    }

    /// C's `git_tree_entry_to_object`, the object released once viewed.
    fn c_to_object(repo: *mut ffi::git_repository, entry: *const ffi::git_tree_entry) -> Looked {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live repository and a live entry of its id type.
        let rc = unsafe { ffi::git_tree_entry_to_object(&raw mut out, repo, entry) };
        if rc < 0 {
            let (class, message) = c_last();
            return Looked::Failed(rc, class, message);
        }
        let seen = looked(out);
        // SAFETY: the reference the lookup handed out, released once.
        unsafe { ffi::git_object_free(out) };
        seen
    }

    fn rust_looked(result: Result<ObjectArc<'_>, RepositoryError>) -> Looked {
        match result {
            Ok(object) => looked(object.as_c_ptr()),
            Err(e) => Looked::Failed(
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            ),
        }
    }

    /// The Rust getters' view of an entry, as [`view`] reads C's.
    fn rust_view(entry: &TreeEntryBox) -> View {
        let entry = entry.as_ref();
        View {
            name: entry.name().to_owned(),
            id: entry.id().to_hex_string().unwrap(),
            raw_mode: entry.filemode_raw(),
            mode: entry.filemode().raw(),
            kind: entry.kind().raw(),
        }
    }

    /// `testrepo.git` trees: commit `763d71a`'s (`README`, `ab`, two files), `ab`
    /// itself, and `HEAD`'s.
    const TREES: [&CStr; 3] = [
        TREE,
        c"f1425cef211cc08caa31e7b545ffb232acb098c3",
        c"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
    ];

    fn open_raw(path: &CStr) -> *mut ffi::git_repository {
        let mut repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path; no ceiling directories.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &raw mut repo,
                path.as_ptr(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        repo
    }

    /// A bare repository of each side's own, holding nothing.
    struct Empty(std::path::PathBuf);

    impl Empty {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-tree-entry-equiv-{}-{tag}",
                std::process::id()
            ));
            drop(std::fs::remove_dir_all(&path));
            Self(path)
        }

        fn c_path(&self) -> CString {
            CString::new(self.0.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for Empty {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_match_c() {
        init();
        let path = CString::new(TESTREPO).unwrap();
        let (c_empty_dir, rust_empty_dir) = (Empty::new("c"), Empty::new("rust"));

        // The C side: its repositories and trees, and the entries they lend.
        let c_repo = open_raw(&path);
        let mut c_empty = ptr::null_mut();
        let c_empty_path = c_empty_dir.c_path();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_init(&raw mut c_empty, c_empty_path.as_ptr(), 1) };
        assert_eq!(rc, 0);
        let mut c_trees = Vec::new();
        let mut c_entries = Vec::new();
        // Each entry's tree and name, which the Rust side looks up on its own.
        let mut paths: Vec<(&CStr, CString)> = Vec::new();
        for hex in TREES {
            let mut id = ffi::git_oid::default();
            let mut tree = ptr::null_mut();
            // SAFETY: writable slots, a hex id and the live repository; each entry lives
            // as long as its tree, which is freed after the last read below, and its name
            // is copied at once.
            unsafe {
                assert_eq!(
                    ffi::git_oid_from_string(&raw mut id, hex.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
            };
            // SAFETY: as above.
            unsafe {
                assert_eq!(
                    ffi::git_tree_lookup(&raw mut tree, c_repo, &raw const id),
                    0
                );
            };
            // SAFETY: as above.
            let count = unsafe { ffi::git_tree_entrycount(tree) };
            for i in 0..count {
                // SAFETY: as above; `i` is below the entry count.
                let entry = unsafe { ffi::git_tree_entry_byindex(tree, i) };
                // SAFETY: a live entry of the tree.
                let name = unsafe { ffi::git_tree_entry_name(entry) };
                // SAFETY: its name is NUL-terminated, copied at once.
                paths.push((hex, unsafe { CStr::from_ptr(name) }.to_owned()));
                c_entries.push(entry);
            }
            c_trees.push(tree);
        }

        // The Rust side: its repositories, and an owned entry for each of C's names.
        let rust_repo = Repository::open(&path).unwrap();
        let rust_empty = Repository::init(&rust_empty_dir.c_path(), true).unwrap();
        let rust_entries: Vec<TreeEntryBox> = paths
            .iter()
            .map(|(hex, name)| {
                let id = Oid::from_string(hex, OidType::Sha1).unwrap();
                let tree = Tree::lookup(rust_repo.as_ref(), id.as_ref()).unwrap();
                TreeEntry::bypath(tree.as_ref(), name).unwrap()
            })
            .collect();
        assert_eq!(c_entries.len(), rust_entries.len());
        assert!(c_entries.len() >= 10, "{}", c_entries.len());

        let compare = |c_entries: &[*const ffi::git_tree_entry],
                       c_repos: [*mut ffi::git_repository; 2],
                       rust_repos: [RepositoryRef<'_>; 2]| {
            for (i, (&c, rust)) in c_entries.iter().zip(&rust_entries).enumerate() {
                assert_eq!(rust_view(rust), view(c), "entry {i}");
                for (&c_other, rust_other) in c_entries.iter().zip(&rust_entries) {
                    let r = rust.as_ref().cmp_in_tree(rust_other.as_ref());
                    assert_eq!(r as c_int, cmp(c, c_other), "entry {i}");
                }
                for (&c_repo, rust_repo) in c_repos.iter().zip(rust_repos) {
                    let c_seen = c_to_object(c_repo, c);
                    let rust_seen = rust_looked(rust.as_ref().to_object(rust_repo));
                    assert_eq!(rust_seen, c_seen, "entry {i}");
                }
            }
        };
        compare(
            &c_entries,
            [c_repo, c_empty],
            [rust_repo.as_ref(), rust_empty.as_ref()],
        );

        // Each side keeps copies of its entries only, and opens the fixture afresh.
        let c_copies: Vec<_> = c_entries.iter().map(|&e| dup(e).cast_const()).collect();
        // SAFETY: the trees and repositories are released once, the trees first; the
        // copies hold nothing of them.
        unsafe {
            for t in c_trees {
                ffi::git_tree_free(t);
            }
        }
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_repo) };
        drop(rust_repo);
        let c_repo = open_raw(&path);
        let rust_repo = Repository::open(&path).unwrap();
        compare(
            &c_copies,
            [c_repo, c_empty],
            [rust_repo.as_ref(), rust_empty.as_ref()],
        );

        // SAFETY: each copy and repository is released once.
        unsafe {
            for e in c_copies {
                ffi::git_tree_entry_free(e.cast_mut());
            }
        }
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_repo) };
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_empty) };
    }
}

mod tree_object {
    //! Equivalence of `git2/tree.h`'s tree owner and constructors with the C routines
    //! they wrap.
    //!
    //! [`Tree::lookup`] must find what `git_tree_lookup` finds, and
    //! [`Tree::lookup_prefix`] what `git_tree_lookup_prefix` does: the same tree (its id
    //! and type, and every entry's name, raw file mode and id, as C reads them) or the
    //! same failure (code, error class and message). Each side opens the repository on
    //! its own and runs one script of lookups, in order: trees of `testrepo.git` at
    //! every depth, ids of commits, blobs and tags, missing and zero ids, prefixes of 0 to
    //! past 40 digits, ambiguous prefixes, and repeats. After every step the sides are
    //! compared, and so is which earlier result each new tree is the same object as (the
    //! cache hands back the tree it holds). Then each side copies every result
    //! (`git_tree_dup`; `clone`, [`TreeRef::to_tree`], and a hand-over to an object
    //! owner and back on the Rust side), releases the originals, and compares the copies,
    //! and the lookups after the releases. A repository without a git directory is
    //! compared the same way, and so is one holding trees libgit2 cannot parse beside
    //! well-formed ones.
    //!
    //! Documented divergence, outside these comparisons: an id of another type than the
    //! repository's is refused by the wrappers (code `-1`, `GIT_ERROR_INVALID`), where the
    //! C routines format it out of bounds; the unit tests of `tree.rs` hold the refusal.
    //!
    //! [`getters_match_c`] compares the tree getters with `git_tree_id`, `_owner`,
    //! `_entrycount` and `git_tree_entry_byindex`, `_byname` and `_byid`, on every tree
    //! of `testrepo.git`, with probes around both ends of the index range, names a byte
    //! longer or shorter than the tree's and past 65535 bytes, and ids of another type or
    //! tail: the entries found, the misses, and the thread's error after each call.
    //!
    //! [`TreeRef::to_tree`]: libgit2::libgit2::object::tree::TreeRef::to_tree

    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::tree::{Tree, TreeArc, TreeEntryRef, TreeRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{init, last_error};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
    );

    /// Trees of `testrepo.git`: `HEAD`'s, commit `763d71a`'s, its subtrees `ab`, `ab/c`,
    /// `ab/de` and `ab/de/fgh`, and the two trees starting `1810`.
    const TREES: [&str; 8] = [
        "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
        "ae90f12eea699729ed24555e40b9fd669da12a12",
        "f1425cef211cc08caa31e7b545ffb232acb098c3",
        "9a03079b8a8ee85a0bee58bf9be3da8b62414ed4",
        "b6361fc6a97178d8fc8639fdeed71c775ab52593",
        "3259a6bd5b57fb9c1281bb7ed3167b50f224cb54",
        "1810dff58d8a660512d4832e740f692884338ccd",
        "181037049a54a1eb5fab404658a3a250b44335d7",
    ];
    const COMMIT: &str = "763d71aadf09a7951596c9746c024e7eece7c7af";
    const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
    const MISSING: &str = "dededededededededededededededededededede";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    /// One entry of a tree, as C reads it: name, raw file mode, id.
    type Entry = (String, u32, String);

    /// What a lookup gave: the tree's id and type and its entries, as C reads them, or
    /// the failure.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Found(String, ffi::git_object_t, Vec<Entry>),
        Failed(c_int, c_int, String),
    }

    fn hex_of(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 65];
        // SAFETY: a live id, formatted into 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// C's view of a live tree.
    fn found(tree: *mut ffi::git_tree) -> Outcome {
        let object = tree.cast::<ffi::git_object>();
        {
            // SAFETY: `tree` is a live tree, a `git_object` of type tree.
            let count = unsafe { ffi::git_tree_entrycount(tree) };
            let entries = (0..count)
                .map(|i| {
                    // SAFETY: as above; `i` is below the entry count.
                    let e = unsafe { ffi::git_tree_entry_byindex(tree, i) };
                    // SAFETY: a live entry, whose name lives as long as it.
                    let name = unsafe { ffi::git_tree_entry_name(e) };
                    // SAFETY: as above, NUL-terminated and copied at once.
                    let name = unsafe { CStr::from_ptr(name) }
                        .to_string_lossy()
                        .into_owned();
                    // SAFETY: a live entry.
                    let mode = unsafe { ffi::git_tree_entry_filemode_raw(e) };
                    // SAFETY: a live entry, whose id lives as long as it and is copied at once.
                    (name, mode, hex_of(unsafe { ffi::git_tree_entry_id(e) }))
                })
                .collect();
            // SAFETY: as above.
            let object_id = unsafe { ffi::git_object_id(object) };
            // SAFETY: as above.
            unsafe { Outcome::Found(hex_of(object_id), ffi::git_object_type(object), entries) }
        }
    }

    /// One lookup of the script: an id (or prefix) in hex, and a length (`None` for the
    /// full lookup).
    #[derive(Clone, Copy, Debug)]
    struct Step<'a> {
        hex: &'a str,
        len: Option<usize>,
    }

    /// The C side: the raw repository and the trees it looked up.
    struct CSide {
        repo: *mut ffi::git_repository,
        held: Vec<*mut ffi::git_tree>,
    }

    impl CSide {
        fn open(path: &str) -> Self {
            let path = CString::new(path).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path; no ceiling directories.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &raw mut repo,
                    path.as_ptr(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        fn pathless() -> Self {
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot.
            assert_eq!(unsafe { ffi::git_repository_new(&raw mut repo) }, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        /// The step's outcome, and the index of the earlier result it is the same tree
        /// as; a found tree is held.
        fn run(&mut self, step: Step<'_>) -> (Outcome, Option<usize>) {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, and `hex.len()` readable bytes of hex digits.
            let rc = unsafe {
                ffi::git_oid_from_prefix(
                    &raw mut id,
                    step.hex.as_ptr().cast(),
                    step.hex.len(),
                    ffi::GIT_OID_SHA1,
                )
            };
            assert_eq!(rc, 0);
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live repository, and a live id of its type
            // read for the call.
            let rc = match step.len {
                // SAFETY: as above.
                None => unsafe { ffi::git_tree_lookup(&raw mut out, self.repo, &raw const id) },
                Some(len) => {
                    // SAFETY: as above.
                    unsafe {
                        ffi::git_tree_lookup_prefix(&raw mut out, self.repo, &raw const id, len)
                    }
                }
            };
            if rc < 0 {
                let (class, message) = last_error();
                return (Outcome::Failed(rc, class, message), None);
            }
            let same = self.held.iter().position(|&h| h == out);
            self.held.push(out);
            (found(out), same)
        }

        /// Copies every held tree (`git_tree_dup`), releases the originals, and views the
        /// copies.
        fn copy_and_release(&mut self) -> Vec<Outcome> {
            let copies: Vec<_> = self
                .held
                .iter()
                .map(|&t| {
                    let mut out = ptr::null_mut();
                    // SAFETY: `t` is a live reference this side holds.
                    assert_eq!(unsafe { ffi::git_tree_dup(&raw mut out, t) }, 0);
                    assert_eq!(out, t);
                    out
                })
                .collect();
            for t in std::mem::replace(&mut self.held, copies) {
                // SAFETY: each original reference is released once; its copy remains.
                unsafe { ffi::git_tree_free(t) };
            }
            self.held.iter().map(|&t| found(t)).collect()
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            // SAFETY: every held reference is released once, then the repository.
            unsafe {
                for &t in &self.held {
                    ffi::git_tree_free(t);
                }
            }
            // SAFETY: as above.
            unsafe { ffi::git_repository_free(self.repo) };
        }
    }

    /// C's view of what a wrapper gave.
    fn outcome(result: &Result<TreeArc<'_>, RepositoryError>) -> Outcome {
        match result {
            Ok(tree) => found(tree.as_c_ptr()),
            Err(e) => Outcome::Failed(
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            ),
        }
    }

    /// The Rust side's step: the outcome, and the earlier result it is the same tree as.
    fn run_rust<'r>(
        repo: &'r RepositoryBox,
        held: &mut Vec<TreeArc<'r>>,
        step: Step<'_>,
    ) -> (Outcome, Option<usize>) {
        let id = Oid::from_prefix(step.hex.as_bytes(), OidType::Sha1).unwrap();
        let result = match step.len {
            None => Tree::lookup(repo.as_ref(), id.as_ref()),
            Some(len) => Tree::lookup_prefix(repo.as_ref(), id.as_ref(), len),
        };
        let seen = outcome(&result);
        let Ok(tree) = result else {
            return (seen, None);
        };
        let same = held.iter().position(|h| TreeArc::ptr_eq(h, &tree));
        held.push(tree);
        (seen, same)
    }

    fn script() -> Vec<Step<'static>> {
        let mut steps = Vec::new();
        for hex in TREES.into_iter().chain([COMMIT, BLOB, TAG, MISSING, ZERO]) {
            steps.push(Step { hex, len: None });
            steps.push(Step { hex, len: Some(9) });
        }
        for len in [0, 1, 3, 4, 5, 7, 8, 20, 39, 40, 41, 64, usize::MAX] {
            steps.push(Step {
                hex: TREES[1],
                len: Some(len),
            });
        }
        // Two trees start `1810`, a commit and a blob `3d35`; the others start one tree.
        for hex in ["1810", "3d35", "1810d", "18103", "ae90f", "944c", "ffff"] {
            steps.push(Step {
                hex,
                len: Some(hex.len()),
            });
        }
        steps.push(Step {
            hex: TREES[1],
            len: None,
        });
        steps
    }

    /// Runs `script` on both sides, comparing after each step and after the copies; the
    /// number of trees the script found.
    fn compare(mut c: CSide, rust: &RepositoryBox, script: &[Step<'_>]) -> usize {
        let mut held = Vec::new();
        for &step in script {
            assert_eq!(c.run(step), run_rust(rust, &mut held, step), "{step:?}");
        }
        let c_copies = c.copy_and_release();
        let copies: Vec<_> = held
            .iter()
            .enumerate()
            .map(|(i, t)| match i % 3 {
                0 => t.clone(),
                1 => t.as_ref().to_tree(),
                _ => Tree::from_object(Tree::into_object(t.clone())).unwrap(),
            })
            .collect();
        for (copy, original) in copies.iter().zip(&held) {
            assert!(TreeArc::ptr_eq(copy, original));
        }
        drop(held);
        let rust_copies: Vec<_> = copies.iter().map(|t| found(t.as_c_ptr())).collect();
        assert_eq!(c_copies, rust_copies);
        let found = copies.len();

        // The same lookups again, with only the copies alive.
        let mut held = copies;
        for &step in script {
            assert_eq!(
                c.run(step),
                run_rust(rust, &mut held, step),
                "again: {step:?}"
            );
        }
        found
    }

    #[test]
    fn lookups_match_c() {
        init();
        let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        // Each tree twice, ten prefix lengths of one, and four unique prefixes, and the
        // repeat.
        assert_eq!(compare(CSide::open(TESTREPO), &rust, &script()), 31);
    }

    #[test]
    fn lookups_in_a_repository_without_paths_match_c() {
        init();
        let rust = Repository::new().unwrap();
        let steps = [
            Step {
                hex: TREES[1],
                len: None,
            },
            Step {
                hex: TREES[1],
                len: Some(7),
            },
            Step {
                hex: TREES[1],
                len: Some(3),
            },
        ];
        assert_eq!(compare(CSide::pathless(), &rust, &steps), 0);
    }

    /// A repository holding trees libgit2 cannot parse, beside well-formed ones (the
    /// empty tree, and one entry): each lookup fails, or succeeds, alike.
    #[test]
    fn lookups_of_unparsable_trees_match_c() {
        init();
        let path = std::env::temp_dir().join(format!("crustify-tree-equiv-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        let path_str = path.to_str().unwrap().to_owned();
        let leaf: Vec<u8> = b"100644 leaf\0".iter().copied().chain([0x5A; 20]).collect();
        let objects: Vec<(Vec<u8>, ffi::git_object_t)> = vec![
            (b"100644 one\0short".to_vec(), ffi::GIT_OBJECT_TREE),
            (
                b"100644 a\0aaaaaaaaaaaaaaaaaaaa100644 b".to_vec(),
                ffi::GIT_OBJECT_TREE,
            ),
            (
                b"10x644 a\0aaaaaaaaaaaaaaaaaaaa".to_vec(),
                ffi::GIT_OBJECT_TREE,
            ),
            (
                b"100644a\0aaaaaaaaaaaaaaaaaaaa".to_vec(),
                ffi::GIT_OBJECT_TREE,
            ),
            (
                b"100644 \0aaaaaaaaaaaaaaaaaaaa".to_vec(),
                ffi::GIT_OBJECT_TREE,
            ),
            (b"100644 no-nul".to_vec(), ffi::GIT_OBJECT_TREE),
            (Vec::new(), ffi::GIT_OBJECT_TREE),
            (leaf, ffi::GIT_OBJECT_TREE),
            (b"fine\n".to_vec(), ffi::GIT_OBJECT_BLOB),
        ];
        let ids: Vec<String> = {
            let repo = Repository::init(&CString::new(path_str.as_str()).unwrap(), true).unwrap();
            let odb = repo.as_ref().odb().unwrap();
            objects
                .iter()
                .map(|(data, kind)| {
                    let mut id = ffi::git_oid::default();
                    // SAFETY: a live odb, a writable id, `data` read for the call.
                    let rc = unsafe {
                        ffi::git_odb_write(
                            &raw mut id,
                            odb.as_c_ptr(),
                            data.as_ptr().cast(),
                            data.len(),
                            *kind,
                        )
                    };
                    assert_eq!(rc, 0);
                    hex_of(&raw const id)
                })
                .collect()
        };
        let mut steps = Vec::new();
        for hex in &ids {
            steps.push(Step { hex, len: None });
            steps.push(Step { hex, len: Some(12) });
        }
        let rust = Repository::open(&CString::new(path_str.as_str()).unwrap()).unwrap();
        assert_eq!(compare(CSide::open(&path_str), &rust, &steps), 4);
        drop(rust);
        drop(std::fs::remove_dir_all(&path));
    }

    /// What a getter gave: the entry as the side's own getters read it (name, raw file
    /// mode, id), or nothing, and the thread's error right after the call.
    type Got = (Option<Entry>, (c_int, String));

    /// Everything the tree getters give for one tree, in call order.
    #[derive(Debug, PartialEq, Eq)]
    struct Read {
        id: String,
        owner: Option<String>,
        len: usize,
        in_order: Vec<Entry>,
        by_index: Vec<Got>,
        by_name: Vec<Got>,
        by_id: Vec<Got>,
    }

    /// Indices around both ends of `len`.
    fn indices(len: usize) -> Vec<usize> {
        vec![0, 1, len.saturating_sub(1), len, len + 1, usize::MAX]
    }

    /// The names probed: every entry's, with a byte more and one less, and names no
    /// tree holds, past the 65535 bytes a tree stores included.
    fn name_probes(names: &[String]) -> Vec<CString> {
        let mut probes: Vec<CString> = Vec::new();
        for name in names {
            probes.push(CString::new(name.as_str()).unwrap());
            probes.push(CString::new(format!("{name}x")).unwrap());
            probes.push(CString::new(&name.as_bytes()[..name.len() - 1]).unwrap());
            probes.push(CString::new(format!("{name}/")).unwrap());
        }
        for extra in ["", "nope", "ab/de", "zzzz"] {
            probes.push(CString::new(extra).unwrap());
        }
        for len in [65535, 65536, 65537, 70000] {
            probes.push(CString::new(vec![b'a'; len]).unwrap());
        }
        probes
    }

    /// The ids probed, as raw `git_oid`s: every entry's, with a non-zero tail and as
    /// SHA-256, the zero id and a missing one.
    fn id_probes(ids: &[ffi::git_oid]) -> Vec<ffi::git_oid> {
        let mut probes = Vec::new();
        for id in ids {
            probes.push(*id);
            let mut tail = *id;
            tail.id[31] = 1;
            probes.push(tail);
            let mut wide = *id;
            wide.type_ = u8::try_from(ffi::GIT_OID_SHA256).unwrap();
            probes.push(wide);
        }
        let mut missing = ffi::git_oid {
            type_: u8::try_from(ffi::GIT_OID_SHA1).unwrap(),
            id: [0; 32],
        };
        probes.push(missing);
        missing.id[..20].fill(0xDE);
        probes.push(missing);
        probes
    }

    /// C's view of the entry `e`, or nothing.
    fn c_entry(e: *const ffi::git_tree_entry) -> Option<Entry> {
        // SAFETY: NULL or a live entry of a live tree; its name and id are copied at once.
        unsafe { e.as_ref()? };
        // SAFETY: as above.
        let tree_entry_name = unsafe { ffi::git_tree_entry_name(e) };
        // SAFETY: as above.
        let cstr = unsafe { CStr::from_ptr(tree_entry_name) };
        // SAFETY: as above.
        let tree_entry_filemode_raw = unsafe { ffi::git_tree_entry_filemode_raw(e) };
        // SAFETY: as above.
        unsafe {
            Some((
                cstr.to_string_lossy().into_owned(),
                tree_entry_filemode_raw,
                hex_of(ffi::git_tree_entry_id(e)),
            ))
        }
    }

    /// The Rust getters' view of a lent entry, or nothing.
    fn rust_entry(e: Option<TreeEntryRef<'_>>) -> Option<Entry> {
        let e = e?;
        Some((
            e.name().to_string_lossy().into_owned(),
            e.filemode_raw(),
            e.id().to_hex_string().unwrap(),
        ))
    }

    /// Runs `call` with the thread's error cleared, and pairs its result with the error
    /// it left.
    fn got(call: impl FnOnce() -> Option<Entry>) -> Got {
        // SAFETY: no arguments.
        unsafe { ffi::git_error_clear() };
        let entry = call();
        (entry, last_error())
    }

    /// The C side's reads of a live tree, with the probes.
    fn c_read(tree: *mut ffi::git_tree, names: &[CString], ids: &[ffi::git_oid]) -> Read {
        {
            // SAFETY: a live tree of a live repository, only read; every returned entry and
            // id lives with it and is copied at once; the probes are live for each call.
            let len = unsafe { ffi::git_tree_entrycount(tree) };
            // SAFETY: as above.
            let tree_owner = unsafe { ffi::git_tree_owner(tree) };
            // SAFETY: as above.
            let path = unsafe { ffi::git_repository_path(tree_owner) };
            // SAFETY: as above.

            Read {
                // SAFETY: as above.
                id: hex_of(unsafe { ffi::git_tree_id(tree) }),
                // SAFETY: as above.
                owner: unsafe { path.as_ref() }.map(|_| {
                    // SAFETY: as above.
                    unsafe { CStr::from_ptr(path) }
                        .to_string_lossy()
                        .into_owned()
                }),
                len,
                in_order: (0..len)
                    // SAFETY: as above.
                    .map(|i| c_entry(unsafe { ffi::git_tree_entry_byindex(tree, i) }).unwrap())
                    .collect(),
                by_index: indices(len)
                    .into_iter()
                    // SAFETY: as above.
                    .map(|i| got(|| c_entry(unsafe { ffi::git_tree_entry_byindex(tree, i) })))
                    .collect(),
                by_name: names
                    .iter()
                    .map(|n| {
                        // SAFETY: as above.
                        got(|| c_entry(unsafe { ffi::git_tree_entry_byname(tree, n.as_ptr()) }))
                    })
                    .collect(),
                by_id: ids
                    .iter()
                    // SAFETY: as above.
                    .map(|id| got(|| c_entry(unsafe { ffi::git_tree_entry_byid(tree, id) })))
                    .collect(),
            }
        }
    }

    /// The Rust side's reads of a tree, through the public getters, with the probes.
    fn rust_read(tree: TreeRef<'_, '_>, names: &[CString], ids: &[ffi::git_oid]) -> Read {
        let ids: Vec<Oid> = ids
            .iter()
            .map(|raw| {
                let mut id = Oid::zero(OidType::Sha1);
                id.as_mut().set_raw_kind(raw.type_);
                id.as_mut().set_id(raw.id);
                id
            })
            .collect();
        Read {
            id: tree.id().to_hex_string().unwrap(),
            owner: tree
                .owner()
                .path()
                .map(|p| p.to_string_lossy().into_owned()),
            len: tree.len(),
            in_order: tree.iter().map(|e| rust_entry(Some(e)).unwrap()).collect(),
            by_index: indices(tree.len())
                .into_iter()
                .map(|i| got(|| rust_entry(tree.get(i))))
                .collect(),
            by_name: names
                .iter()
                .map(|n| got(|| rust_entry(tree.get_by_name(n))))
                .collect(),
            by_id: ids
                .iter()
                .map(|id| got(|| rust_entry(tree.get_by_id(id.as_ref()))))
                .collect(),
        }
    }

    /// The tree getters ([`TreeRef::id`], `owner`, `len`, `get`, `get_by_name`,
    /// `get_by_id`, `iter`) read what `git_tree_id`, `_owner`, `_entrycount`,
    /// `_entry_byindex`, `_entry_byname` and `_entry_byid` read, on every tree of
    /// `testrepo.git`, each side over its own repository: the same entries, the same
    /// misses, and the same thread error after each call (the "path too long" error an
    /// overlong name leaves included). Then again with only a copy of each tree alive.
    ///
    /// [`TreeRef::id`]: libgit2::libgit2::object::tree::TreeRef::id
    #[test]
    fn getters_match_c() {
        init();
        let mut c = CSide::open(TESTREPO);
        let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        let mut held = Vec::new();
        let mut probes = Vec::new();
        for hex in TREES {
            let step = Step { hex, len: None };
            let (Outcome::Found(_, _, entries), _) = c.run(step) else {
                panic!("{hex} is a tree");
            };
            let names: Vec<_> = entries.iter().map(|e| e.0.clone()).collect();
            let ids: Vec<_> = c
                .held
                .last()
                .map(|&t| {
                    (0..entries.len())
                        .map(|i| {
                            // SAFETY: a live tree and an index below its entry count.
                            let entry = unsafe { ffi::git_tree_entry_byindex(t, i) };
                            // SAFETY: a live entry of the tree.
                            let id = unsafe { ffi::git_tree_entry_id(entry) };
                            // SAFETY: the id lives with the entry; it is copied.
                            unsafe { *id }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap();
            let (names, ids) = (name_probes(&names), id_probes(&ids));
            let tree = run_rust(&rust, &mut held, step);
            assert!(tree.1.is_none());
            let tree = held.last().unwrap();
            assert_eq!(
                c_read(*c.held.last().unwrap(), &names, &ids),
                rust_read(tree.as_ref(), &names, &ids),
                "{hex}"
            );
            probes.push((names, ids));
        }
        c.copy_and_release();
        let copies: Vec<_> = held.iter().map(TreeArc::clone).collect();
        drop(held);
        for ((&t, copy), (names, ids)) in c.held.iter().zip(&copies).zip(&probes) {
            assert_eq!(c_read(t, names, ids), rust_read(copy.as_ref(), names, ids));
        }
    }
}

mod tree_updates {
    //! Equivalence of [`Tree::create_updated`] with `git_tree_create_updated`.
    //!
    //! Each side works on a copy of `testrepo.git` of its own and runs one script of
    //! update sets, each applied to no tree, to commit `763d71a`'s tree (`README`, `ab/`
    //! three levels deep, two more blobs), or to the tree the previous step wrote:
    //! upserts of new, nested and replaced entries of every mode, removals of leaves,
    //! of whole subtrees and of the last entries of trees (which drops them), unsorted
    //! sets, and every refusal the routine and its tree builders make (duplicate paths,
    //! D/F conflicts, replacements of a type with another, missing removals, invalid
    //! names, modes and ids, missing objects). After every step the sides compare the
    //! id written, or the code, error class and message, and, for a written tree, its
    //! every entry, recursively, as C reads each side's repository.
    //!
    //! Documented divergences, outside these comparisons, held by `tree.rs`'s unit
    //! tests: a baseline of another id type than the repository's and an upsert of an
    //! id of another type that libgit2 would look up are refused before C, which would
    //! format them out of bounds; an upsert path component of more than 65535 bytes is
    //! refused before C, which writes it into a corrupt tree.

    use std::ffi::{CStr, CString, c_int};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::FileMode;
    use libgit2::libgit2::object::tree::{Tree, TreeUpdate, UpdateAction};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    use super::{init, last_error};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
    );
    /// Commit `763d71a`'s tree.
    const BASE: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
    /// `ab/`, a tree of `BASE`.
    const AB: &str = "f1425cef211cc08caa31e7b545ffb232acb098c3";
    /// `BASE`'s `README`, and another blob.
    const README: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const OTHER: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    const COMMIT: &str = "763d71aadf09a7951596c9746c024e7eece7c7af";
    const MISSING: &str = "dededededededededededededededededededede";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    /// The tree a step updates.
    #[derive(Clone, Copy, Debug)]
    enum Base {
        None,
        Tree(&'static str),
        /// The tree the previous step wrote.
        Previous,
    }

    /// One update: its action, path, id (hex) and mode.
    type Update = (UpdateAction, &'static str, &'static str, FileMode);

    fn up(path: &'static str, id: &'static str, mode: FileMode) -> Update {
        (UpdateAction::Upsert, path, id, mode)
    }

    fn rm(path: &'static str) -> Update {
        (UpdateAction::Remove, path, ZERO, FileMode::Unreadable)
    }

    fn script() -> Vec<(Base, Vec<Update>)> {
        use FileMode::*;
        let base = Base::Tree(BASE);
        vec![
            (Base::None, vec![]),
            (base, vec![]),
            (base, vec![up("README2", README, Blob)]),
            (base, vec![up("README", OTHER, Blob)]),
            (base, vec![up("README", README, BlobExecutable)]),
            (base, vec![up("a/b/c.txt", README, Blob)]),
            (
                base,
                vec![
                    up("ab/de/fgh/new.txt", OTHER, BlobExecutable),
                    up("ab/c/link", OTHER, Link),
                ],
            ),
            (base, vec![rm("README")]),
            (base, vec![rm("ab/de/fgh/1.txt")]),
            (
                base,
                vec![
                    rm("ab/c/3.txt"),
                    rm("ab/4.txt"),
                    rm("ab/de/2.txt"),
                    rm("ab/de/fgh/1.txt"),
                ],
            ),
            (base, vec![rm("ab")]),
            (base, vec![rm("missing.txt")]),
            (base, vec![rm("ab/zz/missing")]),
            (base, vec![up("x", OTHER, Blob), up("x", README, Blob)]),
            (base, vec![rm("x"), up("x", README, Blob)]),
            (base, vec![up("README/x", OTHER, Blob)]),
            (base, vec![up("ab/4.txt/x", OTHER, Blob)]),
            (base, vec![rm("README"), up("README/x", OTHER, Blob)]),
            (base, vec![up("ab", OTHER, Blob)]),
            (base, vec![up("ab/de", OTHER, Blob)]),
            (base, vec![up("README", AB, Tree)]),
            (base, vec![up("new", MISSING, Blob)]),
            (base, vec![up("new", ZERO, Blob)]),
            (base, vec![up("new", AB, Blob)]),
            (base, vec![up("new", OTHER, Tree)]),
            (base, vec![up("README", OTHER, Unreadable)]),
            (base, vec![up(".git", OTHER, Blob)]),
            (base, vec![up("a//b", OTHER, Blob)]),
            (base, vec![up("../x", OTHER, Blob)]),
            (base, vec![up("/lead", OTHER, Blob)]),
            (base, vec![up("dir/", OTHER, Blob)]),
            (base, vec![up("dir/./x", OTHER, Blob)]),
            (base, vec![up("sub", MISSING, Commit)]),
            (base, vec![up("deps/sub", COMMIT, Commit)]),
            (
                base,
                vec![up("tree", AB, Tree), up("tree2/inner", AB, Tree)],
            ),
            (
                base,
                vec![
                    up("z/1", README, Blob),
                    up("a/1", OTHER, Blob),
                    rm("README"),
                    up("m/n/o", README, Link),
                    up("ab/c/9", OTHER, Blob),
                    rm("ab/de/2.txt"),
                    up("ab/de/fgh/0", README, Blob),
                    up("a/0", README, Blob),
                ],
            ),
            (Base::None, vec![up("deep/a/b/c/d/e/f.txt", OTHER, Blob)]),
            (Base::None, vec![rm("x")]),
            (
                Base::Tree(AB),
                vec![up("c/3.txt", OTHER, Blob), rm("4.txt")],
            ),
            // A sequence, each step over the tree the one before wrote.
            (
                base,
                vec![up("seq/a", README, Blob), up("seq/b", OTHER, Blob)],
            ),
            (Base::Previous, vec![rm("seq/a")]),
            (Base::Previous, vec![up("seq/b", README, BlobExecutable)]),
            (Base::Previous, vec![rm("seq/a")]),
            (Base::Previous, vec![rm("seq/b")]),
            (Base::Previous, vec![up("README", OTHER, Blob), rm("ab")]),
        ]
    }

    /// One entry of a written tree, as C reads it: path, raw mode and id.
    type Entry = (String, u32, String);

    /// What a step gave.
    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Written(String, Vec<Entry>),
        Failed(c_int, c_int, String),
    }

    fn hex(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 65];
        // SAFETY: a live id, formatted into 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn raw_id(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable id and a NUL-terminated hex string.
        let rc = unsafe { ffi::git_oid_from_string(&raw mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    /// Every entry of the tree `id` in `repo`, recursively, as C reads them.
    fn entries(repo: *mut ffi::git_repository, id: &ffi::git_oid, prefix: &str) -> Vec<Entry> {
        let mut tree = ptr::null_mut();
        {
            // SAFETY: a writable slot, a live repository and id; the tree is read, then
            // released once.
            unsafe {
                assert_eq!(
                    ffi::git_tree_lookup(&raw mut tree, repo, id),
                    0,
                    "{:?}",
                    last_error()
                );
            };
            let mut out = Vec::new();
            // SAFETY: as above.
            let count = unsafe { ffi::git_tree_entrycount(tree) };
            for i in 0..count {
                // SAFETY: as above; `i` is below the entry count.
                let entry = unsafe { ffi::git_tree_entry_byindex(tree, i) };
                // SAFETY: a live entry of the tree.
                let name = unsafe { ffi::git_tree_entry_name(entry) };
                // SAFETY: its name is NUL-terminated and lives with it.
                let name = unsafe { CStr::from_ptr(name) };
                let path = format!("{prefix}{}", name.to_str().unwrap());
                // SAFETY: a live entry.
                let mode = unsafe { ffi::git_tree_entry_filemode_raw(entry) };
                // SAFETY: as above.
                let id = unsafe { ffi::git_tree_entry_id(entry) };
                // SAFETY: the id lives with the entry; it is copied.
                let entry_id = unsafe { *id };
                out.push((path.clone(), mode, hex(&raw const entry_id)));
                if mode == ffi::GIT_FILEMODE_TREE {
                    out.extend(entries(repo, &entry_id, &format!("{path}/")));
                }
            }
            // SAFETY: as above.
            unsafe { ffi::git_tree_free(tree) };
            out
        }
    }

    fn written(repo: *mut ffi::git_repository, id: &ffi::git_oid) -> Outcome {
        Outcome::Written(hex(id), entries(repo, id, ""))
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// A private copy of `testrepo.git`, removed on drop.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-tree-update-equiv-{}-{tag}",
                std::process::id()
            ));
            drop(std::fs::remove_dir_all(&path));
            copy_dir(Path::new(TESTREPO), &path);
            Self(path)
        }

        fn c_path(&self) -> CString {
            CString::new(self.0.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    /// `git_tree_create_updated` over `base` (looked up, then released) and `updates`.
    fn c_step(
        repo: *mut ffi::git_repository,
        base: Option<&ffi::git_oid>,
        updates: &[Update],
    ) -> (Outcome, Option<ffi::git_oid>) {
        let paths: Vec<CString> = updates.iter().map(|u| CString::new(u.1).unwrap()).collect();
        let raw: Vec<ffi::git_tree_update> = updates
            .iter()
            .zip(&paths)
            .map(|(&(action, _, id, mode), path)| ffi::git_tree_update {
                action: action.raw(),
                id: raw_id(id),
                filemode: mode.raw(),
                path: path.as_ptr(),
            })
            .collect();
        let mut tree = ptr::null_mut();
        if let Some(base) = base {
            // SAFETY: a writable slot, a live repository and id.
            let ret = unsafe { ffi::git_tree_lookup(&raw mut tree, repo, base) };
            assert_eq!(ret, 0);
        }
        let mut out = ffi::git_oid::default();
        let rc = {
            // SAFETY: a writable id; a live repository; NULL or a live tree, released once
            // after; a live array of `raw.len()` updates whose paths outlive the call.
            let rc = unsafe {
                ffi::git_tree_create_updated(&raw mut out, repo, tree, raw.len(), raw.as_ptr())
            };
            // SAFETY: as above.
            unsafe { ffi::git_tree_free(tree) };
            rc
        };
        if rc < 0 {
            let (class, message) = last_error();
            return (Outcome::Failed(rc, class, message), None);
        }
        (written(repo, &out), Some(out))
    }

    /// [`Tree::create_updated`] over `base` and `updates`.
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the closure lets each handle reborrow for its own lifetime, which the method path cannot"
    )]
    fn rust_step(
        repo: &RepositoryBox,
        base: Option<&Oid>,
        updates: &[Update],
    ) -> (Outcome, Option<Oid>) {
        let ids: Vec<Oid> = updates
            .iter()
            .map(|u| Oid::from_string(&CString::new(u.2).unwrap(), OidType::Sha1).unwrap())
            .collect();
        let paths: Vec<CString> = updates.iter().map(|u| CString::new(u.1).unwrap()).collect();
        let built: Vec<TreeUpdate<'_>> = updates
            .iter()
            .zip(&paths)
            .zip(&ids)
            .map(|((&(action, _, _, mode), path), id)| {
                let mut update = TreeUpdate::upsert(path, id.as_ref(), mode);
                update.as_mut().set_action(action);
                update
            })
            .collect();
        let tree = base.map(|id| Tree::lookup(repo.as_ref(), id.as_ref()).unwrap());
        let result = Tree::create_updated(repo.as_ref(), tree.as_ref().map(|x| x.as_ref()), &built);
        drop(tree);
        match result {
            Ok(id) => {
                let raw = ffi::git_oid {
                    type_: id.as_ref().raw_kind(),
                    id: id.as_ref().id(),
                };
                (written(repo.as_c_ptr(), &raw), Some(id))
            }
            Err(e) => (
                Outcome::Failed(
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                ),
                None,
            ),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn scripts_match_c() {
        init();
        let (c_box, rust_box) = (Sandbox::new("c"), Sandbox::new("rust"));
        let mut c_repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_open(&raw mut c_repo, c_box.c_path().as_ptr()) };
        assert_eq!(rc, 0);
        let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
        let (mut c_previous, mut rust_previous) = (None, None);
        let (mut failures, mut writes) = (0, 0);
        for (i, (base, updates)) in script().iter().enumerate() {
            let (c_base, rust_base) = match base {
                Base::None => (None, None),
                Base::Tree(hex) => (
                    Some(raw_id(hex)),
                    Some(Oid::from_string(&CString::new(*hex).unwrap(), OidType::Sha1).unwrap()),
                ),
                Base::Previous => (c_previous, rust_previous),
            };
            let (c_seen, c_out) = c_step(c_repo, c_base.as_ref(), updates);
            let (rust_seen, rust_out) = rust_step(&rust_repo, rust_base.as_ref(), updates);
            assert_eq!(rust_seen, c_seen, "step {i}: {base:?} {updates:?}");
            match c_seen {
                Outcome::Failed(..) => failures += 1,
                Outcome::Written(..) => writes += 1,
            }
            if c_out.is_some() {
                (c_previous, rust_previous) = (c_out, rust_out);
            }
        }
        assert!(failures > 15 && writes > 15, "{failures} {writes}");
        // SAFETY: the raw repository is released once; nothing of it is held.
        unsafe { ffi::git_repository_free(c_repo) };
    }

    /// Makes the loose fan-out directory `hex` would be written under a regular file,
    /// so that writing that object fails (permissions do not stop root); `false` when
    /// the directory already holds objects, which the test then avoids.
    fn block(sandbox: &Sandbox, hex: &str) -> bool {
        let dir = sandbox.0.join("objects").join(&hex[..2]);
        if dir.exists() {
            return false;
        }
        std::fs::write(dir, b"not a directory").unwrap();
        true
    }

    /// The tree a write fails to store, the root or a subtree, fails both sides alike,
    /// with what the step built released (`LSan`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    fn unwritable_objects_match_c() {
        init();
        let scratch = Sandbox::new("unwritable-scratch");
        let scratch_repo = Repository::open(&scratch.c_path()).unwrap();
        // The subtree `a` of a lone `a/<name>`, and the root of a lone `<name>`, under
        // a name whose tree lands in a fan-out the repository does not use yet.
        let mut cases: Vec<(&'static str, Vec<Update>, String)> = Vec::new();
        const SUBTREE_PATHS: [&str; 8] = [
            "a/f0", "a/f1", "a/f2", "a/f3", "a/f4", "a/f5", "a/f6", "a/f7",
        ];
        const ROOT_PATHS: [&str; 8] = ["f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7"];
        for (leaf, candidates) in [(false, SUBTREE_PATHS), (true, ROOT_PATHS)] {
            for path in candidates {
                let updates = vec![up(path, README, FileMode::Blob)];
                let (Outcome::Written(root, entries), _) = rust_step(&scratch_repo, None, &updates)
                else {
                    panic!("{path}")
                };
                let blocked = if leaf { root } else { entries[0].2.clone() };
                let fanout = scratch.0.join("objects").join(&blocked[..2]);
                let fresh = !Path::new(TESTREPO)
                    .join("objects")
                    .join(&blocked[..2])
                    .exists();
                if fresh && fanout.is_dir() {
                    cases.push((path, updates, blocked));
                    break;
                }
            }
        }
        assert_eq!(cases.len(), 2);
        for (path, updates, blocked) in cases {
            let (c_box, rust_box) = (
                Sandbox::new("unwritable-c"),
                Sandbox::new("unwritable-rust"),
            );
            assert!(block(&c_box, &blocked) && block(&rust_box, &blocked));
            let mut c_repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path.
            let rc = unsafe { ffi::git_repository_open(&raw mut c_repo, c_box.c_path().as_ptr()) };
            assert_eq!(rc, 0);
            let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
            // The message names the side's own fan-out directory.
            let anonymous = |outcome: Outcome, sandbox: &Sandbox| match outcome {
                Outcome::Failed(rc, class, message) => {
                    let message = message.replace(sandbox.0.to_str().unwrap(), "<repo>");
                    Outcome::Failed(rc, class, message)
                }
                written @ Outcome::Written(..) => written,
            };
            let c_seen = anonymous(c_step(c_repo, None, &updates).0, &c_box);
            let rust_seen = anonymous(rust_step(&rust_repo, None, &updates).0, &rust_box);
            assert!(matches!(c_seen, Outcome::Failed(..)), "{path}: {c_seen:?}");
            assert_eq!(rust_seen, c_seen, "{path}");
            drop(rust_repo);
            // SAFETY: the raw repository is released once; nothing of it is held.
            unsafe { ffi::git_repository_free(c_repo) };
        }
    }

    /// A baseline of another repository of the same id type: its entries are copied,
    /// and the subtrees an update descends into are looked up in the repository given,
    /// which lacks them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn baselines_of_other_repositories_match_c() {
        init();
        let root = std::env::temp_dir().join(format!(
            "crustify-tree-update-equiv-{}-foreign-base",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&root));
        let open_empty = |side: &str| {
            let path = CString::new(root.join(side).to_str().unwrap()).unwrap();
            drop(Repository::init(&path, true).unwrap());
            path
        };
        let (c_path, rust_path) = (open_empty("c"), open_empty("rust"));
        let mut c_repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_open(&raw mut c_repo, c_path.as_ptr()) };
        assert_eq!(rc, 0);
        let rust_repo = Repository::open(&rust_path).unwrap();
        let mut c_other = ptr::null_mut();
        let other_path = CString::new(TESTREPO).unwrap();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_open(&raw mut c_other, other_path.as_ptr()) };
        assert_eq!(rc, 0);
        let rust_other = Repository::open(&other_path).unwrap();
        let base = raw_id(BASE);
        let rust_base = Oid::from_string(&CString::new(BASE).unwrap(), OidType::Sha1).unwrap();
        for updates in [
            vec![up("ab/c/new", README, FileMode::Commit)],
            vec![rm("README")],
            vec![up("sub", MISSING, FileMode::Commit)],
            vec![],
        ] {
            let mut c_tree = ptr::null_mut();
            // SAFETY: a writable slot, the live other repository and a live id.
            let rc = unsafe { ffi::git_tree_lookup(&raw mut c_tree, c_other, &raw const base) };
            assert_eq!(rc, 0);
            let paths: Vec<CString> = updates.iter().map(|u| CString::new(u.1).unwrap()).collect();
            let raw: Vec<ffi::git_tree_update> = updates
                .iter()
                .zip(&paths)
                .map(|(&(action, _, id, mode), path)| ffi::git_tree_update {
                    action: action.raw(),
                    id: raw_id(id),
                    filemode: mode.raw(),
                    path: path.as_ptr(),
                })
                .collect();
            let mut out = ffi::git_oid::default();
            let rc = {
                // SAFETY: a writable id, the live repository, a live tree of another live
                // repository, released once after, and a live array whose paths outlive it.
                let rc = unsafe {
                    ffi::git_tree_create_updated(
                        &raw mut out,
                        c_repo,
                        c_tree,
                        raw.len(),
                        raw.as_ptr(),
                    )
                };
                // SAFETY: as above.
                unsafe { ffi::git_tree_free(c_tree) };
                rc
            };
            let c_seen = if rc < 0 {
                let (class, message) = last_error();
                Outcome::Failed(rc, class, message)
            } else {
                Outcome::Written(hex(&raw const out), Vec::new())
            };
            let tree = Tree::lookup(rust_other.as_ref(), rust_base.as_ref()).unwrap();
            let ids: Vec<Oid> = updates
                .iter()
                .map(|u| Oid::from_string(&CString::new(u.2).unwrap(), OidType::Sha1).unwrap())
                .collect();
            let built: Vec<TreeUpdate<'_>> = updates
                .iter()
                .zip(&paths)
                .zip(&ids)
                .map(|((&(action, _, _, mode), path), id)| {
                    let mut update = TreeUpdate::upsert(path, id.as_ref(), mode);
                    update.as_mut().set_action(action);
                    update
                })
                .collect();
            let result = Tree::create_updated(rust_repo.as_ref(), Some(tree.as_ref()), &built);
            let rust_seen = match result {
                Ok(id) => Outcome::Written(id.as_ref().to_hex_string().unwrap(), Vec::new()),
                Err(e) => Outcome::Failed(
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                ),
            };
            assert_eq!(rust_seen, c_seen, "{updates:?}");
        }
        drop((rust_repo, rust_other));
        // SAFETY: both raw repositories are released once; nothing of them is held.
        unsafe { ffi::git_repository_free(c_repo) };
        // SAFETY: as above.
        unsafe { ffi::git_repository_free(c_other) };
        drop(std::fs::remove_dir_all(&root));
    }
}
