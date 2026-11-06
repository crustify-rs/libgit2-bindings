# Two pointers C retains were recorded as borrowed for the call only

- **items**: `git_config_lock` (arg 1 `config`, and the note on arg 0 `out`) and
  `git_config_add_file_ondisk` (arg 3 `repo`), submitted by libgit2/config wave 1
  batch 1 (`5a0f495b6`)
- **affected commit**: `d79771f6f`
- **class**: incomplete ownership finding (Wavefront store). The Rust wrappers already
  model both retentions correctly, so no Rust fix was needed.

## Finding

The schema (`query symbols --schema`, `borrowed`) says "A transient read that doesn't
outlive the call borrows from its OWN arg -- `arg:<its own name>`". A pointer the callee
stores past the call is bound to whatever keeps it, not to itself. Two records gave
retained pointers their own name:

1. `git_config_lock`, arg 1 `config`: `borrowed: {lifetime: "arg:config"}`, with the
   generic getter note "Borrowed config; getters may refresh C backend state internally".
   C retains it. `git_config_lock` passes `config` to `git_transaction_config_new`, which
   stores it in the transaction it writes to `out` (`src/libgit2/transaction.c`):

   ```c
   	tx->type = TRANSACTION_CONFIG;
   	tx->cfg = cfg;
   	tx->cfg_data = data;
   ```

   `git_transaction_commit` and `git_transaction_free` later pass it to
   `git_config_unlock(tx->cfg, tx->cfg_data, ..)`. `git_config_lock` also does
   `GIT_REFCOUNT_INC(instance)` on the locked backend. The config must outlive the
   transaction, and its locked backend must not be locked again while the transaction
   lives. A consumer reading the record would bind nothing to `out` and could let the
   config die first. `ConfigTransaction` gets this right: it holds the `ConfigMut<'c, 'r>`
   next to the transaction owner and drops the transaction first. The record did not say
   so.

   The note on arg 0 `out` was the shared constructor template: "success transfers fresh
   object paired with git_config_free (or git_transaction_free for lock)". For this
   symbol the only pairing is `git_transaction_free`.

2. `git_config_add_file_ondisk`, arg 3 `repo`: `borrowed: {lifetime: "arg:repo"}`. The
   record's own note says the opposite ("File backend retains repository without upref
   ... It must outlive config and all level views"). `config_file_open`
   (`src/libgit2/config_file.c`) stores it, `b->repo = repo;`, and every reparse passes
   `b->repo` to the `includeIf` evaluators (`conditional_match_gitdir` →
   `git_repository__item_path`, `conditional_match_onbranch` → `git_repository_refdb`).
   `ConfigMut::add_file` takes a `RepositoryRef<'r>` tied to the config's `'r`, and a
   `compile_fail` case checks that.

## Fix

Resubmitted both records through `wavefront query symbols --name <sym> --update`:

- `git_config_lock`, arg 1: `borrowed: {lifetime: "arg:out"}`, with a note giving the
  `tx->cfg` retention, the backend up-ref and the "outlive the transaction / no relock"
  obligation. Arg 0: the note now names `git_transaction_free` (which unlocks without
  committing) as the only release.
- `git_config_add_file_ondisk`, arg 3: `borrowed: {lifetime: "arg:config"}`, with a note
  citing `config_file_open`'s `b->repo` and the `includeIf` reparse paths.

`query symbols --name git_config_lock git_config_add_file_ondisk` reads the corrected
blocks back. The store diff is the 5 changed lines only.
