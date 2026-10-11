//! `BackendMut::get` (Field: git_config_backend.get) against the backend's own C
//! `get` slot, on a writable backend that implements `get` but not `snapshot`.
//!
//! Both sides get an independently owned instance of the same backend: a minimal
//! writable C-ABI backend whose `get` forwards to an opened in-memory backend.
use libgit2::ffi;
use libgit2::libgit2::config::sys::config::BackendBox;
use libgit2::libgit2::util::errors::last;
use std::ffi::{CStr, c_char};
use std::ptr;

#[repr(C)]
struct Writable {
    base: ffi::git_config_backend,
    inner: *mut ffi::git_config_backend,
}

impl Writable {
    unsafe extern "C" fn get(
        p: *mut ffi::git_config_backend,
        key: *const c_char,
        out: *mut *mut ffi::git_config_backend_entry,
    ) -> i32 {
        // SAFETY: installed only on complete Writable boxes; inner is opened and
        // owned by this allocation, and its get acquires a retained entry.
        unsafe {
            let inner = (*p.cast::<Self>()).inner;
            ((*inner).get.unwrap())(inner, key, out)
        }
    }

    unsafe extern "C" fn free(p: *mut ffi::git_config_backend) {
        // SAFETY: called once with the sole owner of the Writable box.
        let this = unsafe { Box::from_raw(p.cast::<Self>()) };
        // SAFETY: inner is uniquely owned by this allocation.
        unsafe { ((*this.inner).free.unwrap())(this.inner) };
    }

    fn new() -> *mut ffi::git_config_backend {
        let data = b"[a]\nb = value\n";
        let mut inner = ptr::null_mut();
        // SAFETY: readable input, writable out; C copies the bytes.
        let rc = unsafe {
            ffi::git_config_backend_from_string(&mut inner, data.as_ptr().cast(), data.len(), ptr::null_mut())
        };
        assert_eq!(rc, 0);
        // SAFETY: fresh memory backend, opened exactly once; NULL repo is ignored.
        let rc = unsafe { ((*inner).open.unwrap())(inner, ffi::GIT_CONFIG_LEVEL_LOCAL, ptr::null()) };
        assert_eq!(rc, 0);
        let base = ffi::git_config_backend {
            version: ffi::GIT_CONFIG_BACKEND_VERSION,
            readonly: 0,
            get: Some(Self::get),
            free: Some(Self::free),
            ..Default::default()
        };
        Box::into_raw(Box::new(Self { base, inner })).cast()
    }
}

fn value(entry: *const ffi::git_config_entry) -> Vec<u8> {
    // SAFETY: a live entry acquired by a successful get.
    unsafe { CStr::from_ptr((*entry).value) }.to_bytes().to_vec()
}

fn main() {
    let _runtime = libgit2::libgit2::core::global::init().unwrap();

    // C: dispatch the backend's own get slot.
    let raw = Writable::new();
    let mut out = ptr::null_mut();
    // SAFETY: opened writable backend, readable key, writable out slot.
    let rc = unsafe { ((*raw).get.unwrap())(raw, c"a.b".as_ptr(), &mut out) };
    let c = if rc == 0 {
        // SAFETY: success acquired one entry reference, released right after.
        let v = value(unsafe { ptr::addr_of!((*out).entry) });
        unsafe { ffi::git_config_entry_free(ptr::addr_of_mut!((*out).entry)) };
        Ok(v)
    } else {
        Err((rc, last().message().to_owned()))
    };
    // SAFETY: the sole owner of the C instance, released by its own callback.
    unsafe { ((*raw).free.unwrap())(raw) };

    // Rust: the same backend adopted through the documented ffibox seam.
    // SAFETY: complete, exclusively owned, already opened allocation whose
    // callbacks obey the public interface; the NULL slots are permitted.
    let mut backend = unsafe { BackendBox::from_c(Writable::new()) }.unwrap();
    let rust = match backend.as_mut().get(c"a.b") {
        Ok(entry) => Ok(entry.as_ref().entry().value().unwrap().to_bytes().to_vec()),
        Err(e) => Err((e.raw_code(), e.message().to_owned())),
    };
    drop(backend);

    println!("c    {c:?}");
    println!("rust {rust:?}");
    assert_eq!(rust, c, "BackendMut::get diverges from the backend's get slot");
}
