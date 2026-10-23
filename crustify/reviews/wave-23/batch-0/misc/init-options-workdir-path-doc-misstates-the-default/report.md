# `InitOptionsMut::set_workdir_path` documented a working directory as required where libgit2 appends `.git`

- **item**: `git_repository_init_options` (type), field `workdir_path`,
  `crustify/rust/libgit2/libgit2/repository/repository.rs`, the doc comment of
  `InitOptionsMut::set_workdir_path`
- **affected commit**: `f5ba572b1` (libgit2/repository wave 1 batch 0, the `InitOptions`
  wrapper); reviewed at `352115fb7`
- **class**: documentation states a requirement the C does not have; no code change,
  no reproducer

## Finding

The setter's doc said "`None` means the repository directory's parent" and that "a
non-bare repository whose path does not end in `.git` needs one". Neither holds as
stated. In `repo_init_directories` (`repository.c:2749`), with no `workdir_path` and no
`GIT_REPOSITORY_INIT_BARE`, a repository path that ends in neither `/.git` nor `/.git/`
gets `.git` appended (`add_dotgit`), so the path given becomes the working directory
and the repository is created inside it; only a path already ending in `.git` takes its
parent. Nothing is "needed": the equivalence sequence's first step
(`tests/equiv/repository.rs::init_options::init_ext_reads_rust_options_as_c_options`,
path `plain`, no working directory set) succeeds on both sides. What a `workdir_path`
really changes is that the repository path is then used as given (no `.git` appended),
with the working directory resolved against it, which is what the sequence's
`sep/repo` + `../wd` step shows. The getter's doc (`InitOptionsRef::workdir_path`)
already said so; the setter contradicted it.

## Fix

The setter's doc now describes both cases: `None` takes the parent of a `.git` path and
makes libgit2 append `.git` to any other, so that the path given is the working
directory; a value makes libgit2 use the repository path as given, resolve a relative
working directory against it, and (unless it is the parent) write the `.git` gitlink
and `core.worktree`. While there, `set_template_path`'s doc gained the outcome this
review measured for a path the file system refuses (an over-long component):
`GIT_EINVALIDSPEC` once the repository directory exists, where a missing directory or a
file falls back to the built-in template (equivalence step `long-tpl`).
