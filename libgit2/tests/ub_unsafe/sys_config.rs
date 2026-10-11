//! Allocation-specific compatibility for the typed stored backend seam.
use libgit2::libgit2::config::config::Config;
use libgit2::libgit2::config::sys::config::Backend;

#[test]
#[cfg_attr(miri, ignore = "calls C constructor and destructor")]
fn iterator_set_backend_live_and_null() {
    let backend = Backend::new();
    let config = Config::new().unwrap();
    let mut iter = config.as_ref().iterator().unwrap();
    {
        let mut m = iter.as_mut();
        // SAFETY: backend lives until after iterator destruction; built-in
        // next/free ignore this slot, preserving their allocation invariants.
        unsafe { m.set_backend(Some(backend.as_ref())) };
        assert_eq!(m.as_ref().backend().unwrap().version(), 1);
        assert!(m.advance().unwrap().is_none());
        // SAFETY: NULL is valid and ignored by these next/free callbacks.
        unsafe { m.set_backend(None) };
        assert!(m.as_ref().backend().is_none());
        // SAFETY: same live backend and callback compatibility as above.
        unsafe { m.as_mut().set_backend(Some(backend.as_ref())) };
    }
    drop(iter);
    drop(config);
}
