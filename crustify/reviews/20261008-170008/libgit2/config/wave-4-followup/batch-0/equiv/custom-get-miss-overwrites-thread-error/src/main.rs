//! One store behind two configs: on one side each level is a backend written in C's
//! terms (its `get` returns `GIT_ENOTFOUND` on a miss and sets no error, as libgit2's
//! own backends do), on the other each level is the same store as a `CustomBackend`.
//! The higher level lacks the key, the lower one holds it. Both configs are driven
//! through the same routines; the return and the thread's last error are compared
//! after each step.
use libgit2::ffi;
use libgit2::libgit2::config::config::{Config, ConfigBox, ConfigError, ConfigLevel};
use libgit2::libgit2::config::sys::config::{
    BackendEntry, BackendEntryArc, CustomBackend, EntryOrigin, UnopenedBackend,
};
use libgit2::libgit2::util::errors::last;
use libgit2::libgit2::util::sys::errors::clear;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

const KEY: &CStr = c"core.bigfilethreshold";

/// A level's values.
#[derive(Clone)]
struct Store(Vec<(CString, CString)>);

impl Store {
    fn lookup(&self, key: &CStr) -> Option<&(CString, CString)> {
        self.0.iter().find(|(k, _)| k.as_c_str() == key)
    }
}

// --- The store as a CustomBackend ----------------------------------------------------

impl CustomBackend for Store {
    type Entries = std::vec::IntoIter<Result<BackendEntryArc, ConfigError>>;
    fn get(&mut self, key: &CStr) -> Option<BackendEntryArc> {
        let (k, v) = self.lookup(key)?;
        Some(BackendEntry::new(
            k,
            Some(v),
            ConfigLevel::APP,
            EntryOrigin::default(),
        ))
    }
    fn iterator(&mut self) -> Result<Self::Entries, ConfigError> {
        Ok(Vec::new().into_iter())
    }
    fn snapshot(&mut self) -> Result<UnopenedBackend, ConfigError> {
        Ok(UnopenedBackend::new_readonly(self.clone()))
    }
}

// --- The store as a C author's backend --------------------------------------------

#[repr(C)]
struct Raw {
    base: ffi::git_config_backend,
    store: Store,
}

#[repr(C)]
struct RawEntry {
    base: ffi::git_config_backend_entry,
    strings: (CString, CString),
}

unsafe extern "C" fn raw_entry_free(entry: *mut ffi::git_config_backend_entry) {
    // SAFETY: installed only on `RawEntry` boxes, each released once.
    drop(unsafe { Box::from_raw(entry.cast::<RawEntry>()) });
}

unsafe extern "C" fn raw_open(
    _: *mut ffi::git_config_backend,
    _: ffi::git_config_level_t,
    _: *const ffi::git_repository,
) -> c_int {
    0
}

unsafe extern "C" fn raw_get(
    p: *mut ffi::git_config_backend,
    key: *const c_char,
    out: *mut *mut ffi::git_config_backend_entry,
) -> c_int {
    // SAFETY: libgit2 passes the live backend, a live key and a writable slot.
    let (this, key) = unsafe { (&*p.cast::<Raw>(), CStr::from_ptr(key)) };
    let Some((k, v)) = this.store.lookup(key) else {
        // SAFETY: as above. A miss is no error: libgit2's backends set none.
        unsafe { out.write(ptr::null_mut()) };
        return ffi::GIT_ENOTFOUND;
    };
    let mut entry = Box::new(RawEntry {
        base: ffi::git_config_backend_entry {
            entry: ffi::git_config_entry {
                name: ptr::null(),
                value: ptr::null(),
                backend_type: ptr::null(),
                origin_path: ptr::null(),
                include_depth: 0,
                level: ffi::GIT_CONFIG_LEVEL_APP,
            },
            free: Some(raw_entry_free),
        },
        strings: (k.clone(), v.clone()),
    });
    entry.base.entry.name = entry.strings.0.as_ptr();
    entry.base.entry.value = entry.strings.1.as_ptr();
    // SAFETY: as above; the caller releases the entry through its callback.
    unsafe { out.write(Box::into_raw(entry).cast()) };
    0
}

unsafe extern "C" fn raw_free(p: *mut ffi::git_config_backend) {
    // SAFETY: libgit2 releases the backend once.
    drop(unsafe { Box::from_raw(p.cast::<Raw>()) });
}

fn raw_backend(store: Store) -> *mut ffi::git_config_backend {
    let mut base = std::mem::MaybeUninit::uninit();
    // SAFETY: a writable slot for the supported version's template.
    let rc =
        unsafe { ffi::git_config_init_backend(base.as_mut_ptr(), ffi::GIT_CONFIG_BACKEND_VERSION) };
    assert_eq!(rc, 0);
    // SAFETY: initialised by the call above.
    let mut base = unsafe { base.assume_init() };
    base.open = Some(raw_open);
    base.get = Some(raw_get);
    base.free = Some(raw_free);
    Box::into_raw(Box::new(Raw { base, store })).cast()
}

// --- Driving both ---------------------------------------------------------------

fn stores() -> [(Store, ConfigLevel); 2] {
    let high = Store(vec![(c"core.editor".to_owned(), c"vi".to_owned())]);
    // A value whose text after the `k` suffix `git_config_parse_int64` refuses
    // without describing the failure.
    let low = Store(vec![(KEY.to_owned(), c"1kb".to_owned())]);
    [(high, ConfigLevel::APP), (low, ConfigLevel::LOCAL)]
}

fn c_config() -> ConfigBox<'static> {
    let config = Config::new().unwrap();
    for (store, level) in stores() {
        let backend = raw_backend(store);
        // SAFETY: a live config and a complete, unopened backend it takes on success.
        let rc = unsafe {
            ffi::git_config_add_backend(config.as_c_ptr(), backend, level.raw(), ptr::null(), 0)
        };
        assert_eq!(rc, 0);
    }
    config
}

fn rust_config() -> ConfigBox<'static> {
    let mut config = Config::new().unwrap();
    for (store, level) in stores() {
        let backend = UnopenedBackend::new(store);
        config
            .as_mut()
            .add_backend(backend, level, None, false)
            .unwrap();
    }
    config
}

/// The thread's last error: class and message.
fn last_error() -> (c_int, CString) {
    let error = last();
    (error.raw_class(), error.message().to_owned())
}

/// A raw routine's return and the error it left, starting from a cleared state.
fn raw_step(
    config: &ConfigBox<'_>,
    step: fn(*mut ffi::git_config) -> c_int,
) -> (c_int, (c_int, CString)) {
    clear();
    let rc = step(config.as_c_ptr());
    (rc, last_error())
}

fn get_int64(config: *mut ffi::git_config) -> c_int {
    let mut out = 0;
    // SAFETY: a live config and key, a writable scalar.
    unsafe { ffi::git_config_get_int64(&mut out, config, KEY.as_ptr()) }
}

fn get_entry(config: *mut ffi::git_config) -> c_int {
    let mut entry = ptr::null_mut();
    // SAFETY: a live config and key, a writable slot.
    let rc = unsafe { ffi::git_config_get_entry(&mut entry, config, KEY.as_ptr()) };
    // SAFETY: NULL or the reference the lookup acquired, released once.
    unsafe { ffi::git_config_entry_free(entry) };
    rc
}

fn main() {
    let c = c_config();
    let rust = rust_config();

    // 1. A lookup the lower level answers: success on both, and no error left.
    let (c_entry, rust_entry) = (raw_step(&c, get_entry), raw_step(&rust, get_entry));
    println!("git_config_get_entry  c    {c_entry:?}");
    println!("git_config_get_entry  rust {rust_entry:?}");

    // 2. git_config_get_int64 on the stored "1kb": C fails without an error.
    let (c_int64, rust_int64) = (raw_step(&c, get_int64), raw_step(&rust, get_int64));
    println!("git_config_get_int64  c    {c_int64:?}");
    println!("git_config_get_int64  rust {rust_int64:?}");

    // 3. The same lookup through the safe wrapper, which describes C's silent failure.
    let describe = |config: &ConfigBox<'_>| {
        let error = config.as_ref().get_i64(KEY).unwrap_err();
        (
            error.raw_code(),
            error.raw_class(),
            error.message().to_owned(),
        )
    };
    let (c_safe, rust_safe) = (describe(&c), describe(&rust));
    println!("ConfigRef::get_i64    c    {c_safe:?}");
    println!("ConfigRef::get_i64    rust {rust_safe:?}");

    assert_eq!(
        rust_entry, c_entry,
        "git_config_get_entry leaves a different error"
    );
    assert_eq!(
        rust_int64, c_int64,
        "git_config_get_int64 leaves a different error"
    );
    assert_eq!(
        rust_safe, c_safe,
        "ConfigRef::get_i64 reports a different failure"
    );
}
