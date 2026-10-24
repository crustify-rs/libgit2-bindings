//! UB tests for the unsafe API of `git2/buffer.h`: `Buf::from_raw`, called within its
//! contract on every shape of `git_buf` libgit2 leaves behind or C code builds, edge
//! cases included, and then driven through the safe API: read, taken out, refilled by
//! libgit2, and released (`ASan`: double free, invalid free, use after free, overflow;
//! `LSan`: leak).
//!
//! libgit2's `GIT_OPT_*` string options are unsynchronised globals, so the tests that
//! set them rerun alone in a child process.

use std::ffi::c_char;
use std::process::Command;
use std::ptr::{self, NonNull};
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::buf::{Buf, BufVal};
use libgit2::libgit2::src::libgit2::odb::{OdbDataStr, OdbDataStrFree};

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
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

/// The raw C calls below allocate, which needs an initialised libgit2; the extra
/// reference dies with the process.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

fn zeroed_buf() -> ffi::git_buf {
    ffi::git_buf {
        ptr: ptr::null_mut(),
        reserved: 0,
        size: 0,
    }
}

fn set_option(option: u32, value: *const c_char) {
    // SAFETY: the string options copy their `const char *`, or restore their default
    // for NULL.
    let ret = unsafe { ffi::git_libgit2_opts(option.cast_signed(), value) };
    assert_eq!(ret, 0);
}

/// A `GIT_OPT_GET_*` option taking a `git_buf *` into `buf`.
fn get_option(option: u32, buf: &mut ffi::git_buf) {
    // SAFETY: the option takes a `git_buf *` that is zeroed, disposed or as a libgit2
    // routine left it, which it fills.
    let rc = unsafe { ffi::git_libgit2_opts(option.cast_signed(), &raw mut *buf) };
    assert_eq!(rc, 0);
}

/// An allocation of libgit2's of `reserved` bytes holding `bytes` and a NUL, as C code
/// filling a buffer makes one.
fn c_buf(bytes: &[u8], reserved: usize) -> ffi::git_buf {
    assert!(reserved > bytes.len());
    // SAFETY: the routine ignores `backend`; any length.
    let p = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), reserved) }.cast::<u8>();
    assert!(!p.is_null());
    // SAFETY: `p` holds `reserved` writable bytes, distinct from `bytes`.
    unsafe { p.copy_from_nonoverlapping(bytes.as_ptr(), bytes.len()) };
    // SAFETY: as above.
    let value2 = unsafe { p.add(bytes.len()) };
    // SAFETY: as above.
    unsafe { value2.write(0) };
    ffi::git_buf {
        ptr: p.cast(),
        reserved,
        size: bytes.len(),
    }
}

#[test]
fn adopting_git_buf_init_owns_nothing() {
    // SAFETY: `GIT_BUF_INIT` has `reserved == 0`, for which there is no requirement.
    let mut buf = unsafe { Buf::from_raw(zeroed_buf()) };
    assert!(buf.as_ref().is_empty() && buf.as_ref().capacity() == 0);
    assert!(buf.as_mut().take_str().is_none());
    // Dropped without reaching libgit2 (Miri aborts on any foreign call).
}

/// With `reserved == 0` the contract asks nothing of `ptr` and `size`: the buffer owns
/// nothing, reads as empty, and is never freed, whatever they hold.
#[test]
fn adopting_an_unowned_buffer_never_reads_or_frees_it() {
    let shapes = [
        ffi::git_buf {
            ptr: NonNull::<c_char>::dangling().as_ptr(),
            reserved: 0,
            size: 0,
        },
        ffi::git_buf {
            ptr: NonNull::<c_char>::dangling().as_ptr(),
            reserved: 0,
            size: usize::MAX,
        },
        ffi::git_buf {
            ptr: ptr::null_mut(),
            reserved: 0,
            size: 7,
        },
        ffi::git_buf {
            ptr: ptr::without_provenance_mut(0x1001),
            reserved: 0,
            size: 1,
        },
    ];
    for raw in shapes {
        // SAFETY: `reserved == 0`, for which there is no requirement.
        let mut buf = unsafe { Buf::from_raw(raw) };
        let r = buf.as_ref();
        assert_eq!((r.len(), r.capacity(), r.as_bytes()), (0, 0, &b""[..]));
        assert_eq!(r.as_c_str(), c"");
        assert!(buf.as_mut().take_bytes().is_empty());
        // SAFETY: as above.
        let mut buf = unsafe { Buf::from_raw(raw) };
        buf.as_mut().set_bytes(b"").unwrap();
        // The bare value hands back what it was given.
        let back = buf.into_inner().into_raw();
        assert_eq!(
            (back.ptr, back.reserved, back.size),
            (raw.ptr, raw.reserved, raw.size)
        );
    }
}

/// C-built buffers at the contract's edges: `size` at its maximum, `reserved - 1`, and
/// at 0, spare capacity, NUL bytes inside; each read, edited, taken and released once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopting_c_built_buffers_at_the_edges() {
    init();
    let cases: [(&[u8], usize); 6] = [
        (b"", 1),
        (b"", 64),
        (b"a", 2),
        (b"main", 5),
        (b"pa\0ck", 6),
        (b"refs/heads/main", 4096),
    ];
    for (bytes, reserved) in cases {
        // SAFETY: `c_buf` makes an allocation of libgit2's of `reserved` bytes,
        // NUL-terminated at `size < reserved`, which the struct owns and hands over.
        let mut buf = unsafe { Buf::from_raw(c_buf(bytes, reserved)) };
        let r = buf.as_ref();
        assert_eq!(
            (r.as_bytes(), r.len(), r.capacity()),
            (bytes, bytes.len(), reserved)
        );
        let up_to_nul = bytes.split(|&b| b == 0).next().unwrap();
        assert_eq!(r.as_c_str().to_bytes(), up_to_nul);

        // In place up to `reserved - 1` bytes, its NUL in the last one.
        let fill = vec![b'z'; reserved - 1];
        buf.as_mut().set_bytes(&fill).unwrap();
        assert_eq!(
            (buf.as_ref().as_bytes(), buf.as_ref().capacity()),
            (&fill[..], reserved)
        );
        buf.as_mut().set_bytes(bytes).unwrap();

        // SAFETY: as above.
        let mut other = unsafe { Buf::from_raw(c_buf(bytes, reserved)) };
        let s = buf.as_mut().take_str().unwrap();
        assert_eq!(s.as_bytes(), up_to_nul);
        let v = other.as_mut().take_bytes();
        assert_eq!(v.as_slice(), bytes);
        drop((buf, other));
        assert_eq!((s.as_bytes(), v.as_slice()), (up_to_nul, bytes));

        // Released by the owner without a take.
        // SAFETY: as above.
        drop(unsafe { Buf::from_raw(c_buf(bytes, reserved)) });
    }
}

/// libgit2's own results: an owned string, adopted and moved out without a copy; the
/// static empty string, adopted and never freed; and a buffer reused, taken and
/// refilled round after round, as `OdbDataStrFree::take_buf` (which this replaces) was
/// tested.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopting_what_libgit2_leaves() {
    isolated(
        concat!(module_path!(), "::adopting_what_libgit2_leaves"),
        || {
            init();
            let (ua, tp) = (ffi::GIT_OPT_GET_USER_AGENT, ffi::GIT_OPT_GET_TEMPLATE_PATH);
            set_option(ffi::GIT_OPT_SET_TEMPLATE_PATH, c"".as_ptr());

            let mut raw = zeroed_buf();
            get_option(ua, &mut raw);
            assert!(raw.reserved > 0, "the buffer owns its string");
            let (p, size) = (raw.ptr, raw.size);
            // SAFETY: `raw` is as `GIT_OPT_GET_USER_AGENT` left it, and is not used again.
            let mut buf = unsafe { Buf::from_raw(raw) };
            let s = buf.as_mut().take_str().unwrap();
            assert_eq!(
                s.as_ptr(),
                p.cast_const(),
                "the string is moved, not copied"
            );
            assert_eq!(s.len(), size);
            assert!(s.to_str().unwrap().contains("libgit2"));
            assert!(buf.as_ref().is_empty() && buf.as_ref().capacity() == 0);
            let t = s.clone();
            drop((s, buf));
            assert_eq!(t.len(), size);

            let mut raw = zeroed_buf();
            get_option(tp, &mut raw);
            assert!(!raw.ptr.is_null() && raw.reserved == 0 && raw.size == 0);
            // SAFETY: `raw` is as `GIT_OPT_GET_TEMPLATE_PATH` left it: owning nothing.
            let mut buf = unsafe { Buf::from_raw(raw) };
            assert_eq!(buf.as_ref().as_c_str(), c"");
            assert!(buf.as_mut().take_str().is_none());
            drop(buf);

            // A disposed buffer points at the static empty string and owns nothing.
            let mut raw = zeroed_buf();
            get_option(ua, &mut raw);
            // SAFETY: `raw` owns a libgit2 allocation.
            unsafe { ffi::git_buf_dispose(&raw mut raw) };
            assert!(!raw.ptr.is_null() && raw.reserved == 0);
            // SAFETY: `raw` is as `git_buf_dispose` left it.
            drop(unsafe { Buf::from_raw(raw) });

            // Filled, adopted, taken, handed back raw and refilled: never a stale pointer.
            let mut buf: BufVal = Buf::new();
            let mut taken: Vec<OdbDataStr> = Vec::new();
            for round in 0..3 {
                let mut raw = buf.as_mut().take().into_inner().into_raw();
                get_option(if round == 1 { tp } else { ua }, &mut raw);
                get_option(ua, &mut raw);
                // SAFETY: `raw` is as libgit2 left it.
                buf.as_mut().set(unsafe { Buf::from_raw(raw) });
                let s = buf.as_mut().take_str().unwrap();
                assert!(
                    taken.iter().all(|t| t.as_ptr() != s.as_ptr()),
                    "round {round}"
                );
                taken.push(s);
            }
            // A string from the safe API in, then libgit2 reusing its allocation.
            buf.as_mut()
                .set_str(OdbDataStrFree::dup(c"0123456789012345678901234567890123456789").unwrap());
            let mut raw = buf.as_mut().take().into_inner().into_raw();
            get_option(ua, &mut raw);
            // SAFETY: `raw` is as libgit2 left it.
            let buf = unsafe { Buf::from_raw(raw) };
            assert_eq!(buf.as_ref().as_c_str(), taken[0].as_c_str());

            set_option(ffi::GIT_OPT_SET_TEMPLATE_PATH, ptr::null());
            // Adopted buffers and taken strings cross threads.
            let n = thread::spawn(move || {
                let n = buf.as_ref().len();
                n + taken.into_iter().map(|s| s.len()).sum::<usize>()
            })
            .join()
            .unwrap();
            assert!(n > 0);
        },
    );
}

/// An adopted buffer's bare value goes back to C, which releases it with
/// `git_buf_dispose`, exactly once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopted_buffers_handed_back_to_c() {
    init();
    // SAFETY: an allocation of libgit2's of 9 bytes, NUL-terminated at 3.
    let buf = unsafe { Buf::from_raw(c_buf(b"abc", 9)) };
    let mut raw = buf.into_inner().into_raw();
    // SAFETY: `raw` owns its allocation, which C now releases.
    unsafe { ffi::git_buf_dispose(&raw mut raw) };
    // SAFETY: `raw` is as `git_buf_dispose` left it: the static empty string.
    let buf = unsafe { Buf::from_raw(raw) };
    assert_eq!((buf.as_ref().as_c_str(), buf.as_ref().capacity()), (c"", 0));
}

/// A bare value `into_inner` gave up is adopted again by `CVal::new`, whose caller
/// vouches for its ownership: the new owner releases it exactly once, whatever the
/// safe setters wrote into it while it was bare.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn bare_values_return_to_an_owner() {
    init();
    let mut bare = Buf::new().into_inner();
    assert_eq!(bare.as_mut().set_bytes(b""), Some(()));
    // SAFETY: `bare` came out of `into_inner` and only its safe setters wrote it since,
    // so it owns what it points at.
    let again: BufVal = unsafe { ffibox::CVal::new(bare) };
    assert!(again.as_ref().is_empty());

    let copy = Buf::from_bytes(b"source").unwrap();
    // SAFETY: `into_inner` gives up `copy`'s sole ownership of its allocation.
    let again: BufVal = unsafe { ffibox::CVal::new(copy.into_inner()) };
    assert_eq!(again.as_ref().as_bytes(), b"source");
}
