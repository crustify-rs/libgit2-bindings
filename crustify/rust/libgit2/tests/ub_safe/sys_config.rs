//! Public safe surface of `sys/config.h`: memory-options initialization,
//! replacement, copies and exclusive reborrows; backend entries' retained storage
//! and release; iterators' construction, lending, filtering and release points;
//! memory backends' construction, one-time open, dispatch and snapshot chains.
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

mod backends {
    use super::*;
    use libgit2::libgit2::config::config::ConfigLevel;
    use libgit2::libgit2::config::sys::config::{Backend, BackendOpen};

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn backend_copied_inputs_retained_entries_and_snapshots() {
        let pending = {
            let data = b"[a]\nb = one\nb = two\nflag\nempty =\n".to_vec();
            let label = CString::new(vec![b'l', 0xff]).unwrap();
            let origin = CString::new("origin").unwrap();
            let mut options = BackendMemoryOptions::new();
            options.as_mut().set_backend_type(Some(&label));
            options.as_mut().set_origin_path(Some(&origin));
            Backend::from_string(&data, Some(options.as_ref())).unwrap()
        };
        assert_eq!(pending.as_ref().version(), 1);
        assert!(pending.as_ref().is_readonly());
        assert!(pending.as_ref().config_identity().is_none());
        let open = pending.as_ref().open_callback();
        let mut backend = open.call(pending, ConfigLevel::APP).unwrap();
        let release = backend.as_ref().release_policy();
        let entry = backend.as_mut().get(c"a.b").unwrap();
        assert_eq!(entry.as_ref().entry().value(), Some(c"two"));
        assert_eq!(entry.as_ref().entry().level(), ConfigLevel::APP);
        assert_eq!(
            entry.as_ref().entry().backend_type().unwrap().to_bytes(),
            &[b'l', 0xff]
        );
        let mut snapshot = backend.as_mut().snapshot().unwrap();
        {
            let mut s = snapshot.as_mut();
            assert!(s.get(c"missing.key").is_err());
            assert!(s.set(c"a.b", c"v").is_err());
            assert!(s.set_multivar(c"a.b", c".*", c"v").is_err());
            assert!(s.delete(c"a.b").is_err());
            assert!(s.delete_multivar(c"a.b", c".*").is_err());
            assert!(s.lock().is_err());
        }
        {
            let mut b = backend.as_mut();
            let mut iter = b.iterator().unwrap();
            let mut count = 0;
            while let Some(entry) = iter.as_mut().advance().unwrap() {
                assert!(entry.entry().name().is_some());
                count += 1;
            }
            assert_eq!(count, 4);
        }
        release.release(backend);
        assert_eq!(entry.as_ref().entry().value(), Some(c"two"));
        assert_eq!(entry.as_ref().entry().origin_path(), Some(c"origin"));
        let flag = snapshot.as_mut().get(c"a.flag").unwrap();
        assert_eq!(flag.as_ref().entry().value(), None);
        let empty = snapshot.as_mut().get(c"a.empty").unwrap();
        assert_eq!(empty.as_ref().entry().value(), Some(c""));
        drop(snapshot);
        assert_eq!(flag.as_ref().entry().name(), Some(c"a.flag"));
        drop((flag, empty, entry));
        // A callable is not bound to the allocation that supplied its token.
        let _ = BackendOpen;
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn backend_values_readonly_callbacks_errors_and_failed_open() {
        let pending = {
            let value = CString::new(vec![b'a', b'.', b'b', b'=', 0xff]).unwrap();
            let mut options = BackendMemoryOptions::new();
            options.as_mut().set_backend_type(Some(c"values"));
            options.as_mut().set_origin_path(Some(c"values-origin"));
            Backend::from_values(&[&value, c"a.empty="], Some(options.as_ref())).unwrap()
        };
        let mut backend = pending.open(ConfigLevel::LOCAL).unwrap();
        let mut b = backend.as_mut();
        assert_eq!(
            b.get(c"a.b")
                .unwrap()
                .as_ref()
                .entry()
                .value()
                .unwrap()
                .to_bytes(),
            &[0xff]
        );
        let entry = b.get(c"a.b").unwrap();
        assert_eq!(entry.as_ref().entry().backend_type(), Some(c"values"));
        assert_eq!(entry.as_ref().entry().origin_path(), Some(c"values-origin"));
        assert!(b.get(c"missing.key").is_err());
        assert!(b.set(c"a.b", c"v").is_err());
        assert!(b.set_multivar(c"a.b", c".*", c"v").is_err());
        assert!(b.delete(c"a.b").is_err());
        assert!(b.delete_multivar(c"a.b", c".*").is_err());
        assert!(b.lock().is_err());
        let _ = b.as_ref().unlock_callback();
        assert!(Backend::from_values(&[c"missing-equals"], None).is_err());
        let bad = Backend::from_values(&[c"=emptykey"], None).unwrap();
        assert!(bad.open(ConfigLevel::LOCAL).is_err());
        let bad = Backend::from_string(b"[unterminated", None).unwrap();
        assert!(bad.open(ConfigLevel::LOCAL).is_err());
        // Pending storage is safely released even when never opened.
        drop(Backend::from_string(b"", None).unwrap());
        let mut empty = Backend::from_values(&[], None)
            .unwrap()
            .open(ConfigLevel::LOCAL)
            .unwrap();
        assert!(empty.as_mut().get(c"a.b").is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn backend_template_refuses_every_slot_and_snapshot_chains_outlive_sources() {
        let mut template = Backend::new();
        assert!(format!("{template:?}").starts_with("Backend { version: 1, readonly: 0,"));
        {
            let mut b = template.as_mut();
            let message = |e: libgit2::libgit2::config::config::ConfigError| {
                e.message().to_str().unwrap().to_owned()
            };
            let refused = "config backend does not implement";
            assert!(message(b.get(c"a.b").unwrap_err()).starts_with(refused));
            assert!(message(b.iterator().unwrap_err()).starts_with(refused));
            assert!(message(b.snapshot().unwrap_err()).starts_with(refused));
            assert!(message(b.lock().unwrap_err()).ends_with("'lock'"));
        }
        let pending = Backend::from_string(b"[a]\nb = one\n", None).unwrap();
        assert!(format!("{pending:?}").starts_with("UnopenedBackend(Backend {"));
        let mut source = pending.open(ConfigLevel::GLOBAL).unwrap();
        let mut first = source.as_mut().snapshot().unwrap();
        drop(source);
        let mut second = first.as_mut().snapshot().unwrap();
        let entry = first.as_mut().get(c"a.b").unwrap();
        drop(first);
        let mut b = second.as_mut();
        let mut iter = b.iterator().unwrap();
        let mut m = iter.as_mut();
        let lent = m.advance().unwrap().unwrap();
        assert_eq!(lent.entry().value(), entry.as_ref().entry().value());
        assert!(lent.entry().value() == Some(c"one"));
        drop(iter);
        drop(second);
        assert_eq!(entry.as_ref().entry().level(), ConfigLevel::GLOBAL);
    }
}

/// Registration consumes its owner; retained entries and copied snapshots remain
/// usable after replacement and after all registering configurations are dropped.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn backend_registration_transfer_replace_and_retained_entries() {
    use libgit2::libgit2::config::config::{Config, ConfigLevel};
    use libgit2::libgit2::config::sys::config::Backend;
    use libgit2::libgit2::util::errors::ErrorCode;

    let mut config = Config::new().unwrap();
    let values = [
        CString::new("a.b=11").unwrap(),
        CString::new("a.old=12").unwrap(),
    ];
    let backend = Backend::from_values(&[&values[0], &values[1]], None).unwrap();
    drop(values);
    config
        .as_mut()
        .add_backend(backend, ConfigLevel::GLOBAL, None, false)
        .unwrap();
    let retained = config.as_ref().entry(c"a.b").unwrap();
    assert_eq!(retained.as_ref().value(), Some(c"11"));
    for (level, force, expected) in [
        (ConfigLevel::GLOBAL, false, Some(ErrorCode::Exists)),
        (ConfigLevel::LOCAL, false, None),
        (ConfigLevel::GLOBAL, true, None),
    ] {
        let backend = Backend::from_values(&[c"a.b=20"], None).unwrap();
        let result = config
            .as_mut()
            .as_mut()
            .add_backend(backend, level, None, force);
        assert_eq!(result.err().and_then(|e| e.code()), expected);
        assert_eq!(retained.as_ref().value(), Some(c"11"));
    }
    assert_eq!(config.as_ref().get_i32(c"a.b").unwrap(), 20);
    let snapshot = config.as_ref().snapshot().unwrap();
    let entry = {
        let level = config.as_ref().open_level(ConfigLevel::GLOBAL).unwrap();
        level.as_ref().entry(c"a.b").unwrap()
    };
    drop(config);
    assert_eq!(snapshot.as_ref().get_i32(c"a.b").unwrap(), 20);
    assert_eq!(retained.as_ref().value(), Some(c"11"));
    assert_eq!(entry.as_ref().value(), Some(c"20"));
    drop(snapshot);
    assert_eq!(retained.as_ref().value(), Some(c"11"));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn backend_registration_failed_open_empty_and_priority_boundaries() {
    use libgit2::libgit2::config::config::{Config, ConfigLevel};
    use libgit2::libgit2::config::sys::config::Backend;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    let mut config = Config::new().unwrap();
    let backend = Backend::from_string(b"[missing\n", None).unwrap();
    let error = config
        .as_mut()
        .add_backend(backend, ConfigLevel::LOCAL, None, false)
        .unwrap_err();
    assert_eq!(error.class(), Some(ErrorClass::Config));
    assert_eq!(
        config.as_ref().get_i32(c"a.b").unwrap_err().code(),
        Some(ErrorCode::NotFound)
    );
    for raw in [i32::MIN, -1, i32::MAX] {
        let backend = Backend::from_values(&[c"a.b=1"], None).unwrap();
        let error = config
            .as_mut()
            .add_backend(backend, ConfigLevel::from_raw(raw), None, true)
            .unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Invalid));
    }
    for raw in [0, i32::MAX - 1] {
        let backend = Backend::from_values(&[], None).unwrap();
        config
            .as_mut()
            .add_backend(backend, ConfigLevel::from_raw(raw), None, false)
            .unwrap();
    }
    let view = config.as_ref().open_level(ConfigLevel::HIGHEST).unwrap();
    assert_eq!(
        view.as_ref().get_i32(c"a.b").unwrap_err().code(),
        Some(ErrorCode::NotFound)
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn backend_registration_optional_repository_keepalive() {
    use libgit2::libgit2::config::config::{Config, ConfigBox, ConfigLevel};
    use libgit2::libgit2::config::sys::config::Backend;
    use libgit2::libgit2::repository::repository::Repository;
    let dir = std::env::temp_dir().join(format!(
        "crustify-backend-register-repo-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let path = CString::new(dir.to_str().unwrap()).unwrap();
    let repository = Repository::init(&path, true).unwrap();
    let mut config: ConfigBox<'_> = Config::new().unwrap();
    config
        .as_mut()
        .add_backend(
            Backend::from_values(&[c"a.b=1"], None).unwrap(),
            ConfigLevel::LOCAL,
            Some(repository.as_ref()),
            false,
        )
        .unwrap();
    let snapshot = config.as_ref().snapshot().unwrap();
    drop(config);
    drop(repository);
    assert_eq!(snapshot.as_ref().get_i32(c"a.b").unwrap(), 1);
    drop(snapshot);
    std::fs::remove_dir_all(dir).unwrap();
}
