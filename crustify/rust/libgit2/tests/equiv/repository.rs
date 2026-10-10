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

/// [`Item`], [`OpenFlag`] and [`OpenFlags`] against the `git_repository_item_t` and
/// `git_repository_open_flag_t` constants, and against `git_repository_item_path`
/// and `git_repository_open_ext`, which take them.
///
/// [`Item`] and [`OpenFlag`] must name exactly the values of their C enums, each with
/// its C value, and an [`OpenFlags`] set must carry the bits of the C flags it is built
/// from. libgit2 observes them through two routines, called here on repositories
/// built from scratch (a working directory, a bare repository, and a worktree linked
/// by hand): `git_repository_item_path` must resolve every variant's value as it does
/// the C constant, to the path the variant documents; and `git_repository_open_ext`
/// must open (or refuse) every start path under every set's bits as under the OR of
/// the C constants, with the bits no flag names changing nothing.
///
/// The safe wrappers of the routines themselves belong to the `repository` symbol
/// batches, so both are called raw on either side.
mod items_and_open_flags {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{
        Item, OpenFlag, OpenFlags, UnnamedItem, UnnamedOpenFlag,
    };

    /// Every `git_repository_item_t` constant but the sentinel, and its variant.
    const ITEMS: [(ffi::git_repository_item_t, Item); 15] = [
        (ffi::GIT_REPOSITORY_ITEM_GITDIR, Item::GitDir),
        (ffi::GIT_REPOSITORY_ITEM_WORKDIR, Item::WorkDir),
        (ffi::GIT_REPOSITORY_ITEM_COMMONDIR, Item::CommonDir),
        (ffi::GIT_REPOSITORY_ITEM_INDEX, Item::Index),
        (ffi::GIT_REPOSITORY_ITEM_OBJECTS, Item::Objects),
        (ffi::GIT_REPOSITORY_ITEM_REFS, Item::Refs),
        (ffi::GIT_REPOSITORY_ITEM_PACKED_REFS, Item::PackedRefs),
        (ffi::GIT_REPOSITORY_ITEM_REMOTES, Item::Remotes),
        (ffi::GIT_REPOSITORY_ITEM_CONFIG, Item::Config),
        (ffi::GIT_REPOSITORY_ITEM_INFO, Item::Info),
        (ffi::GIT_REPOSITORY_ITEM_HOOKS, Item::Hooks),
        (ffi::GIT_REPOSITORY_ITEM_LOGS, Item::Logs),
        (ffi::GIT_REPOSITORY_ITEM_MODULES, Item::Modules),
        (ffi::GIT_REPOSITORY_ITEM_WORKTREES, Item::Worktrees),
        (
            ffi::GIT_REPOSITORY_ITEM_WORKTREE_CONFIG,
            Item::WorktreeConfig,
        ),
    ];

    /// Every `git_repository_open_flag_t` constant and its variant.
    const FLAGS: [(ffi::git_repository_open_flag_t, OpenFlag); 5] = [
        (ffi::GIT_REPOSITORY_OPEN_NO_SEARCH, OpenFlag::NoSearch),
        (ffi::GIT_REPOSITORY_OPEN_CROSS_FS, OpenFlag::CrossFs),
        (ffi::GIT_REPOSITORY_OPEN_BARE, OpenFlag::Bare),
        (ffi::GIT_REPOSITORY_OPEN_NO_DOTGIT, OpenFlag::NoDotGit),
        (ffi::GIT_REPOSITORY_OPEN_FROM_ENV, OpenFlag::FromEnv),
    ];

    /// The directory an item resolves against, per the variant's documentation.
    #[derive(Clone, Copy, Debug)]
    enum Parent {
        Git,
        Work,
        Common,
    }

    /// What each variant documents: its parent, its name under it, and whether it is
    /// a directory (resolved with a trailing `/`).
    fn documented(item: Item) -> (Parent, &'static str, bool) {
        use Parent::*;
        match item {
            Item::GitDir => (Git, "", true),
            Item::WorkDir => (Work, "", true),
            Item::CommonDir => (Common, "", true),
            Item::Index => (Git, "index", false),
            Item::Objects => (Common, "objects", true),
            Item::Refs => (Common, "refs", true),
            Item::PackedRefs => (Common, "packed-refs", false),
            Item::Remotes => (Common, "remotes", true),
            Item::Config => (Common, "config", false),
            Item::Info => (Common, "info", true),
            Item::Hooks => (Common, "hooks", true),
            Item::Logs => (Common, "logs", true),
            Item::Modules => (Git, "modules", true),
            Item::Worktrees => (Common, "worktrees", true),
            Item::WorktreeConfig => (Git, "config.worktree", false),
            other => unreachable!("{other:?} has no documented path"),
        }
    }

    /// Every value around the declared ones, each single bit, and the extremes.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=300)
            .chain((0..32).map(|k| 1u32 << k))
            .chain([u32::MAX - 1, u32::MAX])
    }

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The calling thread's last error, as C reads it.
    fn c_last() -> (c_int, CString) {
        // SAFETY: the runtime is initialised; the result is a static error or this
        // thread's, live until the next libgit2 call on this thread.
        let e = unsafe { ffi::git_error_last() };
        assert!(!e.is_null());
        // SAFETY: `e` is a live `git_error` whose message is NULL or a C string.
        unsafe {
            let message = (*e).message;
            let message = if message.is_null() {
                c""
            } else {
                CStr::from_ptr(message)
            };
            ((*e).klass, message.to_owned())
        }
    }

    /// A copy of a C string, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// `p` is NULL or a NUL-terminated string live for the copy.
    unsafe fn c_string(p: *const c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` is a NUL-terminated string live for the copy, per the contract.
        let s = unsafe { CStr::from_ptr(p) };
        Some(s.to_string_lossy().into_owned())
    }

    fn c_path(path: &Path) -> CString {
        CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    /// A scratch directory holding the test's repositories, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            // The canonical form, as libgit2 reports paths.
            Self(path.canonicalize().unwrap())
        }

        fn join(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// An open repository, freed on drop.
    struct Repo(*mut ffi::git_repository);

    impl Drop for Repo {
        fn drop(&mut self) {
            // SAFETY: the repository this test opened, released exactly once.
            unsafe { ffi::git_repository_free(self.0) }
        }
    }

    /// `git_repository_init_ext(path)`, bare or with a working directory.
    fn init_repo(path: &Path, bare: bool) -> Repo {
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_MKDIR;
        if bare {
            opts.flags |= ffi::GIT_REPOSITORY_INIT_BARE;
        }
        let path = c_path(path);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a live slot, `path` NUL-terminated and `opts` a live options
        // struct, all read for the call only; the result is NULL or a fresh repository.
        let rc = unsafe { ffi::git_repository_init_ext(&mut out, path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0, "{:?}", c_last());
        Repo(out)
    }

    /// Links a worktree of the non-bare repository `main` at `wt` by hand, as
    /// `git worktree add` lays it out: an administrative directory under
    /// `main/.git/worktrees/` naming the common directory, and a gitlink at `wt/.git`.
    fn link_worktree(main: &Path, wt: &Path) {
        let admin = main.join(".git/worktrees/wt");
        fs::create_dir_all(&admin).unwrap();
        fs::create_dir_all(wt).unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    }

    /// What `git_repository_open_ext` produced: its return, and the opened
    /// repository's git directory, working directory and bareness, or the error.
    type Opened = (
        c_int,
        Result<(String, Option<String>, bool), (c_int, CString)>,
    );

    /// `git_repository_open_ext(start, flags, ceiling)`.
    fn open(start: &CStr, flags: u32, ceiling: &CStr) -> Opened {
        open_nullable(Some(start), flags, Some(ceiling))
    }

    /// `git_repository_open_ext(start, flags, ceiling)`, either string NULL for `None`;
    /// a NULL `start` only under `GIT_REPOSITORY_OPEN_FROM_ENV`, which reads it from
    /// the environment.
    fn open_nullable(start: Option<&CStr>, flags: u32, ceiling: Option<&CStr>) -> Opened {
        assert!(start.is_some() || flags & ffi::GIT_REPOSITORY_OPEN_FROM_ENV != 0);
        let c_str = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
        let (start, ceiling) = (c_str(start), c_str(ceiling));
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a live slot, `start` and `ceiling` NUL-terminated strings
        // read for the call only, or NULL, which libgit2 accepts for `ceiling` and, as
        // asserted, for `start` under `FROM_ENV`. The result is NULL or a fresh
        // repository.
        let rc = unsafe { ffi::git_repository_open_ext(&mut out, start, flags, ceiling) };
        if rc < 0 {
            assert!(out.is_null());
            return (rc, Err(c_last()));
        }
        // A successful open leaves whatever error it recovered from on the thread, so
        // only a failure's error is compared.
        let repo = Repo(out);
        // SAFETY: `repo` is live; the strings it returns live as long as it, past the
        // copies.
        let opened = unsafe {
            (
                c_string(ffi::git_repository_path(repo.0)).unwrap(),
                c_string(ffi::git_repository_workdir(repo.0)),
                ffi::git_repository_is_bare(repo.0) == 1,
            )
        };
        (rc, Ok(opened))
    }

    /// `git_repository_item_path(repo, raw)`: the return, and the path or the error.
    fn item_path(
        repo: &Repo,
        raw: ffi::git_repository_item_t,
    ) -> (c_int, Result<String, (c_int, CString)>) {
        let mut buf = ffi::git_buf::default();
        // SAFETY: `buf` is an empty `git_buf`, `repo` a live repository and `raw` a
        // value below `GIT_REPOSITORY_ITEM__LAST` (the callers pass only the constants
        // and the variants), so libgit2 indexes its item table in bounds.
        let rc = unsafe { ffi::git_repository_item_path(&mut buf, repo.0, raw) };
        let result = if rc < 0 {
            Err(c_last())
        } else {
            // SAFETY: on success `buf.ptr` is a NUL-terminated string the buffer owns,
            // live until it is disposed of below.
            Ok(unsafe { c_string(buf.ptr) }.unwrap())
        };
        // SAFETY: `buf` is the call's buffer, disposed of exactly once.
        unsafe { ffi::git_buf_dispose(&mut buf) };
        (rc, result)
    }

    #[test]
    fn item_from_raw_names_exactly_the_c_constants() {
        for (raw, item) in ITEMS {
            assert_eq!(item.raw(), raw, "{item:?}");
            assert_eq!(Item::from_raw(raw), Some(item));
        }
        assert_eq!(Item::ALL.map(Item::raw), ITEMS.map(|(raw, _)| raw));
        for raw in probes() {
            let named = ITEMS.iter().find(|(c, _)| *c == raw).map(|&(_, item)| item);
            assert_eq!(Item::from_raw(raw), named, "{raw}");
            if named.is_none() {
                assert!(raw >= ffi::GIT_REPOSITORY_ITEM__LAST);
                assert_eq!(Item::try_from(raw).map_err(UnnamedItem::raw), Err(raw));
            }
        }
    }

    #[test]
    fn open_flag_from_raw_names_exactly_the_c_constants() {
        for (raw, flag) in FLAGS {
            assert_eq!(flag.raw(), raw, "{flag:?}");
            assert_eq!(OpenFlag::from_raw(raw), Some(flag));
            assert_eq!(OpenFlags::from(flag).bits(), raw);
        }
        assert_eq!(OpenFlag::ALL.map(OpenFlag::raw), FLAGS.map(|(raw, _)| raw));
        for raw in probes() {
            let named = FLAGS.iter().find(|(c, _)| *c == raw).map(|&(_, flag)| flag);
            assert_eq!(OpenFlag::from_raw(raw), named, "{raw}");
            if named.is_none() {
                assert_eq!(
                    OpenFlag::try_from(raw).map_err(UnnamedOpenFlag::raw),
                    Err(raw)
                );
            }
        }
    }

    /// Every item of a working-directory repository, a bare one and a linked
    /// worktree, through its C constant and through its variant, step after step:
    /// both must resolve alike, to the path the variant documents.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn item_path_resolves_every_variant_as_its_c_constant() {
        init();
        let scratch = Scratch::new("items");
        let main = scratch.join("main");
        let main_repo = init_repo(&main, false);
        let bare_repo = init_repo(&scratch.join("bare.git"), true);
        let wt = scratch.join("wt");
        link_worktree(&main, &wt);
        let (wt_path, ceiling) = (c_path(&wt), c_path(&scratch.0));
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a live slot, the paths NUL-terminated strings read for the
        // call only; the result is kept and freed by `Repo`.
        let rc = unsafe {
            ffi::git_repository_open_ext(&mut out, wt_path.as_ptr(), 0, ceiling.as_ptr())
        };
        assert_eq!(rc, 0, "{:?}", c_last());
        let wt_repo = Repo(out);

        let dir = |p: &Path| format!("{}/", p.display());
        let repos = [
            // (repo, git dir, working dir, common dir)
            (
                &main_repo,
                dir(&main.join(".git")),
                Some(dir(&main)),
                dir(&main.join(".git")),
            ),
            (
                &bare_repo,
                dir(&scratch.join("bare.git")),
                None,
                dir(&scratch.join("bare.git")),
            ),
            (
                &wt_repo,
                dir(&main.join(".git/worktrees/wt")),
                Some(dir(&wt)),
                // libgit2 resolves the `commondir` link.
                dir(&main.join(".git")),
            ),
        ];
        for (repo, git, work, common) in &repos {
            for (raw, item) in ITEMS {
                let c = item_path(repo, raw);
                let rust = item_path(repo, item.raw());
                assert_eq!(rust, c, "{item:?} in {git}");

                let (parent, name, is_dir) = documented(item);
                let base = match parent {
                    Parent::Git => Some(git),
                    Parent::Work => work.as_ref(),
                    Parent::Common => Some(common),
                };
                match base {
                    Some(base) => {
                        let mut path = format!("{base}{name}");
                        if is_dir && !path.ends_with('/') {
                            path.push('/');
                        }
                        assert_eq!(c, (0, Ok(path)), "{item:?} in {git}");
                    }
                    None => assert_eq!(
                        c,
                        (
                            ffi::GIT_ENOTFOUND,
                            Err((
                                ffi::GIT_ERROR_INVALID as c_int,
                                c"path cannot exist in repository".to_owned()
                            ))
                        ),
                        "{item:?} in {git}"
                    ),
                }
            }
        }
    }

    /// A repository with no location (`git_repository_new`) resolves no item: each
    /// variant, as its C constant, falls back to a git directory it does not have.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn item_path_finds_nothing_in_a_repository_without_a_location() {
        init();
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a live slot; the result is a fresh repository, freed by
        // `Repo`.
        assert_eq!(unsafe { ffi::git_repository_new(&mut out) }, 0);
        let repo = Repo(out);
        for (raw, item) in ITEMS {
            let c = item_path(&repo, raw);
            assert_eq!(item_path(&repo, item.raw()), c, "{item:?}");
            assert_eq!(
                c,
                (
                    ffi::GIT_ENOTFOUND,
                    Err((
                        ffi::GIT_ERROR_INVALID as c_int,
                        c"path cannot exist in repository".to_owned()
                    ))
                ),
                "{item:?}"
            );
        }
    }

    /// The start paths and ceiling of the open scenarios: a working directory, a
    /// subdirectory of it, its `.git`, a bare repository, a directory inside that, and
    /// a gitlink worktree.
    fn open_starts(scratch: &Scratch) -> Vec<CString> {
        let main = scratch.join("main");
        drop(init_repo(&main, false));
        fs::create_dir_all(main.join("sub/deeper")).unwrap();
        drop(init_repo(&scratch.join("bare.git"), true));
        link_worktree(&main, &scratch.join("wt"));
        [
            "main",
            "main/sub/deeper",
            "main/.git",
            "bare.git",
            "bare.git/refs",
            "wt",
        ]
        .map(|rel| c_path(&scratch.join(rel)))
        .into()
    }

    /// Every start path under every combination of flags, once with the OR of the C
    /// constants and once with the bits of the `OpenFlags` built from the variants,
    /// then with bits no flag names added: all three opens must agree.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_reads_open_flags_as_their_c_bits() {
        init();
        let scratch = Scratch::new("open");
        let starts = open_starts(&scratch);
        let ceiling = c_path(&scratch.0);
        let stride = if cfg!(bsan) { 7 } else { 1 };
        let mut outcomes = std::collections::BTreeSet::new();
        for start in &starts {
            for subset in (0u32..1 << FLAGS.len()).step_by(stride) {
                let chosen = || {
                    FLAGS
                        .iter()
                        .enumerate()
                        .filter(move |(i, _)| subset & 1 << i != 0)
                        .map(|(_, f)| *f)
                };
                let c_bits = chosen().fold(0, |bits, (raw, _)| bits | raw);
                let set: OpenFlags = chosen().map(|(_, flag)| flag).collect();
                assert_eq!(set.bits(), c_bits);
                assert_eq!(set.unnamed_bits(), 0);

                let c = open(start, c_bits, &ceiling);
                assert_eq!(open(start, set.bits(), &ceiling), c, "{start:?} {set:?}");
                let unnamed = set | OpenFlags::from_bits_retain(1 << 5 | 1 << 31);
                assert_eq!(unnamed.iter().collect::<OpenFlags>(), set);
                assert_eq!(
                    open(start, unnamed.bits(), &ceiling),
                    c,
                    "{start:?} {unnamed:?}"
                );
                outcomes.insert(c.0);
            }
        }
        // The scenarios reach both an open and a refusal.
        assert!(
            outcomes.contains(&0) && outcomes.contains(&ffi::GIT_ENOTFOUND),
            "{outcomes:?}"
        );
    }

    /// What the variants document, step after step on one set: a search walks up
    /// unless `NoSearch`, `.git` is appended unless `NoDotGit`, `Bare` drops the
    /// working directory, and `FromEnv` leaves the other flags in force.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_follows_each_flag_as_documented() {
        init();
        let scratch = Scratch::new("documented");
        let starts = open_starts(&scratch);
        let ceiling = c_path(&scratch.0);
        let [main, deeper, dot_git, bare, bare_refs, wt] = &starts[..] else {
            unreachable!()
        };
        let dir = |rel: &str| format!("{}/", scratch.join(rel).display());
        let found =
            |git: &str, work: Option<&str>, is_bare| (0, Ok((dir(git), work.map(dir), is_bare)));
        let refused = |start: &CString| {
            (
                ffi::GIT_ENOTFOUND,
                Err((
                    ffi::GIT_ERROR_REPOSITORY as c_int,
                    CString::new(format!(
                        "could not find repository at '{}'",
                        start.to_str().unwrap()
                    ))
                    .unwrap(),
                )),
            )
        };

        let mut set = OpenFlags::empty();
        let step = |start: &CString, set: OpenFlags| {
            let c = open(start, set.bits(), &ceiling);
            let bits = set.iter().fold(0, |bits, flag| bits | flag.raw());
            assert_eq!(open(start, bits, &ceiling), c, "{start:?} {set:?}");
            c
        };
        assert_eq!(step(deeper, set), found("main/.git", Some("main"), false));
        assert_eq!(step(dot_git, set), found("main/.git", Some("main"), false));
        assert_eq!(step(bare_refs, set), found("bare.git", None, true));
        assert_eq!(
            step(wt, set).1.map(|(_, work, _)| work),
            Ok(Some(dir("wt")))
        );

        set.insert(OpenFlag::NoSearch);
        assert_eq!(step(deeper, set), refused(deeper));
        assert_eq!(step(bare_refs, set), refused(bare_refs));
        assert_eq!(step(main, set), found("main/.git", Some("main"), false));

        set |= OpenFlag::NoDotGit;
        assert_eq!(step(main, set), refused(main));
        assert_eq!(step(dot_git, set), found("main/.git", Some("main"), false));
        assert_eq!(step(bare, set), found("bare.git", None, true));

        // `Bare` tries the start path itself, not its `.git`; only the parents a
        // search walks up to get one.
        set.remove(OpenFlag::NoDotGit);
        set |= OpenFlag::Bare;
        assert_eq!(step(main, set), refused(main));
        assert_eq!(step(dot_git, set), found("main/.git", None, true));
        set.remove(OpenFlag::NoSearch);
        assert_eq!(step(deeper, set), found("main/.git", None, true));

        set = OpenFlag::FromEnv | OpenFlag::NoSearch;
        assert_eq!(step(deeper, set), refused(deeper));
        set.remove(OpenFlag::NoSearch);
        assert_eq!(step(deeper, set), found("main/.git", Some("main"), false));
        set |= OpenFlag::Bare;
        assert_eq!(step(deeper, set), found("main/.git", None, true));

        // `CrossFs` matters only across a file system boundary, which the scratch
        // directory does not span.
        set = OpenFlag::CrossFs.into();
        assert_eq!(step(deeper, set), found("main/.git", Some("main"), false));
    }

    /// `FromEnv` with no ceiling directories or no start path, which libgit2 then reads
    /// from `GIT_CEILING_DIRECTORIES` and `GIT_DIR` (or the current directory): every
    /// set opens as the OR of its C constants.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_reads_the_environment_as_the_c_flags() {
        init();
        let scratch = Scratch::new("env");
        let starts = open_starts(&scratch);
        let deeper = &starts[1];
        for set in [
            OpenFlags::empty(),
            OpenFlag::FromEnv.into(),
            OpenFlag::FromEnv | OpenFlag::Bare,
            OpenFlag::FromEnv | OpenFlag::NoSearch,
            OpenFlag::FromEnv | OpenFlag::CrossFs,
        ] {
            let bits = set.iter().fold(0, |bits, flag| bits | flag.raw());
            let c = open_nullable(Some(deeper), bits, None);
            assert_eq!(open_nullable(Some(deeper), set.bits(), None), c, "{set:?}");
            let expected = if set.contains(OpenFlag::NoSearch) {
                ffi::GIT_ENOTFOUND
            } else {
                0
            };
            assert_eq!(c.0, expected, "{set:?} {c:?}");
            if set.contains(OpenFlag::FromEnv) {
                // The test's current directory, unless `GIT_DIR` names another.
                let c = open_nullable(None, bits, None);
                assert_eq!(open_nullable(None, set.bits(), None), c, "{set:?}");
            }
        }
    }

    /// A search walking up from a file system mounted below another stops at the
    /// boundary unless `CrossFs`. `/dev/shm` is such a mount where it is a separate
    /// file system from `/dev`; elsewhere there is no boundary to try.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_crosses_file_systems_as_the_c_flags() {
        use std::os::unix::fs::MetadataExt;
        let dev = |p: &str| fs::metadata(p).map(|m| m.dev());
        if !matches!((dev("/dev/shm"), dev("/dev")), (Ok(a), Ok(b)) if a != b) {
            eprintln!("no file system boundary at /dev/shm; skipped");
            return;
        }
        init();
        let start = PathBuf::from(format!(
            "/dev/shm/crustify-repository-equiv-{}-xfs/sub",
            std::process::id()
        ));
        fs::create_dir_all(&start).unwrap();
        let c_start = c_path(&start);
        for set in [OpenFlags::empty(), OpenFlag::CrossFs.into()] {
            let bits = set.iter().fold(0, |bits, flag| bits | flag.raw());
            let c = open_nullable(Some(&c_start), bits, None);
            assert_eq!(
                open_nullable(Some(&c_start), set.bits(), None),
                c,
                "{set:?}"
            );
            // Nothing above is a repository, so both searches end empty-handed.
            assert_eq!(c.0, ffi::GIT_ENOTFOUND, "{set:?} {c:?}");
        }
        let _ = fs::remove_dir_all(start.parent().unwrap());
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";
    const SCRATCH: &str = "CRUSTIFY_EQUIV_REPOSITORY_SCRATCH";

    /// Runs `body` in a child process whose environment adds `vars`, so the variables
    /// `FromEnv` reads are set without mutating this process's environment.
    fn with_env(test: &str, vars: &[(&str, &str)], body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .envs(vars.iter().copied())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The scratch directory a [`with_env`] parent laid out, as the child sees it.
    fn child_scratch() -> PathBuf {
        PathBuf::from(std::env::var(SCRATCH).unwrap())
    }

    /// `FromEnv` reads `GIT_DIR` for a NULL start path (and then neither searches nor
    /// appends `.git`), `GIT_CEILING_DIRECTORIES` for NULL ceiling directories, and
    /// `GIT_DISCOVERY_ACROSS_FILESYSTEM` as a boolean: under each, every set opens as
    /// the OR of its C constants, and as the variables document.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_follows_the_environment_from_env_reads() {
        const TEST: &str = "repository::items_and_open_flags::\
                            open_ext_follows_the_environment_from_env_reads";
        if std::env::var(CHILD).is_err() {
            init();
            let scratch = Scratch::new("env-child");
            open_starts(&scratch);
            let path = |rel: &str| scratch.join(rel).to_str().unwrap().to_owned();
            let (git_dir, ceiling) = (path("main/.git"), path("main/sub"));
            for across in ["true", "maybe"] {
                with_env(
                    TEST,
                    &[
                        (SCRATCH, scratch.0.to_str().unwrap()),
                        ("GIT_DIR", &git_dir),
                        ("GIT_CEILING_DIRECTORIES", &ceiling),
                        ("GIT_DISCOVERY_ACROSS_FILESYSTEM", across),
                    ],
                    || {},
                );
            }
            return;
        }
        with_env(TEST, &[], || {
            init();
            let scratch = child_scratch();
            let main_git = format!("{}/", scratch.join("main/.git").display());
            let deeper = c_path(&scratch.join("main/sub/deeper"));
            let valid = std::env::var("GIT_DISCOVERY_ACROSS_FILESYSTEM").unwrap() == "true";
            let step = |start: Option<&CStr>, set: OpenFlags| {
                let bits = set.iter().fold(0, |bits, flag| bits | flag.raw());
                let c = open_nullable(start, bits, None);
                assert_eq!(
                    open_nullable(start, set.bits(), None),
                    c,
                    "{start:?} {set:?}"
                );
                c
            };
            for set in [
                OpenFlag::FromEnv.into(),
                OpenFlag::FromEnv | OpenFlag::Bare,
                OpenFlag::FromEnv | OpenFlag::CrossFs,
            ] {
                let from_git_dir = step(None, set);
                let from_deeper = step(Some(&deeper), set);
                if !valid {
                    // An unparsable boolean fails every `FromEnv` open.
                    for c in [&from_git_dir, &from_deeper] {
                        assert_eq!(c.0, -1, "{set:?} {c:?}");
                    }
                    continue;
                }
                // `GIT_DIR` is the git directory itself.
                let work = (!set.contains(OpenFlag::Bare))
                    .then(|| format!("{}/", scratch.join("main").display()));
                assert_eq!(
                    from_git_dir,
                    (
                        0,
                        Ok((main_git.clone(), work, set.contains(OpenFlag::Bare)))
                    ),
                    "{set:?}"
                );
                // The search from `deeper` stops at the ceiling `main/sub`.
                assert_eq!(from_deeper.0, ffi::GIT_ENOTFOUND, "{set:?} {from_deeper:?}");
            }
            // Without `FromEnv` nothing is read: the search reaches `main`.
            assert_eq!(step(Some(&deeper), OpenFlags::empty()).0, 0);
        });
    }
}

/// Equivalence of [`RepositoryState`] with the `git_repository_state_t` values
/// libgit2 declares and reports.
///
/// [`RepositoryState`] must name exactly the header's constants, each with its C
/// value, and must name what `git_repository_state` reports for a repository
/// carrying each operation's marker files. A sequence on one repository adds
/// markers so that each step changes the state libgit2 reports (its probe order
/// decides which of several markers wins), then removes them with
/// `git_repository_state_cleanup`; after every step the value C reports, converted
/// with [`RepositoryState::from_raw`], must be the variant the markers predict and
/// convert back to the same C value. A bare repository, with its markers at the
/// top of the git directory, runs the same sequence.
///
/// `git_repository_state` reports `-1` only when copying the git directory's path
/// runs out of memory, which these tests cannot provoke; the unit tests pin that
/// [`RepositoryState::from_raw`] refuses it.
mod state {
    use std::ffi::{CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::RepositoryState;

    /// Every `git_repository_state_t` constant, beside the variant that must carry it.
    const STATES: [(ffi::git_repository_state_t, RepositoryState); 12] = [
        (ffi::GIT_REPOSITORY_STATE_NONE, RepositoryState::Clean),
        (ffi::GIT_REPOSITORY_STATE_MERGE, RepositoryState::Merge),
        (ffi::GIT_REPOSITORY_STATE_REVERT, RepositoryState::Revert),
        (
            ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE,
            RepositoryState::RevertSequence,
        ),
        (
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK,
            RepositoryState::CherryPick,
        ),
        (
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE,
            RepositoryState::CherryPickSequence,
        ),
        (ffi::GIT_REPOSITORY_STATE_BISECT, RepositoryState::Bisect),
        (ffi::GIT_REPOSITORY_STATE_REBASE, RepositoryState::Rebase),
        (
            ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE,
            RepositoryState::RebaseInteractive,
        ),
        (
            ffi::GIT_REPOSITORY_STATE_REBASE_MERGE,
            RepositoryState::RebaseMerge,
        ),
        (
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX,
            RepositoryState::ApplyMailbox,
        ),
        (
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE,
            RepositoryState::ApplyMailboxOrRebase,
        ),
    ];

    /// An object id for the `*_HEAD` references, which libgit2 resolves without
    /// reading the object.
    const HEAD_ID: &str = "0123456789abcdef0123456789abcdef01234567\n";

    /// One step: what it does to the git directory.
    enum Step {
        File(&'static str, &'static str),
        Dir(&'static str),
        RemoveDir(&'static str),
        Cleanup,
    }

    /// The sequence, each step with the state libgit2 must report after it: every
    /// state appears, and each marker outranks the ones added before it.
    fn sequence() -> Vec<(Step, RepositoryState)> {
        use RepositoryState::*;
        vec![
            (Step::File("BISECT_LOG", "# bisect\n"), Bisect),
            (Step::File("CHERRY_PICK_HEAD", HEAD_ID), CherryPick),
            (Step::Dir("sequencer"), CherryPick),
            (
                Step::File("sequencer/todo", "pick 0123456\n"),
                CherryPickSequence,
            ),
            (Step::File("REVERT_HEAD", HEAD_ID), RevertSequence),
            (Step::RemoveDir("sequencer"), Revert),
            (Step::File("MERGE_HEAD", HEAD_ID), Merge),
            (Step::Dir("rebase-apply"), ApplyMailboxOrRebase),
            (Step::File("rebase-apply/applying", ""), ApplyMailbox),
            (Step::File("rebase-apply/rebasing", ""), Rebase),
            (Step::Dir("rebase-merge"), RebaseMerge),
            (
                Step::File("rebase-merge/interactive", ""),
                RebaseInteractive,
            ),
            (Step::Cleanup, Clean),
            // Nothing left to remove, the references included: still clean.
            (Step::Cleanup, Clean),
        ]
    }

    /// A repository of its own under the temp dir, freed and removed on drop.
    struct StateRepo {
        base: PathBuf,
        gitdir: PathBuf,
        repo: *mut ffi::git_repository,
    }

    impl StateRepo {
        fn new(tag: &str, bare: bool) -> Self {
            // SAFETY: no arguments; the reference is never released.
            assert!(unsafe { ffi::git_libgit2_init() } >= 1);
            let base = std::env::temp_dir().join(format!(
                "crustify-repository-state-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&base);
            let c_path = CString::new(base.as_os_str().as_bytes()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's
            // defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_MKPATH;
            if bare {
                opts.flags |= ffi::GIT_REPOSITORY_INIT_BARE;
            }
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot, a NUL-terminated path and live options, read for the
            // call.
            let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
            assert_eq!(rc, 0);
            let gitdir = if bare {
                base.clone()
            } else {
                base.join(".git")
            };
            Self { base, gitdir, repo }
        }

        /// `git_repository_state`'s raw answer.
        fn state(&self) -> c_int {
            // SAFETY: the live repository, only read.
            unsafe { ffi::git_repository_state(self.repo) }
        }

        fn apply(&self, step: &Step) {
            let at = |p: &str| -> PathBuf { Path::new(&self.gitdir).join(p) };
            match step {
                Step::File(p, content) => fs::write(at(p), content).unwrap(),
                Step::Dir(p) => fs::create_dir(at(p)).unwrap(),
                Step::RemoveDir(p) => fs::remove_dir_all(at(p)).unwrap(),
                Step::Cleanup => {
                    // SAFETY: the live repository.
                    let rc = unsafe { ffi::git_repository_state_cleanup(self.repo) };
                    assert_eq!(rc, 0);
                }
            }
        }
    }

    impl Drop for StateRepo {
        fn drop(&mut self) {
            // SAFETY: the repository this fixture opened, freed once.
            unsafe { ffi::git_repository_free(self.repo) };
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    /// The variant C's answer names, checked to convert back to that answer.
    fn converted(raw: c_int) -> Option<RepositoryState> {
        let raw = ffi::git_repository_state_t::try_from(raw).ok()?;
        let state = RepositoryState::from_raw(raw)?;
        assert_eq!(state.raw(), raw);
        assert_eq!(ffi::git_repository_state_t::from(state), raw);
        Some(state)
    }

    #[test]
    fn variants_are_the_header_constants() {
        for (raw, state) in STATES {
            assert_eq!(state.raw(), raw, "{state:?}");
            assert_eq!(RepositoryState::from_raw(raw), Some(state));
        }
        // The constants are dense from 0: nothing past the last is named.
        assert_eq!(
            RepositoryState::from_raw(ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE + 1),
            None
        );
    }

    fn run(tag: &str, bare: bool) {
        let repo = StateRepo::new(tag, bare);
        let mut seen = vec![converted(repo.state())];
        assert_eq!(seen[0], Some(RepositoryState::Clean));
        for (i, (step, expected)) in sequence().iter().enumerate() {
            repo.apply(step);
            let raw = repo.state();
            let state = converted(raw);
            assert_eq!(state, Some(*expected), "step {i}: C reported {raw}");
            assert_eq!(state.unwrap().is_clean(), raw == 0);
            seen.push(state);
        }
        // Every state was reported once the sequence ran.
        for (_, state) in STATES {
            assert!(seen.contains(&Some(state)), "{state:?} never reported");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reported_states_match_the_markers() {
        run("workdir", false);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reported_states_match_the_markers_in_a_bare_repository() {
        run("bare", true);
    }
}

/// The `Repository` constructors against `git_repository_new`, `_new_ext`, `_open`,
/// `_open_bare` and `_init`.
///
/// Each side runs the same sequence of steps on a thread of its own, in a scratch
/// directory of its own, over the same fixtures (libgit2's `testrepo.git`, opened
/// read-only, and files each side writes alike). After every step it records the
/// constructor's outcome: for a repository, its state as libgit2 reports it
/// (`git_repository_path`, `_workdir`, `_commondir`, `_is_bare`, `_oid_type`), and for
/// a failure, the returned code and the thread's last error; after an `init`, every
/// file under the new repository and its contents. Paths under a side's scratch
/// directory are recorded relative to it. A successful constructor leaves whatever
/// error libgit2 set and recovered from on the thread, so only a failure's error is
/// compared.
///
/// Documented divergences, outside these comparisons:
///
/// - `git_repository_init` returns 1, and the repository it opened, when a reftable
///   repository's `reftable` directory cannot be made; `Repository::init` reports
///   that failure with `GIT_ERROR` and frees the repository (`unit_tests`).
/// - `git_repository_init` leaves a repository in its out slot on any failure after
///   its inner open; the wrapper frees it (`unit_tests`, `ub_safe`).
mod repository_owner {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    /// libgit2's bare test repository, only ever opened read-only.
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );

    /// One constructor call; paths are relative to the side's scratch directory,
    /// except [`Step::OpenTestrepo`]'s.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        New,
        WithOidType(OidType),
        OpenTestrepo { bare: bool },
        Open(&'static str),
        OpenBare(&'static str),
        Init(&'static str, bool),
    }

    /// A repository's state as libgit2 reports it, its paths relative to the scratch
    /// directory where they are under it.
    #[derive(Debug, PartialEq, Eq)]
    struct State {
        path: Option<String>,
        workdir: Option<String>,
        commondir: Option<String>,
        is_bare: c_int,
        oid_type: ffi::git_oid_t,
    }

    /// One step's outcome.
    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Repo(State),
        Failed {
            code: c_int,
            class: c_int,
            message: String,
        },
    }

    /// One step's record: its outcome, and after an `init` the files it left.
    type Record = (Outcome, Option<Vec<(String, Vec<u8>)>>);

    /// A side's scratch directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
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
            let path = if rel.is_empty() {
                // The empty path itself, not the scratch directory.
                return CString::default();
            } else {
                self.0.join(rel)
            };
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        /// `s` with the scratch directory replaced by `<root>`.
        fn relative(&self, s: &str) -> String {
            s.replace(&*self.0.to_string_lossy(), "<root>")
        }

        /// Every file under `rel`, with its contents, sorted by path.
        fn tree(&self, rel: &str) -> Vec<(String, Vec<u8>)> {
            fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    let name = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    if path.is_dir() {
                        out.push((format!("{name}/"), Vec::new()));
                        walk(&path, base, out);
                    } else {
                        out.push((name, fs::read(&path).unwrap()));
                    }
                }
            }
            let base = self.0.join(rel);
            let mut out = Vec::new();
            if base.is_dir() {
                walk(&base, &base, &mut out);
            }
            out.sort();
            out
        }

        /// The fixtures both sides open: a file to `init` under, a repository whose
        /// format version libgit2 refuses, one whose `core.bare` is no boolean, and a
        /// working directory whose `.git` file links to `work`'s repository.
        fn with_fixtures(tag: &str) -> Self {
            let this = Self::new(tag);
            fs::write(this.0.join("file"), "").unwrap();
            let badbare = this.0.join("badbare");
            fs::create_dir_all(badbare.join("objects")).unwrap();
            fs::create_dir_all(badbare.join("refs")).unwrap();
            fs::write(badbare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(badbare.join("config"), "[core]\n\tbare = maybe\n").unwrap();
            fs::create_dir_all(this.0.join("linked")).unwrap();
            fs::write(this.0.join("linked/.git"), "gitdir: ../work/.git\n").unwrap();
            let format = this.0.join("format");
            fs::create_dir_all(format.join("objects")).unwrap();
            fs::create_dir_all(format.join("refs")).unwrap();
            fs::write(format.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(
                format.join("config"),
                "[core]\n\trepositoryformatversion = 9\n",
            )
            .unwrap();
            this
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A NULL or NUL-terminated string libgit2 returned, copied.
    ///
    /// # Safety
    ///
    /// `s` must be NULL or a live C string.
    unsafe fn copied(scratch: &Scratch, s: *const c_char) -> Option<String> {
        // SAFETY: per this function's contract.
        (!s.is_null()).then(|| scratch.relative(&unsafe { CStr::from_ptr(s) }.to_string_lossy()))
    }

    /// The state of the live repository `repo`.
    fn state(scratch: &Scratch, repo: *mut ffi::git_repository) -> State {
        // SAFETY: `repo` is live; each getter only reads it, and the strings they
        // return live as long as it does, past the copies.
        unsafe {
            State {
                path: copied(scratch, ffi::git_repository_path(repo)),
                workdir: copied(scratch, ffi::git_repository_workdir(repo)),
                commondir: copied(scratch, ffi::git_repository_commondir(repo)),
                is_bare: ffi::git_repository_is_bare(repo),
                oid_type: ffi::git_repository_oid_type(repo),
            }
        }
    }

    /// The step's paths: the one passed, and the one an `init` fills.
    fn paths(scratch: &Scratch, step: Step) -> (CString, Option<&'static str>) {
        match step {
            Step::New | Step::WithOidType(_) => (CString::default(), None),
            Step::OpenTestrepo { .. } => (CString::new(TESTREPO).unwrap(), None),
            Step::Open(rel) | Step::OpenBare(rel) => (scratch.c_path(rel), None),
            Step::Init(rel, _) => (scratch.c_path(rel), Some(rel)),
        }
    }

    /// One step through libgit2's C API.
    fn c_step(scratch: &Scratch, step: Step) -> Record {
        let (path, filled) = paths(scratch, step);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, `path` NUL-terminated and `opts` live
        // options, each read for the call only; the runtime is initialised.
        let rc = unsafe {
            match step {
                Step::New => ffi::git_repository_new(&mut out),
                Step::WithOidType(kind) => {
                    let mut opts = ffi::git_repository_new_options {
                        version: ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION,
                        oid_type: kind.raw(),
                    };
                    ffi::git_repository_new_ext(&mut out, &mut opts)
                }
                Step::OpenTestrepo { bare: false } | Step::Open(_) => {
                    ffi::git_repository_open(&mut out, path.as_ptr())
                }
                Step::OpenTestrepo { bare: true } | Step::OpenBare(_) => {
                    ffi::git_repository_open_bare(&mut out, path.as_ptr())
                }
                Step::Init(_, bare) => {
                    ffi::git_repository_init(&mut out, path.as_ptr(), bare.into())
                }
            }
        };
        let outcome = if rc == 0 {
            Outcome::Repo(state(scratch, out))
        } else {
            let (class, message) = c_last();
            Outcome::Failed {
                code: rc,
                class,
                message: scratch.relative(&message),
            }
        };
        // SAFETY: `out` is NULL or the repository the call produced, freed once.
        unsafe { ffi::git_repository_free(out) };
        (outcome, filled.map(|rel| scratch.tree(rel)))
    }

    /// One step through the Rust API.
    fn rust_step(scratch: &Scratch, step: Step) -> Record {
        let (path, filled) = paths(scratch, step);
        let result: Result<RepositoryBox, RepositoryError> = match step {
            Step::New => Repository::new(),
            Step::WithOidType(kind) => Repository::with_oid_type(kind),
            Step::OpenTestrepo { bare: false } | Step::Open(_) => Repository::open(&path),
            Step::OpenTestrepo { bare: true } | Step::OpenBare(_) => Repository::open_bare(&path),
            Step::Init(_, bare) => Repository::init(&path, bare),
        };
        let outcome = match &result {
            Ok(repo) => Outcome::Repo(state(scratch, repo.as_c_ptr())),
            Err(error) => Outcome::Failed {
                code: error.raw_code(),
                class: error.raw_class(),
                message: scratch.relative(&error.message().to_string_lossy()),
            },
        };
        drop(result);
        (outcome, filled.map(|rel| scratch.tree(rel)))
    }

    /// The calling thread's last error, as C reads it.
    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; the result is a static error or this
        // thread's, live until the next libgit2 call on this thread, after the copy.
        unsafe {
            let e = ffi::git_error_last();
            let message = (*e).message;
            let message = if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            ((*e).klass, message)
        }
    }

    /// Initialises libgit2 for the C side; never released.
    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// Runs `steps` on each side, each on a thread and in a scratch directory of its
    /// own, and compares the records step by step.
    fn compare(tag: &str, steps: &'static [Step]) {
        init();
        let side = |name: &'static str, step: fn(&Scratch, Step) -> Record| {
            let tag = format!("{tag}-{name}");
            thread::spawn(move || {
                let scratch = Scratch::with_fixtures(&tag);
                steps.iter().map(|&s| step(&scratch, s)).collect::<Vec<_>>()
            })
        };
        let c = side("c", c_step).join().unwrap();
        let rust = side("rust", rust_step).join().unwrap();
        for (i, ((c, rust), step)) in c.iter().zip(&rust).zip(steps).enumerate() {
            assert_eq!(c, rust, "step {i}: {step:?}");
        }
        assert_eq!(c.len(), rust.len());
    }

    /// The constructors without paths, then libgit2's test repository.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_and_open_build_what_c_builds() {
        compare(
            "open",
            &[
                Step::New,
                Step::WithOidType(OidType::Sha1),
                Step::WithOidType(OidType::Sha256),
                Step::New,
                Step::OpenTestrepo { bare: false },
                Step::OpenTestrepo { bare: true },
                Step::OpenTestrepo { bare: false },
                Step::Open(""),
                Step::Open("missing"),
                Step::Open("."),
                Step::Open("file"),
                Step::Open("format"),
                Step::Open("badbare"),
                Step::Open("linked"),
                Step::OpenBare(""),
                Step::OpenBare("missing"),
                Step::OpenBare("."),
                Step::OpenTestrepo { bare: true },
            ],
        );
    }

    /// Repositories created, reinitialised and reopened, each in the sequence C
    /// takes them through; the files `init` writes are compared whole.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_creates_and_reopens_what_c_does() {
        compare(
            "init",
            &[
                Step::Init("bare", true),
                Step::Init("work", false),
                Step::Init("nested/deeper/work", false),
                Step::Open("bare"),
                Step::Open("work"),
                Step::Open("work/.git"),
                Step::Open("nested/deeper/work"),
                Step::Open("linked"),
                Step::OpenBare("bare"),
                Step::OpenBare("work"),
                Step::OpenBare("work/.git"),
                // Reinitialised: as bare and not, over each kind.
                Step::Init("bare", true),
                Step::Init("work", false),
                Step::Init("bare", false),
                Step::Init("work", true),
                Step::Open("bare"),
                Step::Open("work"),
                // Refused.
                Step::Init("", true),
                Step::Init("file", true),
                Step::Init("file/repo", false),
                Step::Init("format", true),
                Step::Open("bare"),
            ],
        );
    }

    /// `git_repository_open_bare` leaks the repository it built when it refuses the
    /// format, as `Repository::open_bare` then does, so the step runs in a child
    /// process with leak detection off; both sides must refuse it alike.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_bare_refuses_a_format_as_c_does() {
        const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";
        let path = concat!(module_path!(), "::open_bare_refuses_a_format_as_c_does");
        let name = path.split_once("::").map_or(path, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            compare("open-bare-format", &[Step::OpenBare("format"), Step::New]);
            return;
        }
        let asan = std::env::var("ASAN_OPTIONS").unwrap_or_default();
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .env("ASAN_OPTIONS", format!("{asan}:detect_leaks=0"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// [`InitOptions`] against the `git_repository_init_options` C code builds.
///
/// [`InitOptions::new`] must build what `git_repository_init_options_init` and
/// `GIT_REPOSITORY_INIT_OPTIONS_INIT` do. Then one sequence of option changes runs
/// twice, under two roots with independently allocated strings: once on a C struct
/// written field by field, as C code writes one, and once on an [`InitOptions`] through
/// its setters, each persisting across the steps. After every step the getters must
/// read what the C struct holds (strings by content, each root masked), and
/// `git_repository_init_ext` must behave the same with either: its return, the error
/// of a failure, the flags it leaves in the options, and every file and directory it
/// creates (kinds, permission bits, contents). The init routine is not wrapped yet, so
/// the Rust options reach it through their documented `#[repr(transparent)]` layout.
///
/// [`InitOptions`]: libgit2::libgit2::repository::repository::InitOptions
/// [`InitOptions::new`]: libgit2::libgit2::repository::repository::InitOptions::new
mod init_options {
    use std::collections::BTreeMap;
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::refs::refdb::RefdbType;
    use libgit2::libgit2::repository::repository::{
        InitFlag, InitFlags, InitMode, InitOptions, InitOptionsRef,
    };

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `git_repository_init_options_init(version)` into a zeroed struct.
    fn c_options(version: u32) -> (c_int, ffi::git_repository_init_options) {
        let mut opts = ffi::git_repository_init_options {
            version: 0,
            flags: 0,
            mode: 0,
            workdir_path: ptr::null(),
            description: ptr::null(),
            template_path: ptr::null(),
            initial_head: ptr::null(),
            origin_url: ptr::null(),
            oid_type: 0,
            refdb_type: 0,
        };
        // SAFETY: `opts` is a live options struct the call may overwrite.
        let rc = unsafe { ffi::git_repository_init_options_init(&mut opts, version) };
        (rc, opts)
    }

    /// The C struct's fields: the integers, then the strings with `root` masked.
    type Fields = ([u32; 5], [Option<String>; 5]);

    /// The string at `s`, NULL as `None`, with `root` masked.
    fn masked(s: *const std::ffi::c_char, root: &Path) -> Option<String> {
        (!s.is_null()).then(|| {
            // SAFETY: a non-NULL field holds a NUL-terminated string a live `CString`
            // of the test owns.
            let s = unsafe { CStr::from_ptr(s) }.to_str().unwrap();
            s.replace(root.to_str().unwrap(), "<root>")
        })
    }

    fn c_fields(o: &ffi::git_repository_init_options, root: &Path) -> Fields {
        (
            [o.version, o.flags, o.mode, o.oid_type, o.refdb_type],
            [
                masked(o.workdir_path, root),
                masked(o.description, root),
                masked(o.template_path, root),
                masked(o.initial_head, root),
                masked(o.origin_url, root),
            ],
        )
    }

    fn rust_fields(o: InitOptionsRef<'_, '_>, root: &Path) -> Fields {
        let s = |s: Option<&CStr>| masked(s.map_or(ptr::null(), CStr::as_ptr), root);
        (
            [
                o.version(),
                o.flags().bits(),
                o.mode().raw(),
                o.oid_type().map_or(0, OidType::raw),
                o.refdb_type().map_or(0, RefdbType::raw),
            ],
            [
                s(o.workdir_path()),
                s(o.description()),
                s(o.template_path()),
                s(o.initial_head()),
                s(o.origin_url()),
            ],
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_c_initialisers() {
        let root = Path::new("/");
        let (rc, c) = c_options(ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION);
        assert_eq!(rc, 0);
        let rust = InitOptions::new();
        assert_eq!(rust_fields(rust.as_ref(), root), c_fields(&c, root));
        // `GIT_REPOSITORY_INIT_OPTIONS_INIT` is `{VERSION}`: every other field zero.
        let mut macro_form = c_options(0).1;
        macro_form.version = ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION;
        assert_eq!(
            rust_fields(rust.as_ref(), root),
            c_fields(&macro_form, root)
        );
        assert_eq!(rust.as_ref().version(), InitOptions::VERSION);
        // C refuses other versions, leaving the struct as it was; `new` has none.
        for version in [0, 2] {
            let (rc, c) = c_options(version);
            assert_eq!((rc, c.version), (-1, 0), "version {version}");
        }
    }

    /// A scratch directory, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-init-options-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("tpl/hooks")).unwrap();
            fs::write(path.join("tpl/hooks/post-commit"), "#!/bin/sh\n").unwrap();
            fs::write(path.join("tpl/description"), "from the template\n").unwrap();
            fs::create_dir_all(path.join("empty-tpl")).unwrap();
            Self(path)
        }

        fn c(&self, rel: &str) -> CString {
            CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// One entry of a created tree: a directory's permission bits, or a file's and its
    /// contents with the root masked; a reftable stack only by its size, its table
    /// names being random.
    #[derive(Debug, PartialEq, Eq)]
    enum Entry {
        Dir(u32),
        File(u32, String),
        Tables(usize),
    }

    /// Every entry under `root` but the templates, by relative path.
    fn snapshot(root: &Path) -> BTreeMap<String, Entry> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        let mask = root.to_str().unwrap();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                if rel == "tpl" || rel == "empty-tpl" {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).unwrap();
                let bits = meta.permissions().mode() & 0o7777;
                if meta.is_dir() && path.file_name().unwrap() == "reftable" {
                    out.insert(rel, Entry::Tables(fs::read_dir(&path).unwrap().count()));
                } else if meta.is_dir() {
                    out.insert(rel, Entry::Dir(bits));
                    stack.push(path);
                } else {
                    let text = fs::read_to_string(&path).unwrap();
                    out.insert(rel, Entry::File(bits, text.replace(mask, "<root>")));
                }
            }
        }
        out
    }

    /// One call's record: the return, the class and message of a failure (root
    /// masked), and the options' flags afterwards.
    type Call = (c_int, Option<(c_int, String)>, u32);

    /// `git_repository_init_ext(root/path, opts)`, freeing any repository it leaves.
    fn init_ext(root: &Path, path: &str, opts: *mut ffi::git_repository_init_options) -> Call {
        let c_path = CString::new(root.join(path).as_os_str().as_bytes()).unwrap();
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot, `c_path` NUL-terminated and `opts` live options
        // whose strings outlive the call.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), opts) };
        // SAFETY: `repo` is NULL or the repository the call left, released once here.
        unsafe { ffi::git_repository_free(repo) };
        // A successful open leaves a stale error behind, so only a failure's counts.
        let error = (rc != 0).then(|| {
            // SAFETY: after a failure `git_error_last` is this thread's error, live
            // until the next libgit2 call on this thread; it is copied out first.
            let e = unsafe { &*ffi::git_error_last() };
            // SAFETY: a set error's message is a NUL-terminated string.
            let message = unsafe { CStr::from_ptr(e.message) }.to_string_lossy();
            (e.klass, message.replace(root.to_str().unwrap(), "<root>"))
        });
        // SAFETY: `opts` is live; the flags are plain data the call may have written.
        (rc, error, unsafe { (*opts).flags })
    }

    /// The strings one side owns, allocated independently of the other side's.
    struct Strings {
        template: CString,
        empty_template: CString,
        missing_template: CString,
        description: CString,
        head: CString,
        full_head: CString,
        url: CString,
        workdir: CString,
    }

    impl Strings {
        fn new(root: &Root) -> Self {
            Self {
                template: root.c("tpl"),
                empty_template: CString::new("").unwrap(),
                missing_template: root.c("no-such-tpl"),
                description: CString::new("described\n").unwrap(),
                head: CString::new("trunk").unwrap(),
                full_head: CString::new("refs/heads/other").unwrap(),
                url: CString::new("https://example.com/r.git").unwrap(),
                workdir: CString::new("../wd").unwrap(),
            }
        }
    }

    /// One change to the options, spelled both ways: through the C struct's fields
    /// and through the setters.
    struct Change {
        c: fn(&mut ffi::git_repository_init_options, &Strings),
        rust: for<'o> fn(&mut InitOptions<'o>, &'o Strings),
    }

    /// A step: the change, then an init of this path.
    struct Step {
        tag: &'static str,
        change: Change,
        path: &'static str,
    }

    fn steps() -> Vec<Step> {
        use InitFlag::*;
        use ffi::{
            GIT_REPOSITORY_INIT_BARE as BARE, GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE as EXTERNAL,
            GIT_REPOSITORY_INIT_MKDIR as MKDIR, GIT_REPOSITORY_INIT_MKPATH as MKPATH,
            GIT_REPOSITORY_INIT_NO_REINIT as NO_REINIT,
            GIT_REPOSITORY_INIT_RELATIVE_GITLINK as RELATIVE,
        };
        macro_rules! change {
            (|$c:ident, $cs:ident| $cbody:expr, |$r:ident, $rs:ident| $rbody:expr) => {
                Change {
                    c: |$c, $cs| {
                        let _ = (&$c, &$cs);
                        $cbody
                    },
                    rust: |$r, $rs| {
                        let _ = &$rs;
                        let mut $r = $r.as_mut();
                        let _ = &mut $r;
                        $rbody
                    },
                }
            };
        }
        vec![
            Step {
                tag: "a working directory, `.git` appended",
                change: change!(|c, s| c.flags = MKPATH, |r, s| r.set_flags(Mkpath.into())),
                path: "plain",
            },
            Step {
                tag: "head, description and origin, into a `.git` path",
                change: change!(
                    |c, s| {
                        c.initial_head = s.head.as_ptr();
                        c.description = s.description.as_ptr();
                        c.origin_url = s.url.as_ptr();
                    },
                    |r, s| {
                        r.set_initial_head(Some(&s.head));
                        r.set_description(Some(&s.description));
                        r.set_origin_url(Some(&s.url));
                    }
                ),
                path: "named/.git",
            },
            Step {
                tag: "the same options again: a reinit that re-adds origin",
                change: change!(|c, s| (), |r, s| ()),
                path: "named/.git",
            },
            Step {
                tag: "a reinit without origin, to a full reference",
                change: change!(
                    |c, s| {
                        c.origin_url = ptr::null();
                        c.initial_head = s.full_head.as_ptr();
                    },
                    |r, s| {
                        r.set_origin_url(None);
                        r.set_initial_head(Some(&s.full_head));
                    }
                ),
                path: "named/.git",
            },
            Step {
                tag: "a bare shared SHA-256 repository from an external template",
                change: change!(
                    |c, s| {
                        c.flags = BARE | MKDIR;
                        c.mode = ffi::GIT_REPOSITORY_INIT_SHARED_GROUP;
                        c.template_path = s.template.as_ptr();
                        c.oid_type = ffi::GIT_OID_SHA256;
                        c.initial_head = ptr::null();
                        c.description = ptr::null();
                    },
                    |r, s| {
                        r.set_flags(Bare | Mkdir);
                        r.set_mode(InitMode::SharedGroup);
                        r.set_template_path(Some(&s.template));
                        r.set_oid_type(Some(OidType::Sha256));
                        r.set_initial_head(None);
                        r.set_description(None);
                    }
                ),
                path: "bare-sha256",
            },
            Step {
                tag: "a bare reftable repository with a custom mode",
                change: change!(
                    |c, s| {
                        c.flags = BARE | MKPATH;
                        c.mode = 0o750;
                        c.template_path = ptr::null();
                        c.oid_type = 0;
                        c.refdb_type = ffi::GIT_REFDB_REFTABLE;
                    },
                    |r, s| {
                        r.set_flags(Bare | Mkpath);
                        r.set_mode(InitMode::from_raw(0o750));
                        r.set_template_path(None);
                        r.set_oid_type(None);
                        r.set_refdb_type(Some(RefdbType::Reftable));
                    }
                ),
                path: "deep/rt",
            },
            Step {
                tag: "a relative working directory with a relative gitlink",
                change: change!(
                    |c, s| {
                        c.flags = MKPATH | RELATIVE;
                        c.mode = ffi::GIT_REPOSITORY_INIT_SHARED_UMASK;
                        c.refdb_type = ffi::GIT_REFDB_FILES;
                        c.workdir_path = s.workdir.as_ptr();
                    },
                    |r, s| {
                        r.set_flags(Mkpath | RelativeGitlink);
                        r.set_mode(InitMode::SharedUmask);
                        r.set_refdb_type(Some(RefdbType::Files));
                        r.set_workdir_path(Some(&s.workdir));
                    }
                ),
                path: "sep/repo",
            },
            Step {
                tag: "a shared bare repository from the configured template",
                change: change!(
                    |c, s| {
                        c.flags = BARE | MKPATH | EXTERNAL;
                        c.mode = ffi::GIT_REPOSITORY_INIT_SHARED_ALL;
                        c.workdir_path = ptr::null();
                    },
                    |r, s| {
                        r.set_flags(Bare | Mkpath | ExternalTemplate);
                        r.set_mode(InitMode::SharedAll);
                        r.set_workdir_path(None);
                    }
                ),
                path: "all",
            },
            Step {
                tag: "a `.git/` path without a working directory",
                change: change!(
                    |c, s| {
                        c.flags = MKPATH;
                        c.mode = ffi::GIT_REPOSITORY_INIT_SHARED_UMASK;
                    },
                    |r, s| {
                        r.set_flags(Mkpath.into());
                        r.set_mode(InitMode::SharedUmask);
                    }
                ),
                path: "slash/.git/",
            },
            Step {
                tag: "version 0",
                change: change!(|c, s| c.version = 0, |r, s| r.set_version(0)),
                path: "never",
            },
            Step {
                tag: "version 2",
                change: change!(|c, s| c.version = 2, |r, s| r.set_version(2)),
                path: "never",
            },
            Step {
                tag: "no reinit over an existing repository",
                change: change!(
                    |c, s| {
                        c.version = 1;
                        c.workdir_path = ptr::null();
                        c.flags = MKPATH | NO_REINIT;
                    },
                    |r, s| {
                        r.set_version(1);
                        r.set_workdir_path(None);
                        r.set_flags(Mkpath | NoReinit);
                    }
                ),
                path: "plain",
            },
            Step {
                tag: "an empty external template copies nothing",
                change: change!(
                    |c, s| {
                        c.flags = BARE | EXTERNAL | MKDIR;
                        c.template_path = s.empty_template.as_ptr();
                        c.description = s.description.as_ptr();
                    },
                    |r, s| {
                        r.set_flags(Bare | ExternalTemplate | Mkdir);
                        r.set_template_path(Some(&s.empty_template));
                        r.set_description(Some(&s.description));
                    }
                ),
                path: "empty",
            },
            Step {
                tag: "a missing template falls back to the built-in one",
                change: change!(
                    |c, s| c.template_path = s.missing_template.as_ptr(),
                    |r, s| r.set_template_path(Some(&s.missing_template))
                ),
                path: "missing",
            },
            Step {
                tag: "every string cleared, the default types",
                change: change!(
                    |c, s| {
                        c.flags = BARE | MKDIR;
                        c.template_path = ptr::null();
                        c.description = ptr::null();
                        c.oid_type = 0;
                        c.refdb_type = 0;
                    },
                    |r, s| {
                        r.set_flags(InitFlags::from(Bare) | Mkdir);
                        r.set_template_path(None);
                        r.set_description(None);
                        r.set_oid_type(None);
                        r.set_refdb_type(None);
                    }
                ),
                path: "cleared",
            },
        ]
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_ext_reads_rust_options_as_c_options() {
        init();
        let roots = [Root::new("c"), Root::new("rust")];
        let strings = [Strings::new(&roots[0]), Strings::new(&roots[1])];
        let mut c = c_options(ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION).1;
        let mut rust = InitOptions::new();
        let mut calls = Vec::new();
        for step in steps() {
            (step.change.c)(&mut c, &strings[0]);
            (step.change.rust)(&mut rust, &strings[1]);
            assert_eq!(
                rust_fields(rust.as_ref(), &roots[1].0),
                c_fields(&c, &roots[0].0),
                "{}: fields",
                step.tag
            );
            let c_call = init_ext(&roots[0].0, step.path, &mut c);
            let rust_ptr = ptr::from_mut(&mut rust).cast::<ffi::git_repository_init_options>();
            let rust_call = init_ext(&roots[1].0, step.path, rust_ptr);
            assert_eq!(rust_call, c_call, "{}", step.tag);
            assert_eq!(
                rust_fields(rust.as_ref(), &roots[1].0),
                c_fields(&c, &roots[0].0),
                "{}: fields after the call",
                step.tag
            );
            assert_eq!(snapshot(&roots[1].0), snapshot(&roots[0].0), "{}", step.tag);
            calls.push((step.tag, c_call));
        }
        // The sequence reached what it meant to: successes, the re-added origin, the
        // version refusals and the refused reinit.
        let rc: Vec<_> = calls.iter().map(|(_, (rc, _, _))| *rc).collect();
        assert_eq!(
            rc,
            [0, 0, -4, 0, 0, 0, 0, 0, 0, -1, -1, -4, 0, 0, 0],
            "{calls:#?}"
        );
        // The internal bits libgit2 ORed into both sides' flags.
        assert_eq!(calls[1].1.2 & !0xffff, 1 << 16 | 1 << 17);
        assert_eq!(calls[2].1.2 & !0xffff, 1 << 16 | 1 << 17 | 1 << 18);
        let snap = snapshot(&roots[0].0);
        let text = |path: &str| match snap.get(path) {
            Some(Entry::File(_, text)) => text.clone(),
            other => panic!("{path}: {other:?}"),
        };
        assert_eq!(text("named/.git/HEAD"), "ref: refs/heads/other\n");
        assert_eq!(text("named/.git/description"), "described\n");
        assert!(text("named/.git/config").contains("url = https://example.com/r.git"));
        assert!(text("bare-sha256/config").contains("objectformat = sha256"));
        assert!(text("deep/rt/config").contains("refstorage = reftable"));
        assert!(text("all/config").contains("sharedrepository = 2"));
        assert!(matches!(snap.get("deep/rt/reftable"), Some(Entry::Tables(n)) if *n > 0));
        assert!(snap.contains_key("bare-sha256/hooks/post-commit"));
        assert!(!snap.contains_key("empty/description"));
        assert!(snap.contains_key("missing/description"));
        assert!(
            matches!(snap.get("sep/wd/.git"), Some(Entry::File(_, s)) if s == "gitdir: ../repo/\n")
        );
    }
}

/// `Repository::wrap_odb`, `RepositoryRef::workdir` and `RepositoryMut::cleanup_state`
/// against `git_repository_wrap_odb`, `git_repository_workdir` and
/// `git_repository_state_cleanup`.
///
/// Each case builds its inputs twice, independently: a C side through raw calls and a
/// Rust side through the safe API (the repository or odb constructors, and the routine
/// under test), under two roots.
///
/// - `workdir` must read the same path (the root masked), or none, for a repository
///   with a working directory, a bare one, one opened at a linked worktree, and one
///   without paths.
/// - A repository over an odb must report the odb's object id type, no git directory or
///   working directory, and the objects the odb holds (`git_repository_odb`, then
///   `git_odb_exists`), after the caller's odb reference is gone on both sides.
/// - Cleaning up the state runs over sequences of marker files and references: after
///   each step the return, the error, `git_repository_state` and the markers left must
///   agree. A repository without paths is refused by Rust before C, which would clean
///   up the process's working directory (a unit test), so it is not compared here.
mod routines {
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::odb::odb::Odb;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The thread's last error as C reports it.
    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; `git_error_last` never returns NULL, and
        // its message is a C string, live until the next libgit2 call on this thread.
        unsafe {
            let e = ffi::git_error_last();
            let message = CStr::from_ptr((*e).message).to_string_lossy().into_owned();
            ((*e).klass, message)
        }
    }

    fn c_path(path: &Path) -> CString {
        CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    /// A root of its own under the temp dir, canonical, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Self {
            init();
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-routines-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn mask(&self, s: &str) -> String {
            s.replace(&self.0.display().to_string(), "<root>")
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A raw repository, freed on drop.
    struct CRepo(*mut ffi::git_repository);

    impl CRepo {
        fn init(path: &Path, bare: bool) -> Self {
            let mut out = ptr::null_mut();
            let path = c_path(path);
            // SAFETY: a writable slot and a NUL-terminated path, read for the call.
            let rc = unsafe { ffi::git_repository_init(&mut out, path.as_ptr(), bare.into()) };
            assert_eq!(rc, 0);
            Self(out)
        }

        fn open(path: &Path) -> Self {
            let mut out = ptr::null_mut();
            let path = c_path(path);
            // SAFETY: a writable slot and a NUL-terminated path, read for the call.
            let rc = unsafe { ffi::git_repository_open(&mut out, path.as_ptr()) };
            assert_eq!(rc, 0);
            Self(out)
        }

        fn new() -> Self {
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot.
            assert_eq!(unsafe { ffi::git_repository_new(&mut out) }, 0);
            Self(out)
        }
    }

    impl Drop for CRepo {
        fn drop(&mut self) {
            // SAFETY: the repository this value made, released once.
            unsafe { ffi::git_repository_free(self.0) }
        }
    }

    /// Links the worktree `wt` of the non-bare repository at `main`, by hand.
    fn link_worktree(main: &Path, wt: &Path) {
        let admin = main.join(".git/worktrees/wt");
        fs::create_dir_all(&admin).unwrap();
        fs::create_dir_all(wt).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    }

    /// The working directory C reads, masked.
    fn c_workdir(root: &Root, repo: &CRepo) -> Option<String> {
        // SAFETY: a live repository, only read; the result is NULL or its string.
        let dir = unsafe { ffi::git_repository_workdir(repo.0) };
        // SAFETY: NULL-checked; the string lives as long as the repository.
        (!dir.is_null()).then(|| root.mask(&unsafe { CStr::from_ptr(dir) }.to_string_lossy()))
    }

    fn rust_workdir(root: &Root, repo: &RepositoryBox) -> Option<String> {
        let dir = repo.as_ref().workdir()?;
        Some(root.mask(&dir.to_string_lossy()))
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn workdir_matches_c() {
        let (c_root, rust_root) = (Root::new("workdir-c"), Root::new("workdir-rust"));
        for (rel, bare) in [("plain", false), ("bare.git", true)] {
            let c = CRepo::init(&c_root.0.join(rel), bare);
            let rust = Repository::init(&c_path(&rust_root.0.join(rel)), bare).unwrap();
            assert_eq!(
                rust_workdir(&rust_root, &rust),
                c_workdir(&c_root, &c),
                "{rel}"
            );
        }
        // Opened again, at the working directory, at the git directory, and at a
        // linked worktree.
        for root in [&c_root, &rust_root] {
            link_worktree(&root.0.join("plain"), &root.0.join("wt"));
        }
        for rel in ["plain", "plain/.git", "wt", "bare.git"] {
            let c = CRepo::open(&c_root.0.join(rel));
            let rust = Repository::open(&c_path(&rust_root.0.join(rel))).unwrap();
            assert_eq!(
                rust_workdir(&rust_root, &rust),
                c_workdir(&c_root, &c),
                "{rel}"
            );
        }
        let (c, rust) = (CRepo::new(), Repository::new().unwrap());
        assert_eq!(rust_workdir(&rust_root, &rust), c_workdir(&c_root, &c));
    }

    /// What a repository over an odb reports: its object id type, git directory,
    /// working directory, and which of `ids` its odb holds.
    fn wrapped_view(
        repo: *mut ffi::git_repository,
        ids: &[ffi::git_oid],
    ) -> (u32, bool, bool, Vec<bool>) {
        // SAFETY: a live repository and live ids, only read; the odb reference is
        // released once.
        unsafe {
            let mut odb = ptr::null_mut();
            assert_eq!(ffi::git_repository_odb(&mut odb, repo), 0);
            let held = ids
                .iter()
                .map(|id| ffi::git_odb_exists(odb, id) == 1)
                .collect();
            ffi::git_odb_free(odb);
            (
                ffi::git_repository_oid_type(repo),
                ffi::git_repository_path(repo).is_null(),
                ffi::git_repository_workdir(repo).is_null(),
                held,
            )
        }
    }

    /// Writes `data` as a blob into the objects directory `objects`; its id.
    fn write_blob(objects: &Path, oid_type: OidType, data: &[u8]) -> ffi::git_oid {
        let odb = Odb::open_with_oid_type(&c_path(objects), oid_type).unwrap();
        let mut id = ffi::git_oid::default();
        // SAFETY: a live odb, a writable id and `data` live for its length.
        let rc = unsafe {
            ffi::git_odb_write(
                &mut id,
                odb.as_c_ptr(),
                data.as_ptr().cast(),
                data.len(),
                ffi::GIT_OBJECT_BLOB,
            )
        };
        assert_eq!(rc, 0);
        id
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn wrap_odb_matches_c() {
        let (c_root, rust_root) = (Root::new("wrap-c"), Root::new("wrap-rust"));
        for (k, oid_type) in [OidType::Sha1, OidType::Sha256].into_iter().enumerate() {
            let rel = format!("objects-{k}");
            let (c_dir, rust_dir) = (c_root.0.join(&rel), rust_root.0.join(&rel));
            fs::create_dir_all(&c_dir).unwrap();
            fs::create_dir_all(&rust_dir).unwrap();
            let ids: Vec<_> = [&b"one"[..], b"two"]
                .iter()
                .map(|d| {
                    let id = write_blob(&c_dir, oid_type, d);
                    assert_eq!(write_blob(&rust_dir, oid_type, d).id, id.id);
                    id
                })
                .collect();
            let elsewhere = c_root.0.join(format!("elsewhere-{k}"));
            fs::create_dir_all(&elsewhere).unwrap();
            let absent = write_blob(&elsewhere, oid_type, b"three");
            let probe = [ids[0], ids[1], absent];

            let opts = ffi::git_odb_options {
                version: ffi::GIT_ODB_OPTIONS_VERSION,
                oid_type: oid_type.raw(),
            };
            let mut c_odb = ptr::null_mut();
            let mut c_repo = ptr::null_mut();
            let dir = c_path(&c_dir);
            // SAFETY: writable slots, a NUL-terminated path and live options; the
            // caller's odb reference is released once the repository holds its own.
            let c_rc = unsafe {
                assert_eq!(ffi::git_odb_open_ext(&mut c_odb, dir.as_ptr(), &opts), 0);
                let rc = ffi::git_repository_wrap_odb(&mut c_repo, c_odb);
                ffi::git_odb_free(c_odb);
                rc
            };
            let c_repo = CRepo(c_repo);

            let odb = Odb::open_with_oid_type(&c_path(&rust_dir), oid_type).unwrap();
            let rust_repo = Repository::wrap_odb(odb).unwrap();
            assert_eq!(c_rc, 0);
            assert_eq!(
                wrapped_view(rust_repo.as_c_ptr(), &probe),
                wrapped_view(c_repo.0, &probe)
            );
            assert_eq!(rust_repo.as_ref().workdir(), None);
        }
    }

    const MARKERS: [&str; 9] = [
        "MERGE_HEAD",
        "MERGE_MODE",
        "MERGE_MSG",
        "BISECT_LOG",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "REVERT_HEAD",
        "CHERRY_PICK_HEAD",
    ];

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Writes the marker file (or, for a directory marker, a file in it).
        Mark(&'static str),
        /// Makes the marker a directory holding a file.
        MarkDir(&'static str),
        Cleanup,
    }

    type Outcome = (c_int, Option<(c_int, String)>, u32, Vec<&'static str>);

    fn mark(gitdir: &Path, step: Step) {
        let id = "0123456789012345678901234567890123456789\n";
        match step {
            Step::Mark(m @ ("rebase-merge" | "rebase-apply" | "sequencer")) => {
                fs::create_dir_all(gitdir.join(m)).unwrap();
                fs::write(gitdir.join(m).join("todo"), "x\n").unwrap();
            }
            Step::Mark(m) => fs::write(gitdir.join(m), id).unwrap(),
            Step::MarkDir(m) => {
                fs::create_dir_all(gitdir.join(m)).unwrap();
                fs::write(gitdir.join(m).join("inner"), id).unwrap();
            }
            Step::Cleanup => unreachable!(),
        }
    }

    fn outcome(
        root: &Root,
        gitdir: &Path,
        repo: *mut ffi::git_repository,
        rc: c_int,
        error: Option<(c_int, String)>,
    ) -> Outcome {
        // SAFETY: a live repository on this thread.
        let state = unsafe { ffi::git_repository_state(repo) } as u32;
        let left = MARKERS
            .into_iter()
            .filter(|m| gitdir.join(m).exists())
            .collect();
        (rc, error.map(|(k, m)| (k, root.mask(&m))), state, left)
    }

    /// Runs `steps` on both sides, comparing after each; the C side's outcomes.
    fn compare_cleanup(tag: &str, bare: bool, steps: &[Step]) -> Vec<Outcome> {
        let (c_root, rust_root) = (
            Root::new(&format!("{tag}-c")),
            Root::new(&format!("{tag}-rust")),
        );
        let rel = if bare { "repo.git" } else { "repo" };
        let c = CRepo::init(&c_root.0.join(rel), bare);
        let mut rust = Repository::init(&c_path(&rust_root.0.join(rel)), bare).unwrap();
        let gitdir = |root: &Root| {
            if bare {
                root.0.join(rel)
            } else {
                root.0.join(rel).join(".git")
            }
        };
        let mut outcomes = Vec::new();
        for (k, &step) in steps.iter().enumerate() {
            let (c_out, rust_out) = match step {
                Step::Cleanup => {
                    // SAFETY: no arguments; clears this thread's error state.
                    unsafe { ffi::git_error_clear() };
                    // SAFETY: the live repository of the C side.
                    let rc = unsafe { ffi::git_repository_state_cleanup(c.0) };
                    let c_error = (rc < 0).then(c_last);
                    let c_out = outcome(&c_root, &gitdir(&c_root), c.0, rc, c_error);
                    let (rc, error) = match rust.as_mut().cleanup_state() {
                        Ok(()) => (0, None),
                        Err(e) => (
                            e.raw_code(),
                            Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                        ),
                    };
                    (
                        c_out,
                        outcome(&rust_root, &gitdir(&rust_root), rust.as_c_ptr(), rc, error),
                    )
                }
                step => {
                    mark(&gitdir(&c_root), step);
                    mark(&gitdir(&rust_root), step);
                    (
                        outcome(&c_root, &gitdir(&c_root), c.0, 0, None),
                        outcome(&rust_root, &gitdir(&rust_root), rust.as_c_ptr(), 0, None),
                    )
                }
            };
            assert_eq!(rust_out, c_out, "step {k} {step:?}");
            outcomes.push(c_out);
        }
        outcomes
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleanup_state_matches_c() {
        use Step::*;
        for bare in [false, true] {
            let outcomes = compare_cleanup(
                if bare { "cleanup-bare" } else { "cleanup" },
                bare,
                &[
                    Cleanup,
                    Mark("MERGE_HEAD"),
                    Mark("MERGE_MSG"),
                    Mark("MERGE_MODE"),
                    Cleanup,
                    Mark("rebase-merge"),
                    Mark("BISECT_LOG"),
                    Cleanup,
                    Mark("rebase-apply"),
                    Mark("sequencer"),
                    Mark("REVERT_HEAD"),
                    Cleanup,
                    Mark("CHERRY_PICK_HEAD"),
                    Mark("sequencer"),
                    Cleanup,
                    Cleanup,
                    MarkDir("MERGE_HEAD"),
                    MarkDir("BISECT_LOG"),
                    Cleanup,
                ],
            );
            // Every step succeeds, and the last cleanup leaves nothing behind.
            assert!(outcomes.iter().all(|o| o.0 == 0), "{outcomes:?}");
            assert_eq!(outcomes.last().unwrap().2, ffi::GIT_REPOSITORY_STATE_NONE);
            assert!(outcomes.last().unwrap().3.is_empty());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleanup_state_failures_match_c() {
        use Step::*;
        // A reference marker that is a directory: the reference backend cannot remove it.
        let outcomes = compare_cleanup(
            "cleanup-fail",
            false,
            &[
                Mark("MERGE_HEAD"),
                MarkDir("REVERT_HEAD"),
                Cleanup,
                Cleanup,
                MarkDir("CHERRY_PICK_HEAD"),
                Cleanup,
            ],
        );
        let failed: Vec<_> = outcomes.iter().filter(|o| o.0 < 0).collect();
        assert!(!failed.is_empty(), "{outcomes:?}");
    }
}

/// Equivalence of the routines on a repository and its handles with the C routines
/// they wrap.
///
/// Each test runs libgit2's routine on one repository and the wrapper on another,
/// independently owned and built the same way (a copy of the same fixture, or the same
/// constructor), applies the same sequence of steps to both, and compares after every
/// step: the return code, the error libgit2 set when it set one, the out values (paths,
/// buffers, ids, installed objects), and the state the step left on disk (`HEAD`, the
/// reflog's messages, the configuration, the gitlink, the marker files).
///
/// Documented divergences are skipped here and pinned by the unit tests: the wrappers
/// refuse a repository without a git directory where libgit2 uses the current directory
/// (or aborts, or crashes), refuse a NULL start without `FROM_ENV`, report an error
/// where libgit2 fails without setting one (`git_repository_message_remove`,
/// `git_repository_open_from_worktree`), and refuse ids and odbs of another type than
/// the repository's, where libgit2 overflows a buffer.
mod heads_and_parts {
    use std::ffi::{CStr, CString, c_int, c_void};
    use std::fs;
    use std::ops::ControlFlow;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::config::config::{Config, ConfigArc};
    use libgit2::libgit2::index::index::{Index, IndexArc};
    use libgit2::libgit2::odb::odb::Odb;
    use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
    use libgit2::libgit2::refs::annotated_commit::AnnotatedCommit;
    use libgit2::libgit2::repository::repository::{
        MergeheadCallback, OpenFlag, OpenFlags, Repository, RepositoryBox, RepositoryError,
    };
    use libgit2::libgit2::repository::worktree::WorktreeBox;

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    fn resource(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/resources")
            .join(name)
            .canonicalize()
            .unwrap()
    }

    fn c_path(path: &Path) -> CString {
        CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    /// A scratch directory of its own under the temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-routines-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn join(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// The calling thread's last error, as C reads it: class and message.
    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; the error is copied before the next
        // libgit2 call on this thread.
        let e = unsafe { ffi::git_error_last() };
        assert!(!e.is_null());
        // SAFETY: a live error whose message is NULL or a C string.
        unsafe {
            let message = (*e).message;
            let message = if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            ((*e).klass, message)
        }
    }

    /// A step's outcome: the code, and the error when it failed.
    type Outcome = (c_int, Option<(c_int, String)>);

    fn c_outcome(rc: c_int) -> Outcome {
        (rc, (rc < 0).then(c_last))
    }

    fn rust_outcome(result: Result<(), RepositoryError>) -> Outcome {
        match result {
            Ok(()) => (0, None),
            Err(e) => (
                e.raw_code(),
                Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
            ),
        }
    }

    /// A repository C opened, freed on drop.
    struct CRepo(*mut ffi::git_repository);

    impl CRepo {
        fn open(path: &Path) -> Self {
            init();
            let mut out = ptr::null_mut();
            // SAFETY: a live slot and a NUL-terminated path.
            let rc = unsafe { ffi::git_repository_open(&mut out, c_path(path).as_ptr()) };
            assert_eq!(rc, 0, "{:?}", c_last());
            Self(out)
        }

        fn new() -> Self {
            init();
            let mut out = ptr::null_mut();
            // SAFETY: a live slot.
            assert_eq!(unsafe { ffi::git_repository_new(&mut out) }, 0);
            Self(out)
        }
    }

    impl Drop for CRepo {
        fn drop(&mut self) {
            // SAFETY: the repository this test opened, released once.
            unsafe { ffi::git_repository_free(self.0) }
        }
    }

    /// A copy of `fixture` at `scratch/name`.
    fn copy(scratch: &Scratch, fixture: &str, name: &str) -> PathBuf {
        let path = scratch.join(name);
        copy_dir(&resource(fixture), &path);
        path
    }

    /// Two copies of `fixture`: one C opens, one the wrapper opens.
    fn pair(scratch: &Scratch, fixture: &str) -> (PathBuf, CRepo, PathBuf, RepositoryBox) {
        let c_dir = copy(scratch, fixture, "c");
        let rust_dir = copy(scratch, fixture, "rust");
        let c = CRepo::open(&c_dir);
        let rust = Repository::open(&c_path(&rust_dir)).unwrap();
        (c_dir, c, rust_dir, rust)
    }

    /// A C string libgit2 returned, copied, with `root` replaced by `<root>`.
    fn masked(p: *const std::ffi::c_char, root: &Path) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: the callers pass strings libgit2 returned, live for the copy.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        Some(s.replace(root.to_str().unwrap(), "<root>"))
    }

    fn read_masked(path: &Path, root: &Path) -> Option<String> {
        let s = fs::read_to_string(path).ok()?;
        Some(s.replace(root.to_str().unwrap(), "<root>"))
    }

    /// The messages of a reflog, one per line, without the identity and time.
    fn reflog(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("logs/HEAD"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split_once('\t').map_or("", |(_, m)| m).to_owned())
            .collect()
    }

    /// The identity of a reflog's last entry: the text between the ids and the time.
    fn reflog_identity(dir: &Path) -> Option<String> {
        let log = fs::read_to_string(dir.join("logs/HEAD")).ok()?;
        let last = log.lines().last()?;
        let (head, _) = last.split_once('>')?;
        Some(head.splitn(3, ' ').nth(2)?.to_owned())
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process.
    fn isolated(test: &str, body: fn()) {
        isolated_with(test, &[], false, body);
    }

    /// Runs `body` alone in a child process with `vars` set and, when `tolerate_leaks`,
    /// LeakSanitizer off (for the C leaks named where it is used).
    fn isolated_with(test: &str, vars: &[(&str, &Path)], tolerate_leaks: bool, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .envs(vars.iter().copied());
        if tolerate_leaks {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            child.env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            );
        }
        let out = child.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn paths_and_types_match_c() {
        init();
        let scratch = Scratch::new("paths");
        let main = scratch.join("main");
        let rust_main = Repository::init(&c_path(&main), false).unwrap();
        let wt = scratch.join("wt");
        let admin = main.join(".git/worktrees/wt");
        fs::create_dir_all(&admin).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        drop(rust_main);
        let dirs = [
            resource("testrepo.git"),
            resource("testrepo_256/.gitted"),
            main.clone(),
            wt.clone(),
        ];
        for dir in &dirs {
            let c = CRepo::open(dir);
            let rust = Repository::open(&c_path(dir)).unwrap();
            // SAFETY: a live repository, only read; the path lives as long as it.
            let c_path = unsafe { masked(ffi::git_repository_path(c.0), &scratch.0) };
            let rust_path = rust.as_ref().path().map(|p| {
                p.to_string_lossy()
                    .replace(scratch.0.to_str().unwrap(), "<root>")
            });
            assert_eq!(rust_path, c_path, "{dir:?}");
            // SAFETY: a live repository, only read.
            let c_type = unsafe { ffi::git_repository_oid_type(c.0) };
            assert_eq!(rust.as_ref().oid_type().raw(), c_type, "{dir:?}");
        }
        let c = CRepo::new();
        let rust = Repository::new().unwrap();
        // SAFETY: a live repository, only read.
        assert!(unsafe { ffi::git_repository_path(c.0) }.is_null());
        assert_eq!(rust.as_ref().path(), None);
        // SAFETY: as above.
        let c_type = unsafe { ffi::git_repository_oid_type(c.0) };
        assert_eq!(rust.as_ref().oid_type().raw(), c_type);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lent_odbs_and_refdbs_match_c() {
        let scratch = Scratch::new("lent");
        let (_, c, _, rust) = pair(&scratch, "testrepo.git");
        let c_new = CRepo::new();
        let rust_new = Repository::new().unwrap();
        for (c, rust) in [(c.0, &rust), (c_new.0, &rust_new)] {
            let mut odb = ptr::null_mut();
            // SAFETY: a live slot and repository.
            let rc = unsafe { ffi::git_repository_odb(&mut odb, c) };
            let c_side = if rc < 0 {
                Err(c_outcome(rc))
            } else {
                // SAFETY: the reference just returned, read and released once.
                let n = unsafe { ffi::git_odb_num_backends(odb) };
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
                Ok(n)
            };
            let rust_side = match rust.as_ref().odb() {
                // SAFETY: the arc's live odb, only read.
                Ok(odb) => Ok(unsafe { ffi::git_odb_num_backends(odb.as_c_ptr()) }),
                Err(e) => Err(rust_outcome(Err(e))),
            };
            assert_eq!(rust_side, c_side);

            let mut db = ptr::null_mut();
            // SAFETY: a live slot and repository.
            let rc = unsafe { ffi::git_repository_refdb(&mut db, c) };
            let c_side = c_outcome(rc);
            // SAFETY: NULL or the reference just returned, released once.
            unsafe { ffi::git_refdb_free(db) };
            let rust_side = rust_outcome(rust.as_ref().refdb().map(drop));
            assert_eq!(rust_side, c_side);
        }
    }

    /// Marker files, written into both git directories in turn; the state is compared
    /// after each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn states_match_c() {
        let scratch = Scratch::new("state");
        let (c_dir, c, rust_dir, rust) = pair(&scratch, "testrepo.git");
        let steps: [(&str, &str); 8] = [
            ("BISECT_LOG", "x\n"),
            (
                "CHERRY_PICK_HEAD",
                "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n",
            ),
            ("sequencer/todo", "pick\n"),
            ("REVERT_HEAD", "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n"),
            ("MERGE_HEAD", "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n"),
            ("rebase-apply/applying", ""),
            ("rebase-merge/head-name", "x\n"),
            ("rebase-merge/interactive", ""),
        ];
        let compare = || {
            // SAFETY: a live repository with a git directory.
            let c_state = unsafe { ffi::git_repository_state(c.0) };
            let rust_state = rust.as_ref().state().unwrap();
            assert_eq!(rust_state.raw() as c_int, c_state);
        };
        compare();
        for (file, contents) in steps {
            for dir in [&c_dir, &rust_dir] {
                let path = dir.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
            compare();
        }
    }

    /// `MERGE_MSG` absent, written, read, removed, and removed again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn message_sequences_match_c() {
        let scratch = Scratch::new("message");
        let (c_dir, c, rust_dir, rust) = pair(&scratch, "testrepo.git");
        let read = || {
            let mut buf = ffi::git_buf::default();
            // SAFETY: an empty buffer and a live repository.
            let rc = unsafe { ffi::git_repository_message(&mut buf, c.0) };
            let c_side = if rc < 0 {
                Err(c_outcome(rc))
            } else {
                // SAFETY: on success `ptr` holds `size` bytes the buffer owns.
                Ok(unsafe { std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size) }.to_vec())
            };
            // SAFETY: the call's buffer, disposed of once.
            unsafe { ffi::git_buf_dispose(&mut buf) };
            let rust_side = match rust.as_ref().message() {
                Ok(buf) => Ok(buf.as_ref().as_bytes().to_vec()),
                Err(e) => Err(rust_outcome(Err(e))),
            };
            assert_eq!(rust_side, c_side);
        };
        let remove = || {
            // SAFETY: a live repository with a git directory.
            let c_rc = unsafe { ffi::git_repository_message_remove(c.0) };
            // libgit2 sets no error when the unlink fails: compare the code only.
            let rust_rc = rust
                .as_ref()
                .remove_message()
                .map_or_else(|e| e.raw_code(), |()| 0);
            assert_eq!(rust_rc, c_rc);
        };
        read();
        remove();
        for contents in [&b"Merge branch 'x'\n"[..], b"", b"a\0b"] {
            fs::write(c_dir.join("MERGE_MSG"), contents).unwrap();
            fs::write(rust_dir.join("MERGE_MSG"), contents).unwrap();
            read();
            read();
            remove();
            read();
            remove();
        }
        // A directory in the way: libgit2 fails to read it.
        fs::create_dir(c_dir.join("MERGE_MSG")).unwrap();
        fs::create_dir(rust_dir.join("MERGE_MSG")).unwrap();
        read();
    }

    /// `HEAD` moved through every kind of target, compared with the reflog after each
    /// step.
    /// Alone in a child process: libgit2 writes the reflog through
    /// `git_signature_now`, whose glibc `tzset` TSan reports against any concurrent
    /// caller in the process.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn head_moves_match_c() {
        isolated(
            concat!(module_path!(), "::head_moves_match_c"),
            head_moves_match_c_alone,
        );
    }

    fn head_moves_match_c_alone() {
        let scratch = Scratch::new("heads");
        let (c_dir, c, rust_dir, rust) = pair(&scratch, "testrepo.git");
        let compare = |what: &str, c_rc: c_int, rust: Result<(), RepositoryError>| {
            assert_eq!(rust_outcome(rust), c_outcome(c_rc), "{what}");
            assert_eq!(
                fs::read_to_string(rust_dir.join("HEAD")).unwrap(),
                fs::read_to_string(c_dir.join("HEAD")).unwrap(),
                "{what}"
            );
            assert_eq!(reflog(&rust_dir), reflog(&c_dir), "{what}");
            assert_eq!(
                reflog_identity(&rust_dir),
                reflog_identity(&c_dir),
                "{what}"
            );
        };
        for name in [
            c"refs/heads/br2",
            c"refs/tags/test",
            c"refs/heads/unborn",
            c"refs/tags/missing",
            c"refs/tags/point_to_blob",
            c"refs/heads/master",
            c"HEAD",
        ] {
            // SAFETY: a live repository and a NUL-terminated name.
            let c_rc = unsafe { ffi::git_repository_set_head(c.0, name.as_ptr()) };
            compare(name.to_str().unwrap(), c_rc, rust.as_ref().set_head(name));
        }
        for hex in [
            c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
            c"b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
            c"0000000000000000000000000000000000000001",
            c"7b4384978d2493e851f9cca7858815fac9b10980",
        ] {
            let mut c_id = ffi::git_oid::default();
            // SAFETY: a writable id and a NUL-terminated hex string.
            let rc =
                unsafe { ffi::git_oid_from_string(&mut c_id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
            assert_eq!(rc, 0);
            // SAFETY: a live repository and a live id of its type.
            let c_rc = unsafe { ffi::git_repository_set_head_detached(c.0, &c_id) };
            let id = Oid::from_string(hex, OidType::Sha1).unwrap();
            compare(
                hex.to_str().unwrap(),
                c_rc,
                rust.as_ref().set_head_detached(id.as_ref()),
            );
        }
        // Each side annotates through its own repository.
        for spec in [c"br2", c"refs/tags/test", c"HEAD~1"] {
            let mut annotated = ptr::null_mut();
            // SAFETY: a live slot, repository and revspec.
            let rc = unsafe {
                ffi::git_annotated_commit_from_revspec(&mut annotated, c.0, spec.as_ptr())
            };
            assert_eq!(rc, 0);
            // SAFETY: a live repository and annotated commit, freed once below.
            let c_rc =
                unsafe { ffi::git_repository_set_head_detached_from_annotated(c.0, annotated) };
            // SAFETY: as above.
            unsafe { ffi::git_annotated_commit_free(annotated) };
            let commit = AnnotatedCommit::from_revspec(rust.as_ref(), spec).unwrap();
            let result = rust
                .as_ref()
                .set_head_detached_from_annotated(commit.as_ref());
            compare(spec.to_str().unwrap(), c_rc, result);
        }
    }

    /// The reflog identity a repository is given, then the namespace, read back.
    /// Alone in a child process: libgit2 writes the reflog through
    /// `git_signature_now`, whose glibc `tzset` TSan reports against any concurrent
    /// caller in the process.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn identities_and_namespaces_match_c() {
        isolated(
            concat!(module_path!(), "::identities_and_namespaces_match_c"),
            identities_and_namespaces_match_c_alone,
        );
    }

    fn identities_and_namespaces_match_c_alone() {
        let scratch = Scratch::new("ident");
        let (c_dir, c, rust_dir, mut rust) = pair(&scratch, "testrepo.git");
        let identities: [(Option<&CStr>, Option<&CStr>); 4] = [
            (Some(c"Name"), Some(c"name@example.com")),
            (Some(c"Only Name"), None),
            (None, Some(c"only@example.com")),
            (None, None),
        ];
        for (name, email) in identities {
            let raw = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: a live repository; the strings are copied.
            let c_rc = unsafe { ffi::git_repository_set_ident(c.0, raw(name), raw(email)) };
            let result = rust.as_mut().set_ident(name, email);
            assert_eq!(rust_outcome(result), c_outcome(c_rc));
            let read = |repo: *mut ffi::git_repository| {
                let (mut n, mut e) = (ptr::null(), ptr::null());
                // SAFETY: live slots and repository; the strings live as long as it.
                let rc = unsafe { ffi::git_repository_ident(&mut n, &mut e, repo) };
                assert_eq!(rc, 0);
                (masked(n, &scratch.0), masked(e, &scratch.0))
            };
            assert_eq!(read(rust.as_c_ptr()), read(c.0));
            // The identity reaches the reflog the same way.
            // SAFETY: a live repository and name.
            let c_rc = unsafe { ffi::git_repository_set_head(c.0, c"refs/heads/br2".as_ptr()) };
            assert_eq!(
                rust_outcome(rust.as_ref().set_head(c"refs/heads/br2")),
                c_outcome(c_rc)
            );
            assert_eq!(reflog_identity(&rust_dir), reflog_identity(&c_dir));
        }
        for namespace in [Some(c"space"), Some(c"nested/space"), None, Some(c"")] {
            let raw = namespace.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: a live repository; the string is copied.
            let c_rc = unsafe { ffi::git_repository_set_namespace(c.0, raw) };
            assert_eq!(
                rust_outcome(rust.as_mut().set_namespace(namespace)),
                c_outcome(c_rc)
            );
            // SAFETY: live repositories; the strings live as long as they do.
            let (c_ns, rust_ns) = unsafe {
                (
                    masked(ffi::git_repository_get_namespace(c.0), &scratch.0),
                    masked(
                        ffi::git_repository_get_namespace(rust.as_c_ptr()),
                        &scratch.0,
                    ),
                )
            };
            assert_eq!(rust_ns, c_ns);
        }
    }

    /// Working directories set with and without the gitlink, then bareness.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn workdirs_and_bareness_match_c() {
        init();
        let scratch = Scratch::new("workdir");
        let c_root = scratch.join("c");
        let rust_root = scratch.join("rust");
        let mut repos = Vec::new();
        for root in [&c_root, &rust_root] {
            fs::create_dir_all(root.join("w1")).unwrap();
            fs::create_dir_all(root.join("w2")).unwrap();
            fs::create_dir_all(root.join("blocked/.git")).unwrap();
            repos.push(Repository::init(&c_path(&root.join("repo")), false).unwrap());
        }
        drop(repos);
        let c = CRepo::open(&c_root.join("repo"));
        let mut rust = Repository::open(&c_path(&rust_root.join("repo"))).unwrap();
        let snapshot = |repo: *mut ffi::git_repository, root: &Path| {
            // SAFETY: a live repository, only read.
            let (workdir, bare) = unsafe {
                (
                    masked(ffi::git_repository_workdir(repo), root),
                    ffi::git_repository_is_bare(repo),
                )
            };
            (
                workdir,
                bare,
                read_masked(&root.join("repo/.git/config"), root),
                read_masked(&root.join("w1/.git"), root),
                read_masked(&root.join("w2/.git"), root),
            )
        };
        let steps: [(&str, bool); 7] = [
            ("w1", false),
            ("w2", true),
            ("w1", true),
            ("repo", true),
            ("missing", false),
            ("blocked", true),
            ("w2", false),
        ];
        for (dir, gitlink) in steps {
            let (c_wd, rust_wd) = (c_path(&c_root.join(dir)), c_path(&rust_root.join(dir)));
            // SAFETY: a live repository with a git directory and a NUL-terminated path.
            let c_rc =
                unsafe { ffi::git_repository_set_workdir(c.0, c_wd.as_ptr(), gitlink.into()) };
            let c_outcome = c_outcome(c_rc);
            let rust_outcome = rust_outcome(rust.as_mut().set_workdir(&rust_wd, gitlink));
            let mask = |o: Outcome, root: &Path| {
                (
                    o.0,
                    o.1.map(|(k, m)| (k, m.replace(root.to_str().unwrap(), "<root>"))),
                )
            };
            assert_eq!(
                mask(rust_outcome, &rust_root),
                mask(c_outcome, &c_root),
                "{dir}"
            );
            assert_eq!(
                snapshot(rust.as_c_ptr(), &rust_root),
                snapshot(c.0, &c_root),
                "{dir}"
            );
        }
        for _ in 0..2 {
            // SAFETY: a live repository.
            let c_rc = unsafe { ffi::git_repository_set_bare(c.0) };
            assert_eq!(rust_outcome(rust.as_mut().set_bare()), c_outcome(c_rc));
            assert_eq!(
                snapshot(rust.as_c_ptr(), &rust_root),
                snapshot(c.0, &c_root)
            );
        }
        // A pathless repository's read-only configuration refuses the change.
        let c_new = CRepo::new();
        let mut rust_new = Repository::new().unwrap();
        // SAFETY: a live repository and path; without the gitlink.
        let c_rc = unsafe { ffi::git_repository_set_workdir(c_new.0, c"/".as_ptr(), 0) };
        assert_eq!(
            rust_outcome(rust_new.as_mut().set_workdir(c"/", false)),
            c_outcome(c_rc)
        );
        // SAFETY: a live repository.
        let c_rc = unsafe { ffi::git_repository_set_bare(c_new.0) };
        assert_eq!(rust_outcome(rust_new.as_mut().set_bare()), c_outcome(c_rc));
    }

    /// The file system settings rewritten into a configuration that lost them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reinit_filesystem_matches_c() {
        let scratch = Scratch::new("reinit");
        let (c_dir, c, rust_dir, mut rust) = pair(&scratch, "testrepo.git");
        for dir in [&c_dir, &rust_dir] {
            fs::write(
                dir.join("config"),
                "[core]\n\trepositoryformatversion = 0\n",
            )
            .unwrap();
        }
        for recurse in [false, true] {
            // SAFETY: a live repository with a git directory.
            let c_rc = unsafe { ffi::git_repository_reinit_filesystem(c.0, recurse.into()) };
            assert_eq!(
                rust_outcome(rust.as_mut().reinit_filesystem(recurse)),
                c_outcome(c_rc)
            );
            assert_eq!(
                read_masked(&rust_dir.join("config"), &rust_dir),
                read_masked(&c_dir.join("config"), &c_dir)
            );
        }
    }

    /// A config, an index and an odb installed, then read back as the repository's.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn installed_parts_match_c() {
        let scratch = Scratch::new("parts");
        let (_, c, _, mut rust) = pair(&scratch, "testrepo.git");
        // The objects each side installs, built the same way.
        let c_config = Config::new().unwrap();
        let rust_config: ConfigArc = Config::new().unwrap().into();
        // SAFETY: live repository and config; C up-refs the config.
        let rc = unsafe { ffi::git_repository_set_config(c.0, c_config.as_c_ptr()) };
        assert_eq!(rc, 0);
        rust.as_mut().set_config(&rust_config);
        let repo_config = |repo: *mut ffi::git_repository| {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot and repository; the reference is released at once.
            assert_eq!(unsafe { ffi::git_repository_config(&mut out, repo) }, 0);
            // SAFETY: as above.
            unsafe { ffi::git_config_free(out) };
            out
        };
        assert_eq!(repo_config(c.0), c_config.as_c_ptr());
        assert_eq!(repo_config(rust.as_c_ptr()), rust_config.as_c_ptr());

        let index_of = |repo: *mut ffi::git_repository| {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot and repository.
            let rc = unsafe { ffi::git_repository_index(&mut out, repo) };
            assert_eq!(rc, 0, "{:?}", c_last());
            // SAFETY: the reference just returned, read and released once.
            let n = unsafe { ffi::git_index_entrycount(out) };
            // SAFETY: as above.
            unsafe { ffi::git_index_free(out) };
            (out, n)
        };
        let c_index: IndexArc = Index::new().unwrap().into();
        let rust_index: IndexArc = Index::new().unwrap().into();
        // SAFETY: live repository and index; C up-refs the index.
        let rc = unsafe { ffi::git_repository_set_index(c.0, c_index.as_c_ptr()) };
        assert_eq!(rc, 0);
        rust.as_mut().set_index(Some(&rust_index));
        assert_eq!(index_of(c.0), (c_index.as_c_ptr(), 0));
        assert_eq!(index_of(rust.as_c_ptr()), (rust_index.as_c_ptr(), 0));
        // Removed: the repository loads its index file again.
        // SAFETY: a live repository; NULL removes the index.
        let rc = unsafe { ffi::git_repository_set_index(c.0, ptr::null_mut()) };
        assert_eq!(rc, 0);
        rust.as_mut().set_index(None);
        let (c_loaded, c_entries) = index_of(c.0);
        let (rust_loaded, rust_entries) = index_of(rust.as_c_ptr());
        assert_ne!(c_loaded, c_index.as_c_ptr());
        assert_ne!(rust_loaded, rust_index.as_c_ptr());
        assert_eq!(rust_entries, c_entries);

        let c_odb = Odb::new().unwrap();
        let rust_odb = Odb::new().unwrap();
        let rust_raw = rust_odb.as_c_ptr();
        // SAFETY: live repository and odb; C up-refs the odb.
        let rc = unsafe { ffi::git_repository_set_odb(c.0, c_odb.as_c_ptr()) };
        assert_eq!(rc, 0);
        rust.as_mut().set_odb(rust_odb).unwrap();
        let mut out = ptr::null_mut();
        // SAFETY: a live slot and repository; the reference is released at once.
        assert_eq!(unsafe { ffi::git_repository_odb(&mut out, c.0) }, 0);
        // SAFETY: as above.
        unsafe { ffi::git_odb_free(out) };
        assert_eq!(out, c_odb.as_c_ptr());
        assert_eq!(rust.as_ref().odb().unwrap().as_c_ptr(), rust_raw);
    }

    /// `git_repository_open_ext` over starts, flag sets and ceilings, against
    /// [`Repository::open_ext`] and [`Repository::exists_ext`].
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_ext_matches_c() {
        init();
        let scratch = Scratch::new("open");
        let bare = copy(&scratch, "testrepo.git", "bare.git");
        fs::create_dir_all(bare.join("refs/deep/er")).unwrap();
        let rust_work = Repository::init(&c_path(&scratch.join("work")), false).unwrap();
        drop(rust_work);
        fs::create_dir_all(scratch.join("work/a/b")).unwrap();
        fs::create_dir_all(scratch.join("plain/c")).unwrap();
        // Repositories that fail to open once found.
        let broken = copy(&scratch, "testrepo.git", "broken-config.git");
        fs::write(broken.join("config"), "[core\n").unwrap();
        let grafted = copy(&scratch, "testrepo.git", "bad-grafts.git");
        fs::create_dir_all(grafted.join("info")).unwrap();
        fs::write(grafted.join("info/grafts"), "not an id\n").unwrap();
        let moved = copy(&scratch, "testrepo.git", "moved-worktree.git");
        fs::write(
            moved.join("config"),
            "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = /nonexistent/crustify\n",
        )
        .unwrap();
        let starts = [
            "bare.git",
            "bare.git/refs/deep/er",
            "work",
            "work/.git",
            "work/a/b",
            "plain/c",
            "missing",
            "broken-config.git",
            "bad-grafts.git",
            "moved-worktree.git",
        ];
        let ceilings = [
            None,
            Some(c_path(&scratch.0)),
            Some(c_path(&scratch.join("work/a"))),
        ];
        let flag_sets = [
            OpenFlags::empty(),
            OpenFlag::NoSearch.into(),
            OpenFlag::Bare.into(),
            OpenFlag::NoDotGit.into(),
            OpenFlag::CrossFs | OpenFlag::NoSearch,
            OpenFlag::Bare | OpenFlag::NoDotGit,
            OpenFlags::from_bits_retain(1 << 9),
        ];
        let describe = |repo: *mut ffi::git_repository| {
            // SAFETY: a live repository, only read; its strings outlive the copies.
            unsafe {
                (
                    masked(ffi::git_repository_path(repo), &scratch.0),
                    masked(ffi::git_repository_workdir(repo), &scratch.0),
                    ffi::git_repository_is_bare(repo),
                )
            }
        };
        let mask = |o: Outcome| {
            (
                o.0,
                o.1.map(|(k, m)| (k, m.replace(scratch.0.to_str().unwrap(), "<root>"))),
            )
        };
        for start in starts {
            let start = c_path(&scratch.join(start));
            for ceiling in &ceilings {
                let ceiling = ceiling.as_deref();
                let c_ceiling = ceiling.map_or(ptr::null(), CStr::as_ptr);
                for flags in flag_sets {
                    let mut out = ptr::null_mut();
                    // SAFETY: a live slot, a NUL-terminated start and ceiling or NULL.
                    let rc = unsafe {
                        ffi::git_repository_open_ext(
                            &mut out,
                            start.as_ptr(),
                            flags.bits(),
                            c_ceiling,
                        )
                    };
                    let c_side = if rc < 0 {
                        Err(mask(c_outcome(rc)))
                    } else {
                        Ok(describe(out))
                    };
                    // SAFETY: NULL or the repository just opened, freed once.
                    unsafe { ffi::git_repository_free(out) };
                    let rust_side = match Repository::open_ext(Some(&start), flags, ceiling) {
                        Ok(repo) => Ok(describe(repo.as_c_ptr())),
                        Err(e) => Err(mask(rust_outcome(Err(e)))),
                    };
                    assert_eq!(rust_side, c_side, "{start:?} {flags:?} {ceiling:?}");

                    // SAFETY: a NULL out slot: only the search runs.
                    let rc = unsafe {
                        ffi::git_repository_open_ext(
                            ptr::null_mut(),
                            start.as_ptr(),
                            flags.bits(),
                            c_ceiling,
                        )
                    };
                    let exists = Repository::exists_ext(Some(&start), flags, ceiling);
                    match rc {
                        0 => assert!(exists.unwrap()),
                        ffi::GIT_ENOTFOUND => assert!(!exists.unwrap()),
                        rc => assert_eq!(exists.map_err(|e| e.raw_code()), Err(rc)),
                    }
                }
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_from_worktree_matches_c() {
        init();
        let scratch = Scratch::new("worktree");
        let main = Repository::init(&c_path(&scratch.join("main")), false).unwrap();
        let admin = scratch.join("main/.git/worktrees/wt");
        fs::create_dir_all(&admin).unwrap();
        fs::create_dir_all(scratch.join("wt")).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
        let link = scratch.join("wt/.git");
        fs::write(admin.join("gitdir"), format!("{}\n", link.display())).unwrap();
        fs::write(&link, format!("gitdir: {}\n", admin.display())).unwrap();
        // Each side looks the worktree up on its own.
        let lookup = || {
            let mut wt = ptr::null_mut();
            // SAFETY: a live slot, repository and name.
            let rc = unsafe { ffi::git_worktree_lookup(&mut wt, main.as_c_ptr(), c"wt".as_ptr()) };
            assert_eq!(rc, 0);
            // SAFETY: the fresh worktree, owned here.
            unsafe { WorktreeBox::from_c(wt) }.unwrap()
        };
        let describe = |repo: *mut ffi::git_repository| {
            // SAFETY: a live repository, only read.
            unsafe {
                (
                    masked(ffi::git_repository_path(repo), &scratch.0),
                    masked(ffi::git_repository_workdir(repo), &scratch.0),
                    ffi::git_repository_is_bare(repo),
                )
            }
        };
        let open_both = || {
            let (c_wt, rust_wt) = (lookup(), lookup());
            let mut out = ptr::null_mut();
            // SAFETY: a live slot and worktree.
            let rc = unsafe { ffi::git_repository_open_from_worktree(&mut out, c_wt.as_c_ptr()) };
            let c_side = if rc < 0 {
                Err(c_outcome(rc))
            } else {
                Ok(describe(out))
            };
            // SAFETY: NULL or the fresh repository, freed once.
            unsafe { ffi::git_repository_free(out) };
            let rust_side = match Repository::open_from_worktree(rust_wt.as_ref()) {
                Ok(repo) => Ok(describe(repo.as_c_ptr())),
                Err(e) => Err(rust_outcome(Err(e))),
            };
            (rust_side, c_side)
        };
        let (rust_side, c_side) = open_both();
        assert!(c_side.is_ok());
        assert_eq!(rust_side, c_side);
        // The gitlink points nowhere: the open fails.
        fs::write(&link, "gitdir: /nonexistent/crustify\n").unwrap();
        let (rust_side, c_side) = open_both();
        assert!(c_side.is_err());
        assert_eq!(rust_side, c_side);
        // The recorded gitlink does not end in `.git`: libgit2 fails without an error,
        // which the wrapper supplies; only the code is compared.
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", scratch.join("wt/link").display()),
        )
        .unwrap();
        let (rust_side, c_side) = open_both();
        let code = |side: Result<_, Outcome>| side.map_err(|(rc, _)| rc);
        let c_code = code(c_side);
        assert_eq!(c_code, Err(-1));
        assert_eq!(code(rust_side), c_code);
    }

    /// The failures of `HEAD` moves: an invalid name, a branch checked out in a linked
    /// worktree, then a repository whose `HEAD` is gone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn head_move_failures_match_c() {
        isolated(
            concat!(module_path!(), "::head_move_failures_match_c"),
            head_move_failures_match_c_alone,
        );
    }

    fn head_move_failures_match_c_alone() {
        let scratch = Scratch::new("head-failures");
        let (c_dir, c, rust_dir, rust) = pair(&scratch, "testrepo.git");
        for (dir, name) in [(&c_dir, "c-wt"), (&rust_dir, "rust-wt")] {
            // A linked worktree with `br2` checked out.
            let admin = dir.join("worktrees/wt");
            let wt = scratch.join(name);
            fs::create_dir_all(&admin).unwrap();
            fs::create_dir_all(&wt).unwrap();
            fs::write(admin.join("commondir"), "../..\n").unwrap();
            fs::write(admin.join("HEAD"), "ref: refs/heads/br2\n").unwrap();
            fs::write(
                admin.join("gitdir"),
                format!("{}\n", wt.join(".git").display()),
            )
            .unwrap();
            fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        }
        let mask = |o: Outcome| {
            let root = scratch.0.to_str().unwrap();
            (o.0, o.1.map(|(k, m)| (k, m.replace(root, "<root>"))))
        };
        let id = |hex: &CStr| Oid::from_string(hex, OidType::Sha1).unwrap();
        let c_id = |hex: &CStr| {
            let mut out = ffi::git_oid::default();
            // SAFETY: a writable id and a NUL-terminated hex string.
            let rc = unsafe { ffi::git_oid_from_string(&mut out, hex.as_ptr(), ffi::GIT_OID_SHA1) };
            assert_eq!(rc, 0);
            out
        };
        let commit = c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
        for step in 0..2 {
            if step == 1 {
                fs::remove_file(c_dir.join("HEAD")).unwrap();
                fs::remove_file(rust_dir.join("HEAD")).unwrap();
            }
            for name in [c"refs/heads/br2", c"refs/heads/a..b", c"not a ref"] {
                // SAFETY: a live repository and a NUL-terminated name.
                let c_rc = unsafe { ffi::git_repository_set_head(c.0, name.as_ptr()) };
                let c_side = mask(c_outcome(c_rc));
                let rust_side = mask(rust_outcome(rust.as_ref().set_head(name)));
                assert_eq!(rust_side, c_side, "{step} {name:?}");
            }
            // SAFETY: a live repository and id of its type.
            let c_rc = unsafe { ffi::git_repository_set_head_detached(c.0, &c_id(commit)) };
            let rust_side = rust_outcome(rust.as_ref().set_head_detached(id(commit).as_ref()));
            assert_eq!(mask(rust_side), mask(c_outcome(c_rc)), "{step}");
        }
    }

    /// The configuration a repository opened as bare loads only on first use: when it
    /// cannot be parsed then, the setters that write it fail.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn deferred_configuration_failures_match_c() {
        let scratch = Scratch::new("deferred-config");
        let c_dir = copy(&scratch, "testrepo.git", "c");
        let rust_dir = copy(&scratch, "testrepo.git", "rust");
        fs::create_dir_all(scratch.join("w")).unwrap();
        let w = c_path(&scratch.join("w"));
        let bare = OpenFlags::from(OpenFlag::Bare);
        let mut c = ptr::null_mut();
        // SAFETY: a live slot and a NUL-terminated path.
        let rc = unsafe {
            ffi::git_repository_open_ext(&mut c, c_path(&c_dir).as_ptr(), bare.bits(), ptr::null())
        };
        assert_eq!(rc, 0);
        let c = CRepo(c);
        let mut rust = Repository::open_ext(Some(&c_path(&rust_dir)), bare, None).unwrap();
        for dir in [&c_dir, &rust_dir] {
            fs::write(dir.join("config"), "[core\n\tbare = false\n").unwrap();
        }
        let mask = |o: Outcome| {
            let root = scratch.0.to_str().unwrap();
            (o.0, o.1.map(|(k, m)| (k, m.replace(root, "<root>"))))
        };
        // SAFETY: a live repository and path; the gitlink needs the configuration.
        let c_rc = unsafe { ffi::git_repository_set_workdir(c.0, w.as_ptr(), 1) };
        assert!(c_rc < 0);
        let rust_side = rust_outcome(rust.as_mut().set_workdir(&w, true));
        assert_eq!(mask(rust_side), mask(c_outcome(c_rc)));
        // Not bare any more without the gitlink; making it bare needs the configuration.
        // SAFETY: as above, without the gitlink.
        let c_rc = unsafe { ffi::git_repository_set_workdir(c.0, w.as_ptr(), 0) };
        assert_eq!(
            rust_outcome(rust.as_mut().set_workdir(&w, false)),
            c_outcome(c_rc)
        );
        // SAFETY: a live repository.
        let c_rc = unsafe { ffi::git_repository_set_bare(c.0) };
        assert!(c_rc < 0);
        assert_eq!(
            mask(rust_outcome(rust.as_mut().set_bare())),
            mask(c_outcome(c_rc))
        );
    }

    /// A non-bare repository's file system settings, rewritten recursively.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn recursive_reinit_matches_c() {
        init();
        let scratch = Scratch::new("reinit-recurse");
        let roots = [scratch.join("c"), scratch.join("rust")];
        for root in &roots {
            drop(Repository::init(&c_path(root), false).unwrap());
            fs::write(
                root.join(".git/config"),
                "[core]\n\trepositoryformatversion = 0\n",
            )
            .unwrap();
        }
        let c = CRepo::open(&roots[0]);
        let mut rust = Repository::open(&c_path(&roots[1])).unwrap();
        // SAFETY: a live repository with a git directory.
        let c_rc = unsafe { ffi::git_repository_reinit_filesystem(c.0, 1) };
        assert_eq!(
            rust_outcome(rust.as_mut().reinit_filesystem(true)),
            c_outcome(c_rc)
        );
        assert_eq!(
            read_masked(&roots[1].join(".git/config"), &roots[1]),
            read_masked(&roots[0].join(".git/config"), &roots[0])
        );
    }

    /// A repository whose `objects` directory is gone cannot load its odb. libgit2 then
    /// leaks the half-built odb (owner already set) and its path, on both sides, so the
    /// child runs with LeakSanitizer off.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn odb_load_failures_match_c() {
        isolated_with(
            concat!(module_path!(), "::odb_load_failures_match_c"),
            &[],
            true,
            || {
                let scratch = Scratch::new("odb-failure");
                let (c_dir, c, rust_dir, rust) = pair(&scratch, "testrepo.git");
                fs::remove_dir_all(c_dir.join("objects")).unwrap();
                fs::remove_dir_all(rust_dir.join("objects")).unwrap();
                let mut odb = ptr::null_mut();
                // SAFETY: a live slot and repository.
                let rc = unsafe { ffi::git_repository_odb(&mut odb, c.0) };
                assert_eq!((rc, odb.is_null()), (-1, true));
                let mask = |o: Outcome| {
                    let root = scratch.0.to_str().unwrap();
                    (
                        o.0,
                        o.1.map(|(k, m)| {
                            (
                                k,
                                m.replace(root, "<root>")
                                    .replace("/c/", "/x/")
                                    .replace("/rust/", "/x/"),
                            )
                        }),
                    )
                };
                let rust_side = mask(rust_outcome(rust.as_ref().odb().map(drop)));
                assert_eq!(rust_side, mask(c_outcome(rc)));
            },
        );
    }

    /// `FROM_ENV` opens: `$GIT_DIR` for a missing start, and the odb paths the
    /// environment names; then an unparsable `$GIT_DISCOVERY_ACROSS_FILESYSTEM`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_env_opens_match_c() {
        let scratch = Scratch::new("from-env");
        let repo = copy(&scratch, "testrepo.git", "repo");
        let alternate = scratch.join("alternate");
        fs::create_dir_all(&alternate).unwrap();
        let objects = repo.join("objects");
        let alternates = scratch.join("alternates-list");
        fs::write(
            &alternates,
            format!("{}:{}", alternate.display(), objects.display()),
        )
        .unwrap();
        let alternates = fs::read_to_string(&alternates).unwrap();
        let vars: [(&str, &Path); 3] = [
            ("GIT_DIR", &repo),
            ("GIT_OBJECT_DIRECTORY", &objects),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", Path::new(&alternates)),
        ];
        isolated_with(
            concat!(module_path!(), "::from_env_opens_match_c"),
            &vars,
            false,
            || {
                init();
                let flags = OpenFlags::from(OpenFlag::FromEnv);
                let mut c = ptr::null_mut();
                // SAFETY: a live slot; NULL start and ceiling under `FROM_ENV`.
                let rc = unsafe {
                    ffi::git_repository_open_ext(&mut c, ptr::null(), flags.bits(), ptr::null())
                };
                assert_eq!(rc, 0, "{:?}", c_last());
                let c = CRepo(c);
                let rust = Repository::open_ext(None, flags, None).unwrap();
                // SAFETY: live repositories, only read.
                let (c_path, rust_path) = unsafe {
                    (
                        CStr::from_ptr(ffi::git_repository_path(c.0)).to_owned(),
                        CStr::from_ptr(ffi::git_repository_path(rust.as_c_ptr())).to_owned(),
                    )
                };
                assert_eq!(rust.as_ref().path(), Some(&*c_path));
                assert_eq!(rust_path, c_path);
                let mut odb = ptr::null_mut();
                // SAFETY: a live slot and repository; the reference is released below.
                assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, c.0) }, 0);
                // SAFETY: the live reference, read and released once.
                let c_backends = unsafe { ffi::git_odb_num_backends(odb) };
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
                let rust_odb = rust.as_ref().odb().unwrap();
                // SAFETY: the arc's live odb, only read.
                let rust_backends = unsafe { ffi::git_odb_num_backends(rust_odb.as_c_ptr()) };
                assert_eq!(rust_backends, c_backends);
                assert!(c_backends > 2, "{c_backends}");
                // SAFETY: a NULL out slot: only the search.
                let rc = unsafe {
                    ffi::git_repository_open_ext(
                        ptr::null_mut(),
                        ptr::null(),
                        flags.bits(),
                        ptr::null(),
                    )
                };
                assert_eq!(
                    (rc, Repository::exists_ext(None, flags, None).unwrap()),
                    (0, true)
                );
            },
        );
        let bad: [(&str, &Path); 1] = [("GIT_DISCOVERY_ACROSS_FILESYSTEM", Path::new("maybe"))];
        isolated_with(
            concat!(module_path!(), "::from_env_opens_match_c_unparsable"),
            &bad,
            false,
            from_env_opens_match_c_unparsable,
        );
    }

    /// The child of [`from_env_opens_match_c`] with an unparsable boolean.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_env_opens_match_c_unparsable() {
        if std::env::var(CHILD).is_err() {
            // Run through its parent, which sets the environment.
            return;
        }
        init();
        let flags = OpenFlag::FromEnv | OpenFlag::NoSearch;
        let start = c_path(&resource("testrepo.git"));
        let mut out = ptr::null_mut();
        // SAFETY: a live slot and NUL-terminated start.
        let rc = unsafe {
            ffi::git_repository_open_ext(&mut out, start.as_ptr(), flags.bits(), ptr::null())
        };
        let c_side = c_outcome(rc);
        assert!(out.is_null());
        let rust_side = rust_outcome(Repository::open_ext(Some(&start), flags, None).map(drop));
        assert_eq!(rust_side, c_side);
        // SAFETY: a NULL out slot: only the search.
        let rc = unsafe {
            ffi::git_repository_open_ext(ptr::null_mut(), start.as_ptr(), flags.bits(), ptr::null())
        };
        let exists = Repository::exists_ext(Some(&start), flags, None).map_err(|e| e.raw_code());
        assert_eq!(exists, Err(rc));
        assert_eq!(rc, -1);
    }

    /// What a C callback records: the ids it saw, and the call to stop at.
    struct Recorder {
        seen: Vec<String>,
        stop_at: usize,
        code: c_int,
    }

    unsafe extern "C" fn record(id: *const ffi::git_oid, payload: *mut c_void) -> c_int {
        // SAFETY: the test passes a live `Recorder`; libgit2 a live id. The hex form
        // is written into a local buffer.
        let recorder = unsafe { &mut *payload.cast::<Recorder>() };
        let mut hex = [0u8; 65];
        // SAFETY: 65 writable bytes and a live id.
        unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), id) };
        let hex = CStr::from_bytes_until_nul(&hex).unwrap();
        recorder.seen.push(hex.to_str().unwrap().to_owned());
        if recorder.seen.len() == recorder.stop_at {
            recorder.code
        } else {
            0
        }
    }

    /// The ids of a `MERGE_HEAD` walked by libgit2 with a C callback, against the
    /// same file's ids handed to a [`MergeheadCallback`] one by one until it breaks:
    /// the ids seen and the value that ends the walk must agree.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mergehead_callbacks_match_c() {
        init();
        let scratch = Scratch::new("mergehead");
        let repo = Repository::init(&c_path(&scratch.join("repo")), true).unwrap();
        let ids = [
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
            "c47800c7266a2be04c571c04d5a6614691ea99bd",
        ];
        let file: String = ids.iter().map(|id| format!("{id}\n")).collect();
        fs::write(scratch.join("repo/MERGE_HEAD"), &file).unwrap();
        for (stop_at, code) in [(usize::MAX, 0), (1, 1), (2, -7), (3, i32::MIN)] {
            let mut recorder = Recorder {
                seen: Vec::new(),
                stop_at,
                code,
            };
            // SAFETY: a live repository; the callback and payload are used for the
            // call only.
            let c_rc = unsafe {
                ffi::git_repository_mergehead_foreach(
                    repo.as_c_ptr(),
                    Some(record),
                    ptr::from_mut(&mut recorder).cast(),
                )
            };
            let mut seen = Vec::new();
            let mut f = |id: OidRef<'_>| {
                seen.push(id.to_hex_string().unwrap());
                match std::num::NonZero::new(code) {
                    Some(code) if seen.len() == stop_at => ControlFlow::Break(code),
                    _ => ControlFlow::Continue(()),
                }
            };
            let mut callback = MergeheadCallback::new(&mut f);
            let mut rust_rc = 0;
            for line in file.lines() {
                let id = Oid::from_string(&CString::new(line).unwrap(), OidType::Sha1).unwrap();
                if let ControlFlow::Break(code) = callback.call(id.as_ref()) {
                    rust_rc = code.get();
                    break;
                }
            }
            drop(callback);
            assert_eq!((rust_rc, seen), (c_rc, recorder.seen), "stop at {stop_at}");
        }
    }
}

/// The routines of `git2/repository.h` and `git2/sys/repository.h` on
/// [`RepositoryRef`] / [`RepositoryMut`], [`discover`] and the `FETCH_HEAD` callback,
/// against the C routines they wrap.
///
/// Each side copies the same fixtures from `tests/resources` into a scratch directory
/// of its own and runs the same steps on its own repositories: the C side through the
/// raw routines, the Rust side through the safe API, with only the setup (the
/// namespace and ident setters, files written) and the reading of a returned object
/// (a config value, a reference's name) through raw calls on both. The outcome of
/// every step, its value or its failure (code, class, message, with the root masked),
/// must agree.
///
/// `git_repository_commit_parents` is run only where it is defined: the steps never
/// reach the failures after which it frees cached commits, nor a `HEAD` naming
/// anything but a commit, nor a missing object (where it leaks); and `git_repository_head_detached_for_worktree` only where
/// its answer is right: on a detached `HEAD`, or a failure. The unit tests cover those
/// corrections.
mod head_and_lookups {
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
    use std::fs;
    use std::ops::ControlFlow;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;
    use std::sync::LazyLock;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::repository::repository::{
        FetchheadEntry, FetchheadForeachCallback, Item, Repository, RepositoryBox, RepositoryError,
        discover,
    };

    const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/resources");

    /// Copies the tree at `from` to `to`, symlinks as symlinks.
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), &target).unwrap();
            } else if kind.is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// A side's scratch directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-routines-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// `tests/resources/<name>` copied to `<root>/<to>`, its `.gitted` renamed
        /// `.git` as libgit2's own test sandboxes do.
        fn sandbox(&self, name: &str, to: &str) {
            let to = self.0.join(to);
            copy_tree(&Path::new(RESOURCES).join(name), &to);
            if to.join(".gitted").exists() {
                fs::rename(to.join(".gitted"), to.join(".git")).unwrap();
            }
        }

        /// The fixtures: `bare` (`testrepo.git`), `empty` (`empty_bare.git`, an unborn
        /// `HEAD`), `shallow` (`shallow.git`), `testrepo` with its linked worktree
        /// `testrepo-worktree`, and `merged` (`mergedrepo`, mid-merge).
        fn with_fixtures(tag: &str) -> Self {
            let this = Self::new(tag);
            this.sandbox("testrepo.git", "bare");
            this.sandbox("empty_bare.git", "empty");
            this.sandbox("shallow.git", "shallow");
            this.sandbox("testrepo", "testrepo");
            this.sandbox("testrepo-worktree", "testrepo-worktree");
            this.sandbox("mergedrepo", "merged");
            this
        }

        /// `rel` under the root; a path starting with `@` is passed as is, without it.
        fn c_path(&self, rel: &str) -> CString {
            match rel.strip_prefix('@') {
                Some(raw) => CString::new(raw).unwrap(),
                None => CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap(),
            }
        }

        /// `s` with the scratch directory replaced by `<root>`.
        fn relative(&self, s: &str) -> String {
            s.replace(&*self.0.to_string_lossy(), "<root>")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// One step on a side's current repository.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Opens the repository at a root-relative path.
        Open(&'static str),
        /// A repository without paths.
        New,
        /// Writes a root-relative file.
        Write(&'static str, &'static str),
        SetNamespace(Option<&'static CStr>),
        SetIdent(Option<&'static CStr>, Option<&'static CStr>),
        Commondir,
        IsBare,
        IsWorktree,
        Namespace,
        Ident,
        IsShallow,
        ItemPath(Item),
        /// Reads an `int32` from the config.
        Config(&'static CStr),
        /// Reads an `int32` from a snapshot of the config.
        Snapshot(&'static CStr),
        /// The index's path and entry count.
        Index,
        /// Hashes a file (see [`Scratch::c_path`]).
        HashFile(&'static str, ObjectType, Option<&'static CStr>),
        Head,
        HeadDetached,
        HeadUnborn,
        IsEmpty,
        HeadForWorktree(&'static CStr),
        HeadDetachedForWorktree(&'static CStr),
        DetachHead,
        CommitParents,
        Cleanup,
    }

    /// One step's outcome.
    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Value(String),
        Failed {
            code: c_int,
            class: c_int,
            message: String,
        },
    }

    /// The calling thread's last error, as C reads it.
    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; the result is a static error or this
        // thread's, live until the next libgit2 call on this thread, after the copy.
        unsafe {
            let e = ffi::git_error_last();
            let message = (*e).message;
            let message = if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            ((*e).klass, message)
        }
    }

    /// Initialises libgit2 for the C side; never released.
    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// A NULL or live C string, copied with the root masked.
    ///
    /// # Safety
    ///
    /// `s` must be NULL or a live C string.
    unsafe fn text(scratch: &Scratch, s: *const c_char) -> String {
        if s.is_null() {
            return "(null)".into();
        }
        // SAFETY: per this function's contract.
        scratch.relative(&unsafe { CStr::from_ptr(s) }.to_string_lossy())
    }

    /// An id's hex form.
    ///
    /// # Safety
    ///
    /// `id` must be NULL or a live id.
    unsafe fn hex(id: *const ffi::git_oid) -> String {
        if id.is_null() {
            return "(none)".into();
        }
        let mut out = [0u8; 65];
        // SAFETY: a live id and 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    /// A reference's name, target and peeled id.
    ///
    /// # Safety
    ///
    /// `r` must be a live direct reference; only its fields are read.
    unsafe fn describe(r: *const ffi::git_reference) -> String {
        // SAFETY: per this function's contract.
        unsafe {
            format!(
                "{} -> {} (peel {}, type {})",
                CStr::from_ptr(ffi::git_reference_name(r)).to_string_lossy(),
                hex(ffi::git_reference_target(r)),
                hex(ffi::git_reference_target_peel(r)),
                ffi::git_reference_type(r),
            )
        }
    }

    /// An `int32` read from a live config, or its failure.
    fn read_int32(scratch: &Scratch, config: *mut ffi::git_config, key: &CStr) -> Outcome {
        let mut value = 0;
        // SAFETY: a live config, only read, and a NUL-terminated key.
        let rc = unsafe { ffi::git_config_get_int32(&mut value, config, key.as_ptr()) };
        if rc < 0 {
            c_failed(scratch, rc)
        } else {
            Outcome::Value(value.to_string())
        }
    }

    /// A live index's path and entry count.
    fn read_index(scratch: &Scratch, index: *mut ffi::git_index) -> Outcome {
        // SAFETY: a live index, only read; its path lives as long as it does.
        unsafe {
            Outcome::Value(format!(
                "{} {}",
                text(scratch, ffi::git_index_path(index)),
                ffi::git_index_entrycount(index)
            ))
        }
    }

    /// The commits of a live array, by id.
    ///
    /// # Safety
    ///
    /// `commits` must address `count` live commits.
    unsafe fn commit_ids(commits: *const *mut ffi::git_commit, count: usize) -> String {
        let ids: Vec<String> = (0..count)
            // SAFETY: per this function's contract.
            .map(|i| unsafe { hex(ffi::git_commit_id(*commits.add(i))) })
            .collect();
        format!("[{}]", ids.join(", "))
    }

    fn c_failed(scratch: &Scratch, rc: c_int) -> Outcome {
        let (class, message) = c_last();
        Outcome::Failed {
            code: rc,
            class,
            message: scratch.relative(&message),
        }
    }

    fn rust_failed(scratch: &Scratch, error: &RepositoryError) -> Outcome {
        Outcome::Failed {
            code: error.raw_code(),
            class: error.raw_class(),
            message: scratch.relative(&error.message().to_string_lossy()),
        }
    }

    /// A C routine's `int` answer: the value, or the failure for a negative one.
    fn c_answer(scratch: &Scratch, rc: c_int) -> Outcome {
        if rc < 0 {
            c_failed(scratch, rc)
        } else {
            Outcome::Value(rc.to_string())
        }
    }

    /// A Rust routine's answer, as C's `int`.
    fn rust_answer<T: Into<c_int>>(
        scratch: &Scratch,
        result: Result<T, RepositoryError>,
    ) -> Outcome {
        match result {
            Ok(value) => Outcome::Value(value.into().to_string()),
            Err(error) => rust_failed(scratch, &error),
        }
    }

    /// Steps the sides share: opening, and the raw setup.
    fn setup(scratch: &Scratch, repo: *mut ffi::git_repository, step: Step) -> Option<Outcome> {
        // SAFETY: `repo` is the side's live repository; the setters copy their strings.
        let rc = unsafe {
            match step {
                Step::Write(rel, contents) => {
                    let path = scratch.0.join(rel);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, contents).unwrap();
                    0
                }
                Step::SetNamespace(namespace) => ffi::git_repository_set_namespace(
                    repo,
                    namespace.map_or(ptr::null(), CStr::as_ptr),
                ),
                Step::SetIdent(name, email) => ffi::git_repository_set_ident(
                    repo,
                    name.map_or(ptr::null(), CStr::as_ptr),
                    email.map_or(ptr::null(), CStr::as_ptr),
                ),
                _ => return None,
            }
        };
        Some(c_answer(scratch, rc))
    }

    /// The C side's repository: freed on drop.
    struct CRepo(*mut ffi::git_repository);

    impl Drop for CRepo {
        fn drop(&mut self) {
            // SAFETY: NULL or the side's own repository, freed once.
            unsafe { ffi::git_repository_free(self.0) };
        }
    }

    /// One step through libgit2's C API.
    fn c_step(scratch: &Scratch, repo: &mut CRepo, step: Step) -> Outcome {
        let r = repo.0;
        match step {
            Step::Open(rel) => {
                let path = scratch.c_path(rel);
                *repo = CRepo(ptr::null_mut());
                // SAFETY: a writable slot and a NUL-terminated path.
                let rc = unsafe { ffi::git_repository_open(&mut repo.0, path.as_ptr()) };
                c_answer(scratch, rc)
            }
            Step::New => {
                *repo = CRepo(ptr::null_mut());
                // SAFETY: a writable slot.
                c_answer(scratch, unsafe { ffi::git_repository_new(&mut repo.0) })
            }
            Step::Write(..) | Step::SetNamespace(_) | Step::SetIdent(..) => {
                setup(scratch, r, step).unwrap()
            }
            Step::Commondir => {
                // SAFETY: a live repository, only read; the string lives as long as it does.
                Outcome::Value(unsafe { text(scratch, ffi::git_repository_commondir(r)) })
            }
            // SAFETY: as above.
            Step::IsBare => c_answer(scratch, unsafe { ffi::git_repository_is_bare(r) }),
            // SAFETY: as above.
            Step::IsWorktree => c_answer(scratch, unsafe { ffi::git_repository_is_worktree(r) }),
            Step::Namespace => {
                // SAFETY: as above.
                Outcome::Value(unsafe { text(scratch, ffi::git_repository_get_namespace(r)) })
            }
            Step::Ident => {
                let (mut name, mut email) = (ptr::null(), ptr::null());
                // SAFETY: as above, with two writable slots.
                let rc = unsafe { ffi::git_repository_ident(&mut name, &mut email, r) };
                assert_eq!(rc, 0);
                // SAFETY: both NULL or the repository's own strings.
                Outcome::Value(unsafe {
                    format!("{} <{}>", text(scratch, name), text(scratch, email))
                })
            }
            // SAFETY: as above.
            Step::IsShallow => c_answer(scratch, unsafe { ffi::git_repository_is_shallow(r) }),
            Step::ItemPath(item) => {
                let mut buf = ffi::git_buf::default();
                // SAFETY: an empty buffer, a live repository and a named item.
                let rc = unsafe { ffi::git_repository_item_path(&mut buf, r, item.raw()) };
                let outcome = if rc < 0 {
                    c_failed(scratch, rc)
                } else {
                    // SAFETY: what libgit2 left in the buffer, read before it is disposed.
                    Outcome::Value(unsafe { text(scratch, buf.ptr) })
                };
                // SAFETY: as above.
                unsafe { ffi::git_buf_dispose(&mut buf) };
                outcome
            }
            Step::Config(key) | Step::Snapshot(key) => {
                let mut config = ptr::null_mut();
                // SAFETY: a writable slot and a live repository.
                let rc = unsafe {
                    if matches!(step, Step::Config(_)) {
                        ffi::git_repository_config(&mut config, r)
                    } else {
                        ffi::git_repository_config_snapshot(&mut config, r)
                    }
                };
                if rc < 0 {
                    return c_failed(scratch, rc);
                }
                let outcome = read_int32(scratch, config, key);
                // SAFETY: the reference the call handed out, released once.
                unsafe { ffi::git_config_free(config) };
                outcome
            }
            Step::Index => {
                let mut index = ptr::null_mut();
                // SAFETY: a writable slot and a live repository.
                let rc = unsafe { ffi::git_repository_index(&mut index, r) };
                if rc < 0 {
                    return c_failed(scratch, rc);
                }
                let outcome = read_index(scratch, index);
                // SAFETY: the reference the call handed out, released once.
                unsafe { ffi::git_index_free(index) };
                outcome
            }
            Step::HashFile(path, kind, as_path) => {
                let path = scratch.c_path(path);
                let mut id = ffi::git_oid::default();
                // SAFETY: a writable id, a live repository, NUL-terminated strings.
                let rc = unsafe {
                    ffi::git_repository_hashfile(
                        &mut id,
                        r,
                        path.as_ptr(),
                        kind.raw(),
                        as_path.map_or(ptr::null(), CStr::as_ptr),
                    )
                };
                if rc < 0 {
                    c_failed(scratch, rc)
                } else {
                    // SAFETY: the id the call wrote.
                    Outcome::Value(unsafe { hex(&id) })
                }
            }
            Step::Head | Step::HeadForWorktree(_) => {
                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, a live repository and a NUL-terminated name.
                let rc = unsafe {
                    match step {
                        Step::HeadForWorktree(name) => {
                            ffi::git_repository_head_for_worktree(&mut out, r, name.as_ptr())
                        }
                        _ => ffi::git_repository_head(&mut out, r),
                    }
                };
                if rc < 0 {
                    return c_failed(scratch, rc);
                }
                // SAFETY: the fresh reference the call handed out, read and freed once.
                unsafe {
                    let described = describe(out);
                    ffi::git_reference_free(out);
                    Outcome::Value(described)
                }
            }
            Step::HeadDetached => {
                // SAFETY: a live repository.
                c_answer(scratch, unsafe { ffi::git_repository_head_detached(r) })
            }
            // SAFETY: as above.
            Step::HeadUnborn => c_answer(scratch, unsafe { ffi::git_repository_head_unborn(r) }),
            // SAFETY: as above.
            Step::IsEmpty => c_answer(scratch, unsafe { ffi::git_repository_is_empty(r) }),
            Step::HeadDetachedForWorktree(name) => {
                // SAFETY: as above, with a NUL-terminated name.
                c_answer(scratch, unsafe {
                    ffi::git_repository_head_detached_for_worktree(r, name.as_ptr())
                })
            }
            Step::DetachHead => {
                // SAFETY: as above.
                let rc = unsafe { ffi::git_repository_detach_head(r) };
                if rc < 0 {
                    c_failed(scratch, rc)
                } else {
                    Outcome::Value(head_file(scratch, r))
                }
            }
            Step::CommitParents => {
                let mut array = ffi::git_commitarray::default();
                // SAFETY: an empty array and a live repository whose HEAD names a commit
                // or nothing, and whose MERGE_HEAD (if any) names commits only.
                let rc = unsafe { ffi::git_repository_commit_parents(&mut array, r) };
                if rc < 0 {
                    return c_failed(scratch, rc);
                }
                // SAFETY: the array the call filled, read and disposed once.
                unsafe {
                    let ids = commit_ids(array.commits, array.count);
                    ffi::git_commitarray_dispose(&mut array);
                    Outcome::Value(ids)
                }
            }
            Step::Cleanup => {
                // SAFETY: a live repository nothing else uses.
                c_answer(scratch, unsafe { ffi::git_repository__cleanup(r) })
            }
        }
    }

    /// The contents of the repository's `HEAD` file.
    fn head_file(scratch: &Scratch, repo: *mut ffi::git_repository) -> String {
        // SAFETY: a live repository, only read; its path lives as long as it does.
        let gitdir = unsafe { CStr::from_ptr(ffi::git_repository_path(repo)) };
        let head = fs::read_to_string(Path::new(&*gitdir.to_string_lossy()).join("HEAD")).unwrap();
        scratch.relative(&head)
    }

    /// One step through the Rust API.
    fn rust_step(scratch: &Scratch, repo: &mut Option<RepositoryBox>, step: Step) -> Outcome {
        let opened = |result: Result<RepositoryBox, RepositoryError>,
                      repo: &mut Option<RepositoryBox>| match result {
            Ok(r) => {
                *repo = Some(r);
                Outcome::Value("0".into())
            }
            Err(error) => rust_failed(scratch, &error),
        };
        match step {
            Step::Open(rel) => {
                *repo = None;
                return opened(Repository::open(&scratch.c_path(rel)), repo);
            }
            Step::New => {
                *repo = None;
                return opened(Repository::new(), repo);
            }
            Step::Cleanup => {
                repo.as_mut().unwrap().as_mut().cleanup();
                return Outcome::Value("0".into());
            }
            _ => {}
        }
        let owner = repo.as_ref().unwrap();
        if let Some(outcome) = setup(scratch, owner.as_c_ptr(), step) {
            return outcome;
        }
        let r = owner.as_ref();
        let text = |s: Option<&CStr>| {
            s.map_or("(null)".into(), |s| scratch.relative(&s.to_string_lossy()))
        };
        match step {
            Step::Commondir => Outcome::Value(text(r.commondir())),
            Step::IsBare => Outcome::Value(c_int::from(r.is_bare()).to_string()),
            Step::IsWorktree => Outcome::Value(c_int::from(r.is_worktree()).to_string()),
            Step::Namespace => Outcome::Value(text(r.namespace())),
            Step::Ident => {
                let (name, email) = r.ident();
                Outcome::Value(format!("{} <{}>", text(name), text(email)))
            }
            Step::IsShallow => rust_answer(scratch, r.is_shallow()),
            Step::ItemPath(item) => match r.item_path(item) {
                Ok(buf) => Outcome::Value(text(Some(buf.as_ref().as_c_str()))),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::Config(key) => match r.config() {
                Ok(config) => read_int32(scratch, config.as_c_ptr(), key),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::Snapshot(key) => match r.config_snapshot() {
                Ok(config) => read_int32(scratch, config.as_c_ptr(), key),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::Index => match r.index() {
                Ok(index) => read_index(scratch, index.as_c_ptr()),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::HashFile(path, kind, as_path) => {
                match r.hash_file(&scratch.c_path(path), kind, as_path) {
                    Ok(id) => Outcome::Value(id.as_ref().to_hex_string().unwrap()),
                    Err(error) => rust_failed(scratch, &error),
                }
            }
            Step::Head => match r.head() {
                // SAFETY: a live reference the box owns; only its fields are read.
                Ok(head) => Outcome::Value(unsafe { describe(head.as_c_ptr()) }),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::HeadForWorktree(name) => match r.head_for_worktree(name) {
                // SAFETY: as above.
                Ok(head) => Outcome::Value(unsafe { describe(head.as_c_ptr()) }),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::HeadDetached => rust_answer(scratch, r.is_head_detached()),
            Step::HeadUnborn => rust_answer(scratch, r.is_head_unborn()),
            Step::IsEmpty => rust_answer(scratch, r.is_empty()),
            Step::HeadDetachedForWorktree(name) => {
                rust_answer(scratch, r.is_head_detached_for_worktree(name))
            }
            Step::DetachHead => match r.detach_head() {
                Ok(()) => Outcome::Value(head_file(scratch, owner.as_c_ptr())),
                Err(error) => rust_failed(scratch, &error),
            },
            Step::CommitParents => match r.commit_parents() {
                Ok(array) => {
                    let commits: Vec<_> = array.as_ref().commit_ptrs().elems().collect();
                    // SAFETY: the array keeps its commits alive.
                    Outcome::Value(unsafe { commit_ids(commits.as_ptr(), commits.len()) })
                }
                Err(error) => rust_failed(scratch, &error),
            },
            Step::Open(_)
            | Step::New
            | Step::Cleanup
            | Step::Write(..)
            | Step::SetNamespace(_)
            | Step::SetIdent(..) => unreachable!(),
        }
    }

    /// Runs `steps` on each side, each on a thread and in fixtures of its own, and
    /// compares the outcomes step by step.
    fn compare(tag: &str, steps: &'static [Step]) {
        init();
        let c = thread::spawn({
            let tag = format!("{tag}-c");
            move || {
                let scratch = Scratch::with_fixtures(&tag);
                let mut repo = CRepo(ptr::null_mut());
                steps
                    .iter()
                    .map(|&s| c_step(&scratch, &mut repo, s))
                    .collect::<Vec<_>>()
            }
        })
        .join()
        .unwrap();
        let rust = thread::spawn({
            let tag = format!("{tag}-rust");
            move || {
                let scratch = Scratch::with_fixtures(&tag);
                let mut repo = None;
                steps
                    .iter()
                    .map(|&s| rust_step(&scratch, &mut repo, s))
                    .collect::<Vec<_>>()
            }
        })
        .join()
        .unwrap();
        for (i, ((c, rust), step)) in c.iter().zip(&rust).zip(steps).enumerate() {
            assert_eq!(c, rust, "step {i}: {step:?}");
        }
        assert_eq!(c.len(), rust.len());
    }

    /// Every item, on the current repository.
    const ITEMS: [Step; 15] = [
        Step::ItemPath(Item::GitDir),
        Step::ItemPath(Item::WorkDir),
        Step::ItemPath(Item::CommonDir),
        Step::ItemPath(Item::Index),
        Step::ItemPath(Item::Objects),
        Step::ItemPath(Item::Refs),
        Step::ItemPath(Item::PackedRefs),
        Step::ItemPath(Item::Remotes),
        Step::ItemPath(Item::Config),
        Step::ItemPath(Item::Info),
        Step::ItemPath(Item::Hooks),
        Step::ItemPath(Item::Logs),
        Step::ItemPath(Item::Modules),
        Step::ItemPath(Item::Worktrees),
        Step::ItemPath(Item::WorktreeConfig),
    ];

    /// The getters and predicates on each kind of repository: bare, unborn, shallow,
    /// with a working directory, a linked worktree's, mid-merge, and without paths.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_read_what_c_reads() {
        const fn on(open: Step) -> [Step; 16] {
            [
                open,
                Step::Commondir,
                Step::IsBare,
                Step::IsWorktree,
                Step::Namespace,
                Step::Ident,
                Step::IsShallow,
                Step::Config(c"core.repositoryformatversion"),
                Step::Config(c"core.bare"),
                Step::Snapshot(c"core.repositoryformatversion"),
                Step::Snapshot(c"no.such"),
                Step::Index,
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                Step::IsEmpty,
            ]
        }
        const STEPS: &[Step] = &concat_steps();
        const fn concat_steps() -> [Step; 7 * 16 + 15 * 3] {
            let opens = [
                Step::Open("bare"),
                Step::Open("empty"),
                Step::Open("shallow"),
                Step::Open("testrepo"),
                Step::Open("testrepo-worktree"),
                Step::Open("merged"),
                Step::New,
            ];
            let mut out = [Step::IsBare; 7 * 16 + 15 * 3];
            let mut n = 0;
            let mut i = 0;
            while i < opens.len() {
                let steps = on(opens[i]);
                let mut j = 0;
                while j < steps.len() {
                    out[n] = steps[j];
                    n += 1;
                    j += 1;
                }
                // Every item of a bare, a linked worktree's and a pathless repository.
                if i == 0 || i == 4 || i == 6 {
                    let mut k = 0;
                    while k < ITEMS.len() {
                        out[n] = ITEMS[k];
                        n += 1;
                        k += 1;
                    }
                }
                i += 1;
            }
            out
        }
        compare("getters", STEPS);
    }

    /// The namespace and ident read back what the setters stored, across replacements
    /// and resets.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn namespace_and_ident_read_back_what_was_set() {
        compare(
            "setters",
            &[
                Step::Open("bare"),
                Step::Namespace,
                Step::SetNamespace(Some(c"foo")),
                Step::Namespace,
                Step::SetNamespace(Some(c"foo/bar")),
                Step::Namespace,
                Step::SetNamespace(None),
                Step::Namespace,
                Step::Ident,
                Step::SetIdent(Some(c"Name"), Some(c"name@example.com")),
                Step::Ident,
                Step::SetIdent(None, Some(c"other@example.com")),
                Step::Ident,
                Step::SetIdent(None, None),
                Step::Ident,
                Step::New,
                Step::SetNamespace(Some(c"in-memory")),
                Step::Namespace,
                Step::SetIdent(Some(c"x"), None),
                Step::Ident,
            ],
        );
    }

    /// `HEAD` in each state: attached, detached, at a missing object, at a tag, at a
    /// missing branch, missing altogether; and the parents a commit would get.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn head_states_read_as_c_reads_them() {
        compare(
            "head",
            &[
                Step::Open("bare"),
                Step::CommitParents,
                // Detached at a commit.
                Step::Write("bare/HEAD", "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n"),
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                Step::IsEmpty,
                Step::CommitParents,
                // Detached at a missing object.
                Step::Write("bare/HEAD", "0123456789012345678901234567890123456789\n"),
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                // Detached at a tag.
                Step::Write("bare/HEAD", "b25fa35b38051e4ae45d4222e795f9df2e43f1d1\n"),
                Step::Head,
                Step::HeadDetached,
                // At a missing branch, then a symbolic chain.
                Step::Write("bare/HEAD", "ref: refs/heads/missing\n"),
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                Step::IsEmpty,
                Step::CommitParents,
                Step::Write("bare/HEAD", "ref: refs/heads/packed\n"),
                Step::Head,
                Step::CommitParents,
                // Garbage, then no HEAD at all.
                Step::Write("bare/HEAD", "garbage\n"),
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                Step::IsEmpty,
                Step::CommitParents,
                Step::Open("empty"),
                Step::Head,
                Step::CommitParents,
                Step::Write(
                    "empty/refs/heads/other",
                    "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n",
                ),
                Step::IsEmpty,
                Step::Open("merged"),
                Step::CommitParents,
                Step::Write(
                    "merged/.git/MERGE_HEAD",
                    "e2809157a7766f272e4cfe26e61ef2678a5357ff\n3a34580a35add43a4cf361e8e9a30060a905c876\n",
                ),
                Step::CommitParents,
                Step::New,
                Step::Head,
                Step::HeadDetached,
                Step::HeadUnborn,
                Step::IsEmpty,
                Step::CommitParents,
            ],
        );
    }

    /// A linked worktree's `HEAD` through its main repository, and the routines'
    /// answers for worktrees that do not exist or are broken.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn worktree_heads_read_as_c_reads_them() {
        compare(
            "worktree",
            &[
                Step::Open("testrepo"),
                // Attached: C answers 1 (see the unit tests), so only the reference is
                // compared.
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::HeadForWorktree(c"missing"),
                Step::HeadDetachedForWorktree(c"missing"),
                // Detached, then at a missing branch.
                Step::Write(
                    "testrepo/.git/worktrees/testrepo-worktree/HEAD",
                    "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\n",
                ),
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::HeadDetachedForWorktree(c"testrepo-worktree"),
                Step::Write(
                    "testrepo/.git/worktrees/testrepo-worktree/HEAD",
                    "ref: refs/heads/missing\n",
                ),
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::HeadDetachedForWorktree(c"testrepo-worktree"),
                // A packed branch, whose reference may carry a peeled id.
                Step::Write(
                    "testrepo/.git/worktrees/testrepo-worktree/HEAD",
                    "ref: refs/heads/packed\n",
                ),
                Step::HeadForWorktree(c"testrepo-worktree"),
                // The worktree's own repository, and one with no worktrees.
                Step::Open("testrepo-worktree"),
                Step::Head,
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::Open("bare"),
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::HeadDetachedForWorktree(c"testrepo-worktree"),
                // A broken worktree: its gitdir is gone.
                Step::Open("testrepo"),
                Step::Write(
                    "testrepo/.git/worktrees/testrepo-worktree/gitdir",
                    "/nonexistent/.git\n",
                ),
                Step::HeadForWorktree(c"testrepo-worktree"),
                Step::HeadDetachedForWorktree(c"testrepo-worktree"),
            ],
        );
    }

    /// Files hashed as each type, with and without the repository's filters; then a
    /// filter that is required but missing, and a very long path. (No attribute
    /// macros: loading one takes libgit2's attribute-cache locks in the order freeing
    /// the repository reverses, which TSan reports as a lock-order inversion.)
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn hash_file_matches_c() {
        // Statics, not leaks, so LSan sees them reachable.
        static LONG: LazyLock<String> = LazyLock::new(|| format!("@{}", "a/".repeat(2100)));
        static FAILING: LazyLock<Vec<Step>> = LazyLock::new(|| {
            vec![
                Step::Open("testrepo"),
                Step::HashFile(&LONG, ObjectType::Blob, None),
                Step::HashFile(&LONG, ObjectType::Blob, Some(c"")),
                Step::Write("testrepo/.gitattributes", "*.txt filter=missing ident\n"),
                Step::Write(
                    "testrepo/.git/config",
                    "[core]\n\tbare = false\n[filter \"missing\"]\n\trequired = true\n",
                ),
                Step::Write("testrepo/x.txt", "$Id$\n"),
                Step::HashFile("testrepo/x.txt", ObjectType::Blob, None),
            ]
        });
        compare("hash-failing", &FAILING);
        compare(
            "hash",
            &[
                Step::Open("testrepo"),
                Step::Write("testrepo/.gitattributes", "*.txt text eol=lf\n"),
                Step::Write("testrepo/crlf.txt", "a\r\nb\r\n"),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, None),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, Some(c"")),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, Some(c"other.txt")),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, Some(c"other.bin")),
                Step::HashFile("@crlf.txt", ObjectType::Blob, None),
                Step::HashFile("@missing.txt", ObjectType::Blob, None),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Commit, None),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Tree, None),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Tag, None),
                Step::HashFile("testrepo", ObjectType::Blob, None),
                Step::HashFile("bare/HEAD", ObjectType::Blob, None),
                Step::Open("bare"),
                Step::HashFile("bare/HEAD", ObjectType::Blob, None),
                Step::HashFile("bare/HEAD", ObjectType::Blob, Some(c"x.txt")),
                Step::New,
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, None),
                Step::HashFile("testrepo/crlf.txt", ObjectType::Blob, Some(c"crlf.txt")),
            ],
        );
    }

    /// The repository cleaned up, then used again: everything is loaded afresh.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cleanup_resets_what_c_resets() {
        compare(
            "cleanup",
            &[
                Step::Open("testrepo"),
                Step::Index,
                Step::Config(c"core.repositoryformatversion"),
                Step::Head,
                Step::Cleanup,
                Step::Cleanup,
                Step::Index,
                Step::Config(c"core.repositoryformatversion"),
                Step::Head,
                Step::CommitParents,
                Step::Write(
                    "testrepo/.git/config",
                    "[core]\n\trepositoryformatversion = 0\n\tbare = false\n[x]\n\ty = 7\n",
                ),
                Step::Config(c"x.y"),
                Step::Cleanup,
                Step::Config(c"x.y"),
                // A config that no longer parses, loaded afresh after a cleanup. (Not
                // read through the config already loaded: its file backend refreshes,
                // fails, and `git_config_get_*` then forms `&be->entry` from a NULL
                // entry, config.c:811, which UBSan reports.)
                Step::Write("testrepo/.git/config", "[core\n"),
                Step::Cleanup,
                Step::Config(c"x.y"),
                Step::Snapshot(c"x.y"),
                Step::Index,
                Step::New,
                Step::Cleanup,
                Step::IsBare,
            ],
        );
    }

    /// Runs `body` alone in a child process (`test` is its path), with LSan off when
    /// `tolerating_leaks`.
    fn in_child(test: &str, tolerating_leaks: bool, body: fn()) {
        const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name);
        if tolerating_leaks {
            let asan = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            command.env("ASAN_OPTIONS", format!("{asan}:detect_leaks=0"));
        }
        let out = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// `git_repository_detach_head` and its wrapper, on each kind of `HEAD`. Writing a
    /// reference takes the reflog's time through glibc's `tzset`, which TSan reports
    /// against other tests of this binary, so this runs alone in a child process.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn detach_head_moves_what_c_moves() {
        in_child(
            concat!(module_path!(), "::detach_head_moves_what_c_moves"),
            false,
            || {
                compare(
                    "detach",
                    &[
                        Step::Open("bare"),
                        Step::SetIdent(Some(c"Name"), Some(c"name@example.com")),
                        Step::DetachHead,
                        Step::HeadDetached,
                        Step::DetachHead,
                        Step::Write("bare/HEAD", "b25fa35b38051e4ae45d4222e795f9df2e43f1d1\n"),
                        Step::DetachHead,
                        Step::Write("bare/HEAD", "ref: refs/heads/missing\n"),
                        Step::DetachHead,
                        Step::Write("bare/HEAD", "ref: refs/tags/point_to_blob\n"),
                        Step::DetachHead,
                        Step::Open("empty"),
                        Step::DetachHead,
                        Step::Open("testrepo-worktree"),
                        Step::DetachHead,
                        Step::Head,
                        Step::Open("testrepo"),
                        Step::HeadDetachedForWorktree(c"testrepo-worktree"),
                        Step::New,
                        Step::DetachHead,
                    ],
                );
            },
        );
    }

    /// A `HEAD` naming a missing object gives no parents on either side, and one naming
    /// an unreadable object fails alike. C leaks `HEAD`'s reference in both (inside
    /// `git_revparse_ext`), so this runs in a child with leak detection off; the unit
    /// tests hold the wrapper to no leak.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_head_at_a_missing_object_has_no_parents() {
        in_child(
            concat!(
                module_path!(),
                "::a_head_at_a_missing_object_has_no_parents"
            ),
            true,
            || {
                compare(
                    "missing-head",
                    &[
                        Step::Open("bare"),
                        Step::Write("bare/HEAD", "0123456789012345678901234567890123456789\n"),
                        Step::CommitParents,
                        Step::Write("bare/HEAD", "ref: refs/heads/master\n"),
                        Step::Write(
                            "bare/refs/heads/master",
                            "0123456789012345678901234567890123456789\n",
                        ),
                        Step::CommitParents,
                        Step::Open("merged"),
                        Step::Write(
                            "merged/.git/HEAD",
                            "0123456789012345678901234567890123456789\n",
                        ),
                        Step::CommitParents,
                        // An object that cannot be read: a failure, before any is collected.
                        Step::Write(
                            "merged/.git/objects/01/23456789012345678901234567890123456789",
                            "garbage",
                        ),
                        Step::CommitParents,
                    ],
                );
            },
        );
    }

    /// Discovery from each start, across filesystems or not, under ceilings.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn discover_finds_what_c_finds() {
        init();
        let scratch = Scratch::with_fixtures("discover");
        fs::create_dir_all(scratch.0.join("testrepo/a/b")).unwrap();
        fs::create_dir_all(scratch.0.join("plain/a")).unwrap();
        fs::write(scratch.0.join("plain/a/.git"), "gitdir: ../../missing\n").unwrap();
        let root = scratch.0.to_str().unwrap().to_owned();
        let starts = [
            "testrepo",
            "testrepo/a/b",
            "testrepo/.git",
            "testrepo/.git/objects",
            "bare",
            "bare/refs",
            "testrepo-worktree",
            "plain",
            "plain/a",
            "missing",
            "",
        ];
        let ceilings = [
            None,
            Some(String::new()),
            Some(format!("{root}/testrepo")),
            Some(format!("{root}/testrepo/a")),
            Some(format!("/nonexistent:{root}/testrepo/a:{root}")),
            Some("relative".to_owned()),
        ];
        for start in starts {
            for ceiling in &ceilings {
                for across_fs in [false, true] {
                    let start_path = if start.is_empty() {
                        CString::default()
                    } else {
                        scratch.c_path(start)
                    };
                    let ceiling = ceiling.as_deref().map(|c| CString::new(c).unwrap());
                    let ceiling_ptr = ceiling.as_deref().map_or(ptr::null(), CStr::as_ptr);
                    let mut buf = ffi::git_buf::default();
                    // SAFETY: an empty buffer and NUL-terminated (or NULL ceiling) paths.
                    let rc = unsafe {
                        ffi::git_repository_discover(
                            &mut buf,
                            start_path.as_ptr(),
                            across_fs.into(),
                            ceiling_ptr,
                        )
                    };
                    let c = if rc < 0 {
                        c_failed(&scratch, rc)
                    } else {
                        // SAFETY: the buffer the call filled.
                        Outcome::Value(unsafe { text(&scratch, buf.ptr) })
                    };
                    // SAFETY: disposed once.
                    unsafe { ffi::git_buf_dispose(&mut buf) };
                    let rust = match discover(&start_path, across_fs, ceiling.as_deref()) {
                        Ok(buf) => Outcome::Value(scratch.relative(buf.as_ref().to_str().unwrap())),
                        Err(error) => rust_failed(&scratch, &error),
                    };
                    assert_eq!(
                        c, rust,
                        "start {start:?}, ceiling {ceiling:?}, across_fs {across_fs}"
                    );
                }
            }
        }
    }

    // --- `FETCH_HEAD` callback ---------------------------------------------------------

    /// An entry as a callback records it.
    type Entry = (Option<String>, Option<String>, String, bool);

    /// What a recording callback does after recording entry `i`.
    #[derive(Clone, Copy, Debug)]
    enum Answer {
        Continue,
        StopAt(usize),
    }

    /// The C callback's payload: its answer, and the entries it saw.
    struct Recorder {
        answer: Answer,
        code: c_int,
        entries: Vec<Entry>,
    }

    fn copied(s: *const c_char) -> Option<String> {
        // SAFETY: libgit2 passes NULL or a string live for the call.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
    }

    /// A C callback recording each entry, stopping with `code` where told.
    unsafe extern "C" fn c_recorder(
        ref_name: *const c_char,
        remote_url: *const c_char,
        oid: *const ffi::git_oid,
        is_merge: c_uint,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: the payload is the test's live recorder, used by nothing else.
        let recorder = unsafe { &mut *payload.cast::<Recorder>() };
        // SAFETY: a live id.
        recorder.entries.push((
            copied(ref_name),
            copied(remote_url),
            unsafe { hex(oid) },
            is_merge != 0,
        ));
        match recorder.answer {
            Answer::StopAt(i) if recorder.entries.len() == i + 1 => recorder.code,
            _ => 0,
        }
    }

    /// `git_repository_fetchhead_foreach` over `contents` with the C recorder and with
    /// the Rust callback, each in a repository of its own; the returns, the errors and
    /// the entries seen must agree.
    fn compare_fetchhead(contents: Option<&str>, answer: Answer) {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        init();
        let run = |tag: &str, rust: bool| {
            let scratch = Scratch::new(&format!("{tag}-{}", RUNS.fetch_add(1, SeqCst)));
            scratch.sandbox("testrepo.git", "bare");
            match contents {
                Some(contents) => fs::write(scratch.0.join("bare/FETCH_HEAD"), contents).unwrap(),
                // The fixture has one.
                None => fs::remove_file(scratch.0.join("bare/FETCH_HEAD")).unwrap(),
            }
            let repo = Repository::open(&scratch.c_path("bare")).unwrap();
            let mut entries = Vec::new();
            let rc = if rust {
                let mut seen = 0;
                let mut record = |entry: FetchheadEntry<'_>| {
                    let text = |s: Option<&CStr>| s.map(|s| s.to_string_lossy().into_owned());
                    entries.push((
                        text(entry.ref_name),
                        text(entry.remote_url),
                        entry.id.to_hex_string().unwrap(),
                        entry.is_merge,
                    ));
                    seen += 1;
                    match answer {
                        Answer::StopAt(i) if seen == i + 1 => ControlFlow::Break(()),
                        _ => ControlFlow::Continue(()),
                    }
                };
                let mut callback = FetchheadForeachCallback::new(&mut record);
                let (f, payload) = callback.raw();
                // SAFETY: a live repository and the callback's own function and payload,
                // which stays put and unused for the call.
                let rc =
                    unsafe { ffi::git_repository_fetchhead_foreach(repo.as_c_ptr(), f, payload) };
                callback.resume_panic();
                assert_eq!(callback.stopped(), rc == ffi::GIT_EUSER);
                rc
            } else {
                let mut recorder = Recorder {
                    answer,
                    code: ffi::GIT_EUSER,
                    entries: Vec::new(),
                };
                // SAFETY: a live repository and the recorder, live for the call.
                let rc = unsafe {
                    ffi::git_repository_fetchhead_foreach(
                        repo.as_c_ptr(),
                        Some(c_recorder),
                        (&raw mut recorder).cast(),
                    )
                };
                entries = recorder.entries;
                rc
            };
            let outcome = if rc < 0 {
                c_failed(&scratch, rc)
            } else {
                Outcome::Value(rc.to_string())
            };
            (outcome, entries)
        };
        let c = run("fetchhead-c", false);
        let rust = run("fetchhead-rust", true);
        assert_eq!(c, rust, "FETCH_HEAD {contents:?}, {answer:?}");
    }

    const FETCH_HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\t\tbranch 'master' of git://example.com/git/testrepo.git\n\
        258f0e2a959a364e40ed6603d5d44fbb24765b10\tnot-for-merge\tbranch 'haacked' of git://example.com/git/testrepo.git\n\
        b25fa35b38051e4ae45d4222e795f9df2e43f1d1\tnot-for-merge\ttag 'test' of git://example.com/git/testrepo.git\n\
        e90810b8df3e80c413d903f631643c716887138d\tnot-for-merge\t'refs/pull/1/head' of git://example.com/git/testrepo.git\n\
        41bc8c69075bbdb46c5c6f0566cc8cc5b46e8bd9\t\tgit://example.com/git/testrepo.git\n\
        a4a7dce85cf63874e984719f4fdd239f5145052f\n";

    /// Every entry form, read through; then stopped at each entry.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn fetchhead_callback_sees_what_a_c_callback_sees() {
        compare_fetchhead(Some(FETCH_HEAD), Answer::Continue);
        for i in 0..6 {
            compare_fetchhead(Some(FETCH_HEAD), Answer::StopAt(i));
        }
    }

    /// Malformed or missing files fail before or between calls alike.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn fetchhead_callback_fails_where_c_fails() {
        compare_fetchhead(None, Answer::Continue);
        for contents in [
            "",
            "\n",
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\t\tbranch 'master' of x\nshort\n",
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\tmaybe\tbranch 'master' of x\n",
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\t\tbranch 'master' from x\n",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\t\tbranch 'master' of x\n",
        ] {
            compare_fetchhead(Some(contents), Answer::Continue);
        }
    }

    /// The Rust callback stops the C iteration with `GIT_EUSER` when its closure
    /// breaks, as a C callback returning that code does, and a closure's panic stops
    /// it the same way before resuming once C has returned.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn fetchhead_callback_panic_stops_as_euser() {
        init();
        let scratch = Scratch::new("fetchhead-panic");
        scratch.sandbox("testrepo.git", "bare");
        fs::write(scratch.0.join("bare/FETCH_HEAD"), FETCH_HEAD).unwrap();
        let repo = Repository::open(&scratch.c_path("bare")).unwrap();
        let seen = RefCell::new(0);
        let mut panicking = |_: FetchheadEntry<'_>| -> ControlFlow<()> {
            *seen.borrow_mut() += 1;
            panic!("closure panicked");
        };
        let mut callback = FetchheadForeachCallback::new(&mut panicking);
        let (f, payload) = callback.raw();
        // libgit2 names the callback in the error only when none is set.
        // SAFETY: the runtime is initialised.
        unsafe { ffi::git_error_clear() };
        // SAFETY: a live repository and the callback's own function and payload.
        let rc = unsafe { ffi::git_repository_fetchhead_foreach(repo.as_c_ptr(), f, payload) };
        assert_eq!(rc, ffi::GIT_EUSER);
        assert!(callback.stopped());
        let (class, message) = c_last();
        assert_eq!(
            (class, message.as_str()),
            (
                ffi::GIT_ERROR_CALLBACK as c_int,
                "git_repository_fetchhead_foreach callback returned -7"
            )
        );
        let resumed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback.resume_panic()));
        assert_eq!(
            *resumed.unwrap_err().downcast::<&str>().unwrap(),
            "closure panicked"
        );
        drop(callback);
        assert_eq!(*seen.borrow(), 1);
    }
}
