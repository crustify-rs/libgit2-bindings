//! Public lifecycle and path-matching boundary: all inputs use safe string-array
//! constructors.
use std::ffi::{CStr, CString};

use libgit2::libgit2::src::libgit2::index::Index;
use libgit2::libgit2::src::libgit2::pathspec::{
    MatchList, Pathspec, PathspecArc, PathspecFlags, PathspecMut, PathspecRef,
};
use libgit2::libgit2::src::libgit2::strarray::{StrArray, StrArrayView};

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
                CString::new("src/*.rs").unwrap(),
                CString::new("!src/skip.rs").unwrap(),
                CString::new(vec![0xFF, b'*']).unwrap(),
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

/// libgit2 reads past an empty pattern once another is not empty: such arrays are
/// refused before it parses them (`misc/pathspec-empty-entry-overread`). Heap strings
/// of exactly one byte put the byte after the terminator in a redzone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_patterns_beside_others_are_refused() {
    let empty = CString::new("").unwrap();
    let other = CString::new("a").unwrap();
    for patterns in [[&other, &empty], [&empty, &other]] {
        let view = StrArrayView::new(&patterns);
        assert!(Pathspec::new(view.as_ref()).is_err());
        let owned = StrArray::from_strs(&patterns).unwrap();
        assert!(Pathspec::new(owned.as_ref()).is_err());
    }
    let alone = [&empty, &empty];
    let view = StrArrayView::new(&alone);
    shared(Pathspec::new(view.as_ref()).unwrap().as_ref());
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

/// Every flag bit, including both case flags and bits C does not know; sampled under
/// `BSan`, whose per-call cost makes the full sweep slow.
fn all_flag_sets() -> impl Iterator<Item = PathspecFlags> {
    let stride = if cfg!(bsan) { 7 } else { 1 };
    (0..64)
        .step_by(stride)
        .chain([u32::MAX])
        .map(PathspecFlags::from_bits_retain)
}

/// Exactly sized heap paths put the byte after each terminator in a redzone: short
/// paths against long patterns, `!`-prefixed paths against negative patterns (whose
/// check reads one byte past the compared prefix), and directory-prefix probes.
fn heap_paths() -> Vec<CString> {
    [
        &b""[..],
        b"!",
        b"/",
        b"s",
        b"src",
        b"src/",
        b"src/a.rs",
        b"!src/skip.rs",
        b"!src/skip.rs/x",
        b"src/skip.rs",
        b"SRC/A.RS",
        b"docs/a/b/c",
        b"docsx",
        b"\xff\xfe",
        b"x/y/z/w/v/u/t/s/r/q/p/o/n/m/l/k/j/i/h/g/f/e/d/c/b/a.rs",
    ]
    .into_iter()
    .map(|path| CString::new(path).unwrap())
    .collect()
}

fn match_all(handle: PathspecRef<'_>, paths: &[CString]) -> Vec<bool> {
    let mut seen = Vec::new();
    for flags in all_flag_sets() {
        for path in paths {
            seen.push(handle.matches_path(flags, path));
        }
    }
    seen
}

/// Matching through every owner and handle form, while a match list retains a
/// second counted reference and after it releases it, with inputs already dropped.
/// Results never change and nothing is read past a terminator or after a release.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn matches_path_across_owners_reborrows_and_retaining_list() {
    let paths = heap_paths();
    let pattern_sets: &[&[&CStr]] = &[
        &[],
        &[c""],
        &[c"#only a comment"],
        &[c"/"],
        &[c"!/", c"*"],
        &[c"src/*.rs", c"!src/skip.rs", c"docs", c"\\*"],
        &[c"**/a.rs", c"[!a-z]*", c"docs/"],
    ];
    for patterns in pattern_sets {
        let mut owner = {
            let inputs: Vec<CString> = patterns.iter().map(|p| (*p).to_owned()).collect();
            let view = StrArrayView::new(&inputs);
            Pathspec::new(view.as_ref()).unwrap()
        };
        let baseline = match_all(owner.as_ref(), &paths);
        let mut exclusive = owner.as_mut();
        let mut child = exclusive.as_mut();
        assert_eq!(match_all(child.as_ref(), &paths), baseline);
        assert_eq!(match_all(child.as_mut().as_ref(), &paths), baseline);
        assert_eq!(match_all(exclusive.as_ref(), &paths), baseline);
        let shared: PathspecArc = owner.into();
        let mut index = Index::new().unwrap();
        let list =
            MatchList::from_index(index.as_mut(), &shared, PathspecFlags::FIND_FAILURES).unwrap();
        assert_eq!(match_all(shared.as_ref(), &paths), baseline);
        drop(index);
        drop(list);
        assert_eq!(match_all(shared.as_ref(), &paths), baseline);
    }
}

/// Heap path strings are freed right after each call: C retains no pointer to them,
/// so reusing their memory cannot change a later result.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn matches_path_retains_no_path() {
    let view = StrArrayView::new(&[c"!keep/no", c"keep/*"]);
    let owner = Pathspec::new(view.as_ref()).unwrap();
    drop(view);
    for round in 0..256 {
        let hit = CString::new(format!("keep/{round}")).unwrap();
        assert!(owner.as_ref().matches_path(PathspecFlags::DEFAULT, &hit));
        drop(hit);
        let miss = CString::new(format!("drop/{round}")).unwrap();
        assert!(!owner.as_ref().matches_path(PathspecFlags::DEFAULT, &miss));
        let excluded = CString::new("keep/no").unwrap();
        assert!(
            !owner
                .as_ref()
                .matches_path(PathspecFlags::DEFAULT, &excluded)
        );
    }
}

/// Thread-local pathspecs match concurrently; C keeps no shared matching state.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn independent_thread_local_matching() {
    let threads: Vec<_> = (0..8)
        .map(|t| {
            std::thread::spawn(move || {
                let view = StrArrayView::new(&[c"!src/skip", c"src/*", c"DOCS"]);
                let owner = Pathspec::new(view.as_ref()).unwrap();
                for round in 0..64 {
                    let path = CString::new(format!("src/{t}-{round}")).unwrap();
                    assert!(owner.as_ref().matches_path(PathspecFlags::DEFAULT, &path));
                    assert!(
                        !owner
                            .as_ref()
                            .matches_path(PathspecFlags::DEFAULT, c"src/skip")
                    );
                    assert!(
                        !owner
                            .as_ref()
                            .matches_path(PathspecFlags::DEFAULT, c"docs/x")
                    );
                    assert!(
                        owner
                            .as_ref()
                            .matches_path(PathspecFlags::IGNORE_CASE, c"docs/x")
                    );
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

mod match_list;
