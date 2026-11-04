# crustify — `libgit2 / libgit2`

Copy this template into the campaign checkout, then fill it as work lands.

## Settings

As answered in `crustify/CRUSTIFY.md`; a change during the campaign is in its history
and in Notes.

- **source** — `https://github.com/libgit2/libgit2.git` @ `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`
- **campaign** — `wrap`
- **scope** — the whole public API of `libgit2.so`, deprecated items excluded
- **translate-agent** — `openrouter/anthropic/claude-opus-5.5`, `api`
- **review-agent** — `openrouter/anthropic/claude-opus-5.5`, `api`; objectives `all`
- **plan-agent** — `anthropic/claude-opus-5-5`, `subscription`; `2` runs
- **identity** — author `Marius Momeu <marius.momeu@berkeley.edu>`, co-author `crustify <crustify@berkeley.edu>`, from 2026-11-03; `crustify` as the author before
- **workload** — max-types `2`, max-syms `25`, max-loc `500`, parallelism `assistant's choice` from 2026-11-03, `32` in the annotation pass (wave `525`), `16` before
- **review-workload** — orchestrator's choice: max-types `20`, max-syms `100`
- **sanitizers** — `asan-ubsan`, `tsan`, `bsan`
- **target-config** — default, deprecated features disabled (`-DDEPRECATE_HARD=ON`), HTTPS through OpenSSL, SSH, Negotiate auth and i18n off, threads on; OpenSSL, PCRE2 and zlib from the side campaigns' installs of each variant; `build.json` version `3`

## Legend

- `module` / `wave` / `batch` — the batch's place in `schedule.json`: its wave's module and
  the wave and batch indices
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
- `cost` / `review cost` / `total` — a module's translation, review and combined cost;
  `module wall` — first launch to final gate; `Σ recorded agents` leaves the assistant
  out, `Σ campaign` adds it, without per-unit cells
- `UB safe tests` … `Unit tests` — `#[test]` functions per suite, each with the coverage of
  running it alone; `Total tests` — all four, whose coverage is one joint run's union,
  never the sum
- `total`, `ported` and `wrapped` types and symbols — the scheduled public-API units, and
  those with a `Replaces:` or a `Wraps:` anchor at the tip; `share` is of the total

Every table below is a heading, a model line and the table. All prose belongs
in Notes.

## Overview

- **branch** — `crustify/devel`, tip `<sha>`; `main` shipped at `<sha>`; crustify `4605156`, ffibox `7950244` (`main`), wavefront `db28284`
- **C test baseline** — `3443/3498`, `55` skipped; disabled: ctest targets `invasive`, `gitdaemon`, `gitdaemon_namespace`, `gitdaemon_sha256`, `ssh`, `online`, `online_customcert`, `proxy`, `auth_clone`, `auth_clone_and_push`
- **Rust LoC** — `<n>` non-test, `<n>` tests; **C LoC** — `<n>`

### Coverage stats

| | count | share | C LoC coverage | Rust LoC coverage |
|---|---:|---:|---:|---:|
| UB safe tests | `694` | — | `28.38`% | `79.75`% |
| UB unsafe tests | `91` | — | `19.36`% | `12.13`% |
| Equiv tests | `489` | — | `43.44`% | `75.60`% |
| Unit tests | `791` | — | `32.00`% | `87.46`% |
| **Total tests** | **`2065`** | — | **`45.87`%** | **`98.93`%** |
| total types | `258` | — | — | — |
| ported types | `0` | `0.00`% | — | — |
| wrapped types | `105` | `40.70`% | — | — |
| total symbols | `979` | — | — | — |
| ported symbols | `4` | `0.41`% | — | — |
| wrapped symbols | `448` | `45.76`% | — | — |

### Module stats

Implementation `openrouter/anthropic/claude-opus-5.5` via `claude`, `openrouter/openai/gpt-6.1-sol` via `codex` for `libgit2/config`, `openrouter/openai/gpt-6-astra` via `codex` for `libgit2/index` wave 0; review
`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0, `openrouter/anthropic/claude-opus-5.5` via `claude` from its review wave 1.

| module | nr types | nr symbols | module wall | cost | tokens | $/type | $/sym | review cost | review tokens | review $/type | review $/sym | total | total tokens | total $/type | total $/sym |
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
| side campaigns | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| planners | — | — | `1h26m11s` | — | — | — | — | — | — | — | — | `$33.01` | `91.28M` | — | — |
| assistant | — | — | — | — | — | — | — | — | — | — | — | `$438.96` | `337.45M` | — | — |
| **Σ recorded agents** | **`95`** | **`389`** | **`52h27m`** | **`$896.34`** | **`2391.61M`** | **`$9.44`** | **`$2.30`** | **`$701.91`** | **`1175.51M`** | **`$7.39`** | **`$1.80`** | **`$1631.26`** | **`3658.40M`** | **`$17.17`** | **`$4.19`** |
| **Σ campaign** | **`95`** | **`389`** | **`52h27m`** | **`$896.34`** | **`2391.61M`** | — | — | **`$701.91`** | **`1175.51M`** | — | — | **`$2070.22`** | **`3995.85M`** | — | — |

## Safety audit

Deterministic `ffimap <workdir> --json`; no model. Every counter is in the `ffimap.json`
tracked in the campaign's artifact dir, in each wave's `waves/wave-<index>/` and in each
batch run's dir.

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

| run | modules changed | items moved | wavefront artifacts changed | reports | $ | tokens | wall |
|---|---:|---:|---|---:|---|---|---|
| 1 | `12` | `167` | `configs/wavefront-config.json`, `configs/libgit2/*/wavefront-config.json` (22), `t3/chased_casts.csv`, `t3/type_generators.csv` | `8` | `$13.36` | `37.11M` | `32m23s` |
| 2 | `196` | `1144` | `configs/wavefront-config.json`, `configs/libgit2/*/wavefront-config.json` (196, replacing 22), `t3/chased_casts.csv`, `ownership-store.json` | `1` | `$19.65` | `54.17M` | `53m48s` |
| **Σ** | **`208`** | **`1311`** | | **`9`** | **`$33.01`** | **`91.28M`** | **`86m11s`** |

## Side campaigns

One row per out-of-tree library the campaign wraps in a side campaign, as
`specs/dependencies.md` describes. Each wrapper's own dashboard, in its
`<source>-bindings` repository, is its record; its row here copies that dashboard's
figures as of the commit the campaign pins, and the `side campaigns` row of `Module stats`
sums them.

| library | repository | pin | shipped at | linked from | waves | types | symbols | wall | cost | tokens | review cost | review tokens | total | total tokens |
|---|---|---|---|---|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|
| **Σ** | | | | | **`0`** | **`0`** | **`0`** | — | **`$0.00`** | **`0.00M`** | **`$0.00`** | **`0.00M`** | **`$0.00`** | **`0.00M`** |

## Types

### Batches — wrap

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | pass | types | fields | +LoC | cov (C/Rust) | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/util` | `4` | `0` | first | `2` | `5` | `+1934/-2` | `1.30`% / `98.16`% | `$7.55` | `19.09M` | `33m43s` | `$3.77` | `$1.51` |
| `libgit2/util` | `4` | `1` | first | `2` | `0` | `+849/-26` | `1.35`% / `98.34`% | `$5.25` | `12.24M` | `38m51s` | `$2.62` | — |
| `libgit2/oid` | `8` | `0` | first | `2` | `2` | `+1514/-4` | `1.73`% / `99.82`% | `$9.89` | `28.93M` | `49m08s` | `$4.95` | `$4.95` |
| `libgit2/oid` | `8` | `1` | first | `1` | `0` | `+488/-0` | `1.68`% / `99.76`% | `$2.87` | `6.58M` | `14m31s` | `$2.87` | — |
| `libgit2/oid` | `9` | `0` | first | `1` | `2` | `+1397/-0` | `5.89`% / `100.00`% | `$5.30` | `12.40M` | `19m02s` | `$5.30` | `$2.65` |
| `libgit2/core` | `14` | `0` | first | `2` | `3` | `+2495/-315` | `15.24`% / `99.77`% | `$14.63` | `28.15M` | `54m01s` | `$7.32` | `$4.88` |
| `libgit2/core` | `14` | `1` | first | `2` | `0` | `+909/-0` | `9.22`% / `99.59`% | `$2.54` | `5.29M` | `16m38s` | `$1.27` | — |
| `libgit2/core` | `14` | `2` | first | `2` | `0` | `+1784/-6` | `12.15`% / `99.67`% | `$7.38` | `20.11M` | `28m17s` | `$3.69` | — |
| `libgit2/core` | `14` | `3` | first | `2` | `0` | `+871/-19` | `10.96`% / `99.64`% | `$5.25` | `13.05M` | `22m58s` | `$2.62` | — |
| `libgit2/core` | `14` | `4` | first | `2` | `2` | `+2575/-6` | `14.28`% / `99.74`% | `$12.07` | `30.57M` | `43m43s` | `$6.04` | `$6.04` |
| `libgit2/core` | `14` | `5` | first | `2` | `0` | `+839/-0` | `9.19`% / `99.52`% | `$3.05` | `6.68M` | `13m22s` | `$1.52` | — |
| `libgit2/core` | `14` | `6` | first | `2` | `3` | `+1134/-3` | `9.42`% / `99.62`% | `$4.22` | `10.43M` | `17m55s` | `$2.11` | `$1.41` |
| `libgit2/core` | `14` | `7` | first | `1` | `3` | `+2428/-3` | `13.87`% / `99.71`% | `$10.74` | `30.16M` | `33m33s` | `$10.74` | `$3.58` |
| `libgit2/core` | `16` | `0` | first | `1` | `3` | `+1745/-3` | `18.22`% / `99.82`% | `$10.23` | `31.28M` | `26m16s` | `$10.23` | `$3.41` |
| `libgit2/repository` | `20` | `0` | first | `1` | `0` | `+1799/-0` | `32.72`% / `99.69`% | `$24.34` | `54.96M` | `1h23m54s` | `$24.34` | — |
| `libgit2/repository` | `20` | `1` | first | `1` | `0` | `+3376/-0` | `31.33`% / `99.79`% | `$16.26` | `50.26M` | `55m56s` | `$16.26` | — |
| `libgit2/repository` | `20` | `2` | first | `2` | `0` | `+1925/-0` | `19.74`% / `99.86`% | `$8.59` | `24.90M` | `29m29s` | `$4.29` | — |
| `libgit2/repository` | `20` | `3` | first | `1` | `0` | `+1789/-0` | `25.32`% / `99.77`% | `$11.26` | `34.18M` | `42m24s` | `$11.26` | — |
| `libgit2/repository` | `20` | `4` | first | `2` | `0` | `+1635/-1` | `23.97`% / `99.79`% | `$11.67` | `36.52M` | `35m52s` | `$5.83` | — |
| `libgit2/repository` | `20` | `5` | first | `2` | `0` | `+1585/-0` | `23.69`% / `99.79`% | `$10.66` | `33.24M` | `32m15s` | `$5.33` | — |
| `libgit2/repository` | `20` | `6` | first | `2` | `0` | `+2089/-0` | `24.02`% / `99.77`% | `$10.91` | `33.42M` | `38m41s` | `$5.46` | — |
| `libgit2/repository` | `20` | `7` | first | `1` | `0` | `+1491/-194` | `25.76`% / `99.78`% | `$15.47` | `52.12M` | `51m17s` | `$15.47` | — |
| `libgit2/repository` | `20` | `8` | first | `2` | `0` | `+1279/-0` | `18.62`% / `99.83`% | `$4.27` | `9.83M` | `18m41s` | `$2.13` | — |
| `libgit2/repository` | `20` | `9` | first | `2` | `0` | `+1516/-0` | `19.22`% / `99.85`% | `$6.34` | `17.05M` | `27m40s` | `$3.17` | — |
| `libgit2/repository` | `20` | `10` | first | `2` | `0` | `+1587/-1` | `25.67`% / `99.77`% | `$7.92` | `22.15M` | `45m21s` | `$3.96` | — |
| `libgit2/repository` | `20` | `11` | first | `2` | `2` | `+1545/-0` | `18.98`% / `99.84`% | `$5.41` | `12.69M` | `23m56s` | `$2.71` | `$2.71` |
| `libgit2/repository` | `22` | `0` | first | `2` | `12` | `+2748/-28` | `33.79`% / `99.72`% | `$10.14` | `30.40M` | `28m55s` | `$5.07` | `$0.85` |
| `libgit2/repository` | `22` | `1` | first | `1` | `5` | `+1563/-3` | `33.24`% / `99.70`% | `$6.36` | `17.21M` | `20m02s` | `$6.36` | `$1.27` |
| `libgit2/object` | `26` | `0` | first | `2` | `2` | `+1681/-3` | `36.70`% / `99.33`% | `$6.21` | `16.56M` | `31m12s` | `$3.10` | `$3.10` |
| `libgit2/object` | `26` | `1` | first | `2` | `0` | `+2706/-1` | `37.91`% / `99.24`% | `$17.37` | `56.17M` | `54m52s` | `$8.69` | — |
| `libgit2/object` | `26` | `2` | first | `2` | `2` | `+2491/-0` | `37.67`% / `99.30`% | `$10.57` | `25.58M` | `47m19s` | `$5.29` | `$5.29` |
| `libgit2/object` | `26` | `3` | first | `2` | `0` | `+2509/-3` | `36.94`% / `99.30`% | `$14.40` | `45.71M` | `42m11s` | `$7.20` | — |
| `libgit2/object` | `26` | `4` | first | `2` | `0` | `+1791/-0` | `36.85`% / `99.31`% | `$8.08` | `24.57M` | `32m20s` | `$4.04` | — |
| `libgit2/object` | `26` | `5` | first | `1` | `0` | `+620/-0` | `36.19`% / `99.32`% | `$2.46` | `5.55M` | `13m29s` | `$2.46` | — |
| `libgit2/object` | `28` | `0` | first | `2` | `4` | `+2198/-10` | `38.08`% / `99.28`% | `$8.50` | `24.63M` | `27m01s` | `$4.25` | `$2.12` |
| `libgit2/object` | `28` | `1` | first | `2` | `7` | `+3634/-350` | `38.27`% / `99.33`% | `$15.08` | `46.58M` | `38m26s` | `$7.54` | `$2.15` |
| `libgit2/object` | `28` | `2` | first | `2` | `7` | `+2338/-49` | `38.16`% / `99.28`% | `$10.87` | `30.98M` | `36m44s` | `$5.43` | `$1.55` |
| `libgit2/object` | `28` | `3` | first | `2` | `0` | `+3188/-330` | `38.25`% / `99.32`% | `$17.37` | `57.50M` | `42m30s` | `$8.69` | — |
| `libgit2/object` | `28` | `4` | first | `1` | `4` | `+1169/-9` | `38.05`% / `99.28`% | `$4.42` | `11.16M` | `17m24s` | `$4.42` | `$1.10` |
| `libgit2/object` | `30` | `0` | first | `1` | `9` | `+1784/-9` | `39.72`% / `99.39`% | `$6.63` | `17.54M` | `28m41s` | `$6.63` | `$0.74` |
| `libgit2/config` | `34` | `0` | first | `2` | `3` | `+536/-0` | `42.24`% / `99.44`% | `$1.54` | `5.82M` | `11m22s` | `$0.77` | `$0.51` |
| `libgit2/config` | `34` | `1` | first | `2` | `0` | `+743/-1` | `42.52`% / `99.41`% | `$2.54` | `10.00M` | `24m29s` | `$1.27` | — |
| `libgit2/config` | `36` | `0` | first | `2` | `9` | `+772/-2` | `42.53`% / `99.43`% | `$1.86` | `7.12M` | `14m33s` | `$0.93` | `$0.21` |
| `libgit2/config` | `38` | `0` | first | `1` | `2` | `+487/-1` | `42.74`% / `99.44`% | `$1.67` | `5.71M` | `16m30s` | `$1.67` | `$0.83` |
| `libgit2/config` | `40` | `0` | first | `1` | `4` | `+1131/-24` | `42.79`% / `99.45`% | `$1.95` | `6.79M` | `19m01s` | `$1.95` | `$0.49` |
| `libgit2/config` | `42` | `0` | first | `1` | `14` | `+1118/-35` | `43.11`% / `99.45`% | `$2.49` | `9.45M` | `24m50s` | `$2.49` | `$0.18` |
| `libgit2/config` | `46` | `0` | first | `1` | `14` | `+2746/-5` | `42.84`% / `99.46`% | `$11.63` | `34.73M` | `39m04s` | `$11.63` | `$0.83` |
| `libgit2/index` | `48` | `0` | first | `2` | `0` | `+1340/-43` | `45.00`% / `99.50`% | `$20.29` | `12.38M` | `27m57s` | `$10.14` | — |
| `libgit2/index` | `48` | `1` | first | `2` | `0` | `+431/-0` | `42.82`% / `99.49`% | `$5.76` | `2.63M` | `7m04s` | `$2.88` | — |
| `libgit2/index` | `48` | `2` | first | `2` | `0` | `+522/-0` | `44.19`% / `99.50`% | `$16.64` | `10.75M` | `19m17s` | `$8.32` | — |
| `libgit2/index` | `48` | `3` | first | `2` | `0` | `+402/-5` | `44.19`% / `99.50`% | `$13.75` | `8.80M` | `14m41s` | `$6.88` | — |
| `libgit2/index` | `48` | `4` | first | `2` | `3` | `+543/-0` | `44.14`% / `99.50`% | `$9.70` | `5.50M` | `12m25s` | `$4.85` | `$3.23` |
| `libgit2/index` | `48` | `5` | first | `2` | `2` | `+914/-1` | `45.58`% / `99.51`% | `$12.84` | `19.71M` | `36m20s` | `$6.42` | `$6.42` |
| `libgit2/index` | `48` | `6` | first | `2` | `0` | `+889/-520` | `45.46`% / `99.50`% | `$16.15` | `16.46M` | `36m16s` | `$8.07` | — |
| `libgit2/index` | `48` | `7` | first | `1` | `0` | `+1226/-1` | `45.43`% / `99.50`% | `$21.81` | `13.93M` | `40m51s` | `$21.81` | — |
| `libgit2/index` | `50` | `0` | first | `2` | `14` | `+1543/-14` | `45.80`% / `99.53`% | `$5.84` | `14.76M` | `23m16s` | `$2.92` | `$0.42` |
| `libgit2/index` | `50` | `1` | first | `1` | `3` | `+806/-5` | `45.85`% / `99.54`% | `$6.21` | `17.23M` | `25m45s` | `$6.21` | `$2.07` |
| `libgit2/src/libgit2/indexer.c` | `172` | `0` | first | `1` | `7` | `+727/-0` | — | `$5.75` | `16.14M` | `26m16s` | `$5.75` | `$0.82` |
| `libgit2/src/libgit2/odb.c` | `176` | `0` | first | `2` | `5` | `+1256/-20` | — | `$9.40` | `29.40M` | `38m40s` | `$4.70` | `$1.88` |
| `libgit2/src/libgit2/odb.c` | `176` | `1` | first | `2` | `14` | `+2132/-4` | — | `$13.38` | `42.08M` | `53m07s` | `$6.69` | `$0.96` |
| `libgit2/src/libgit2/odb.c` | `180` | `0` | first | `1` | `16` | `+1599/-22` | — | `$9.04` | `25.94M` | `33m36s` | `$9.04` | `$0.56` |
| `libgit2/src/libgit2/diff_parse.c` | `228` | `0` | first | `1` | `2` | `+756/-8` | — | `$3.11` | `7.21M` | `15m29s` | `$3.11` | `$1.55` |
| `libgit2/src/libgit2/checkout.c` | `330` | `0` | first | `1` | `3` | `+490/-4` | — | `$4.39` | `11.97M` | `17m30s` | `$4.39` | `$1.46` |
| `libgit2/src/libgit2/checkout.c` | `334` | `0` | first | `1` | `20` | `+1435/-433` | `45.87`% / `98.93`% | `$12.11` | `36.14M` | `41m32s` | `$12.11` | `$0.61` |
| `libgit2/util` | `6` | `0` | review | `2` | `5` | `+172/-15` | `1.63`% / `99.74`% | `$20.66` | `23.31M` | `48m16s` | `$10.33` | `$4.13` |
| `libgit2/util` | `6` | `1` | review | `2` | `0` | `+149/-12` | `1.63`% / `98.42`% | `$7.58` | `10.10M` | `18m18s` | `$3.79` | — |
| `libgit2/oid` | `11` | `0` | review | `2` | `2` | `+224/-6` | `7.81`% / `99.48`% | `$14.93` | `18.05M` | `29m10s` | `$7.46` | `$7.46` |
| `libgit2/oid` | `11` | `1` | review | `1` | `0` | `+185/-8` | `7.81`% / `99.47`% | `$7.16` | `8.57M` | `19m44s` | `$7.16` | — |
| `libgit2/oid` | `12` | `0` | review | `1` | `2` | `+235/-18` | `7.80`% / `99.49`% | `$11.49` | `15.40M` | `22m41s` | `$11.49` | `$5.75` |
| `libgit2/core` | `15` | `0` | review | `15` | `11` | `+263/-9` | `15.84`% / `99.81`% | `$24.78` | `30.19M` | `43m19s` | `$1.65` | `$2.25` |
| `libgit2/core` | `17` | `0` | review | `1` | `3` | `+126/-7` | `18.23`% / `99.82`% | `$18.99` | `21.31M` | `39m44s` | `$18.99` | `$6.33` |
| `libgit2/repository` | `21` | `0` | review | `20` | `2` | `+743/-98` | `32.73`% / `99.69`% | `$32.90` | `55.40M` | `46m45s` | `$1.65` | `$16.45` |
| `libgit2/repository` | `23` | `0` | review | `3` | `17` | `+227/-11` | `36.05`% / `99.30`% | `$21.87` | `26.76M` | `41m52s` | `$7.29` | `$1.29` |
| `libgit2/object` | `27` | `0` | review | `11` | `4` | `+1767/-853` | `37.98`% / `99.26`% | `$128.50` | `105.82M` | `3h16m09s` | `$11.68` | `$32.12` |
| `libgit2/object` | `29` | `0` | review | `9` | `22` | `+789/-37` | `39.49`% / `99.37`% | `$12.34` | `40.09M` | `30m23s` | `$1.37` | `$0.56` |
| `libgit2/object` | `31` | `0` | review | `1` | `9` | `+179/-26` | `42.18`% / `99.43`% | `$4.94` | `13.02M` | `15m40s` | `$4.94` | `$0.55` |
| `libgit2/config` | `35` | `0` | review | `4` | `3` | `+646/-125` | `42.52`% / `99.42`% | `$7.75` | `21.91M` | `42m25s` | `$1.94` | `$2.58` |
| `libgit2/config` | `37` | `0` | review | `2` | `9` | `+796/-17` | `42.72`% / `99.44`% | `$14.18` | `43.14M` | `49m58s` | `$7.09` | `$1.58` |
| `libgit2/config` | `39` | `0` | review | `1` | `2` | `+366/-151` | `42.75`% / `99.46`% | `$6.29` | `17.40M` | `25m11s` | `$6.29` | `$3.15` |
| `libgit2/config` | `41` | `0` | review | `1` | `4` | `+627/-239` | `42.82`% / `99.45`% | `$8.39` | `24.83M` | `28m21s` | `$8.39` | `$2.10` |
| `libgit2/config` | `43` | `0` | review | `1` | `14` | `+631/-286` | `43.14`% / `99.45`% | `$8.66` | `25.44M` | `27m14s` | `$8.66` | `$0.62` |
| `libgit2/config` | `47` | `0` | review | `1` | `14` | `+198/-26` | `42.79`% / `99.49`% | `$8.96` | `25.80M` | `26m55s` | `$8.96` | `$0.64` |
| `libgit2/index` | `49` | `0` | review | `15` | `5` | `+738/-79` | `45.75`% / `99.53`% | `$15.85` | `50.35M` | `46m20s` | `$1.06` | `$3.17` |
| `libgit2/index` | `51` | `0` | review | `3` | `17` | `+203/-8` | `46.57`% / `99.53`% | `$12.82` | `35.96M` | `48m41s` | `$4.27` | `$0.75` |
| `libgit2/src/libgit2/indexer.c` | `173` | `0` | review | `1` | `7` | `+10/-7` | — | `$3.93` | `10.78M` | `16m22s` | `$3.93` | `$0.56` |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | review | `4` | `19` | `+64/-8` | — | `$1.37` | `4.35M` | `28m33s` | `$0.34` | `$0.07` |
| `libgit2/src/libgit2/odb.c` | `181` | `0` | review | `1` | `16` | `+39/-14` | — | `$7.12` | `20.62M` | `23m15s` | `$7.12` | `$0.45` |
| `libgit2/src/libgit2/diff_parse.c` | `229` | `0` | review | `1` | `2` | `+6/-3` | — | `$4.69` | `10.34M` | `36m13s` | `$4.69` | `$2.34` |
| `libgit2/src/libgit2/checkout.c` | `331` | `0` | review | `1` | `3` | `+103/-5` | — | `$3.73` | `10.13M` | `16m05s` | `$3.73` | `$1.24` |
| **Σ first** |  |  |  | **`105`** | **`212`** | **`+97542/-2526`** | **`45.87`% / `98.93`%** | **`$582.38`** | **`1417.09M`** | — | **`$5.55`** | **`$2.75`** |
| **Σ review** |  |  |  | **`104`** | **`192`** | **`+9486/-2068`** | **`46.57`% / `99.53`%** | **`$409.88`** | **`669.07M`** | — | **`$3.94`** | **`$2.13`** |

### Batches — wrap: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `libgit2/util` | `4` | `0` | first | `+4` (`+0.02`/`-6.85` pp) | `+7` (`+0.00`/`-10.50` pp) | `+2` (`+0.00`/`-10.62` pp) | `+14` (`+0.04`/`+6.71` pp) |
| `libgit2/util` | `4` | `1` | first | `+4` (`+0.00`/`-3.94` pp) | `+0` (`+0.00`/`-5.56` pp) | `+6` (`+0.05`/`-2.60` pp) | `+10` (`+0.00`/`+1.42` pp) |
| `libgit2/oid` | `8` | `0` | first | `+11` (`+0.09`/`-0.22` pp) | `+0` (`+0.00`/`-10.49` pp) | `+6` (`+0.04`/`-4.69` pp) | `+11` (`+0.09`/`+1.74` pp) |
| `libgit2/oid` | `8` | `1` | first | `+4` (`+0.00`/`+0.27` pp) | `+0` (`+0.00`/`-4.15` pp) | `+7` (`+0.06`/`+0.74` pp) | `+5` (`+0.00`/`+0.69` pp) |
| `libgit2/oid` | `9` | `0` | first | `+8` (`+0.01`/`+2.05` pp) | `+0` (`+0.00`/`-4.51` pp) | `+2` (`+4.15`/`+1.38` pp) | `+8` (`+0.00`/`+1.07` pp) |
| `libgit2/core` | `14` | `0` | first | `+11` (`+0.12`/`+1.49` pp) | `+5` (`+0.00`/`+4.15` pp) | `+4` (`+0.96`/`+0.37` pp) | `+12` (`+0.12`/`+1.23` pp) |
| `libgit2/core` | `14` | `1` | first | `+5` (`+0.00`/`-2.56` pp) | `+0` (`+0.00`/`-3.05` pp) | `+5` (`+0.03`/`-1.89` pp) | `+7` (`+0.00`/`+1.30` pp) |
| `libgit2/core` | `14` | `2` | first | `+7` (`+0.00`/`+0.47` pp) | `+0` (`+0.00`/`-1.44` pp) | `+10` (`+1.19`/`+0.76` pp) | `+9` (`+0.00`/`+0.61` pp) |
| `libgit2/core` | `14` | `3` | first | `+5` (`+0.00`/`+0.32` pp) | `+0` (`+0.00`/`-0.78` pp) | `+4` (`+1.54`/`-0.56` pp) | `+8` (`+0.00`/`+0.33` pp) |
| `libgit2/core` | `14` | `4` | first | `+13` (`+0.12`/`+1.50` pp) | `+0` (`+0.00`/`-1.38` pp) | `+6` (`+0.04`/`-0.06` pp) | `+18` (`+0.52`/`+0.59` pp) |
| `libgit2/core` | `14` | `5` | first | `+4` (`+0.00`/`-0.59` pp) | `+0` (`+0.00`/`-1.52` pp) | `+7` (`+1.39`/`+0.31` pp) | `+3` (`+0.00`/`+0.66` pp) |
| `libgit2/core` | `14` | `6` | first | `+10` (`+0.00`/`+0.88` pp) | `+0` (`+0.00`/`-1.74` pp) | `+9` (`+0.20`/`-0.42` pp) | `+13` (`+0.00`/`+0.75` pp) |
| `libgit2/core` | `14` | `7` | first | `+18` (`+7.17`/`+0.31` pp) | `+0` (`+0.00`/`-2.03` pp) | `+4` (`+1.70`/`-1.93` pp) | `+14` (`+0.04`/`+0.88` pp) |
| `libgit2/core` | `16` | `0` | first | `+11` (`+0.21`/`+1.00` pp) | `+0` (`+0.00`/`-2.05` pp) | `+6` (`+0.02`/`-0.08` pp) | `+8` (`+0.18`/`+0.21` pp) |
| `libgit2/repository` | `20` | `0` | first | `+7` (`+2.18`/`+0.15` pp) | `+0` (`+0.03`/`-0.90` pp) | `+2` (`+1.08`/`+0.08` pp) | `+5` (`+0.42`/`-0.11` pp) |
| `libgit2/repository` | `20` | `1` | first | `+8` (`+3.49`/`+0.11` pp) | `+12` (`+6.85`/`+0.54` pp) | `+4` (`+5.84`/`+0.48` pp) | `+15` (`+0.00`/`+0.92` pp) |
| `libgit2/repository` | `20` | `2` | first | `+9` (`+2.35`/`+0.41` pp) | `+3` (`+5.08`/`+0.94` pp) | `+5` (`+0.52`/`+0.21` pp) | `+11` (`+2.64`/`+0.37` pp) |
| `libgit2/repository` | `20` | `3` | first | `+15` (`+1.38`/`+0.16` pp) | `+0` (`+0.01`/`-0.26` pp) | `+7` (`+1.28`/`-0.12` pp) | `+6` (`+1.17`/`+0.13` pp) |
| `libgit2/repository` | `20` | `4` | first | `+8` (`+0.37`/`+0.13` pp) | `+0` (`+0.01`/`-0.63` pp) | `+9` (`+0.27`/`+0.16` pp) | `+10` (`+0.06`/`+0.24` pp) |
| `libgit2/repository` | `20` | `5` | first | `+7` (`+0.03`/`-0.34` pp) | `+4` (`+3.44`/`+0.38` pp) | `+5` (`+3.92`/`-0.43` pp) | `+10` (`+3.50`/`+0.28` pp) |
| `libgit2/repository` | `20` | `6` | first | `+14` (`+0.19`/`-0.08` pp) | `+0` (`-0.01`/`-0.82` pp) | `+5` (`+0.06`/`-0.90` pp) | `+12` (`+0.07`/`+0.37` pp) |
| `libgit2/repository` | `20` | `7` | first | `+7` (`+0.34`/`-0.46` pp) | `+0` (`+0.02`/`-0.85` pp) | `+3` (`+0.07`/`-0.58` pp) | `+10` (`+0.10`/`+0.29` pp) |
| `libgit2/repository` | `20` | `8` | first | `+5` (`-0.01`/`-0.75` pp) | `+0` (`+0.01`/`-1.76` pp) | `+6` (`+0.39`/`-0.12` pp) | `+6` (`+0.00`/`+0.63` pp) |
| `libgit2/repository` | `20` | `9` | first | `+6` (`+0.01`/`-0.71` pp) | `+0` (`-0.01`/`-1.35` pp) | `+9` (`+0.24`/`-0.29` pp) | `+8` (`+0.00`/`+0.72` pp) |
| `libgit2/repository` | `20` | `10` | first | `+10` (`+0.32`/`+0.16` pp) | `+0` (`+0.00`/`-0.31` pp) | `+4` (`+0.31`/`-0.04` pp) | `+11` (`+0.30`/`+0.14` pp) |
| `libgit2/repository` | `20` | `11` | first | `+13` (`+0.01`/`+0.60` pp) | `+0` (`+0.00`/`-1.97` pp) | `+5` (`+0.37`/`-1.79` pp) | `+9` (`+0.01`/`+0.38` pp) |
| `libgit2/repository` | `22` | `0` | first | `+24` (`+0.94`/`+1.58` pp) | `+0` (`+0.01`/`-1.26` pp) | `+6` (`+0.55`/`-0.31` pp) | `+12` (`+0.02`/`+0.43` pp) |
| `libgit2/repository` | `22` | `1` | first | `+10` (`+0.88`/`+0.36` pp) | `+0` (`+0.23`/`-0.57` pp) | `+3` (`+0.31`/`+0.32` pp) | `+7` (`+0.00`/`+0.27` pp) |
| `libgit2/object` | `26` | `0` | first | `+9` (`+0.02`/`+0.08` pp) | `+0` (`+0.01`/`-0.69` pp) | `+5` (`+0.43`/`-0.80` pp) | `+9` (`+0.01`/`+0.48` pp) |
| `libgit2/object` | `26` | `1` | first | `+7` (`+0.46`/`-0.14` pp) | `+6` (`+0.33`/`+1.37` pp) | `+6` (`+0.19`/`+0.10` pp) | `+8` (`+1.01`/`-0.18` pp) |
| `libgit2/object` | `26` | `2` | first | `+9` (`+0.58`/`+0.17` pp) | `+2` (`+0.56`/`+0.11` pp) | `+7` (`+0.71`/`+0.09` pp) | `+20` (`+0.62`/`+0.41` pp) |
| `libgit2/object` | `26` | `3` | first | `+15` (`+0.07`/`+0.15` pp) | `+0` (`-0.26`/`-0.44` pp) | `+4` (`+0.28`/`-0.16` pp) | `+10` (`+0.38`/`+0.25` pp) |
| `libgit2/object` | `26` | `4` | first | `+6` (`+0.06`/`-0.59` pp) | `+3` (`+0.22`/`+0.91` pp) | `+4` (`+0.13`/`-0.34` pp) | `+4` (`+0.14`/`-0.35` pp) |
| `libgit2/object` | `26` | `5` | first | `+3` (`+0.00`/`+0.02` pp) | `+0` (`-0.05`/`-0.09` pp) | `+3` (`+0.04`/`+0.02` pp) | `+3` (`-0.01`/`+0.06` pp) |
| `libgit2/object` | `28` | `0` | first | `+10` (`+0.39`/`-0.07` pp) | `+0` (`+0.10`/`-0.53` pp) | `+6` (`+0.01`/`-0.21` pp) | `+8` (`+0.01`/`+0.34` pp) |
| `libgit2/object` | `28` | `1` | first | `+9` (`+0.47`/`-0.03` pp) | `+2` (`+0.22`/`+0.89` pp) | `+5` (`+0.10`/`+0.09` pp) | `+9` (`+0.03`/`+0.55` pp) |
| `libgit2/object` | `28` | `2` | first | `+7` (`+0.10`/`+0.42` pp) | `+4` (`-0.04`/`+0.71` pp) | `+5` (`+0.09`/`+0.20` pp) | `+9` (`+0.02`/`+0.28` pp) |
| `libgit2/object` | `28` | `3` | first | `+22` (`+0.19`/`+0.02` pp) | `+0` (`-0.34`/`-0.61` pp) | `+6` (`+0.04`/`-0.33` pp) | `+12` (`+0.05`/`+0.32` pp) |
| `libgit2/object` | `28` | `4` | first | `+5` (`-0.02`/`+0.49` pp) | `+0` (`-0.02`/`-0.37` pp) | `+1` (`+0.09`/`-0.18` pp) | `+6` (`+0.02`/`+0.28` pp) |
| `libgit2/object` | `30` | `0` | first | `+6` (`+0.28`/`+0.25` pp) | `+0` (`+0.27`/`-0.24` pp) | `+2` (`+0.22`/`+0.04` pp) | `+4` (`+0.27`/`+0.18` pp) |
| `libgit2/config` | `34` | `0` | first | `+2` (`+0.02`/`-0.01` pp) | `+0` (`-0.07`/`-0.12` pp) | `+2` (`-0.03`/`-0.04` pp) | `+3` (`-0.01`/`-0.05` pp) |
| `libgit2/config` | `34` | `1` | first | `+3` (`+0.09`/`-0.06` pp) | `+0` (`+0.38`/`-0.13` pp) | `+3` (`+0.25`/`-0.22` pp) | `+4` (`+0.14`/`-0.02` pp) |
| `libgit2/config` | `36` | `0` | first | `+2` (`+0.04`/`+0.16` pp) | `+0` (`+0.31`/`-0.16` pp) | `+2` (`+0.07`/`+0.17` pp) | `+2` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `38` | `0` | first | `+1` (`-0.04`/`+0.01` pp) | `+0` (`-0.49`/`-0.04` pp) | `+1` (`-0.02`/`+0.04` pp) | `+2` (`+0.01`/`+0.04` pp) |
| `libgit2/config` | `40` | `0` | first | `+4` (`-0.01`/`+0.06` pp) | `+1` (`+0.03`/`+0.48` pp) | `+1` (`+0.01`/`-0.06` pp) | `+3` (`+0.02`/`-0.03` pp) |
| `libgit2/config` | `42` | `0` | first | `+2` (`+0.32`/`-0.35` pp) | `+0` (`-0.55`/`-0.18` pp) | `+3` (`+0.25`/`-0.26` pp) | `+5` (`+0.14`/`-0.17` pp) |
| `libgit2/config` | `46` | `0` | first | `+9` (`+0.03`/`+1.56` pp) | `+0` (`+0.22`/`-0.44` pp) | `+3` (`+0.01`/`+0.40` pp) | `+9` (`+0.00`/`+0.39` pp) |
| `libgit2/index` | `48` | `0` | first | `+4` (`+0.82`/`-0.36` pp) | `+1` (`+0.35`/`+2.04` pp) | `+3` (`+0.78`/`+0.00` pp) | `+3` (`+0.16`/`-0.92` pp) |
| `libgit2/index` | `48` | `1` | first | `+2` (`-0.03`/`+0.05` pp) | `+0` (`+0.01`/`-0.06` pp) | `+3` (`+0.02`/`-0.07` pp) | `+2` (`-0.01`/`-0.29` pp) |
| `libgit2/index` | `48` | `2` | first | `+2` (`-0.01`/`-0.11` pp) | `+0` (`+0.19`/`-0.04` pp) | `+2` (`-0.02`/`-0.08` pp) | `+3` (`+0.04`/`+0.05` pp) |
| `libgit2/index` | `48` | `3` | first | `+2` (`+0.05`/`-0.08` pp) | `+0` (`-0.21`/`-0.04` pp) | `+2` (`+0.05`/`-0.05` pp) | `+2` (`+0.02`/`+0.03` pp) |
| `libgit2/index` | `48` | `4` | first | `+3` (`+0.05`/`+0.06` pp) | `+0` (`+0.02`/`-0.06` pp) | `+2` (`+0.08`/`+0.10` pp) | `+4` (`+0.06`/`+0.00` pp) |
| `libgit2/index` | `48` | `5` | first | `+5` (`+0.02`/`-0.03` pp) | `+0` (`-0.11`/`-0.10` pp) | `+3` (`+0.10`/`-0.03` pp) | `+5` (`+0.03`/`+0.11` pp) |
| `libgit2/index` | `48` | `6` | first | `+6` (`+0.19`/`+0.33` pp) | `+0` (`-1.22`/`-1.08` pp) | `+2` (`+0.06`/`-0.07` pp) | `+4` (`+0.03`/`-0.05` pp) |
| `libgit2/index` | `48` | `7` | first | `+2` (`+0.35`/`-0.35` pp) | `+5` (`+1.23`/`+0.91` pp) | `+3` (`+0.45`/`+0.14` pp) | `+2` (`+0.13`/`-0.35` pp) |
| `libgit2/index` | `50` | `0` | first | `+6` (`+0.02`/`+0.20` pp) | `+0` (`+0.18`/`-0.27` pp) | `+2` (`+0.02`/`-0.73` pp) | `+4` (`+0.00`/`-0.17` pp) |
| `libgit2/index` | `50` | `1` | first | `+4` (`+0.12`/`+0.00` pp) | `+0` (`-0.16`/`-0.06` pp) | `+2` (`+0.13`/`+0.03` pp) | `+2` (`+0.02`/`-0.04` pp) |
| `libgit2/src/libgit2/indexer.c` | `172` | `0` | first | `+6` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+5` (—/— pp) |
| `libgit2/src/libgit2/odb.c` | `176` | `0` | first | `+2` (—/— pp) | `+0` (—/— pp) | `+2` (—/— pp) | `+10` (—/— pp) |
| `libgit2/src/libgit2/odb.c` | `176` | `1` | first | `+9` (—/— pp) | `+0` (—/— pp) | `+4` (—/— pp) | `+8` (—/— pp) |
| `libgit2/src/libgit2/odb.c` | `180` | `0` | first | `+5` (—/— pp) | `+0` (—/— pp) | `+3` (—/— pp) | `+6` (—/— pp) |
| `libgit2/src/libgit2/diff_parse.c` | `228` | `0` | first | `+4` (—/— pp) | `+0` (—/— pp) | `+3` (—/— pp) | `+5` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `330` | `0` | first | `+2` (—/— pp) | `+0` (—/— pp) | `+1` (—/— pp) | `+5` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `334` | `0` | first | `+6` (`+0.27`/`+0.47` pp) | `-9` (`-2.75`/`-0.68` pp) | `+0` (`+0.00`/`-0.33` pp) | `+4` (`+0.01`/`+0.12` pp) |
| `libgit2/util` | `6` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+2` (`+0.00`/`+0.00` pp) |
| `libgit2/util` | `6` | `1` | review | `+0` (`+0.00`/`-0.44` pp) | `+0` (`+0.00`/`-0.79` pp) | `+1` (`+0.27`/`-0.49` pp) | `+1` (`+0.00`/`+0.20` pp) |
| `libgit2/oid` | `11` | `0` | review | `+1` (`+0.00`/`+0.32` pp) | `+0` (`+0.00`/`-0.39` pp) | `+1` (`+0.00`/`-0.61` pp) | `+4` (`+0.00`/`+0.57` pp) |
| `libgit2/oid` | `11` | `1` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+1.74`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/oid` | `12` | `0` | review | `+2` (`+0.00`/`+2.43` pp) | `+0` (`+0.00`/`-0.47` pp) | `+0` (`+0.00`/`+0.39` pp) | `+2` (`+4.32`/`+0.20` pp) |
| `libgit2/core` | `15` | `0` | review | `+0` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+2` (`+0.32`/`+0.15` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `17` | `0` | review | `+1` (`-0.02`/`-0.06` pp) | `+0` (`+0.00`/`-0.04` pp) | `+0` (`-0.01`/`-0.06` pp) | `+1` (`+0.01`/`+0.07` pp) |
| `libgit2/repository` | `21` | `0` | review | `+2` (`+0.24`/`-0.34` pp) | `+0` (`-0.32`/`-0.48` pp) | `+3` (`+0.02`/`-0.30` pp) | `+2` (`+0.00`/`+0.23` pp) |
| `libgit2/repository` | `23` | `0` | review | `+0` (`-0.01`/`+0.00` pp) | `+0` (`-0.28`/`+0.00` pp) | `+1` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/object` | `27` | `0` | review | `+15` (`+0.09`/`+0.79` pp) | `+0` (`-0.35`/`-3.27` pp) | `+2` (`+0.06`/`+0.35` pp) | `+2` (`+0.07`/`-0.03` pp) |
| `libgit2/object` | `29` | `0` | review | `+3` (`-0.01`/`+0.05` pp) | `+0` (`-0.51`/`-0.06` pp) | `+1` (`+0.11`/`+0.10` pp) | `+1` (`+0.05`/`+0.08` pp) |
| `libgit2/object` | `31` | `0` | review | `+0` (`+0.02`/`+0.00` pp) | `+0` (`+0.49`/`+0.00` pp) | `+0` (`+0.05`/`+0.00` pp) | `+0` (`-0.02`/`+0.00` pp) |
| `libgit2/config` | `35` | `0` | review | `+2` (`+0.00`/`-0.42` pp) | `+0` (`+0.31`/`-0.08` pp) | `+1` (`-0.03`/`-0.27` pp) | `+3` (`+0.01`/`+0.11` pp) |
| `libgit2/config` | `37` | `0` | review | `+3` (`+0.00`/`-0.23` pp) | `+0` (`-0.49`/`-0.08` pp) | `+2` (`-0.01`/`-0.16` pp) | `+3` (`+0.05`/`+0.18` pp) |
| `libgit2/config` | `39` | `0` | review | `+1` (`-0.01`/`+0.06` pp) | `+0` (`+0.24`/`-0.01` pp) | `+1` (`+0.02`/`-0.09` pp) | `+2` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `41` | `0` | review | `+1` (`-0.01`/`-0.01` pp) | `+0` (`+0.06`/`-0.07` pp) | `+2` (`+0.01`/`-0.01` pp) | `+4` (`+0.00`/`+0.17` pp) |
| `libgit2/config` | `43` | `0` | review | `+1` (`+0.02`/`+0.14` pp) | `+0` (`+0.37`/`-0.01` pp) | `+2` (`+0.02`/`-0.14` pp) | `+7` (`+0.09`/`+0.40` pp) |
| `libgit2/config` | `47` | `0` | review | `+0` (`-0.02`/`+0.00` pp) | `+0` (`-0.29`/`-0.01` pp) | `+1` (`-0.04`/`-0.03` pp) | `+1` (`-0.02`/`+0.03` pp) |
| `libgit2/index` | `49` | `0` | review | `+1` (`+0.08`/`-0.09` pp) | `+2` (`+0.33`/`+0.40` pp) | `+1` (`+0.18`/`-0.04` pp) | `+2` (`+0.00`/`+0.09` pp) |
| `libgit2/index` | `51` | `0` | review | `+1` (`-0.01`/`+0.00` pp) | `+0` (`+0.04`/`+0.00` pp) | `+0` (`-0.01`/`+0.00` pp) | `+2` (`+0.10`/`+0.05` pp) |
| `libgit2/src/libgit2/indexer.c` | `173` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | review | `+1` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/odb.c` | `181` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/diff_parse.c` | `229` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `331` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+1` (—/— pp) | `+0` (—/— pp) |
| **Σ first** |  |  |  | **`+459` (`+25.12`/`-2.23` pp)** | **`+46` (`+14.03`/`-52.88` pp)** | **`+257` (`+31.12`/`-25.16` pp)** | **`+476` (`+12.62`/`+24.53` pp)** |
| **Σ review** |  |  |  | **`+35` (`+0.37`/`+2.20` pp)** | **`+3` (`-0.40`/`-5.36` pp)** | **`+23` (`+2.71`/`-1.21` pp)** | **`+39` (`+4.66`/`+2.36` pp)** |

### Batches — port

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| module | wave | batch | pass | types | fields | +LoC | cov (C/Rust) | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `<link-unit>/<module>` | `<n>` | `<n>` | first | `<n>` | `<n>` | `+<n>/-<n>` | `<n>`% / `<n>`% | `$<n>` | `<n>M` | `<n>m<n>s` | `$<n>` | `$<n>` |
| `<link-unit>/<module>` | `<n>` | `<n>` | review | `<n>` | `<n>` | `+<n>/-<n>` | `<n>`% / `<n>`% | `$<n>` | `<n>M` | `<n>m<n>s` | `$<n>` | `$<n>` |
| **Σ first** |  |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`$<n>`** | **`<n>M`** | — | **`$<n>`** | **`$<n>`** |
| **Σ review** |  |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`$<n>`** | **`<n>M`** | — | **`$<n>`** | **`$<n>`** |

### Batches — port: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| module | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `<link-unit>/<module>` | `<n>` | `<n>` | first | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) |
| `<link-unit>/<module>` | `<n>` | `<n>` | review | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) | `+<n>` (`+<n>`/`+<n>` pp) |
| **Σ first** |  |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |
| **Σ review** |  |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |

### Batches: safety audit

Deterministic `ffimap`; no model.

| module | wave | batch | objective | pass | pub raw ptr (safe) | pub raw ptr | pub void ptr | pub unsafe fn | C borrow | slice borrow | unsafe loc % | unsafe blocks | Send/Sync | C by value | Drop/Clone | deref outside impl |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/util` | `4` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.06` pp | `+24` | — | — | `+0` | `+0` |
| `libgit2/util` | `4` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-2.56` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `8` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.54` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/oid` | `8` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.98` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/oid` | `9` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+2.27` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.39` pp | `+20` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-1.93` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.97` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.47` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `4` | wrap | first | `+0` | `+1` | `+0` | `+0` | — | — | `+0.44` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-1.02` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.29` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `7` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.35` pp | `+26` | — | — | `+1` | `+3` |
| `libgit2/core` | `16` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.59` pp | `+32` | — | — | `+1` | `+2` |
| `libgit2/repository` | `20` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.30` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `1` | wrap | first | `+4` | `+8` | `+6` | `+4` | — | — | `+1.07` pp | `+46` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `2` | wrap | first | `+1` | `+1` | `+0` | `+1` | — | — | `+0.39` pp | `+20` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `5` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `-0.15` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+15` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `7` | wrap | first | `+0` | `+0` | `+0` | `-1` | — | — | `+0.06` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `8` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.79` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `9` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.72` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `10` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/repository` | `20` | `11` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.57` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/repository` | `22` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.27` pp | `+36` | — | — | `+0` | `+0` |
| `libgit2/repository` | `22` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+16` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.41` pp | `+9` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `1` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `-0.07` pp | `+18` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `2` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `-0.16` pp | `+18` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.16` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `4` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `-0.09` pp | `+9` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.08` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/object` | `28` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.17` pp | `+34` | — | — | `+0` | `+0` |
| `libgit2/object` | `28` | `1` | wrap | first | `+0` | `+1` | `+2` | `+1` | — | — | `+0.33` pp | `+36` | — | — | `-1` | `+0` |
| `libgit2/object` | `28` | `2` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.14` pp | `+28` | — | — | `+0` | `+0` |
| `libgit2/object` | `28` | `3` | wrap | first | `-1` | `-2` | `+0` | `-1` | — | — | `+0.03` pp | `+29` | — | — | `+0` | `+0` |
| `libgit2/object` | `28` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.10` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/object` | `30` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.18` pp | `+19` | — | — | `+0` | `+0` |
| `libgit2/config` | `34` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+13` | — | — | `+1` | `+0` |
| `libgit2/config` | `34` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.12` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/config` | `36` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+28` | — | — | `+1` | `+0` |
| `libgit2/config` | `38` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/config` | `40` | `0` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.18` pp | `+37` | — | — | `+0` | `+2` |
| `libgit2/config` | `42` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.12` pp | `+40` | — | — | `+1` | `+0` |
| `libgit2/config` | `46` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.19` pp | `+44` | — | — | `+1` | `+0` |
| `libgit2/index` | `48` | `0` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.53` pp | `+45` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.10` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.06` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.21` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `5` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+13` | — | — | `+1` | `+0` |
| `libgit2/index` | `48` | `6` | wrap | first | `+0` | `-4` | `+0` | `-4` | — | — | `+0.11` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `7` | wrap | first | `+0` | `+4` | `+0` | `+4` | — | — | `+0.06` pp | `+17` | — | — | `+1` | `+0` |
| `libgit2/index` | `50` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+40` | — | — | `+0` | `+0` |
| `libgit2/index` | `50` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.08` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/src/libgit2/indexer.c` | `172` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+30` | `+6` | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `176` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.06` pp | `+27` | `+12` | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `176` | `1` | wrap | first | `+2` | `+2` | `+0` | `+2` | — | — | `+0.09` pp | `+35` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `180` | `0` | wrap | first | `-2` | `-2` | `+0` | `+1` | — | — | `+0.14` pp | `+51` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/diff_parse.c` | `228` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+10` | `+6` | — | `+0` | `+0` |
| `libgit2/src/libgit2/checkout.c` | `330` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+14` | `+6` | — | `+0` | `+0` |
| `libgit2/src/libgit2/checkout.c` | `334` | `0` | wrap | first | `-3` | `-6` | `-6` | `-3` | — | — | `+0.01` pp | `+26` | `+0` | — | `+0` | `+12` |
| `libgit2/util` | `6` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/util` | `6` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/oid` | `11` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.18` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `11` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `12` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.37` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/core` | `15` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `17` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/repository` | `21` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.18` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/repository` | `23` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `27` | `0` | wrap | review | `+0` | `+0` | `+0` | `-3` | — | — | `+0.09` pp | `+8` | — | — | `+0` | `+0` |
| `libgit2/object` | `29` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/object` | `31` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `35` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.09` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `37` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `39` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `41` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `-1` | — | — | `+0` | `-2` |
| `libgit2/config` | `43` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/config` | `47` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `-1` | — | — | `+0` | `+0` |
| `libgit2/index` | `49` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.03` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `51` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/src/libgit2/indexer.c` | `173` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `181` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/diff_parse.c` | `229` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/checkout.c` | `331` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| **Σ first** |  |  |  |  | **`+1`** | **`+3`** | **`+2`** | **`+13`** | — | — | **`-0.51` pp** | **`+1158`** | — | — | **`+7`** | **`+19`** |
| **Σ review** |  |  |  |  | **`+0`** | **`+0`** | **`+0`** | **`-3`** | — | — | **`-0.14` pp** | **`+20`** | — | — | **`+0`** | **`-2`** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/util` | `6` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `6` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/oid` | `11` | `0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/oid` | `11` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/oid` | `12` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+1` | `3` |
| `libgit2/core` | `15` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `5` |
| `libgit2/core` | `17` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `2` |
| `libgit2/repository` | `21` | `0` | `+1` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `4` |
| `libgit2/repository` | `23` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/object` | `27` | `0` | `+2` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `8` |
| `libgit2/object` | `29` | `0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `8` |
| `libgit2/object` | `31` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/config` | `35` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+2` | `+4` | `7` |
| `libgit2/config` | `37` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+4` | `6` |
| `libgit2/config` | `39` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+2` | `4` |
| `libgit2/config` | `41` | `0` | `+0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+2` | `+1` | `5` |
| `libgit2/config` | `43` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+2` | `+3` | `7` |
| `libgit2/config` | `47` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+3` | `4` |
| `libgit2/index` | `49` | `0` | `+0` | `+2` | `+0` | `+1` | `+0` | `+0` | `+2` | `+2` | `7` |
| `libgit2/index` | `51` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` |
| `libgit2/src/libgit2/indexer.c` | `173` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/src/libgit2/odb.c` | `181` | `0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/src/libgit2/diff_parse.c` | `229` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` |
| `libgit2/src/libgit2/checkout.c` | `331` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| **Σ** |  |  | **`+7`** | **`+9`** | **`+3`** | **`+1`** | **`+3`** | **`+3`** | **`+15`** | **`+57`** | **`98`** |

## Symbols

### Batches

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | objective | pass | symbols | loc | +LoC | cov (C/Rust) | $ | tokens | wall | $/symbol | $/loc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | first | `2` | — | `+1245/-1` | — | `$8.28` | `17.79M` | `51m29s` | `$4.14` | — |
| `libgit2/raw-lifetime-string` | `2` | `0` | wrap | first | `2` | — | `+1362/-14` | — | `$5.49` | `12.39M` | `23m12s` | `$2.75` | — |
| `libgit2/util` | `4` | `2` | wrap | first | `4` | `51` | `+1008/-26` | `1.30`% / `97.32`% | `$4.00` | `8.72M` | `21m23s` | `$1.00` | `$0.08` |
| `libgit2/util` | `5` | `0` | wrap | first | `1` | `15` | `+756/-71` | `1.36`% / `98.40`% | `$3.06` | `6.69M` | `15m41s` | `$3.06` | `$0.20` |
| `libgit2/oid` | `9` | `1` | wrap | first | `18` | `252` | `+1351/-11` | `6.07`% / `99.47`% | `$8.55` | `17.50M` | `40m06s` | `$0.48` | `$0.03` |
| `libgit2/oid` | `10` | `0` | wrap | first | `1` | `3` | `+0/-0` | `6.07`% / `99.47`% | `$0.64` | `0.93M` | `4m17s` | `$0.64` | `$0.21` |
| `libgit2/core` | `14` | `8` | wrap | first | `6` | `444` | `+3504/-75` | `15.52`% / `99.81`% | `$19.90` | `44.40M` | `1h04m11s` | `$3.32` | `$0.04` |
| `libgit2/core` | `16` | `1` | wrap | first | `5` | `212` | `+1238/-4` | `18.20`% / `99.81`% | `$4.61` | `11.51M` | `19m37s` | `$0.92` | `$0.02` |
| `libgit2/core` | `18` | `0` | wrap | first | `1` | `9` | `+658/-41` | `18.24`% / `99.82`% | `$2.30` | `4.24M` | `10m40s` | `$2.30` | `$0.26` |
| `libgit2/repository` | `22` | `2` | wrap | first | `25` | `436` | `+3311/-54` | `36.06`% / `99.30`% | `$24.81` | `84.41M` | `1h01m00s` | `$0.99` | `$0.06` |
| `libgit2/repository` | `22` | `3` | wrap | first | `25` | `432` | `+3392/-29` | `35.39`% / `99.35`% | `$20.18` | `66.23M` | `53m42s` | `$0.81` | `$0.05` |
| `libgit2/repository` | `22` | `4` | wrap | first | `18` | `396` | `+3072/-105` | `34.12`% / `99.40`% | `$15.55` | `50.84M` | `45m11s` | `$0.86` | `$0.04` |
| `libgit2/repository` | `24` | `0` | wrap | first | `7` | `373` | `+1449/-382` | `36.17`% / `99.32`% | `$17.18` | `58.64M` | `32m30s` | `$2.45` | `$0.05` |
| `libgit2/object` | `26` | `6` | wrap | first | `2` | `28` | `+996/-0` | `36.26`% / `99.31`% | `$3.49` | `8.11M` | `27m05s` | `$1.75` | `$0.12` |
| `libgit2/object` | `28` | `5` | wrap | first | `25` | `455` | `+4438/-388` | `39.43`% / `99.39`% | `$34.78` | `128.15M` | `1h09m40s` | `$1.39` | `$0.08` |
| `libgit2/object` | `28` | `6` | wrap | first | `25` | `261` | `+3882/-94` | `39.17`% / `99.36`% | `$26.99` | `97.07M` | `1h03m06s` | `$1.08` | `$0.10` |
| `libgit2/object` | `28` | `7` | wrap | first | `5` | `51` | `+2334/-111` | `38.42`% / `99.33`% | `$13.81` | `38.98M` | `48m22s` | `$2.76` | `$0.27` |
| `libgit2/object` | `30` | `1` | wrap | first | `25` | `268` | `+5204/-170` | `42.10`% / `99.43`% | `$24.53` | `82.24M` | `1h11m24s` | `$0.98` | `$0.09` |
| `libgit2/object` | `30` | `2` | wrap | first | `25` | `350` | `+1923/-76` | `39.84`% / `99.37`% | `$11.68` | `35.74M` | `39m28s` | `$0.47` | `$0.03` |
| `libgit2/object` | `30` | `3` | wrap | first | `25` | `392` | `+4093/-32` | `40.44`% / `99.39`% | `$20.74` | `70.47M` | `1h00m04s` | `$0.83` | `$0.05` |
| `libgit2/object` | `30` | `4` | wrap | first | `14` | `167` | `+1555/-41` | `39.51`% / `99.38`% | `$8.21` | `23.01M` | `24m37s` | `$0.59` | `$0.05` |
| `libgit2/object` | `32` | `0` | wrap | first | `6` | `144` | `+1978/-296` | `42.23`% / `99.43`% | `$13.22` | `40.69M` | `38m33s` | `$2.20` | `$0.09` |
| `libgit2/config` | `34` | `2` | wrap | first | `3` | `74` | `+405/-0` | `42.28`% / `99.43`% | `$1.37` | `5.31M` | `8m00s` | `$0.46` | `$0.02` |
| `libgit2/config` | `36` | `1` | wrap | first | `25` | `265` | `+1429/-4` | `42.71`% / `99.33`% | `$2.46` | `10.12M` | `21m59s` | `$0.10` | `$0.01` |
| `libgit2/config` | `36` | `2` | wrap | first | `3` | `78` | `+630/-8` | `42.73`% / `99.33`% | `$2.40` | `10.91M` | `27m41s` | `$0.80` | `$0.03` |
| `libgit2/config` | `38` | `1` | wrap | first | `5` | `60` | `+816/-6` | `42.78`% / `99.44`% | `$1.63` | `6.29M` | `11m14s` | `$0.33` | `$0.03` |
| `libgit2/config` | `40` | `1` | wrap | first | `3` | `52` | `+707/-9` | `42.76`% / `99.46`% | `$1.41` | `4.66M` | `9m46s` | `$0.47` | `$0.03` |
| `libgit2/config` | `42` | `1` | wrap | first | `5` | `91` | `+422/-88` | `42.85`% / `99.45`% | `$1.17` | `3.84M` | `6m28s` | `$0.23` | `$0.01` |
| `libgit2/config` | `44` | `0` | wrap | first | `3` | `66` | `+1073/-2` | `43.15`% / `99.45`% | `$2.04` | `8.40M` | `14m30s` | `$0.68` | `$0.03` |
| `libgit2/index` | `48` | `8` | wrap | first | `1` | `0` | `+548/-0` | `44.05`% / `99.50`% | `$8.57` | `4.72M` | `9m53s` | `$8.57` | — |
| `libgit2/index` | `50` | `2` | wrap | first | `25` | `370` | `+2142/-57` | `46.56`% / `99.55`% | `$22.30` | `78.75M` | `1h06m15s` | `$0.89` | `$0.06` |
| `libgit2/index` | `50` | `3` | wrap | first | `25` | `405` | `+2120/-46` | `45.96`% / `99.54`% | `$15.01` | `42.85M` | `55m51s` | `$0.60` | `$0.04` |
| `libgit2/index` | `50` | `4` | wrap | first | `5` | `47` | `+342/-22` | `45.76`% / `99.53`% | `$3.09` | `7.11M` | `12m00s` | `$0.62` | `$0.07` |
| `libgit2/index` | `52` | `0` | wrap | first | `17` | `383` | `+2323/-79` | `46.66`% / `99.55`% | `$14.38` | `43.62M` | `43m51s` | `$0.85` | `$0.04` |
| `libgit2/index` | `52` | `1` | wrap | first | `2` | `0` | `+369/-6` | `46.57`% / `99.53`% | `$3.31` | `8.07M` | `18m25s` | `$1.66` | — |
| `libgit2/src/libgit2/odb.c` | `176` | `2` | wrap | first | `22` | `1143` | `+2136/-126` | — | `$20.37` | `63.30M` | `1h01m07s` | `$0.93` | `$0.02` |
| `libgit2/src/libgit2/indexer.c` | `178` | `0` | wrap | first | `1` | `157` | `+970/-4` | — | `$4.45` | `10.63M` | `21m12s` | `$4.45` | `$0.03` |
| `libgit2/src/libgit2/checkout.c` | `330` | `1` | wrap | first | `1` | `153` | `+855/-8` | — | `$7.63` | `22.75M` | `37m23s` | `$7.63` | `$0.05` |
| `libgit2/src/libgit2/checkout.c` | `332` | `0` | wrap | first | `2` | `395` | `+1711/-20` | — | `$8.62` | `22.76M` | `47m38s` | `$4.31` | `$0.02` |
| `libgit2/raw-lifetime-void` | `1` | `0` | wrap | review | `2` | — | `+80/-1` | — | `$7.89` | `10.78M` | `18m36s` | `$3.94` | — |
| `libgit2/raw-lifetime-string` | `3` | `0` | wrap | review | `2` | — | `+345/-23` | — | `$10.89` | `14.41M` | `22m03s` | `$5.45` | — |
| `libgit2/util` | `6` | `2` | wrap | review | `4` | — | `+281/-117` | `1.63`% / `99.74`% | `$12.00` | `16.87M` | `28m49s` | `$3.00` | — |
| `libgit2/util` | `7` | `0` | wrap | review | `1` | — | `+0/-0` | `1.63`% / `99.74`% | `$6.26` | `6.37M` | `23m51s` | `$6.26` | — |
| `libgit2/oid` | `12` | `1` | wrap | review | `18` | — | `+181/-10` | `7.80`% / `99.49`% | `$11.62` | `12.73M` | `34m10s` | `$0.65` | — |
| `libgit2/oid` | `13` | `0` | wrap | review | `1` | — | `+0/-0` | `7.80`% / `99.49`% | `$3.39` | `3.23M` | `8m46s` | `$3.39` | — |
| `libgit2/core` | `15` | `1` | wrap | review | `6` | — | `+345/-20` | `15.51`% / `99.81`% | `$12.53` | `17.80M` | `23m38s` | `$2.09` | — |
| `libgit2/core` | `17` | `1` | wrap | review | `5` | — | `+59/-10` | `18.24`% / `99.82`% | `$7.69` | `10.83M` | `17m46s` | `$1.54` | — |
| `libgit2/core` | `19` | `0` | wrap | review | `1` | — | `+61/-9` | `18.23`% / `99.82`% | `$6.83` | `5.78M` | `18m10s` | `$6.83` | — |
| `libgit2/repository` | `23` | `1` | wrap | review | `68` | — | `+640/-143` | `36.16`% / `99.39`% | `$38.68` | `57.91M` | `1h08m35s` | `$0.57` | — |
| `libgit2/repository` | `25` | `0` | wrap | review | `7` | — | `+574/-18` | `36.17`% / `99.32`% | `$29.96` | `37.71M` | `57m28s` | `$4.28` | — |
| `libgit2/object` | `27` | `1` | wrap | review | `2` | — | `+105/-6` | `37.89`% / `99.25`% | `$20.94` | `18.07M` | `1h08m26s` | `$10.47` | — |
| `libgit2/object` | `29` | `1` | wrap | review | `55` | — | `+305/-110` | `39.40`% / `99.37`% | `$12.26` | `40.91M` | `26m40s` | `$0.22` | — |
| `libgit2/object` | `31` | `1` | wrap | review | `89` | — | `+216/-18` | `42.20`% / `99.43`% | `$13.54` | `45.38M` | `27m21s` | `$0.15` | — |
| `libgit2/object` | `33` | `0` | wrap | review | `6` | — | `+226/-17` | `42.24`% / `99.43`% | `$39.58` | `57.30M` | `3h04m22s` | `$6.60` | — |
| `libgit2/config` | `35` | `1` | wrap | review | `3` | — | `+178/-43` | `42.51`% / `99.41`% | `$2.71` | `5.91M` | `12m53s` | `$0.90` | — |
| `libgit2/config` | `37` | `1` | wrap | review | `28` | — | `+625/-242` | `42.75`% / `99.45`% | `$11.84` | `37.34M` | `35m07s` | `$0.42` | — |
| `libgit2/config` | `39` | `1` | wrap | review | `5` | — | `+222/-51` | `42.75`% / `99.46`% | `$4.64` | `11.54M` | `16m26s` | `$0.93` | — |
| `libgit2/config` | `41` | `1` | wrap | review | `3` | — | `+561/-34` | `42.78`% / `99.45`% | `$6.96` | `18.87M` | `24m40s` | `$2.32` | — |
| `libgit2/config` | `43` | `1` | wrap | review | `5` | — | `+575/-147` | `43.14`% / `99.45`% | `$6.30` | `15.93M` | `20m24s` | `$1.26` | — |
| `libgit2/config` | `45` | `0` | wrap | review | `3` | — | `+172/-99` | `43.17`% / `99.45`% | `$7.39` | `17.00M` | `47m40s` | `$2.46` | — |
| `libgit2/index` | `49` | `1` | wrap | review | `1` | — | `+303/-147` | `45.57`% / `99.51`% | `$6.50` | `18.04M` | `24m32s` | `$6.50` | — |
| `libgit2/index` | `51` | `1` | wrap | review | `55` | — | `+442/-9` | `46.58`% / `99.53`% | `$12.37` | `39.58M` | `39m02s` | `$0.22` | — |
| `libgit2/index` | `53` | `0` | wrap | review | `19` | — | `+190/-21` | `46.64`% / `99.55`% | `$20.10` | `42.35M` | `1h18m28s` | `$1.06` | — |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | wrap | review | `22` | — | `+23/-2` | — | `$7.52` | `23.94M` | `28m33s` | `$0.34` | — |
| `libgit2/src/libgit2/indexer.c` | `179` | `0` | wrap | review | `1` | — | `+35/-0` | — | `$3.18` | `8.05M` | `12m18s` | `$3.18` | — |
| `libgit2/src/libgit2/checkout.c` | `331` | `1` | wrap | review | `1` | — | `+113/-14` | — | `$4.80` | `10.64M` | `27m50s` | `$4.80` | — |
| `libgit2/src/libgit2/checkout.c` | `333` | `0` | wrap | review | `2` | — | `+217/-15` | — | `$11.03` | `25.98M` | `47m17s` | `$5.51` | — |
| **Σ first** |  |  |  |  | **`415`** | **`8478`** | **`+67747/-2506`** | **`46.66`% / `99.55`%** | **`$412.21`** | **`1262.84M`** | — | **`$0.99`** | **`$0.05`** |
| **Σ review** |  |  |  |  | **`415`** | — | **`+7074/-1326`** | **`46.64`% / `99.55`%** | **`$339.40`** | **`631.25M`** | — | **`$0.82`** | — |

### Batches: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

Review rows: `openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | pass | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|---|
| `libgit2/util` | `4` | `2` | first | `+11` (`+0.18`/`-1.19` pp) | `+0` (`+0.00`/`-10.21` pp) | `+4` (`+0.12`/`-2.07` pp) | `+4` (`+0.07`/`-8.60` pp) |
| `libgit2/util` | `5` | `0` | first | `+6` (`+0.00`/`+7.16` pp) | `+0` (`+0.00`/`-1.80` pp) | `+5` (`+0.00`/`+5.40` pp) | `+3` (`+0.01`/`+0.47` pp) |
| `libgit2/oid` | `9` | `1` | first | `+8` (`+0.17`/`+1.11` pp) | `+0` (`+0.00`/`-5.19` pp) | `+5` (`+0.18`/`+3.02` pp) | `+7` (`+0.10`/`-6.64` pp) |
| `libgit2/oid` | `10` | `0` | first | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `14` | `8` | first | `+9` (`+0.22`/`-6.72` pp) | `+1` (`+0.63`/`+15.87` pp) | `+17` (`+0.17`/`+2.31` pp) | `+10` (`+0.14`/`-12.68` pp) |
| `libgit2/core` | `16` | `1` | first | `+6` (`+2.54`/`-0.21` pp) | `+5` (`+3.62`/`+0.85` pp) | `+4` (`+2.38`/`-0.11` pp) | `+6` (`+0.00`/`-0.10` pp) |
| `libgit2/core` | `18` | `0` | first | `+0` (`+0.02`/`+0.08` pp) | `+5` (`+0.01`/`+0.29` pp) | `+3` (`+0.00`/`+0.08` pp) | `+0` (`+0.00`/`-0.30` pp) |
| `libgit2/repository` | `22` | `2` | first | `+11` (`+0.83`/`+0.46` pp) | `+0` (`-0.05`/`-1.30` pp) | `+14` (`+0.78`/`+0.63` pp) | `+10` (`+0.87`/`-1.56` pp) |
| `libgit2/repository` | `22` | `3` | first | `+14` (`+1.28`/`+1.23` pp) | `+2` (`+1.07`/`-0.48` pp) | `+18` (`+1.29`/`+0.67` pp) | `+11` (`+0.14`/`-2.09` pp) |
| `libgit2/repository` | `22` | `4` | first | `+18` (`+0.63`/`+0.17` pp) | `+0` (`-0.26`/`-1.36` pp) | `+10` (`+0.34`/`+0.34` pp) | `+13` (`+1.37`/`-0.32` pp) |
| `libgit2/repository` | `24` | `0` | first | `+6` (`-0.20`/`+0.33` pp) | `+0` (`-0.02`/`+1.36` pp) | `+3` (`+0.08`/`+0.31` pp) | `+6` (`+0.16`/`-0.22` pp) |
| `libgit2/object` | `26` | `6` | first | `+9` (`+0.08`/`-0.39` pp) | `+0` (`+0.02`/`-0.19` pp) | `+9` (`+0.08`/`-0.35` pp) | `+7` (`+0.07`/`+0.12` pp) |
| `libgit2/object` | `28` | `5` | first | `+23` (`+0.42`/`+0.28` pp) | `+0` (`+0.29`/`-1.46` pp) | `+8` (`+0.25`/`-0.52` pp) | `+20` (`+0.46`/`-0.12` pp) |
| `libgit2/object` | `28` | `6` | first | `+15` (`+0.53`/`+0.57` pp) | `+0` (`-0.59`/`-0.82` pp) | `+6` (`+0.74`/`-0.36` pp) | `+21` (`+1.51`/`+0.18` pp) |
| `libgit2/object` | `28` | `7` | first | `+9` (`+0.23`/`+0.30` pp) | `+0` (`+0.32`/`-0.32` pp) | `+5` (`+0.06`/`+0.27` pp) | `+5` (`+0.29`/`+0.26` pp) |
| `libgit2/object` | `30` | `1` | first | `+18` (`+0.21`/`+0.75` pp) | `+0` (`-0.49`/`-0.88` pp) | `+5` (`+0.14`/`+0.08` pp) | `+16` (`+3.31`/`+0.73` pp) |
| `libgit2/object` | `30` | `2` | first | `+9` (`+0.12`/`+0.10` pp) | `+0` (`+0.06`/`-0.23` pp) | `+2` (`+0.10`/`+0.22` pp) | `+7` (`+0.08`/`+0.11` pp) |
| `libgit2/object` | `30` | `3` | first | `+13` (`+0.51`/`+0.54` pp) | `+0` (`+0.12`/`-0.42` pp) | `+15` (`+0.57`/`+1.05` pp) | `+9` (`+0.75`/`-0.49` pp) |
| `libgit2/object` | `30` | `4` | first | `+9` (`+0.06`/`+0.13` pp) | `+0` (`-0.46`/`-0.15` pp) | `+3` (`+0.03`/`+0.36` pp) | `+2` (`+0.00`/`-0.09` pp) |
| `libgit2/object` | `32` | `0` | first | `+6` (`-0.01`/`+0.30` pp) | `+0` (`-0.21`/`-0.32` pp) | `+0` (`+0.00`/`+0.57` pp) | `+4` (`+0.32`/`+0.03` pp) |
| `libgit2/config` | `34` | `2` | first | `+3` (`+0.05`/`+0.05` pp) | `+0` (`-0.46`/`-0.04` pp) | `+1` (`+0.04`/`+0.05` pp) | `+0` (`+0.01`/`-0.17` pp) |
| `libgit2/config` | `36` | `1` | first | `+6` (`+0.20`/`+0.23` pp) | `+0` (`-0.02`/`-0.35` pp) | `+3` (`+0.15`/`+0.34` pp) | `+2` (`+0.02`/`-2.06` pp) |
| `libgit2/config` | `36` | `2` | first | `+4` (`+0.01`/`+0.04` pp) | `+0` (`-0.28`/`-0.05` pp) | `+3` (`+0.01`/`+0.04` pp) | `+3` (`-0.01`/`+0.03` pp) |
| `libgit2/config` | `38` | `1` | first | `+5` (`+0.05`/`+0.03` pp) | `+0` (`+0.02`/`-0.12` pp) | `+3` (`+0.03`/`+0.03` pp) | `+3` (`+0.00`/`-0.07` pp) |
| `libgit2/config` | `40` | `1` | first | `+4` (`+0.02`/`+0.07` pp) | `+0` (`+0.01`/`-0.07` pp) | `+3` (`+0.02`/`+0.11` pp) | `+2` (`+0.00`/`+0.06` pp) |
| `libgit2/config` | `42` | `1` | first | `+3` (`+0.03`/`+0.02` pp) | `+0` (`+0.06`/`-0.02` pp) | `+1` (`+0.00`/`+0.03` pp) | `+1` (`+0.00`/`+0.01` pp) |
| `libgit2/config` | `44` | `0` | first | `+4` (`+0.02`/`+0.02` pp) | `+0` (`-0.31`/`-0.14` pp) | `+2` (`+0.02`/`+0.07` pp) | `+4` (`+0.08`/`+0.15` pp) |
| `libgit2/index` | `48` | `8` | first | `+3` (`+0.00`/`+0.08` pp) | `+0` (`+0.18`/`-0.08` pp) | `+1` (`+0.09`/`+0.07` pp) | `+3` (`+1.70`/`-0.06` pp) |
| `libgit2/index` | `50` | `2` | first | `+8` (`+1.45`/`+0.13` pp) | `+0` (`+0.01`/`-0.15` pp) | `+5` (`+1.06`/`+0.28` pp) | `+8` (`+0.12`/`-0.10` pp) |
| `libgit2/index` | `50` | `3` | first | `+8` (`+0.27`/`+0.15` pp) | `+0` (`+0.04`/`-0.20` pp) | `+10` (`+0.17`/`+0.38` pp) | `+7` (`+0.07`/`-0.21` pp) |
| `libgit2/index` | `50` | `4` | first | `+3` (`+0.10`/`+0.04` pp) | `+0` (`-0.04`/`+0.00` pp) | `+2` (`+0.02`/`+0.06` pp) | `+0` (`+0.00`/`-0.04` pp) |
| `libgit2/index` | `52` | `0` | first | `+10` (`+0.19`/`+0.18` pp) | `+0` (`-0.13`/`-0.15` pp) | `+10` (`+0.09`/`+0.47` pp) | `+6` (`+0.14`/`+0.01` pp) |
| `libgit2/index` | `52` | `1` | first | `+2` (`+0.00`/`+0.00` pp) | `+0` (`+0.03`/`-0.01` pp) | `+3` (`-0.02`/`+0.08` pp) | `+2` (`-0.02`/`+0.01` pp) |
| `libgit2/src/libgit2/odb.c` | `176` | `2` | first | `+5` (—/— pp) | `+4` (—/— pp) | `+3` (—/— pp) | `+7` (—/— pp) |
| `libgit2/src/libgit2/indexer.c` | `178` | `0` | first | `+5` (—/— pp) | `+0` (—/— pp) | `+1` (—/— pp) | `+5` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `330` | `1` | first | `+5` (—/— pp) | `+0` (—/— pp) | `+2` (—/— pp) | `+8` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `332` | `0` | first | `+10` (—/— pp) | `+0` (—/— pp) | `+2` (—/— pp) | `+14` (—/— pp) |
| `libgit2/util` | `6` | `2` | review | `+1` (`+0.00`/`-0.79` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`-0.79` pp) | `+1` (`+0.00`/`+4.47` pp) |
| `libgit2/util` | `7` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/oid` | `12` | `1` | review | `+0` (`+0.00`/`+0.02` pp) | `+0` (`+0.00`/`-0.04` pp) | `+1` (`+0.00`/`+0.03` pp) | `+1` (`+0.00`/`+0.27` pp) |
| `libgit2/oid` | `13` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `15` | `1` | review | `+0` (`-0.01`/`-0.79` pp) | `+0` (`+0.01`/`+0.69` pp) | `+0` (`-0.01`/`-0.22` pp) | `+3` (`+0.02`/`+0.75` pp) |
| `libgit2/core` | `17` | `1` | review | `+0` (`+0.01`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.01`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/core` | `19` | `0` | review | `+0` (`+0.00`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/repository` | `23` | `1` | review | `+0` (`+0.08`/`-0.28` pp) | `+0` (`-0.04`/`-0.10` pp) | `+1` (`+0.03`/`-0.33` pp) | `+3` (`+0.00`/`+0.16` pp) |
| `libgit2/repository` | `25` | `0` | review | `+1` (`+0.00`/`+0.11` pp) | `+0` (`-0.06`/`-0.13` pp) | `+4` (`+0.01`/`+0.14` pp) | `+1` (`+2.18`/`+0.32` pp) |
| `libgit2/object` | `27` | `1` | review | `+0` (`+0.22`/`+0.00` pp) | `+0` (`+0.05`/`+0.00` pp) | `+1` (`+0.00`/`+0.00` pp) | `+3` (`+0.00`/`+0.02` pp) |
| `libgit2/object` | `29` | `1` | review | `+1` (`+0.01`/`+0.10` pp) | `+0` (`-0.09`/`+0.02` pp) | `+0` (`-0.02`/`+0.23` pp) | `+2` (`-0.03`/`-0.03` pp) |
| `libgit2/object` | `31` | `1` | review | `+0` (`-0.01`/`+0.00` pp) | `+0` (`-0.01`/`+0.00` pp) | `+3` (`+0.03`/`-0.01` pp) | `+0` (`+0.00`/`+0.00` pp) |
| `libgit2/object` | `33` | `0` | review | `+0` (`+0.01`/`+0.01` pp) | `+0` (`-0.29`/`-0.02` pp) | `+0` (`+0.03`/`+0.02` pp) | `+2` (`+0.03`/`+0.31` pp) |
| `libgit2/config` | `35` | `1` | review | `+0` (`+0.01`/`+0.02` pp) | `+0` (`-0.47`/`-0.01` pp) | `+1` (`+0.00`/`+0.03` pp) | `+2` (`+0.04`/`+0.22` pp) |
| `libgit2/config` | `37` | `1` | review | `+1` (`+0.00`/`+0.10` pp) | `+0` (`+0.20`/`-0.03` pp) | `+2` (`+0.00`/`+0.16` pp) | `+4` (`+0.07`/`+0.82` pp) |
| `libgit2/config` | `39` | `1` | review | `+0` (`-0.02`/`+0.02` pp) | `+0` (`+0.01`/`+0.01` pp) | `+1` (`+0.01`/`+0.04` pp) | `+1` (`-0.01`/`+0.18` pp) |
| `libgit2/config` | `41` | `1` | review | `+3` (`+0.02`/`+0.01` pp) | `+0` (`-0.49`/`-0.04` pp) | `+2` (`+0.00`/`+0.15` pp) | `+0` (`+0.00`/`-0.24` pp) |
| `libgit2/config` | `43` | `1` | review | `+1` (`-0.01`/`+0.00` pp) | `+0` (`+0.12`/`+0.00` pp) | `+2` (`-0.02`/`+0.00` pp) | `+2` (`-0.01`/`+0.01` pp) |
| `libgit2/config` | `45` | `0` | review | `+0` (`+0.00`/`+0.02` pp) | `+0` (`+0.19`/`-0.01` pp) | `+0` (`+0.02`/`+0.02` pp) | `+0` (`+0.00`/`+0.01` pp) |
| `libgit2/index` | `49` | `1` | review | `+1` (`-0.02`/`+0.02` pp) | `+0` (`-0.13`/`-0.01` pp) | `+0` (`-0.01`/`-0.01` pp) | `+2` (`+0.01`/`+0.01` pp) |
| `libgit2/index` | `51` | `1` | review | `+2` (`+0.04`/`+0.01` pp) | `+0` (`-0.02`/`-0.06` pp) | `+0` (`+0.01`/`+0.16` pp) | `+4` (`+0.16`/`+0.17` pp) |
| `libgit2/index` | `53` | `0` | review | `+3` (`+0.01`/`+0.01` pp) | `+0` (`+0.10`/`+0.00` pp) | `+1` (`+0.00`/`+0.01` pp) | `+1` (`-0.02`/`+0.00` pp) |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/indexer.c` | `179` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+0` (—/— pp) | `+1` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `331` | `1` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+2` (—/— pp) | `+0` (—/— pp) |
| `libgit2/src/libgit2/checkout.c` | `333` | `0` | review | `+0` (—/— pp) | `+0` (—/— pp) | `+1` (—/— pp) | `+1` (—/— pp) |
| **Σ first** |  |  |  | **`+288` (`+10.21`/`+6.04` pp)** | **`+17` (`+3.17`/`-8.14` pp)** | **`+191` (`+8.99`/`+13.91` pp)** | **`+236` (`+11.76`/`-33.75` pp)** |
| **Σ review** |  |  |  | **`+14` (`+0.34`/`-1.41` pp)** | **`+2` (`-0.92`/`+0.27` pp)** | **`+22` (`+0.09`/`-0.37` pp)** | **`+34` (`+2.44`/`+7.45` pp)** |

### Batches: safety audit

Deterministic `ffimap`; no model.

| module | wave | batch | objective | pass | pub raw ptr (safe) | pub raw ptr | pub void ptr | pub unsafe fn | C borrow | slice borrow | unsafe loc % | unsafe blocks | Send/Sync | C by value | Drop/Clone | deref outside impl |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/raw-lifetime-void` | `0` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+13.04` pp | `+12` | — | — | `+0` | `+0` |
| `libgit2/raw-lifetime-string` | `2` | `0` | wrap | first | `+2` | `+3` | `+0` | `+1` | — | — | `-0.15` pp | `+17` | — | — | `+2` | `+0` |
| `libgit2/util` | `4` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/util` | `5` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.09` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/oid` | `9` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.62` pp | `+13` | — | — | `+0` | `+0` |
| `libgit2/oid` | `10` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `14` | `8` | wrap | first | `+0` | `+0` | `+0` | `+33` | — | — | `+2.41` pp | `+65` | — | — | `+1` | `+0` |
| `libgit2/core` | `16` | `1` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.01` pp | `+6` | — | — | `+0` | `+0` |
| `libgit2/core` | `18` | `0` | wrap | first | `+0` | `+0` | `+0` | `+2` | — | — | `+0.03` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/repository` | `22` | `2` | wrap | first | `+1` | `+1` | `+1` | `+0` | — | — | `+0.54` pp | `+43` | — | — | `+1` | `+2` |
| `libgit2/repository` | `22` | `3` | wrap | first | `+0` | `+0` | `+0` | `+1` | — | — | `+0.17` pp | `+34` | — | — | `+0` | `+1` |
| `libgit2/repository` | `22` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+26` | — | — | `+0` | `+0` |
| `libgit2/repository` | `24` | `0` | wrap | first | `-1` | `-1` | `-1` | `+0` | — | — | `+0.04` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/object` | `26` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.07` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/object` | `28` | `5` | wrap | first | `-1` | `-2` | `-2` | `-1` | — | — | `+0.12` pp | `+24` | — | — | `+0` | `+2` |
| `libgit2/object` | `28` | `6` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.17` pp | `+28` | — | — | `+0` | `+2` |
| `libgit2/object` | `28` | `7` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+9` | — | — | `+0` | `+1` |
| `libgit2/object` | `30` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+1.40` pp | `+52` | — | — | `+0` | `+1` |
| `libgit2/object` | `30` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+26` | — | — | `+0` | `+0` |
| `libgit2/object` | `30` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.36` pp | `+29` | — | — | `+0` | `+0` |
| `libgit2/object` | `30` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.08` pp | `+10` | — | — | `+0` | `+0` |
| `libgit2/object` | `32` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.11` pp | `+11` | — | — | `+0` | `+0` |
| `libgit2/config` | `34` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.06` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `36` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+30` | — | — | `+0` | `+0` |
| `libgit2/config` | `36` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+4` | — | — | `+0` | `+0` |
| `libgit2/config` | `38` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.01` pp | `+5` | — | — | `+0` | `+1` |
| `libgit2/config` | `40` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.07` pp | `+3` | — | — | `+0` | `+0` |
| `libgit2/config` | `42` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/config` | `44` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.03` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/index` | `48` | `8` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+4` | — | — | `+0` | `+1` |
| `libgit2/index` | `50` | `2` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+17` | — | — | `+0` | `+0` |
| `libgit2/index` | `50` | `3` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.16` pp | `+27` | — | — | `+0` | `+0` |
| `libgit2/index` | `50` | `4` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/index` | `52` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.03` pp | `+14` | — | — | `+2` | `+0` |
| `libgit2/index` | `52` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `176` | `2` | wrap | first | `+0` | `+0` | `+0` | `+4` | — | — | `+0.02` pp | `+27` | `+6` | — | `+0` | `+0` |
| `libgit2/src/libgit2/indexer.c` | `178` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+3` | `+0` | — | `+0` | `+1` |
| `libgit2/src/libgit2/checkout.c` | `330` | `1` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.03` pp | `+3` | `+0` | — | `+0` | `+1` |
| `libgit2/src/libgit2/checkout.c` | `332` | `0` | wrap | first | `+0` | `+0` | `+0` | `+0` | — | — | `-0.05` pp | `+8` | `+0` | — | `+0` | `+2` |
| `libgit2/raw-lifetime-void` | `1` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/raw-lifetime-string` | `3` | `0` | wrap | review | `+0` | `+0` | `+0` | `+1` | — | — | `-0.34` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/util` | `6` | `2` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/util` | `7` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `12` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/oid` | `13` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `15` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.20` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/core` | `17` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/core` | `19` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/repository` | `23` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.13` pp | `-4` | — | — | `+0` | `+0` |
| `libgit2/repository` | `25` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.02` pp | `+5` | — | — | `+0` | `+0` |
| `libgit2/object` | `27` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `29` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/object` | `31` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/object` | `33` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+1` | — | — | `+0` | `+0` |
| `libgit2/config` | `35` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.02` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `37` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `39` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `41` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.04` pp | `+2` | — | — | `+0` | `+0` |
| `libgit2/config` | `43` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/config` | `45` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/index` | `49` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.01` pp | `+0` | — | — | `+0` | `-1` |
| `libgit2/index` | `51` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+7` | — | — | `+0` | `+0` |
| `libgit2/index` | `53` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | — | — | `+0` | `+0` |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/indexer.c` | `179` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `-0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/checkout.c` | `331` | `1` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| `libgit2/src/libgit2/checkout.c` | `333` | `0` | wrap | review | `+0` | `+0` | `+0` | `+0` | — | — | `+0.00` pp | `+0` | `+0` | — | `+0` | `+0` |
| **Σ first** |  |  |  |  | **`+1`** | **`+1`** | **`-2`** | **`+41`** | — | — | **`+18.18` pp** | **`+582`** | — | — | **`+6`** | **`+15`** |
| **Σ review** |  |  |  |  | **`+0`** | **`+0`** | **`+0`** | **`+1`** | — | — | **`-0.38` pp** | **`+19`** | — | — | **`+0`** | **`-1`** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| module | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `libgit2/raw-lifetime-void` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/raw-lifetime-string` | `3` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `6` | `2` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/util` | `7` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/oid` | `12` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` |
| `libgit2/oid` | `13` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `0` |
| `libgit2/core` | `15` | `1` | — | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+2` | `3` |
| `libgit2/core` | `17` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/core` | `19` | `0` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `2` |
| `libgit2/repository` | `23` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+2` | `4` |
| `libgit2/repository` | `25` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` |
| `libgit2/object` | `27` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/object` | `29` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+2` | `4` |
| `libgit2/object` | `31` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+1` | `3` |
| `libgit2/object` | `33` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+4` | `6` |
| `libgit2/config` | `35` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` |
| `libgit2/config` | `37` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `6` |
| `libgit2/config` | `39` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+2` | `4` |
| `libgit2/config` | `41` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| `libgit2/config` | `43` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` |
| `libgit2/config` | `45` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` |
| `libgit2/index` | `49` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` |
| `libgit2/index` | `51` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` |
| `libgit2/index` | `53` | `0` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+4` | `6` |
| `libgit2/src/libgit2/odb.c` | `177` | `0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` |
| `libgit2/src/libgit2/indexer.c` | `179` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` |
| `libgit2/src/libgit2/checkout.c` | `331` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| `libgit2/src/libgit2/checkout.c` | `333` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` |
| — (`tests/ub_safe`) | `526` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` |
| **Σ** |  |  | **`+1`** | **`+5`** | **`+3`** | **`+1`** | **`+3`** | **`+5`** | **`+11`** | **`+51`** | **`80`** |

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

Review wave `526` (`reviewer/run-132`, $3.48, 8.54M tokens, 16m41s; outside `schedule.json`, at the user's request) moved the
fixture onto the wrappers that had landed since: of the 51 FFI functions it called, 17 are
gone, and its `unsafe` blocks went from 117 to 92. Six of the 23 functions with wrappers stay
raw, each for a form the wrappers do not offer, as its docs say: a C blob stream lent as a
`WriteStreamBox`, a caller-owned `git_buf` that the option getters fill, and an exclusive
handle to a repository's shared index. The other 28 have no wrapper yet. See
`crustify/reviews/wave-526/batch-0/misc/ub-safe-fixture-outlives-its-safe-constructors/`.

The safe UB suite then forbids `unsafe_code` at its root (`tests/ub_safe/main.rs`), as the
testing spec (crustify `40f922d`) and the conventions ask, done by the assistant at the user's
request: the fixture's `expect(unsafe_code)` is gone, and with it every fixture that built its
inputs through a raw producer, and the test module of `checkout.rs` that called libgit2 directly.
118 of the suite's 806 tests needed them and are removed (688 remain); each comes back in the batch
that wraps the producer it needs. `removed-tests.md` in that review's directory lists them, with the
24 unwrapped producers the removed fixtures called.

### BSan skips the compile-fail suite

`trybuild`'s nested cargo build does not finish under BorrowSanitizer, so
`crustify/scripts/cargo-variant.sh bsan test` skips `compile_fail`; the plain variant
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
row's sub-campaign as `<link-unit>/<module>`, the same name as its Overview row.
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

The Coverage stats test lines were measured again on 2026-11-04, at `crustify/devel`
`1e732ebff`, with the testing spec's Coverage method, which replaces the gcov one the
earlier figures used. Those figures are not comparable with these. Each count is the
number of tests its compiled test binary registers (`-- --list`): `--lib` for the unit
tests, one of them ignored, and the `ub_safe`, `ub_unsafe` and `equiv` integration crates.
The trybuild runner is one `ub_safe` test, and it was skipped in the coverage run since it
runs no code. `ub_safe` holds 688 tests since the unsafe fixtures were removed. Doctests
are not counted in the table.

The `coverage` variant, `build.json` version 4, and the side libraries' `coverage`
installs (version 3 of each) compile the C with clang 23,
`-fprofile-instr-generate -fcoverage-mapping`. The Rust is a scratch copy under
`crustify/tmp/coverage/` with `#[coverage(off)]` on every test item, compiled on
`nightly-2026-10-08` (LLVM 23) with `-C instrument-coverage`. Each suite ran alone into
its own profile, and `Total tests` is the merge of the four suites' profiles, their union.
Every figure is the `TOTAL` line of the nightly's `llvm-cov report`, with `libgit2.so` and
every test binary as objects for every suite.

- C is line coverage over the 86,770 lines `libgit2.so` maps to `c/src/libgit2`,
  `c/src/util`, `c/include` and the bundled `c/deps` it compiles (`llhttp`, `ntlmclient`,
  `reftable`, `xdiff`). The generated headers and the C's own tests count toward none.
  libgit2 proper, `src/` and `include/` without `deps/`, is `52.42`% of 68,023 lines. The
  total covers `61.63`% of functions and `47.65`% of regions.
- Rust is line coverage over the 13,226 lines of the crate's 42 files under `src/`, with
  `tests/`, the `-sys` crate and dependencies left out by path. The total covers `99.02`% of
  functions and `97.65`% of regions.
- Doctests, run as a suite of their own: 347, of which 346 pass under the nightly. The
  compile-fail doctest on `CommitBuilder` expects `E0624`, which stable reports and the
  nightly does not; it passes on plain. They reach `11.94`% of C lines and `23.30`% of
  Rust lines, and add no line to the four suites' union.

`crustify/scripts/coverage/` holds the scripts that measured them:
`run.sh "$PWD" libgit2 "$PWD/crustify/tmp/coverage"`, then
`report.py crustify/tmp/coverage "C=$PWD/c/src/libgit2:$PWD/c/src/util:$PWD/c/include:$PWD/c/deps,Rust=$PWD/crustify/tmp/coverage/tree/src" --c crustify/builds/coverage/libgit2.so.1.9.0`.

The UB suites also run under ASan+UBSan, TSan, Miri and BorrowSanitizer in the agents'
gates. Those runs share one test set and are not counted again.
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
### `libgit2/src/libgit2/indexer.c` wave 172: coverage not measured

Wave 172 batch 0 (`git_indexer_progress`) leaves its `cov (C/Rust)` cell and its tests'
`(+C/+Rust pp)` pairs at `—`, and the coverage percentages in Coverage stats and the
`Σ first` `cov` cell as the previous landing left them; its test counts are updated. The
documented method needs `cargo-llvm-cov` and `lcov`, neither installed on the host, and the
`coverage` variant is a gcov build whose `.gcda` files are written into the shared build
directory. The batch's own Rust paths were checked with source-based coverage instead
(`-C instrument-coverage`, `nightly-2026-10-07`'s `llvm-cov`): its unit and `ub_safe`
tests cover every line, function and region of `src/libgit2/src/libgit2/indexer.rs`. It
calls no C. Its safety audit row fills `Send/Sync` (`+6`: `Send` and `Sync` for the value
and its two handles) from the dotted ffimap counters `ty.impl_trait.of_which.std.send` and
`.sync`; `C borrow`, `slice borrow` and `C by value` stay `—`.

### `libgit2/src/libgit2/indexer.c` wave 173: review of `git_indexer_progress`

Wave 173 batch 0 reviews wave 172 batch 0. Of its two reports, the `contracts` one (the
`CZeroable` justification) is counted under `+misc`, since the table has no `contracts`
column. Its `cov (C/Rust)` cell and test pairs stay `—`, as wave 172's do, for the same
reason. Source-based coverage on `nightly-2026-10-07`, with the unit tests under
`#[coverage(off)]` in a scratch copy, measured `src/libgit2/src/libgit2/indexer.rs` at
100% of lines, functions and regions. The landing changes only comments, so every ffimap
counter equals the wave tip's. The Miri gate passed on the wave tip (`lib` 324, `ub_safe`
160, `ub_unsafe` 18 and `equiv` 32 passed, the tests that call C ignored). TSan and
ASan/UBSan passed the workset's unit and `ub_safe` tests. On both the wave tip and the
landed tree, five compile-fail snapshots of other modules (`commit_arc_no_mut_handle`,
`mailmap_not_clone`, `object_arc_no_mut_handle`, `tag_and_tree_arc_no_mut_handle`,
`tree_builder_not_clone`) mismatch. The installed ffibox is `0.1.1` while `Cargo.lock`
pins `0.1.0`, and the actual output prints `/opt/venv/share/ffibox/src/refs.rs:<line>`
where the snapshots expect `$FFIBOX/src/refs.rs`. They are left for a batch that owns
those modules or the pin.

### `libgit2/src/libgit2/odb.c` wave 176 batch 0: `git_odb_options` and `git_odb_expand_id`

Wave 176 batch 0 wraps `git_odb_options` as `OdbOptions` and `git_odb_expand_id` as
`OdbExpandId`. Its `cov (C/Rust)` cell and its tests' `(+C/+Rust pp)` pairs stay `—`, and
the coverage percentages in Coverage stats and the `Σ first` `cov` cell stay as the
previous landing left them, for wave 172's reason. Source-based coverage on
`nightly-2026-10-07`, with the unit tests under `#[coverage(off)]` in a scratch copy and the
odb unit, `equiv` and `ub_safe` tests as the workloads, covers every line of the two
types. `src/libgit2/src/libgit2/odb.rs` as a whole is at `98.97`% of lines; its 4 missed
lines are all in earlier code, such as the runtime-pin failure paths. `+LoC` leaves out the 77
regenerated lines of `libgit2-sys/src/bindings.rs`. The test counts in Coverage stats are
the tests each binary registers at this landing. `ub_unsafe` registers `96` where the cell
read `95`, although this batch adds no `ub_unsafe` test. The safety audit
row comes from `ffimap` scans of the wave base and this tree. `Send/Sync` is `+12`: `Send`
and `Sync` for each type and its two handles. Derived `Clone` counts under neither
`Drop/Clone` nor any other column. `C borrow`, `slice borrow` and `C by value` stay `—`.
Five compile-fail snapshots of other modules mismatch, as at wave 173, from the ffibox pin
drift.

### `libgit2/src/libgit2/odb.c` wave 176 batch 1: `git_odb_stream` and `git_odb_writepack`

Wave 176 batch 1 wraps `git_odb_stream` as `OdbStream` and `git_odb_writepack` as
`OdbWritepack`. It also binds the stream's lifecycle routines, `git_odb_open_wstream`,
`git_odb_open_rstream` and `git_odb_stream_free`, which Coverage stats counts as `+3`
wrapped symbols. `git_odb_write_pack` stays with wave 180, since it takes wave 178's
`git_indexer_progress_cb`. Coverage cells stay `—` for wave 172's reason. Source-based
coverage on `nightly-2026-10-07` over the new odb unit, `ub_safe` and `equiv` tests covers
every line of the new code except the open routines' passthrough branch, which only a custom
backend reaches. The C fake write stream, used for a backend without streaming writes, is
also out of reach for the same reason. `+LoC` leaves out the 144 regenerated lines of
`libgit2-sys/src/bindings.rs`.

The safety audit row comes from `ffimap` scans of the wave tip this landing rebased onto
(`603bf0085`) and this tree. `pub raw ptr (safe)` is `+2`: the two `backend()` getters,
which return `*mut git_odb_backend` until wave 180 wraps that type. `pub unsafe fn` is `+2`:
the two `CDrop::c_drop` methods. Derived `Clone` counts in no column; no `Drop` or `Clone`
is hand-written. `deref outside impl` is `+0` because every raw-pointer dereference sits in
an impl or trait body; the scanner no longer reports that counter.

BorrowSanitizer did not run. `cargo-variant.sh bsan` stops in `libgit2-sys`'s bindings check:
the `bsan` toolchain's clang 23 adds doc comments to 26 macro constants, none of them this
batch's, so its regenerated `bindings.rs` differs from the committed one. ASan+UBSan, TSan and
Miri ran on every new test. The same five compile-fail snapshots mismatch from the ffibox pin
drift.

### `libgit2/src/libgit2/odb.c` wave 176 batch 2: odb readers, writers and objects

Coverage is not measured, as for wave 172: its `cov (C/Rust)` cell and its tests'
`(+C/+Rust pp)` pairs stay `—`, and the Coverage stats percentages stand as the previous
landing left them; the test counts move. `nightly-2026-10-08` ships no `llvm-tools`, and the
`coverage` variant is a gcov build. The batch's Rust was checked with source-based coverage
instead (`-C instrument-coverage`, `nightly-2026-10-07`'s `llvm-cov`, every test of the four
suites matching `odb`): every line of its additions to `src/libgit2/src/libgit2/odb.rs` runs
but `OdbObjectMut`'s `Debug`, which nothing reaches, since nothing lends an `OdbObjectMut`.

The batch wraps `ty::git_odb_object` with its readers (wave 170 batch 0's type), since its
signatures need it. The readers learn the odb's id type through `git_odb_expand_ids` and
refuse ids of any other type, which libgit2's loose backend would format out of bounds.
`git_odb_write_multi_pack_index` and `git_odb_set_commit_graph` are bound as `unsafe fn`s
(`+2` `pub unsafe fn`): the first walks the backends and the packed backend's packs without
the odb's lock, the second frees a commit-graph whose file a revision walk reads past it, and
no type of the crate proves that no other thread uses the odb. Only the NULL form of
`git_odb_set_commit_graph` is bound (`clear_commit_graph`), pending `git_commit_graph`.

The safety audit row comes from `ffimap` scans of the wave tip this landing rebased onto
(`3f43c106d`, batch 1's recorded scan) and this tree. `pub unsafe fn` is `+4`: the two
methods above and `OdbObjectFree`'s `CDrop::c_drop` and `CRefClone::c_up_ref`. `Send/Sync` is
`+6`, for `OdbObject` and its two handles. `OdbObjectRef::data` is one more slice borrowed
over C memory, the object's buffer, which libgit2 never writes and frees only with the
object; the column stays `—`. The four new `Clone`s are derived and count in no column.
`+LoC` leaves out the 187 regenerated lines of `libgit2-sys/src/bindings.rs`.

ASan+UBSan and TSan ran every test matching `odb` (and `index::entries`, whose fixture call
changed) after the rebase; Miri ran the batch's Rust-only tests, and each new test marked
`ignore` under Miri was run there alone and seen to stop where its reason says. No test of
the batch is expected to trigger BorrowSanitizer, so none ran under it. Under ASan, the
`ub_unsafe` binary's untouched `repository` tests stop in the dynamic linker:
`libpcre2-8.so.0` of the `asan-ubsan` pcre2 install lacks `__asan_stack_malloc_9`; the
batch's own `ub_unsafe` tests pass there. The same five compile-fail snapshots mismatch from
the ffibox pin drift.

### `libgit2/src/libgit2/odb.c` wave 177: review of wave 176

Wave 177 batch 0 reviews wave 176's three batches. Its rows are split by item kind. One reviewer ran for both rows (reviewer run 124, $8.89, 28.30M tokens), so its cost and tokens are split between them by the items each covers (4 types, 22 symbols), and each row gives the run's whole wall. The
`Types` rows (batches 0 and 1: 4 types, 19 fields) count the `ub_safe` report, which covers
`OdbRef::open_read_stream`, bound with `git_odb_stream`, and two `misc` ones: the
`git_odb_open_rstream` ownership note and the `OdbExpandId` docs. The `Symbols` rows (batch 2:
22 symbols) count the `git_odb_object` lifecycle report and the commit-graph `contracts`
report. Wave 173 counted its `contracts` report under `+misc`, and so does this batch. `+LoC`
counts the Rust source only, split by finding: `odb.rs` changed by `+50/-10` and the `ub_safe`
stream test by `+37`. Every ffimap counter in the safety audit equals the wave tip's. The
unsafe LoC share falls by 0.003 pp, from the 3 non-test lines added.

The `cov (C/Rust)` cells and test pairs stay `—`, as wave 176's do. Source-based coverage on
`nightly-2026-10-07`, with test items under `#[coverage(off)]` in a scratch copy, measured
`src/libgit2/src/libgit2/odb.rs` at 99.38% of lines (808, 5 missed), 98.79% of functions and
99.17% of regions. The missed lines are unreachable or depend on the environment:
`OdbObjectMut`'s `Debug`, since nothing ever lends an `OdbObjectMut`, and the
`runtime_pinned()` failure branches of the constructors. C coverage is not measured (the
`coverage` variant is gcov).

The Miri gate passed on the wave tip (`lib` 337, `ub_safe` 161, `ub_unsafe` 18 and `equiv`
32 passed, the tests that call C ignored). After the fixes, the odb tests of `ub_safe`,
`ub_unsafe` and `equiv` passed under ASan/UBSan and TSan. With the check removed, the new
`ub_safe` test `read_streams_refuse_ids_of_another_type` aborts under ASan. BSan ran on the
reproducer alone, on the fixed tree, and it built because the reproducer's `libgit2-sys`
uses the committed bindings unchecked. The five compile-fail snapshots of other modules
noted under wave 173 still mismatch, on the wave tip and on the landed tree alike.

### `libgit2/src/libgit2/indexer.c` wave 178 batch 0: `git_indexer_progress_cb`

Wave 178 batch 0 binds the callback `git_indexer_progress_cb` as `ProgressCallback`, which
holds an exclusively borrowed closure and an `extern "C"` trampoline, after
`ForeachCallback`'s pattern. Every invoker (`do_progress_callback`, `network_packetsize`,
`git_smart__download_pack`'s trailing call) calls it synchronously and lends the progress
for the call only, so there is one contract and no fork. The closure returns
`ControlFlow<()>`, and `Break` reaches C as `GIT_EUSER`. The header documents only negative
codes as cancelling, and `resolve_deltas` continues after a positive code although
`git_error_set_after_callback_function` has set an error. A panic cancels with `GIT_EUSER`,
every later call cancels without running the closure, and the panic resumes once control is
back in Rust. `git_odb_write_pack`, its first consumer, stays with wave 180. It keeps the
callback across several `append` calls rather than one walk, so its wrapper needs a borrow
that outlives the writepack, and must resume the panic after each call. The unit tests cover
this case through `walk_with`.

Coverage cells stay `—`, for wave 172's reason. Source-based coverage on
`nightly-2026-10-07`, with test items under `#[coverage(off)]` in a scratch copy and the
indexer unit, `ub_safe` and `equiv` tests as the workloads, measures
`src/libgit2/src/libgit2/indexer.rs` at 100% of lines (156), functions (32) and regions
(164). The unit and `equiv` tests have libgit2 call the callback while it indexes a
`git pack-objects` pack, one blob stored as a delta, through `git_odb_write_pack`'s
writepack. The `equiv` test runs a C callback beside it on an independent odb, and the two
sides cancel at the same calls, the delta's resolution at commit included. The safety audit
row comes from `ffimap` scans of the wave base (`27e9a49c9`) and this tree. No public
`unsafe fn` and no raw pointer on the public interface is added. `deref outside impl` is
`+1`, read from the source: the scanner no longer reports the counter, and the trampoline's
payload dereference sits in a free `extern "C"` fn, as `ForeachCallback`'s does
(`raw_ptr.deref.wr` `+1`).

ASan+UBSan and TSan ran the batch's unit, `ub_safe`, `equiv` and doc tests. Miri ran its
`lib` and `ub_safe` tests. Each test marked `ignore` under Miri was run there alone: the unit
tests stop at `statx` (file system isolation) and the `equiv` test at `git_libgit2_init`.
No test of the batch is expected to trigger BorrowSanitizer, so none ran under it. The five
compile-fail snapshots of other modules still mismatch from the ffibox pin drift. The three
new snapshots pass.

### `libgit2/src/libgit2/indexer.c` wave 179: review of wave 178

Wave 179 batch 0 reviews wave 178's `ProgressCallback` and files one `internal` report,
`progress-callback-stale-panic`. When the walk itself unwinds after the closure panicked
inside C, `walk_with` never takes the stored panic. Every later call then cancels without
running the closure and resumes the old panic. The fix drops any stored panic before
each walk starts, and the reproducing unit test becomes its regression test. `walk_with` is
`pub(crate)`, so no public path reached the defect. The lifecycle, unsafety, contracts,
UB, equivalence, conventions and ownership-store checks found nothing more. Every C invoker
lends a non-NULL progress for the call only, and the store's findings match the other
callbacks'. `+LoC` is `indexer.rs` alone: 5 non-test lines and the 30-line test.

The `cov (C/Rust)` cells stay `—`, as wave 178's do. Wave 178's method, run on the landed
tree, measures `src/libgit2/src/libgit2/indexer.rs` at 100% of lines (165), functions (32)
and regions (157), so no coverage tests were added. The safety audit row comes from
`ffimap` scans of the wave base (`f1f360f7b`) and this tree. Every counter is equal; the
unsafe LoC share falls by 0.001 pp. The Miri gate passed on the wave tip in all four
binaries, and its indexer tests pass again on the fixed tree. The indexer unit,
`ub_safe` and `equiv` tests pass under ASan+UBSan and TSan. This wave is not the module's
last, so BSan was not run. `cargo test --workspace` passes, except for the five
compile-fail snapshots that the ffibox pin drift breaks.

### `libgit2/src/libgit2/odb.c` wave 180 batch 0: `git_odb_backend`

Wave 180 batch 0 wraps `git_odb_backend` as `OdbBackend`, with `OdbBackendBox` as its sole
owner. `OdbBackendFree` releases a backend through its own `free` member, and leaves alone a
backend without one, where libgit2 would call through NULL. `OdbBackend::new` binds
`git_odb_init_backend` (wave 182's symbol) as the inert template, which Coverage stats counts
as `+1` wrapped symbol. All 16 fields have accessors. `version` reads directly. `odb` is
compared, never followed (`in_odb`, `belongs_to`). Each member has a presence getter
(`has_<member>`). The exclusive handle calls the three members that take no object id:
`refresh` and `writemidx` read only a negative return as an error, as the odb does; an unset
`refresh` succeeds, and an unset `foreach` or `writemidx` fails with a `GIT_ERROR_ODB` error
naming the member. `foreach` tells a callback's break from a failure through a `stopped`
slot this batch adds to `ForeachCallback`. The id-taking members are not called directly:
the struct does not record the backend's id type, and libgit2's loose backend formats any id
into a path sized for its own type, so they stay with the odb's wrappers, which check the
id. `writestream` and `writepack` need the hashing context and the odb that only the odb's
own routines supply.

The two getters wave 176 batch 1 left raw until this type was wrapped,
`OdbStreamRef::backend` and `OdbWritepackRef::backend`, now return `Option<OdbBackendRef>`.
That is the `-2` in `pub raw ptr (safe)` and `pub raw ptr`. `pub unsafe fn` is `+1`, for
`OdbBackendFree`'s `CDrop::c_drop`. The safety audit row comes from `ffimap` scans of the
wave tip this landing rebased onto (`5a5d90372`) and of this tree. The tests adopt
libgit2's packed and loose backends through declared `git_odb_backend_pack` and
`git_odb_backend_loose`, without allowlisting them: their options types belong to waves 440
and 462. `+LoC` leaves out the 13 regenerated lines of `libgit2-sys/src/bindings.rs`.

Coverage cells stay `—`, for wave 172's reason. Source-based coverage on
`nightly-2026-10-07`, over the batch's unit, `equiv` and `ub_safe` tests, runs every one of
the 126 counted lines of the new section. ASan+UBSan and TSan ran every test matching `odb`.
Miri ran the batch's two Rust-only unit tests; each of the four marked `calls C` was run
alone there and seen to stop at `git_libgit2_init`. No test of the batch is expected to
trigger BorrowSanitizer, so none ran under it. `libgit2-sys` packages in shipped mode against
a staged `plain` prefix (`--offline`: the host has no network). The same five compile-fail
snapshots mismatch from the ffibox pin drift.

### `libgit2/src/libgit2/odb.c` wave 181: review of wave 180

Wave 181 batch 0 reviews wave 180 batch 0's `OdbBackend` (`9fe83bce6`) and files three
reports. Wave 180 batch 1 has not run, so its row covers the type alone. Under
`+lifecycle`, `odb-backend-constructors-and-droppers`: the store recorded only
`git_odb_init_backend` for the type. The four public `alloc` constructors
(`git_odb_backend_loose`, `_pack`, `_one_pack`, `git_mempack_new`) and the three `free`
members (`loose_backend__free`, `pack_backend__free`, `impl__free`) are now submitted, with
their out-parameter and argument blocks. Under `+misc`, following wave 173, two `contracts`
reports. `odb-stream-backend-invariant-unstated`: `OdbStreamRef::backend` and
`OdbWritepackRef::backend` now lend the `backend` field as a handle, but no stated contract
asked a `from_c` or `from_ptr` caller to vouch for that field. A reproducer that honours
`from_ptr`'s obligation reads a freed backend through safe `version()`, under Miri and BSan.
The batch's own unit test adopted a stream whose `backend` was `0x1000`. The types now state
the invariant, the SAFETY comments cite every path, and the test lends a live backend.
`odb-backend-box-adoption-asks-for-free`: `OdbBackendBox`'s adoption contract required a
`free` member that `OdbBackendFree` does not need, and left implicit the "no odb" condition
it does need. The unsafety, UB, equivalence, internal, conventions and ownership-store
checks found nothing more. Every field finding matches the C, the pack and loose members
the handle calls never follow the odb back-pointer, and the pack walk unlocks before calling
back. `+LoC` is `odb.rs` alone: `+19/-9` in docs and SAFETY comments, `+20/-5` in the unit
test.

The `cov (C/Rust)` cells stay `—`, for wave 172's reason. No code path changed, and wave
180 measured every counted line of the new section as run, so coverage was not measured
again and no coverage tests were added. The safety audit row comes from `ffimap` scans of
the wave tip (`d10f34cdd`) and this tree. Every counter is equal except test LoC (`+11`).
The Miri gate passed on the wave tip (`lib` 342, `ub_safe` 166, `ub_unsafe` 18 and `equiv`
32 passed, the tests that call C ignored), and `lib` passed again on the fixed tree. The
odb tests of `lib`, `ub_safe` and `equiv` pass under ASan+UBSan and TSan, on the tip and on
the fixed tree. This wave is not the module's last, so BSan ran only on the reproducer.
`cargo test --workspace` fails only on the five compile-fail snapshots that the ffibox pin
drift breaks. `cargo doc` with warnings denied fails on three pre-existing private links to
`MISMATCH` in `odb.rs` (waves 176 and 177), outside this workset.

### `libgit2/src/libgit2/diff_parse.c` wave 228 batch 0: `git_diff_parse_options`

Wave 228 batch 0 wraps `git_diff_parse_options` as `DiffParseOptions`, after `OdbOptions`'s
pattern, and adds `Diff::from_buffer_with_options`, which passes it to
`git_diff_from_buffer_ext`; `from_buffer_with_oid_type` keeps its anchor and now goes through
it. libgit2 has no init routine for these options, so `DiffParseOptions::new` spells
`GIT_DIFF_PARSE_OPTIONS_INIT` in Rust and calls no C. `git_diff_from_buffer_ext` never reads
`version`, so any version is stored and parsed with. It does not check `oid_type`, so the value
holds 0 or a named `OidType` only. Coverage cells stay `—`, for wave 172's reason.
Source-based coverage on `nightly-2026-10-07`, with test items under `#[coverage(off)]` in a
scratch copy and the batch's unit, `ub_safe` and `equiv` tests as the workloads, measures
`src/libgit2/src/libgit2/diff_parse.rs` at `97.92`% of lines (96, 2 missed), 100% of
functions and `98.02`% of regions. The two missed lines are the `runtime_pinned()` failure
branches. The safety audit row comes from `ffimap` scans of the wave base (`273bc0d4d`) and this
tree. No public `unsafe fn` and no raw pointer on the public interface is added. `Send/Sync` is
`+6`: `Send` and `Sync` for the value and its two handles. The derived `Clone` counts in no
column. ASan+UBSan and TSan ran the batch's unit, `ub_safe` and `equiv` tests. Miri ran them all.
Each test marked `ignore` under Miri was run there alone and stops at `git_libgit2_init`. No test
of the batch is expected to trigger BorrowSanitizer, so none ran under it. The five compile-fail
snapshots of other modules still mismatch from the ffibox pin drift.

### `libgit2/src/libgit2/diff_parse.c` wave 229: review of wave 228

Wave 229 batch 0 reviews wave 228 batch 0's `DiffParseOptions` (`0fcf482ab`) and files two
reports. Wave 228 batch 1 has not run, so its row covers the type alone. Under
`+conventions`, `diff-parse-options-entry-point-unanchored`: the batch's new
`Diff::from_buffer_with_options` is the only caller of `git_diff_from_buffer_ext` and takes its
whole options surface. The `Wraps` anchor and alias stayed on `from_buffer_with_oid_type`,
which only delegates to it. The anchor now sits on `from_buffer_with_options`, and the
wrapped-symbol count is unchanged. Under `+misc`, as a `wavefront` report,
`diff-from-buffer-ext-opts-note`: the `opts` note on `git_diff_from_buffer_ext`, submitted
by `8f238f339`, described `git_diff_options` (pathspec, callbacks, payload, nested
referents) and said `version` is read. The same properties are resubmitted with a note
stating that only `oid_type` is read, during the call and unchecked. The lifecycle,
unsafety, contracts, UB, equivalence and internal checks found nothing more. The type has no
lifecycle routines (`--lifecycle-ops` is empty, and `git_diff_from_buffer_ext` is its only
reader). `define_ctype!` keeps the layout field private, so safe code outside the module
cannot store an unnamed `oid_type`. The six `Send`/`Sync` impls follow ffibox's documented
pattern for a resource-free value. The C reads `oid_type` alone, and `GIT_OID_SHA256` is
built unconditionally. `+LoC` is doc comments in `diff_parse.rs` alone.

The `cov (C/Rust)` cells stay `—`, for wave 172's reason. No code path changed, and wave 228
measured `diff_parse.rs` at `97.92`% of lines, the misses being the environment-dependent
`runtime_pinned()` failure branches, so no coverage tests were added. The safety audit row
compares binder run 99's scan of the wave tip's Rust with a scan of this tree, and every
counter is equal. The Miri gate passed on the wave tip (`lib` 347, `ub_safe` 168,
`ub_unsafe` 18 and `equiv` 32 passed, the tests that call C ignored), and again on the fixed
tree. There, the side-by-side `ub_safe` invocation passed every test but exited on a
`libc_sys` "rlib format" error, a race between the concurrent builds sharing `target/miri`.
Rerun alone, it passed cleanly. The workset's `lib`, `ub_safe` and `equiv` tests pass under ASan+UBSan and TSan. This
wave is not the module's last (wave 230 follows), and the review emitted no reproducer, so
BSan did not run. `cargo test --workspace` fails only on the five compile-fail snapshots
that the ffibox pin drift breaks. `cargo doc` warns only on the three pre-existing private
`MISMATCH` links in `odb.rs`.

### `libgit2/src/libgit2/checkout.c` wave 330 batch 0: `git_checkout_perfdata`

Wave 330 batch 0 wraps `git_checkout_perfdata` as `CheckoutPerfdata`, a plain `Copy` value of
three `usize` counters with shared and exclusive handles, after `DiffParseOptions`'s pattern.
It has no lifecycle routines (`--lifecycle-ops` is empty). libgit2 counts into one inside the
zero-initialised `checkout_data` and lends it to `perfdata_cb` once, after a successful
checkout, for the call only. So the type is `CZeroable`, a callback reads its argument through
a `CheckoutPerfdataRef` borrowed for the call, and `copied` keeps the counts beyond it. Every
field has a getter and a setter. Checkout never passes `GIT_MKDIR_CHMOD*` to
`git_futils_mkdir_relative`, so `chmod_calls` is always 0 from libgit2. The equivalence test
asserts that, and covers the field with C's own reads. Coverage cells stay `—`, for wave 172's
reason. Source-based coverage on `nightly-2026-10-07`, with test items under
`#[coverage(off)]` in a scratch copy and the `checkout` tests of the unit, `ub_safe` and `equiv`
suites as workloads, measures `src/libgit2/src/libgit2/checkout.rs` at 100% of lines (331),
functions (61) and regions (338). The safety audit row compares `ffimap` scans of the wave
base (`1ff0d6308`) and this tree. No public `unsafe fn` and no raw pointer on the public
interface is added. `Send/Sync` is `+6`: `Send` and `Sync` for the value and its two handles.
The derived `Clone`/`Copy` counts in no column. ASan+UBSan and TSan ran the `checkout` tests
of the unit, `ub_safe` and `equiv` suites. Miri ran the batch's tests, and the one test marked
`ignore` stops at `git_libgit2_init` when run alone there. No test of the batch is expected to
trigger BorrowSanitizer, so none ran under it. The five compile-fail snapshots of other
modules still mismatch from the ffibox pin drift. `cargo doc` warns only on the three
pre-existing private `MISMATCH` links in `odb.rs`. The type's findings are in wavefront's
ownership store.

### `libgit2/src/libgit2/checkout.c` wave 330 batch 1: `git_checkout_progress_cb`

Wave 330 batch 1 binds the callback `git_checkout_progress_cb` as `checkout::ProgressCallback`,
after the indexer's `ProgressCallback`: an exclusively borrowed closure, an `extern "C"`
trampoline, `call`, and a crate-private `walk_with` for the wrappers of the routines that check
out. Its one invoker, `report_progress`, calls it synchronously during a checkout, so there is one
contract and no fork. That invoker passes a NULL path once, for the zero baseline, and otherwise a
path the checkout owns for the call, which the closure receives as an `Option<&CStr>` that cannot
outlive the call. The callback returns `void`, so it cannot cancel. A panic lets the checkout finish,
every later call skips the closure, and the panic resumes once control is back in Rust. The options'
`set_progress_callback` stays a raw seam for C callbacks; its docs and the module's now point to
the wrapper. A second public `ProgressCallback` makes rustc spell the indexer's by its path, so the
indexer's `progress_callback_not_send` snapshot was blessed again, its error otherwise unchanged.

Coverage cells stay `—`, for wave 172's reason. Source-based coverage on `nightly-2026-10-07`,
with test items under `#[coverage(off)]` in a scratch copy and the batch's unit, `ub_safe` and
`equiv` checkout tests as the workloads, measures `src/libgit2/src/libgit2/checkout.rs` at 100% of
lines (379), functions (69) and regions (384) on the landed tree, batch 0's code included. The unit tests have libgit2 call the callback
through `walk_with` during real `git_checkout_index` runs, a dry run and a panic included. The
`equiv` tests run a C callback beside it on independent fixtures, over tree and index checkouts,
removals, pathspecs, dry runs and conflicts, and compare every call. The safety audit row comes from
`ffimap` scans of the wave base (`1ff0d6308`) and this tree, and again of batch 0's tip
(`eda4ae102`) and the rebased tree, which agree. No public `unsafe fn` and no raw
pointer on the public interface is added. `deref outside impl` is `+1`: the trampoline's payload
dereference in a free `extern "C"` fn (`raw_ptr.deref.wr` `+1`), as in wave 178.

ASan+UBSan and TSan ran the batch's unit, `ub_safe` and `equiv` tests. TSan first reported
libgit2's `filter_initialize` race between two unit fixtures staging files at once. The fixture now
builds its first instance alone, as the `equiv` checkout fixture does. Miri ran the unit and
`ub_safe` tests. Each test marked `ignore` under Miri was run there alone and stops at
`git_libgit2_init`. No test of the batch is expected to trigger BorrowSanitizer, so none ran under
it. The five compile-fail snapshots of other modules still mismatch from the ffibox pin drift. The
three new snapshots pass.

The batch rebased onto batch 0 (`eda4ae102`) under the landing lock. Both batches add to
`checkout.rs`, its `equiv` and `ub_safe` tests and the dashboard, so every conflict kept both sides.
Over the resolved tree, the workspace's check, format, clippy and tests (`ub_unsafe` and the
doctests included) ran again, and so did ASan+UBSan, TSan and Miri on every `checkout` test. All
passed but for the five drifted snapshots.

### `libgit2/src/libgit2/checkout.c` wave 331 batch 1: review of wave 330 batch 1

Wave 331 batch 1 reviews wave 330 batch 1's `ProgressCallback` (`3b654f097`) alone; batch 0
reviews `CheckoutPerfdata`, at the user's request. It files one `misc` report,
`progress-callback-docs-promise-every-step`. The docs promised a call after every step, but
libgit2's `checkout_remove_conflicts` counts each stale index conflict it removes as a step and
reports none. A checkout that only removes conflicts calls back once, with `(None, 0, n)`, so
a closure never sees the steps complete. The wrapper matches C call for call. The fix corrects the
module and type docs. Two `equiv` tests compare the case with a C callback, and also the symlink
and submodule calls (`checkout.c:1900`, `1921`), which no test reached before;
`compare_progress` now takes a fixture-preparing function instead of a conflict flag. The
lifecycle, unsafety, contracts, UB, internal and conventions checks found nothing more:
`ProgressCallback` repeats the reviewed indexer pattern, wave 179's stale-panic fix
included, and the store's findings for `git_checkout_progress_cb` match the other callbacks'.
`+LoC` is the Rust tree alone: 11 doc lines and the 102-line test diff, not the reproducer.

The `cov (C/Rust)` cells stay `—`, for wave 172's reason; the batch measured `checkout.rs` at
100% of lines, and this review adds no Rust code. The safety audit row comes from `ffimap`
scans of the wave base (`9dacc279e`) and this tree. Every counter is equal; only the test
counters moved. The Miri gate passed on the wave tip in all four binaries, and the `equiv`
binary passed again on the fixed tree. Each new test was run alone under Miri and stops at
`git_libgit2_init`. The new tests, and the batch's unit and `ub_safe` progress tests, pass
under ASan+UBSan and TSan. This wave is not the module's last, since wave 332 follows it, so
BSan was not run, and the report's reproducer shows no UB. `cargo test --workspace
--no-fail-fast` passes, except for the five compile-fail snapshots that the ffibox pin drift
breaks.

### `libgit2/src/libgit2/checkout.c` wave 332 batch 0: the notify and perfdata callbacks

Wave 332 batch 0 binds `git_checkout_notify_cb` as `checkout::NotifyCallback` and
`git_checkout_perfdata_cb` as `checkout::PerfdataCallback`, after wave 330's `ProgressCallback`:
an exclusively borrowed closure, an `extern "C"` trampoline, `call`, and a crate-private
`walk_with` for the wrappers of the routines that check out. Each has one invoker, so one contract
and no fork. `checkout_notify` calls the notify callback while the checkout plans, before it writes,
dry runs included. Its closure takes a `CheckoutNotification<'_>`, a `Copy` struct of the reason
bit, the path and the baseline, target and workdir sides as `Option<DiffFileRef<'_>>`, all lent
for the call. One struct rather than five arguments lets a closure name the call's lifetime once:
inferred `DiffFileRef` arguments are not higher-ranked. `ControlFlow::Break` cancels with
`GIT_EUSER`, and so does a panic, whose later calls cancel without the closure and which resumes
in Rust, as the indexer's callback does. `git_checkout_iterator` calls the perfdata callback once,
after every write, never on a dry run, a failure or a cancel. It lends `&data.perfdata` as a
`CheckoutPerfdataRef`, returns nothing and cannot fail the checkout. The options' notify and
perfdata accessors stay raw seams for C callbacks, like the progress ones; their docs and the
module's now point to the wrappers. T3's `erased_field` row typing `perfdata_payload` as a
`git_checkout_perfdata` reflects libgit2's own tests; the payload is the caller's. The callbacks'
pointer findings are in wavefront's ownership store.

`loc` is the non-test lines `checkout.rs` gains, as wave 330 counted it; `+LoC` counts the
five new compile-fail sources. Coverage cells stay `—`, for wave 172's reason. Source-based
coverage on `nightly-2026-10-07`, with test items under `#[coverage(off)]` in a scratch copy and
the `checkout` unit, `ub_safe` and `equiv` tests as the workloads, measures
`src/libgit2/src/libgit2/checkout.rs` at 100% of lines (499), functions (90) and regions (506). The
unit tests drive both callbacks through `walk_with` during real `git_checkout_index` runs: every
reason with its sides, a cancel, conflicts, a dry run and panics in each. The `equiv` tests run C
callbacks beside them on independent fixtures, over tree and index checkouts, conflicts, dry runs,
partial flags and cancels, and compare every call. A cancelled checkout's message is
libgit2's: it is set only when the thread holds no error, and libgit2's config refresh sometimes
leaves a benign one (no `$HOME/.gitconfig`), on either side and with C callbacks alone too, so
those steps accept either message. The safety audit row compares `ffimap` scans of the wave base
(`660a045fc`) and this tree. No public `unsafe fn` and no raw pointer on the public interface
is added. `deref outside impl` is `+2`, the two trampolines' payload dereferences; the one new
`Clone` is derived, on `CheckoutNotification`.

ASan+UBSan and TSan ran the batch's unit, `ub_safe` and `equiv` tests, and Miri the unit and
`ub_safe` ones. Each test marked `ignore` under Miri was run there alone and stops at
`git_libgit2_init`. No test of the batch is expected to trigger BorrowSanitizer, so none ran
under it. The five compile-fail snapshots of other modules still mismatch from the ffibox pin
drift; the five new ones pass.

### `libgit2/src/libgit2/checkout.c` wave 331 batch 0: review of wave 330 batch 0

Wave 331 batch 0 reviews wave 330 batch 0's `CheckoutPerfdata` (`eda4ae102`) alone, at the
user's request: batch 1 reviews `ProgressCallback`, and review wave 333 reviews wave 332, which
landed first. It files one report, under `+misc`, `perfdata-reported-before-index-write`. The
module docs and `set_perfdata_callback` said libgit2 reports the counts once a checkout
succeeds, and the batch's ownership-store note said the same. But `git_checkout_iterator` calls
`perfdata_cb` before `git_index_write` (`checkout.c:2680-2686`), and that write can still fail the
checkout. A new `equiv` step with `.git/index.lock` present sees one report followed by
`GIT_ELOCKED`. Both docs are corrected, and the note is resubmitted through `--update`. The
lifecycle, unsafety, contracts, UB, equivalence, internal and conventions checks found nothing
more. The type has no lifecycle routines (`--lifecycle-ops` is empty), and its three fields
hold no pointer. The six `Send`/`Sync` impls follow ffibox's documented pattern for a
resource-free value. Every `unsafe` block is one projection, read or write through a handle's
pointer, and its justification holds. `chmod_calls` is 0 from libgit2, since checkout never
passes `GIT_MKDIR_CHMOD*`.

The coverage objective adds the `equiv` test
`perfdata_reads_match_c_on_links_update_only_and_a_failed_index_write`. It reaches three C
counting paths the batch's own tests left out: a symbolic link written (`blob_content_to_link`),
files updated under `GIT_CHECKOUT_UPDATE_ONLY` (`checkout_safe_for_update_only`), and the failed
index write. Rust and C read the same counts after every step. The `cov (C/Rust)` cells stay `—`,
for wave 172's reason. The Rust code is unchanged but for docs, and wave 330 measured
`checkout.rs` at 100%. The safety audit row compares `ffimap` scans of the wave tip
(`9dacc279e`) and this tree: only test code changes. The Miri gate passed on the wave tip
(`lib` 366, `ub_safe` 184, `ub_unsafe` 18 and `equiv` 32 passed, with the tests that call C
ignored). The `equiv` binary passed again on the fixed tree, the only binary whose code
changed. The new test was run alone under Miri and stops at `git_libgit2_init`. The workset's
`lib`, `ub_safe` and `equiv` perfdata tests pass under ASan+UBSan and TSan. Wave 332 follows
in this module, and the review emitted no reproducer, so BSan did not run. `cargo test
--workspace --no-fail-fast` fails only on the five compile-fail snapshots that the ffibox pin
drift breaks.

### `libgit2/src/libgit2/checkout.c` wave 333: review of wave 332

Wave 333 batch 0 reviews wave 332 batch 0's `NotifyCallback` and `PerfdataCallback`
(`b694c3653`) and files two reports, both under `+misc`. The first,
`checkout-notify-repeats-and-workdir-side`, concerns the module docs and `NotifyCallback`'s
docs, which promised one call per path, and `CheckoutNotification::workdir`, documented as
`None` only for a path missing on disk. Where a directory stands in place of a file that the
baseline and the target share, `checkout_action_with_wd_dir` (`checkout.c:612-617`)
notifies the file as dirty with no working-directory side. It then notifies the directory,
as `a.txt/`, as untracked, and the workdir walk notifies it again. Under
`GIT_CHECKOUT_FORCE`, the file is also notified as updated, with the directory as its
working-directory side. The docs of the four fields now state when each is `None`, and that
a side can be a directory. The second, a `wavefront` report,
`checkout-notify-cb-side-notes`, concerns the store's notes on `git_checkout_notify_cb`'s
baseline, target and workdir sides, which repeated the claim and missed the workdir-only
dirty case (`checkout.c:413-415`). The same properties are resubmitted with complete notes.

Its usage cells sum two runs. Reviewer run 130 reviewed wave 332 beside review wave 331, from the same base on its own integration branch, so its fix conflicted with 331's landing and could not be promoted; reviewer run 131 replayed its two commits onto 331's landing, kept both sides of the conflicts, ran the gates again and landed them ($9.40 and $1.63).

The lifecycle, unsafety, contracts, UB, equivalence, internal and conventions checks found
nothing more:
- Callbacks have no lifecycle routines.
- The batch adds no public `unsafe fn` and no raw pointer to the public interface. The
  options' raw callback setters and getters are an earlier wave's seams, and their contracts
  still hold.
- The trampolines' `&mut` over the payload aliases nothing: `walk_with` lends a pointer
  derived from `&mut self` and uses `self` only once the walk returns.
- The closures are higher-ranked over every lent borrow, so nothing escapes the call, as the
  compile-fail cases check.
- libgit2 never passes NULL `path` or `perfdata`, and the trampolines tolerate it anyway.
- The callbacks are neither `Send` nor `Sync`.

`+LoC` is the doc fix and the two new tests. Two C branches had no test, ignored files
(`checkout.c:438-448`) and a directory in place of a kept file (`checkout.c:612-617`):
- `a_directory_in_place_of_a_kept_file_is_notified_more_than_once` drives the real trampoline
  through `walk_with` over both branches, safe and forced. It is a unit test, `+1`.
- `a_notify_callback_sees_directories_and_ignored_files_as_a_c_callback` compares a
  `NotifyCallback` with C callbacks call for call over the same steps and a cancel. It is
  an `equiv` test, `+1`, and passed in 5 of 5 runs.

Ignore rules apply from the repository's working directory, not a `target_directory`, so the
unit test writes `.git/info/exclude`. The `equiv` harness's `Step` gains a `prepare` hook.

The `cov (C/Rust)` cells stay `—`, for wave 172's reason. No non-test code changed, and
wave 332 measured `checkout.rs` at 100% of lines, functions and regions. The safety audit row
compares binder run 102's scan (`b694c3653`, the Rust of the wave tip, which an `ffimap`
rerun on `9dacc279e` matched counter for counter) with a scan of this tree. Only the
`code.tests.*` counters change.

The Miri gate passed on the wave tip: `lib` 366, `ub_safe` 184, `ub_unsafe` 18 and `equiv` 32
passed, and the tests that call C were ignored. It passed again on the fixed tree with the same counts. The two new tests are marked `ignore` there: each was run alone under Miri and stops at `git_libgit2_init`. The checkout unit, `ub_safe`
and `equiv` tests pass under ASan+UBSan and TSan. This wave is not the module's last (waves
334 and 356 follow), and the review emitted no reproducer, so BSan did not run.
`cargo test --workspace --no-fail-fast` fails only on the five compile-fail snapshots that
the ffibox pin drift breaks. `ub_unsafe` and the doctests pass.

Reviewer run 130 reviewed the batch beside review wave 331, so its commits could not be
promoted. Run 131 replayed them onto wave 331's landing, keeping both sides of the conflicts:
wave 331's `equiv` perfdata and progress tests beside this batch's notify test, and both
batches' dashboard rows, with the `Σ` rows and the Coverage stats counts recounted. On the
resolved tree, check, fmt, clippy and `cargo test --workspace --no-fail-fast` were rerun.
Equiv now has 489 tests and unit 787, and only the five compile-fail snapshots from the
ffibox pin drift fail. The Miri gate's `lib` and `equiv` binaries were rerun, the two whose
code the merge combines, and passed with 366 and 32 tests. The checkout unit, `ub_safe` and
`equiv` tests were rerun under ASan+UBSan and TSan and pass. Neither side changed non-test
code beyond docs, so the safety audit row and the `ub_unsafe` Miri result stand.

### `libgit2/src/libgit2/checkout.c` wave 334 batch 0: `git_checkout_options` callbacks

`CheckoutOptions` is re-bound with the callback wrappers waves 330 and 332 added. The
setters `set_notify_callback`, `set_progress_callback` and `set_perfdata_callback` are safe:
each takes an `Option<&'o mut …Callback<'_>>`, which the options borrow exclusively for their
lifetime, and stores the module's trampoline as `*_cb` and the callback itself as
`*_payload`. The getters return a `CallbackSlot` (`Empty`, `Rust` or `Foreign`) instead of
the raw function and payload. `Rust` means the slot holds the trampoline, compared with
`ptr::fn_addr_eq`, and a payload. Options C filled in, and the checkout options a
`git_worktree_add_options` embeds, can hold C callbacks that no Rust wrapper stored. Those are
`Foreign` and are left alone, and C interop installs C callbacks through the raw seam,
`CMutSeam::as_mut_ptr`. A panic of a stored callback resumes from the crate-private
`CheckoutOptionsRef::walk_with`, which wraps the routine's C call. `Worktree::add` now uses it
for its embedded options, and the checkout routines' wrappers (wave 356) will too. A closure
can reach its own options through interior mutability (a `RefCell`, which the `ub_safe` test
compiles) and check out with them again. So each callback carries a `busy` flag, and the
trampolines no longer hold a `&mut` to the callback across the closure. A nested call cancels
(notify, `GIT_EUSER`) or skips (progress, perfdata). Miri runs the unit tests that drive the
trampolines through the options' fields, the nested calls included. The rebase wrappers must
also keep stored callbacks alive across a rebase, since `git_rebase_init` keeps the payloads.

`ub_unsafe`'s checkout suite tested only the three removed `unsafe fn`s, so it is retired
(`-9`). Its scenarios move to `ub_safe`, which runs real checkouts through `Worktree::add`:
callbacks run, a break cancels, panics resume after the worktree box is freed, a nested add
finds the callback busy, and threads share nothing. Each runs in an `isolated` child, since
the checkouts load filters. The compile-fail case `checkout_options_raw_seams_require_unsafe`
now writes through the seam, and three new cases reject a stored callback that is dropped,
called, or stored twice. The equivalence suite's option sequences still install C callbacks,
through the seam. Its callback comparisons store the Rust callbacks with the safe setters on
options built for each step, since a stored callback stays borrowed for the options'
lifetime. That removes the C-ABI forwarders.

Coverage was measured with `crustify/scripts/coverage/` on this tree and, for the `pp` cells,
on the wave base (`14822244e`) in the same environment. The base there reads `45.89`% /
`98.92`%, where the table's `1e732ebff` figures read `45.95`%. Against the base, the C union
loses 30 lines and gains 14, in files this batch does not touch (`httpclient.c`,
`patch_parse.c`, …), except one line of `fs_path.h` that only the retired suite reached.
`checkout.rs` is fully line-covered. The safety audit row compares `ffimap` scans of the wave
base and this tree (`a15f6f3`, the ten seams). The three `unsafe fn`s and the three raw-pointer
getters leave the public interface. `deref outside impl` is `+12`: each trampoline now
dereferences its payload five times instead of once. The one new `Clone` is derived, on
`CallbackSlot`. ASan+UBSan and TSan ran the batch's unit, `ub_safe` and `equiv` tests. Each
new `ub_safe` test marked `ignore` under Miri was run there alone and stops at the child's
spawn (`readlink`) or at `git_checkout_options_init`. No test of the batch is expected to
trigger BorrowSanitizer, so none ran under it. The five compile-fail snapshots of other
modules still mismatch from the ffibox pin drift.

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

### Campaign moved to `libgit2-bindings`

On 2026-10-23 the campaign moved from the libgit2 checkout at `/target`, where it lived
under `crustify/`, to its own repository, as the current `specs/versioning.md` lays it out:
the C is the `c/` submodule at `0551dfd4a`, the safe crate and the workspace are at the
root (`crustify/rust/libgit2/` → `src/`, `tests/`; `crustify/rust/libgit2-sys/` →
`libgit2-sys/`), and the branches are `main` and `crustify/devel`. The history was
rewritten onto `c/`, keeping every commit; the old hashes map to the new ones in
`crustify/tmp/migration-commit-map.txt`, and the old repository keeps the old branches.

- Waves carry global indices, in the order they ran: `0` (`raw-lifetime-void` wave 0)
  through `53` (`index` review wave 2). Every batch row's `wave` is that index, and its
  `module` cell keeps the subsystem it ran in, as `libgit2/<subsystem>`, since a subsystem
  spans several of the current modules.
- Each batch's records moved from `crustify/artifacts/libgit2/<subsystem>/<wave>/batch-<n>/`
  to its agent's run dir, `binder/run-1` … `binder/run-92` and `reviewer/run-1` …
  `reviewer/run-44`, numbered in launch order; each `task.json` gains its `wave`, `batch`,
  `link_unit`, `module` (`null`), `foundational` and new `base_branch`, its reviewed
  `commits` are the rewritten hashes. The migration commit, `436d4f2f9`, records each run
  dir's old location as the renames of its files (`git show -M --name-status 436d4f2f9`), and
  the old batch branch is named from it.
  Planning run 0 is `planner/run-1`. Each wave's records are under `waves/wave-<index>/`,
  and each subsystem's end-of-sub-campaign scan, as `subsystem-ffimap*.json`, under its
  last wave's.
- Review findings moved from `crustify/reviews/libgit2/<subsystem>/wave-<n>/` to
  `crustify/reviews/wave-<index>/`, the review wave's index; each reproducer's path
  dependency was repointed at the root crate.
- `Module stats` keeps one row per subsystem until the modules' rows can be filled.
  `planners` now holds `planner/run-1`, the former run 0, and the two `Σ` rows include it.

### Lint conformance

On 2026-10-24 the workspace took `specs/rust.md`'s lint table, deny levels included, and
was brought to build without a warning under it (`d499792e3`), by hand and by scripts
the orchestrator ran, outside any wave: 801 unsafe blocks held several operations and
are split to one each, with its own SAFETY comment; 325 fallible and 143 panicking
public fns gained `# Errors` and `# Panics` (or, for a panic that cannot happen, a local
`#[expect(clippy::missing_panics_doc)]` naming why); 93 items gained docs; about 450
lossy or trivial casts became `cast_signed`/`cast_unsigned`, `try_from` or the low byte
of `to_le_bytes`; and the pedantic lints test scenarios trip on purpose take local
`#[expect]`s with reasons. No behaviour changed: every suite passes as before. The
change has no batch row and no agent cost.

### The Rust tree homed per translation unit

Also on 2026-10-24, the tree moved from the old subsystem modules to planner run 2's
modules, one per TU at its path below the C root (`324180e8f`):
`libgit2::object::commit` is now `libgit2::src::libgit2::commit`, the typed lookups and
dups of blob, commit, tag and tree are in `object_api`, `core::common` is `settings` and
`libgit2`, and so on; the test suites follow, as `tests/<suite>/<TU path>.rs`, a TU's
`sys/` header tests as its `sys` module. Each item went where the todo waves home it; the
done waves keep the homes they ran with, so their rows above name the old files.

### Side campaigns set up

The out-of-tree libraries got their wrappers' side campaigns, beside this repository:
`openssl-bindings` (libcrypto, libssl), `pcre2-bindings` (libpcre2-8) and `zlib-bindings`
(libz), new, and `glibc-bindings` (libc), continued, with this campaign as its second
parent. Each new one holds its library at the release libgit2 links under `c/`, the
settings, a dashboard, its builds (installed once per variant of this campaign, with the
same instrumentation but under `bsan`, where they are built uninstrumented: BorrowSanitizer
reports false positives inside OpenSSL and PCRE2, so `bsan` instruments libgit2 alone), its own test suite as baseline and wavefront's CodeQL tables;
none has a Rust tree yet, nor a pin here, until its first planner run plans it. They
are not on `crustify-rs` yet. `build.json` version 3 links every variant of libgit2 to
those installs, libc staying the system's; the C baseline ran again against them and
matches its version-1 record in every variant, and `libgit2-sys` puts the installs on the
test binaries' rpath.

### ffibox's seam traits

ffibox `7950244` (`main`, merged from `feat/public-handle-seam`) moves the handles' raw
seam (`from_ptr`, `as_ptr`, `as_mut_ptr` and their `void *` twins) from inherent
`pub(crate)` methods that `define_ctype!` and `impl_cguarded!` emitted onto the traits
`CRefSeam`, `CMutSeam` and `CLockedSeam`, so that a crate holding another crate's handles,
such as a parent over its side campaigns' wrappers, can reach the pointer. Every file that
uses the seam imports the traits it calls; the macro-made `as_ref` / `as_mut` stay inherent,
as the user-facing API. The public surface is unchanged (ffimap: 48 `unsafe fn`, 5 raw
pointers in safe signatures, 0 in public fields). The compile-fail case
`commit_builder_handles` now checks that safe code outside the crate cannot make a handle
even with the trait in scope (E0133), where it checked that the inherent `from_ptr` was
private, a claim `CCell::mut_from_ptr` already made vacuous.

### Annotation pass

On 2026-11-02 the conventions' Anchors section changed to kind-qualified anchors in backticks,
``/// Wraps `fn::<name>` ``, each beside a `#[doc(alias = "<name>")]`, on the public item that stands
in for the C item. The landed bindings are brought to it by review batches with the `conventions`
and `fix` objectives, one per `.rs` file, on the `paths` route, filing no reports, under
`openrouter/openai/gpt-6.1-sol`: a smoke batch on `filter.rs` (wave `524`, `reviewer/run-45`,
`$0.66`), then the remaining files in wave `525`. Neither wave has an entry in `schedule.json`, and
their batches fill no dashboard rows. Each file's checklist lists the scheduled items whose
anchors sit in it, since the schedule's `home`s predate the current tree. From wave `525` the
settings' `workload` answer raises the parallelism to `32` agents.

Wave `525` landed all 40 files, at one commit each: `reviewer/run-53`, `run-60` and `run-72` under the
prompts before the landing lock, `$1.82`, and `run-86` to `run-122` on the current prompts, `$30.73`,
a median run of 15 minutes, all under `openrouter/openai/gpt-6.1-sol`. Its other 31 runs of
`run-46` to `run-79` were paused while ffmpeg's wave `671` landed and then discarded, `$21.41`, so
that their batches ran with trimmed instructions; `run-80` to `run-85` never started. Their records
are kept. The 37 relaunched runs, up to 20 at once, landed one at a time under the landing lock,
each rebasing once, in 41 minutes. The `/// Handwritten` markers, which ffibox's specs no longer ask
for, were then removed from the tree, and the workspace's gates passed over the result but for the
five `tests/ub_safe` compile-fail snapshots that fail on this host before any change, whose
rustc gutter is wider than the snapshots record.

From 2026-11-03 the settings' `workload` answer leaves the parallelism to the assistant, and their
new `identity` answer makes Marius Momeu the author of the campaign's commits, with Crustify as
co-author. `build.json`'s commands carry no job count, its `version` unchanged: the jobs are given
through the environment of each run.

On 2026-11-03 the campaign's checkout moved to `/target/libgit2-bindings`, beside its side
campaigns' clones. Its variants had been built in `/work/libgit2-bindings`, where the configure
commands' relative paths to the OpenSSL, PCRE2 and zlib installs, `../../../../<source>-bindings`,
named clones that did not exist there: those variants linked the system's libraries, uninstrumented,
in place of the side campaigns' installs, and every gate run there tested against them. Every
variant was configured again from the checkout, its commands and `build.json`'s `version`
unchanged, and links the side campaigns' installs of the same variant. The C baseline's `offline`
and `util` suites pass on `plain`, `util` on every other variant, and the workspace's tests pass but
for the five `tests/ub_safe` compile-fail snapshots that fail on this host.
