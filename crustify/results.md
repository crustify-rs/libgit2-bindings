# crustify — `libgit2 / libgit2`

Copy this template into the campaign checkout, then fill it as work lands.

## Campaign

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
- **branch** — `crustify/campaigns/20261008-170008`, tip `<sha>`
- **deps** — crustify `0fcfcae` (`main`, from `libgit2/core`; through `libgit2/oid` `7912683` with `scan-unsafe` from a local driver port, since superseded by upstream's nightly pin `4efc0cc`; every batch and wave `scan-unsafe` from `4771856`, the sub-campaign and campaign scans from `cbf6fbc`), ffibox `695d52d` (`main`)
- **build version** — `1`
- **C test baseline** — `3443/3498, 55 skipped, and ctest targets invasive, gitdaemon, gitdaemon_namespace, gitdaemon_sha256, ssh, online, online_customcert, proxy, auth_clone, auth_clone_and_push disabled`

## Review pass

Thin batches with `"objective": "review"`, LLM-as-a-Judge over each landed wave.

- **agent backend** — `claude`
- **model** — `openrouter/anthropic/claude-fable-5.1` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` from `libgit2/object` review wave 1
- **`--billing`** — `api`
- **`--max-types`** — `2` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave; from `libgit2/repository`, `20` (10× the translation cap), a route split into near-equal batches only past it
- **`--max-syms`** — `25` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave; from `libgit2/repository`, `100` symbols and callbacks (4× the translation cap), a route split into near-equal batches only past it
- **`--max-loc`** — `500` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave
- **`--min-fields`** — `20` through `libgit2/oid`; from `libgit2/core`, uncapped: a review batch holds its route's whole wave
- **orchestrator parallelism** — `10` concurrent batch processes through `libgit2/core`; `16` from `libgit2/repository`
- **branch** — `<branch>`, tip `<sha>`
- **agents** — `<n>`, over `<n>` wave(s)

`rv`-prefixed columns below carry the review pass; the unprefixed ones remain
the campaign's.

## Legend

- `objective` — what the batch's agents were told to do: `wrap`, `port`, or
  `raw lifetime`. The Types tables are split by it, so it appears as a column
  only in the Symbols `Batches` table, which mixes the two
- `subsystem` — in the Types and Symbols tables, the batch's sub-campaign as
  `<link-unit>/<subsystem>`, the same name as its Overview row; with `wave` and `batch`
  it keys the row to `schedule.json`
- `wave` / `batch` — the batch's position in its sub-campaign's plan: the index
  of its wave in `waves` and of the batch within that wave's `batches`, so a
  batch row joins to `schedule.json`, to the agent that landed it, and to its
  Overview sub-campaign row. Schema-v2 plans carry `steps` and `plan_items`
  rather than indexed waves and batches, so historical rows leave both blank
- `types` / `symbols` — scheduler units in the batch. Callbacks are scheduled
  in symbol batches and counted there
- `fields` — in-scope fields: the field accessors the oracle assigned to that
  type batch, not the type's full declared field count
- `strategies emitted` — in `Raw lifetime discovery`, the release and clone
  strategies that tier's batch emitted for its `void` or string primitives
- `$` / `wall` / `loc` — that agent's computed cost, its elapsed time, and the
  `.rs` insertions of its landing commit. `wall` is `ended_at − started_at` from
  the agent's own `usage.json`, so it INCLUDES the per-worktree C rebuild
- `$/type` / `$/symbol` / `$/field` / `$/loc` — that row's `$` over its units,
  its in-scope fields, or its `loc`
- `tokens` / `rv tokens` / `batch tokens` / `review tokens` / `total tokens` — the
  tokens behind the `$` cell to their left, in millions: input + output + cache read +
  cache write, summed over the agent's `usage.json` requests (the figure `crustify cost`
  reports). Cache reads dominate it
- `sub-campaign` — the Overview row's `<link-unit>/<subsystem>`, or
  `raw-lifetime-void` / `raw-lifetime-string`
- `sub-campaign wall` — from its first batch launch through its final review
  and regression gate
- `batch` / `review` — in the Overview, the computed cost of the sub-campaign's
  translation batches and of its review batches; `—` where it ran none
- `total` — `batch` + `review`; for the orchestrator, its own cost
- `$/type` / `$/sym` — in the Overview, a sub-campaign's cost over the types or
  symbols it was scheduled for; `—` where it was scheduled for none
- `+LoC` — the landed batch's Rust-source insertions/deletions relative to its
  wave's base
- `cov (C/Rust)` — in a `Batches` table, the absolute line coverage right after
  that batch landed, from ONE run of all four test workloads together (their
  union): C over the sub-campaign link unit's TUs in the `coverage` prebuild,
  Rust over the authored non-test sources. Its **Σ** cell is the coverage at the
  table's last landing, not a sum
- `+UB safe tests` / `+UB unsafe tests` / `+equiv tests` / `+unit tests` — in a
  `Batches …: tests` table, the tests added by that landed batch; each parenthesized pair is its C/Rust
  line-coverage change in percentage points. Negative coverage deltas are valid
  when the landed source adds more executable lines than the new tests cover
- `+lifecycle` … `+misc` — in a `Batches — review: PoC + reports` table, the
  defect reports that review batch filed under
  `crustify/reviews/<artifact-dir>/<category>/`, one per defect it reported;
  `+UB safe` / `+UB unsafe` are its `ub_safe` and `ub_unsafe` categories.
  `+lifecycle` covers type and raw-lifetime items only, so it reads `—` for
  symbols. `total` is the row's sum
- `+rejected PoCs` — the `ub_safe`, `ub_unsafe` and `equiv` PoCs that review
  batch filed which the orchestrator rejected after rerunning them: they did not
  reproduce, or did not show a real bug, and their fixes were reverted. They
  stay counted in their categories; `total` minus this column is the confirmed
  defects
- `+unsafe fn smell` / `+raw-ptr smell` — the change the landed batch made to the
  `unsafe fn` smell and the raw-pointer smell (total − seam) of the unseeded
  static scan, at its landing against the commit before it; a review batch's fixes show as
  negative values
- `rv $` / `rv wall` / `rv loc` — the REVIEW agent's cost, elapsed time, and net
  `.rs` line delta (`+ins/-del`) of its landing commit. Under subscription
  billing `rv $` is an API-equivalent comparison value, not a charged amount
- `UB tests` / `Equiv tests` / `Unit tests` — counts of `#[test]`
  functions under `mod ub_tests` / `mod equiv_tests` / `mod units_tests`. Each
  coverage pair comes from running only that workload, against the
  coverage-instrumented target C sources and authored Rust sources; state any
  excluded files or generated code in Notes. The three workloads bound different
  obligations, so report them apart and never sum them. Report the UB
  workload once, from a single coverage run on the plain build: it executes once
  per prepared sanitizer, but those runs share one test set and re-counting them
  inflates the figure. Name the instruments it ran under in Notes
- `Total tests` — the three counts added. Its coverage pair is NOT the three
  pairs added: it comes from one run of all three workloads together and is the
  union of the lines any of them reached, which is smaller than the sum wherever
  two workloads cover the same line. Deriving it arithmetically overstates it.
  It is a reach figure only — it mixes three verdict sources and so bounds no
  single obligation, which is why the three stay reported apart above it
- `total types` / `total symbols` — the units the target's public API exports, deprecated
  items excluded: every type, and every symbol and callback, scheduled across all of
  `schedule.json`, raw-lifetime items aside. They are fixed for the campaign
- `wrapped types` / `wrapped symbols` — scheduled units whose `Wraps:` or `Replaces:` anchor
  is present at the campaign tip; the parenthesized share is of `total types` /
  `total symbols`

Every table below is a heading, a model line and the table. All prose belongs
in Notes.

## Overview

- **Rust LoC, non-test** — `<n>`
- **Rust LoC, tests** — `<n>`
- **UB safe tests** — `<count of #[test]>` (`<n>`% C LoC coverage, `<n>`% Rust LoC coverage)
- **UB unsafe tests** — `<count of #[test]>` (`<n>`% C LoC coverage, `<n>`% Rust LoC coverage)
- **Equiv tests** — `<count of #[test]>` (`<n>`% C LoC coverage, `<n>`% Rust LoC coverage)
- **Unit tests** — `<count of #[test]>` (`<n>`% C LoC coverage, `<n>`% Rust LoC coverage)
- **Total tests** — `<sum of the three counts>` (`<n>`% C LoC coverage, `<n>`% Rust LoC coverage)
- **C LoC** — `<n>`
- **total types** — `258`
- **total symbols** — `979`
- **ported types** — `<n>`
- **ported symbols** — `<n>`
- **wrapped types** — `77` (`29.84`% of total types)
- **wrapped symbols** — `341` (`34.83`% of total symbols)

Implementation `openrouter/anthropic/claude-opus-5.5` via `claude`, `openrouter/openai/gpt-6.1-sol` via `codex` for `libgit2/config`, `openrouter/openai/gpt-6-astra` via `codex` for `libgit2/index` wave 0; review
`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0, `openrouter/anthropic/claude-opus-5.5` via `claude` from its review wave 1.

| sub-campaign | nr types | nr symbols | sub-campaign wall | batch | batch tokens | review | review tokens | total | total tokens | $/type | $/sym |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `raw-lifetime-void` | `0` | `2` | `1h54m19s` | `$8.28` | `17.79M` | `$7.89` | `10.78M` | `$16.17` | `28.57M` | — | `$8.09` |
| `raw-lifetime-string` | `0` | `2` | `52m30s` | `$5.49` | `12.39M` | `$10.89` | `14.41M` | `$16.38` | `26.80M` | — | `$8.19` |
| `libgit2/util` | `4` | `5` | `2h24m28s` | `$19.86` | `46.74M` | `$46.50` | `56.66M` | `$66.36` | `103.39M` | `$16.59` | `$13.27` |
| `libgit2/oid` | `4` | `19` | `3h14m42s` | `$27.25` | `66.34M` | `$48.59` | `57.98M` | `$75.84` | `124.32M` | `$18.96` | `$3.99` |
| `libgit2/core` | `16` | `12` | `5h06m51s` | `$96.92` | `235.87M` | `$70.82` | `85.91M` | `$167.74` | `321.78M` | `$10.48` | `$13.98` |
| `libgit2/repository` | `23` | `75` | `8h27m28s` | `$227.32` | `689.05M` | `$123.41` | `177.79M` | `$350.73` | `866.84M` | `$15.25` | `$4.68` |
| `libgit2/object` | `21` | `152` | `12h10m54s` | `$279.41` | `887.00M` | `$232.10` | `320.59M` | `$511.51` | `1207.59M` | `$24.36` | `$3.37` |
| `libgit2/config` | `9` | `47` | `6h09m05s` | `$36.16` | `129.17M` | `$94.07` | `265.11M` | `$130.23` | `394.28M` | `$14.47` | `$2.77` |
| `libgit2/index` | `18` | `75` | `10h41m03s` | `$195.65` | `307.26M` | `$67.64` | `186.28M` | `$263.29` | `493.54M` | `$14.63` | `$3.51` |
| orchestrator | `<n>` | `<n>` | — | — | — | — | — | `$<n>`+ | `<n>M`+ | — | — |
| **Σ recorded agents** | **`95`** | **`389`** | **`51h01m`** | **`$896.34`** | **`2391.61M`** | **`$701.91`** | **`1175.51M`** | **`$1598.25`** | **`3567.12M`** | **`$16.82`** | **`$4.11`** |

## Raw lifetime discovery

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| tier | strategies emitted | $ | tokens | wall |
|---|---|---|---|---|
| void | `2` | `$8.28` | `17.79M` | `51m29s` |
| string | `2` | `$5.49` | `12.39M` | `23m12s` |
| **Σ** | **`4`** | **`$13.77`** | **`30.18M`** | **`74m41s`** |

### Review

`openrouter/anthropic/claude-fable-5.1` via `claude`.

| tier | rv loc | rv $ | rv tokens | rv wall |
|---|---|---|---|---|
| void | `+80/-1` | `$7.89` | `10.78M` | `18m36s` |
| string | `+345/-23` | `$10.89` | `14.41M` | `22m03s` |
| **Σ** | **`+425/-24`** | **`$18.78`** | **`25.19M`** | **`40m39s`** |

## Types

### Batches — wrap

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

| subsystem | wave | batch | types | fields | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`| `2` | `5` |`+1934/-2`| `1.30`% / `95.76`% | `+1` | `+1` | `$7.55` | `19.09M` | `33m43s` | `$3.77` | `$1.51` |
|`libgit2/util`|`0`|`1`| `2` | `0` |`+849/-26`| `1.35`% / `96.22`% | `+0` | `+0` | `$5.25` | `12.24M` | `38m51s` | `$2.62` | — |
|`libgit2/oid`|`0`|`0`| `2` | `2` |`+1514/-4`| `1.73`% / `96.25`% | `+0` | `+0` | `$9.89` | `28.93M` | `49m08s` | `$4.95` | `$4.95` |
|`libgit2/oid`|`0`|`1`| `1` | `0` |`+488/-0`| `1.68`% / `95.42`% | `+0` | `+0` | `$2.87` | `6.58M` | `14m31s` | `$2.87` | — |
|`libgit2/oid`|`1`|`0`| `1` | `2` |`+1397/-0`| `5.89`% / `96.82`% | `+0` | `+0` | `$5.30` | `12.40M` | `19m02s` | `$5.30` | `$2.65` |
|`libgit2/core`|`0`|`0`| `2` | `3` |`+2495/-315`| `15.24`% / `98.02`% | `+0` | `+1` | `$14.63` | `28.15M` | `54m01s` | `$7.32` | `$4.88` |
|`libgit2/core`|`0`|`1`| `2` | `0` |`+909/-0`| `9.22`% / `96.78`% | `+0` | `+0` | `$2.54` | `5.29M` | `16m38s` | `$1.27` | — |
|`libgit2/core`|`0`|`2`| `2` | `0` |`+1784/-6`| `12.15`% / `97.29`% | `+0` | `+0` | `$7.38` | `20.11M` | `28m17s` | `$3.69` | — |
|`libgit2/core`|`0`|`3`| `2` | `0` |`+871/-19`| `10.96`% / `97.14`% | `+0` | `+0` | `$5.25` | `13.05M` | `22m58s` | `$2.62` | — |
|`libgit2/core`|`0`|`4`| `2` | `2` |`+2575/-6`| `14.28`% / `97.84`% | `+0` | `+1` | `$12.07` | `30.57M` | `43m43s` | `$6.04` | `$6.04` |
|`libgit2/core`|`0`|`5`| `2` | `0` |`+839/-0`| `9.19`% / `96.51`% | `+0` | `+0` | `$3.05` | `6.68M` | `13m22s` | `$1.52` | — |
|`libgit2/core`|`0`|`6`| `2` | `3` |`+1134/-3`| `9.42`% / `97.01`% | `+0` | `+0` | `$4.22` | `10.43M` | `17m55s` | `$2.11` | `$1.41` |
|`libgit2/core`|`0`|`7`| `1` | `3` |`+2428/-3`| `13.87`% / `97.53`% | `+3` | `+3` | `$10.74` | `30.16M` | `33m33s` | `$10.74` | `$3.58` |
|`libgit2/core`|`1`|`0`| `1` | `3` |`+1745/-3`| `18.22`% / `98.23`% | `+2` | `+3` | `$10.23` | `31.28M` | `26m16s` | `$10.23` | `$3.41` |
|`libgit2/repository`|`0`|`0`| `1` | `0` |`+1799/-0`| `32.72`% / `98.56`% | `+0` | `+0` | `$24.34` | `54.96M` | `1h23m54s` | `$24.34` | — |
|`libgit2/repository`|`0`|`1`| `1` | `0` |`+3376/-0`| `31.33`% / `98.56`% | `+4` | `+8` | `$16.26` | `50.26M` | `55m56s` | `$16.26` | — |
|`libgit2/repository`|`0`|`2`| `2` | `0` |`+1925/-0`| `19.74`% / `98.49`% | `+0` | `+1` | `$8.59` | `24.90M` | `29m29s` | `$4.29` | — |
|`libgit2/repository`|`0`|`3`| `1` | `0` |`+1789/-0`| `25.32`% / `98.59`% | `+0` | `+0` | `$11.26` | `34.18M` | `42m24s` | `$11.26` | — |
|`libgit2/repository`|`0`|`4`| `2` | `0` |`+1635/-1`| `23.97`% / `98.53`% | `+0` | `+0` | `$11.67` | `36.52M` | `35m52s` | `$5.83` | — |
|`libgit2/repository`|`0`|`5`| `2` | `0` |`+1585/-0`| `23.69`% / `98.51`% | `+2` | `+0` | `$10.66` | `33.24M` | `32m15s` | `$5.33` | — |
|`libgit2/repository`|`0`|`6`| `2` | `0` |`+2089/-0`| `24.02`% / `98.57`% | `+0` | `+0` | `$10.91` | `33.42M` | `38m41s` | `$5.46` | — |
|`libgit2/repository`|`0`|`7`| `1` | `0` |`+1491/-194`| `25.76`% / `98.64`% | `-1` | `+0` | `$15.47` | `52.12M` | `51m17s` | `$15.47` | — |
|`libgit2/repository`|`0`|`8`| `2` | `0` |`+1279/-0`| `18.62`% / `98.30`% | `+0` | `+0` | `$4.27` | `9.83M` | `18m41s` | `$2.13` | — |
|`libgit2/repository`|`0`|`9`| `2` | `0` |`+1516/-0`| `19.22`% / `98.43`% | `+0` | `+0` | `$6.34` | `17.05M` | `27m40s` | `$3.17` | — |
|`libgit2/repository`|`0`|`10`| `2` | `0` |`+1587/-1`| `25.67`% / `98.62`% | `+0` | `+0` | `$7.92` | `22.15M` | `45m21s` | `$3.96` | — |
|`libgit2/repository`|`0`|`11`| `2` | `2` |`+1545/-0`| `18.98`% / `98.37`% | `+0` | `+0` | `$5.41` | `12.69M` | `23m56s` | `$2.71` | `$2.71` |
|`libgit2/repository`|`1`|`0`| `2` | `12` |`+2748/-28`| `33.79`% / `98.66`% | `+0` | `+0` | `$10.14` | `30.40M` | `28m55s` | `$5.07` | `$0.85` |
|`libgit2/repository`|`1`|`1`| `1` | `5` |`+1563/-3`| `33.24`% / `98.60`% | `+0` | `+0` | `$6.36` | `17.21M` | `20m02s` | `$6.36` | `$1.27` |
|`libgit2/object`|`0`|`0`| `2` | `2` |`+1681/-3`| `36.70`% / `98.49`% | `+0` | `+0` | `$6.21` | `16.56M` | `31m12s` | `$3.10` | `$3.10` |
|`libgit2/object`|`0`|`1`| `2` | `0` |`+2706/-1`| `37.91`% / `98.57`% | `+2` | `+0` | `$17.37` | `56.17M` | `54m52s` | `$8.69` | — |
|`libgit2/object`|`0`|`2`| `2` | `2` |`+2491/-0`| `37.67`% / `98.57`% | `+0` | `+0` | `$10.57` | `25.58M` | `47m19s` | `$5.29` | `$5.29` |
|`libgit2/object`|`0`|`3`| `2` | `0` |`+2509/-3`| `36.94`% / `98.52`% | `+0` | `+0` | `$14.40` | `45.71M` | `42m11s` | `$7.20` | — |
|`libgit2/object`|`0`|`4`| `2` | `0` |`+1791/-0`| `36.85`% / `98.49`% | `+1` | `+0` | `$8.08` | `24.57M` | `32m20s` | `$4.04` | — |
|`libgit2/object`|`0`|`5`| `1` | `0` |`+620/-0`| `36.19`% / `98.48`% | `+0` | `+0` | `$2.46` | `5.55M` | `13m29s` | `$2.46` | — |
|`libgit2/object`|`1`|`0`| `2` | `4` |`+2198/-10`| `38.08`% / `98.63`% | `+0` | `+0` | `$8.50` | `24.63M` | `27m01s` | `$4.25` | `$2.12` |
|`libgit2/object`|`1`|`1`| `2` | `7` |`+3634/-350`| `38.27`% / `98.65`% | `+1` | `+0` | `$15.08` | `46.58M` | `38m26s` | `$7.54` | `$2.15` |
|`libgit2/object`|`1`|`2`| `2` | `7` |`+2338/-49`| `38.16`% / `98.65`% | `+0` | `+0` | `$10.87` | `30.98M` | `36m44s` | `$5.43` | `$1.55` |
|`libgit2/object`|`1`|`3`| `2` | `0` |`+3188/-330`| `38.25`% / `98.66`% | `-1` | `-2` | `$17.37` | `57.50M` | `42m30s` | `$8.69` | — |
|`libgit2/object`|`1`|`4`| `1` | `4` |`+1169/-9`| `38.05`% / `98.60`% | `+0` | `+0` | `$4.42` | `11.16M` | `17m24s` | `$4.42` | `$1.10` |
|`libgit2/object`|`2`|`0`| `1` | `9` |`+1784/-9`| `39.72`% / `98.75`% | `+0` | `+0` | `$6.63` | `17.54M` | `28m41s` | `$6.63` | `$0.74` |
|`libgit2/config`|`0`|`0`| `2` | `3` |`+536/-0`| `42.24`% / `98.75`% | `+0` | `+0` | `$1.54` | `5.82M` | `11m22s` | `$0.77` | `$0.51` |
|`libgit2/config`|`0`|`1`| `2` | `0` |`+743/-1`| `42.52`% / `98.74`% | `+0` | `+0` | `$2.54` | `10.00M` | `24m29s` | `$1.27` | — |
|`libgit2/config`|`1`|`0`| `2` | `9` |`+772/-2`| `42.53`% / `98.77`% | `+0` | `+0` | `$1.86` | `7.12M` | `14m33s` | `$0.93` | `$0.21` |
|`libgit2/config`|`2`|`0`| `1` | `2` |`+487/-1`| `42.74`% / `98.75`% | `+0` | `+0` | `$1.67` | `5.71M` | `16m30s` | `$1.67` | `$0.83` |
|`libgit2/config`|`3`|`0`| `1` | `4` |`+1131/-24`| `42.79`% / `98.76`% | `+2` | `+1` | `$1.95` | `6.79M` | `19m01s` | `$1.95` | `$0.49` |
|`libgit2/config`|`4`|`0`| `1` | `14` |`+1118/-35`| `43.11`% / `98.77`% | `+0` | `+0` | `$2.49` | `9.45M` | `24m50s` | `$2.49` | `$0.18` |
|`libgit2/config`|`4`|`0`| `1` | `14` |`+2746/-5`| `42.84`% / `98.75`% | `+0` | `+0` | `$11.63` | `34.73M` | `39m04s` | `$11.63` | `$0.83` |
|`libgit2/index`|`0`|`0`| `2` | `0` |`+1340/-43`| `45.00`% / `98.80`% | `+2` | `+1` | `$20.29` | `12.38M` | `27m57s` | `$10.14` | — |
|`libgit2/index`|`0`|`1`| `2` | `0` |`+431/-0`| `42.82`% / `98.77`% | `+0` | `+0` | `$5.76` | `2.63M` | `7m04s` | `$2.88` | — |
|`libgit2/index`|`0`|`2`| `2` | `0` |`+522/-0`| `44.19`% / `98.79`% | `+0` | `+0` | `$16.64` | `10.75M` | `19m17s` | `$8.32` | — |
|`libgit2/index`|`0`|`3`| `2` | `0` |`+402/-5`| `44.19`% / `98.79`% | `+0` | `+0` | `$13.75` | `8.80M` | `14m41s` | `$6.88` | — |
|`libgit2/index`|`0`|`4`| `2` | `3` |`+543/-0`| `44.14`% / `98.79`% | `+0` | `+0` | `$9.70` | `5.50M` | `12m25s` | `$4.85` | `$3.23` |
|`libgit2/index`|`0`|`5`| `2` | `2` |`+914/-1`| `45.58`% / `98.81`% | `+0` | `+0` | `$12.84` | `19.71M` | `36m20s` | `$6.42` | `$6.42` |
|`libgit2/index`|`0`|`6`| `2` | `0` |`+889/-520`| `45.46`% / `98.80`% | `-4` | `-4` | `$16.15` | `16.46M` | `36m16s` | `$8.07` | — |
|`libgit2/index`|`0`|`7`| `1` | `0` |`+1226/-1`| `45.43`% / `98.80`% | `+5` | `+5` | `$21.81` | `13.93M` | `40m51s` | `$21.81` | — |
|`libgit2/index`|`1`|`0`| `2` | `14` |`+1543/-14`| `45.80`% / `98.83`% | `+0` | `+0` | `$5.84` | `14.76M` | `23m16s` | `$2.92` | `$0.42` |
|`libgit2/index`|`1`|`1`| `1` | `3` |`+806/-5`| `45.85`% / `98.84`% | `+0` | `+0` | `$6.21` | `17.23M` | `25m45s` | `$6.21` | `$2.07` |
| **Σ** |  |  | **`96`** | **`145`** | **`+89147/-2035`** | **`45.85`% / `98.84`%** | **`+19`** | **`+19`** | **`$525.20`** | **`1248.21M`** | — | **`$5.47`** | **`$3.62`** |

### Batches — wrap: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`|`+4` (`+0.02`/`-6.50` pp)|`+7` (`+0.00`/`-7.18` pp)|`+2` (`+0.00`/`-8.45` pp)|`+14` (`+0.04`/`+3.56` pp)|
|`libgit2/util`|`0`|`1`|`+4` (`+0.00`/`-1.55` pp)|`+0` (`+0.00`/`-1.95` pp)|`+6` (`+0.05`/`-1.12` pp)|`+10` (`+0.00`/`+0.87` pp)|
|`libgit2/oid`|`0`|`0`|`+11` (`+0.09`/`+0.88` pp)|`+0` (`+0.00`/`-2.63` pp)|`+6` (`+0.04`/`-0.37` pp)|`+11` (`+0.09`/`+1.19` pp)|
|`libgit2/oid`|`0`|`1`|`+4` (`+0.00`/`+0.87` pp)|`+0` (`+0.00`/`-0.69` pp)|`+7` (`+0.06`/`+1.05` pp)|`+5` (`+0.00`/`+0.33` pp)|
|`libgit2/oid`|`1`|`0`|`+8` (`+0.01`/`-0.01` pp)|`+0` (`+0.00`/`-1.62` pp)|`+2` (`+4.15`/`-0.16` pp)|`+8` (`+0.00`/`+0.83` pp)|
|`libgit2/core`|`0`|`0`|`+11` (`+0.12`/`+0.47` pp)|`+5` (`+0.00`/`+1.43` pp)|`+4` (`+0.96`/`+0.09` pp)|`+12` (`+0.12`/`+0.58` pp)|
|`libgit2/core`|`0`|`1`|`+5` (`+0.00`/`+0.84` pp)|`+0` (`+0.00`/`-0.57` pp)|`+5` (`+0.03`/`+1.05` pp)|`+7` (`+0.00`/`+0.47` pp)|
|`libgit2/core`|`0`|`2`|`+7` (`+0.00`/`+1.20` pp)|`+0` (`+0.00`/`-0.30` pp)|`+10` (`+1.19`/`+1.27` pp)|`+9` (`+0.00`/`+0.25` pp)|
|`libgit2/core`|`0`|`3`|`+5` (`+0.00`/`+0.10` pp)|`+0` (`+0.00`/`-0.27` pp)|`+4` (`+1.54`/`-0.19` pp)|`+8` (`+0.00`/`+0.22` pp)|
|`libgit2/core`|`0`|`4`|`+13` (`+0.12`/`-0.48` pp)|`+0` (`+0.00`/`-0.68` pp)|`+6` (`+0.04`/`-0.98` pp)|`+18` (`+0.52`/`+0.55` pp)|
|`libgit2/core`|`0`|`5`|`+4` (`+0.00`/`+0.60` pp)|`+0` (`+0.00`/`-0.25` pp)|`+7` (`+1.39`/`+0.86` pp)|`+3` (`+0.00`/`+0.21` pp)|
|`libgit2/core`|`0`|`6`|`+10` (`+0.00`/`+0.81` pp)|`+0` (`+0.00`/`-0.48` pp)|`+9` (`+0.20`/`+0.36` pp)|`+13` (`+0.00`/`+0.40` pp)|
|`libgit2/core`|`0`|`7`|`+18` (`+7.17`/`+0.83` pp)|`+0` (`+0.00`/`-0.61` pp)|`+4` (`+1.70`/`+0.00` pp)|`+14` (`+0.04`/`+0.46` pp)|
|`libgit2/core`|`1`|`0`|`+11` (`+0.21`/`+0.48` pp)|`+0` (`+0.00`/`-0.76` pp)|`+6` (`+0.02`/`+0.08` pp)|`+8` (`+0.18`/`+0.15` pp)|
|`libgit2/repository`|`0`|`0`|`+7` (`+2.18`/`+0.23` pp)|`+0` (`+0.03`/`-0.28` pp)|`+2` (`+1.08`/`+0.20` pp)|`+5` (`+0.42`/`-0.02` pp)|
|`libgit2/repository`|`0`|`1`|`+8` (`+3.49`/`+0.49` pp)|`+12` (`+6.85`/`+0.34` pp)|`+4` (`+5.84`/`+0.62` pp)|`+15` (`+0.00`/`+0.18` pp)|
|`libgit2/repository`|`0`|`2`|`+9` (`+2.35`/`-0.61` pp)|`+3` (`+5.08`/`+0.10` pp)|`+5` (`+0.52`/`-0.69` pp)|`+11` (`+2.64`/`+0.35` pp)|
|`libgit2/repository`|`0`|`3`|`+15` (`+1.38`/`-0.43` pp)|`+0` (`+0.01`/`-0.28` pp)|`+7` (`+1.28`/`-0.53` pp)|`+6` (`+1.17`/`+0.14` pp)|
|`libgit2/repository`|`0`|`4`|`+8` (`+0.37`/`-0.27` pp)|`+0` (`+0.01`/`-0.38` pp)|`+9` (`+0.27`/`-0.25` pp)|`+10` (`+0.06`/`+0.16` pp)|
|`libgit2/repository`|`0`|`5`|`+7` (`+0.03`/`-0.23` pp)|`+4` (`+3.44`/`+0.12` pp)|`+5` (`+3.92`/`-0.25` pp)|`+10` (`+3.50`/`+0.17` pp)|
|`libgit2/repository`|`0`|`6`|`+14` (`+0.19`/`-0.54` pp)|`+0` (`-0.01`/`-0.50` pp)|`+5` (`+0.06`/`-0.85` pp)|`+12` (`+0.07`/`+0.28` pp)|
|`libgit2/repository`|`0`|`7`|`+7` (`+0.34`/`-0.27` pp)|`+0` (`+0.02`/`-0.35` pp)|`+3` (`+0.07`/`-0.31` pp)|`+10` (`+0.10`/`+0.16` pp)|
|`libgit2/repository`|`0`|`8`|`+5` (`-0.01`/`+0.43` pp)|`+0` (`+0.01`/`-0.38` pp)|`+6` (`+0.39`/`+0.68` pp)|`+6` (`+0.00`/`+0.13` pp)|
|`libgit2/repository`|`0`|`9`|`+6` (`+0.01`/`+0.18` pp)|`+0` (`-0.01`/`-0.38` pp)|`+9` (`+0.24`/`+0.35` pp)|`+8` (`+0.00`/`+0.25` pp)|
|`libgit2/repository`|`0`|`10`|`+10` (`+0.32`/`-0.20` pp)|`+0` (`+0.00`/`-0.19` pp)|`+4` (`+0.31`/`-0.26` pp)|`+11` (`+0.30`/`+0.12` pp)|
|`libgit2/repository`|`0`|`11`|`+13` (`+0.01`/`+0.96` pp)|`+0` (`+0.00`/`-0.49` pp)|`+5` (`+0.37`/`+0.04` pp)|`+9` (`+0.01`/`+0.06` pp)|
|`libgit2/repository`|`1`|`0`|`+24` (`+0.94`/`+0.97` pp)|`+0` (`+0.01`/`-0.39` pp)|`+6` (`+0.55`/`+0.24` pp)|`+12` (`+0.02`/`+0.13` pp)|
|`libgit2/repository`|`1`|`1`|`+10` (`+0.88`/`+0.14` pp)|`+0` (`+0.23`/`-0.22` pp)|`+3` (`+0.31`/`+0.13` pp)|`+7` (`+0.00`/`+0.12` pp)|
|`libgit2/object`|`0`|`0`|`+9` (`+0.02`/`+0.37` pp)|`+0` (`+0.01`/`-0.18` pp)|`+5` (`+0.43`/`+0.01` pp)|`+9` (`+0.01`/`+0.16` pp)|
|`libgit2/object`|`0`|`1`|`+7` (`+0.46`/`-0.07` pp)|`+6` (`+0.33`/`+0.54` pp)|`+6` (`+0.19`/`+0.04` pp)|`+8` (`+1.01`/`-0.05` pp)|
|`libgit2/object`|`0`|`2`|`+9` (`+0.58`/`-0.20` pp)|`+2` (`+0.56`/`-0.05` pp)|`+7` (`+0.71`/`-0.21` pp)|`+20` (`+0.62`/`+0.26` pp)|
|`libgit2/object`|`0`|`3`|`+15` (`+0.07`/`-0.17` pp)|`+0` (`-0.26`/`-0.23` pp)|`+4` (`+0.28`/`-0.28` pp)|`+10` (`+0.38`/`+0.16` pp)|
|`libgit2/object`|`0`|`4`|`+6` (`+0.06`/`-0.11` pp)|`+3` (`+0.22`/`+0.39` pp)|`+4` (`+0.13`/`-0.02` pp)|`+4` (`+0.14`/`-0.14` pp)|
|`libgit2/object`|`0`|`5`|`+3` (`+0.00`/`+0.04` pp)|`+0` (`-0.05`/`-0.02` pp)|`+3` (`+0.04`/`+0.04` pp)|`+3` (`-0.01`/`+0.02` pp)|
|`libgit2/object`|`1`|`0`|`+10` (`+0.39`/`+0.15` pp)|`+0` (`+0.10`/`-0.16` pp)|`+6` (`+0.01`/`+0.08` pp)|`+8` (`+0.01`/`+0.13` pp)|
|`libgit2/object`|`1`|`1`|`+9` (`+0.47`/`+0.29` pp)|`+2` (`+0.22`/`+0.43` pp)|`+5` (`+0.10`/`+0.31` pp)|`+9` (`+0.03`/`+0.15` pp)|
|`libgit2/object`|`1`|`2`|`+7` (`+0.10`/`+0.27` pp)|`+4` (`-0.04`/`+0.29` pp)|`+5` (`+0.09`/`+0.18` pp)|`+9` (`+0.02`/`+0.12` pp)|
|`libgit2/object`|`1`|`3`|`+22` (`+0.19`/`+0.12` pp)|`+0` (`-0.34`/`-0.22` pp)|`+6` (`+0.04`/`-0.03` pp)|`+12` (`+0.05`/`+0.13` pp)|
|`libgit2/object`|`1`|`4`|`+5` (`-0.02`/`+0.28` pp)|`+0` (`-0.02`/`-0.13` pp)|`+1` (`+0.09`/`+0.02` pp)|`+6` (`+0.02`/`+0.11` pp)|
|`libgit2/object`|`2`|`0`|`+6` (`+0.28`/`+0.07` pp)|`+0` (`+0.27`/`-0.11` pp)|`+2` (`+0.22`/`-0.02` pp)|`+4` (`+0.27`/`+0.08` pp)|
|`libgit2/config`|`0`|`0`|`+2` (`+0.02`/`+0.10` pp)|`+0` (`-0.07`/`-0.03` pp)|`+2` (`-0.03`/`+0.09` pp)|`+3` (`-0.01`/`-0.03` pp)|
|`libgit2/config`|`0`|`1`|`+3` (`+0.09`/`+0.09` pp)|`+0` (`+0.38`/`-0.03` pp)|`+3` (`+0.25`/`+0.01` pp)|`+4` (`+0.14`/`-0.01` pp)|
|`libgit2/config`|`1`|`0`|`+2` (`+0.04`/`+0.23` pp)|`+0` (`+0.31`/`-0.03` pp)|`+2` (`+0.07`/`+0.22` pp)|`+2` (`+0.00`/`+0.00` pp)|
|`libgit2/config`|`2`|`0`|`+1` (`-0.04`/`-0.03` pp)|`+0` (`-0.49`/`-0.02` pp)|`+1` (`-0.02`/`-0.02` pp)|`+2` (`+0.01`/`+0.02` pp)|
|`libgit2/config`|`3`|`0`|`+4` (`-0.01`/`+0.25` pp)|`+1` (`+0.03`/`+0.21` pp)|`+1` (`+0.01`/`+0.19` pp)|`+3` (`+0.02`/`-0.03` pp)|
|`libgit2/config`|`4`|`0`|`+2` (`+0.32`/`+0.09` pp)|`+0` (`-0.55`/`-0.03` pp)|`+3` (`+0.25`/`+0.09` pp)|`+5` (`+0.14`/`-0.09` pp)|
|`libgit2/config`|`4`|`0`|`+9` (`+0.03`/`+0.90` pp)|`+0` (`+0.22`/`-0.13` pp)|`+3` (`+0.01`/`+0.43` pp)|`+9` (`+0.00`/`+0.06` pp)|
|`libgit2/index`|`0`|`0`|`+4` (`+0.82`/`+0.25` pp)|`+1` (`+0.35`/`+0.89` pp)|`+3` (`+0.78`/`+0.36` pp)|`+3` (`+0.16`/`-0.44` pp)|
|`libgit2/index`|`0`|`1`|`+2` (`-0.03`/`+0.11` pp)|`+0` (`+0.01`/`-0.01` pp)|`+3` (`+0.02`/`+0.05` pp)|`+2` (`-0.01`/`-0.12` pp)|
|`libgit2/index`|`0`|`2`|`+2` (`-0.01`/`-0.04` pp)|`+0` (`+0.19`/`-0.02` pp)|`+2` (`-0.02`/`-0.02` pp)|`+3` (`+0.04`/`+0.02` pp)|
|`libgit2/index`|`0`|`3`|`+2` (`+0.05`/`-0.04` pp)|`+0` (`-0.21`/`-0.02` pp)|`+2` (`+0.05`/`-0.03` pp)|`+2` (`+0.02`/`+0.02` pp)|
|`libgit2/index`|`0`|`4`|`+3` (`+0.05`/`+0.05` pp)|`+0` (`+0.02`/`-0.02` pp)|`+2` (`+0.08`/`+0.06` pp)|`+4` (`+0.06`/`+0.00` pp)|
|`libgit2/index`|`0`|`5`|`+5` (`+0.02`/`-0.03` pp)|`+0` (`-0.11`/`-0.05` pp)|`+3` (`+0.10`/`-0.02` pp)|`+5` (`+0.03`/`+0.05` pp)|
|`libgit2/index`|`0`|`6`|`+6` (`+0.19`/`+0.06` pp)|`+0` (`-1.22`/`-0.45` pp)|`+2` (`+0.06`/`-0.10` pp)|`+4` (`+0.03`/`-0.01` pp)|
|`libgit2/index`|`0`|`7`|`+2` (`+0.35`/`-0.02` pp)|`+5` (`+1.23`/`+0.39` pp)|`+3` (`+0.45`/`+0.16` pp)|`+2` (`+0.13`/`-0.15` pp)|
|`libgit2/index`|`1`|`0`|`+6` (`+0.02`/`+0.35` pp)|`+0` (`+0.18`/`-0.07` pp)|`+2` (`+0.02`/`-0.06` pp)|`+4` (`+0.00`/`-0.10` pp)|
|`libgit2/index`|`1`|`1`|`+4` (`+0.12`/`+0.00` pp)|`+0` (`-0.16`/`-0.02` pp)|`+2` (`+0.13`/`+0.02` pp)|`+2` (`+0.02`/`-0.01` pp)|
| **Σ** |  |  | **`+425` (`+24.85`/`+2.75` pp)** | **`+55` (`+16.78`/`-18.73` pp)** | **`+244` (`+31.12`/`-5.84` pp)** | **`+433` (`+12.61`/`+12.61` pp)** |

### Batches — port

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | types | fields | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | tokens | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`<link-unit>/<subsystem>`|`<n>`|`<n>`| `<n>` | `<n>` |`+<n>/-<n>`| `<n>`% / `<n>`% | `±<n>` | `±<n>` | `$<n>` | `<n>M` | `<n>m<n>s` | `$<n>` | `$<n>` |
| **Σ** |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`±<n>`** | **`±<n>`** | **`$<n>`** | **`<n>M`** | — | **`$<n>`** | **`$<n>`** |

### Batches — port: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`<link-unit>/<subsystem>`|`<n>`|`<n>`|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|
| **Σ** |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |

### Batches — review

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | types | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | rv loc | rv $ | rv tokens | rv wall | rv $/type |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`| `2` |`+172/-15`| `1.63`% / `95.18`% | `+0` | `+0` | `+172/-15` | `$20.66` | `23.31M` | `48m16s` | `$10.33` |
|`libgit2/util`|`0`|`1`| `2` |`+149/-12`| `1.63`% / `96.51`% | `+0` | `+0` | `+149/-12` | `$7.58` | `10.10M` | `18m18s` | `$3.79` |
|`libgit2/oid`|`0`|`0`| `2` |`+224/-6`| `7.81`% / `96.25`% | `+0` | `+0` | `+224/-6` | `$14.93` | `18.05M` | `29m10s` | `$7.46` |
|`libgit2/oid`|`0`|`1`| `1` |`+185/-8`| `7.81`% / `96.12`% | `+0` | `+0` | `+185/-8` | `$7.16` | `8.57M` | `19m44s` | `$7.16` |
|`libgit2/oid`|`1`|`0`| `1` |`+235/-18`| `7.80`% / `96.36`% | `+0` | `+0` | `+235/-18` | `$11.49` | `15.40M` | `22m41s` | `$11.49` |
|`libgit2/core`|`0`|`0`| `15` |`+263/-9`| `15.84`% / `98.11`% | `+0` | `+0` | `+263/-9` | `$24.78` | `30.19M` | `43m19s` | `$1.65` |
|`libgit2/core`|`1`|`0`| `1` |`+126/-7`| `18.23`% / `98.26`% | `+0` | `+0` | `+126/-7` | `$18.99` | `21.31M` | `39m44s` | `$18.99` |
|`libgit2/repository`|`0`|`0`| `20` |`+743/-98`| `32.73`% / `98.58`% | `+0` | `+0` | `+743/-98` | `$32.90` | `55.40M` | `46m45s` | `$1.65` |
|`libgit2/repository`|`1`|`0`| `3` |`+227/-11`| `36.05`% / `98.53`% | `+0` | `+0` | `+227/-11` | `$21.87` | `26.76M` | `41m52s` | `$7.29` |
|`libgit2/object`|`0`|`0`| `11` |`+1767/-853`| `37.98`% / `98.58`% | `-3` | `+0` | `+1767/-853` | `$128.50` | `105.82M` | `3h16m09s` | `$11.68` |
|`libgit2/object`|`1`|`0`| `9` |`+789/-37`| `39.49`% / `98.75`% | `+0` | `+0` | `+789/-37` | `$12.34` | `40.09M` | `30m23s` | `$1.37` |
|`libgit2/object`|`2`|`0`| `1` |`+179/-26`| `42.18`% / `98.74`% | `+0` | `+0` | `+179/-26` | `$4.94` | `13.02M` | `15m40s` | `$4.94` |
|`libgit2/config`|`0`|`0`| `4` |`+646/-125`| `42.52`% / `98.76`% | `+0` | `+0` | `+646/-125` | `$7.75` | `21.91M` | `42m25s` | `$1.94` |
|`libgit2/config`|`1`|`0`| `2` |`+796/-17`| `42.72`% / `98.74`% | `+0` | `+0` | `+796/-17` | `$14.18` | `43.14M` | `49m58s` | `$7.09` |
|`libgit2/config`|`2`|`0`| `1` |`+366/-151`| `42.75`% / `98.76`% | `+0` | `+0` | `+366/-151` | `$6.29` | `17.40M` | `25m11s` | `$6.29` |
|`libgit2/config`|`3`|`0`| `1` |`+627/-239`| `42.82`% / `98.76`% | `-1` | `-1` | `+627/-239` | `$8.39` | `24.83M` | `28m21s` | `$8.39` |
|`libgit2/config`|`4`|`0`| `1` |`+631/-286`| `43.14`% / `98.78`% | `+0` | `+0` | `+631/-286` | `$8.66` | `25.44M` | `27m14s` | `$8.66` |
|`libgit2/config`|`4`|`0`| `1` |`+198/-26`| `42.79`% / `98.77`% | `+0` | `+0` | `+198/-26` | `$8.96` | `25.80M` | `26m55s` | `$8.96` |
|`libgit2/index`|`0`|`0`| `15` |`+738/-79`| `45.75`% / `98.82`% | `-2` | `-2` | `+738/-79` | `$15.85` | `50.35M` | `46m20s` | `$1.06` |
|`libgit2/index`|`1`|`0`| `3` |`+203/-8`| `46.57`% / `98.83`% | `+0` | `+0` | `+203/-8` | `$12.82` | `35.96M` | `48m41s` | `$4.27` |
| **Σ** |  |  | **`96`** | **`+9264/-2031`** | **`46.57`% / `98.83`%** | **`-6`** | **`-3`** | **`+9264/-2031`** | **`$389.04`** | **`612.85M`** | — | **`$4.05`** |

### Batches — review: tests

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`|`+0` (`+0.00`/`-0.99` pp)|`+1` (`+0.00`/`-0.64` pp)|`+0` (`+0.00`/`-1.02` pp)|`+2` (`+0.00`/`-0.77` pp)|
|`libgit2/util`|`0`|`1`|`+0` (`+0.00`/`-0.26` pp)|`+0` (`+0.00`/`-0.32` pp)|`+1` (`+0.27`/`-0.28` pp)|`+1` (`+0.00`/`+0.14` pp)|
|`libgit2/oid`|`0`|`0`|`+1` (`+0.00`/`-0.39` pp)|`+0` (`+0.00`/`-0.28` pp)|`+1` (`+0.00`/`-0.66` pp)|`+4` (`+0.00`/`+0.36` pp)|
|`libgit2/oid`|`0`|`1`|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+1` (`+1.74`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/oid`|`1`|`0`|`+2` (`+0.00`/`+0.49` pp)|`+0` (`+0.00`/`-0.23` pp)|`+0` (`+0.00`/`-0.16` pp)|`+2` (`+4.32`/`+0.20` pp)|
|`libgit2/core`|`0`|`0`|`+0` (`+0.01`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+2` (`+0.32`/`+0.05` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/core`|`1`|`0`|`+1` (`-0.02`/`-0.26` pp)|`+0` (`+0.00`/`-0.11` pp)|`+0` (`-0.01`/`-0.26` pp)|`+1` (`+0.01`/`+0.09` pp)|
|`libgit2/repository`|`0`|`0`|`+2` (`+0.24`/`+0.11` pp)|`+0` (`-0.32`/`-0.11` pp)|`+3` (`+0.02`/`+0.12` pp)|`+2` (`+0.00`/`+0.06` pp)|
|`libgit2/repository`|`1`|`0`|`+0` (`-0.01`/`+0.00` pp)|`+0` (`-0.28`/`+0.00` pp)|`+1` (`+0.01`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/object`|`0`|`0`|`+15` (`+0.09`/`+0.31` pp)|`+0` (`-0.35`/`-1.27` pp)|`+2` (`+0.06`/`+0.13` pp)|`+2` (`+0.07`/`-0.01` pp)|
|`libgit2/object`|`1`|`0`|`+3` (`-0.01`/`+0.02` pp)|`+0` (`-0.51`/`-0.02` pp)|`+1` (`+0.11`/`+0.03` pp)|`+1` (`+0.05`/`+0.04` pp)|
|`libgit2/object`|`2`|`0`|`+0` (`+0.02`/`+0.00` pp)|`+0` (`+0.49`/`+0.00` pp)|`+0` (`+0.05`/`+0.00` pp)|`+0` (`-0.02`/`+0.00` pp)|
|`libgit2/config`|`0`|`0`|`+2` (`+0.00`/`-0.21` pp)|`+0` (`+0.31`/`-0.04` pp)|`+1` (`-0.03`/`-0.15` pp)|`+3` (`+0.01`/`+0.04` pp)|
|`libgit2/config`|`1`|`0`|`+3` (`+0.00`/`-0.12` pp)|`+0` (`-0.49`/`-0.04` pp)|`+2` (`-0.01`/`-0.09` pp)|`+3` (`+0.05`/`+0.08` pp)|
|`libgit2/config`|`2`|`0`|`+1` (`-0.01`/`+0.02` pp)|`+0` (`+0.24`/`+0.00` pp)|`+1` (`+0.02`/`-0.04` pp)|`+2` (`+0.00`/`+0.00` pp)|
|`libgit2/config`|`3`|`0`|`+1` (`-0.01`/`-0.10` pp)|`+0` (`+0.06`/`-0.04` pp)|`+2` (`+0.01`/`-0.10` pp)|`+4` (`+0.00`/`+0.08` pp)|
|`libgit2/config`|`4`|`0`|`+1` (`+0.02`/`-0.04` pp)|`+0` (`+0.37`/`-0.02` pp)|`+2` (`+0.02`/`-0.15` pp)|`+7` (`+0.09`/`+0.19` pp)|
|`libgit2/config`|`4`|`0`|`+0` (`-0.02`/`-0.02` pp)|`+0` (`-0.29`/`-0.01` pp)|`+1` (`-0.04`/`-0.03` pp)|`+1` (`-0.02`/`+0.02` pp)|
|`libgit2/index`|`0`|`0`|`+1` (`+0.08`/`-0.09` pp)|`+2` (`+0.33`/`+0.15` pp)|`+1` (`+0.18`/`-0.07` pp)|`+2` (`+0.00`/`+0.04` pp)|
|`libgit2/index`|`1`|`0`|`+1` (`-0.01`/`-0.10` pp)|`+0` (`+0.04`/`-0.01` pp)|`+0` (`-0.01`/`-0.10` pp)|`+2` (`+0.10`/`+0.04` pp)|
| **Σ** |  |  | **`+34` (`+0.37`/`-1.63` pp)** | **`+3` (`-0.40`/`-2.99` pp)** | **`+22` (`+2.71`/`-2.78` pp)** | **`+39` (`+4.66`/`+0.60` pp)** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total | +rejected PoCs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `libgit2/util` | `0` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` | `0` |
| `libgit2/util` | `0` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` | `0` |
| `libgit2/oid` | `0` | `0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `3` | `0` |
| `libgit2/oid` | `0` | `1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` | `0` |
| `libgit2/oid` | `1` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+1` | `3` | `0` |
| `libgit2/core` | `0` | `0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `5` | `0` |
| `libgit2/core` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `2` | `0` |
| `libgit2/repository` | `0` | `0` | `+1` | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `4` | `0` |
| `libgit2/repository` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` | `0` |
| `libgit2/object` | `0` | `0` | `+2` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+4` | `8` | `0` |
| `libgit2/object` | `1` | `0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `8` | `0` |
| `libgit2/object` | `2` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` | `0` |
| `libgit2/config` | `0` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+2` | `+4` | `7` | `0` |
| `libgit2/config` | `1` | `0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+4` | `6` | `0` |
| `libgit2/config` | `2` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+2` | `4` | `0` |
| `libgit2/config` | `3` | `0` | `+0` | `+2` | `+0` | `+0` | `+0` | `+0` | `+2` | `+1` | `5` | `0` |
| `libgit2/config` | `4` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+2` | `+3` | `7` | `0` |
| `libgit2/config` | `4` | `0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `+3` | `4` | `0` |
| `libgit2/index` | `0` | `0` | `+0` | `+2` | `+0` | `+1` | `+0` | `+0` | `+2` | `+2` | `7` | `0` |
| `libgit2/index` | `1` | `0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` | `0` |
| **Σ** |  |  | **`+6`** | **`+9`** | **`+2`** | **`+1`** | **`+3`** | **`+3`** | **`+14`** | **`+49`** | **`87`** | **`0`** |

## Symbols

### Batches

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

| subsystem | wave | batch | objective | symbols | loc | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | tokens | wall | $/symbol | $/loc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`| wrap | `4` | `51` |`+1008/-26`| `1.30`% / `95.11`% | `+0` | `+0` | `$4.00` | `8.72M` | `21m23s` | `$1.00` | `$0.08` |
|`libgit2/util`|`1`|`0`| wrap | `1` | `15` |`+756/-71`| `1.36`% / `96.43`% | `+0` | `+0` | `$3.06` | `6.69M` | `15m41s` | `$3.06` | `$0.20` |
|`libgit2/oid`|`1`|`1`| wrap | `18` | `252` |`+1351/-11`| `6.07`% / `96.12`% | `+0` | `+0` | `$8.55` | `17.50M` | `40m06s` | `$0.48` | `$0.03` |
|`libgit2/oid`|`2`|`0`| wrap | `1` | `3` |`+0/-0`| `6.07`% / `96.12`% | `+0` | `+0` | `$0.64` | `0.93M` | `4m17s` | `$0.64` | `$0.21` |
|`libgit2/core`|`0`|`8`| wrap | `6` | `444` |`+3504/-75`| `15.52`% / `98.11`% | `+37` | `+0` | `$19.90` | `44.40M` | `1h04m11s` | `$3.32` | `$0.04` |
|`libgit2/core`|`1`|`1`| wrap | `5` | `212` |`+1238/-4`| `18.20`% / `98.15`% | `+1` | `+0` | `$4.61` | `11.51M` | `19m37s` | `$0.92` | `$0.02` |
|`libgit2/core`|`2`|`0`| wrap | `1` | `9` |`+658/-41`| `18.24`% / `98.26`% | `+2` | `+0` | `$2.30` | `4.24M` | `10m40s` | `$2.30` | `$0.26` |
|`libgit2/repository`|`1`|`2`| wrap | `25` | `436` |`+3311/-54`| `36.06`% / `98.53`% | `+0` | `+2` | `$24.81` | `84.41M` | `1h01m00s` | `$0.99` | `$0.06` |
|`libgit2/repository`|`1`|`3`| wrap | `25` | `432` |`+3392/-29`| `35.39`% / `98.55`% | `+1` | `+0` | `$20.18` | `66.23M` | `53m42s` | `$0.81` | `$0.05` |
|`libgit2/repository`|`1`|`4`| wrap | `18` | `396` |`+3072/-105`| `34.12`% / `98.60`% | `+0` | `+0` | `$15.55` | `50.84M` | `45m11s` | `$0.86` | `$0.04` |
|`libgit2/repository`|`2`|`0`| wrap | `7` | `373` |`+1449/-382`| `36.17`% / `98.46`% | `+0` | `+0` | `$17.18` | `58.64M` | `32m30s` | `$2.45` | `$0.05` |
|`libgit2/object`|`0`|`6`| wrap | `2` | `28` |`+996/-0`| `36.26`% / `98.45`% | `+0` | `+0` | `$3.49` | `8.11M` | `27m05s` | `$1.75` | `$0.12` |
|`libgit2/object`|`1`|`5`| wrap | `25` | `455` |`+4438/-388`| `39.43`% / `98.78`% | `-1` | `-2` | `$34.78` | `128.15M` | `1h09m40s` | `$1.39` | `$0.08` |
|`libgit2/object`|`1`|`6`| wrap | `25` | `261` |`+3882/-94`| `39.17`% / `98.70`% | `+0` | `+0` | `$26.99` | `97.07M` | `1h03m06s` | `$1.08` | `$0.10` |
|`libgit2/object`|`1`|`7`| wrap | `5` | `51` |`+2334/-111`| `38.42`% / `98.69`% | `+0` | `+0` | `$13.81` | `38.98M` | `48m22s` | `$2.76` | `$0.27` |
|`libgit2/object`|`2`|`1`| wrap | `25` | `268` |`+5204/-170`| `42.10`% / `98.74`% | `+0` | `+0` | `$24.53` | `82.24M` | `1h11m24s` | `$0.98` | `$0.09` |
|`libgit2/object`|`2`|`2`| wrap | `25` | `350` |`+1923/-76`| `39.84`% / `98.76`% | `+0` | `+0` | `$11.68` | `35.74M` | `39m28s` | `$0.47` | `$0.03` |
|`libgit2/object`|`2`|`3`| wrap | `25` | `392` |`+4093/-32`| `40.44`% / `98.79`% | `+0` | `+0` | `$20.74` | `70.47M` | `1h00m04s` | `$0.83` | `$0.05` |
|`libgit2/object`|`2`|`4`| wrap | `14` | `167` |`+1555/-41`| `39.51`% / `98.74`% | `+0` | `+0` | `$8.21` | `23.01M` | `24m37s` | `$0.59` | `$0.05` |
|`libgit2/object`|`3`|`0`| wrap | `6` | `144` |`+1978/-296`| `42.23`% / `98.74`% | `+0` | `+0` | `$13.22` | `40.69M` | `38m33s` | `$2.20` | `$0.09` |
|`libgit2/config`|`0`|`2`| wrap | `3` | `74` |`+405/-0`| `42.28`% / `98.74`% | `+0` | `+0` | `$1.37` | `5.31M` | `8m00s` | `$0.46` | `$0.02` |
|`libgit2/config`|`1`|`1`| wrap | `25` | `265` |`+1429/-4`| `42.71`% / `98.69`% | `+0` | `+0` | `$2.46` | `10.12M` | `21m59s` | `$0.10` | `$0.01` |
|`libgit2/config`|`1`|`2`| wrap | `3` | `78` |`+630/-8`| `42.73`% / `98.70`% | `+0` | `+0` | `$2.40` | `10.91M` | `27m41s` | `$0.80` | `$0.03` |
|`libgit2/config`|`2`|`1`| wrap | `5` | `60` |`+816/-6`| `42.78`% / `98.74`% | `+0` | `+0` | `$1.63` | `6.29M` | `11m14s` | `$0.33` | `$0.03` |
|`libgit2/config`|`3`|`1`| wrap | `3` | `52` |`+707/-9`| `42.76`% / `98.75`% | `+0` | `+0` | `$1.41` | `4.66M` | `9m46s` | `$0.47` | `$0.03` |
|`libgit2/config`|`4`|`1`| wrap | `5` | `91` |`+422/-88`| `42.85`% / `98.76`% | `+0` | `+0` | `$1.17` | `3.84M` | `6m28s` | `$0.23` | `$0.01` |
|`libgit2/config`|`5`|`0`| wrap | `3` | `66` |`+1073/-2`| `43.15`% / `98.80`% | `+0` | `+0` | `$2.04` | `8.40M` | `14m30s` | `$0.68` | `$0.03` |
|`libgit2/index`|`0`|`8`| wrap | `1` | `0` |`+548/-0`| `44.05`% / `98.78`% | `+0` | `+0` | `$8.57` | `4.72M` | `9m53s` | `$8.57` | — |
|`libgit2/index`|`1`|`2`| wrap | `25` | `370` |`+2142/-57`| `46.56`% / `98.83`% | `+0` | `+0` | `$22.30` | `78.75M` | `1h06m15s` | `$0.89` | `$0.06` |
|`libgit2/index`|`1`|`3`| wrap | `25` | `405` |`+2120/-46`| `45.96`% / `98.83`% | `+0` | `+0` | `$15.01` | `42.85M` | `55m51s` | `$0.60` | `$0.04` |
|`libgit2/index`|`1`|`4`| wrap | `5` | `47` |`+342/-22`| `45.76`% / `98.82`% | `+0` | `+0` | `$3.09` | `7.11M` | `12m00s` | `$0.62` | `$0.07` |
|`libgit2/index`|`2`|`0`| wrap | `17` | `383` |`+2323/-79`| `46.66`% / `98.85`% | `+0` | `+0` | `$14.38` | `43.62M` | `43m51s` | `$0.85` | `$0.04` |
|`libgit2/index`|`2`|`1`| wrap | `2` | `0` |`+369/-6`| `46.57`% / `98.84`% | `+0` | `+0` | `$3.31` | `8.07M` | `18m25s` | `$1.66` | — |
| **Σ** |  |  |  | **`385`** | **`6630`** | **`+59468/-2333`** | **`46.66`% / `98.85`%** | **`+40`** | **`+0`** | **`$357.37`** | **`1113.22M`** | — | **`$0.93`** | **`$0.05`** |

### Batches: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`; `openrouter/openai/gpt-6.1-sol` via `codex` (reasoning effort `high`) for `libgit2/config`; `openrouter/openai/gpt-6-astra` via `codex` (reasoning effort `high`) for `libgit2/index` wave 0, whose batches 5 and 6 add an `openrouter/anthropic/claude-opus-5.5` rerun.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`|`+11` (`+0.18`/`-1.10` pp)|`+0` (`+0.00`/`-4.37` pp)|`+4` (`+0.12`/`-1.47` pp)|`+4` (`+0.07`/`-2.41` pp)|
|`libgit2/util`|`1`|`0`|`+6` (`+0.00`/`+2.09` pp)|`+0` (`+0.00`/`-0.60` pp)|`+5` (`+0.00`/`+1.57` pp)|`+3` (`+0.01`/`+0.35` pp)|
|`libgit2/oid`|`1`|`1`|`+8` (`+0.17`/`+1.77` pp)|`+0` (`+0.00`/`-1.12` pp)|`+5` (`+0.18`/`+2.35` pp)|`+7` (`+0.10`/`-2.56` pp)|
|`libgit2/oid`|`2`|`0`|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/core`|`0`|`8`|`+9` (`+0.22`/`+0.18` pp)|`+1` (`+0.63`/`+6.69` pp)|`+17` (`+0.17`/`+3.38` pp)|`+10` (`+0.14`/`-5.01` pp)|
|`libgit2/core`|`1`|`1`|`+6` (`+2.54`/`-0.16` pp)|`+5` (`+3.62`/`+0.27` pp)|`+4` (`+2.38`/`-0.12` pp)|`+6` (`+0.00`/`-0.01` pp)|
|`libgit2/core`|`2`|`0`|`+0` (`+0.02`/`+0.09` pp)|`+5` (`+0.01`/`+0.13` pp)|`+3` (`+0.00`/`+0.09` pp)|`+0` (`+0.00`/`-0.12` pp)|
|`libgit2/repository`|`1`|`2`|`+11` (`+0.83`/`+0.71` pp)|`+0` (`-0.05`/`-0.39` pp)|`+14` (`+0.78`/`+0.77` pp)|`+10` (`+0.87`/`-0.70` pp)|
|`libgit2/repository`|`1`|`3`|`+14` (`+1.28`/`+0.92` pp)|`+2` (`+1.07`/`-0.08` pp)|`+18` (`+1.29`/`+0.69` pp)|`+11` (`+0.14`/`-0.91` pp)|
|`libgit2/repository`|`1`|`4`|`+18` (`+0.63`/`+0.30` pp)|`+0` (`-0.26`/`-0.46` pp)|`+10` (`+0.34`/`+0.36` pp)|`+13` (`+1.37`/`-0.10` pp)|
|`libgit2/repository`|`2`|`0`|`+6` (`-0.20`/`-0.20` pp)|`+0` (`-0.02`/`+0.45` pp)|`+3` (`+0.08`/`-0.19` pp)|`+6` (`+0.16`/`-0.10` pp)|
|`libgit2/object`|`0`|`6`|`+9` (`+0.08`/`-0.38` pp)|`+0` (`+0.02`/`-0.14` pp)|`+9` (`+0.08`/`-0.35` pp)|`+7` (`+0.07`/`+0.06` pp)|
|`libgit2/object`|`1`|`5`|`+23` (`+0.42`/`-0.36` pp)|`+0` (`+0.29`/`-0.66` pp)|`+8` (`+0.25`/`-0.64` pp)|`+20` (`+0.46`/`+0.09` pp)|
|`libgit2/object`|`1`|`6`|`+15` (`+0.53`/`-0.09` pp)|`+0` (`-0.59`/`-0.40` pp)|`+6` (`+0.74`/`-0.44` pp)|`+21` (`+1.51`/`+0.11` pp)|
|`libgit2/object`|`1`|`7`|`+9` (`+0.23`/`-0.08` pp)|`+0` (`+0.32`/`-0.17` pp)|`+5` (`+0.06`/`-0.08` pp)|`+5` (`+0.29`/`+0.14` pp)|
|`libgit2/object`|`2`|`1`|`+18` (`+0.21`/`-0.02` pp)|`+0` (`-0.49`/`-0.40` pp)|`+5` (`+0.14`/`-0.25` pp)|`+16` (`+3.31`/`+0.27` pp)|
|`libgit2/object`|`2`|`2`|`+9` (`+0.12`/`+0.11` pp)|`+0` (`+0.06`/`-0.07` pp)|`+2` (`+0.10`/`+0.16` pp)|`+7` (`+0.08`/`+0.04` pp)|
|`libgit2/object`|`2`|`3`|`+13` (`+0.51`/`-0.02` pp)|`+0` (`+0.12`/`-0.20` pp)|`+15` (`+0.57`/`+0.20` pp)|`+9` (`+0.75`/`-0.14` pp)|
|`libgit2/object`|`2`|`4`|`+9` (`+0.06`/`+0.05` pp)|`+0` (`-0.46`/`-0.05` pp)|`+3` (`+0.03`/`+0.15` pp)|`+2` (`+0.00`/`-0.05` pp)|
|`libgit2/object`|`3`|`0`|`+6` (`-0.01`/`+0.25` pp)|`+0` (`-0.21`/`-0.11` pp)|`+0` (`+0.00`/`+0.34` pp)|`+4` (`+0.32`/`-0.03` pp)|
|`libgit2/config`|`0`|`2`|`+3` (`+0.05`/`+0.08` pp)|`+0` (`-0.46`/`+0.00` pp)|`+1` (`+0.04`/`+0.07` pp)|`+0` (`+0.01`/`-0.07` pp)|
|`libgit2/config`|`1`|`1`|`+6` (`+0.20`/`+0.60` pp)|`+0` (`-0.02`/`-0.06` pp)|`+3` (`+0.15`/`+0.59` pp)|`+2` (`+0.02`/`-0.87` pp)|
|`libgit2/config`|`1`|`2`|`+4` (`+0.01`/`-0.01` pp)|`+0` (`-0.28`/`-0.02` pp)|`+3` (`+0.01`/`+0.00` pp)|`+3` (`-0.01`/`+0.01` pp)|
|`libgit2/config`|`2`|`1`|`+5` (`+0.05`/`+0.09` pp)|`+0` (`+0.02`/`-0.03` pp)|`+3` (`+0.03`/`+0.09` pp)|`+3` (`+0.00`/`-0.02` pp)|
|`libgit2/config`|`3`|`1`|`+4` (`+0.02`/`+0.08` pp)|`+0` (`+0.01`/`-0.02` pp)|`+3` (`+0.02`/`+0.09` pp)|`+2` (`+0.00`/`+0.02` pp)|
|`libgit2/config`|`4`|`1`|`+3` (`+0.03`/`+0.01` pp)|`+0` (`+0.06`/`-0.01` pp)|`+1` (`+0.00`/`+0.02` pp)|`+1` (`+0.00`/`+0.00` pp)|
|`libgit2/config`|`5`|`0`|`+4` (`+0.02`/`-0.10` pp)|`+0` (`-0.31`/`-0.07` pp)|`+2` (`+0.02`/`-0.07` pp)|`+4` (`+0.08`/`+0.08` pp)|
|`libgit2/index`|`0`|`8`|`+3` (`+0.00`/`+0.01` pp)|`+0` (`+0.18`/`-0.03` pp)|`+1` (`+0.09`/`+0.01` pp)|`+3` (`+1.70`/`-0.02` pp)|
|`libgit2/index`|`1`|`2`|`+8` (`+1.45`/`+0.07` pp)|`+0` (`+0.01`/`-0.06` pp)|`+5` (`+1.06`/`+0.14` pp)|`+8` (`+0.12`/`-0.05` pp)|
|`libgit2/index`|`1`|`3`|`+8` (`+0.27`/`+0.19` pp)|`+0` (`+0.04`/`-0.06` pp)|`+10` (`+0.17`/`+0.26` pp)|`+7` (`+0.07`/`-0.11` pp)|
|`libgit2/index`|`1`|`4`|`+3` (`+0.10`/`+0.02` pp)|`+0` (`-0.04`/`+0.00` pp)|`+2` (`+0.02`/`+0.03` pp)|`+0` (`+0.00`/`-0.01` pp)|
|`libgit2/index`|`2`|`0`|`+10` (`+0.19`/`+0.09` pp)|`+0` (`-0.13`/`-0.06` pp)|`+10` (`+0.09`/`+0.20` pp)|`+6` (`+0.14`/`+0.01` pp)|
|`libgit2/index`|`2`|`1`|`+2` (`+0.00`/`-0.02` pp)|`+0` (`+0.03`/`-0.01` pp)|`+3` (`-0.02`/`+0.02` pp)|`+2` (`-0.02`/`+0.01` pp)|
| **Σ** |  |  | **`+263` (`+10.21`/`+5.17` pp)** | **`+13` (`+3.17`/`-2.11` pp)** | **`+183` (`+8.99`/`+7.97` pp)** | **`+202` (`+11.76`/`-12.10` pp)** |

### Batches — review

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | symbols | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | rv loc | rv $ | rv tokens | rv wall | rv $/symbol |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`| `4` |`+281/-117`| `1.63`% / `96.03`% | `+0` | `+0` | `+281/-117` | `$12.00` | `16.87M` | `28m49s` | `$3.00` |
|`libgit2/util`|`1`|`0`| `1` |`+0/-0`| `1.63`% / `95.18`% | `+0` | `+0` | `+0/-0` | `$6.26` | `6.37M` | `23m51s` | `$6.26` |
|`libgit2/oid`|`1`|`1`| `18` |`+181/-10`| `7.80`% / `96.39`% | `+0` | `+0` | `+181/-10` | `$11.62` | `12.73M` | `34m10s` | `$0.65` |
|`libgit2/oid`|`2`|`0`| `1` |`+0/-0`| `7.80`% / `96.39`% | `+0` | `+0` | `+0/-0` | `$3.39` | `3.23M` | `8m46s` | `$3.39` |
|`libgit2/core`|`0`|`1`| `6` |`+345/-20`| `15.51`% / `98.11`% | `+1` | `+0` | `+345/-20` | `$12.53` | `17.80M` | `23m38s` | `$2.09` |
|`libgit2/core`|`1`|`1`| `5` |`+59/-10`| `18.24`% / `98.23`% | `+0` | `+0` | `+59/-10` | `$7.69` | `10.83M` | `17m46s` | `$1.54` |
|`libgit2/core`|`2`|`0`| `1` |`+61/-9`| `18.23`% / `98.26`% | `+0` | `+0` | `+61/-9` | `$6.83` | `5.78M` | `18m10s` | `$6.83` |
|`libgit2/repository`|`1`|`1`| `68` |`+640/-143`| `36.16`% / `98.55`% | `+0` | `+0` | `+640/-143` | `$38.68` | `57.91M` | `1h08m35s` | `$0.57` |
|`libgit2/repository`|`2`|`0`| `7` |`+574/-18`| `36.17`% / `98.48`% | `+0` | `+0` | `+574/-18` | `$29.96` | `37.71M` | `57m28s` | `$4.28` |
|`libgit2/object`|`0`|`1`| `2` |`+105/-6`| `37.89`% / `98.57`% | `+0` | `+0` | `+105/-6` | `$20.94` | `18.07M` | `1h08m26s` | `$10.47` |
|`libgit2/object`|`1`|`1`| `55` |`+305/-110`| `39.40`% / `98.74`% | `+0` | `+0` | `+305/-110` | `$12.26` | `40.91M` | `26m40s` | `$0.22` |
|`libgit2/object`|`2`|`1`| `89` |`+216/-18`| `42.20`% / `98.74`% | `+0` | `+0` | `+216/-18` | `$13.54` | `45.38M` | `27m21s` | `$0.15` |
|`libgit2/object`|`3`|`0`| `6` |`+226/-17`| `42.24`% / `98.75`% | `+0` | `+0` | `+226/-17` | `$39.58` | `57.30M` | `3h04m22s` | `$6.60` |
|`libgit2/config`|`0`|`1`| `3` |`+178/-43`| `42.51`% / `98.76`% | `+0` | `+0` | `+178/-43` | `$2.71` | `5.91M` | `12m53s` | `$0.90` |
|`libgit2/config`|`1`|`1`| `28` |`+625/-242`| `42.75`% / `98.74`% | `+0` | `+0` | `+625/-242` | `$11.84` | `37.34M` | `35m07s` | `$0.42` |
|`libgit2/config`|`2`|`1`| `5` |`+222/-51`| `42.75`% / `98.76`% | `+0` | `+0` | `+222/-51` | `$4.64` | `11.54M` | `16m26s` | `$0.93` |
|`libgit2/config`|`3`|`1`| `3` |`+561/-34`| `42.78`% / `98.76`% | `+0` | `+0` | `+561/-34` | `$6.96` | `18.87M` | `24m40s` | `$2.32` |
|`libgit2/config`|`4`|`1`| `5` |`+575/-147`| `43.14`% / `98.78`% | `+0` | `+0` | `+575/-147` | `$6.30` | `15.93M` | `20m24s` | `$1.26` |
|`libgit2/config`|`5`|`0`| `3` |`+172/-99`| `43.17`% / `98.80`% | `+0` | `+0` | `+172/-99` | `$7.39` | `17.00M` | `47m40s` | `$2.46` |
|`libgit2/index`|`0`|`1`| `1` |`+303/-147`| `45.57`% / `98.82`% | `+0` | `+0` | `+303/-147` | `$6.50` | `18.04M` | `24m32s` | `$6.50` |
|`libgit2/index`|`1`|`1`| `55` |`+442/-9`| `46.58`% / `98.83`% | `+0` | `+0` | `+442/-9` | `$12.37` | `39.58M` | `39m02s` | `$0.22` |
|`libgit2/index`|`2`|`0`| `19` |`+190/-21`| `46.64`% / `98.85`% | `+0` | `+0` | `+190/-21` | `$20.10` | `42.35M` | `1h18m28s` | `$1.06` |
| **Σ** |  |  | **`385`** | **`+6261/-1271`** | **`46.64`% / `98.85`%** | **`+1`** | **`+0`** | **`+6261/-1271`** | **`$294.09`** | **`537.47M`** | — | **`$0.76`** |

### Batches — review: tests

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`|`+1` (`+0.00`/`-1.18` pp)|`+0` (`+0.00`/`-0.60` pp)|`+0` (`+0.00`/`-1.19` pp)|`+1` (`+0.00`/`+0.88` pp)|
|`libgit2/util`|`1`|`0`|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/oid`|`1`|`1`|`+0` (`+0.00`/`-0.15` pp)|`+0` (`+0.00`/`-0.06` pp)|`+1` (`+0.00`/`-0.15` pp)|`+1` (`+0.00`/`+0.12` pp)|
|`libgit2/oid`|`2`|`0`|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/core`|`0`|`1`|`+0` (`-0.01`/`-0.63` pp)|`+0` (`+0.01`/`+0.12` pp)|`+0` (`-0.01`/`-0.41` pp)|`+3` (`+0.02`/`+0.37` pp)|
|`libgit2/core`|`1`|`1`|`+0` (`+0.01`/`+0.00` pp)|`+1` (`+0.00`/`+0.00` pp)|`+0` (`+0.01`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/core`|`2`|`0`|`+0` (`+0.00`/`+0.00` pp)|`+1` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/repository`|`1`|`1`|`+0` (`+0.08`/`-0.23` pp)|`+0` (`-0.04`/`-0.07` pp)|`+1` (`+0.03`/`-0.25` pp)|`+3` (`+0.00`/`+0.08` pp)|
|`libgit2/repository`|`2`|`0`|`+1` (`+0.00`/`-0.12` pp)|`+0` (`-0.06`/`-0.10` pp)|`+4` (`+0.01`/`-0.10` pp)|`+1` (`+2.18`/`+0.18` pp)|
|`libgit2/object`|`0`|`1`|`+0` (`+0.22`/`-0.10` pp)|`+0` (`+0.05`/`-0.03` pp)|`+1` (`+0.00`/`-0.10` pp)|`+3` (`+0.00`/`+0.02` pp)|
|`libgit2/object`|`1`|`1`|`+1` (`+0.01`/`-0.12` pp)|`+0` (`-0.09`/`-0.03` pp)|`+0` (`-0.02`/`-0.05` pp)|`+2` (`-0.03`/`-0.02` pp)|
|`libgit2/object`|`2`|`1`|`+0` (`-0.01`/`-0.01` pp)|`+0` (`-0.01`/`+0.00` pp)|`+3` (`+0.03`/`-0.01` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/object`|`3`|`0`|`+0` (`+0.01`/`-0.07` pp)|`+0` (`-0.29`/`-0.02` pp)|`+0` (`+0.03`/`-0.06` pp)|`+2` (`+0.03`/`+0.16` pp)|
|`libgit2/config`|`0`|`1`|`+0` (`+0.01`/`-0.02` pp)|`+0` (`-0.47`/`-0.01` pp)|`+1` (`+0.00`/`-0.01` pp)|`+2` (`+0.04`/`+0.08` pp)|
|`libgit2/config`|`1`|`1`|`+1` (`+0.00`/`-0.09` pp)|`+0` (`+0.20`/`-0.03` pp)|`+2` (`+0.00`/`-0.06` pp)|`+4` (`+0.07`/`+0.33` pp)|
|`libgit2/config`|`2`|`1`|`+0` (`-0.02`/`-0.05` pp)|`+0` (`+0.01`/`-0.01` pp)|`+1` (`+0.01`/`-0.04` pp)|`+1` (`-0.01`/`+0.08` pp)|
|`libgit2/config`|`3`|`1`|`+3` (`+0.02`/`+0.08` pp)|`+0` (`-0.49`/`+0.00` pp)|`+2` (`+0.00`/`+0.13` pp)|`+0` (`+0.00`/`-0.11` pp)|
|`libgit2/config`|`4`|`1`|`+1` (`-0.01`/`-0.02` pp)|`+0` (`+0.12`/`-0.01` pp)|`+2` (`-0.02`/`-0.01` pp)|`+2` (`-0.01`/`+0.01` pp)|
|`libgit2/config`|`5`|`0`|`+0` (`+0.00`/`+0.03` pp)|`+0` (`+0.19`/`+0.00` pp)|`+0` (`+0.02`/`+0.03` pp)|`+0` (`+0.00`/`+0.00` pp)|
|`libgit2/index`|`0`|`1`|`+1` (`-0.02`/`-0.07` pp)|`+0` (`-0.13`/`-0.01` pp)|`+0` (`-0.01`/`-0.07` pp)|`+2` (`+0.01`/`+0.02` pp)|
|`libgit2/index`|`1`|`1`|`+2` (`+0.04`/`-0.08` pp)|`+0` (`-0.02`/`-0.04` pp)|`+0` (`+0.01`/`-0.01` pp)|`+4` (`+0.16`/`+0.08` pp)|
|`libgit2/index`|`2`|`0`|`+3` (`+0.01`/`+0.00` pp)|`+0` (`+0.10`/`+0.00` pp)|`+1` (`+0.00`/`+0.01` pp)|`+1` (`-0.02`/`+0.00` pp)|
| **Σ** |  |  | **`+14` (`+0.34`/`-2.83` pp)** | **`+2` (`-0.92`/`-0.90` pp)** | **`+19` (`+0.09`/`-2.35` pp)** | **`+32` (`+2.44`/`+2.28` pp)** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude` through `libgit2/object` review wave 0; `openrouter/anthropic/claude-opus-5.5` via `claude` from `libgit2/object` review wave 1.

| subsystem | wave | batch | +lifecycle | +unsafe | +UB safe | +UB unsafe | +equiv | +internal | +conventions | +misc | total | +rejected PoCs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `libgit2/util` | `0` | `2` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` | `0` |
| `libgit2/util` | `1` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `1` | `0` |
| `libgit2/oid` | `1` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` | `0` |
| `libgit2/oid` | `2` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `0` | `0` |
| `libgit2/core` | `0` | `1` | — | `+0` | `+0` | `+1` | `+0` | `+0` | `+0` | `+2` | `3` | `0` |
| `libgit2/core` | `1` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `2` | `0` |
| `libgit2/core` | `2` | `0` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `2` | `0` |
| `libgit2/repository` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+1` | `+2` | `4` | `0` |
| `libgit2/repository` | `2` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+4` | `5` | `0` |
| `libgit2/object` | `0` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` | `0` |
| `libgit2/object` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+1` | `+0` | `+2` | `4` | `0` |
| `libgit2/object` | `2` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+1` | `3` | `0` |
| `libgit2/object` | `3` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `+4` | `6` | `0` |
| `libgit2/config` | `0` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` | `0` |
| `libgit2/config` | `1` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `+4` | `6` | `0` |
| `libgit2/config` | `2` | `1` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+2` | `4` | `0` |
| `libgit2/config` | `3` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+0` | `+2` | `2` | `0` |
| `libgit2/config` | `4` | `1` | — | `+0` | `+0` | `+0` | `+0` | `+0` | `+1` | `+1` | `2` | `0` |
| `libgit2/config` | `5` | `0` | — | `+0` | `+0` | `+0` | `+0` | `+1` | `+0` | `+0` | `1` | `0` |
| `libgit2/index` | `0` | `1` | — | `+1` | `+0` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` | `0` |
| `libgit2/index` | `1` | `1` | — | `+0` | `+1` | `+0` | `+0` | `+0` | `+0` | `+5` | `6` | `0` |
| `libgit2/index` | `2` | `0` | — | `+0` | `+0` | `+0` | `+1` | `+0` | `+1` | `+4` | `6` | `0` |
| **Σ** |  |  | — | **`+4`** | **`+3`** | **`+1`** | **`+3`** | **`+4`** | **`+11`** | **`+44`** | **`70`** | **`0`** |

## Safety audit

Deterministic `crustify scan-unsafe <workdir>`; no model.

### Final tally overview

| | final (`a26547a6d`) |
|---|---|
| unsafe loc | `2137` |
| % of loc | `15.64`% |
| blocks | `1313` |
| % in `impl T` | `71.97`% |
| `unsafe fn` | `254` |
| ...of which not sanctioned | `59` |
| raw-ptr smell | `25` |
| void-ptr smell | `7` |
| FFI calls | `435` |
| `&`/`&mut` on a wrapper | `8` |
| field proj outside an accessor | `4` |

### All metrics

| metric | final (`a26547a6d`) | `raw-lifetime-void` (`99533715f`) | `raw-lifetime-string` (`3830c3c3d`) | `libgit2/util` (`6993f6bcd`) | `libgit2/oid` (`2752d23f4`) | `libgit2/core` (`bde2697ad`) | `libgit2/repository` (`e2e2a7473`) | `libgit2/object` (`8faffad69`) | `libgit2/config` (`6afdb3b46`) |
|---|---|---|---|---|---|---|---|---|---|
| `code_lines` | `13666` | `92` | `239` | `571` | `1106` | `3209` | `6735` | `11421` | `13666` |
| `total_stmts` | `3318` | `29` | `71` | `248` | `378` | `794` | `1647` | `2806` | `3318` |
| `unsafe_blocks` | `1313` | `12` | `30` | `64` | `117` | `312` | `642` | `1081` | `1313` |
| `unsafe_block_stmts` | `79` | `0` | `0` | `0` | `9` | `18` | `40` | `72` | `79` |
| `unsafe_block_lines` | `2183` | `12` | `30` | `64` | `150` | `436` | `922` | `1832` | `2183` |
| `unsafe_block_code_lines` | `2137` | `12` | `30` | `62` | `146` | `412` | `884` | `1789` | `2137` |
| `unsafe_blocks_wrapper_impl` | `945` | `0` | `11` | `30` | `78` | `157` | `435` | `758` | `945` |
| `unsafe_blocks_ffi_export` | `76` | `0` | `0` | `4` | `4` | `9` | `16` | `37` | `76` |
| `unsafe_fns` | `254` | `6` | `12` | `22` | `30` | `101` | `152` | `206` | `254` |
| `unsafe_fns_seam` | `195` | `3` | `6` | `15` | `23` | `48` | `93` | `148` | `195` |
| `unsafe_fns_pub_seam` | `49` | `3` | `6` | `6` | `8` | `14` | `26` | `44` | `49` |
| `unsafe_fns_priv_seam` | `146` | `0` | `0` | `9` | `15` | `34` | `67` | `104` | `146` |
| **`unsafe fn` smell** | **`59`** | `3` | `6` | `7` | `7` | `53` | `59` | `58` | `59` |
| `unsafe_fns_pub_smell` | `42` | `0` | `1` | `1` | `1` | `36` | `42` | `41` | `42` |
| `unsafe_fns_priv_smell` | `17` | `3` | `5` | `6` | `6` | `17` | `17` | `17` | `17` |
| `fns_pub` | `1207` | `7` | `23` | `58` | `111` | `335` | `676` | `1037` | `1207` |
| `fns_priv` | `474` | `6` | `12` | `30` | `50` | `112` | `224` | `362` | `474` |
| `unsafe_impls` / `unsafe_traits` | `241` / `1` | `5` / `0` | `7` / `0` | `15` / `1` | `35` / `1` | `76` / `1` | `140` / `1` | `213` / `1` | `241` / `1` |
| `ffi_calls` | `435` | `3` | `3` | `10` | `29` | `76` | `203` | `380` | `435` |
| `wrapper_newtypes` | `60` | `0` | `1` | `3` | `6` | `12` | `27` | `45` | `60` |
| `wrapper_newtypes_declared` | `52` | `0` | `0` | `2` | `5` | `10` | `25` | `43` | `52` |
| `wrapper_declared_nonconformant` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `wrapper_newtypes_undeclared` | `8` | `0` | `1` | `1` | `1` | `2` | `2` | `2` | `8` |
| `raw_ptr_args` | `200` | `5` | `8` | `19` | `25` | `49` | `91` | `142` | `200` |
| `raw_ptr_rets` | `172` | `2` | `6` | `16` | `26` | `49` | `99` | `143` | `172` |
| `raw_ptr_fields` | `1` | `0` | `0` | `0` | `0` | `1` | `2` | `1` | `1` |
| **total positions** | **`373`** | `7` | `14` | `35` | `51` | `99` | `192` | `286` | `373` |
| `raw_ptr_seam` | `348` | `2` | `5` | `25` | `41` | `81` | `163` | `261` | `348` |
| `raw_ptr_pub_seam` | `5` | `2` | `5` | `5` | `5` | `5` | `5` | `5` | `5` |
| `raw_ptr_priv_seam` | `343` | `0` | `0` | `20` | `36` | `76` | `158` | `256` | `343` |
| `raw_ptr_pub_smell` | `6` | `0` | `0` | `0` | `0` | `0` | `9` | `6` | `6` |
| `raw_ptr_priv_smell` | `19` | `5` | `9` | `10` | `10` | `18` | `20` | `19` | `19` |
| `raw_ptr_pub` | `11` | `2` | `5` | `5` | `5` | `5` | `14` | `11` | `11` |
| `raw_ptr_priv` | `362` | `5` | `9` | `30` | `46` | `94` | `178` | `275` | `362` |
| **smell (total − seam)** | **`25`** | `5` | `9` | `10` | `10` | `18` | `29` | `25` | `25` |
| `raw_ptr_wrapped` | `5` | `0` | `0` | `0` | `0` | `5` | `5` | `5` | `5` |
| `raw_ptr_derefs` | `311` | `0` | `0` | `8` | `26` | `65` | `152` | `243` | `311` |
| `ref_to_type_wrapper_sanctioned` | `134` | `0` | `2` | `8` | `21` | `36` | `70` | `110` | `134` |
| `ref_to_type_wrapper_smell` | `8` | `0` | `8` | `8` | `8` | `8` | `8` | `8` | `8` |
| `ref_to_type_wrapper_body_smell` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `deref_impl_on_wrapper` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `field_proj_wrapped` | `285` | `0` | `0` | `8` | `26` | `60` | `144` | `227` | `285` |
| `field_proj_outside_impl` | `4` | `0` | `0` | `0` | `0` | `4` | `4` | `4` | `4` |
| `field_ref_wrapped` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` | `0` |
| `void_ptr_seam` | `89` | `0` | `0` | `10` | `16` | `31` | `55` | `73` | `89` |
| `void_ptr_pub_smell` | `6` | `0` | `0` | `0` | `0` | `0` | `6` | `6` | `6` |
| `void_ptr_priv_smell` | `1` | `0` | `0` | `0` | `0` | `0` | `1` | `1` | `1` |

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

Say why that non-test figure differs from the `code_lines` the Safety audit
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
which units a review schedule dropped and why; which sub-campaigns the Overview
lists but no table details, and the cost that leaves unaccounted; and any column
a campaign could not fill, said once rather than left as a field of em-dashes.

### C test baseline

Every offline `libgit2_tests` suite and every `util_tests` suite ran in isolation
(`crustify/scripts/c-suites.sh`) against each prebuild; the per-variant record is
`crustify/campaigns/20261008-170008/c-baseline.json`. Plain, coverage and ASan+UBSan pass
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
non-`tests/` sources. Inline `#[cfg(test)] mod unit_tests` code lives in those sources, so
the Rust figure includes it. A Σ `cov` cell is the state after the table's chronologically
last landing. The `(+C/+Rust pp)` pairs in the `…: tests` tables are each workload's own
coverage after minus before, so a batch that adds code its workload does not reach shows a
negative Rust delta. Smell deltas are the crate-wide `unsafe_fn_smell` and `raw_ptr_smell`
of an unseeded `crustify scan-unsafe` over the same two states.

Test counts are `#[test]` functions the landing commits add, by directory; trybuild
`compile_fail` cases are not counted. `+LoC` and `rv loc` count `.rs` lines under
`crustify/rust/`, so review reproducer crates under `crustify/reviews/` are excluded. The
measurement scripts live outside the repository, beside the orchestrator's other helpers;
each batch's `scan-unsafe.json` beside it is the unseeded scan of its last landing commit.

### `libgit2/util`

Wave 0 wrapped `git_allocator`, `git_error`, `git_error_code`, `git_error_t` and the
`git2/sys/errors.h` setters; wave 1 wrapped `git_error_last`. The review's two reproducers
held on rerun: an ASan double free when a caller follows the documented `realloc` contract
to the letter for a zero-byte resize on glibc (`unsafe/alloc-realloc-contract-zero-size-resize`),
and an equivalence-test flake where a fresh thread can start with `errno == EAGAIN`
(`misc/equiv-runners-read-errno-of-fresh-thread`, also filed twice more as no-PoC reports).
`misc/asan-tsan-runner-sanitizes-host-proc-macros` did not reproduce at its base commit,
where `asan-ubsan` and `tsan` build and pass `ub_safe` 50/50 from clean target dirs. It was
rejected and its runner change reverted (`util/rejected-pocs.md`); it is a tooling report,
so the `+rejected PoCs` column, which counts `ub_safe`, `ub_unsafe` and `equiv` PoCs, does not
include it.

### `git_allocator` exposes raw callback pointers

`util/sys/alloc.rs` wraps `git_allocator` through an `unsafe trait Alloc` with private
`extern "C"` trampolines (`Allocator::new::<A>()`), but also publishes the raw form: the
`MallocFn` / `ReallocFn` / `FreeFn` aliases, `Allocator::from_fns`, and accessors that take
or return those pointers. The raw form is unnecessary once every `Allocator` is built from an
`Alloc` implementation, and the review's confirmed zero-byte `realloc` double free came from
its contract. The `util` review judged the aliases necessary ("C function pointers and must
stay so"), and `scan-unsafe` does not count function-pointer types, so neither flagged it.
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

### `scan-unsafe` schema from crustify `4771856`

Crustify's scanner changed after `libgit2/core` (`f66b2c3`..`4771856`): it lost its seeded
`--name` mode, split `unsafe_fns`, raw-pointer and `void *` positions into seam, pub smell and
private smell, added `raw_ptr_fields`, the three `ref_to_type_wrapper_*` counters and
`deref_impl_on_wrapper`, and dropped `raw_ptr_in_wrapper`. Every recorded scan was rerun with
it: each batch's `scan-unsafe.json` at its last landing commit (the seeded per-batch scans,
and `raw-lifetime-void`'s seeded `scan-unsafe.primitives.json` over the C primitives, have no
counterpart and are gone), each wave's and sub-campaign's at the commit that recorded it. The
smell deltas in the batch tables were recomputed from the same scanner, which moved
`libgit2/oid` wave 0 batch 0 and wave 1 batch 0 from `+1` to `+0` `unsafe fn` smell; no other earlier row
changed.


### Safety audit columns and `cbf6fbc` counters

Crustify `cbf6fbc` added eight `scan-unsafe` counters, all split by effective visibility:
`unsafe_fns_pub_seam`/`_priv_seam`, `raw_ptr_pub_seam`/`_priv_seam`, `raw_ptr_pub`/`_priv`
and `fns_pub`/`fns_priv`. It changed no existing counter. The campaign scan and every
sub-campaign scan were rerun with it, and the batch and wave scans, whose counters it left
unchanged, were not. The rerun matched every earlier value. Each All-metrics column is the
crate-wide scan at that sub-campaign's end, so the columns accumulate left to right. `final`
is the campaign tip. `libgit2/config` is scanned at its tip after the export revert and the
`git_config_backend` follow-up, so it equals `final`.

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
`git_config_backend` translated again in one `claude-opus-5.5` batch, shown as the second
`libgit2/config` wave `4` batch `0` row. Its `batch.json` is the scheduled entry verbatim,
landed through `crustify/waves/20261008-170008/libgit2/config/wave-4-followup`. Shared
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
