//! UB tests for `git2/buffer.h`: the `Buf` value held by its `BufVal` owner, its
//! handles and their accessors, and the `git_buf_dispose` policy.
//!
//! Buffers built in Rust hold exactly `len + 1` bytes (`from_bytes`), so ASan flags any
//! access past the terminator; every replaced, taken or copied allocation must be
//! released exactly once (ASan: double free, use after free; LSan: leak). libgit2's
//! own results come from the `GIT_OPT_GET_*` options through [`fill_from_option`], in a
//! child process, since the options are unsynchronised globals. Illegal borrows are in
//! `compile_fail/`. Only empty buffers are built without libgit2, so little runs under
//! Miri.

use std::sync::Barrier;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::buffer::{Buf, BufMut, BufRef, BufVal};
use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataVec};

use crate::fixture::{fill_from_option, isolated, set_str_option};

/// `len` bytes without a NUL, so a C string read must stop at the terminator.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 255) as u8 + 1).collect()
}

#[test]
fn buffers_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Buf>();
    send_sync::<BufVal>();
    send_sync::<BufRef<'_>>();
    send_sync::<BufMut<'_>>();
}

/// The empty buffer over a bare value: no allocation, no disposal, so Miri runs it.
#[test]
fn empty_bare_buffer_in_rust_storage() {
    let mut bare = Buf::new().into_inner();
    let mut m = bare.as_mut();
    assert_eq!(m.set_bytes(b""), Some(()));
    assert!(m.take_str().is_none());
    assert!(m.take_bytes().is_empty());
    let r = m.as_ref();
    assert_eq!((r.len(), r.capacity(), r.as_bytes()), (0, 0, &b""[..]));
    assert_eq!(r.as_c_str(), c"");
    assert_eq!(format!("{m:?}"), r#"Buf("")"#);
    // Empty, so the bare value owns nothing and going out of scope leaks nothing.
    // Handing a bare value back to an owner is `CVal::new`, unsafe: see
    // `ub_unsafe::buffer`.
}

/// Before `git_libgit2_init` an empty buffer needs nothing from libgit2 (its dispose
/// never calls it), and an allocation pins the runtime first.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn buffers_before_init() {
    isolated(concat!(module_path!(), "::buffers_before_init"), || {
        let mut empty = Buf::new();
        assert!(empty.as_ref().is_empty());
        assert!(empty.as_mut().take_str().is_none());
        drop(empty);
        drop(Buf::from_bytes(b"").unwrap());
        let buf = Buf::from_bytes(b"first").unwrap();
        assert_eq!(buf.as_ref().as_bytes(), b"first");
    });
}

/// Empty buffers, which never initialise libgit2, dropped while another thread's first
/// allocation initialises it: their dispose must not reach `git__free`, which reads the
/// allocator that initialisation writes (TSan). Alone in a child, so the initialisation
/// is really the process's first.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_buffer_drops_do_not_race_the_first_init() {
    isolated(
        concat!(
            module_path!(),
            "::empty_buffer_drops_do_not_race_the_first_init"
        ),
        || {
            let start = Barrier::new(2);
            thread::scope(|s| {
                s.spawn(|| {
                    start.wait();
                    for _ in 0..64 {
                        drop(Buf::new());
                        drop(Buf::from_bytes(b"").unwrap());
                        let mut emptied = Buf::new();
                        drop(emptied.as_mut().take());
                        assert!(emptied.as_mut().take_str().is_none());
                        drop(emptied.as_mut().take_bytes());
                        emptied.as_mut().set_bytes(b"").unwrap();
                        drop(emptied);
                    }
                });
                s.spawn(|| {
                    start.wait();
                    let buf = Buf::from_bytes(b"x").unwrap();
                    assert_eq!(buf.as_ref().len(), 1);
                });
            });
        },
    );
}

/// Every view reads exactly the contents and their terminator: `as_c_str` scans to the
/// NUL at `len`, and `as_bytes` stops before it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn views_stay_in_bounds() {
    for n in [1, 2, 7, 8, 9, 15, 16, 17, 64, 4096] {
        let src = pattern(n);
        let buf = Buf::from_bytes(&src).unwrap();
        let r = buf.as_ref();
        assert_eq!((r.len(), r.capacity()), (n, n + 1));
        assert_eq!(r.as_bytes(), &src[..]);
        assert_eq!(r.as_c_str().to_bytes(), &src[..]);
        assert_eq!(r.to_str().is_ok(), std::str::from_utf8(&src).is_ok());
        assert!(format!("{r:?}").starts_with("Buf(\""));
        let copy = r.to_buf().unwrap();
        assert_eq!(copy.as_ref().as_bytes(), &src[..]);

        // NUL bytes inside: the C string stops at the first, the bytes do not.
        let mut binary = src.clone();
        binary[n - 1] = 0;
        binary[n / 2] = 0;
        let buf = Buf::from_bytes(&binary).unwrap();
        assert_eq!(buf.as_ref().as_bytes(), &binary[..]);
        assert_eq!(buf.as_ref().as_c_str().to_bytes(), &binary[..n / 2]);
    }
}

/// `set_bytes` writes in place up to `capacity - 1` bytes, its NUL in the last byte,
/// and allocates afresh from `capacity` bytes on: ASan flags an off-by-one write.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_bytes_at_the_capacity_boundary() {
    for cap in [2, 8, 17, 4097] {
        let mut buf = Buf::from_bytes(&pattern(cap - 1)).unwrap();
        assert_eq!(buf.as_ref().capacity(), cap);
        for len in [cap - 1, cap / 2, 1, 0, cap - 1] {
            let src = pattern(len);
            buf.as_mut().set_bytes(&src).unwrap();
            assert_eq!(buf.as_ref().capacity(), cap, "in place");
            assert_eq!(buf.as_ref().as_bytes(), &src[..]);
            assert_eq!(buf.as_ref().as_c_str().to_bytes(), &src[..]);
        }
        let src = pattern(cap);
        buf.as_mut().set_bytes(&src).unwrap();
        assert_eq!(buf.as_ref().capacity(), cap + 1, "grown");
        assert_eq!(buf.as_ref().as_c_str().to_bytes(), &src[..]);
    }
}

/// Contents moved between buffers and out of them are released once, whichever owner
/// drops first, and stay readable after the buffer they came from is gone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn moves_and_replacements_release_once() {
    let mut first = Buf::from_bytes(b"refs/heads/main").unwrap();
    let mut second = Buf::from_bytes(b"origin").unwrap();

    // The allocation moves from one live buffer to another, which releases its own.
    let moved = first.as_mut().take();
    second.as_mut().set(moved);
    assert!(first.as_ref().is_empty() && first.as_ref().capacity() == 0);
    assert_eq!(second.as_ref().as_bytes(), b"refs/heads/main");

    // Out as a string, then the buffer dropped first.
    let s = second.as_mut().take_str().unwrap();
    drop(second);
    assert_eq!(s.to_str(), Ok("refs/heads/main"));

    // In again without a copy, then out as bytes, the parent dropped first again.
    first.as_mut().set_str(s);
    first.as_mut().set_str(OdbDataStrFree::dup(c"").unwrap());
    assert_eq!((first.as_ref().len(), first.as_ref().capacity()), (0, 1));
    first.as_mut().set(Buf::from_bytes(b"pa\0ck").unwrap());
    let bytes = first.as_mut().take_bytes();
    drop(first);
    assert_eq!(bytes.as_slice(), b"pa\0ck");

    // A copy outlives its source; replacing the source leaves it alone.
    let mut src = Buf::from_bytes(b"source").unwrap();
    let copy = src.as_ref().to_buf().unwrap();
    src.as_mut()
        .set_bytes(b"edited, and longer than before")
        .unwrap();
    drop(src);
    assert_eq!(copy.as_ref().as_bytes(), b"source");

    drop(copy);
    let empty: OdbDataVec<u8> = Buf::new().as_mut().take_bytes();
    assert!(empty.is_empty());
}

/// libgit2's results: an owned string, refilled in place and reallocated, the
/// unallocated static empty string, which is never freed, and a Rust-built allocation
/// libgit2 reuses. C writes each buffer between Rust's reads (BSan: no stale view).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_results_are_read_reused_and_released_once() {
    isolated(
        concat!(
            module_path!(),
            "::libgit2_results_are_read_reused_and_released_once"
        ),
        || {
            let (ua, tp) = (ffi::GIT_OPT_GET_USER_AGENT, ffi::GIT_OPT_GET_TEMPLATE_PATH);
            set_str_option(ffi::GIT_OPT_SET_TEMPLATE_PATH, Some(c""));
            let mut buf = Buf::new();
            for agent in [
                c"crustify/1.0",
                c"x",
                c"a much longer user agent than before",
                c"",
            ] {
                set_str_option(ffi::GIT_OPT_SET_USER_AGENT, Some(agent));
                fill_from_option(ua, &mut buf);
                assert_eq!(buf.as_ref().as_c_str(), agent);
                assert_eq!(buf.as_ref().as_bytes(), agent.to_bytes());
                assert!(buf.as_ref().capacity() > buf.as_ref().len());
            }
            fill_from_option(tp, &mut buf);
            assert!(buf.as_ref().is_empty() && buf.as_ref().capacity() > 0);

            let mut empty = Buf::new();
            fill_from_option(tp, &mut empty);
            assert_eq!((empty.as_ref().len(), empty.as_ref().capacity()), (0, 0));
            assert_eq!(empty.as_ref().as_c_str(), c"");
            assert!(empty.as_ref().to_buf().unwrap().as_ref().is_empty());
            fill_from_option(tp, &mut empty);
            assert!(empty.as_mut().take_str().is_none());
            drop(empty);

            set_str_option(ffi::GIT_OPT_SET_USER_AGENT, Some(c"crustify/2.0"));
            let mut ours = Buf::from_bytes(&pattern(3)).unwrap();
            fill_from_option(ua, &mut ours);
            assert_eq!(ours.as_ref().to_str(), Ok("crustify/2.0"));
            let s = ours.as_mut().take_str().unwrap();
            fill_from_option(ua, &mut ours);
            let bytes = ours.as_mut().take_bytes();
            assert_eq!(s.as_bytes(), bytes.as_slice());
            fill_from_option(ua, &mut ours);
            fill_from_option(tp, &mut buf);
            drop((buf, ours));
            assert_eq!(s.to_str(), Ok("crustify/2.0"));

            set_str_option(ffi::GIT_OPT_SET_TEMPLATE_PATH, None);
            set_str_option(ffi::GIT_OPT_SET_USER_AGENT, None);
        },
    );
}

/// Shared handles read one buffer from several threads at once; owners and the
/// contents taken from them move to other threads and drop there (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn buffers_shared_and_sent_across_threads() {
    let buf = Buf::from_bytes(&pattern(1024)).unwrap();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let r = buf.as_ref();
                assert_eq!(r.as_bytes(), &pattern(1024)[..]);
                assert_eq!(r.as_c_str().to_bytes().len(), 1024);
                drop(r.to_buf().unwrap());
            });
        }
    });
    let mut sent = buf;
    let s = thread::spawn(move || {
        sent.as_mut().set_bytes(b"from another thread").unwrap();
        let s = sent.as_mut().take_str().unwrap();
        drop(sent);
        s
    })
    .join()
    .unwrap();
    assert_eq!(s.to_str(), Ok("from another thread"));
    let owner = Buf::from_bytes(b"dropped elsewhere").unwrap();
    thread::spawn(move || drop(owner)).join().unwrap();
}
