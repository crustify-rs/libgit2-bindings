# crustify — `libgit2 / libgit2`

Copy this template into the campaign checkout, then fill it as work lands.

## Campaign

- **target repo** — `libgit2` @ `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`
- **target** — `libgit2` (`libgit2.so` public API, deprecated items excluded)
- **campaign objective** — `wrap`
- **`impl_files`** — `src/libgit2/`, `src/util/`, `deps/xdiff/`, `deps/reftable/`, `deps/ntlmclient/`, `deps/llhttp/`
- **`api_headers`** — `include/git2.h`, `include/git2/*.h`, `include/git2/sys/*.h`, except `deprecated.h`, `cred_helpers.h`, `stdint.h` and `sys/cred.h`
- **agent backend** — `claude`
- **model** — `openrouter/anthropic/claude-opus-5.5`
- **`--billing`** — `api`
- **`--max-types`** — `2`
- **`--max-syms`** — `25`
- **`--max-loc`** — `500`
- **`--min-fields`** — `20`
- **orchestrator parallelism** — `10` concurrent batch processes through `libgit2/core`; `16` from `libgit2/repository`
- **branch** — `crustify/campaigns/20261008-170008`, tip `<sha>`
- **deps** — crustify `0fcfcae` (`main`, from `libgit2/core`; through `libgit2/oid` `7912683` with `scan-unsafe` from a local driver port, since superseded by upstream's nightly pin `4efc0cc`; every recorded `scan-unsafe` from `4771856`), ffibox `695d52d` (`main`)
- **build version** — `1`
- **C test baseline** — `3443/3498, 55 skipped, and ctest targets invasive, gitdaemon, gitdaemon_namespace, gitdaemon_sha256, ssh, online, online_customcert, proxy, auth_clone, auth_clone_and_push disabled`

## Review pass

Thin batches with `"objective": "review"`, LLM-as-a-Judge over each landed wave.

- **agent backend** — `claude`
- **model** — `openrouter/anthropic/claude-fable-5.1`
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
- **ported types** — `<n>`
- **ported symbols** — `<n>`
- **wrapped types** — `<n>`
- **wrapped symbols** — `<n>`

Implementation `openrouter/anthropic/claude-opus-5.5` via `claude`; review
`openrouter/anthropic/claude-fable-5.1` via `claude`.

| sub-campaign | nr types | nr symbols | sub-campaign wall | batch | review | total | $/type | $/sym |
|---|---:|---:|---|---:|---:|---:|---:|---:|
| `raw-lifetime-void` | `0` | `2` | `1h54m19s` | `$8.28` | `$7.89` | `$16.17` | — | `$8.09` |
| `raw-lifetime-string` | `0` | `2` | `52m30s` | `$5.49` | `$10.89` | `$16.38` | — | `$8.19` |
| `libgit2/util` | `4` | `5` | `2h24m28s` | `$19.86` | `$46.50` | `$66.36` | `$16.59` | `$13.27` |
| `libgit2/oid` | `4` | `19` | `3h14m42s` | `$27.25` | `$48.59` | `$75.84` | `$18.96` | `$3.99` |
| `libgit2/core` | `16` | `12` | `5h06m51s` | `$96.92` | `$70.82` | `$167.74` | `$10.48` | `$13.98` |
| `libgit2/repository` | `23` | `75` | `8h27m28s` | `$227.32` | `$123.41` | `$350.73` | `$15.25` | `$4.68` |
| orchestrator | `<n>` | `<n>` | — | — | — | `$<n>`+ | — | — |
| **Σ recorded agents** | **`47`** | **`115`** | **`22h00m`** | **`$385.12`** | **`$308.10`** | **`$693.22`** | **`$14.75`** | **`$6.03`** |

## Raw lifetime discovery

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| tier | strategies emitted | $ | wall |
|---|---|---|---|
| void | `2` | `$8.28` | `51m29s` |
| string | `2` | `$5.49` | `23m12s` |
| **Σ** | **`4`** | **`$13.77`** | **`74m41s`** |

### Review

`openrouter/anthropic/claude-fable-5.1` via `claude`.

| tier | rv loc | rv $ | rv wall |
|---|---|---|---|
| void | `+80/-1` | `$7.89` | `18m36s` |
| string | `+345/-23` | `$10.89` | `22m03s` |
| **Σ** | **`+425/-24`** | **`$18.78`** | **`40m39s`** |

## Types

### Batches — wrap

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | types | fields | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`| `2` | `5` |`+1934/-2`| `1.30`% / `95.76`% | `+1` | `+1` | `$7.55` | `33m43s` | `$3.77` | `$1.51` |
|`libgit2/util`|`0`|`1`| `2` | `0` |`+849/-26`| `1.35`% / `96.22`% | `+0` | `+0` | `$5.25` | `38m51s` | `$2.62` | — |
|`libgit2/oid`|`0`|`0`| `2` | `2` |`+1514/-4`| `1.73`% / `96.25`% | `+0` | `+0` | `$9.89` | `49m08s` | `$4.95` | `$4.95` |
|`libgit2/oid`|`0`|`1`| `1` | `0` |`+488/-0`| `1.68`% / `95.42`% | `+0` | `+0` | `$2.87` | `14m31s` | `$2.87` | — |
|`libgit2/oid`|`1`|`0`| `1` | `2` |`+1397/-0`| `5.89`% / `96.82`% | `+0` | `+0` | `$5.30` | `19m02s` | `$5.30` | `$2.65` |
|`libgit2/core`|`0`|`0`| `2` | `3` |`+2495/-315`| `15.24`% / `98.02`% | `+0` | `+1` | `$14.63` | `54m01s` | `$7.32` | `$4.88` |
|`libgit2/core`|`0`|`1`| `2` | `0` |`+909/-0`| `9.22`% / `96.78`% | `+0` | `+0` | `$2.54` | `16m38s` | `$1.27` | — |
|`libgit2/core`|`0`|`2`| `2` | `0` |`+1784/-6`| `12.15`% / `97.29`% | `+0` | `+0` | `$7.38` | `28m17s` | `$3.69` | — |
|`libgit2/core`|`0`|`3`| `2` | `0` |`+871/-19`| `10.96`% / `97.14`% | `+0` | `+0` | `$5.25` | `22m58s` | `$2.62` | — |
|`libgit2/core`|`0`|`4`| `2` | `2` |`+2575/-6`| `14.28`% / `97.84`% | `+0` | `+1` | `$12.07` | `43m43s` | `$6.04` | `$6.04` |
|`libgit2/core`|`0`|`5`| `2` | `0` |`+839/-0`| `9.19`% / `96.51`% | `+0` | `+0` | `$3.05` | `13m22s` | `$1.52` | — |
|`libgit2/core`|`0`|`6`| `2` | `3` |`+1134/-3`| `9.42`% / `97.01`% | `+0` | `+0` | `$4.22` | `17m55s` | `$2.11` | `$1.41` |
|`libgit2/core`|`0`|`7`| `1` | `3` |`+2428/-3`| `13.87`% / `97.53`% | `+3` | `+3` | `$10.74` | `33m33s` | `$10.74` | `$3.58` |
|`libgit2/core`|`1`|`0`| `1` | `3` |`+1745/-3`| `18.22`% / `98.23`% | `+2` | `+3` | `$10.23` | `26m16s` | `$10.23` | `$3.41` |
|`libgit2/repository`|`0`|`0`| `1` | `0` |`+1799/-0`| `32.72`% / `98.56`% | `+0` | `+0` | `$24.34` | `1h23m54s` | `$24.34` | — |
|`libgit2/repository`|`0`|`1`| `1` | `0` |`+3376/-0`| `31.33`% / `98.56`% | `+4` | `+8` | `$16.26` | `55m56s` | `$16.26` | — |
|`libgit2/repository`|`0`|`2`| `2` | `0` |`+1925/-0`| `19.74`% / `98.49`% | `+0` | `+1` | `$8.59` | `29m29s` | `$4.29` | — |
|`libgit2/repository`|`0`|`3`| `1` | `0` |`+1789/-0`| `25.32`% / `98.59`% | `+0` | `+0` | `$11.26` | `42m24s` | `$11.26` | — |
|`libgit2/repository`|`0`|`4`| `2` | `0` |`+1635/-1`| `23.97`% / `98.53`% | `+0` | `+0` | `$11.67` | `35m52s` | `$5.83` | — |
|`libgit2/repository`|`0`|`5`| `2` | `0` |`+1585/-0`| `23.69`% / `98.51`% | `+2` | `+0` | `$10.66` | `32m15s` | `$5.33` | — |
|`libgit2/repository`|`0`|`6`| `2` | `0` |`+2089/-0`| `24.02`% / `98.57`% | `+0` | `+0` | `$10.91` | `38m41s` | `$5.46` | — |
|`libgit2/repository`|`0`|`7`| `1` | `0` |`+1491/-194`| `25.76`% / `98.64`% | `-1` | `+0` | `$15.47` | `51m17s` | `$15.47` | — |
|`libgit2/repository`|`0`|`8`| `2` | `0` |`+1279/-0`| `18.62`% / `98.30`% | `+0` | `+0` | `$4.27` | `18m41s` | `$2.13` | — |
|`libgit2/repository`|`0`|`9`| `2` | `0` |`+1516/-0`| `19.22`% / `98.43`% | `+0` | `+0` | `$6.34` | `27m40s` | `$3.17` | — |
|`libgit2/repository`|`0`|`10`| `2` | `0` |`+1587/-1`| `25.67`% / `98.62`% | `+0` | `+0` | `$7.92` | `45m21s` | `$3.96` | — |
|`libgit2/repository`|`0`|`11`| `2` | `2` |`+1545/-0`| `18.98`% / `98.37`% | `+0` | `+0` | `$5.41` | `23m56s` | `$2.71` | `$2.71` |
|`libgit2/repository`|`1`|`0`| `2` | `12` |`+2748/-28`| `33.79`% / `98.66`% | `+0` | `+0` | `$10.14` | `28m55s` | `$5.07` | `$0.85` |
|`libgit2/repository`|`1`|`1`| `1` | `5` |`+1563/-3`| `33.24`% / `98.60`% | `+0` | `+0` | `$6.36` | `20m02s` | `$6.36` | `$1.27` |
| **Σ** |  |  | **`47`** | **`42`** | **`+46889/-614`** | **`33.79`% / `98.66`%** | **`+11`** | **`+18`** | **`$250.57`** | — | **`$5.33`** | **`$5.97`** |

### Batches — wrap: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

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
| **Σ** |  |  | **`+258` (`+20.22`/`-0.61` pp)** | **`+31` (`+15.67`/`-19.84` pp)** | **`+151` (`+26.58`/`-7.39` pp)** | **`+272` (`+9.28`/`+12.30` pp)** |

### Batches — port

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | types | fields | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | wall | $/type | $/field |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`<link-unit>/<subsystem>`|`<n>`|`<n>`| `<n>` | `<n>` |`+<n>/-<n>`| `<n>`% / `<n>`% | `±<n>` | `±<n>` | `$<n>` | `<n>m<n>s` | `$<n>` | `$<n>` |
| **Σ** |  |  | **`<n>`** | **`<n>`** | **`+<n>/-<n>`** | **`<n>`% / `<n>`%** | **`±<n>`** | **`±<n>`** | **`$<n>`** | — | **`$<n>`** | **`$<n>`** |

### Batches — port: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | +UB safe tests (+C/+Rust pp) | +UB unsafe tests (+C/+Rust pp) | +equiv tests (+C/+Rust pp) | +unit tests (+C/+Rust pp) |
|---|---|---|---|---|---|---|
|`<link-unit>/<subsystem>`|`<n>`|`<n>`|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|`+<n>` (`+<n>`/`+<n>` pp)|
| **Σ** |  |  | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** | **`+<n>` (`+<n>`/`+<n>` pp)** |

### Batches — review

`openrouter/anthropic/claude-fable-5.1` via `claude`.

| subsystem | wave | batch | types | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | rv loc | rv $ | rv wall | rv $/type |
|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`0`| `2` |`+172/-15`| `1.63`% / `95.18`% | `+0` | `+0` | `+172/-15` | `$20.66` | `48m16s` | `$10.33` |
|`libgit2/util`|`0`|`1`| `2` |`+149/-12`| `1.63`% / `96.51`% | `+0` | `+0` | `+149/-12` | `$7.58` | `18m18s` | `$3.79` |
|`libgit2/oid`|`0`|`0`| `2` |`+224/-6`| `7.81`% / `96.25`% | `+0` | `+0` | `+224/-6` | `$14.93` | `29m10s` | `$7.46` |
|`libgit2/oid`|`0`|`1`| `1` |`+185/-8`| `7.81`% / `96.12`% | `+0` | `+0` | `+185/-8` | `$7.16` | `19m44s` | `$7.16` |
|`libgit2/oid`|`1`|`0`| `1` |`+235/-18`| `7.80`% / `96.36`% | `+0` | `+0` | `+235/-18` | `$11.49` | `22m41s` | `$11.49` |
|`libgit2/core`|`0`|`0`| `15` |`+263/-9`| `15.84`% / `98.11`% | `+0` | `+0` | `+263/-9` | `$24.78` | `43m19s` | `$1.65` |
|`libgit2/core`|`1`|`0`| `1` |`+126/-7`| `18.23`% / `98.26`% | `+0` | `+0` | `+126/-7` | `$18.99` | `39m44s` | `$18.99` |
|`libgit2/repository`|`0`|`0`| `20` |`+743/-98`| `32.73`% / `98.58`% | `+0` | `+0` | `+743/-98` | `$32.90` | `46m45s` | `$1.65` |
|`libgit2/repository`|`1`|`0`| `3` |`+227/-11`| `36.05`% / `98.53`% | `+0` | `+0` | `+227/-11` | `$21.87` | `41m52s` | `$7.29` |
| **Σ** |  |  | **`47`** | **`+2324/-184`** | **`36.05`% / `98.53`%** | **`+0`** | **`+0`** | **`+2324/-184`** | **`$160.36`** | — | **`$3.41`** |

### Batches — review: tests

`openrouter/anthropic/claude-fable-5.1` via `claude`.

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
| **Σ** |  |  | **`+6` (`+0.22`/`-1.30` pp)** | **`+1` (`-0.60`/`-1.69` pp)** | **`+9` (`+2.35`/`-2.21` pp)** | **`+12` (`+4.33`/`+0.08` pp)** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude`.

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
| **Σ** |  |  | **`+2`** | **`+3`** | **`+1`** | **`+0`** | **`+0`** | **`+2`** | **`+0`** | **`+16`** | **`24`** | **`0`** |

## Symbols

### Batches

`openrouter/anthropic/claude-opus-5.5` via `claude`.

| subsystem | wave | batch | objective | symbols | loc | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | $ | wall | $/symbol | $/loc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`| wrap | `4` | `51` |`+1008/-26`| `1.30`% / `95.11`% | `+0` | `+0` | `$4.00` | `21m23s` | `$1.00` | `$0.08` |
|`libgit2/util`|`1`|`0`| wrap | `1` | `15` |`+756/-71`| `1.36`% / `96.43`% | `+0` | `+0` | `$3.06` | `15m41s` | `$3.06` | `$0.20` |
|`libgit2/oid`|`1`|`1`| wrap | `18` | `252` |`+1351/-11`| `6.07`% / `96.12`% | `+0` | `+0` | `$8.55` | `40m06s` | `$0.48` | `$0.03` |
|`libgit2/oid`|`2`|`0`| wrap | `1` | `3` |`+0/-0`| `6.07`% / `96.12`% | `+0` | `+0` | `$0.64` | `4m17s` | `$0.64` | `$0.21` |
|`libgit2/core`|`0`|`8`| wrap | `6` | `444` |`+3504/-75`| `15.52`% / `98.11`% | `+37` | `+0` | `$19.90` | `1h04m11s` | `$3.32` | `$0.04` |
|`libgit2/core`|`1`|`1`| wrap | `5` | `212` |`+1238/-4`| `18.20`% / `98.15`% | `+1` | `+0` | `$4.61` | `19m37s` | `$0.92` | `$0.02` |
|`libgit2/core`|`2`|`0`| wrap | `1` | `9` |`+658/-41`| `18.24`% / `98.26`% | `+2` | `+0` | `$2.30` | `10m40s` | `$2.30` | `$0.26` |
|`libgit2/repository`|`1`|`2`| wrap | `25` | `436` |`+3311/-54`| `36.06`% / `98.53`% | `+0` | `+2` | `$24.81` | `1h01m00s` | `$0.99` | `$0.06` |
|`libgit2/repository`|`1`|`3`| wrap | `25` | `432` |`+3392/-29`| `35.39`% / `98.55`% | `+1` | `+0` | `$20.18` | `53m42s` | `$0.81` | `$0.05` |
|`libgit2/repository`|`1`|`4`| wrap | `18` | `396` |`+3072/-105`| `34.12`% / `98.60`% | `+0` | `+0` | `$15.55` | `45m11s` | `$0.86` | `$0.04` |
|`libgit2/repository`|`2`|`0`| wrap | `7` | `373` |`+1449/-382`| `36.17`% / `98.46`% | `+0` | `+0` | `$17.18` | `32m30s` | `$2.45` | `$0.05` |
| **Σ** |  |  |  | **`111`** | **`2623`** | **`+19739/-798`** | **`36.17`% / `98.46`%** | **`+41`** | **`+2`** | **`$120.78`** | — | **`$1.09`** | **`$0.05`** |

### Batches: tests

`openrouter/anthropic/claude-opus-5.5` via `claude`.

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
| **Σ** |  |  | **`+89` (`+5.67`/`+4.60` pp)** | **`+13` (`+5.00`/`+0.52` pp)** | **`+83` (`+5.34`/`+7.43` pp)** | **`+70` (`+2.86`/`-11.57` pp)** |

### Batches — review

`openrouter/anthropic/claude-fable-5.1` via `claude`.

| subsystem | wave | batch | symbols | +LoC | cov (C/Rust) | +unsafe fn smell | +raw-ptr smell | rv loc | rv $ | rv wall | rv $/symbol |
|---|---|---|---|---|---|---|---|---|---|---|---|
|`libgit2/util`|`0`|`2`| `4` |`+281/-117`| `1.63`% / `96.03`% | `+0` | `+0` | `+281/-117` | `$12.00` | `28m49s` | `$3.00` |
|`libgit2/util`|`1`|`0`| `1` |`+0/-0`| `1.63`% / `95.18`% | `+0` | `+0` | `+0/-0` | `$6.26` | `23m51s` | `$6.26` |
|`libgit2/oid`|`1`|`1`| `18` |`+181/-10`| `7.80`% / `96.39`% | `+0` | `+0` | `+181/-10` | `$11.62` | `34m10s` | `$0.65` |
|`libgit2/oid`|`2`|`0`| `1` |`+0/-0`| `7.80`% / `96.39`% | `+0` | `+0` | `+0/-0` | `$3.39` | `8m46s` | `$3.39` |
|`libgit2/core`|`0`|`1`| `6` |`+345/-20`| `15.51`% / `98.11`% | `+1` | `+0` | `+345/-20` | `$12.53` | `23m38s` | `$2.09` |
|`libgit2/core`|`1`|`1`| `5` |`+59/-10`| `18.24`% / `98.23`% | `+0` | `+0` | `+59/-10` | `$7.69` | `17m46s` | `$1.54` |
|`libgit2/core`|`2`|`0`| `1` |`+61/-9`| `18.23`% / `98.26`% | `+0` | `+0` | `+61/-9` | `$6.83` | `18m10s` | `$6.83` |
|`libgit2/repository`|`1`|`1`| `68` |`+640/-143`| `36.16`% / `98.55`% | `+0` | `+0` | `+640/-143` | `$38.68` | `1h08m35s` | `$0.57` |
|`libgit2/repository`|`2`|`0`| `7` |`+574/-18`| `36.17`% / `98.48`% | `+0` | `+0` | `+574/-18` | `$29.96` | `57m28s` | `$4.28` |
| **Σ** |  |  | **`111`** | **`+2141/-327`** | **`36.17`% / `98.48`%** | **`+1`** | **`+0`** | **`+2141/-327`** | **`$128.96`** | — | **`$1.16`** |

### Batches — review: tests

`openrouter/anthropic/claude-fable-5.1` via `claude`.

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
| **Σ** |  |  | **`+2` (`+0.08`/`-2.31` pp)** | **`+2` (`-0.09`/`-0.71` pp)** | **`+6` (`+0.04`/`-2.10` pp)** | **`+9` (`+2.20`/`+1.63` pp)** |

### Batches — review: PoC + reports

`openrouter/anthropic/claude-fable-5.1` via `claude`.

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
| **Σ** |  |  | — | **`+3`** | **`+1`** | **`+1`** | **`+0`** | **`+1`** | **`+3`** | **`+11`** | **`20`** | **`0`** |

## Safety audit

Deterministic `crustify scan-unsafe <workdir>`; no model.

### Final tally overview

| | final (`<sha>`) |
|---|---|
| unsafe loc | `<n>` |
| % of loc | `<n>`% |
| blocks | `<n>` |
| % in `impl T` | `<n>`% |
| `unsafe fn` | `<n>` |
| ...of which not sanctioned | `<n>` |
| raw-ptr smell | `<n>` |
| void-ptr smell | `<n>` |
| FFI calls | `<n>` |
| `&`/`&mut` on a wrapper | `<n>` |
| field proj outside an accessor | `<n>` |

### All metrics

| metric | final (`<sha>`) | `<link-unit>/<subsystem>` (`<sha>`) | … |
|---|---|---|---|
| `code_lines` | `<n>` | `<n>` | … |
| `total_stmts` | `<n>` | `<n>` | … |
| `unsafe_blocks` | `<n>` | `<n>` | … |
| `unsafe_block_stmts` | `<n>` | `<n>` | … |
| `unsafe_block_lines` | `<n>` | `<n>` | … |
| `unsafe_block_code_lines` | `<n>` | `<n>` | … |
| `unsafe_blocks_wrapper_impl` | `<n>` | `<n>` | … |
| `unsafe_blocks_ffi_export` | `<n>` | `<n>` | … |
| `unsafe_fns` | `<n>` | `<n>` | … |
| `unsafe_fns_seam` | `<n>` | `<n>` | … |
| **`unsafe fn` smell** | **`<n>`** | `<n>` | … |
| `unsafe_fns_pub_smell` | `<n>` | `<n>` | … |
| `unsafe_fns_priv_smell` | `<n>` | `<n>` | … |
| `unsafe_impls` / `unsafe_traits` | `<n>` / `<n>` | `<n>` / `<n>` | … |
| `ffi_calls` | `<n>` | `<n>` | … |
| `wrapper_newtypes` | `<n>` | `<n>` | … |
| `wrapper_newtypes_declared` | `<n>` | `<n>` | … |
| `wrapper_declared_nonconformant` | `<n>` | `<n>` | … |
| `wrapper_newtypes_undeclared` | `<n>` | `<n>` | … |
| `raw_ptr_args` | `<n>` | `<n>` | … |
| `raw_ptr_rets` | `<n>` | `<n>` | … |
| `raw_ptr_fields` | `<n>` | `<n>` | … |
| **total positions** | **`<n>`** | `<n>` | … |
| `raw_ptr_seam` | `<n>` | `<n>` | … |
| `raw_ptr_pub_smell` | `<n>` | `<n>` | … |
| `raw_ptr_priv_smell` | `<n>` | `<n>` | … |
| **smell (total − seam)** | **`<n>`** | `<n>` | … |
| `raw_ptr_wrapped` | `<n>` | `<n>` | … |
| `raw_ptr_derefs` | `<n>` | `<n>` | … |
| `ref_to_type_wrapper_sanctioned` | `<n>` | `<n>` | … |
| `ref_to_type_wrapper_smell` | `<n>` | `<n>` | … |
| `ref_to_type_wrapper_body_smell` | `<n>` | `<n>` | … |
| `deref_impl_on_wrapper` | `<n>` | `<n>` | … |
| `field_proj_wrapped` | `<n>` | `<n>` | … |
| `field_proj_outside_impl` | `<n>` | `<n>` | … |
| `field_ref_wrapped` | `<n>` | `<n>` | … |
| `void_ptr_seam` | `<n>` | `<n>` | … |
| `void_ptr_pub_smell` | `<n>` | `<n>` | … |
| `void_ptr_priv_smell` | `<n>` | `<n>` | … |

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

### Parallelism and review caps from `libgit2/repository`

Before `libgit2/repository` the operator raised the batch parallelism from orchestrator's
choice (`10`) to up to `16`, and turned the review caps over to the orchestrator. Each
review batch now takes up to `20` types (10× the translation cap) and up to `100` symbols
and callbacks (4×). A route goes into near-equal batches only when it exceeds its cap.
The largest remaining waves have 20 types (`repository` wave 0) and 92 symbols (`refs`
wave 1), so every review wave so far still has at most two batches, as it did from
`libgit2/core`. The widest translation wave left has 12 batches, so with `16` every wave
runs all its batches at once.
