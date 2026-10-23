# Planner run 2: re-plan the whole campaign from scratch

This campaign is resumed and has just moved to its own repository, laid out as the
current specs describe: the C under `c/` (libgit2 at `0551dfd4a`), the safe crate at the
root, the branches `main` and `crustify/devel`. Under the earlier specs it landed 54 waves
(nine subsystems: `raw-lifetime-void`, `raw-lifetime-string`, `util`, `oid`, `core`,
`repository`, `object`, `config`, `index`) and planner run 1 re-planned it under the
schema-v2 artifacts that are still on your branch: `crustify/artifacts/subsystems.json`
and the per-subsystem `crustify/artifacts/libgit2/<subsystem>/schedule.json`.

The user wants the whole plan redone, to observe how the current specs would have
planned the campaign, the work that has landed included. The sections below depart from
your role's "keep the earlier decisions" on purpose; apply them as written.

## 1. Re-plan from the evidence alone (departure)

Emit `modules.json`, `dependencies.json` and `schedule.json` for the campaign's whole
scope, as the decomposition and scheduling specs describe, as if no wave had run and no
earlier run had decided anything:

- decompose the build into modules, one per translation unit with its headers and tests,
  shared headers split item by item; select the foundational modules and their members
  afresh, with their lifecycle roles. Do not carry over the subsystems, their composition,
  the `raw-lifetime-void` and `raw-lifetime-string` tiers, the order or the objectives
  of `subsystems.json`: use them, and the v2 schedules, only as evidence to check against;
- plan every module's waves and batches, the items that have already landed included,
  lazily as the Wrap waves section describes, every wave `todo` with empty `commits`,
  each followed by its review wave (review is enabled after each wave). Their
  `depends_on` name only waves of this re-plan, never an executed wave;
- write each item's `home` as the conventions place it, `src/libgit2/<TU path>.rs`, not
  where the landed tree has it today: the orchestrator re-homes the landed tree to your
  modules once your plan is promoted;
- every module's `status` is `todo`; the orchestrator freezes those the landed work
  completes once your plan is promoted.

The four bundled dependencies under `deps/` (`llhttp`, `ntlmclient`, `xdiff`,
`reftable`) are compiled into `libgit2.so`: decompose them like any other TU, and schedule
what the public API's consumers need of them, if anything, as the Wrap waves section
describes.

The wavefront configurations, T3 tables and ownership store on your branch were written
by run 1 with the checkout's root as wavefront's `repo_root`. Rewrite the configurations
for `repo_root` `c/`, with `state_dir` `../crustify/wavefront`, and regenerate the T3
tables and the DAGs from the CodeQL tables the orchestrator has just re-extracted under
`crustify/wavefront/codeql/` from a trace of `c/`. Keep a finding of the ownership store
only once you have checked it against the source.

## 2. The executed waves (departure)

`crustify/artifacts/planner/run-2/executed-waves.json` lists the 54 waves that ran, with
the global indices the orchestrator gave them in the order they ran (0 to 53), as the
schedule schema's Earlier versions section describes, and their batches as they ran: each
batch's `kind`, `items` (with the homes of the time), its run dir, its landed `commits`
on the rewritten history, and, for a review batch, the batches it `reviews`. Fold them
into `schedule.json` first, as waves `0` to `53`, `done`, exactly as listed: their
indices, batches, `items`, `commits` and `reviews` are records, and the run dirs'
`task.json` files name them. For each, set:

- `link_unit` `libgit2`; `module`, the module of the re-decomposition that holds every
  item of the wave when one does, and `null` when its items span several modules, which a
  subsystem's wave usually does (departure: a wave of no module);
- `foundational`, whether that module is foundational, `false` for `null`;
- `depends_on`: `scheduled`, the wave that ran before it, and `cut` empty;
- `budgets`: the caps it was packed under, as the dashboard's Notes record them per
  subsystem.

Number the re-plan's waves from `54`. Then delete `subsystems.json` and the v2
schedules, as your role describes.

## 3. Out-of-tree libraries

The user has decided:

- `libc` is wrapped by the existing `crustify-rs/glibc-bindings`, a side campaign started
  for `crustify-rs/ffmpeg-bindings`; it is cloned at `/work/glibc-bindings`. Record what
  it already wraps and what this campaign's waves need from it;
- every other out-of-tree library the target links (zlib, OpenSSL's `libssl` and
  `libcrypto`, PCRE2, and any other the linker outputs show) gets a new side campaign,
  `crustify-rs/<source>-bindings`; none is adopted from crates.io. The crate already
  depends on the `libc` and `pcre2-sys` crates directly; record them under the libraries'
  `bindings`, as for any other candidate.

## 4. Reports

The re-plan replaces every earlier decision, so do not file a finding per changed entry.
File one report instead, `crustify/reviews/planner/modules/replan-from-scratch/report.md`,
for the user to read: how the current specs' plan of the landed work differs from what
ran. Compare, for the nine landed subsystems: their modules and foundational members
against the subsystems and raw-lifetime tiers; the order; the waves and batches the
re-plan gives their items against the executed waves, item by item where it moved; the
landed items the re-plan would not have scheduled (such as enums, which are never
wrapped, or wraps no consumer needs) and the items it schedules that never landed. Give
counts first, then the detail, and the commands that reproduce them.
