//! Public safe surface of `sys/config.h`: memory-options initialization,
//! replacement, copies and exclusive reborrows; backend entries' retained storage
//! and release; iterators' construction, lending, filtering and release points;
//! Rust-implemented backends' registration, dispatch, kept lookups, reentry and release.
//! The lifetime/aliasing failures are compile-fail doctests.
#![forbid(unsafe_code)]

use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
use std::ffi::CString;

#[test]
#[cfg_attr(
    miri,
    ignore = "constructor calls libgit2; Rust-only projections tested inline"
)]
fn memory_options_reborrow_and_replace() {
    let label = CString::new("command line").unwrap();
    let origin = CString::new(vec![b'/', 0xff, b'x']).unwrap();
    let mut options = BackendMemoryOptions::new();
    assert_eq!(options.as_ref().version(), 1);
    assert_eq!(options.as_ref().backend_type(), None);
    assert_eq!(options.as_ref().origin_path(), None);
    {
        let mut exclusive = options.as_mut();
        exclusive.as_mut().set_backend_type(Some(&label));
        exclusive.as_mut().set_origin_path(Some(&origin));
        let shared = exclusive.as_ref();
        let copied = shared;
        assert_eq!(shared.backend_type(), Some(label.as_c_str()));
        assert_eq!(copied.origin_path(), Some(origin.as_c_str()));
    }
    let clone = {
        // The original goes out of scope before the clone is read.
        let mut options = options;
        let clone = options.clone();
        options.as_mut().set_backend_type(Some(c""));
        options.as_mut().set_origin_path(Some(c""));
        assert_eq!(options.as_ref().backend_type(), Some(c""));
        assert_eq!(options.as_ref().origin_path(), Some(c""));
        options.as_mut().set_backend_type(None);
        options.as_mut().set_origin_path(None);
        options.as_mut().set_version(u32::MAX);
        assert_eq!(options.as_ref().version(), u32::MAX);
        assert_eq!(options.as_ref().backend_type(), None);
        assert_eq!(options.as_ref().origin_path(), None);
        clone
    };
    // The clone keeps the same immutable borrow, not a freed owned pointer.
    assert_eq!(clone.as_ref().backend_type(), Some(label.as_c_str()));
    assert_eq!(clone.as_ref().origin_path(), Some(origin.as_c_str()));
    assert_eq!(label.as_bytes(), b"command line");
    assert_eq!(origin.as_bytes(), &[b'/', 0xff, b'x']);
    let defaults = BackendMemoryOptions::default();
    assert_eq!(defaults.as_ref().version(), 1);
    assert_eq!(defaults.as_ref().backend_type(), None);
}

#[test]
#[cfg_attr(miri, ignore = "constructor calls libgit2")]
fn memory_options_cross_threads() {
    static LABEL: &std::ffi::CStr = c"shared";
    let origin = CString::new("scoped origin").unwrap();
    let origin = origin.as_c_str();
    let mut options = BackendMemoryOptions::new();
    options.as_mut().set_backend_type(Some(LABEL));
    options.as_mut().set_origin_path(Some(origin));
    // Concurrent readers through the owner and through copied shared handles.
    std::thread::scope(|s| {
        let shared = options.as_ref();
        for _ in 0..4 {
            s.spawn(move || {
                assert_eq!(shared.backend_type(), Some(LABEL));
                assert_eq!(shared.origin_path(), Some(origin));
            });
        }
        let owner = &options;
        s.spawn(move || assert_eq!(owner.as_ref().version(), 1));
    });
    // An exclusive handle writes on another thread; the owner reads it afterwards.
    std::thread::scope(|s| {
        let mut exclusive = options.as_mut();
        s.spawn(move || exclusive.set_origin_path(None));
    });
    assert_eq!(options.as_ref().origin_path(), None);
    // Options holding only 'static borrows move to a detached thread.
    let mut detached = BackendMemoryOptions::new();
    detached.as_mut().set_backend_type(Some(LABEL));
    let observed = std::thread::spawn(move || {
        let r = detached.as_ref();
        (
            r.version(),
            r.backend_type().map(ToOwned::to_owned),
            r.origin_path().is_none(),
        )
    })
    .join()
    .unwrap();
    assert_eq!(observed, (1, Some(LABEL.to_owned()), true));
}

mod backend_entries {
    use libgit2::libgit2::config::config::Config;
    use libgit2::libgit2::config::sys::config::{BackendEntry, BackendEntryArc};
    use std::ffi::CString;

    #[test]
    #[cfg_attr(miri, ignore = "acquisition and release call libgit2")]
    fn backend_entry_retained_storage_and_release() {
        let dir = std::env::temp_dir().join(format!(
            "crustify-backend-entry-safe-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(
            &file,
            b"[test]\nvalue = first\nflag\nempty =\n[include]\npath = extra\n",
        )
        .unwrap();
        std::fs::write(dir.join("extra"), b"[test]\nnested = included\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let mut config = Config::open(&path).unwrap();
        let first = BackendEntry::from_entry(config.as_ref().entry(c"TEST.value").unwrap());
        let second = BackendEntry::from_entry(config.as_ref().entry(c"test.value").unwrap());
        let flag = BackendEntry::from_entry(config.as_ref().entry(c"test.flag").unwrap());
        let empty = BackendEntry::from_entry(config.as_ref().entry(c"test.empty").unwrap());
        let nested = BackendEntry::from_entry(config.as_ref().entry(c"test.nested").unwrap());
        {
            let borrowed = first.as_ref();
            let copy = borrowed;
            assert_eq!(copy.entry().name(), Some(c"test.value"));
            assert_eq!(borrowed.entry().value(), Some(c"first"));
            assert_eq!(borrowed.entry().origin_path(), Some(path.as_c_str()));
        }
        assert_eq!(flag.as_ref().entry().value(), None);
        assert_eq!(empty.as_ref().entry().value(), Some(c""));
        assert_eq!(nested.as_ref().entry().include_depth(), 1);
        // An entry owner does not borrow its config: refreshing cannot invalidate it.
        let bytes = CString::new([b'x', 0xff]).unwrap();
        config.as_mut().set_string(c"test.value", &bytes).unwrap();
        let changed = BackendEntry::from_entry(config.as_ref().entry(c"test.value").unwrap());
        assert_eq!(first.as_ref().entry().value(), Some(c"first"));
        assert_eq!(changed.as_ref().entry().value(), Some(bytes.as_c_str()));
        let policy = first.as_ref().release_policy();
        let public = BackendEntry::into_entry(first);
        assert_eq!(public.as_ref().value(), Some(c"first"));
        let first = BackendEntry::from_entry(public);
        drop(config);
        drop(bytes);
        // No parent or input string remains: backing storage owns the copies.
        assert_eq!(
            changed.as_ref().entry().value().unwrap().to_bytes(),
            &[b'x', 0xff]
        );
        policy.release(first);
        assert_eq!(second.as_ref().entry().value(), Some(c"first"));
        assert_eq!(nested.as_ref().entry().value(), Some(c"included"));
        // A strategy from a now-released entry recovers the second owner's callback.
        policy.release(second);
        drop((flag, empty, nested, changed));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `Debug` reads every string through the retained list, after the config,
    /// its files and a sibling owner are gone.
    #[test]
    #[cfg_attr(miri, ignore = "acquisition and release call libgit2")]
    fn backend_entry_debug_reads_retained_storage() {
        let dir = std::env::temp_dir().join(format!(
            "crustify-backend-entry-debug-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config");
        std::fs::write(&file, b"[a]\nkey = value\n[include]\npath = extra\n").unwrap();
        std::fs::write(dir.join("extra"), b"[a]\nnested\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let config = Config::open(&path).unwrap();
        let mut entries: Vec<BackendEntryArc> = [c"a.key", c"a.nested"]
            .into_iter()
            .map(|key| BackendEntry::from_entry(config.as_ref().entry(key).unwrap()))
            .collect();
        drop(config);
        std::fs::remove_dir_all(dir).unwrap();
        let printed: Vec<String> = entries
            .iter()
            .map(|e| format!("{:?}", e.as_ref()))
            .collect();
        let origin = format!("{:?}", Some(path.as_c_str()));
        assert_eq!(
            printed[0],
            format!(
                "BackendEntry {{ entry: Entry {{ name: Some(\"a.key\"), value: Some(\"value\"), \
                 backend_type: Some(\"file\"), origin_path: {origin}, level: ConfigLevel(5), \
                 include_depth: 0 }}, .. }}"
            )
        );
        assert!(printed[1].contains("name: Some(\"a.nested\"), value: None"));
        assert!(printed[1].ends_with("include_depth: 1 }, .. }"));
        let last = entries.pop().unwrap();
        drop(entries);
        assert_eq!(format!("{:?}", last.as_ref()), printed[1]);
    }
}

mod iterators {
    use libgit2::libgit2::config::config::{Config, ConfigLevel};
    use libgit2::libgit2::config::sys::config::IteratorNext;
    use std::ffi::CString;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterator_lifecycle_reborrow_and_callbacks() {
        let dir =
            std::env::temp_dir().join(format!("crustify-iterator-safe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let low = dir.join("low");
        let high = dir.join("high");
        let empty_file = dir.join("empty");
        std::fs::write(&empty_file, b"").unwrap();
        std::fs::write(&low, b"[a]\nx = one\nx = two\nflag\nempty =\n").unwrap();
        std::fs::write(&high, b"[a]\nx = high\n").unwrap();
        let low_path = CString::new(low.to_str().unwrap()).unwrap();
        let high_path = CString::new(high.to_str().unwrap()).unwrap();
        let mut config = Config::open(&low_path).unwrap();
        let empty_path = CString::new(empty_file.to_str().unwrap()).unwrap();
        config
            .as_mut()
            .add_file(&empty_path, ConfigLevel::SYSTEM, None, false)
            .unwrap();
        config
            .as_mut()
            .add_file(&high_path, ConfigLevel::GLOBAL, None, false)
            .unwrap();
        let mut iter = config.as_ref().iterator().unwrap();
        let callback = iter.as_ref().next_callback();
        let policy = iter.as_ref().release_policy();
        assert_eq!(iter.as_ref().flags(), 0);
        assert!(iter.as_ref().backend().is_none());
        {
            let mut m = iter.as_mut();
            m.as_mut().set_flags(u32::MAX);
            assert_eq!(m.as_ref().flags(), u32::MAX);
            let shared = m.as_ref();
            let copied = shared;
            assert!(copied.backend().is_none());
            assert!(format!("{shared:?}").contains("flags: 4294967295"));
            m.set_flags(0);
            let mut count = 0;
            while let Some(entry) = callback.call(&mut m).unwrap() {
                assert!(entry.entry().name().is_some());
                // Release token is harmless for a borrowed entry; it cannot free it.
                let _ = entry.release_policy();
                count += 1;
            }
            assert_eq!(count, 5);
            assert!(m.advance().unwrap().is_none());
            assert!(IteratorNext.call(&mut m).unwrap().is_none());
        }
        policy.release(iter);
        // Input regex can be destroyed before advancing the C-compiled regex.
        let mut matched = {
            let regexp = CString::new("^a\\.x$").unwrap();
            config.as_ref().iterator_matching(Some(&regexp)).unwrap()
        };
        let mut count = 0;
        while let Some(entry) = matched.as_mut().advance().unwrap() {
            assert_eq!(entry.entry().name(), Some(c"a.x"));
            count += 1;
        }
        assert_eq!(count, 3);
        // A policy/callable from another iterator always dispatches THIS allocation.
        policy.release(matched);
        let mut values = {
            let key = CString::new("A.x").unwrap();
            let regex = CString::new("^(one|high)$").unwrap();
            config
                .as_ref()
                .multivar_iterator(&key, Some(&regex))
                .unwrap()
        };
        let mut seen = vec![];
        while let Some(entry) = callback.call(&mut values.as_mut()).unwrap() {
            seen.push(entry.entry().value().unwrap().to_owned());
        }
        assert_eq!(seen, [c"high".to_owned(), c"one".to_owned()]);
        policy.release(values);
        // Early release while nested backend state is still live.
        let mut early = config.as_ref().iterator().unwrap();
        assert!(early.as_mut().advance().unwrap().is_some());
        drop(early);
        let mut empty = config
            .as_ref()
            .multivar_iterator(c"a.missing", None)
            .unwrap();
        assert!(empty.as_mut().advance().unwrap().is_none());
        drop(empty);
        drop(config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterator_empty_and_failed_construction() {
        let config = Config::new().unwrap();
        for mut iter in [
            config.as_ref().iterator().unwrap(),
            config.as_ref().iterator_matching(None).unwrap(),
            config.as_ref().iterator_matching(Some(c"nomatch")).unwrap(),
            config.as_ref().multivar_iterator(c"a.b", None).unwrap(),
        ] {
            assert!(iter.as_mut().advance().unwrap().is_none());
            assert!(iter.as_mut().advance().unwrap().is_none());
        }
        assert!(config.as_ref().iterator_matching(Some(c"[")).is_err());
        assert!(config.as_ref().multivar_iterator(c"invalid", None).is_err());
        // Avoids upstream C's normalized-name leak on invalid regex.
        assert!(
            config
                .as_ref()
                .multivar_iterator(c"a.b", Some(c"["))
                .is_err()
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2 and PCRE2")]
    fn iterator_regex_valueless_and_non_utf8() {
        let file =
            std::env::temp_dir().join(format!("crustify-iterator-null-{}", std::process::id()));
        std::fs::write(&file, b"[a]\nx\nx =\nx = \xff\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let config = Config::open(&path).unwrap();
        let mut empty = config
            .as_ref()
            .multivar_iterator(c"a.x", Some(c"^$"))
            .unwrap();
        assert_eq!(
            empty.as_mut().advance().unwrap().unwrap().entry().value(),
            Some(c"")
        );
        assert!(empty.as_mut().advance().unwrap().is_none());
        let mut binary = config
            .as_ref()
            .multivar_iterator(c"a.x", Some(c"^\xff$"))
            .unwrap();
        assert_eq!(
            binary.as_mut().advance().unwrap().unwrap().entry().value(),
            Some(c"\xff")
        );
        assert!(binary.as_mut().advance().unwrap().is_none());
        let mut all = config
            .as_ref()
            .multivar_iterator(c"a.x", Some(c".*"))
            .unwrap();
        let callback = all.as_ref().next_callback();
        let mut count = 0;
        while let Some(entry) = callback.call(&mut all.as_mut()).unwrap() {
            assert!(entry.entry().value().is_some());
            count += 1;
        }
        assert_eq!(count, 2);
        drop((empty, binary, all));
        drop(config);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterator_foreign_refresh_error_and_retry() {
        let file =
            std::env::temp_dir().join(format!("crustify-iterator-refresh-{}", std::process::id()));
        std::fs::write(&file, b"[a]\nb = before\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let config = Config::open(&path).unwrap();
        let second = Config::open(&path).unwrap();
        let mut filtered = second
            .as_ref()
            .multivar_iterator(c"a.b", Some(c".*"))
            .unwrap();
        let mut iter = config.as_ref().iterator().unwrap();
        // Backend is opened lazily on advance: C writes its state after the
        // constructor's Rust reborrow has ended. No reference covers C storage.
        std::fs::write(&file, b"[unterminated\n").unwrap();
        assert!(iter.as_mut().advance().is_err());
        assert!(filtered.as_mut().advance().is_err());
        assert!(filtered.as_mut().advance().unwrap().is_none());
        drop(filtered);
        drop(second);
        assert!(iter.as_mut().advance().unwrap().is_none());
        drop(iter);
        std::fs::write(&file, b"[a]\nb = a much longer changed value\n").unwrap();
        let mut iter = config.as_ref().iterator().unwrap();
        assert_eq!(
            iter.as_mut().advance().unwrap().unwrap().entry().value(),
            Some(c"a much longer changed value")
        );
        drop(iter);
        drop(config);
        std::fs::remove_file(file).unwrap();
    }

    /// The Rust-owned filter's lifecycle: released before, during and after
    /// iteration (its inner C iterator and compiled regex still live), over a
    /// config without backends, and after a failed compile whose name was valid.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2 and PCRE2")]
    fn filtered_iterator_release_points() {
        let file =
            std::env::temp_dir().join(format!("crustify-filtered-release-{}", std::process::id()));
        std::fs::write(&file, b"[a]\nx = one\nx\nx = two\n[b]\nx = one\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let config = Config::open(&path).unwrap();
        let filtered = |regex: &std::ffi::CStr| {
            config
                .as_ref()
                .multivar_iterator(c"a.x", Some(regex))
                .unwrap()
        };
        drop(filtered(c"o"));
        let mut mid = filtered(c"o");
        let policy = mid.as_ref().release_policy();
        assert_eq!(
            mid.as_mut().advance().unwrap().unwrap().entry().value(),
            Some(c"one")
        );
        policy.release(mid);
        let mut done = filtered(c"o");
        let mut m = done.as_mut();
        let values: Vec<_> = std::iter::from_fn(|| {
            m.advance()
                .unwrap()
                .map(|e| e.entry().value().unwrap().to_owned())
        })
        .collect();
        assert_eq!(values, [c"one".to_owned(), c"two".to_owned()]);
        assert!(m.advance().unwrap().is_none());
        drop(done);
        let empty = Config::new().unwrap();
        let mut none = empty
            .as_ref()
            .multivar_iterator(c"a.x", Some(c".*"))
            .unwrap();
        assert!(none.as_mut().advance().unwrap().is_none());
        assert!(none.as_mut().advance().unwrap().is_none());
        drop(none);
        for _ in 0..4 {
            let error = config
                .as_ref()
                .multivar_iterator(c"a.x", Some(c"("))
                .unwrap_err();
            assert_eq!(error.message(), c"missing closing parenthesis");
        }
        drop((config, empty));
        std::fs::remove_file(file).unwrap();
    }
}

mod custom_backends {
    //! Rust stores behind `git_config_backend`: registration and release, every slot
    //! through the config and the backend handles, lookups that outlive their config
    //! and snapshot, reentry through a config the store keeps, failing walks, and
    //! threads that each drive their own.
    use libgit2::libgit2::config::config::{
        Config, ConfigArc, ConfigError, ConfigLevel, EntryRef, ForeachCallback,
    };
    use libgit2::libgit2::config::sys::config::{
        BackendEntry, BackendEntryArc, CustomBackend, EntryOrigin, UnopenedBackend,
    };
    use libgit2::libgit2::core::types::Signature;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryRef};
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};
    use std::cell::{Cell, RefCell};
    use std::ffi::{CStr, CString};
    use std::ops::ControlFlow;
    use std::rc::Rc;

    /// The calls a store saw, and its releases, shared with the test.
    #[derive(Default)]
    struct Log {
        calls: RefCell<Vec<String>>,
        drops: Cell<usize>,
    }

    impl Log {
        fn take(&self) -> Vec<String> {
            self.calls.take()
        }
    }

    /// A multivar-capable store in insertion order, which a transaction stages.
    #[derive(Clone, Default)]
    struct Values(Vec<(CString, CString)>);

    struct Store {
        values: Values,
        staged: Option<Values>,
        log: Rc<Log>,
        /// A config through which the store reaches its own backend, when set.
        own: Rc<RefCell<Option<ConfigArc<'static>>>>,
    }

    impl Store {
        fn new(values: &[(&CStr, &CStr)], log: &Rc<Log>) -> Self {
            Self {
                values: Values(
                    values
                        .iter()
                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                        .collect(),
                ),
                staged: None,
                log: log.clone(),
                own: Rc::default(),
            }
        }

        fn record(&self, call: String) {
            self.log.calls.borrow_mut().push(call);
        }

        fn writing(&mut self) -> &mut Values {
            self.staged.as_mut().unwrap_or(&mut self.values)
        }

        fn entry(name: &CStr, value: &CStr) -> BackendEntryArc {
            let origin = EntryOrigin {
                backend_type: Some(c"rust"),
                origin_path: Some(c"memory"),
                include_depth: 0,
            };
            BackendEntry::new(name, Some(value), ConfigLevel::APP, origin)
        }

        /// What the store's own config answers while one of its callbacks runs.
        fn reach_own(&self) {
            let own = self.own.borrow();
            let Some(own) = own.as_ref() else { return };
            let lookup = own.as_ref().get_bool(c"own.flag").map(drop);
            // The config's walk asks the backend for its iterator on the first advance.
            let walk = own
                .as_ref()
                .iterator()
                .and_then(|mut iter| iter.as_mut().advance().map(drop));
            let snapshot = own.as_ref().snapshot().map(drop);
            for (name, result) in [("get", lookup), ("iterator", walk), ("snapshot", snapshot)] {
                let error = result.unwrap_err();
                self.record(format!("own {name}: {:?} {}", error.code(), error));
            }
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            self.log.drops.set(self.log.drops.get() + 1);
        }
    }

    fn not_unique() -> ConfigError {
        ConfigError::new(ErrorClass::Config, c"entry is not unique").with_code(ErrorCode::Exists)
    }

    fn missing(key: &CStr) -> ConfigError {
        let message = format!("could not find key '{}' to delete", key.to_str().unwrap());
        ConfigError::new(ErrorClass::Config, &CString::new(message).unwrap())
            .with_code(ErrorCode::NotFound)
    }

    impl CustomBackend for Store {
        type Entries = std::vec::IntoIter<Result<BackendEntryArc, ConfigError>>;

        fn open(
            &mut self,
            level: ConfigLevel,
            repository: Option<RepositoryRef<'_>>,
        ) -> Result<(), ConfigError> {
            self.record(format!("open {} {}", level.raw(), repository.is_some()));
            if self
                .values
                .0
                .iter()
                .any(|(k, _)| k.as_c_str() == c"open.fail")
            {
                return Err(ConfigError::new(
                    ErrorClass::Os,
                    c"cannot open: store refused",
                ));
            }
            Ok(())
        }
        fn get(&mut self, key: &CStr) -> Option<BackendEntryArc> {
            self.record(format!("get {}", key.to_str().unwrap()));
            self.reach_own();
            let (k, v) = self
                .values
                .0
                .iter()
                .rev()
                .find(|(k, _)| k.as_c_str() == key)?;
            Some(Self::entry(k, v))
        }
        fn iterator(&mut self) -> Result<Self::Entries, ConfigError> {
            self.record("iterator".to_owned());
            self.reach_own();
            if self
                .values
                .0
                .iter()
                .any(|(k, _)| k.as_c_str() == c"walk.none")
            {
                let error = ConfigError::new(ErrorClass::Config, c"nothing to walk");
                return Err(error.with_code(ErrorCode::NotFound));
            }
            let mut entries: Vec<_> = self
                .values
                .0
                .iter()
                .map(|(k, v)| Ok(Self::entry(k, v)))
                .collect();
            if self
                .values
                .0
                .iter()
                .any(|(k, _)| k.as_c_str() == c"walk.fail")
            {
                entries.insert(1, Err(ConfigError::new(ErrorClass::Config, c"walk failed")));
            }
            if self
                .values
                .0
                .iter()
                .any(|(k, _)| k.as_c_str() == c"walk.first")
            {
                entries.insert(0, Err(ConfigError::new(ErrorClass::Config, c"walk failed")));
            }
            Ok(entries.into_iter())
        }
        fn snapshot(&mut self) -> Result<UnopenedBackend, ConfigError> {
            self.record("snapshot".to_owned());
            let copy = Store {
                values: self.values.clone(),
                staged: None,
                log: self.log.clone(),
                own: Rc::default(),
            };
            // Writable as built: the snapshot slot marks it read-only.
            Ok(UnopenedBackend::new(copy))
        }
        fn set(&mut self, key: &CStr, value: &CStr) -> Result<(), ConfigError> {
            self.record(format!("set {}", key.to_str().unwrap()));
            let values = self.writing();
            let mut matching = values.0.iter_mut().filter(|(k, _)| k.as_c_str() == key);
            match (matching.next(), matching.next()) {
                (Some(_), Some(_)) => Err(not_unique()),
                (Some((_, v)), None) => {
                    *v = value.to_owned();
                    Ok(())
                }
                _ => {
                    values.0.push((key.to_owned(), value.to_owned()));
                    Ok(())
                }
            }
        }
        fn set_multivar(
            &mut self,
            key: &CStr,
            regexp: &CStr,
            value: &CStr,
        ) -> Result<(), ConfigError> {
            self.record(format!("set_multivar {}", key.to_str().unwrap()));
            let values = self.writing();
            let pattern = regexp.to_bytes();
            let mut replaced = false;
            for (k, v) in &mut values.0 {
                if k.as_c_str() == key && v.to_bytes().windows(pattern.len()).any(|w| w == pattern)
                {
                    *v = value.to_owned();
                    replaced = true;
                }
            }
            if !replaced {
                values.0.push((key.to_owned(), value.to_owned()));
            }
            Ok(())
        }
        fn delete(&mut self, key: &CStr) -> Result<(), ConfigError> {
            self.record(format!("del {}", key.to_str().unwrap()));
            let values = self.writing();
            match values.0.iter().filter(|(k, _)| k.as_c_str() == key).count() {
                0 => Err(missing(key)),
                1 => {
                    values.0.retain(|(k, _)| k.as_c_str() != key);
                    Ok(())
                }
                _ => Err(not_unique()),
            }
        }
        fn delete_multivar(&mut self, key: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
            self.record(format!("del_multivar {}", key.to_str().unwrap()));
            let values = self.writing();
            let pattern = regexp.to_bytes();
            let before = values.0.len();
            values.0.retain(|(k, v)| {
                k.as_c_str() != key || !v.to_bytes().windows(pattern.len()).any(|w| w == pattern)
            });
            if values.0.len() == before {
                Err(missing(key))
            } else {
                Ok(())
            }
        }
        fn lock(&mut self) -> Result<(), ConfigError> {
            self.record("lock".to_owned());
            self.staged = Some(self.values.clone());
            Ok(())
        }
        fn unlock(&mut self, commit: bool) -> Result<(), ConfigError> {
            self.record(format!("unlock {commit}"));
            let staged = self.staged.take().expect("unlocked without a lock");
            if commit {
                self.values = staged;
            }
            Ok(())
        }
    }

    fn registered(
        store: Store,
        readonly: bool,
    ) -> libgit2::libgit2::config::config::ConfigBox<'static> {
        let mut config = Config::new().unwrap();
        let backend = if readonly {
            UnopenedBackend::new_readonly(store)
        } else {
            UnopenedBackend::new(store)
        };
        config
            .as_mut()
            .add_backend(backend, ConfigLevel::APP, None, false)
            .unwrap();
        config
    }

    fn names(config: &libgit2::libgit2::config::config::ConfigBox<'_>) -> Vec<(CString, CString)> {
        let mut iter = config.as_ref().iterator().unwrap();
        let mut m = iter.as_mut();
        std::iter::from_fn(|| {
            m.advance().unwrap().map(|e| {
                let e = e.entry();
                (e.name().unwrap().to_owned(), e.value().unwrap().to_owned())
            })
        })
        .collect()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn custom_backend_serves_every_config_routine() {
        let log = Rc::new(Log::default());
        let store = Store::new(
            &[
                (c"core.editor", c"vi"),
                (c"multi.v", c"a1"),
                (c"multi.v", c"b2"),
            ],
            &log,
        );
        let mut config = registered(store, false);
        assert_eq!(
            log.take(),
            [format!("open {} false", ConfigLevel::APP.raw())]
        );
        let editor = config.as_ref().entry(c"core.editor").unwrap();
        assert_eq!(editor.as_ref().value(), Some(c"vi"));
        assert_eq!(editor.as_ref().backend_type(), Some(c"rust"));
        assert_eq!(editor.as_ref().origin_path(), Some(c"memory"));
        log.take();
        config.as_mut().set_string(c"core.editor", c"nano").unwrap();
        config.as_mut().set_bool(c"core.bare", true).unwrap();
        assert!(config.as_ref().get_bool(c"core.bare").unwrap());
        assert_eq!(
            config.as_ref().get_i32(c"core.missing").unwrap_err().code(),
            Some(ErrorCode::NotFound)
        );
        let not_unique = config.as_mut().set_string(c"multi.v", c"x").unwrap_err();
        assert_eq!(
            (not_unique.code(), not_unique.message()),
            (Some(ErrorCode::Exists), c"entry is not unique")
        );
        config
            .as_mut()
            .set_multivar(c"multi.v", c"b", c"b3")
            .unwrap();
        config.as_mut().delete_multivar(c"multi.v", c"a").unwrap();
        config.as_mut().delete_entry(c"core.bare").unwrap();
        let gone = config.as_mut().delete_entry(c"core.bare").unwrap_err();
        assert_eq!(gone.message(), c"could not find key 'core.bare' to delete");
        assert_eq!(
            names(&config),
            [
                (c"core.editor".to_owned(), c"nano".to_owned()),
                (c"multi.v".to_owned(), c"b3".to_owned())
            ]
        );
        let mut seen = Vec::new();
        let mut each = |entry: EntryRef<'_>| {
            seen.push(entry.name().unwrap().to_owned());
            ControlFlow::Continue(())
        };
        let walked = config
            .as_ref()
            .foreach(&mut ForeachCallback::new(&mut each))
            .unwrap();
        assert_eq!(walked, ControlFlow::Continue(()));
        assert_eq!(seen, [c"core.editor".to_owned(), c"multi.v".to_owned()]);
        // The entry looked up before the writes still reads its own copy.
        assert_eq!(editor.as_ref().value(), Some(c"vi"));
        {
            let transaction = config.as_mut().lock().unwrap();
            assert!(transaction.config().get_bool(c"core.none").is_err());
        }
        let calls = log.take();
        assert!(
            calls.ends_with(&[
                "lock".to_owned(),
                "get core.none".to_owned(),
                "unlock false".to_owned()
            ]),
            "{calls:?}"
        );
        let level = config.as_ref().open_level(ConfigLevel::APP).unwrap();
        assert_eq!(
            level.as_ref().entry(c"multi.v").unwrap().as_ref().value(),
            Some(c"b3")
        );
        drop(level);
        drop(config);
        // The original store, and each snapshot taken for a lookup.
        assert!(log.drops.get() >= 2);
        assert_eq!(editor.as_ref().name(), Some(c"core.editor"));
        drop(editor);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn readonly_lookups_outlive_their_entries_config_and_snapshot() {
        let log = Rc::new(Log::default());
        let store = Store::new(
            &[
                (c"user.name", c"Rust Store"),
                (c"user.email", c"store@example.com"),
            ],
            &log,
        );
        let config = registered(store, true);
        // A read-only config: `git_config_get_string` frees the entry it found and
        // returns its value, which the backend keeps.
        assert_eq!(
            config.as_ref().get_string(c"user.name").unwrap(),
            c"Rust Store".to_owned()
        );
        assert_eq!(
            config.as_ref().get_string(c"user.name").unwrap(),
            c"Rust Store".to_owned()
        );
        let error = config.as_ref().get_string(c"user.missing").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::NotFound));
        let writable = registered(Store::new(&[(c"user.name", c"w")], &log), false);
        let snapshot = writable.as_ref().snapshot().unwrap();
        drop(writable);
        assert_eq!(
            snapshot.as_ref().get_string(c"user.name").unwrap(),
            c"w".to_owned()
        );
        assert!(snapshot.as_ref().open_level(ConfigLevel::APP).is_ok());
        let entry = snapshot.as_ref().entry(c"user.name").unwrap();
        drop(snapshot);
        assert_eq!(entry.as_ref().value(), Some(c"w"));
        // libgit2 reads the kept values itself: a signature from a repository whose
        // configuration is the read-only store's, through a snapshot.
        let mut repo = Repository::new().unwrap();
        let shared = ConfigArc::from(config);
        repo.as_mut().set_config(&shared);
        drop(shared);
        let signature = Signature::default(repo.as_ref()).unwrap();
        assert_eq!(signature.as_ref().name(), Some(c"Rust Store"));
        assert_eq!(signature.as_ref().email(), Some(c"store@example.com"));
        let drops = log.drops.get();
        drop(repo);
        assert_eq!(log.drops.get(), drops + 1);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn backend_handles_dispatch_to_the_store() {
        let log = Rc::new(Log::default());
        let store = Store::new(&[(c"a.b", c"one")], &log);
        let unopened = UnopenedBackend::new(store);
        assert!(!unopened.as_ref().is_readonly());
        assert!(format!("{unopened:?}").starts_with("UnopenedBackend(Backend {"));
        let mut backend = unopened.open(ConfigLevel::WORKTREE).unwrap();
        assert_eq!(
            log.take(),
            [format!("open {} false", ConfigLevel::WORKTREE.raw())]
        );
        let mut b = backend.as_mut();
        assert_eq!(
            b.get(c"a.b").unwrap().as_ref().entry().value(),
            Some(c"one")
        );
        let missing = b.get(c"a.c").unwrap_err();
        assert_eq!(
            (missing.code(), missing.message()),
            (
                Some(ErrorCode::NotFound),
                c"config value 'a.c' was not found"
            )
        );
        b.set(c"a.c", c"two").unwrap();
        {
            let mut transaction = b.lock().unwrap();
            transaction.set(c"a.d", c"staged").unwrap();
            transaction.delete(c"a.b").unwrap();
            transaction.finish(true).unwrap();
        }
        let mut snapshot = b.snapshot().unwrap();
        assert!(snapshot.as_ref().is_readonly());
        drop(backend);
        let mut view = snapshot.as_mut();
        let mut iter = view.iterator().unwrap();
        let mut m = iter.as_mut();
        let mut walked = Vec::new();
        while let Some(entry) = m.advance().unwrap() {
            walked.push(entry.entry().name().unwrap().to_owned());
        }
        drop(iter);
        assert_eq!(walked, [c"a.c".to_owned(), c"a.d".to_owned()]);
        drop(snapshot);
        assert_eq!(log.drops.get(), 2);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn registration_failures_release_the_store() {
        let log = Rc::new(Log::default());
        let mut config = Config::new().unwrap();
        let failing = UnopenedBackend::new(Store::new(&[(c"open.fail", c"1")], &log));
        let error = config
            .as_mut()
            .add_backend(failing, ConfigLevel::LOCAL, None, false)
            .unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Os));
        assert_eq!(error.message(), c"cannot open: store refused");
        assert_eq!(log.drops.get(), 1);
        let first = UnopenedBackend::new(Store::new(&[], &log));
        config
            .as_mut()
            .add_backend(first, ConfigLevel::LOCAL, None, false)
            .unwrap();
        let duplicate = UnopenedBackend::new(Store::new(&[], &log));
        let error = config
            .as_mut()
            .add_backend(duplicate, ConfigLevel::LOCAL, None, false)
            .unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Exists));
        assert_eq!(log.drops.get(), 2);
        let repo = Repository::new().unwrap();
        let mut bound = Config::new().unwrap();
        let mut view = bound.as_mut();
        let backend = UnopenedBackend::new(Store::new(&[], &log));
        view.add_backend(backend, ConfigLevel::GLOBAL, Some(repo.as_ref()), false)
            .unwrap();
        assert_eq!(
            log.take().last().unwrap(),
            &format!("open {} true", ConfigLevel::GLOBAL.raw())
        );
        drop((bound, config));
        assert_eq!(log.drops.get(), 4);
        drop(repo);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reentry_through_a_kept_config_is_refused() {
        let log = Rc::new(Log::default());
        let store = Store::new(&[(c"a.b", c"one")], &log);
        let own = store.own.clone();
        *own.borrow_mut() = Some(ConfigArc::from(registered(store, false)));
        log.take();
        {
            let kept = own.borrow();
            let mut iter = kept.as_ref().unwrap().as_ref().iterator().unwrap();
            assert!(iter.as_mut().advance().unwrap().is_some());
        }
        let calls = log.take();
        assert_eq!(calls[0], "iterator");
        assert!(calls[1].starts_with("own get: Some(NotFound)"), "{calls:?}");
        assert_eq!(
            calls[2],
            "own iterator: Some(Generic) config backend reentered from its own callback"
        );
        assert_eq!(
            calls[3],
            "own snapshot: Some(Generic) config backend reentered from its own callback"
        );
        // Break the cycle the kept config makes.
        let kept = own.borrow_mut().take();
        assert_eq!(log.drops.get(), 0);
        drop(kept);
        assert_eq!(log.drops.get(), 1);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failing_walks_end_and_lookups_terminate() {
        let log = Rc::new(Log::default());
        let store = Store::new(&[(c"walk.fail", c"1"), (c"a.b", c"two")], &log);
        let config = registered(store, false);
        let mut iter = config.as_ref().iterator().unwrap();
        let mut m = iter.as_mut();
        assert_eq!(
            m.advance().unwrap().unwrap().entry().name(),
            Some(c"walk.fail")
        );
        assert_eq!(m.advance().unwrap_err().message(), c"walk failed");
        assert!(m.advance().unwrap().is_none());
        drop(iter);
        // `entry` rereads through a walk first, which the failure must not stall.
        let error = config.as_ref().entry(c"a.b").unwrap_err();
        assert_eq!(error.message(), c"walk failed");
        // A walk failing on its first entry: the config reports it, and the backend's
        // ended walk lets the next advance finish.
        let config = registered(Store::new(&[(c"walk.first", c"1")], &log), false);
        let mut iter = config.as_ref().iterator().unwrap();
        let mut m = iter.as_mut();
        assert_eq!(m.advance().unwrap_err().message(), c"walk failed");
        assert!(m.advance().unwrap().is_none());
        drop(iter);
        // A backend with nothing to walk says so as not found: the walk moves on.
        let mut config = registered(Store::new(&[(c"a.b", c"c")], &log), false);
        let none = UnopenedBackend::new(Store::new(&[(c"walk.none", c"1")], &log));
        config
            .as_mut()
            .add_backend(none, ConfigLevel::LOCAL, None, false)
            .unwrap();
        assert_eq!(names(&config), [(c"a.b".to_owned(), c"c".to_owned())]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn readonly_and_layered_backends() {
        let log = Rc::new(Log::default());
        let mut config = Config::new().unwrap();
        let low = Store::new(&[(c"a.low", c"1")], &log);
        let readonly = Store::new(&[(c"a.high", c"2")], &log);
        let writer = Store::new(&[], &log);
        let mut view = config.as_mut();
        view.add_backend(
            UnopenedBackend::new_readonly(readonly),
            ConfigLevel::APP,
            None,
            false,
        )
        .unwrap();
        // Nowhere to write: every write and lock is refused before a backend runs.
        let refused = [
            view.set_string(c"a.x", c"v"),
            view.set_multivar(c"a.x", c".", c"v"),
            view.delete_multivar(c"a.x", c"."),
        ];
        for error in refused.into_iter().map(Result::unwrap_err) {
            assert_eq!(error.code(), Some(ErrorCode::ReadOnly));
        }
        assert_eq!(
            config.as_mut().lock().unwrap_err().code(),
            Some(ErrorCode::ReadOnly)
        );
        let mut view = config.as_mut();
        view.add_backend(
            UnopenedBackend::new(writer),
            ConfigLevel::LOCAL,
            None,
            false,
        )
        .unwrap();
        let empty = UnopenedBackend::new(Store::new(&[], &log));
        view.add_backend(empty, ConfigLevel::GLOBAL, None, false)
            .unwrap();
        view.add_backend(UnopenedBackend::new(low), ConfigLevel::SYSTEM, None, false)
            .unwrap();
        log.take();
        // Writes skip the read-only backend for the highest writable one.
        config.as_mut().set_string(c"a.new", c"3").unwrap();
        assert_eq!(log.take(), ["set a.new"]);
        // A live config is refused C's borrowed-string lookup.
        let live = config.as_ref().get_string(c"a.high").unwrap_err();
        assert_eq!(live.message(), c"get_string called on a live config object");
        // The walk crosses every backend, lowest priority first, past an empty one.
        assert_eq!(
            names(&config),
            [
                (c"a.low".to_owned(), c"1".to_owned()),
                (c"a.new".to_owned(), c"3".to_owned()),
                (c"a.high".to_owned(), c"2".to_owned()),
            ]
        );
        drop(config);
        assert_eq!(log.drops.get(), 4);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn snapshots_that_fail_to_open_are_released() {
        let log = Rc::new(Log::default());
        // The store opened; a snapshot copies the key its `open` then refuses.
        let mut config = registered(Store::new(&[(c"a.b", c"c")], &log), false);
        config.as_mut().set_string(c"open.fail", c"now").unwrap();
        let error = config.as_ref().snapshot().unwrap_err();
        assert_eq!(error.message(), c"cannot open: store refused");
        assert_eq!(log.drops.get(), 1, "the snapshot that failed to open");
        let error = config.as_ref().entry(c"a.b").unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Os));
        drop(config);
        assert_eq!(log.drops.get(), 3);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_drive_their_own_backends() {
        let threads: Vec<_> = (0..4)
            .map(|i| {
                std::thread::spawn(move || {
                    let log = Rc::new(Log::default());
                    let mut config = registered(Store::new(&[], &log), false);
                    for round in 0..16 {
                        let value = CString::new(format!("{i}-{round}")).unwrap();
                        config.as_mut().set_string(c"t.value", &value).unwrap();
                        let entry = config.as_ref().entry(c"t.value").unwrap();
                        assert_eq!(entry.as_ref().value(), Some(value.as_c_str()));
                    }
                    drop(config);
                    log.drops.get()
                })
            })
            .collect();
        for thread in threads {
            assert!(thread.join().unwrap() >= 1);
        }
    }
}
