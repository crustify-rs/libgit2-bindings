# `git_config_backend` slot notes left out implementer contracts the follow-up relied on

- **item**: `git_config_backend` (type), fields `get`, `iterator`, `snapshot`, `lock`,
  in `crustify/wavefront/ownership-store.json`
- **affected commit**: `df0deef13` (record last updated by `a66691e90`)
- **class**: wavefront finding incomplete

## Finding

Now that the public constructor is the author path, the record's function-pointer
notes are a backend author's contract. The follow-up added that contract to `get`
(succeed or `GIT_ENOTFOUND` only, read-only answers outlive their release). It did not
record the other C behaviour its `CustomBackend` trampolines depend on, or the behaviour
this review found. The `iterator`, `snapshot` and `lock` notes still read only
"Allocation-specific immutable function pointer; nullable in git_config_init_backend
template...". Evidence from `src/libgit2/config.c`:

- `get`: the note did not say that a miss must not set an error. `get_entry` returns 0
  without clearing the error for `GET_NO_MISSING`/`GET_NO_ERRORS` lookups and when a
  lower backend answers (`:820`). An error set on the miss then survives a successful
  call or a later silent failure (`../../equiv/custom-get-miss-overwrites-thread-error`).
- `iterator`: in `all_iter_next` (`:436-494`), `GIT_ENOTFOUND` from the slot skips the
  backend (`:471`). Another failure is returned, and the next advance moves on, because
  `iter->i` was already decremented. A failing `next` is called again on the following
  advance (`:446-447`). So an iterator must end after an error. `Entries::next` does that
  (`entries = None`), and without it `ConfigRef::entry`'s reread walk never ends. The
  produced iterator is the caller's, freed through its own `free` slot.
- `snapshot`: `git_config_snapshot` (`:182-212`) hands the produced backend to
  `git_config_add_backend` at the source's level with a NULL repository. That call opens
  it, and the backend is freed through its own `free` slot when this fails (`:200-201`).
  The result must be unopened and read-only, which is why the trampoline marks it
  read-only.
- `lock`: `git_config_lock` dispatches `unlock` only after `lock` succeeds. When
  `git_transaction_config_new` then fails, the backend stays locked and `unlock` is never
  called (`:1318-1320`).

## Fix

Resubmitted through `wavefront ... query types --name git_config_backend --update`. Each
of the four notes now ends with the contract above. The store diff is those four lines
(no re-sort), and `query types --name git_config_backend` reads them back with
`_analysis.submitted: true, pending: []`. No ownership flag changed: the existing
`owned`/`borrowed`/`nullable` values are correct for function pointers.
