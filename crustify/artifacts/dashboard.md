# crustify — `libgit2 / libgit2`

Copy this template into the campaign checkout, then fill it as work lands.

## Settings

As answered in `crustify/CRUSTIFY.md`; a change during the campaign is in its history
and in Notes.

- **source** — `/target` (libgit2) @ `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`
- **campaign** — `wrap`
- **scope** — the whole public API of `libgit2.so`, deprecated items excluded
- **translate-agent** — `openrouter/anthropic/claude-opus-5.5`, `api`
- **review-agent** — `openrouter/anthropic/claude-opus-5.5`, `api`; objectives `all`
- **plan-agent** — `anthropic/claude-opus-5-5`, `subscription`; `2` runs
- **workload** — max-types `2`, max-syms `25`, max-loc `500`, parallelism `16`
- **review-workload** — orchestrator's choice: max-types `20`, max-syms `100`
- **sanitizers** — `asan-ubsan`, `tsan`, `bsan`
- **target-config** — default, deprecated features disabled (`-DDEPRECATE_HARD=ON`), HTTPS through OpenSSL, SSH, Negotiate auth and i18n off, threads on; `build.json` version `1`

## Legend

- `subsystem` / `wave` / `batch` — the batch's place in its subsystem's `schedule.json`
- `objective` — `wrap` or `port`; `pass` — `first` for a translation batch, `review` for a
  review batch; `Σ first` / `Σ review` total each pass apart
- `types` / `symbols` / `fields` — the batch's scheduled units (callbacks count as
  symbols) and its types' assigned field accessors; a review row counts what it reviewed
- `$` / `tokens` / `wall` — the agent's computed cost (API-equivalent under
  subscription), its tokens in millions as `crustify usage` reports them, and its elapsed
  time, C rebuild included; `$/<unit>` divides `$` by the row's units
- `+LoC` — the landing's Rust insertions/deletions; `loc` — its `.rs` insertions
- `cov (C/Rust)` — line coverage after the landing, all test workloads in one run: C over
  the link unit's TUs, Rust over non-test sources; a `Σ` cell is the pass's last landing
- `+UB safe tests` … `+unit tests` — the tests the landing added, with its C/Rust coverage
  change in pp
- `+lifecycle` … `+misc`, `total` — the reports a review batch filed, by kind
- safety audit columns — the Safety audit's metrics, as a landing changed them
- `cost` / `review cost` / `total` — a subsystem's translation, review and combined cost;
  `subsystem wall` — first launch to final gate; `Σ recorded agents` leaves the assistant
  out, `Σ campaign` adds it, without per-unit cells
- `UB safe tests` … `Unit tests` — `#[test]` functions per suite, each with the coverage of
  running it alone; `Total tests` — all four, whose coverage is one joint run's union,
  never the sum
- `total`, `ported` and `wrapped` types and symbols — the scheduled public-API units, and
  those with a `Replaces:` or a `Wraps:` anchor at the tip; `share` is of the total

Every table below is a heading, a model line and the table. All prose belongs
in Notes.

## Overview

- **branch** — `crustify/main`, tip `<sha>`; crustify `0fcfcae`, ffibox `695d52d`, wavefront `<sha>`
- **C test baseline** — `3443/3498`, `55` skipped; disabled: ctest targets `invasive`, `gitdaemon`, `gitdaemon_namespace`, `gitdaemon_sha256`, `ssh`, `online`, `online_customcert`, `proxy`, `auth_clone`, `auth_clone_and_push`
- **Rust LoC** — `<n>` non-test, `<n>` tests; **C LoC** — `<n>`

### Coverage stats

| | count | share | C LoC coverage | Rust LoC coverage |
|---|---:|---:|---:|---:|
| UB safe tests | `752` | — | `35.80`% | `84.57`% |
| UB unsafe tests | `95` | — | `22.20`% | `12.70`% |
| Equiv tests | `464` | — | `43.73`% | `76.50`% |
| Unit tests | `712` | — | `32.13`% | `87.48`% |
| **Total tests** | **`2023`** | — | **`46.65`%** | **`99.55`%** |
| total types | `258` | — | — | — |
| ported types | `0` | `0.00`% | — | — |
| wrapped types | `96` | `37.21`% | — | — |
| total symbols | `979` | — | — | — |
| ported symbols | `4` | `0.41`% | — | — |
| wrapped symbols | `421` | `43.00`% | — | — |

### Subsystem stats

Implementation `openrouter/anthropic/claude-opus-5.5` via `claude`, `openrouter/openai/gpt-6.1-sol` via `codex` for `libgit2/config`, `openrouter/openai/gpt-6-astra` via `codex` for `libgit2/index` wave 0; review
`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0, `openrouter/anthropic/claude-opus-5.5` via `claude` from its review wave 1.

| subsystem | nr types | nr symbols | subsystem wall | cost | tokens | $/type | $/sym | review cost | review tokens | review $/type | review $/sym | total | total tokens | total $/type | total $/sym |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `libgit2/raw-lifetime-void` | `0` | `2` | `1h54m19s` | `$8.28` | `17.79M` | — | `$4.14` | `$7.89` | `10.78M` | — | `$3.94` | `$16.17` | `28.57M` | — | `$8.09` |
| `libgit2/raw-lifetime-string` | `0` | `2` | `52m30s` | `$5.49` | `12.39M` | — | `$2.75` | `$10.89` | `14.41M` | — | `$5.45` | `$16.38` | `26.80M` | — | `$8.19` |
| `libgit2/util` | `4` | `5` | `2h24m28s` | `$19.86` | `46.74M` | `$4.96` | `$3.97` | `$46.50` | `56.66M` | `$11.62` | `$9.30` | `$66.36` | `103.39M` | `$16.59` | `$13.27` |
| `libgit2/oid` | `4` | `19` | `3h14m42s` | `$27.25` | `66.34M` | `$6.81` | `$1.43` | `$48.59` | `57.98M` | `$12.15` | `$2.56` | `$75.84` | `124.32M` | `$18.96` | `$3.99` |
| `libgit2/core` | `16` | `12` | `5h06m51s` | `$96.92` | `235.87M` | `$6.06` | `$8.08` | `$70.82` | `85.91M` | `$4.43` | `$5.90` | `$167.74` | `321.78M` | `$10.48` | `$13.98` |
| `libgit2/repository` | `23` | `75` | `8h27m28s` | `$227.32` | `689.05M` | `$9.88` | `$3.03` | `$123.41` | `177.79M` | `$5.37` | `$1.65` | `$350.73` | `866.84M` | `$15.25` | `$4.68` |
| `libgit2/object` | `21` | `152` | `12h10m54s` | `$279.41` | `887.00M` | `$13.31` | `$1.84` | `$232.10` | `320.59M` | `$11.05` | `$1.53` | `$511.51` | `1207.59M` | `$24.36` | `$3.37` |
| `libgit2/config` | `9` | `47` | `6h09m05s` | `$36.16` | `129.17M` | `$4.02` | `$0.77` | `$94.07` | `265.11M` | `$10.45` | `$2.00` | `$130.23` | `394.28M` | `$14.47` | `$2.77` |
| `libgit2/index` | `18` | `75` | `10h41m03s` | `$195.65` | `307.26M` | `$10.87` | `$2.61` | `$67.64` | `186.28M` | `$3.76` | `$0.90` | `$263.29` | `493.54M` | `$14.63` | `$3.51` |
| planners | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| assistant | — | — | — | — | — | — | — | — | — | — | — | `$438.96` | `337.45M` | — | — |
| **Σ recorded agents** | **`95`** | **`389`** | **`51h01m`** | **`$896.34`** | **`2391.61M`** | **`$9.44`** | **`$2.30`** | **`$701.91`** | **`1175.51M`** | **`$7.39`** | **`$1.80`** | **`$1598.25`** | **`3567.12M`** | **`$16.82`** | **`$4.11`** |
| **Σ campaign** | **`95`** | **`389`** | **`51h01m`** | **`$896.34`** | **`2391.61M`** | — | — | **`$701.91`** | **`1175.51M`** | — | — | **`$2037.21`** | **`3904.57M`** | — | — |

## Safety audit

Deterministic `ffimap <workdir> --json`; no model. Every counter is in the `ffimap.json`
tracked in the campaign's artifact dir and in each subsystem's, wave's and batch's.

### Metrics

| metric | counters | what it counts |
|---|---|---|
| pub raw ptr (safe) | `raw_ptr_pub_safe_interface` | raw pointers on the safe public interface: public fields and safe public fns |
| pub raw ptr | `raw_ptr_pub` | raw pointers anywhere on the public interface, `unsafe fn`s included |
| pub void ptr | `void_ptr_ty_pub_fields_pub` + `void_ptr_fn_pub_safe` + `void_ptr_fn_pub_unsafe` | type-erased pointers left to the API's users |
| pub unsafe fn | `unsafe_fns_pub` | the obligations the crate exports to its callers |
| C borrow | `c_type_borrow_non_sanctioned` | Rust references into C memory outside the sanctioned handle positions; target 0 |
| slice borrow | `raw_ptr_as_slice_borrow` | slices and strings borrowed over raw pointers, which C must neither free nor change |
| unsafe loc % | `unsafe_loc_ratio` | unsafe block lines over code lines; context, not a score |
| unsafe blocks | `unsafe_blocks` | unsafe blocks, the denominator of the ratio's reading |
| Send/Sync | `unsafe_send_impl` + `unsafe_sync_impl` | thread safety asserted rather than derived from the fields |
| C by value | Σ `c_type_by_val_*` + Σ `c_type_std_owner_*` | C types held outside an ffibox owner, which a move can leave dangling |
| Drop/Clone | `drop_impl` + `clone_impl` | lifecycles written by hand rather than through an owner |
| deref outside impl | `raw_ptr_derefs_outside_impl` | raw pointer dereferences outside any `impl` or trait body |

## Planning

`anthropic/claude-opus-5-5` via `claude`. One row per planner run, in order; its reports are
the corrections it made to what earlier runs committed.

| run | subsystems changed | items moved | wavefront artifacts changed | reports | $ | tokens | wall |
|---|---:|---:|---|---:|---|---|---|
| 0 | `12` | `167` | `configs/wavefront-config.json`, `configs/libgit2/*/wavefront-config.json` (22), `t3/chased_casts.csv`, `t3/type_generators.csv` | `8` | `$13.36` | `37.11M` | `32m23s` |
| 1 | `<n>` | `<n>` | `<files>` | `<n>` | `$<n>` | `<n>M` | `<n>m<n>s` |
| **Σ** | **`12`** | **`167`** | | **`8`** | **`$13.36`** | **`37.11M`** | **`32m23s`** |

## Types

### Batches — wrap

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | pass | types | fields | +LoC | cov (C/Rust) | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/util` | `0` | `0` | first | `2` | `5` | `+1934/-2` | `1.30`% / `98.16`% | `$7.55` | `19.09M` | `33m43s` | `$3.77` | `$1.51` |
| `libgit2/util` | `0` | `1` | first | `2` | `0` | `+849/-26` | `1.35`% / `98.34`% | `$5.25` | `12.24M` | `38m51s` | `$2.62` | — |
| `libgit2/oid` | `0` | `0` | first | `2` | `2` | `+1514/-4` | `1.73`% / `99.82`% | `$9.89` | `28.93M` | `49m08s` | `$4.95` | `$4.95` |
| `libgit2/oid` | `0` | `1` | first | `1` | `0` | `+488/-0` | `1.68`% / `99.76`% | `$2.87` | `6.58M` | `14m31s` | `$2.87` | — |
| `libgit2/oid` | `1` | `0` | first | `1` | `2` | `+1397/-0` | `5.89`% / `100.00`% | `$5.30` | `12.40M` | `19m02s` | `$5.30` | `$2.65` |
| `libgit2/core` | `0` | `0` | first | `2` | `3` | `+2495/-315` | `15.24`% / `99.77`% | `$14.63` | `28.15M` | `54m01s` | `$7.32` | `$4.88` |
| `libgit2/core` | `0` | `1` | first | `2` | `0` | `+909/-0` | `9.22`% / `99.59`% | `$2.54` | `5.29M` | `16m38s` | `$1.27` | — |
| `libgit2/core` | `0` | `2` | first | `2` | `0` | `+1784/-6` | `12.15`% / `99.67`% | `$7.38` | `20.11M` | `28m17s` | `$3.69` | — |
| `libgit2/core` | `0` | `3` | first | `2` | `0` | `+871/-19` | `10.96`% / `99.64`% | `$5.25` | `13.05M` | `22m58s` | `$2.62` | — |
| `libgit2/core` | `0` | `4` | first | `2` | `2` | `+2575/-6` | `14.28`% / `99.74`% | `$12.07` | `30.57M` | `43m43s` | `$6.04` | `$6.04` |
| `libgit2/core` | `0` | `5` | first | `2` | `0` | `+839/-0` | `9.19`% / `99.52`% | `$3.05` | `6.68M` | `13m22s` | `$1.52` | — |
| `libgit2/core` | `0` | `6` | first | `2` | `3` | `+1134/-3` | `9.42`% / `99.62`% | `$4.22` | `10.43M` | `17m55s` | `$2.11` | `$1.41` |
| `libgit2/core` | `0` | `7` | first | `1` | `3` | `+2428/-3` | `13.87`% / `99.71`% | `$10.74` | `30.16M` | `33m33s` | `$10.74` | `$3.58` |
| `libgit2/core` | `1` | `0` | first | `1` | `3` | `+1745/-3` | `18.22`% / `99.82`% | `$10.23` | `31.28M` | `26m16s` | `$10.23` | `$3.41` |
| `libgit2/repository` | `0` | `0` | first | `1` | `0` | `+1799/-0` | `32.72`% / `99.69`% | `$24.34` | `54.96M` | `1h23m54s` | `$24.34` | — |
| `libgit2/repository` | `0` | `1` | first | `1` | `0` | `+3376/-0` | `31.33`% / `99.79`% | `$16.26` | `50.26M` | `55m56s` | `$16.26` | — |
| `libgit2/repository` | `0` | `2` | first | `2` | `0` | `+1925/-0` | `19.74`% / `99.86`% | `$8.59` | `24.90M` | `29m29s` | `$4.29` | — |
| `libgit2/repository` | `0` | `3` | first | `1` | `0` | `+1789/-0` | `25.32`% / `99.77`% | `$11.26` | `34.18M` | `42m24s` | `$11.26` | — |
| `libgit2/repository` | `0` | `4` | first | `2` | `0` | `+1635/-1` | `23.97`% / `99.79`% | `$11.67` | `36.52M` | `35m52s` | `$5.83` | — |
| `libgit2/repository` | `0` | `5` | first | `2` | `0` | `+1585/-0` | `23.69`% / `99.79`% | `$10.66` | `33.24M` | `32m15s` | `$5.33` | — |
| `libgit2/repository` | `0` | `6` | first | `2` | `0` | `+2089/-0` | `24.02`% / `99.77`% | `$10.91` | `33.42M` | `38m41s` | `$5.46` | — |
| `libgit2/repository` | `0` | `7` | first | `1` | `0` | `+1491/-194` | `25.76`% / `99.78`% | `$15.47` | `52.12M` | `51m17s` | `$15.47` | — |
| `libgit2/repository` | `0` | `8` | first | `2` | `0` | `+1279/-0` | `18.62`% / `99.83`% | `$4.27` | `9.83M` | `18m41s` | `$2.13` | — |
| `libgit2/repository` | `0` | `9` | first | `2` | `0` | `+1516/-0` | `19.22`% / `99.85`% | `$6.34` | `17.05M` | `27m40s` | `$3.17` | — |
| `libgit2/repository` | `0` | `10` | first | `2` | `0` | `+1587/-1` | `25.67`% / `99.77`% | `$7.92` | `22.15M` | `45m21s` | `$3.96` | — |
| `libgit2/repository` | `0` | `11` | first | `2` | `2` | `+1545/-0` | `18.98`% / `99.84`% | `$5.41` | `12.69M` | `23m56s` | `$2.71` | `$2.71` |
| `libgit2/repository` | `1` | `0` | first | `2` | `12` | `+2748/-28` | `33.79`% / `99.72`% | `$10.14` | `30.40M` | `28m55s` | `$5.07` | `$0.85` |
| `libgit2/repository` | `1` | `1` | first | `1` | `5` | `+1563/-3` | `33.24`% / `99.70`% | `$6.36` | `17.21M` | `20m02s` | `$6.36` | `$1.27` |
| `libgit2/object` | `0` | `0` | first | `2` | `2` | `+1681/-3` | `36.70`% / `99.33`% | `$6.21` | `16.56M` | `31m12s` | `$3.10` | `$3.10` |
| `libgit2/object` | `0` | `1` | first | `2` | `0` | `+2706/-1` | `37.91`% / `99.24`% | `$17.37` | `56.17M` | `54m52s` | `$8.69` | — |
| `libgit2/object` | `0` | `2` | first | `2` | `2` | `+2491/-0` | `37.67`% / `99.30`% | `$10.57` | `25.58M` | `47m19s` | `$5.29` | `$5.29` |
| `libgit2/object` | `0` | `3` | first | `2` | `0` | `+2509/-3` | `36.94`% / `99.30`% | `$14.40` | `45.71M` | `42m11s` | `$7.20` | — |
| `libgit2/object` | `0` | `4` | first | `2` | `0` | `+1791/-0` | `36.85`% / `99.31`% | `$8.08` | `24.57M` | `32m20s` | `$4.04` | — |
| `libgit2/object` | `0` | `5` | first | `1` | `0` | `+620/-0` | `36.19`% / `99.32`% | `$2.46` | `5.55M` | `13m29s` | `$2.46` | — |
| `libgit2/object` | `1` | `0` | first | `2` | `4` | `+2198/-10` | `38.08`% / `99.28`% | `$8.50` | `24.63M` | `27m01s` | `$4.25` | `$2.12` |
| `libgit2/object` | `1` | `1` | first | `2` | `7` | `+3634/-350` | `38.27`% / `99.33`% | `$15.08` | `46.58M` | `38m26s` | `$7.54` | `$2.15` |
| `libgit2/object` | `1` | `2` | first | `2` | `7` | `+2338/-49` | `38.16`% / `99.28`% | `$10.87` | `30.98M` | `36m44s` | `$5.43` | `$1.55` |
| `libgit2/object` | `1` | `3` | first | `2` | `0` | `+3188/-330` | `38.25`% / `99.32`% | `$17.37` | `57.50M` | `42m30s` | `$8.69` | — |
| `libgit2/object` | `1` | `4` | first | `1` | `4` | `+1169/-9` | `38.05`% / `99.28`% | `$4.42` | `11.16M` | `17m24s` | `$4.42` | `$1.10` |
| `libgit2/object` | `2` | `0` | first | `1` | `9` | `+1784/-9` | `39.72`% / `99.39`% | `$6.63` | `17.54M` | `28m41s` | `$6.63` | `$0.74` |
| `libgit2/config` | `0` | `0` | first | `2` | `3` | `+536/-0` | `42.24`% / `99.44`% | `$1.54` | `5.82M` | `11m22s` | `$0.77` | `$0.51` |
| `libgit2/config` | `0` | `1` | first | `2` | `0` | `+743/-1` | `42.52`% / `99.41`% | `$2.54` | `10.00M` | `24m29s` | `$1.27` | — |
| `libgit2/config` | `1` | `0` | first | `2` | `9` | `+772/-2` | `42.53`% / `99.43`% | `$1.86` | `7.12M` | `14m33s` | `$0.93` | `$0.21` |
| `libgit2/config` | `2` | `0` | first | `1` | `2` | `+487/-1` | `42.74`% / `99.44`% | `$1.67` | `5.71M` | `16m30s` | `$1.67` | `$0.83` |
| `libgit2/config` | `3` | `0` | first | `1` | `4` | `+1131/-24` | `42.79`% / `99.45`% | `$1.95` | `6.79M` | `19m01s` | `$1.95` | `$0.49` |
| `libgit2/config` | `4` | `0` | first | `1` | `14` | `+1118/-35` | `43.11`% / `99.45`% | `$2.49` | `9.45M` | `24m50s` | `$2.49` | `$0.18` |
| `libgit2/config` | `4-followup` | `0` | first | `1` | `14` | `+2746/-5` | `42.84`% / `99.46`% | `$11.63` | `34.73M` | `39m04s` | `$11.63` | `$0.83` |
| `libgit2/index` | `0` | `0` | first | `2` | `0` | `+1340/-43` | `45.00`% / `99.50`% | `$20.29` | `12.38M` | `27m57s` | `$10.14` | — |
| `libgit2/index` | `0` | `1` | first | `2` | `0` | `+431/-0` | `42.82`% / `99.49`% | `$5.76` | `2.63M` | `7m04s` | `$2.88` | — |
| `libgit2/index` | `0` | `2` | first | `2` | `0` | `+522/-0` | `44.19`% / `99.50`% | `$16.64` | `10.75M` | `19m17s` | `$8.32` | — |
| `libgit2/index` | `0` | `3` | first | `2` | `0` | `+402/-5` | `44.19`% / `99.50`% | `$13.75` | `8.80M` | `14m41s` | `$6.88` | — |
| `libgit2/index` | `0` | `4` | first | `2` | `3` | `+543/-0` | `44.14`% / `99.50`% | `$9.70` | `5.50M` | `12m25s` | `$4.85` | `$3.23` |
| `libgit2/index` | `0` | `5` | first | `2` | `2` | `+914/-1` | `45.58`% / `99.51`% | `$12.84` | `19.71M` | `36m20s` | `$6.42` | `$6.42` |
| `libgit2/index` | `0` | `6` | first | `2` | `0` | `+889/-520` | `45.46`% / `99.50`% | `$16.15` | `16.46M` | `36m16s` | `$8.07` | — |
| `libgit2/index` | `0` | `7` | first | `1` | `0` | `+1226/-1` | `45.43`% / `99.50`% | `$21.81` | `13.93M` | `40m51s` | `$21.81` | — |
| `libgit2/index` | `1` | `0` | first | `2` | `14` | `+1543/-14` | `45.80`% / `99.53`% | `$5.84` | `14.76M` | `23m16s` | `$2.92` | `$0.42` |
| `libgit2/index` | `1` | `1` | first | `1` | `3` | `+806/-5` | `45.85`% / `99.54`% | `$6.21` | `17.23M` | `25m45s` | `$6.21` | `$2.07` |
| `libgit2/util` | `0` | `0` | review | `2` | `5` | `+172/-15` | `1.63`% / `99.74`% | `$20.66` | `23.31M` | `48m16s` | `$10.33` | `$4.13` |
| `libgit2/util` | `0` | `1` | review | `2` | `0` | `+149/-12` | `1.63`% / `98.42`% | `$7.58` | `10.10M` | `18m18s` | `$3.79` | — |
| `libgit2/oid` | `0` | `0` | review | `2` | `2` | `+224/-6` | `7.81`% / `99.48`% | `$14.93` | `18.05M` | `29m10s` | `$7.46` | `$7.46` |
| `libgit2/oid` | `0` | `1` | review | `1` | `0` | `+185/-8` | `7.81`% / `99.47`% | `$7.16` | `8.57M` | `19m44s` | `$7.16` | — |
| `libgit2/oid` | `1` | `0` | review | `1` | `2` | `+235/-18` | `7.80`% / `99.49`% | `$11.49` | `15.40M` | `22m41s` | `$11.49` | `$5.75` |
| `libgit2/core` | `0` | `0` | review | `15` | `11` | `+263/-9` | `15.84`% / `99.81`% | `$24.78` | `30.19M` | `43m19s` | `$1.65` | `$2.25` |
| `libgit2/core` | `1` | `0` | review | `1` | `3` | `+126/-7` | `18.23`% / `99.82`% | `$18.99` | `21.31M` | `39m44s` | `$18.99` | `$6.33` |
| `libgit2/repository` | `0` | `0` | review | `20` | `2` | `+743/-98` | `32.73`% / `99.69`% | `$32.90` | `55.40M` | `46m45s` | `$1.65` | `$16.45` |
| `libgit2/repository` | `1` | `0` | review | `3` | `17` | `+227/-11` | `36.05`% / `99.30`% | `$21.87` | `26.76M` | `41m52s` | `$7.29` | `$1.29` |
| `libgit2/object` | `0` | `0` | review | `11` | `4` | `+1767/-853` | `37.98`% / `99.26`% | `$128.50` | `105.82M` | `3h16m09s` | `$11.68` | `$32.12` |
| `libgit2/object` | `1` | `0` | review | `9` | `22` | `+789/-37` | `39.49`% / `99.37`% | `$12.34` | `40.09M` | `30m23s` | `$1.37` | `$0.56` |
| `libgit2/object` | `2` | `0` | review | `1` | `9` | `+179/-26` | `42.18`% / `99.43`% | `$4.94` | `13.02M` | `15m40s` | `$4.94` | `$0.55` |
| `libgit2/config` | `0` | `0` | review | `4` | `3` | `+646/-125` | `42.52`% / `99.42`% | `$7.75` | `21.91M` | `42m25s` | `$1.94` | `$2.58` |
| `libgit2/config` | `1` | `0` | review | `2` | `9` | `+796/-17` | `42.72`% / `99.44`% | `$14.18` | `43.14M` | `49m58s` | `$7.09` | `$1.58` |
| `libgit2/config` | `2` | `0` | review | `1` | `2` | `+366/-151` | `42.75`% / `99.46`% | `$6.29` | `17.40M` | `25m11s` | `$6.29` | `$3.15` |
| `libgit2/config` | `3` | `0` | review | `1` | `4` | `+627/-239` | `42.82`% / `99.45`% | `$8.39` | `24.83M` | `28m21s` | `$8.39` | `$2.10` |
| `libgit2/config` | `4` | `0` | review | `1` | `14` | `+631/-286` | `43.14`% / `99.45`% | `$8.66` | `25.44M` | `27m14s` | `$8.66` | `$0.62` |
| `libgit2/config` | `4-followup` | `0` | review | `1` | `14` | `+198/-26` | `42.79`% / `99.49`% | `$8.96` | `25.80M` | `26m55s` | `$8.96` | `$0.64` |
| `libgit2/index` | `0` | `0` | review | `15` | `5` | `+738/-79` | `45.75`% / `99.53`% | `$15.85` | `50.35M` | `46m20s` | `$1.06` | `$3.17` |
| `libgit2/index` | `1` | `0` | review | `3` | `17` | `+203/-8` | `46.57`% / `99.53`% | `$12.82` | `35.96M` | `48m41s` | `$4.27` | `$0.75` |
| **Σ first** |  |  |  | **`96`** | **`145`** | **`+89147/-2035`** | **`45.85`% / `99.54`%** | **`$525.20`** | **`1248.21M`** | — | **`$5.47`** | **`$3.62`** |
| **Σ review** |  |  |  | **`96`** | **`145`** | **`+9264/-2031`** | **`46.57`% / `99.53`%** | **`$389.04`** | **`612.85M`** | — | **`$4.05`** | **`$2.68`** |

### Batches — wrap: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `libgit2/util` | `0` | `0` | first | `+4` (`+0.02`/`-6.85` pp) | `+7` (`+0.00`/`-10.50` pp) | `+2` (`+0.00`/`-10.62` pp) | `+14` (`+0.04`/`+6.71` pp) |
| `libgit2/util` | `0` | `1` | first | `+4` (`+0.00`/`-3.94` pp) | `+0` (`+0.00`/`-5.56` pp) | `+6` (`+0.05`/`-2.60` pp) | `+10` (`+0.00`/`+1.42` pp) |
| `libgit2/oid` | `0` | `0` | first | `+11` (`+0.09`/`-0.22` pp) | `+0` (`+0.00`/`-10.49` pp) | `+6` (`+0.04`/`-4.69` pp) | `+11` (`+0.09`/`+1.74` pp) |
| `libgit2/oid` | `0` | `1` | first | `+4` (`+0.00`/`+0.27` pp) | `+0` (`+0.00`/`-4.15` pp) | `+7` (`+0.06`/`+0.74` pp) | `+5` (`+0.00`/`+0.69` pp) |
| `libgit2/oid` | `1` | `0` | first | `+8` (`+0.01`/`+2.05` pp) | `+0` (`+0.00`/`-4.51` pp) | `+2` (`+4.15`/`+1.38` pp) | `+8` (`+0.00`/`+1.07` pp) |
| `libgit2/core` | `0` | `0` | first | `+11` (`+0.12`/`+1.49` pp) | `+5` (`+0.00`/`+4.15` pp) | `+4` (`+0.96`/`+0.37` pp) | `+12` (`+0.12`/`+1.23` pp) |
| `libgit2/core` | `0` | `1` | first | `+5` (`+0.00`/`-2.56` pp) | `+0` (`+0.00`/`-3.05` pp) | `+5` (`+0.03`/`-1.89` pp) | `+7` (`+0.00`/`+1.30` pp) |
| `libgit2/core` | `0` | `2` | first | `+7` (`+0.00`/`+0.47` pp) | `+0` (`+0.00`/`-1.44` pp) | `+10` (`+1.19`/`+0.76` pp) | `+9` (`+0.00`/`+0.61` pp) |
| `libgit2/core` | `0` | `3` | first | `+5` (`+0.00`/`+0.32` pp) | `+0` (`+0.00`/`-0.78` pp) | `+4` (`+1.54`/`-0.56` pp) | `+8` (`+0.00`/`+0.33` pp) |
| `libgit2/core` | `0` | `4` | first | `+13` (`+0.12`/`+1.50` pp) | `+0` (`+0.00`/`-1.38` pp) | `+6` (`+0.04`/`-0.06` pp) | `+18` (`+0.52`/`+0.59` pp) |
| `libgit2/core` | `0` | `5` | first | `+4` (`+0.00`/`-0.59` pp) | `+0` (`+0.00`/`-1.52` pp) | `+7` (`+1.39`/`+0.31` pp) | `+3` (`+0.00`/`+0.66` pp) |
| `libgit2/core` | `0` | `6` | first | `+10` (`+0.00`/`+0.88` pp) | `+0` (`+0.00`/`-1.74` pp) | `+9` (`+0.20`/`-0.42` pp) | `+13` (`+0.00`/`+0.75` pp) |
| `libgit2/core` | `0` | `7` | first | `+18` (`+7.17`/`+0.31` pp) | `+0` (`+0.00`/`-2.03` pp) | `+4` (`+1.70`/`-1.93` pp) | `+14` (`+0.04`/`+0.88` pp) |
| `libgit2/core` | `1` | `0` | first | `+11` (`+0.21`/`+1.00` pp) | `+0` (`+0.00`/`-2.05` pp) | `+6` (`+0.02`/`-0.08` pp) | `+8` (`+0.18`/`+0.21` pp) |
| `libgit2/repository` | `0` | `0` | first | `+7` (`+2.18`/`+0.15` pp) | `+0` (`+0.03`/`-0.90` pp) | `+2` (`+1.08`/`+0.08` pp) | `+5` (`+0.42`/`-0.11` pp) |
| `libgit2/repository` | `0` | `1` | first | `+8` (`+3.49`/`+0.11` pp) | `+12` (`+6.85`/`+0.54` pp) | `+4` (`+5.84`/`+0.48` pp) | `+15` (`+0.00`/`+0.92` pp) |
| `libgit2/repository` | `0` | `2` | first | `+9` (`+2.35`/`+0.41` pp) | `+3` (`+5.08`/`+0.94` pp) | `+5` (`+0.52`/`+0.21` pp) | `+11` (`+2.64`/`+0.37` pp) |
| `libgit2/repository` | `0` | `3` | first | `+15` (`+1.38`/`+0.16` pp) | `+0` (`+0.01`/`-0.26` pp) | `+7` (`+1.28`/`-0.12` pp) | `+6` (`+1.17`/`+0.13` pp) |
| `libgit2/repository` | `0` | `4` | first | `+8` (`+0.37`/`+0.13` pp) | `+0` (`+0.01`/`-0.63` pp) | `+9` (`+0.27`/`+0.16` pp) | `+10` (`+0.06`/`+0.24` pp) |
| `libgit2/repository` | `0` | `5` | first | `+7` (`+0.03`/`-0.34` pp) | `+4` (`+3.44`/`+0.38` pp) | `+5` (`+3.92`/`-0.43` pp) | `+10` (`+3.50`/`+0.28` pp) |
| `libgit2/repository` | `0` | `6` | first | `+14` (`+0.19`/`-0.08` pp) | `+0` (`-0.01`/`-0.82` pp) | `+5` (`+0.06`/`-0.90` pp) | `+12` (`+0.07`/`+0.37` pp) |
| `libgit2/repository` | `0` | `7` | first | `+7` (`+0.34`/`-0.46` pp) | `+0` (`+0.02`/`-0.85` pp) | `+3` (`+0.07`/`-0.58` pp) | `+10` (`+0.10`/`+0.29` pp) |
| `libgit2/repository` | `0` | `8` | first | `+5` (`-0.01`/`-0.75` pp) | `+0` (`+0.01`/`-1.76` pp) | `+6` (`+0.39`/`-0.12` pp) | `+6` (`+0.00`/`+0.63` pp) |
| `libgit2/repository` | `0` | `9` | first | `+6` (`+0.01`/`-0.71` pp) | `+0` (`-0.01`/`-1.35` pp) | `+9` (`+0.24`/`-0.29` pp) | `+8` (`+0.00`/`+0.72` pp) |
| `libgit2/repository` | `0` | `10` | first | `+10` (`+0.32`/`+0.16` pp) | `+0` (`+0.00`/`-0.31` pp) | `+4` (`+0.31`/`-0.04` pp) | `+11` (`+0.30`/`+0.14` pp) |
| `libgit2/repository` | `0` | `11` | first | `+13` (`+0.01`/`+0.60` pp) | `+0` (`+0.00`/`-1.97` pp) | `+5` (`+0.37`/`-1.79` pp) | `+9` (`+0.01`/`+0.38` pp) |
| `libgit2/repository` | `1` | `0` | first | `+24` (`+0.94`/`+1.58` pp) | `+0` (`+0.01`/`-1.26` pp) | `+6` (`+0.55`/`-0.31` pp) | `+12` (`+0.02`/`+0.43` pp) |
| `libgit2/repository` | `1` | `1` | first | `+10` (`+0.88`/`+0.36` pp) | `+0` (`+0.23`/`-0.57` pp) | `+3` (`+0.31`/`+0.32` pp) | `+7` (`+0.00`/`+0.27` pp) |
| `libgit2/object` | `0` | `0` | first | `+9` (`+0.02`/`+0.08` pp) | `+0` (`+0.01`/`-0.69` pp) | `+5` (`+0.43`/`-0.80` pp) | `+9` (`+0.01`/`+0.48` pp) |
| `libgit2/object` | `0` | `1` | first | `+7` (`+0.46`/`-0.14` pp) | `+6` (`+0.33`/`+1.37` pp) | `+6` (`+0.19`/`+0.10` pp) | `+8` (`+1.01`/`-0.18` pp) |
| `libgit2/object` | `0` | `2` | first | `+9` (`+0.58`/`+0.17` pp) | `+2` (`+0.56`/`+0.11` pp) | `+7` (`+0.71`/`+0.09` pp) | `+20` (`+0.62`/`+0.41` pp) |
| `libgit2/object` | `0` | `3` | first | `+15` (`+0.07`/`+0.15` pp) | `+0` (`-0.26`/`-0.44` pp) | `+4` (`+0.28`/`-0.16` pp) | `+10` (`+0.38`/`+0.25` pp) |
| `libgit2/object` | `0` | `4` | first | `+6` (`+0.06`/`-0.59` pp) | `+3` (`+0.22`/`+0.91` pp) | `+4` (`+0.13`/`-0.34` pp) | `+4` (`+0.14`/`-0.35` pp) |
| `libgit2/object` | `0` | `5` | first | `+3` (`+0.00`/`+0.02` pp) | `+0` (`-0.05`/`-0.09` pp) | `+3` (`+0.04`/`+0.02` pp) | `+3` (`-0.01`/`+0.06` pp) |
| `libgit2/object` | `1` | `0` | first | `+10` (`+0.39`/`-0.07` pp) | `+0` (`+0.10`/`-0.53` pp) | `+6` (`+0.01`/`-0.21` pp) | `+8` (`+0.01`/`+0.34` pp) |
| `libgit2/object` | `1` | `1` | first | `+9` (`+0.47`/`-0.03` pp) | `+2` (`+0.22`/`+0.89` pp) | `+5` (`+0.10`/`+0.09` pp) | `+9` (`+0.03`/`+0.55` pp) |
| `libgit2/object` | `1` | `2` | first | `+7` (`+0.10`/`+0.42` pp) | `+4` (`-0.04`/`+0.71` pp) | `+5` (`+0.09`/`+0.20` pp) | `+9` (`+0.02`/`+0.28` pp) |
| `libgit2/object` | `1` | `3` | first | `+22` (`+0.19`/`+0.02` pp) | `+0` (`-0.34`/`-0.61` pp) | `+6` (`+0.04`/`-0.33` pp) | `+12` (`+0.05`/`+0.32` pp) |
| `libgit2/object` | `1` | `4` | first | `+5` (`-0.02`/`+0.49` pp) | `+0` (`-0.02`/`-0.37` pp) | `+1` (`+0.09`/`-0.18` pp) | `+6` (`+0.02`/`+0.28` pp) |
| `libgit2/object` | `2` | `0` | first | `+6` (`+0.28`/`+0.25` pp) | `+0` (`+0.27`/`-0.24` pp) | `+2` (`+0.22`/`+0.04` pp) | `+4` (`+0.27`/`+0.18` pp) |
| `libgit2/config` | `0` | `0` | first | `+2` (`+0.02`/`-0.01` pp) | `+0` (`-0.07`/`-0.12` pp) | `+2` (`-0.03`/`-0.04` pp) | `+3` (`-0.01`/`-0.05` pp) |
| `libgit2/config` | `0` | `1` | first | `+3` (`+0.09`/`-0.06` pp) | `+0` (`+0.38`/`-0.13` pp) | `+3` (`+0.25`/`-0.22` pp) | `+4` (`+0.14`/`-0.02` pp) |
| `libgit2/config` | `1` | `0` | first | `+2` (`+0.04`/`+0.16` pp) | `+0` (`+0.31`/`-0.16` pp) | `+2` (`+0.07`/`+0.17` pp) | `+2` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `2` | `0` | first | `+1` (`-0.04`/`+0.01` pp) | `+0` (`-0.49`/`-0.04` pp) | `+1` (`-0.02`/`+0.04` pp) | `+2` (`+0.01`/`+0.04` pp) |
| `libgit2/config` | `3` | `0` | first | `+4` (`-0.01`/`+0.06` pp) | `+1` (`+0.03`/`+0.48` pp) | `+1` (`+0.01`/`-0.06` pp) | `+3` (`+0.02`/`-0.03` pp) |
| `libgit2/config` | `4` | `0` | first | `+2` (`+0.32`/`-0.35` pp) | `+0` (`-0.55`/`-0.18` pp) | `+3` (`+0.25`/`-0.26` pp) | `+5` (`+0.14`/`-0.17` pp) |
| `libgit2/config` | `4-followup` | `0` | first | `+9` (`+0.03`/`+1.56` pp) | `+0` (`+0.22`/`-0.44` pp) | `+3` (`+0.01`/`+0.40` pp) | `+9` (`+0.00`/`+0.39` pp) |
| `libgit2/index` | `0` | `0` | first | `+4` (`+0.82`/`-0.36` pp) | `+1` (`+0.35`/`+2.04` pp) | `+3` (`+0.78`/`+0.00` pp) | `+3` (`+0.16`/`-0.92` pp) |
| `libgit2/index` | `0` | `1` | first | `+2` (`-0.03`/`+0.05` pp) | `+0` (`+0.01`/`-0.06` pp) | `+3` (`+0.02`/`-0.07` pp) | `+2` (`-0.01`/`-0.29` pp) |
| `libgit2/index` | `0` | `2` | first | `+2` (`-0.01`/`-0.11` pp) | `+0` (`+0.19`/`-0.04` pp) | `+2` (`-0.02`/`-0.08` pp) | `+3` (`+0.04`/`+0.05` pp) |
| `libgit2/index` | `0` | `3` | first | `+2` (`+0.05`/`-0.08` pp) | `+0` (`-0.21`/`-0.04` pp) | `+2` (`+0.05`/`-0.05` pp) | `+2` (`+0.02`/`+0.03` pp) |
| `libgit2/index` | `0` | `4` | first | `+3` (`+0.05`/`+0.06` pp) | `+0` (`+0.02`/`-0.06` pp) | `+2` (`+0.08`/`+0.10` pp) | `+4` (`+0.06`/`+0.00` pp) |
| `libgit2/index` | `0` | `5` | first | `+5` (`+0.02`/`-0.03` pp) | `+0` (`-0.11`/`-0.10` pp) | `+3` (`+0.10`/`-0.03` pp) | `+5` (`+0.03`/`+0.11` pp) |
| `libgit2/index` | `0` | `6` | first | `+6` (`+0.19`/`+0.33` pp) | `+0` (`-1.22`/`-1.08` pp) | `+2` (`+0.06`/`-0.07` pp) | `+4` (`+0.03`/`-0.05` pp) |
| `libgit2/index` | `0` | `7` | first | `+2` (`+0.35`/`-0.35` pp) | `+5` (`+1.23`/`+0.91` pp) | `+3` (`+0.45`/`+0.14` pp) | `+2` (`+0.13`/`-0.35` pp) |
| `libgit2/index` | `1` | `0` | first | `+6` (`+0.02`/`+0.20` pp) | `+0` (`+0.18`/`-0.27` pp) | `+2` (`+0.02`/`-0.73` pp) | `+4` (`+0.00`/`-0.17` pp) |
| `libgit2/index` | `1` | `1` | first | `+4` (`+0.12`/`+0.00` pp) | `+0` (`-0.16`/`-0.06` pp) | `+2` (`+0.13`/`+0.03` pp) | `+2` (`+0.02`/`-0.04` pp) |
| `libgit2/util` | `0` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+2` (`+0.00`/`+0.00` pp) |
| `libgit2/util` | `0` | `1` | review | `+0` (`+0.00`/`-0.44` pp) | `+0` (`+0.00`/`-0.79` pp) | `+1` (`+0.27`/`-0.49` pp) | `+1` (`+0.00`/`+0.20` pp) |
| `libgit2/oid` | `0` | `0` | review | `+1` (`+0.00`/`+0.32` pp) | `+0` (`+0.00`/`-0.39` pp) | `+1` (`+0.00`/`-0.61` pp) | `+4` (`+0.00`/`+0.57` pp) |
| `libgit2/oid` | `0` | `1` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+1.74`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/oid` | `1` | `0` | review | `+2` (`+0.00`/`+2.43` pp) | `+0` (`+0.00`/`-0.47` pp) | `+0` (`+0.00`/`+0.39` pp) | `+2` (`+4.32`/`+0.20` pp) |
| `libgit2/core` | `0` | `0` | review | `+0` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+2` (`+0.32`/`+0.15` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `1` | `0` | review | `+1` (`-0.02`/`-0.06` pp) | `+0` (`+0.00`/`-0.04` pp) | `+0` (`-0.01`/`-0.06` pp) | `+1` (`+0.01`/`+0.07` pp) |
| `libgit2/repository` | `0` | `0` | review | `+2` (`+0.24`/`-0.34` pp) | `+0` (`-0.32`/`-0.48` pp) | `+3` (`+0.02`/`-0.30` pp) | `+2` (`+0.00`/`+0.23` pp) |
| `libgit2/repository` | `1` | `0` | review | `+0` (`-0.01`/`+0.00` pp) | `+0` (`-0.28`/`+0.00` pp) | `+1` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/object` | `0` | `0` | review | `+15` (`+0.09`/`+0.79` pp) | `+0` (`-0.35`/`-3.27` pp) | `+2` (`+0.06`/`+0.35` pp) | `+2` (`+0.07`/`-0.03` pp) |
| `libgit2/object` | `1` | `0` | review | `+3` (`-0.01`/`+0.05` pp) | `+0` (`-0.51`/`-0.06` pp) | `+1` (`+0.11`/`+0.10` pp) | `+1` (`+0.05`/`+0.08` pp) |
| `libgit2/object` | `2` | `0` | review | `+0` (`+0.02`/`+0.00` pp) | `+0` (`+0.49`/`+0.00` pp) | `+0` (`+0.05`/`+0.00` pp) | `+0` (`-0.02`/`+0.00` pp) |
| `libgit2/config` | `0` | `0` | review | `+2` (`+0.00`/`-0.42` pp) | `+0` (`+0.31`/`-0.08` pp) | `+1` (`-0.03`/`-0.27` pp) | `+3` (`+0.01`/`+0.11` pp) |
| `libgit2/config` | `1` | `0` | review | `+3` (`+0.00`/`-0.23` pp) | `+0` (`-0.49`/`-0.08` pp) | `+2` (`-0.01`/`-0.16` pp) | `+3` (`+0.05`/`+0.18` pp) |
| `libgit2/config` | `2` | `0` | review | `+1` (`-0.01`/`+0.06` pp) | `+0` (`+0.24`/`-0.01` pp) | `+1` (`+0.02`/`-0.09` pp) | `+2` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `3` | `0` | review | `+1` (`-0.01`/`-0.01` pp) | `+0` (`+0.06`/`-0.07` pp) | `+2` (`+0.01`/`-0.01` pp) | `+4` (`+0.00`/`+0.17` pp) |
| `libgit2/config` | `4` | `0` | review | `+1` (`+0.02`/`+0.14` pp) | `+0` (`+0.37`/`-0.01` pp) | `+2` (`+0.02`/`-0.14` pp) | `+7` (`+0.09`/`+0.40` pp) |
| `libgit2/config` | `4-followup` | `0` | review | `+0` (`-0.02`/`+0.00` pp) | `+0` (`-0.29`/`-0.01` pp) | `+1` (`-0.04`/`-0.03` pp) | `+1` (`-0.02`/`+0.03` pp) |
| `libgit2/index` | `0` | `0` | review | `+1` (`+0.08`/`-0.09` pp) | `+2` (`+0.33`/`+0.40` pp) | `+1` (`+0.18`/`-0.04` pp) | `+2` (`+0.00`/`+0.09` pp) |
| `libgit2/index` | `1` | `0` | review | `+1` (`-0.01`/`+0.00` pp) | `+0` (`+0.04`/`+0.00` pp) | `+0` (`-0.01`/`+0.00` pp) | `+2` (`+0.10`/`+0.05` pp) |
| **Σ first** |  |  |  | **`+425` (`+24.85`/`-2.70` pp)** | **`+55` (`+16.78`/`-52.20` pp)** | **`+244` (`+31.12`/`-24.83` pp)** | **`+433` (`+12.61`/`+24.41` pp)** |
| **Σ review** |  |  |  | **`+34` (`+0.37`/`+2.20` pp)** | **`+3` (`-0.40`/`-5.36` pp)** | **`+22` (`+2.71`/`-1.21` pp)** | **`+39` (`+4.66`/`+2.36` pp)** |

### Batches — port

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | pass | types | fields | +LoC | cov (C/Rust) | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `<link-unit>/<subsystem>` | `<n>` | `<n>` | first | `<n>` | `<n>` | `+<n>/-<n>` | `<n>`% / `<n>`% | `$<n>` | `<n>M` | `<n>m<n>s` | `$<n>` | `$<n>` |
| `<link-unit>/<subsystem>` | `<n>` | `<n>` | review | `<n>` | `<n>` | `+<n>/-<n>` | `<n>`% / `<n>`% | `$<n>` | `<n>M` | `<n>m<n>s` | `$<n>` | `$<n>` |
| **Σ first** |  |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`$<n>`** | **`<n>M`** | — | **`$<n>`** | **`$<n>`** |
| **Σ review** |  |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`$<n>`** | **`<n>M`** | — | **`$<n>`** | **`$<n>`** |

### Batches — port: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `<link-unit>/<subsystem>` | `<n>` | `<n>` | first | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) |
| `<link-unit>/<subsystem>` | `<n>` | `<n>` | review | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) |
| **Σ first** |  |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |
| **Σ review** |  |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |

### Batches: safety audit

Deterministic `ffimap`; no model.

| subsystem | wave | batch | objective | pass | pub raw ptr (safe) | pub raw ptr | pub void ptr | pub unsafe fn | C borrow | slice borrow | unsafe loc % | unsafe blocks | Send/Sync | C by value | Drop/Clone | deref outside impl |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/util` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.06` pp | `+24` | — | — | `+0` | `+0` |
| `libgit2/util` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-2.56` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.54` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/oid` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.98` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/oid` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+2.27` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.39` pp | `+20` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-1.93` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.97` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.47` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `4` | wrap | first | `+0` | `+1` | `+0` | `+0` | — | — | `+0.44` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-1.02` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.29` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `7` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.35` pp | `+26` | — | — | `+1` | `+3` |
| `libgit2/core` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.59` pp | `+32` | — | — | `+1` | `+2` |
| `libgit2/repository` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.30` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `1` | wrap | first | `+4` | `+8` | `+6` | `+4` | — | — | `+1.07` pp | `+46` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `2` | wrap | first | `+1` | `+1` | `+0` | `+1` | — | — | `+0.39` pp | `+20` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `5` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `-0.15` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+15` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `7` | wrap | first | `+0` | `+0` | `+0` | `-1` | — | — | `+0.06` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `8` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.79` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `9` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.72` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `10` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `11` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.57` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/repository` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.27` pp | `+36` | — | — | `+0` | `+0` |
| `libgit2/repository` | `1` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+16` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.41` pp | `+9` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `-0.07` pp | `+18` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `2` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `-0.16` pp | `+18` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.16` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `4` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `-0.09` pp | `+9` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.08` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.17` pp | `+34` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `1` | wrap | first | `+0` | `+1` | `+2` | `+1` | — | — | `+0.33` pp | `+36` | — | — | `-1` | `+0` |
| `libgit2/object` | `1` | `2` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.14` pp | `+28` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `3` | wrap | first | `-1` | `-2` | `+0` | `-1` | — | — | `+0.03` pp | `+29` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.10` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/object` | `2` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.18` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/config` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+13` | — | — | `+1` | `+0` |
| `libgit2/config` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.12` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/config` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+28` | — | — | `+1` | `+0` |
| `libgit2/config` | `2` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/config` | `3` | `0` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.18` pp | `+37` | — | — | `+0` | `+2` |
| `libgit2/config` | `4` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.12` pp | `+40` | — | — | `+1` | `+0` |
| `libgit2/config` | `4-followup` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.19` pp | `+44` | — | — | `+1` | `+0` |
| `libgit2/index` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.53` pp | `+45` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.10` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.06` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.21` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+13` | — | — | `+1` | `+0` |
| `libgit2/index` | `0` | `6` | wrap | first | `+0` | `-4` | `+0` | `-4` | — | — | `+0.11` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `7` | wrap | first | `+0` | `+4` | `+0` | `+4` | — | — | `+0.06` pp | `+17` | — | — | `+1` | `+0` |
| `libgit2/index` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+40` | — | — | `+0` | `+0` |
| `libgit2/index` | `1` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.08` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/util` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/util` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/oid` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.18` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.37` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/repository` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.18` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/repository` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `-3` | — | — | `+0.09` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/object` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.09` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `3` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `-1` | — | — | `+0` | `-2` |
| `libgit2/config` | `4` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/config` | `4-followup` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `-1` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.03` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| **Σ first** |  |  |  |  | **`+4`** | **`+9`** | **`+8`** | **`+13`** | — | — | **`-0.67` pp** | **`+965`** | — | — | **`+7`** | **`+7`** |
| **Σ review** |  |  |  |  | **`+0`** | **`+0`** | **`+0`** | **`-3`** | — | — | **`-0.14` pp** | **`+20`** | — | — | **`+0`** | **`-2`** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `libgit2/util` | `0` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `0` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/oid` | `0` | `0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/oid` | `0` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/oid` | `1` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+1` | `3` |
| `libgit2/core` | `0` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `5` |
| `libgit2/core` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `2` |
| `libgit2/repository` | `0` | `0` | `+1` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `4` |
| `libgit2/repository` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/object` | `0` | `0` | `+2` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `8` |
| `libgit2/object` | `1` | `0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `8` |
| `libgit2/object` | `2` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/config` | `0` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+2` | `+4` | `7` |
| `libgit2/config` | `1` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+4` | `6` |
| `libgit2/config` | `2` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+2` | `4` |
| `libgit2/config` | `3` | `0` | `+0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+2` | `+1` | `5` |
| `libgit2/config` | `4` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+2` | `+3` | `7` |
| `libgit2/config` | `4-followup` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+3` | `4` |
| `libgit2/index` | `0` | `0` | `+0` | `+2` | `+0` | `+1` | `+0` | `+0` | `+2` | `+2` | `7` |
| `libgit2/index` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` |
| **Σ** |  |  | **`+6`** | **`+9`** | **`+2`** | **`+1`** | **`+3`** | **`+3`** | **`+14`** | **`+49`** | **`87`** |

## Symbols

### Batches

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | objective | pass | symbols | loc | +LoC | cov (C/Rust) | $ | tokens | wall | $/symbol | $/loc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | first | `2` | — | `+1245/-1` | — | `$8.28` | `17.79M` | `51m29s` | `$4.14` | — |
| `libgit2/raw-lifetime-string` | `0` | `0` | wrap | first | `2` | — | `+1362/-14` | — | `$5.49` | `12.39M` | `23m12s` | `$2.75` | — |
| `libgit2/util` | `0` | `2` | wrap | first | `4` | `51` | `+1008/-26` | `1.30`% / `97.32`% | `$4.00` | `8.72M` | `21m23s` | `$1.00` | `$0.08` |
| `libgit2/util` | `1` | `0` | wrap | first | `1` | `15` | `+756/-71` | `1.36`% / `98.40`% | `$3.06` | `6.69M` | `15m41s` | `$3.06` | `$0.20` |
| `libgit2/oid` | `1` | `1` | wrap | first | `18` | `252` | `+1351/-11` | `6.07`% / `99.47`% | `$8.55` | `17.50M` | `40m06s` | `$0.48` | `$0.03` |
| `libgit2/oid` | `2` | `0` | wrap | first | `1` | `3` | `+0/-0` | `6.07`% / `99.47`% | `$0.64` | `0.93M` | `4m17s` | `$0.64` | `$0.21` |
| `libgit2/core` | `0` | `8` | wrap | first | `6` | `444` | `+3504/-75` | `15.52`% / `99.81`% | `$19.90` | `44.40M` | `1h04m11s` | `$3.32` | `$0.04` |
| `libgit2/core` | `1` | `1` | wrap | first | `5` | `212` | `+1238/-4` | `18.20`% / `99.81`% | `$4.61` | `11.51M` | `19m37s` | `$0.92` | `$0.02` |
| `libgit2/core` | `2` | `0` | wrap | first | `1` | `9` | `+658/-41` | `18.24`% / `99.82`% | `$2.30` | `4.24M` | `10m40s` | `$2.30` | `$0.26` |
| `libgit2/repository` | `1` | `2` | wrap | first | `25` | `436` | `+3311/-54` | `36.06`% / `99.30`% | `$24.81` | `84.41M` | `1h01m00s` | `$0.99` | `$0.06` |
| `libgit2/repository` | `1` | `3` | wrap | first | `25` | `432` | `+3392/-29` | `35.39`% / `99.35`% | `$20.18` | `66.23M` | `53m42s` | `$0.81` | `$0.05` |
| `libgit2/repository` | `1` | `4` | wrap | first | `18` | `396` | `+3072/-105` | `34.12`% / `99.40`% | `$15.55` | `50.84M` | `45m11s` | `$0.86` | `$0.04` |
| `libgit2/repository` | `2` | `0` | wrap | first | `7` | `373` | `+1449/-382` | `36.17`% / `99.32`% | `$17.18` | `58.64M` | `32m30s` | `$2.45` | `$0.05` |
| `libgit2/object` | `0` | `6` | wrap | first | `2` | `28` | `+996/-0` | `36.26`% / `99.31`% | `$3.49` | `8.11M` | `27m05s` | `$1.75` | `$0.12` |
| `libgit2/object` | `1` | `5` | wrap | first | `25` | `455` | `+4438/-388` | `39.43`% / `99.39`% | `$34.78` | `128.15M` | `1h09m40s` | `$1.39` | `$0.08` |
| `libgit2/object` | `1` | `6` | wrap | first | `25` | `261` | `+3882/-94` | `39.17`% / `99.36`% | `$26.99` | `97.07M` | `1h03m06s` | `$1.08` | `$0.10` |
| `libgit2/object` | `1` | `7` | wrap | first | `5` | `51` | `+2334/-111` | `38.42`% / `99.33`% | `$13.81` | `38.98M` | `48m22s` | `$2.76` | `$0.27` |
| `libgit2/object` | `2` | `1` | wrap | first | `25` | `268` | `+5204/-170` | `42.10`% / `99.43`% | `$24.53` | `82.24M` | `1h11m24s` | `$0.98` | `$0.09` |
| `libgit2/object` | `2` | `2` | wrap | first | `25` | `350` | `+1923/-76` | `39.84`% / `99.37`% | `$11.68` | `35.74M` | `39m28s` | `$0.47` | `$0.03` |
| `libgit2/object` | `2` | `3` | wrap | first | `25` | `392` | `+4093/-32` | `40.44`% / `99.39`% | `$20.74` | `70.47M` | `1h00m04s` | `$0.83` | `$0.05` |
| `libgit2/object` | `2` | `4` | wrap | first | `14` | `167` | `+1555/-41` | `39.51`% / `99.38`% | `$8.21` | `23.01M` | `24m37s` | `$0.59` | `$0.05` |
| `libgit2/object` | `3` | `0` | wrap | first | `6` | `144` | `+1978/-296` | `42.23`% / `99.43`% | `$13.22` | `40.69M` | `38m33s` | `$2.20` | `$0.09` |
| `libgit2/config` | `0` | `2` | wrap | first | `3` | `74` | `+405/-0` | `42.28`% / `99.43`% | `$1.37` | `5.31M` | `8m00s` | `$0.46` | `$0.02` |
| `libgit2/config` | `1` | `1` | wrap | first | `25` | `265` | `+1429/-4` | `42.71`% / `99.33`% | `$2.46` | `10.12M` | `21m59s` | `$0.10` | `$0.01` |
| `libgit2/config` | `1` | `2` | wrap | first | `3` | `78` | `+630/-8` | `42.73`% / `99.33`% | `$2.40` | `10.91M` | `27m41s` | `$0.80` | `$0.03` |
| `libgit2/config` | `2` | `1` | wrap | first | `5` | `60` | `+816/-6` | `42.78`% / `99.44`% | `$1.63` | `6.29M` | `11m14s` | `$0.33` | `$0.03` |
| `libgit2/config` | `3` | `1` | wrap | first | `3` | `52` | `+707/-9` | `42.76`% / `99.46`% | `$1.41` | `4.66M` | `9m46s` | `$0.47` | `$0.03` |
| `libgit2/config` | `4` | `1` | wrap | first | `5` | `91` | `+422/-88` | `42.85`% / `99.45`% | `$1.17` | `3.84M` | `6m28s` | `$0.23` | `$0.01` |
| `libgit2/config` | `5` | `0` | wrap | first | `3` | `66` | `+1073/-2` | `43.15`% / `99.45`% | `$2.04` | `8.40M` | `14m30s` | `$0.68` | `$0.03` |
| `libgit2/index` | `0` | `8` | wrap | first | `1` | `0` | `+548/-0` | `44.05`% / `99.50`% | `$8.57` | `4.72M` | `9m53s` | `$8.57` | — |
| `libgit2/index` | `1` | `2` | wrap | first | `25` | `370` | `+2142/-57` | `46.56`% / `99.55`% | `$22.30` | `78.75M` | `1h06m15s` | `$0.89` | `$0.06` |
| `libgit2/index` | `1` | `3` | wrap | first | `25` | `405` | `+2120/-46` | `45.96`% / `99.54`% | `$15.01` | `42.85M` | `55m51s` | `$0.60` | `$0.04` |
| `libgit2/index` | `1` | `4` | wrap | first | `5` | `47` | `+342/-22` | `45.76`% / `99.53`% | `$3.09` | `7.11M` | `12m00s` | `$0.62` | `$0.07` |
| `libgit2/index` | `2` | `0` | wrap | first | `17` | `383` | `+2323/-79` | `46.66`% / `99.55`% | `$14.38` | `43.62M` | `43m51s` | `$0.85` | `$0.04` |
| `libgit2/index` | `2` | `1` | wrap | first | `2` | `0` | `+369/-6` | `46.57`% / `99.53`% | `$3.31` | `8.07M` | `18m25s` | `$1.66` | — |
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | review | `2` | — | `+80/-1` | — | `$7.89` | `10.78M` | `18m36s` | `$3.94` | — |
| `libgit2/raw-lifetime-string` | `0` | `0` | wrap | review | `2` | — | `+345/-23` | — | `$10.89` | `14.41M` | `22m03s` | `$5.45` | — |
| `libgit2/util` | `0` | `2` | wrap | review | `4` | — | `+281/-117` | `1.63`% / `99.74`% | `$12.00` | `16.87M` | `28m49s` | `$3.00` | — |
| `libgit2/util` | `1` | `0` | wrap | review | `1` | — | `+0/-0` | `1.63`% / `99.74`% | `$6.26` | `6.37M` | `23m51s` | `$6.26` | — |
| `libgit2/oid` | `1` | `1` | wrap | review | `18` | — | `+181/-10` | `7.80`% / `99.49`% | `$11.62` | `12.73M` | `34m10s` | `$0.65` | — |
| `libgit2/oid` | `2` | `0` | wrap | review | `1` | — | `+0/-0` | `7.80`% / `99.49`% | `$3.39` | `3.23M` | `8m46s` | `$3.39` | — |
| `libgit2/core` | `0` | `1` | wrap | review | `6` | — | `+345/-20` | `15.51`% / `99.81`% | `$12.53` | `17.80M` | `23m38s` | `$2.09` | — |
| `libgit2/core` | `1` | `1` | wrap | review | `5` | — | `+59/-10` | `18.24`% / `99.82`% | `$7.69` | `10.83M` | `17m46s` | `$1.54` | — |
| `libgit2/core` | `2` | `0` | wrap | review | `1` | — | `+61/-9` | `18.23`% / `99.82`% | `$6.83` | `5.78M` | `18m10s` | `$6.83` | — |
| `libgit2/repository` | `1` | `1` | wrap | review | `68` | — | `+640/-143` | `36.16`% / `99.39`% | `$38.68` | `57.91M` | `1h08m35s` | `$0.57` | — |
| `libgit2/repository` | `2` | `0` | wrap | review | `7` | — | `+574/-18` | `36.17`% / `99.32`% | `$29.96` | `37.71M` | `57m28s` | `$4.28` | — |
| `libgit2/object` | `0` | `1` | wrap | review | `2` | — | `+105/-6` | `37.89`% / `99.25`% | `$20.94` | `18.07M` | `1h08m26s` | `$10.47` | — |
| `libgit2/object` | `1` | `1` | wrap | review | `55` | — | `+305/-110` | `39.40`% / `99.37`% | `$12.26` | `40.91M` | `26m40s` | `$0.22` | — |
| `libgit2/object` | `2` | `1` | wrap | review | `89` | — | `+216/-18` | `42.20`% / `99.43`% | `$13.54` | `45.38M` | `27m21s` | `$0.15` | — |
| `libgit2/object` | `3` | `0` | wrap | review | `6` | — | `+226/-17` | `42.24`% / `99.43`% | `$39.58` | `57.30M` | `3h04m22s` | `$6.60` | — |
| `libgit2/config` | `0` | `1` | wrap | review | `3` | — | `+178/-43` | `42.51`% / `99.41`% | `$2.71` | `5.91M` | `12m53s` | `$0.90` | — |
| `libgit2/config` | `1` | `1` | wrap | review | `28` | — | `+625/-242` | `42.75`% / `99.45`% | `$11.84` | `37.34M` | `35m07s` | `$0.42` | — |
| `libgit2/config` | `2` | `1` | wrap | review | `5` | — | `+222/-51` | `42.75`% / `99.46`% | `$4.64` | `11.54M` | `16m26s` | `$0.93` | — |
| `libgit2/config` | `3` | `1` | wrap | review | `3` | — | `+561/-34` | `42.78`% / `99.45`% | `$6.96` | `18.87M` | `24m40s` | `$2.32` | — |
| `libgit2/config` | `4` | `1` | wrap | review | `5` | — | `+575/-147` | `43.14`% / `99.45`% | `$6.30` | `15.93M` | `20m24s` | `$1.26` | — |
| `libgit2/config` | `5` | `0` | wrap | review | `3` | — | `+172/-99` | `43.17`% / `99.45`% | `$7.39` | `17.00M` | `47m40s` | `$2.46` | — |
| `libgit2/index` | `0` | `1` | wrap | review | `1` | — | `+303/-147` | `45.57`% / `99.51`% | `$6.50` | `18.04M` | `24m32s` | `$6.50` | — |
| `libgit2/index` | `1` | `1` | wrap | review | `55` | — | `+442/-9` | `46.58`% / `99.53`% | `$12.37` | `39.58M` | `39m02s` | `$0.22` | — |
| `libgit2/index` | `2` | `0` | wrap | review | `19` | — | `+190/-21` | `46.64`% / `99.55`% | `$20.10` | `42.35M` | `1h18m28s` | `$1.06` | — |
| **Σ first** |  |  |  |  | **`389`** | **`6630`** | **`+62075/-2348`** | **`46.66`% / `99.55`%** | **`$371.14`** | **`1143.40M`** | — | **`$0.95`** | **`$0.05`** |
| **Σ review** |  |  |  |  | **`389`** | — | **`+6686/-1295`** | **`46.64`% / `99.55`%** | **`$312.87`** | **`562.64M`** | — | **`$0.80`** | — |

### Batches: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `libgit2/util` | `0` | `2` | first | `+11` (`+0.18`/`-1.19` pp) | `+0` (`+0.00`/`-10.21` pp) | `+4` (`+0.12`/`-2.07` pp) | `+4` (`+0.07`/`-8.60` pp) |
| `libgit2/util` | `1` | `0` | first | `+6` (`+0.00`/`+7.16` pp) | `+0` (`+0.00`/`-1.80` pp) | `+5` (`+0.00`/`+5.40` pp) | `+3` (`+0.01`/`+0.47` pp) |
| `libgit2/oid` | `1` | `1` | first | `+8` (`+0.17`/`+1.11` pp) | `+0` (`+0.00`/`-5.19` pp) | `+5` (`+0.18`/`+3.02` pp) | `+7` (`+0.10`/`-6.64` pp) |
| `libgit2/oid` | `2` | `0` | first | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `0` | `8` | first | `+9` (`+0.22`/`-6.72` pp) | `+1` (`+0.63`/`+15.87` pp) | `+17` (`+0.17`/`+2.31` pp) | `+10` (`+0.14`/`-12.68` pp) |
| `libgit2/core` | `1` | `1` | first | `+6` (`+2.54`/`-0.21` pp) | `+5` (`+3.62`/`+0.85` pp) | `+4` (`+2.38`/`-0.11` pp) | `+6` (`+0.00`/`-0.10` pp) |
| `libgit2/core` | `2` | `0` | first | `+0` (`+0.02`/`+0.08` pp) | `+5` (`+0.01`/`+0.29` pp) | `+3` (`+0.00`/`+0.08` pp) | `+0` (`+0.00`/`-0.30` pp) |
| `libgit2/repository` | `1` | `2` | first | `+11` (`+0.83`/`+0.46` pp) | `+0` (`-0.05`/`-1.30` pp) | `+14` (`+0.78`/`+0.63` pp) | `+10` (`+0.87`/`-1.56` pp) |
| `libgit2/repository` | `1` | `3` | first | `+14` (`+1.28`/`+1.23` pp) | `+2` (`+1.07`/`-0.48` pp) | `+18` (`+1.29`/`+0.67` pp) | `+11` (`+0.14`/`-2.09` pp) |
| `libgit2/repository` | `1` | `4` | first | `+18` (`+0.63`/`+0.17` pp) | `+0` (`-0.26`/`-1.36` pp) | `+10` (`+0.34`/`+0.34` pp) | `+13` (`+1.37`/`-0.32` pp) |
| `libgit2/repository` | `2` | `0` | first | `+6` (`-0.20`/`+0.33` pp) | `+0` (`-0.02`/`+1.36` pp) | `+3` (`+0.08`/`+0.31` pp) | `+6` (`+0.16`/`-0.22` pp) |
| `libgit2/object` | `0` | `6` | first | `+9` (`+0.08`/`-0.39` pp) | `+0` (`+0.02`/`-0.19` pp) | `+9` (`+0.08`/`-0.35` pp) | `+7` (`+0.07`/`+0.12` pp) |
| `libgit2/object` | `1` | `5` | first | `+23` (`+0.42`/`+0.28` pp) | `+0` (`+0.29`/`-1.46` pp) | `+8` (`+0.25`/`-0.52` pp) | `+20` (`+0.46`/`-0.12` pp) |
| `libgit2/object` | `1` | `6` | first | `+15` (`+0.53`/`+0.57` pp) | `+0` (`-0.59`/`-0.82` pp) | `+6` (`+0.74`/`-0.36` pp) | `+21` (`+1.51`/`+0.18` pp) |
| `libgit2/object` | `1` | `7` | first | `+9` (`+0.23`/`+0.30` pp) | `+0` (`+0.32`/`-0.32` pp) | `+5` (`+0.06`/`+0.27` pp) | `+5` (`+0.29`/`+0.26` pp) |
| `libgit2/object` | `2` | `1` | first | `+18` (`+0.21`/`+0.75` pp) | `+0` (`-0.49`/`-0.88` pp) | `+5` (`+0.14`/`+0.08` pp) | `+16` (`+3.31`/`+0.73` pp) |
| `libgit2/object` | `2` | `2` | first | `+9` (`+0.12`/`+0.10` pp) | `+0` (`+0.06`/`-0.23` pp) | `+2` (`+0.10`/`+0.22` pp) | `+7` (`+0.08`/`+0.11` pp) |
| `libgit2/object` | `2` | `3` | first | `+13` (`+0.51`/`+0.54` pp) | `+0` (`+0.12`/`-0.42` pp) | `+15` (`+0.57`/`+1.05` pp) | `+9` (`+0.75`/`-0.49` pp) |
| `libgit2/object` | `2` | `4` | first | `+9` (`+0.06`/`+0.13` pp) | `+0` (`-0.46`/`-0.15` pp) | `+3` (`+0.03`/`+0.36` pp) | `+2` (`+0.00`/`-0.09` pp) |
| `libgit2/object` | `3` | `0` | first | `+6` (`-0.01`/`+0.30` pp) | `+0` (`-0.21`/`-0.32` pp) | `+0` (`+0.00`/`+0.57` pp) | `+4` (`+0.32`/`+0.03` pp) |
| `libgit2/config` | `0` | `2` | first | `+3` (`+0.05`/`+0.05` pp) | `+0` (`-0.46`/`-0.04` pp) | `+1` (`+0.04`/`+0.05` pp) | `+0` (`+0.01`/`-0.17` pp) |
| `libgit2/config` | `1` | `1` | first | `+6` (`+0.20`/`+0.23` pp) | `+0` (`-0.02`/`-0.35` pp) | `+3` (`+0.15`/`+0.34` pp) | `+2` (`+0.02`/`-2.06` pp) |
| `libgit2/config` | `1` | `2` | first | `+4` (`+0.01`/`+0.04` pp) | `+0` (`-0.28`/`-0.05` pp) | `+3` (`+0.01`/`+0.04` pp) | `+3` (`-0.01`/`+0.03` pp) |
| `libgit2/config` | `2` | `1` | first | `+5` (`+0.05`/`+0.03` pp) | `+0` (`+0.02`/`-0.12` pp) | `+3` (`+0.03`/`+0.03` pp) | `+3` (`+0.00`/`-0.07` pp) |
| `libgit2/config` | `3` | `1` | first | `+4` (`+0.02`/`+0.07` pp) | `+0` (`+0.01`/`-0.07` pp) | `+3` (`+0.02`/`+0.11` pp) | `+2` (`+0.00`/`+0.06` pp) |
| `libgit2/config` | `4` | `1` | first | `+3` (`+0.03`/`+0.02` pp) | `+0` (`+0.06`/`-0.02` pp) | `+1` (`+0.00`/`+0.03` pp) | `+1` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `5` | `0` | first | `+4` (`+0.02`/`+0.02` pp) | `+0` (`-0.31`/`-0.14` pp) | `+2` (`+0.02`/`+0.07` pp) | `+4` (`+0.08`/`+0.15` pp) |
| `libgit2/index` | `0` | `8` | first | `+3` (`+0.00`/`+0.08` pp) | `+0` (`+0.18`/`-0.08` pp) | `+1` (`+0.09`/`+0.07` pp) | `+3` (`+1.70`/`-0.06` pp) |
| `libgit2/index` | `1` | `2` | first | `+8` (`+1.45`/`+0.13` pp) | `+0` (`+0.01`/`-0.15` pp) | `+5` (`+1.06`/`+0.28` pp) | `+8` (`+0.12`/`-0.10` pp) |
| `libgit2/index` | `1` | `3` | first | `+8` (`+0.27`/`+0.15` pp) | `+0` (`+0.04`/`-0.20` pp) | `+10` (`+0.17`/`+0.38` pp) | `+7` (`+0.07`/`-0.21` pp) |
| `libgit2/index` | `1` | `4` | first | `+3` (`+0.10`/`+0.04` pp) | `+0` (`-0.04`/`+0.00` pp) | `+2` (`+0.02`/`+0.06` pp) | `+0` (`+0.00`/`-0.04` pp) |
| `libgit2/index` | `2` | `0` | first | `+10` (`+0.19`/`+0.18` pp) | `+0` (`-0.13`/`-0.15` pp) | `+10` (`+0.09`/`+0.47` pp) | `+6` (`+0.14`/`+0.01` pp) |
| `libgit2/index` | `2` | `1` | first | `+2` (`+0.00`/`+0.00` pp) | `+0` (`+0.03`/`-0.01` pp) | `+3` (`-0.02`/`+0.08` pp) | `+2` (`-0.02`/`+0.01` pp) |
| `libgit2/util` | `0` | `2` | review | `+1` (`+0.00`/`-0.79` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`-0.79` pp) | `+1` (`+0.00`/`+4.47` pp) |
| `libgit2/util` | `1` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/oid` | `1` | `1` | review | `+0` (`+0.00`/`+0.02` pp) | `+0` (`+0.00`/`-0.04` pp) | `+1` (`+0.00`/`+0.03` pp) | `+1` (`+0.00`/`+0.27` pp) |
| `libgit2/oid` | `2` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `0` | `1` | review | `+0` (`-0.01`/`-0.79` pp) | `+0` (`+0.01`/`+0.69` pp) | `+0` (`-0.01`/`-0.22` pp) | `+3` (`+0.02`/`+0.75` pp) |
| `libgit2/core` | `1` | `1` | review | `+0` (`+0.01`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `2` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/repository` | `1` | `1` | review | `+0` (`+0.08`/`-0.28` pp) | `+0` (`-0.04`/`-0.10` pp) | `+1` (`+0.03`/`-0.33` pp) | `+3` (`+0.00`/`+0.16` pp) |
| `libgit2/repository` | `2` | `0` | review | `+1` (`+0.00`/`+0.11` pp) | `+0` (`-0.06`/`-0.13` pp) | `+4` (`+0.01`/`+0.14` pp) | `+1` (`+2.18`/`+0.32` pp) |
| `libgit2/object` | `0` | `1` | review | `+0` (`+0.22`/`+0.00` pp) | `+0` (`+0.05`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+3` (`+0.00`/`+0.02` pp) |
| `libgit2/object` | `1` | `1` | review | `+1` (`+0.01`/`+0.10` pp) | `+0` (`-0.09`/`+0.02` pp) | `+0` (`-0.02`/`+0.23` pp) | `+2` (`-0.03`/`-0.03` pp) |
| `libgit2/object` | `2` | `1` | review | `+0` (`-0.01`/`+0.00` pp) | `+0` (`-0.01`/`+0.00` pp) | `+3` (`+0.03`/`-0.01` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/object` | `3` | `0` | review | `+0` (`+0.01`/`+0.01` pp) | `+0` (`-0.29`/`-0.02` pp) | `+0` (`+0.03`/`+0.02` pp) | `+2` (`+0.03`/`+0.31` pp) |
| `libgit2/config` | `0` | `1` | review | `+0` (`+0.01`/`+0.02` pp) | `+0` (`-0.47`/`-0.01` pp) | `+1` (`+0.00`/`+0.03` pp) | `+2` (`+0.04`/`+0.22` pp) |
| `libgit2/config` | `1` | `1` | review | `+1` (`+0.00`/`+0.10` pp) | `+0` (`+0.20`/`-0.03` pp) | `+2` (`+0.00`/`+0.16` pp) | `+4` (`+0.07`/`+0.82` pp) |
| `libgit2/config` | `2` | `1` | review | `+0` (`-0.02`/`+0.02` pp) | `+0` (`+0.01`/`+0.01` pp) | `+1` (`+0.01`/`+0.04` pp) | `+1` (`-0.01`/`+0.18` pp) |
| `libgit2/config` | `3` | `1` | review | `+3` (`+0.02`/`+0.01` pp) | `+0` (`-0.49`/`-0.04` pp) | `+2` (`+0.00`/`+0.15` pp) | `+0` (`+0.00`/`-0.24` pp) |
| `libgit2/config` | `4` | `1` | review | `+1` (`-0.01`/`+0.00` pp) | `+0` (`+0.12`/`+0.00` pp) | `+2` (`-0.02`/`+0.00` pp) | `+2` (`-0.01`/`+0.01` pp) |
| `libgit2/config` | `5` | `0` | review | `+0` (`+0.00`/`+0.02` pp) | `+0` (`+0.19`/`-0.01` pp) | `+0` (`+0.02`/`+0.02` pp) | `+0` (`+0.00`/`+0.01` pp) |
| `libgit2/index` | `0` | `1` | review | `+1` (`-0.02`/`+0.02` pp) | `+0` (`-0.13`/`-0.01` pp) | `+0` (`-0.01`/`-0.01` pp) | `+2` (`+0.01`/`+0.01` pp) |
| `libgit2/index` | `1` | `1` | review | `+2` (`+0.04`/`+0.01` pp) | `+0` (`-0.02`/`-0.06` pp) | `+0` (`+0.01`/`+0.16` pp) | `+4` (`+0.16`/`+0.17` pp) |
| `libgit2/index` | `2` | `0` | review | `+3` (`+0.01`/`+0.01` pp) | `+0` (`+0.10`/`+0.00` pp) | `+1` (`+0.00`/`+0.01` pp) | `+1` (`-0.02`/`+0.00` pp) |
| **Σ first** |  |  |  | **`+263` (`+10.21`/`+6.04` pp)** | **`+13` (`+3.17`/`-8.14` pp)** | **`+183` (`+8.99`/`+13.91` pp)** | **`+202` (`+11.76`/`-33.75` pp)** |
| **Σ review** |  |  |  | **`+14` (`+0.34`/`-1.41` pp)** | **`+2` (`-0.92`/`+0.27` pp)** | **`+19` (`+0.09`/`-0.37` pp)** | **`+32` (`+2.44`/`+7.45` pp)** |

### Batches: safety audit

Deterministic `ffimap`; no model.

| subsystem | wave | batch | objective | pass | pub raw ptr (safe) | pub raw ptr | pub void ptr | pub unsafe fn | C borrow | slice borrow | unsafe loc % | unsafe blocks | Send/Sync | C by value | Drop/Clone | deref outside impl |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+13.04` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/raw-lifetime-string` | `0` | `0` | wrap | first | `+2` | `+3` | `+0` | `+1` | — | — | `-0.15` pp | `+17` | — | — | `+2` | `+0` |
| `libgit2/util` | `0` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/util` | `1` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.09` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/oid` | `1` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.62` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/oid` | `2` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `8` | wrap | first | `+0` | `+0` | `+0` | `+33` | — | — | `+2.41` pp | `+65` | — | — | `+1` | `+0` |
| `libgit2/core` | `1` | `1` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.01` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/core` | `2` | `0` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `+0.03` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/repository` | `1` | `2` | wrap | first | `+1` | `+1` | `+1` | `+0` | — | — | `+0.54` pp | `+43` | — | — | `+1` | `+2` |
| `libgit2/repository` | `1` | `3` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.17` pp | `+34` | — | — | `+0` | `+1` |
| `libgit2/repository` | `1` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+26` | — | — | `+0` | `+0` |
| `libgit2/repository` | `2` | `0` | wrap | first | `-1` | `-1` | `-1` | `+0` | — | — | `+0.04` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.07` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `5` | wrap | first | `-1` | `-2` | `-2` | `-1` | — | — | `+0.12` pp | `+24` | — | — | `+0` | `+2` |
| `libgit2/object` | `1` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.17` pp | `+28` | — | — | `+0` | `+2` |
| `libgit2/object` | `1` | `7` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+9` | — | — | `+0` | `+1` |
| `libgit2/object` | `2` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.40` pp | `+52` | — | — | `+0` | `+1` |
| `libgit2/object` | `2` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+26` | — | — | `+0` | `+0` |
| `libgit2/object` | `2` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.36` pp | `+29` | — | — | `+0` | `+0` |
| `libgit2/object` | `2` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.08` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/object` | `3` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/config` | `0` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `1` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+30` | — | — | `+0` | `+0` |
| `libgit2/config` | `1` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/config` | `2` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.01` pp | `+5` | — | — | `+0` | `+1` |
| `libgit2/config` | `3` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.07` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `4` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/config` | `5` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `8` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+4` | — | — | `+0` | `+1` |
| `libgit2/index` | `1` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/index` | `1` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.16` pp | `+27` | — | — | `+0` | `+0` |
| `libgit2/index` | `1` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/index` | `2` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.03` pp | `+14` | — | — | `+2` | `+0` |
| `libgit2/index` | `2` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/raw-lifetime-string` | `0` | `0` | wrap | review | `+0` | `+0` | `+0` | `+1` | — | — | `-0.34` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/util` | `0` | `2` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/util` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.20` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/core` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/repository` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.13` pp | `-4` | — | — | `+0` | `+0` |
| `libgit2/repository` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.02` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/object` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/object` | `2` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `3` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/config` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `2` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `3` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/config` | `4` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `5` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `0` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `-1` |
| `libgit2/index` | `1` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/index` | `2` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| **Σ first** |  |  |  |  | **`+1`** | **`+1`** | **`-2`** | **`+37`** | — | — | **`+18.28` pp** | **`+541`** | — | — | **`+6`** | **`+11`** |
| **Σ review** |  |  |  |  | **`+0`** | **`+0`** | **`+0`** | **`+1`** | — | — | **`-0.38` pp** | **`+19`** | — | — | **`+0`** | **`-1`** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `libgit2/raw-lifetime-void` | `0` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/raw-lifetime-string` | `0` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `0` | `2` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `1` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/oid` | `1` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` |
| `libgit2/oid` | `2` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `0` |
| `libgit2/core` | `0` | `1` | — | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/core` | `1` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/core` | `2` | `0` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `2` |
| `libgit2/repository` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+2` | `4` |
| `libgit2/repository` | `2` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` |
| `libgit2/object` | `0` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/object` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+2` | `4` |
| `libgit2/object` | `2` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+1` | `3` |
| `libgit2/object` | `3` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+4` | `6` |
| `libgit2/config` | `0` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` |
| `libgit2/config` | `1` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `6` |
| `libgit2/config` | `2` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+2` | `4` |
| `libgit2/config` | `3` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/config` | `4` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` |
| `libgit2/config` | `5` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` |
| `libgit2/index` | `0` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` |
| `libgit2/index` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` |
| `libgit2/index` | `2` | `0` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+4` | `6` |
| **Σ** |  |  | **`+0`** | **`+5`** | **`+3`** | **`+1`** | **`+3`** | **`+4`** | **`+11`** | **`+46`** | **`73`** |

## Notes

The only prose outside the setup and legend above: pitfalls, findings, and the
context each table cannot carry. One `###` subsection per finding, titled by
what it is about. Describe the EXPERIMENT and its results — a fix made to
crustify or ffibox along the way belongs in that repo's history, not here.

> Gate misses and anything the oracle and `translate` disagreed on; a wave that
> was superseded and why; what each wave's diff actually contained beyond its
> row counts; where a metric moved and what moved it; what the judge found and
> whether it held. Everything else stays in the tables.

State where the LoC figures come from, and note that all of them exclude
comments and blank lines.

`C LoC` is `wavefront query dag --name <every scheduled entity> --loc`:
the oracle's translated-LoC view, a function seed valued at its body LoC and a
type seed at its field and op count. It reports the seeds only, with no closure
expansion, so it is the C the campaign translated rather than the surface it
was drawn from. Give the defining files' and the whole target's raw totals
beside it for scale.

`Rust LoC` is counted from source over the authored `.rs` files under
`crustify/rust`, excluding anything generated into `target/`, and split by
`#[cfg(test)]` module.

Say why that non-test figure differs from the `code_lines` ffimap
reports. The audit measures the union of HIR definition spans, so it counts
only what sits inside an item — no `use`, `mod` or free-standing attribute
lines — and by construction cannot see `cfg`-disabled code, which is why it
yields no test figure. Its number is the right denominator for the unsafe
ratios and the wrong one for how much Rust was written; the two must not be
added.

Say plainly that the Rust-to-C ratio is not like-for-like in any measure,
because the Rust carries tests, `// SAFETY:` justifications, `ffi_export`
gateways and scaffolding with no C counterpart.

The four unit counts are de-duplicated over ENTITIES, not scheduled units: an
entity appears once, under the last objective it ran, so a type wrapped and
later ported counts as ported and never in both. Name the entities that took
both paths. Callbacks count with symbols.

Some of it is structural and belongs here every time: whether a review wave
reused the translation batches or the oracle re-batched it under distinct caps;
which units a review schedule dropped and why; which subsystems the Overview
lists but no table details, and the cost that leaves unaccounted; and any column
a campaign could not fill, said once rather than left as a field of em-dashes.

### C test baseline

Every offline `libgit2_tests` suite and every `util_tests` suite ran in isolation
(`crustify/tmp/scripts/c-suites.sh`, untracked scratch) against each prebuild; the per-variant record is
`crustify/artifacts/c-baseline.json`. Plain, coverage and ASan+UBSan pass
every case they run (`3443/3498`; coverage `3444/3498`). The 55 skips are clar's own
environment guards, not disabled tests: `checkout::icase` (12), `index::bypath` (6),
`odb::largefiles` (6), `repo::init` (6), `regexp` locale cases (4), `transport::ssh::exec`
(3), `checkout::index` (2), `ftruncate` (2), `filebuf` (2), `iterator::workdir` (2), and one
each in `status::worktree`, `process::start`, `object::tree::read`, `futils`,
`filter::stream`, `filter::query`, `diff::workdir`, `core::structinit` and `config::stress`.
The `online::*` suites need the network and are excluded, as the `offline` ctest target
excludes them.

TSan reports pre-existing C findings in 10 suites: lock-order inversions in the attr cache
(`attr::macro`, `attr::repo`, `diff::{blob,diffiter,parse,tree}`, `pack::filelimit`,
`reset::mixed`) and data races in `threads::{diff,refdb}`. BSan reports an out-of-bounds
4-byte access in an 8-byte allocation in the four `object::cache` variants, and
`pack::threadsafety` and `stress::diff` exceed the 30-minute suite timeout under it. These
suites are the sanitizer baseline the regression gates compare against.

### Scope

The schedule covers the 912 exported functions, 63 callbacks and 258 types the public
headers publish, except six declarations `libgit2.so` does not export and so cannot be
linked: `git_blame_file_from_buffer`, `git_config_backend_from_string`,
`git_config_backend_from_values`, `git_stdalloc_init_allocator`, `git_transport_dummy` and
`git_win32_crtdbg_init_allocator`. Deprecated items vanish under `GIT_DEPRECATE_HARD`;
`deprecated.h`, `cred_helpers.h`, `sys/cred.h` and the MSVC-only `stdint.h` are not
inventoried. `git_cached_obj`, an internal type the signature closure reaches through
`git_odb_object`, is skipped.

Wavefront schedules no macro units for this surface: constant and `*_INIT` macros reach
Rust as generated `-sys` constants and through the scheduled `*_options_init` functions.
The ten public function-like macros (`GIT_ATTR_IS_*`, `GIT_ATTR_HAS_VALUE`,
`GIT_INDEX_ENTRY_STAGE{,_SET}`, `GIT_SUBMODULE_STATUS_IS_*`) are scheduled as symbol
batches in the last wave of `attr`, `index` and `submodule`.

Wavefront dropped a published callback no target body reaches: six callbacks were missing
from the campaign-wide inventory (`git_apply_hunk_cb`, `git_commit_signature_cb`,
`git_filter_check_fn`, `git_filter_stream_fn`, `git_merge_driver_apply_fn`,
`git_remote_ready_cb`) and `git_push_transfer_progress_cb` from the `remote` subsystem's.
The schedule was regenerated, before any subsystem sub-campaign started, with the fixed
composer (wavefront `fix/subsystem-callback-scope`), which leaves its shape unchanged
(20 sub-campaigns, 75 waves, 246 batches) and adds the six callbacks. Translators query the
installed wavefront, whose named lookups resolve these callbacks.

### Sub-campaign order and transitive producers

The implementation graph is one strongly connected component over every subsystem except
`llhttp` and `ntlmclient`; the cycle cut schedules first the member most other members
depend on. Sub-campaigns are scheduled with `--transitive`, so a signature-level producer
owned by a later subsystem is translated with its first consumer and homed under its
owner's header module (for example `git_checkout_options` in `repository`, homed in
`workdir/checkout.rs`). `llhttp`, `ntlmclient`, `xdiff` and `reftable` publish no API and
have no sub-campaign.

### `raw-lifetime-void`

The batch found the only public `void *` lifecycle pair, `git_odb_backend_data_alloc` /
`git_odb_backend_data_free`, and emitted two release strategies over it, `OdbDataFree` for
one object (`OdbDataBox<T>`) and `OdbDataArrayFree` for a counted run (`OdbDataVec<T>`),
homed in `odb/sys/odb_backend.rs` and re-exported from `raw.rs`. The review filed one `misc`
report, both integration suites aborting under Miri before their Rust-only tests, which
reproduced and whose `cfg_attr(miri, ignore)` gating holds; no PoC was rejected. The
sub-campaign wall includes orchestrator self-repair time between the batch and its review
and an operator pause before the final gate.

### `ub_safe` admits a fixture module

The `raw-lifetime-void` batch relaxed `tests/ub_safe/main.rs` from the scaffolded
`#![forbid(unsafe_code)]` to `#![deny(unsafe_code)]` so that a `fixture` module can stand in,
with `unsafe`, for safe constructors the crate does not have yet; the review did not flag it.
It is an open departure from the `ub_safe` contract, left in place until the constructors
it stands in for are wrapped.

### BSan skips the compile-fail suite

`trybuild`'s nested cargo build does not finish under BorrowSanitizer, so
`crustify/rust/scripts/cargo-variant.sh bsan test` skips `compile_fail`; the plain variant
runs it.

### `raw-lifetime-string`

The batch emitted two string strategies over the same exported pair: `OdbDataStrFree` for
one NUL-terminated string (`OdbDataStr`) and an owned run of them (`OdbDataStrVec`,
reproducing `git_strarray_copy`, which `GIT_DEPRECATE_HARD` hides). The review filed two
reports. `unsafe/odb-data-str-adoption-ignores-git-buf-ownership` held on rerun: on the
affected commit its reproducer, under ASan against the `asan-ubsan` libgit2 build, frees
libgit2's static `git_str__initstr`, because a `git_buf` owns its `ptr` only while
`reserved > 0` and the docs invited adopting any `git_buf` string; the fix documents the
rule and adds a checked `take_buf` seam. `wavefront/string-cloners-unrecorded` submitted
the hidden `git__strdup` family as the tier's cloners. No PoC was rejected.

### Reading the batch tables

Batch rows from every sub-campaign share one table per kind; the `subsystem` cell names the
row's sub-campaign as `<link-unit>/<subsystem>`, the same name as its Overview row.
A wrap campaign has no port batches, so the two `Types — Batches — port` tables stay
unfilled. A `wavefront` report category, for ownership findings the review resubmitted, is
counted under `+misc`.

Each batch is measured on two tree states: just before its first landing commit (the wave
base or the previous batch's landing) and its last landing commit. `cov (C/Rust)` is the
latter, from one instrumented build per state: the four workloads (`--lib` unit tests,
`ub_safe`, `ub_unsafe`, `equiv`) each run alone into their own profile directory against
the `coverage` prebuild, and their union is the cell. C is lcov line coverage over the 196
translation units `libgit2.so` links, with a zero baseline from the `.gcno` files so that
untouched TUs count; Rust is cargo-llvm-cov line coverage over `crustify/rust/libgit2`'s
non-`tests/` sources, minus every line inside a `#[cfg(test)]` item (the inline unit-test
modules). All workloads share one denominator, and the union is the union of their covered
lines. Every Rust cell and Rust delta in these tables was restated on 2026-10-12. The
earlier figures counted those test lines, 60% of the instrumented lines at the
`libgit2/index` tip, which held the integration suites' coverage far down and credited
the unit tests for running themselves. All 182 recorded states were re-measured; the C
cells and C deltas stay as first recorded. The eight `libgit2/config` states between
`beeda6a07` and the export revert `c5440d3ee` call the two reverted constructors, so they
were measured against a private coverage build of their own source, which exports them. A Σ `cov` cell is the state after the table's chronologically
last landing. The `(+C/+Rust pp)` pairs in the `…: tests` tables are each workload's own
coverage after minus before, so a batch that adds code its workload does not reach shows a
negative Rust delta. Smell deltas are the crate-wide `unsafe_fn_smell` and `raw_ptr_smell`
of an unseeded `crustify ffimap` over the same two states.

Test counts are `#[test]` functions the landing commits add, by directory; trybuild
`compile_fail` cases are not counted. `+LoC` and `rv loc` count `.rs` lines under
`crustify/rust/`, so review reproducer crates under `crustify/reviews/` are excluded. The
measurement scripts live outside the repository, beside the orchestrator's other helpers;
each batch's `ffimap.json` beside it is the unseeded scan of its last landing commit.

### Overview test counts

The five test lines are measured at the campaign tip `14b9001c0`. Each count is the number of
tests its compiled test binary registers on the plain build (`cargo test -- --list`):
`--lib` for the unit tests, which live in inline `#[cfg(test)] mod unit_tests` modules,
and the `ub_safe`, `ub_unsafe` and `equiv` integration crates. Ignored tests count (one
unit test); doctests (`compile_fail` included) and trybuild cases do not, while the
trybuild runner itself is one `ub_safe` test. `ub_unsafe` holds 59 written `#[test]`s and
36 generated by its `isolated_test!` macro. `Total tests` adds the four counts. The coverage pairs come from the method in "Reading the batch
tables". Each workload runs alone against the `coverage` prebuild, and `Total tests`
carries the union from the same run. Rust coverage excludes `tests/`, the `-sys`
bindings and dependencies. It also excludes every line inside a `#[cfg(test)]` item, the
inline unit-test modules being 60% of the instrumented library lines, so that the tests
are not counted as code to cover. All four workloads share one denominator, and the total
is the union of their covered lines. The UB suites also run under ASan+UBSan, TSan and
BorrowSanitizer in the agents' gates. Those runs share one test set and are not counted
again.
### `libgit2/util`

Wave 0 wrapped `git_allocator`, `git_error`, `git_error_code`, `git_error_t` and the
`git2/sys/errors.h` setters; wave 1 wrapped `git_error_last`. The review's two reproducers
held on rerun: an ASan double free when a caller follows the documented `realloc` contract
to the letter for a zero-byte resize on glibc (`unsafe/alloc-realloc-contract-zero-size-resize`),
and an equivalence-test flake where a fresh thread can start with `errno == EAGAIN`
(`misc/equiv-runners-read-errno-of-fresh-thread`, also filed twice more as no-PoC reports).
`misc/asan-tsan-runner-sanitizes-host-proc-macros` did not reproduce at its base commit,
where `asan-ubsan` and `tsan` build and pass `ub_safe` 50/50 from clean target dirs. It was
rejected and its runner change reverted; it is a tooling report,
so the `+rejected PoCs` column, which counts `ub_safe`, `ub_unsafe` and `equiv` PoCs, does not
include it.

### `git_allocator` exposes raw callback pointers

`util/sys/alloc.rs` wraps `git_allocator` through an `unsafe trait Alloc` with private
`extern "C"` trampolines (`Allocator::new::<A>()`), but also publishes the raw form: the
`MallocFn` / `ReallocFn` / `FreeFn` aliases, `Allocator::from_fns`, and accessors that take
or return those pointers. The raw form is unnecessary once every `Allocator` is built from an
`Alloc` implementation, and the review's confirmed zero-byte `realloc` double free came from
its contract. The `util` review judged the aliases necessary ("C function pointers and must
stay so"), and `ffimap` does not count function-pointer types, so neither flagged it.
Left in place by decision; the same pattern can recur for the backend, filter, merge-driver,
transport and stream vtables.

### `libgit2/oid`

Wave 0 wrapped `git_oid`, `git_oid_shorten` and `git_oid_t`; wave 1 wrapped `git_oidarray` and
the 18 `git_oid_*` symbols; wave 2's only item, `git_oidarray_dispose`, was already wrapped as
the `git_oidarray` drop policy, so that batch landed an empty verification commit (`+0/-0`).
Every review PoC held on rerun at its reports commit:
`ub_safe/empty-array-dispose-races-first-init`, a ThreadSanitizer data race reached from
`#![forbid(unsafe_code)]` code, between dropping an empty `OidArray` (`git__free` under
`git_oidarray_dispose`) and the first `git_libgit2_init` on another thread (3 of 3 runs);
and `internal/format-cstr-empty-buffer-told-by-address`, a panic in
`OidRef::format_cstr`. The other eight reports carry no reproducer (documentation,
`Hash` for `Oid`, the header's typed null-id initializers, two wavefront note corrections,
an accessor inconsistency). No PoC was rejected.

### BorrowSanitizer's GC hang threshold

In `oid`'s final gate BorrowSanitizer aborted the parallel `equiv` suite: its garbage
collector waited past its default 10 s `gc_hang_threshold_ms` for a test thread to reach a
safepoint, which is a runtime abort, not an undefined-behaviour report. Serialized, the suite
passed. With the threshold at 120 s it also passes in parallel (42 passed, 1 ignored, 298 s
against 380 s serialized), so the stall is slow collection under thread churn, not a
deadlock. `cargo-variant.sh bsan` keeps tests parallel and sets
`BSAN_OPTIONS=gc_hang_threshold_ms=120000`; the full workspace gate then passes in 431 s
(244 passed) against 641 s serialized.

### Review and BorrowSanitizer policy from `libgit2/core`

Through `libgit2/oid`, review ran after each sub-campaign's translation as a mirror of its
batches (one review batch per translation batch, same caps), and translators validated under
every instrument, BorrowSanitizer included. From `libgit2/core` on, by the operator's decision:

- every translated wave is reviewed before the next translation wave starts, by at most two
  review batches, one with all the wave's types and one with all its symbols and callbacks,
  so review caps scale with the wave;
- translators write BSan tests but do not run the BSan variant; the orchestrator gates every
  translated wave and every reviewed wave under BSan (plain and ASan+UBSan alongside), and a
  failing translated-wave gate hands its log to the review wave as `bsan-gate.log`;
- reviewers keep BSan.

The translator prompt for these rules is crustify `e9ff79a` and `0fcfcae`
(`policy/wave-reviews-orchestrator-bsan`). Review rows before and after the change are
therefore not like-for-like: earlier review rows are per translation batch, later ones per
wave and route.

### `ffimap` schema from crustify `4771856`

Crustify's scanner changed after `libgit2/core` (`f66b2c3`..`4771856`): it lost its seeded
`--name` mode, split `unsafe_fns`, raw-pointer and `void *` positions into seam, pub smell and
private smell, added `raw_ptr_fields`, the three `ref_to_type_wrapper_*` counters and
`deref_impl_on_wrapper`, and dropped `raw_ptr_in_wrapper`. Every recorded scan was rerun with
it: each batch's `ffimap.json` at its last landing commit (the seeded per-batch scans,
and `raw-lifetime-void`'s seeded `ffimap.primitives.json` over the C primitives, have no
counterpart and are gone), each wave's and sub-campaign's at the commit that recorded it. The
smell deltas in the batch tables were recomputed from the same scanner, which moved
`libgit2/oid` wave 0 batch 0 and wave 1 batch 0 from `+1` to `+0` `unsafe fn` smell; no other earlier row
changed.

### Safety audit columns and `cbf6fbc` counters

Crustify `cbf6fbc` added eight `ffimap` counters, all split by effective visibility:
`unsafe_fns_pub_seam`/`_priv_seam`, `raw_ptr_pub_seam`/`_priv_seam`, `raw_ptr_pub`/`_priv`
and `fns_pub`/`fns_priv`. It changed no existing counter. The campaign scan and every
sub-campaign scan were rerun with it, and the batch and wave scans, whose counters it left
unchanged, were not. The rerun matched every earlier value. Each All-metrics column is the
crate-wide scan at that sub-campaign's end, so the columns accumulate left to right. `final`
is the campaign tip. `libgit2/config` is scanned at its tip after the export revert and the
`git_config_backend` follow-up.

In the Final tally, `unsafe loc` is `unsafe_block_code_lines` and `% of loc` is
`unsafe_loc_ratio`. `% in impl T` is `unsafe_blocks_wrapper_impl` over `unsafe_blocks`, and
`...of which not sanctioned` is `unsafe_fn_smell`. `void-ptr smell` sums the public and
private `void *` smells. `&`/`&mut` on a wrapper sums `ref_to_type_wrapper_smell` and
`ref_to_type_wrapper_body_smell`, and `field proj outside an accessor` is
`field_proj_outside_impl`. The 8 wrapper-reference smells have stood since
`raw-lifetime-string`, and the 4 outside-accessor projections since `libgit2/core`.
### Parallelism and review caps from `libgit2/repository`

Before `libgit2/repository` the operator raised the batch parallelism from orchestrator's
choice (`10`) to up to `16`, and turned the review caps over to the orchestrator. Each
review batch now takes up to `20` types (10× the translation cap) and up to `100` symbols
and callbacks (4×). A route goes into near-equal batches only when it exceeds its cap.
The largest remaining waves have 20 types (`repository` wave 0) and 92 symbols (`refs`
wave 1), so every review wave so far still has at most two batches, as it did from
`libgit2/core`. The widest translation wave left has 12 batches, so with `16` every wave
runs all its batches at once.

### `libgit2/object`

Four waves wrapped the 21 types and 152 symbols and callbacks of `blob.h`, `commit.h`,
`mailmap.h`, `message.h`, `object.h`, `tag.h` and `tree.h`: the object types in wave 0,
their typed handles (`git_blob`, `git_commit`, `git_tag`, `git_tree`) and the options
structs in wave 1, the bulk of the symbols in wave 2, and the commit creation and amend
routines composed over them in wave 3. The reviews filed 33 reports and fixed every finding
in place: 10, 12, 5 and 6 per wave. Among them were four `lifecycle` reports, on alloc
constructors returned through out-parameters that the type batches neither wrapped nor
submitted (`git_object_peel`, `git_tree_entry_bypath`, `git_commit_parent`, `git_commit_tree`).
Two `unsafe` reports replaced raw `git_tree`/`git_blob` seams with type-checked handles. One
`ub_safe` reproducer, `tree-builder-insert-formats-foreign-id-out-of-bounds`, overflows a
heap buffer in C from safe code: a SHA-256 id inserted into a SHA-1 repository's tree builder
makes `git_oid_pathfmt` write past the loose backend's path buffer. The wrapper now refuses
such ids. One `equiv` report covers `tag::foreach`, which panicked on a debug assertion where
`git_tag_foreach` returns `0` after a callback stops the walk with `GIT_ITEROVER`. Five `wavefront` reports resubmitted ownership records. Reviewer
reproducers are no longer rerun or rejected by the orchestrator, so `+rejected PoCs` is `0`
by construction.

The review model changed from `claude-fable-5.1` to `claude-opus-5.5` after review wave 0,
by the operator's decision. Review wave 0's types batch cost `$128.50` over `3h16m` for 11
types, mostly in whole-workspace BSan gates stalled by mailmap corpus tests. Review waves 1
and 2 under Opus took 15 to 30 minutes per batch. Review wave 3 (`$39.58`, `3h04m` for 6
symbols) owned the sub-campaign's final BSan gate. It filed two BSan false positives
(variadic parents reported as freed, `strtoul` end pointers), each with a
`#![forbid(unsafe_code)]` probe. It also gave 12 slow tests `#[cfg_attr(bsan, ignore)]`;
every other instrument still runs them.

The sub-campaign wall runs from wave 0's first launch (2026-10-10 18:04) to review wave 3's
landing (2026-10-11 06:11). It includes the operator pause after wave 1's translation, about
an hour between sessions. Only directories holding a `report.md` count as reports: review
wave 3 also committed its BSan gate logs under `bsan-gate-before/`.

### `libgit2/config`

By the operator's decision, `config`'s translators ran `openrouter/openai/gpt-6.1-sol` through
Codex, which crustify's backend drives at reasoning effort `high` for that model.
Before launch, an OpenRouter smoke run confirmed `high` reached the API and the model
produced reasoning tokens. Reviews stayed on `claude-opus-5.5`. Six waves wrapped
`config.h` and `sys/config.h` (9 types, 47 symbols and callbacks), with every item and
in-scope field anchored. Translation cost `$24.53` over 13 batches (6 to 28 minutes each),
against `$85.11` for review. A 25-symbol batch cost `$2.46` here, against `$27`–`$35`
for Opus batches of the same size in `object`. The translations were also leaner: 4 to 11
`#[test]`s per batch where `object`'s Opus batches added 9 to 51.

The reviews filed 46 reports (9, 12, 8, 7, 9 and 1 per review wave) and fixed them in
place. Review wave 1 found a safe-API UB in a wave 1 wrapper
(`ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry`). Review wave 3 found two
`unsafe` reports: value-filter callbacks projecting outside an impl, and an iterator-adoption
helper published as a raw `unsafe fn`. Review wave 4 found a transaction rollback that panics
in `Drop` (`internal`) and a writable `get` that ignored the backend's `get` slot (`equiv`).

Wave 4 batch 0 (`git_config_backend`) edited the C target. It found
`git_config_backend_from_string` and `git_config_backend_from_values` listed as the type's
allocating constructors, wrapped them, and when they failed to link (they are declared
`extern`, not `GIT_EXTERN`, under `-fvisibility=hidden`), it marked them `GIT_EXTERN` in
`include/git2/sys/config.h` and validated against private C builds. The Scope note excluded
those two declarations as unlinkable. Nothing in a translator's context says so: the
exclusion lives only here, and the translator prompt allows C changes as long as they are
built privately. The reviewer kept the change, and the shared prebuilds were rebuilt with it
after review wave 4.

After the sub-campaign, by the operator's decision, the orchestrator reverted the export
(`c5440d3ee`). The commit restores the header, removes `Backend::from_string`/`from_values`
and their allowlist entries, and deletes the 16 tests, their helpers, 4 emptied test modules
and 5 `compile_fail` examples that depended on them (`-1347` lines). It rebuilds
the shared prebuilds without the exports. Plain and ASan+UBSan workspace tests, clippy, and
plain `offline`/`util` ctest pass after it. Union coverage fell from `43.17`% to `42.82`% C
(`98.80`% to `98.75`% Rust).

The two functions had been the only working way to build a backend, so the operator had
`git_config_backend` translated again in one `claude-opus-5.5` batch, shown as the
`libgit2/config` wave `4-followup` batch `0` rows. Its `task.json` is the scheduled entry verbatim,
landed through `crustify/waves/libgit2/config/wave-4-followup`. Shared
memory told it why the batch was rerun. It added the public constructor through the
author's path. A `CustomBackend` trait puts a Rust store behind every slot.
`UnopenedBackend::new`/`new_readonly` box it after a `git_config_init_backend`-initialized
base, with trampolines that refuse reentry and defer a release requested mid-callback. The
batch also rebuilt the `BackendMut` tests on such a store: unit (Miri), `ub_safe`, and
`equiv` against a C-ABI backend serving the same store. It cost `$11.63` over `39m04s`,
with no C change and no smell delta. Its review, one `claude-opus-5.5` batch on the same
item (`$8.96`, `26m55s`), filed four reports and fixed them in place. The one `equiv`
finding: a custom backend's `get` miss overwrote the thread's error where C leaves it.
Two `misc` reports corrected docs, one still naming the reverted constructors. One
`wavefront` report completed the implementer contracts of the backend slots. The
Overview's `config` row includes the cost of both batches, but not their wall time.

Codex agents do not load the shared agent memory that Claude Code reads at startup, so
`config`'s translators worked without the campaign's notes and wrote none back. Wave 0
batch 2 rewrote the whole of `crustify/wavefront/ownership-store.json` with wavefront's
re-sort (`+5904/-5784`), which a memory note warns against. The reviewers read and updated the
memory as before. A crustify fix that passes the notes to non-Claude backends exists on
branch `fix/codex-shared-memory` but was not deployed for this sub-campaign, so the Opus
and GPT translation rows also differ in memory access.

### `libgit2/index`

Three waves wrapped `index.h`, `pathspec.h`, the `git_diff` and `git_diff_delta` types it
needs, and the notes and index iterators (18 types, 75 symbols and callbacks). The
reviews filed 30 reports (13, 11 and 6 per review wave) and fixed them in place. Two were
UB through the API: an empty-pattern over-read reachable through the unsafe diff-options
seam (`ub_unsafe`), and an over-read in `git_index_write`'s racily-clean scan of an
index holding an empty path (`ub_safe`). The final review's BSan gate passed on every
suite, and no wave changed the C target.

By the operator's decision, wave 0's translators ran `openrouter/openai/gpt-6-astra` at
reasoning effort `high`, through Codex; waves 1 and 2 ran `claude-opus-5.5`. OpenAI's
filter refused three of the nine wave 0 sessions as a "possible cybersecurity risk".
Batch 7 survived the refusal messages and landed. Batches 5 and 6 were cut off, after
7 and 17 minutes, while writing allocator-failure and sanitizer tests, the work the
translator prompt asks for. Batch 5 had also edited `src/libgit2/notes.c` to initialize a
`git_tree *` freed uninitialized when `git_commit_tree` fails. The operator had the edit
discarded and both batches rerun on `claude-opus-5.5` in their retained worktrees. The
rerun of batch 5 works around that defect in Rust and leaves C unchanged. Each of those
two rows counts both attempts in `$`, `tokens` and `wall`; the first attempts' records
are under each batch's `attempt-1/`. Wave 0 cost `$125.51`: `$96.52` for the seven
batches that landed on `gpt-6-astra`, `$19.65` for the two refused attempts, and
`$9.34` for the Opus reruns.

Batch 0's `diff_not_sync` trybuild case expected rustc's `core/src/marker.rs` source
snippet, which appears only with the `rust-src` component, so `ub_safe`'s `compile_fail`
test failed on every variant from that landing until review wave 0 fixed it. The shared
memory note on toolchain-dependent compile-fail output exists, but Codex agents do not
load it.

ffibox `5a98173`, which gives `CCell` generic handle constructors from typed and `void *`
pointers, merged to its `main` before wave 2. The crate built and passed every suite
against it unchanged; its adoption, a generic `git_object` downcast and the 10
hand-built `from_ptr` copies, is not part of this sub-campaign. The sub-campaign wall
runs from 2026-10-11 19:00 to 2026-10-12 05:37 and includes about four hours of operator
pauses (the refusal decision, then a pause after review wave 1).

### Orchestrator cost

The Overview's assistant row sums the 10 `assistant-*.usage.json` records in
`crustify/artifacts/assistant/`: `$438.96` for `337.45M` tokens over `98.0h` of session wall,
all `openrouter/anthropic/claude-opus-5.5`. Three of them were recovered from the session
transcripts under the container's `/work/.claude/projects/-target/`:

- `assistant-20261011-000919`, session `90f46020`: the running orchestrator, `$267.21` and
  `185.15M` through its last request at `2026-10-12T18:33:53Z`. The container exited on
  `2026-10-13` before the session ended, so it never wrote its record.
- `assistant-20261008-162351` (`4beea589`, `$0.53`) and `assistant-20261008-163758`
  (`7898a051`, `$2.05`): launches before this campaign id was minted, which left a transcript
  but no log.

The row excludes the CLI's auxiliary requests, which the transcript does not record. Also absent
from every table: the `followups/ffibox-ref-from-ptr` review batch (session `4ac75278`, `$0.81`,
`1.06M` tokens), whose worktree transcript survives but which wrote no record under the campaign
dir.

### Dashboard moved to the current template

On 2026-10-20 the dashboard was rebuilt in the current translate template's format. The
figures are the ones recorded before; what moved, and what it left blank:

- `Settings` replaces `Campaign` and `Review pass`. Those sections recorded settings that
  changed across subsystems, which the `Settings` lines cannot carry; as they stood:

  Campaign:
  - **target repo** — `libgit2` @ `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`
  - **target** — `libgit2` (`libgit2.so` public API, deprecated items excluded)
  - **campaign objective** — `wrap`
  - **`impl_files`** — `src/libgit2/`, `src/util/`, `deps/xdiff/`, `deps/reftable/`, `deps/ntlmclient/`, `deps/llhttp/`
  - **`api_headers`** — `include/git2.h`, `include/git2/*.h`, `include/git2/sys/*.h`, except `deprecated.h`, `cred_helpers.h`, `stdint.h` and `sys/cred.h`
  - **agent backend** — `claude`; `codex` for `libgit2/config` and `libgit2/index` wave 0
  - **model** — `openrouter/anthropic/claude-opus-5.5`; `openrouter/openai/gpt-6.1-sol` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` (reasoning effort `high`) for `libgit2/index` wave 0, except batches 5 and 6, rerun on `openrouter/anthropic/claude-opus-5.5`
  - **`--billing`** — `api`
  - **`--max-types`** — `2`
  - **`--max-syms`** — `25`
  - **`--max-loc`** — `500`
  - **`--min-fields`** — `20`
  - **orchestrator parallelism** — `10` concurrent batch processes through `libgit2/core`; `16` from `libgit2/repository`
  - **deps** — crustify `0fcfcae` (`main`, from `libgit2/core`; through `libgit2/oid` `7912683` with `ffimap` from a local driver port, since superseded by upstream's nightly pin `4efc0cc`; batch and wave `ffimap` records through `libgit2/config` from `4771856`, later ones and every sub-campaign and campaign scan from `cbf6fbc`), ffibox `695d52d` (`main`)
  - **build version** — `1`

  Review pass:
  - **agent backend** — `claude`
  - **model** — `openrouter/anthropic/claude-fable-5.1` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` from `libgit2/object` review wave 1
  - **`--billing`** — `api`
  - **`--max-types`** — `2` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave; from `libgit2/repository`, `20` (10× the translation cap), a route split into near-equal batches only past it
  - **`--max-syms`** — `25` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave; from `libgit2/repository`, `100` symbols and callbacks (4× the translation cap), a route split into near-equal batches only past it
  - **`--max-loc`** — `500` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave
  - **`--min-fields`** — `20` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave
  - **orchestrator parallelism** — `10` concurrent batch processes through `libgit2/core`; `16` from `libgit2/repository`
- The Overview's sub-campaign table is `Subsystem stats`. `raw-lifetime-void` and
  `raw-lifetime-string`, the campaign's foundational subsystems, which the current layout
  names `void` and `string`, keep their landed names as `libgit2/raw-lifetime-*`. The
  per-pass `$/type` and `$/sym` cells are each pass's cost over the subsystem's units; the
  `total` ones are the recorded cells.
- `wrapped symbols` counts `Wraps:` anchors only, so the four scheduled symbols whose only
  anchor is `Replaces:`, composed in Rust over C defects, now count as ported:
  `git_commit_create_from_stage`, `git_commit_create_from_tree`,
  `git_repository_commit_parents` and `git_repository_head_detached_for_worktree`.
- The `Batches — review` and `Batches — review: tests` tables are the `review` rows of
  each kind's tables. Their `+LoC` is the old `rv loc`, which equalled it; a review row's
  `fields` are the field anchors of the translation batches it reviewed, from their
  `task.json`; no review row recorded `loc`.
- `Raw lifetime discovery` and its `Review` are `libgit2/raw-lifetime-*` rows of the
  Symbols tables, with the Overview's `2` symbols each. Their `+LoC` counts the `.rs` lines
  of their landing commits under `crustify/rust/`, as every other `+LoC` does; no `loc`,
  `cov` or test counts were recorded for them, so they are absent from the tests tables and
  from `Σ … loc`, `$/loc`. With them the Symbols `Σ first` holds the Overview's `389`
  symbols, and the two kinds' `$` cells add up to the Overview's batch and review cost.
- `libgit2/config`'s follow-up wave, which the old rows listed as a second wave `4`,
  batch `0`, is wave `4-followup`, as its artifact dirs and branches name it.
- `+rejected PoCs` is gone: it read `0` in every row.
- The `Batches: safety audit` tables are new. Each cell is the change between the batch's
  recorded scans, `ffimap.json` at its landing and `ffimap.base.json` at the tip it landed
  on (`ffimap.provenance.json`; crustify `1a7d434`). Those scans predate the
  `C borrow`, `slice borrow`, `Send/Sync` and `C by value` counters, which read `—`: they need a
  rescan with the current `ffimap`. The old `Final tally overview` and `All metrics`
  tables were never filled and are dropped.
- `Planning` is empty: the plan was made by the orchestrator, before planner runs
  existed.
- The `assistant` row is the former `orchestrator` row; its records are re-derived in
  `crustify/artifacts/assistant/` (see Orchestrator cost).

The dashboard before the move is `git show 0827c7651:crustify/artifacts/dashboard.md`; its
Legend defines the old columns.
