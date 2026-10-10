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
