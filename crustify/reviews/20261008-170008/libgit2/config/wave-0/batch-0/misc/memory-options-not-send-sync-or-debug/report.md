# `BackendMemoryOptions` is neither `Send`/`Sync` nor `Debug`

- **item**: `git_config_backend_memory_options` (type),
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `831989948` (libgit2/config wave 0 batch 0)
- **class**: misc — API ergonomics; the existing surface is sound

## Finding

`BackendMemoryOptions<'o>` is a `u32` and two pointers, each NULL or the pointer of a
`&'o CStr` stored by the setters. The struct has no interior mutability, no thread affinity
and nothing to release. Its `PhantomData<(*const (), &'o CStr)>` marker still made it, and
its `Ref`/`Mut` handles (`CBorrowedPtr`), `!Send` and `!Sync`. So options built on one
thread could not be handed to another, for example to a thread that will build an
in-memory backend from them. It also had no `Debug`, so `{:?}` on options or on a struct
holding them did not compile.

The crate's other plain options structs that hold string borrows have both: `TreeUpdate<'p>`
(a `&'p CStr`) and `FilterOptions` are `Send + Sync`, and their handles carry the matching
impls, under the same immutability argument. `FilterOptions`, `AddOptions` and
`CreateExtOptions` also implement `Debug` through their `Ref` handle.

## Fix

- `unsafe impl Send/Sync for BackendMemoryOptions<'_>`, `Send + Sync` for
  `BackendMemoryOptionsRef` (as `&T`), `Send` for `BackendMemoryOptionsMut` (as `&mut T`)
  and `Sync` for it (a `&Mut` only reaches the getters). Each has a `// SAFETY:` comment:
  the string pointers are `&CStr` borrows, which are `Send + Sync`, and every write goes
  through the exclusive handle. `'o` still ties the options to the borrowed strings, so a
  value holding a non-`'static` borrow cannot reach `thread::spawn`. A new
  `compile_fail,E0597` doctest on `BackendMemoryOptions` checks this.
- `Debug` for the owner and both handles prints `version`, `backend_type` and `origin_path`.

## Tests

- `tests/ub_safe/sys_config.rs::memory_options_cross_threads` (TSan): concurrent readers
  through copied `Ref` handles and through `&BackendMemoryOptions`, a `Mut` writing on
  another scoped thread, and `'static` options moved to a detached thread.
- `sys/config.rs::unit_tests::debug_and_thread_traits` checks the trait bounds and the
  `Debug` output of the owner and both handles (also run under Miri).
