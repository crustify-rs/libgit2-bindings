//! The temporary stored-backend seam, called within its lifetime/compatibility
//! contract. Built-in iterators ignore this base field in next/free.
use libgit2::ffi;
use libgit2::libgit2::config::config::Config;
use std::ptr::NonNull;

#[test]
#[cfg_attr(miri, ignore = "calls C constructor and destructor")]
fn iterator_set_backend_raw_live_and_null() {
    // There is no wrapped backend constructor yet. This initialized, inert
    // caller-owned layout lives until after the iterator and never enters C.
    let mut backend = Box::new(ffi::git_config_backend::default());
    let backend_ptr = NonNull::from(&mut *backend);
    let config = Config::new().unwrap();
    let mut iter = config.as_ref().iterator().unwrap();
    {
        let mut m = iter.as_mut();
        // SAFETY: backend lives until after iterator destruction; these built-in
        // next/free callbacks ignore the backend slot, so invariants are preserved.
        unsafe { m.set_backend_raw(Some(backend_ptr)) };
        assert_eq!(m.as_ref().backend_raw(), Some(backend_ptr));
        assert!(m.advance().unwrap().is_none());
        // SAFETY: NULL is valid and ignored by this iterator's callbacks.
        unsafe { m.set_backend_raw(None) };
        assert!(m.as_ref().backend_raw().is_none());
        // SAFETY: same live backend and callback compatibility as above.
        unsafe { m.as_mut().set_backend_raw(Some(backend_ptr)) };
    }
    drop(iter);
    drop(config);
    drop(backend);
}
