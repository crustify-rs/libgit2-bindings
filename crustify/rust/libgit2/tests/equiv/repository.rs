//! Equivalence of `git2/repository.h`'s wrappers with the C values and routines they
//! stand for, one section per batch.

/// [`InitFlag`], [`InitFlags`] and [`InitMode`] against the
/// `git_repository_init_flag_t` and `git_repository_init_mode_t` constants, and
/// against `git_repository_init_ext`, which reads them from its options.
///
/// [`InitFlag`] must name exactly the flag constants, each with its C value, and
/// [`InitFlags`] must test bits as C does; [`InitMode`] must give the named variant
/// for exactly the mode constants and keep every other value's bits.
///
/// Then each case initialises a repository twice, under two roots: once with options
/// built from the C constants, once with options built from the Rust values. Each side
/// owns its options and runs three steps on them: an init, a second init with the
/// options the first one left (libgit2 ORs its internal bits into them, so they carry
/// a reinitialisation), and an init that adds [`InitFlag::NoReinit`]. After every step
/// the return, the error, the options' flags and the tree each side created (kinds,
/// permission bits, contents, with the root's path masked) must agree.
///
/// The safe wrapper of `git_repository_init_ext` belongs to a later batch, so the
/// call itself stays raw on both sides.
mod init_flags_and_modes {
    use std::collections::BTreeMap;
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{InitFlag, InitFlags, InitMode};

    /// Every `git_repository_init_flag_t` constant and the variant carrying it.
    const FLAGS: [(ffi::git_repository_init_flag_t, InitFlag); 6] = [
        (ffi::GIT_REPOSITORY_INIT_BARE, InitFlag::Bare),
        (ffi::GIT_REPOSITORY_INIT_NO_REINIT, InitFlag::NoReinit),
        (ffi::GIT_REPOSITORY_INIT_MKDIR, InitFlag::Mkdir),
        (ffi::GIT_REPOSITORY_INIT_MKPATH, InitFlag::Mkpath),
        (
            ffi::GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE,
            InitFlag::ExternalTemplate,
        ),
        (
            ffi::GIT_REPOSITORY_INIT_RELATIVE_GITLINK,
            InitFlag::RelativeGitlink,
        ),
    ];

    /// Every `git_repository_init_mode_t` constant and the variant carrying it.
    const MODES: [(ffi::git_repository_init_mode_t, InitMode); 3] = [
        (ffi::GIT_REPOSITORY_INIT_SHARED_UMASK, InitMode::SharedUmask),
        (ffi::GIT_REPOSITORY_INIT_SHARED_GROUP, InitMode::SharedGroup),
        (ffi::GIT_REPOSITORY_INIT_SHARED_ALL, InitMode::SharedAll),
    ];

    /// Every value up to past the highest flag bit and around the shared modes, each
    /// single bit and its neighbours, and a coarse stride across the whole type.
    fn probes() -> impl Iterator<Item = u32> {
        let dense = if cfg!(any(bsan, miri)) {
            0o200
        } else {
            0o10000
        };
        (0..=dense)
            .chain((0..32).flat_map(|k| {
                let bit = 1u32 << k;
                [bit - 1, bit, bit + 1]
            }))
            .chain((0..=u32::MAX).step_by(1 << 26))
            .chain([u32::MAX - 1, u32::MAX])
    }

    #[test]
    fn flag_from_raw_names_exactly_the_c_constants() {
        for (c, flag) in FLAGS {
            assert_eq!(flag.raw(), c, "{flag:?}");
            assert_eq!(ffi::git_repository_init_flag_t::from(flag), c, "{flag:?}");
        }
        assert_eq!(InitFlag::ALL.map(InitFlag::raw), FLAGS.map(|(c, _)| c));
        for raw in probes() {
            let c = FLAGS.iter().find(|(c, _)| *c == raw).map(|&(_, f)| f);
            assert_eq!(InitFlag::from_raw(raw), c, "{raw:#x}");
            assert_eq!(
                InitFlag::try_from(raw).map_err(|e| e.raw()),
                c.ok_or(raw),
                "{raw:#x}"
            );
        }
    }

    #[test]
    fn flags_test_bits_as_c_does() {
        let named = FLAGS.iter().fold(0, |bits, (c, _)| bits | c);
        for bits in probes() {
            let set = InitFlags::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
            assert_eq!(set.unnamed_bits(), bits & !named, "{bits:#x}");
            assert_eq!(InitFlags::from_bits_truncate(bits).bits(), bits & named);
            let c: Vec<_> = FLAGS
                .iter()
                .filter(|(c, _)| bits & c != 0)
                .map(|&(_, f)| f)
                .collect();
            assert_eq!(set.iter().collect::<Vec<_>>(), c, "{bits:#x}");
            for (c, flag) in FLAGS {
                assert_eq!(set.contains(flag), bits & c != 0, "{bits:#x} {flag:?}");
            }
            assert_eq!(set.iter().collect::<InitFlags>().bits(), bits & named);
        }
    }

    #[test]
    fn mode_from_raw_names_exactly_the_c_constants() {
        for (c, mode) in MODES {
            assert_eq!(mode.raw(), c, "{mode:?}");
            assert_eq!(ffi::git_repository_init_mode_t::from(mode), c, "{mode:?}");
        }
        for raw in probes() {
            let mode = InitMode::from_raw(raw);
            assert_eq!(mode.raw(), raw, "{raw:#o}");
            assert_eq!(InitMode::from(raw), mode);
            match MODES.iter().find(|(c, _)| *c == raw) {
                Some(&(_, named)) => assert_eq!(mode, named, "{raw:#o}"),
                None => match mode {
                    InitMode::Custom(custom) => assert_eq!(custom.bits(), raw),
                    other => panic!("{raw:#o} read as {other:?}"),
                },
            }
        }
    }

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// A scratch directory, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn c_path(&self, rel: &str) -> CString {
            CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
        }

        /// A template directory, `tpl`: a hook, a nested directory's file and a
        /// description; libgit2 copies no empty directory.
        fn template(&self) -> CString {
            let tpl = self.0.join("tpl");
            fs::create_dir_all(tpl.join("hooks")).unwrap();
            fs::create_dir_all(tpl.join("info/nested")).unwrap();
            fs::write(tpl.join("hooks/post-commit"), "#!/bin/sh\n").unwrap();
            fs::write(tpl.join("info/nested/exclude"), "*.o\n").unwrap();
            fs::write(tpl.join("description"), "from the template\n").unwrap();
            self.c_path("tpl")
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// One entry of a created tree: a directory's permission bits, or a file's and
    /// its contents with the root's path masked.
    #[derive(Debug, PartialEq, Eq)]
    enum Entry {
        Dir(u32),
        File(u32, String),
    }

    /// Every entry under `root` but the template, by relative path.
    fn snapshot(root: &Path) -> BTreeMap<String, Entry> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        let masked = root.to_str().unwrap();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                if rel == "tpl" {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).unwrap();
                let bits = meta.permissions().mode() & 0o7777;
                if meta.is_dir() {
                    out.insert(rel, Entry::Dir(bits));
                    stack.push(path);
                } else {
                    let text = fs::read_to_string(&path).unwrap();
                    out.insert(rel, Entry::File(bits, text.replace(masked, "<root>")));
                }
            }
        }
        out
    }

    /// One step's record: the return, the class and message of a failure (with the
    /// root masked), and the options' flags afterwards.
    type Step = (c_int, Option<(c_int, String)>, u32);

    /// `git_repository_init_ext(root/path, opts)`, freeing the repository it opens.
    fn init_ext(root: &Path, path: &str, opts: &mut ffi::git_repository_init_options) -> Step {
        let c_path = CString::new(root.join(path).as_os_str().as_bytes()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` a live
        // options struct whose strings outlive the call; the result is NULL or a
        // fresh repository.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), opts) };
        // SAFETY: `repo` is NULL or the fresh repository, released once here.
        unsafe { ffi::git_repository_free(repo) };
        // A successful open leaves a stale error behind, so only a failure's counts.
        let error = (rc < 0).then(|| {
            // SAFETY: after a failure `git_error_last` is this thread's error, live
            // until the next libgit2 call on this thread; it is copied out first.
            let e = unsafe { &*ffi::git_error_last() };
            // SAFETY: a set error's message is a NUL-terminated string.
            let message = unsafe { CStr::from_ptr(e.message) }.to_string_lossy();
            (e.klass, message.replace(root.to_str().unwrap(), "<root>"))
        });
        (rc, error, opts.flags)
    }

    /// Default options, as `git_repository_init_options_init` fills them.
    fn options() -> ffi::git_repository_init_options {
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts
    }

    /// A case: its flags and mode spelled both ways, the path to initialise, and a
    /// working directory and external template if any.
    struct Case {
        tag: &'static str,
        c: (u32, u32),
        rust: (InitFlags, InitMode),
        path: &'static str,
        workdir: Option<&'static CStr>,
        /// The template directory's name under the root; only `tpl` exists.
        template: Option<&'static str>,
    }

    /// Runs `case` on both sides, step by step, comparing after each step.
    fn run(case: &Case) -> Vec<Step> {
        let roots = [
            Root::new(&format!("{}-c", case.tag)),
            Root::new(&format!("{}-rust", case.tag)),
        ];
        // A regular file where a case wants a directory, and a working directory that
        // already holds a `.git` directory, for the cases that fail on them.
        for root in &roots {
            fs::write(root.0.join("blocker"), "").unwrap();
            fs::create_dir_all(root.0.join("taken/.git")).unwrap();
        }
        let templates = roots.each_ref().map(|r| {
            case.template.map(|name| {
                r.template();
                r.c_path(name)
            })
        });
        let as_ptr = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
        let mut opts = [options(), options()];
        for (side, opts) in opts.iter_mut().enumerate() {
            (opts.flags, opts.mode) = if side == 0 {
                case.c
            } else {
                (case.rust.0.bits(), case.rust.1.raw())
            };
            opts.workdir_path = as_ptr(case.workdir);
            opts.template_path = as_ptr(templates[side].as_deref());
        }
        let mut steps = Vec::new();
        for step in 0..3 {
            if step == 2 {
                opts[0].flags |= ffi::GIT_REPOSITORY_INIT_NO_REINIT;
                let flags = InitFlags::from_bits_retain(opts[1].flags) | InitFlag::NoReinit;
                opts[1].flags = flags.bits();
            }
            let c = init_ext(&roots[0].0, case.path, &mut opts[0]);
            let rust = init_ext(&roots[1].0, case.path, &mut opts[1]);
            assert_eq!(rust, c, "{} step {step}", case.tag);
            assert_eq!(
                snapshot(&roots[1].0),
                snapshot(&roots[0].0),
                "{} step {step}",
                case.tag
            );
            steps.push(c);
        }
        steps
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_ext_reads_rust_values_as_c_constants() {
        use InitFlag::*;
        use ffi::{
            GIT_REPOSITORY_INIT_BARE as BARE, GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE as TEMPLATE,
            GIT_REPOSITORY_INIT_MKDIR as MKDIR, GIT_REPOSITORY_INIT_MKPATH as MKPATH,
            GIT_REPOSITORY_INIT_NO_REINIT as NO_REINIT,
            GIT_REPOSITORY_INIT_RELATIVE_GITLINK as RELATIVE,
            GIT_REPOSITORY_INIT_SHARED_ALL as ALL, GIT_REPOSITORY_INIT_SHARED_GROUP as GROUP,
            GIT_REPOSITORY_INIT_SHARED_UMASK as UMASK,
        };
        init();
        let custom = |bits| InitMode::from_raw(bits);
        let cases = [
            Case {
                tag: "mkpath",
                c: (MKPATH, UMASK),
                rust: (Mkpath.into(), InitMode::SharedUmask),
                path: "a/b/work",
                workdir: None,
                template: None,
            },
            Case {
                tag: "bare-group",
                c: (BARE | MKDIR, GROUP),
                rust: (Bare | Mkdir, InitMode::SharedGroup),
                path: "bare.git",
                workdir: None,
                template: None,
            },
            Case {
                tag: "missing-parent",
                c: (BARE | MKDIR, ALL),
                rust: (Bare | Mkdir, InitMode::SharedAll),
                path: "x/y/bare",
                workdir: None,
                template: None,
            },
            Case {
                tag: "dotgit",
                c: (0, UMASK),
                rust: (InitFlags::empty(), InitMode::default()),
                path: "plain/.git",
                workdir: None,
                template: None,
            },
            Case {
                tag: "not-dotgit",
                c: (MKDIR, UMASK),
                rust: (Mkdir.into(), InitMode::SharedUmask),
                path: "neither",
                workdir: None,
                template: None,
            },
            Case {
                tag: "relative-gitlink",
                c: (MKPATH | RELATIVE, 0o750),
                rust: (Mkpath | RelativeGitlink, custom(0o750)),
                path: "repos/r.git",
                workdir: Some(c"../../wd"),
                template: None,
            },
            Case {
                tag: "absolute-gitlink",
                c: (MKPATH, 0o2770),
                rust: (Mkpath.into(), custom(0o2770)),
                path: "repos/r.git",
                workdir: Some(c"../../wd"),
                template: None,
            },
            Case {
                tag: "template",
                c: (BARE | MKPATH | TEMPLATE, 0o750),
                rust: (Bare | Mkpath | ExternalTemplate, custom(0o750)),
                path: "t/bare",
                workdir: None,
                template: Some("tpl"),
            },
            Case {
                // No template path: `init.templatedir`, else the system's templates.
                tag: "default-template",
                c: (BARE | MKPATH | TEMPLATE, UMASK),
                rust: (Bare | Mkpath | ExternalTemplate, InitMode::SharedUmask),
                path: "dt",
                workdir: None,
                template: None,
            },
            Case {
                tag: "mkpath-workdir-blocked",
                c: (MKPATH, UMASK),
                rust: (Mkpath.into(), InitMode::SharedUmask),
                path: "blocker/sub/work",
                workdir: None,
                template: None,
            },
            Case {
                tag: "mkpath-repo-blocked",
                c: (BARE | MKPATH, GROUP),
                rust: (Bare | Mkpath, InitMode::SharedGroup),
                path: "blocker/sub/bare",
                workdir: None,
                template: None,
            },
            Case {
                tag: "mkdir-workdir-blocked",
                c: (MKDIR, UMASK),
                rust: (Mkdir.into(), InitMode::SharedUmask),
                path: "r.git",
                workdir: Some(c"../blocker/wd"),
                template: None,
            },
            Case {
                // A template path that does not exist: the built-in template instead.
                tag: "missing-template",
                c: (BARE | MKPATH | TEMPLATE, 0o750),
                rust: (Bare | Mkpath | ExternalTemplate, custom(0o750)),
                path: "mt",
                workdir: None,
                template: Some("missing"),
            },
            Case {
                // The working directory's `.git` is a directory, not a gitlink.
                tag: "gitlink-blocked",
                c: (MKPATH | RELATIVE, UMASK),
                rust: (Mkpath | RelativeGitlink, InitMode::SharedUmask),
                path: "repos/g.git",
                workdir: Some(c"../../taken"),
                template: None,
            },
            Case {
                tag: "no-reinit",
                c: (BARE | MKPATH | NO_REINIT, ALL),
                rust: (Bare | Mkpath | NoReinit, InitMode::SharedAll),
                path: "nr",
                workdir: None,
                template: None,
            },
            Case {
                // A caller's unnamed bit reaches C as is: init_ext's own "natural
                // working directory" bit, which skips the gitlink.
                tag: "unnamed-bit",
                c: (MKPATH | (1 << 17), UMASK),
                rust: (
                    InitFlags::from_bits_retain(1 << 17) | Mkpath,
                    InitMode::SharedUmask,
                ),
                path: "u/r.git",
                workdir: Some(c"../wd"),
                template: None,
            },
        ];
        let mut outcomes = BTreeMap::new();
        for case in &cases {
            outcomes.insert(case.tag, run(case));
        }

        // What the steps show about C, beyond agreement: init_ext ORs its internal
        // bits into the options it was given, and the second init reinitialises.
        let flags = |tag: &str| outcomes[tag].iter().map(|s| s.2).collect::<Vec<_>>();
        let rcs = |tag: &str| outcomes[tag].iter().map(|s| s.0).collect::<Vec<_>>();
        assert_eq!(
            flags("mkpath"),
            [0x3_0010, 0x7_0010, 0x7_0012],
            "has .git, natural workdir, then reinit"
        );
        assert_eq!(rcs("mkpath"), [0, 0, ffi::GIT_EEXISTS]);
        assert_eq!(flags("bare-group"), [0x9, 0x4_0009, 0x4_000b]);
        assert_eq!(rcs("missing-parent"), [-1, -1, -1]);
        for tag in [
            "mkpath-workdir-blocked",
            "mkpath-repo-blocked",
            "mkdir-workdir-blocked",
            "gitlink-blocked",
        ] {
            assert!(rcs(tag).iter().all(|&rc| rc < 0), "{tag}");
        }
        assert_eq!(rcs("missing-template"), [0, 0, ffi::GIT_EEXISTS]);
        assert_eq!(rcs("no-reinit"), [0, ffi::GIT_EEXISTS, ffi::GIT_EEXISTS]);
        assert_eq!(
            outcomes["no-reinit"][1].1,
            Some((
                ffi::GIT_ERROR_REPOSITORY as c_int,
                "attempt to reinitialize '<root>/nr'".to_owned()
            ))
        );
        let read = InitFlags::from_bits_retain(flags("mkpath")[2]);
        assert_eq!(read.iter().collect::<Vec<_>>(), [NoReinit, Mkpath]);
        assert_eq!(read.unnamed_bits(), 0x7_0000);
    }

    /// The trees the modes produce, on the Rust side only (`run` compared them with
    /// C's): the shared modes' setgid repository directory and recorded setting, and
    /// a custom mode on the built-in and an external template's directories.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn modes_shape_the_created_tree() {
        init();
        let root = Root::new("modes");
        for (path, mode) in [
            ("group", InitMode::SharedGroup),
            ("all", InitMode::SharedAll),
            ("custom", InitMode::from_raw(0o751)),
        ] {
            let mut opts = options();
            opts.flags = (InitFlag::Bare | InitFlag::Mkdir).bits();
            opts.mode = mode.raw();
            assert_eq!(init_ext(&root.0, path, &mut opts).0, 0, "{mode:?}");
        }
        let template = root.template();
        let mut opts = options();
        opts.flags = (InitFlag::Bare | InitFlag::Mkdir | InitFlag::ExternalTemplate).bits();
        opts.mode = InitMode::from_raw(0o751).raw();
        opts.template_path = template.as_ptr();
        assert_eq!(init_ext(&root.0, "external", &mut opts).0, 0);
        let tree = snapshot(&root.0);
        assert_eq!(tree["group"], Entry::Dir(0o2775));
        assert_eq!(tree["all"], Entry::Dir(0o2777));
        assert_eq!(tree["custom/objects/info"], Entry::Dir(0o751));
        assert_eq!(tree["custom/hooks"], Entry::Dir(0o751));
        assert_eq!(tree["external/info/nested"], Entry::Dir(0o751));
        assert!(matches!(
            &tree["external/description"],
            Entry::File(_, text) if text == "from the template\n"
        ));
        let config = |path: &str| match &tree[&format!("{path}/config")] {
            Entry::File(_, text) => text.clone(),
            other => panic!("{other:?}"),
        };
        assert!(config("group").contains("sharedrepository = 1"));
        assert!(config("all").contains("sharedrepository = 2"));
        assert!(!config("custom").contains("sharedrepository"));
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";
    const ROOT: &str = "CRUSTIFY_EQUIV_ROOT";

    /// [`InitFlag::Mkpath`]'s bit is `GIT_REPOSITORY_OPEN_FROM_ENV`'s, which init_ext
    /// also reads: with `$GIT_COMMON_DIR` naming a repository, a directory holding only
    /// a `HEAD` reads as a repository under `Mkpath` and is refused by `NoReinit`,
    /// while `Mkdir` initialises it. Both spellings of the flags agree. The variable is
    /// only set on a child process, which no other test shares.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mkpath_reads_the_environment_as_open_from_env() {
        assert_eq!(InitFlag::Mkpath.raw(), ffi::GIT_REPOSITORY_OPEN_FROM_ENV);
        let module = module_path!().split_once("::").unwrap().1;
        let name = format!("{module}::mkpath_reads_the_environment_as_open_from_env");
        if std::env::var(CHILD).as_deref() == Ok(name.as_str()) {
            init();
            // The parent owns the directory and removes it.
            let root = PathBuf::from(std::env::var(ROOT).unwrap());
            let mut outcomes = Vec::new();
            for (tag, c, rust) in [
                (
                    "mkpath",
                    ffi::GIT_REPOSITORY_INIT_MKPATH,
                    InitFlags::from(InitFlag::Mkpath),
                ),
                (
                    "mkdir",
                    ffi::GIT_REPOSITORY_INIT_MKDIR,
                    InitFlags::from(InitFlag::Mkdir),
                ),
            ] {
                let base = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_NO_REINIT;
                let mut c_opts = options();
                c_opts.flags = base | c;
                let c = init_ext(&root, &format!("{tag}-c"), &mut c_opts);
                let mut rust_opts = options();
                rust_opts.flags = (rust | InitFlag::Bare | InitFlag::NoReinit).bits();
                let rust = init_ext(&root, &format!("{tag}-rust"), &mut rust_opts);
                assert_eq!(rust.0, c.0, "{tag}");
                assert_eq!(
                    rust.1.map(|(k, m)| (k, m.replace("-rust", ""))),
                    c.1.map(|(k, m)| (k, m.replace("-c", ""))),
                    "{tag}"
                );
                outcomes.push(c.0);
            }
            assert_eq!(outcomes, [ffi::GIT_EEXISTS, 0]);
            return;
        }

        init();
        let root = Root::new("from-env");
        let mut opts = options();
        opts.flags = (InitFlag::Bare | InitFlag::Mkdir).bits();
        assert_eq!(init_ext(&root.0, "common", &mut opts).0, 0);
        for dir in ["mkpath-c", "mkpath-rust", "mkdir-c", "mkdir-rust"] {
            fs::create_dir(root.0.join(dir)).unwrap();
            fs::write(root.0.join(dir).join("HEAD"), "ref: refs/heads/master\n").unwrap();
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([&name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, &name)
            .env(ROOT, &root.0)
            .env("GIT_COMMON_DIR", root.0.join("common"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
