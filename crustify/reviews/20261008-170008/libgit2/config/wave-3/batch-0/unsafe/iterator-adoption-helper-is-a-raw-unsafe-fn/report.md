# Iterator constructors adopt through a private raw-pointer `unsafe fn`

- **item**: `git_config_iterator` (type): constructors `ConfigRef::iterator`,
  `iterator_matching` and `multivar_iterator`,
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `0de2b59b4` (libgit2/config wave 3 batch 0)
- **class**: unsafe (scan-unsafe smell)

## Finding

The three constructors handed C's out-pointer and status to a free helper:

```rust
unsafe fn adopt_iterator<'c>(p: *mut ffi::git_config_iterator, rc: i32)
    -> Result<IteratorBox<'c>, ConfigError>
```

The function was private and sound. But it moved the ownership claim away from the C call
that justifies it. Its `# Safety` contract ("a successful status supplies a uniquely owned
complete iterator ... dependencies remain live for `'c`") applied the same way to all
three constructors. It did not say which allocation each one adopts, and `'c` was an
unconstrained generic lifetime that each caller had to pick correctly. `crustify
scan-unsafe --sites all` flagged it at `config/sys/config.rs:983` as
`unsafe_fns_priv_smell` and `raw_ptr_priv_smell`. These were the workset's only private
smells.

## Fix

Each constructor now checks the status and adopts its own result in one documented
`unsafe` block. The block's `SAFETY` comment names the allocation C just wrote (all, glob
or multivar iterator), its free callback and the `'a` config borrow it inherits. The
helper is gone, and `ConfigError` is imported once in the module rather than spelled
`crate::libgit2::config::config::ConfigError` at each use. Behaviour is unchanged:
`iterator_empty_and_failed_construction`, `iterator_sequences_match_c` and a new inline
`allocation_failures::constructors_report_allocation_failure` test cover the success and
failure paths. Scan: `unsafe_fns` 232 → 231, `unsafe_fns_priv_smell` 18 → 17,
`raw_ptr_priv_smell` 20 → 19, no new undocumented site.
