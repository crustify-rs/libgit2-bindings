//! Public lifecycle boundary: all inputs use safe string-array constructors.
use libgit2::libgit2::core::strarray::{StrArray, StrArrayView};
use libgit2::libgit2::index::pathspec::{Pathspec, PathspecArc, PathspecMut, PathspecRef};

fn shared(_: PathspecRef<'_>) {}
fn exclusive(mut handle: PathspecMut<'_>) {
    shared(handle.as_ref());
    let mut child = handle.as_mut();
    shared(child.as_ref());
    let _grandchild = child.as_mut();
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copied_inputs_and_reborrows() {
    for _ in 0..64 {
        let mut compiled = {
            let inputs = [
                std::ffi::CString::new("src/*.rs").unwrap(),
                std::ffi::CString::new("!src/skip.rs").unwrap(),
                std::ffi::CString::new(vec![0xff, b'*']).unwrap(),
            ];
            let view = StrArrayView::new(&inputs);
            Pathspec::new(view.as_ref()).unwrap()
        };
        let handle = compiled.as_ref();
        shared(handle);
        shared(handle); // shared handles are Copy
        exclusive(compiled.as_mut());
        exclusive(compiled.as_mut());
        let shared_owner: PathspecArc = compiled.into();
        shared(shared_owner.as_ref());
        drop(shared_owner);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_owned_and_borrowed_inputs() {
    for patterns in [
        &[][..],
        &[c""][..],
        &[c"", c"", c""][..],
        &[c"#comment", c"!", c"\\"][..],
    ] {
        let view = StrArrayView::new(patterns);
        let first = Pathspec::new(view.as_ref()).unwrap();
        let mut owned = StrArray::from_strs(patterns).unwrap();
        let second = Pathspec::new(owned.as_ref()).unwrap();
        owned.as_mut().truncate(0);
        drop(owned);
        drop(view);
        shared(first.as_ref());
        shared(second.as_ref());
        drop(second);
        drop(first);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn independent_thread_local_lifecycles() {
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..32 {
                    let view = StrArrayView::new(&[c"src/*", c"docs/**"]);
                    let owner = Pathspec::new(view.as_ref()).unwrap();
                    let shared_owner: PathspecArc = owner.into();
                    shared(shared_owner.as_ref());
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

mod match_list;
