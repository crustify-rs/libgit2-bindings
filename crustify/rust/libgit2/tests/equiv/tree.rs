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

use std::collections::HashSet;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

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
            ffi::GIT_ERROR_CALLBACK as c_int,
            format!("git_tree_walk callback returned {rc}"),
        )
    } else {
        (ffi::GIT_ERROR_NONE as c_int, "no error".to_owned())
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
    // SAFETY: the runtime is initialised; `git_error_last` never returns NULL then,
    // and its message is a NUL-terminated string live until the next libgit2 call on
    // this thread, after the copy.
    unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
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
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot and `c_path` NUL-terminated; `1` asks for a
        // bare repository.
        let rc = unsafe { ffi::git_repository_init(&mut repo, c_path.as_ptr(), 1) };
        assert_eq!(rc, 0);
        let id = write_tree(repo, TREE);
        let mut tree = ptr::null_mut();
        // SAFETY: `tree` is a live slot, `repo` a live repository and `id` a live oid.
        let rc = unsafe { ffi::git_tree_lookup(&mut tree, repo, &id) };
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
        unsafe {
            ffi::git_tree_free(self.tree);
            ffi::git_repository_free(self.repo);
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Writes `nodes` to `repo` as a tree, its subtrees first; its id.
fn write_tree(repo: *mut ffi::git_repository, nodes: &[Node]) -> ffi::git_oid {
    let mut builder = ptr::null_mut();
    // SAFETY: `builder` is a live slot and `repo` a live repository; no source tree.
    let rc = unsafe { ffi::git_treebuilder_new(&mut builder, repo, ptr::null()) };
    assert_eq!(rc, 0);
    for node in nodes {
        let (id, mode) = match node {
            Node::Blob(name) => {
                let mut id = ffi::git_oid::default();
                // SAFETY: `id` is a writable oid and `name` readable for its length.
                let rc = unsafe {
                    ffi::git_blob_create_from_buffer(
                        &mut id,
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
            ffi::git_treebuilder_insert(ptr::null_mut(), builder, name.as_ptr(), &id, mode)
        };
        assert_eq!(rc, 0);
    }
    let mut id = ffi::git_oid::default();
    // SAFETY: `id` is a writable oid and `builder` live; it is freed once, after.
    unsafe {
        assert_eq!(ffi::git_treebuilder_write(&mut id, builder), 0);
        ffi::git_treebuilder_free(builder);
    }
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
    // SAFETY: `TreeRepo::walk` passes a live, otherwise unreached `Walk`; libgit2
    // passes a NUL-terminated root and a live entry whose name is NUL-terminated, all
    // live for the call, and both strings are copied before it returns.
    let (walk, root, name) = unsafe {
        (
            &mut *payload.cast::<Walk>(),
            CStr::from_ptr(root).to_str().unwrap().to_owned(),
            CStr::from_ptr(ffi::git_tree_entry_name(entry))
                .to_str()
                .unwrap()
                .to_owned(),
        )
    };
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
        match WalkMode::from_raw(raw) {
            Some(mode) => {
                assert_eq!(c, model(mode, walk_on), "mode {raw}");
                assert_eq!(mode.raw(), raw);
                assert_eq!(WalkMode::try_from(raw), Ok(mode));
                walked.insert(mode);
            }
            None => {
                let refused = (
                    -1,
                    Vec::new(),
                    (
                        ffi::GIT_ERROR_INVALID as c_int,
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
            let refused = repo.walk(MODES.len() as ffi::git_treewalk_mode, reply);
            assert_eq!(refused.0, -1);
            assert!(refused.1.is_empty());
        }
    }
}
