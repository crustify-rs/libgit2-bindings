//! UB tests for `git2/sys/odb_backend.h`'s `git_odb_backend`: the `OdbBackend`
//! template, the `OdbBackendBox` owner over libgit2's packed and loose backends, the
//! getters, and the members the exclusive handle calls (`refresh`, `foreach`,
//! `writemidx`).
//!
//! Backends are built and released over and over, fresh, walked, after a failed rescan
//! and after a panicking walk (`ASan` flags a use after free or double free, `LSan` a
//! leaked pack, index or path); handles are taken again and again between the calls
//! (`BSan`); threads each walk and rescan a backend of their own over one shared
//! directory (`TSan`). The template's members are all unset, and every call through it
//! fails or does nothing, never calling through NULL. The constructors are stand-ins
//! from `fixture::odb_backend`.

use std::ffi::CString;
use std::io::Write as _;
use std::num::NonZero;
use std::ops::ControlFlow;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::{fs, thread};

use libgit2::libgit2::src::libgit2::odb::{ForeachCallback, Odb, OdbBackend, OdbBackendBox};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidRef};
use libgit2::libgit2::src::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::init;
use crate::fixture::odb_backend::{loose, pack};

const PACKED: usize = 5;
const LOOSE: usize = 3;

/// An `objects` directory of its own, removed on drop: `PACKED` blobs in one pack, and
/// `LOOSE` loose ones beside it.
struct Objects(PathBuf, CString);

impl Objects {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "crustify-odb-backend-ub-{}-{tag}",
            std::process::id()
        ));
        drop(fs::remove_dir_all(&root));
        let git = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", std::env::temp_dir());
            cmd
        };
        let root_s = root.to_str().unwrap();
        assert!(
            git(&["init", "-q", "--bare", root_s])
                .status()
                .unwrap()
                .success()
        );
        let hash = |blob: String| {
            let mut child = git(&["-C", root_s, "hash-object", "-w", "--stdin"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(blob.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            String::from_utf8(out.stdout).unwrap()
        };
        let ids: String = (0..PACKED).map(|i| hash(format!("packed {i}\n"))).collect();
        let base = root.join("objects/pack/pack");
        let mut child = git(&["-C", root_s, "pack-objects", "-q", base.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(ids.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert!(
            git(&["-C", root_s, "prune-packed", "-q"])
                .status()
                .unwrap()
                .success()
        );
        for i in 0..LOOSE {
            hash(format!("loose {i}\n"));
        }
        let objects = root.join("objects");
        let c = CString::new(objects.to_str().unwrap()).unwrap();
        Self(root, c)
    }

    fn pack_dir(&self) -> PathBuf {
        self.0.join("objects/pack")
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// Every id the backend walks, through a fresh callback.
fn walk(backend: &mut OdbBackendBox) -> Vec<Oid> {
    let mut seen = Vec::new();
    let mut collect = |id: OidRef<'_>| {
        seen.push(id.to_oid());
        ControlFlow::Continue(())
    };
    let flow = backend
        .as_mut()
        .foreach(&mut ForeachCallback::new(&mut collect))
        .unwrap();
    assert_eq!(flow, ControlFlow::Continue(()));
    seen
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_template_never_calls_an_unset_member() {
    init();
    for _ in 0..8 {
        let mut template = OdbBackend::new();
        let r = template.as_ref();
        assert_eq!(r.version(), 1);
        assert!(!r.in_odb() && !r.has_free() && !r.has_foreach() && !r.has_refresh());
        let mut m = template.as_mut();
        m.refresh().unwrap();
        let mut never = |_: OidRef<'_>| -> ControlFlow<NonZero<i32>> { unreachable!() };
        let err = m
            .foreach(&mut ForeachCallback::new(&mut never))
            .unwrap_err();
        assert_eq!(
            (err.code(), err.class()),
            (Some(ErrorCode::Generic), Some(ErrorClass::Odb))
        );
        let err = m.as_mut().write_multi_pack_index().unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Odb));
        assert!(format!("{template:?}").contains("has_read: false"));
        let odb = Odb::new().unwrap();
        assert!(!template.as_ref().belongs_to(odb.as_ref()));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn backends_release_once_however_they_were_used() {
    init();
    let objects = Objects::new("lifecycle");
    for round in 0..6 {
        // Released fresh.
        drop(pack(&objects.1).unwrap());
        drop(loose(&objects.1).unwrap());
        // Released after walks, rescans, a stopped walk and a panicking one.
        let mut packed = pack(&objects.1).unwrap();
        let mut unpacked = loose(&objects.1).unwrap();
        assert_eq!(walk(&mut packed).len(), PACKED);
        assert_eq!(walk(&mut unpacked).len(), LOOSE);
        packed.as_mut().refresh().unwrap();
        unpacked.as_mut().refresh().unwrap();
        let mut calls = 0;
        let mut stop = |_: OidRef<'_>| {
            calls += 1;
            ControlFlow::Break(NonZero::new(-1 - round).unwrap())
        };
        let flow = packed
            .as_mut()
            .foreach(&mut ForeachCallback::new(&mut stop));
        assert_eq!(
            flow.unwrap(),
            ControlFlow::Break(NonZero::new(-1 - round).unwrap())
        );
        assert_eq!(calls, 1);
        let mut boom = |_: OidRef<'_>| -> ControlFlow<NonZero<i32>> { panic!("boom") };
        let mut callback = ForeachCallback::new(&mut boom);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            unpacked.as_mut().foreach(&mut callback)
        }));
        assert_eq!(caught.unwrap_err().downcast_ref::<&str>(), Some(&"boom"));
        assert_eq!(walk(&mut unpacked).len(), LOOSE);
        if round % 2 == 0 {
            drop(unpacked);
            drop(packed);
        } else {
            drop(packed);
            drop(unpacked);
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn handles_taken_between_calls_see_one_backend() {
    init();
    let objects = Objects::new("handles");
    let mut backend = pack(&objects.1).unwrap();
    let before = format!("{:?}", backend.as_ref());
    for _ in 0..16 {
        let mut m = backend.as_mut();
        assert!(m.as_ref().has_foreach() && m.as_ref().has_write_multi_pack_index());
        m.as_mut().refresh().unwrap();
        let r = m.as_ref();
        assert!(!r.in_odb());
        assert_eq!(r.version(), 1);
        assert_eq!(walk(&mut backend).len(), PACKED);
    }
    backend.as_mut().write_multi_pack_index().unwrap();
    assert!(objects.pack_dir().join("multi-pack-index").is_file());
    let mut ids = walk(&mut backend);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), PACKED);
    assert_eq!(format!("{:?}", backend.as_ref()), before);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_failed_rescan_leaves_a_releasable_backend() {
    init();
    let objects = Objects::new("failed");
    let mut backend = pack(&objects.1).unwrap();
    assert_eq!(walk(&mut backend).len(), PACKED);
    fs::remove_dir_all(objects.pack_dir()).unwrap();
    let err = backend.as_mut().refresh().unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::NotFound));
    let mut go = |_: OidRef<'_>| ControlFlow::Continue(());
    let err = backend
        .as_mut()
        .foreach(&mut ForeachCallback::new(&mut go))
        .unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::NotFound));
    drop(backend);
    // A backend over a directory that never had packs walks nothing.
    let mut empty = pack(&objects.1).unwrap();
    assert_eq!(walk(&mut empty), []);
    empty.as_mut().refresh().unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn threads_each_walk_their_own_backend() {
    init();
    let objects = Arc::new(Objects::new("threads"));
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let objects = Arc::clone(&objects);
            thread::spawn(move || {
                for _ in 0..4 {
                    let mut backend = if i % 2 == 0 {
                        pack(&objects.1).unwrap()
                    } else {
                        loose(&objects.1).unwrap()
                    };
                    let expected = if i % 2 == 0 { PACKED } else { LOOSE };
                    assert_eq!(walk(&mut backend).len(), expected);
                    backend.as_mut().refresh().unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}
