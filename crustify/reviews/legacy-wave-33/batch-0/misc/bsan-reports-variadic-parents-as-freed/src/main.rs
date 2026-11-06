//! Calls `create_from_array` (git_commit_create_v) with N parents, N from argv.
#![forbid(unsafe_code)]
use libgit2::libgit2::core::types::{SignatureView, Time};
use libgit2::libgit2::object::commit::{Commit, create, create_from_array};
use libgit2::libgit2::object::tree::Tree;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let dir = std::env::temp_dir().join(format!("varargs-probe-{}", std::process::id()));
    let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
    let _ = &dir;
    let repo = Repository::init(&path, true).unwrap();
    let r = repo.as_ref();
    let tid = Oid::from_string(c"4b825dc642cb6eb9a060e54bf8d69288fbee4904", OidType::Sha1).unwrap();
    let tree = Tree::lookup(r, tid.as_ref()).unwrap();
    let me = SignatureView::new(c"A", c"a@example.com", Time::new(1, 0));
    let mk = |m: &std::ffi::CStr| {
        let id = create(r, None, me.as_ref(), me.as_ref(), None, m, tree.as_ref(), &[]).unwrap();
        Commit::lookup(r, id.as_ref()).unwrap()
    };
    let (a, b) = (mk(c"a\n"), mk(c"b\n"));
    let (a, b) = (a.as_ref(), b.as_ref());
    let t = tree.as_ref();
    let id = match mode.as_str() {
        "1" => create_from_array(r, None, me.as_ref(), me.as_ref(), None, c"n\n", t, [a]),
        "2" => create_from_array(r, None, me.as_ref(), me.as_ref(), None, c"n\n", t, [a, b]),
        "2same" => create_from_array(r, None, me.as_ref(), me.as_ref(), None, c"n\n", t, [a, a]),
        "slice2" => create(r, None, me.as_ref(), me.as_ref(), None, c"n\n", t, &[a, b]),
        _ => panic!(),
    }
    .unwrap();
    println!("{mode}: {}", id.as_ref().to_hex_string().unwrap());
}
