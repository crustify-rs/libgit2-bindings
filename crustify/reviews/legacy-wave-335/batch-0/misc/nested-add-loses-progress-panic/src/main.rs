//! A progress closure panics once, during the checkout of a `Worktree::add`. The
//! performance-data closure, which runs later in the same checkout, adds another worktree
//! with the same options, as `CheckoutOptionsMut::set_notify_callback`'s docs allow. The
//! outer add should resume the progress panic once libgit2 has returned. It returns `Ok`
//! instead: the nested add's `walk_with` dropped the panic as stale.
#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutPerfdataRef, PerfdataCallback, ProgressCallback,
};
use libgit2::libgit2::src::libgit2::libgit2::init;
use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
use libgit2::libgit2::src::libgit2::worktree::{AddOptions, Worktree};

/// `GIT_CHECKOUT_FORCE`.
const FORCE: u32 = 1 << 1;

/// A repository at `<base>/repo` whose `main` holds `a.txt`; each add creates its branch.
fn repository(base: &Path) -> RepositoryBox {
    let path = base.join("repo");
    let repo =
        Repository::init(&CString::new(path.as_os_str().as_bytes()).unwrap(), false).unwrap();
    let odb = repo.as_ref().odb().unwrap();
    let write = |data: &[u8], kind| {
        let id = odb.as_ref().write(data, kind).unwrap();
        id.as_ref().to_hex_string().unwrap()
    };
    let blob = write(b"one\n", ObjectType::Blob);
    let mut tree = b"100644 a.txt\0".to_vec();
    tree.extend(
        (0..blob.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&blob[i..i + 2], 16).unwrap()),
    );
    let tree = write(&tree, ObjectType::Tree);
    let who = "A U Thor <author@example.com> 1700000000 +0000";
    let commit = format!("tree {tree}\nauthor {who}\ncommitter {who}\n\nfirst\n");
    let commit = write(commit.as_bytes(), ObjectType::Commit);
    drop(odb);
    std::fs::write(path.join(".git/refs/heads/main"), format!("{commit}\n")).unwrap();
    std::fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir(base.join("wt")).unwrap();
    repo
}

fn main() -> ExitCode {
    let _library = std::mem::ManuallyDrop::new(init().unwrap());
    let base: PathBuf =
        std::env::temp_dir().join(format!("nested-add-panic-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&base));
    let repo = repository(&base);
    let add = |name: &str, opts: &AddOptions<'_>| {
        let c_name = CString::new(name).unwrap();
        let dir = CString::new(base.join("wt").join(name).as_os_str().as_bytes()).unwrap();
        Worktree::add(repo.as_ref(), &c_name, &dir, Some(opts.as_ref())).map(drop)
    };

    let opts = RefCell::new(AddOptions::new());
    opts.borrow_mut()
        .as_mut()
        .checkout_options()
        .set_strategy(FORCE);
    let panicked = Cell::new(false);
    let progress_calls = RefCell::new(Vec::new());
    let mut on_progress = |path: Option<&CStr>, done: usize, total: usize| {
        progress_calls
            .borrow_mut()
            .push((path.map(CStr::to_owned), done, total));
        if path.is_some() && !panicked.replace(true) {
            panic!("progress panics once");
        }
    };
    let nested = RefCell::new(None);
    let mut on_perfdata = |_: CheckoutPerfdataRef<'_>| {
        if nested.borrow().is_none() {
            let result = add("inner", &opts.borrow());
            *nested.borrow_mut() = Some(result.is_ok());
        }
    };
    let mut progress = ProgressCallback::new(&mut on_progress);
    let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
    {
        let mut o = opts.borrow_mut();
        let mut m = o.as_mut();
        let mut checkout = m.checkout_options();
        checkout.set_progress_callback(Some(&mut progress));
        checkout.set_perfdata_callback(Some(&mut perfdata));
    }
    let outer = panic::catch_unwind(AssertUnwindSafe(|| add("outer", &opts.borrow())));
    println!("progress calls: {:?}", progress_calls.borrow());
    println!("nested add succeeded: {:?}", nested.borrow());
    drop(std::fs::remove_dir_all(&base));
    match outer {
        Err(p) => {
            println!("outer add resumed: {:?}", p.downcast_ref::<&str>());
            ExitCode::SUCCESS
        }
        Ok(result) => {
            println!(
                "outer add returned {:?}: the progress panic was lost, and the closure that \
                 panicked was called again during the same checkout",
                result.map_err(|e| e.raw_code())
            );
            ExitCode::FAILURE
        }
    }
}
