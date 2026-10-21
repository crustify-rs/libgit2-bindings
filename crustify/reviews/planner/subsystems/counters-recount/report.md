# Subsystem counters recounted from the traced build

## What the earlier plan decided

`subsystems.json` carried `counters` for each of the 22 subsystems that no definition in
`docs/artifacts/subsystems.md` reproduces. For example:

```json
"oid": {"implementation": {"files": 4, "loc": 566, "structs": 3, "functions": 47, "global_variables": 4,
                           "enums": 0, "unions": 0, "callbacks": 0, "macros": 0},
        "api": {"files": 2, "loc": 74, "structs": 2, "opaque_structs": 1, "functions": 19,
                "global_variables": 0, "enums": 1, "unions": 0, "callbacks": 0, "macros": 8}}
```

`unions` and implementation `callbacks` were 0 in every subsystem, and `macros` matched no
selection of the macro definitions (all, object-like, function-like, without include guards, or
expanded somewhere).

## Evidence against it

- **Unions.** The traced build defines 12 unions in these files, among them `trie_node`
  (`src/libgit2/oid.c:306`), the anonymous unions of `src/util/hash.h:29`,
  `src/util/bitvec.h:21`, `src/libgit2/blob.h:20`, `src/libgit2/refs.h:73`,
  `src/libgit2/attr_file.h:110`, `src/libgit2/transaction.c:33` and `src/util/rand.c:20`.
  T1's `types.csv` files them as structs (`trie_node` is `typedef,struct_anonymous`), which is
  why `oid`'s implementation `structs` was 3 where it defines 2 structs and 1 union.
- **Callbacks.** 35 function-pointer typedefs sit in implementation files, such as the 10 in
  `src/util/` (T1 `types.csv`, `unaliased_kind` `callback`).
- **Enums and structs.** Anonymous `typedef enum { … } x;` and `typedef struct { … } x;`
  definitions, and embedded anonymous aggregates, were left out in some subsystems and not in
  others (`util` defines 20 enums; 11 were counted).
- **Macros.** `macros` counts object- and function-like macro definitions, which
  `t1/macros.csv` lists by defining file: `oid` defines 4 in its implementation files
  (`GIT_OID_NONE`, `GIT_OID_INIT` and two include guards) and 14 in its two public headers
  (13 in `include/git2/oid.h`, the include guard of `include/git2/oidarray.h`).
- **LoC.** Five subsystems' implementation `loc` disagree with the per-file count, which
  `gcc -fpreprocessed -dD -E -P <file> | grep -cv '^\s*$'` confirms file by file. For example
  `history` has 4264 lines, not 4008: `blame.c` alone has 509.

The C source is unchanged since the recorded `version`:
`git diff --stat cdbe98f53e3c4383a190f1f72d058eee5bf5e75d HEAD -- src include deps` is empty.
Counters that did match (files, functions, global variables, every API block except macros)
still match; the API `functions` sum over all subsystems is 912, the number of distinct
functions the 89 public headers declare.

Rerun:

```sh
codeql query run --database crustify/wavefront/codeql/db --output /tmp/agg.bqrs \
  crustify/reviews/planner/subsystems/counters-recount/ql/aggregates.ql
codeql bqrs decode --format=csv --output /tmp/agg.csv /tmp/agg.bqrs
python3 crustify/reviews/planner/subsystems/counters-recount/counters.py /tmp/agg.csv
```

## The edit

Every subsystem's `counters` is replaced by the recount. The changed fields:

| subsystem | block | field: recorded → recounted |
|---|---|---|
| `llhttp` | implementation | callbacks: 0 → 3; macros: 6 → 19 |
| `ntlmclient` | implementation | unions: 0 → 1; macros: 17 → 51 |
| `util` | implementation | structs: 40 → 42; enums: 11 → 20; unions: 0 → 3; callbacks: 0 → 10; macros: 80 → 350 |
| `util` | api | macros: 0 → 3 |
| `oid` | implementation | structs: 3 → 2; unions: 0 → 1; macros: 0 → 4 |
| `oid` | api | macros: 8 → 14 |
| `reftable` | implementation | structs: 49 → 51; enums: 4 → 7; unions: 0 → 3; callbacks: 0 → 3; macros: 2 → 46 |
| `xdiff` | implementation | callbacks: 0 → 3; macros: 35 → 76 |
| `core` | implementation | macros: 1 → 16 |
| `core` | api | macros: 14 → 28 |
| `repository` | implementation | loc: 4367 → 4371; structs: 8 → 12; enums: 2 → 3; callbacks: 0 → 1; macros: 8 → 39 |
| `repository` | api | macros: 8 → 12 |
| `config` | implementation | structs: 22 → 25; enums: 0 → 1; callbacks: 0 → 4; macros: 2 → 11 |
| `config` | api | macros: 4 → 6 |
| `object` | implementation | structs: 24 → 29; unions: 0 → 1; macros: 13 → 24 |
| `object` | api | macros: 8 → 18 |
| `index` | implementation | structs: 35 → 40; enums: 4 → 6; callbacks: 0 → 2; macros: 21 → 33 |
| `index` | api | macros: 7 → 11 |
| `attr` | implementation | enums: 2 → 3; unions: 0 → 1; callbacks: 0 → 1; macros: 2 → 29 |
| `attr` | api | macros: 11 → 14 |
| `filter` | implementation | structs: 10 → 11; macros: 0 → 3 |
| `filter` | api | macros: 8 → 11 |
| `refs` | implementation | loc: 6649 → 6704; enums: 3 → 7; unions: 0 → 2; macros: 7 → 53 |
| `refs` | api | macros: 2 → 11 |
| `diff` | implementation | structs: 38 → 41; enums: 3 → 7; callbacks: 0 → 4; macros: 25 → 54 |
| `diff` | api | macros: 12 → 22 |
| `odb` | implementation | structs: 68 → 70; enums: 3 → 4; callbacks: 0 → 3; macros: 50 → 84 |
| `odb` | api | macros: 16 → 24 |
| `workdir` | implementation | enums: 0 → 1; macros: 6 → 8 |
| `workdir` | api | macros: 9 → 13 |
| `history` | implementation | loc: 4008 → 4264; structs: 15 → 18; macros: 3 → 20 |
| `history` | api | macros: 8 → 13 |
| `merge` | implementation | structs: 12 → 16; macros: 25 → 32 |
| `merge` | api | macros: 15 → 23 |
| `remote` | implementation | loc: 3522 → 3642; macros: 8 → 15 |
| `remote` | api | macros: 15 → 20 |
| `transport` | implementation | loc: 11765 → 11826; structs: 36 → 38; enums: 10 → 11; callbacks: 0 → 1; macros: 16 → 75 |
| `transport` | api | macros: 3 → 11 |
| `submodule` | implementation | enums: 0 → 3; macros: 1 → 3 |
| `submodule` | api | macros: 7 → 10 |

| block | field | Σ recorded | Σ recounted |
|---|---|---:|---:|
| api | macros | 155 | 264 |
| implementation | callbacks | 0 | 35 |
| implementation | enums | 42 | 73 |
| implementation | loc | 30311 | 30807 |
| implementation | macros | 328 | 1045 |
| implementation | structs | 360 | 395 |
| implementation | unions | 0 | 12 |

The two foundational tiers, new entries in `subsystems.json`, own no C entity and count zeros.

## What moves downstream

Nothing scheduled moves: counters describe the subsystems and feed no wave or batch.
