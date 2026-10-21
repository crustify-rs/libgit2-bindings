# imported_deps recounted from the campaign-wide graph

## What the earlier plan decided

`subsystems.json`'s `imported_deps` recorded 178 in-tree records and their out-of-tree
libraries. In every in-tree record `macros` and `callbacks` were 0, and macros and callbacks
consumed across the edge were counted under `functions`. For example `oid`'s only record:

```json
{"link_unit": "libgit2", "subsystem": "util", "outgoing_edges": 32, "incoming_edges": 0,
 "counters": {"structs": 0, "functions": 16, "global_variables": 0, "enums": 1, "unions": 0,
              "callbacks": 0, "macros": 0}}
```

It also recorded `xdiff → odb` (8 outgoing edges, 1 struct) and its mirror in `odb`.

## Evidence against it

The records are derived from wavefront's default graph of the campaign-wide configuration:
an edge per dependency of an entity one subsystem owns on an entity another owns, `fallback`
(cut) edges included, and each destination entity counted once by its kind. The graph that
derivation reproduces is the one the earlier records came from: 134 of their 177 common
records have identical edge counts. It now differs because:

- **Kinds.** Wavefront classifies its nodes (`subkind` `macro`, `callback`, `function_*`);
  `oid` consumes 12 functions and 5 macros from `util` (`CONST_STRLEN`,
  `GIT_ERROR_CHECK_ALLOC`, `git_array_alloc`, `git_array_foreach`, `git_array_t`), not 16
  functions.
- **T3.** The T3 tables this run added under `crustify/wavefront/t3/` change edges: the
  hierarchy rows reorder a base before the private types that embed it, the generator rows make
  every `GIT_HASHMAP_STRUCT` and `git_array_t` instance depend on its macro in `util`, and the
  drops remove reversed variant edges (`git_odb_stream.hash_ctx` read as `git_hash_ctx`,
  `reftable_mmap.priv` holding a `git_map`, the `git_clone_options` payloads holding a
  `git_submodule`).
- **`xdiff → odb`.** No `deps/xdiff/` file includes anything but `regexp.h` (util) and its own
  headers (`grep -n '#include' deps/xdiff/*.[ch]`), and the graph has no edge from an xdiff
  entity to an odb one.
- **Out of tree.** External entities are attributed to the library whose header declares them,
  matching `ldd crustify/builds/plain/libgit2.so` (libssl, libcrypto, libpcre2-8, libz, libc):
  `util`'s 18 external structs are 11 of libc, 5 of libpcre2-8 (`pcre2_real_*`), 1 of libcrypto
  (`SHA256state_st`) and 1 of libz (`z_stream_s`), not 18 of libc.

Rerun:

```sh
PYTHONHASHSEED=0 /opt/venv/bin/python \
  crustify/reviews/planner/subsystems/imported-deps-recount/subsystem_graph.py deps
```

prints every subsystem's records exactly as `subsystems.json` now holds them.

## The edit

Every subsystem's `imported_deps` is replaced by the recount, in-tree records listed in the
destinations' execution order, mirrored pairwise.

In-tree totals over all records (recorded → recounted):

| kind | Σ recorded | Σ recounted |
|---|---:|---:|
| structs | 483 | 463 |
| functions | 3320 | 2479 |
| global_variables | 49 | 49 |
| enums | 101 | 101 |
| unions | 0 | 0 |
| callbacks | 0 | 13 |
| macros | 0 | 838 |

Records removed: `xdiff → odb` (8 edges)

Records added: none

Edge counts changed:

| record | outgoing: recorded → recounted | incoming: recorded → recounted |
|---|---|---|
| `attr → index` | 11 → 11 | 23 → 22 |
| `config → repository` | 29 → 28 | 102 → 101 |
| `config → util` | 601 → 602 | 0 → 0 |
| `core → filter` | 1 → 1 | 38 → 39 |
| `core → merge` | 1 → 1 | 49 → 50 |
| `core → object` | 1 → 1 | 168 → 169 |
| `diff → index` | 138 → 138 | 19 → 18 |
| `diff → submodule` | 25 → 24 | 10 → 10 |
| `diff → util` | 805 → 807 | 0 → 0 |
| `diff → workdir` | 4 → 4 | 104 → 105 |
| `diff → xdiff` | 20 → 26 | 0 → 0 |
| `filter → core` | 38 → 39 | 1 → 1 |
| `history → index` | 21 → 22 | 0 → 0 |
| `history → oid` | 154 → 155 | 0 → 0 |
| `history → repository` | 65 → 66 | 1 → 1 |
| `history → util` | 347 → 348 | 0 → 0 |
| `index → attr` | 23 → 22 | 11 → 11 |
| `index → diff` | 19 → 18 | 138 → 138 |
| `index → oid` | 64 → 65 | 0 → 0 |
| `index → repository` | 50 → 50 | 14 → 12 |
| `index → util` | 824 → 832 | 0 → 0 |
| `merge → core` | 49 → 50 | 1 → 1 |
| `merge → object` | 124 → 126 | 0 → 0 |
| `merge → repository` | 93 → 95 | 3 → 3 |
| `merge → util` | 490 → 489 | 0 → 0 |
| `object → core` | 168 → 169 | 1 → 1 |
| `object → oid` | 190 → 193 | 0 → 0 |
| `object → repository` | 107 → 108 | 36 → 36 |
| `object → util` | 669 → 670 | 0 → 0 |
| `odb → oid` | 386 → 388 | 0 → 0 |
| `odb → repository` | 21 → 21 | 26 → 25 |
| `odb → util` | 1380 → 1384 | 0 → 0 |
| `odb → xdiff` | 6 → 6 | 8 → 0 |
| `oid → util` | 32 → 33 | 0 → 0 |
| `refs → repository` | 129 → 128 | 151 → 149 |
| `remote → config` | 64 → 65 | 0 → 0 |
| `remote → history` | 9 → 10 | 0 → 0 |
| `remote → odb` | 17 → 19 | 0 → 0 |
| `remote → repository` | 97 → 100 | 4 → 4 |
| `remote → transport` | 17 → 23 | 88 → 95 |
| `remote → util` | 508 → 510 | 0 → 0 |
| `repository → config` | 102 → 101 | 29 → 28 |
| `repository → history` | 1 → 1 | 65 → 66 |
| `repository → index` | 14 → 12 | 50 → 50 |
| `repository → merge` | 3 → 3 | 93 → 95 |
| `repository → object` | 36 → 36 | 107 → 108 |
| `repository → odb` | 26 → 25 | 21 → 21 |
| `repository → oid` | 55 → 57 | 0 → 0 |
| `repository → refs` | 151 → 149 | 129 → 128 |
| `repository → remote` | 4 → 4 | 97 → 100 |
| `submodule → diff` | 10 → 10 | 25 → 24 |
| `transport → diff` | 4 → 3 | 0 → 0 |
| `transport → llhttp` | 27 → 29 | 0 → 0 |
| `transport → odb` | 33 → 37 | 0 → 0 |
| `transport → remote` | 88 → 95 | 17 → 23 |
| `workdir → diff` | 104 → 105 | 4 → 4 |
| `workdir → oid` | 36 → 37 | 0 → 0 |

Out-of-tree records changed:

| subsystem | library | changes |
|---|---|---|
| `attr` | libc | callbacks: 2 → 0 |
| `config` | libc | structs: 4 → 3; callbacks: 5 → 0 |
| `config` | libpcre2-8 | added: structs 1 |
| `core` | libc | structs: 3 → 2; callbacks: 1 → 0; macros: 11 → 9 |
| `core` | libcrypto | added: structs 1 |
| `diff` | libc | structs: 4 → 3; functions: 23 → 21; callbacks: 10 → 0; macros: 29 → 28 |
| `diff` | libpcre2-8 | added: structs 1 |
| `filter` | libc | functions: 16 → 14; callbacks: 3 → 0 |
| `history` | libc | structs: 1 → 0; callbacks: 2 → 0 |
| `history` | libpcre2-8 | added: structs 1 |
| `index` | libc | structs: 4 → 2; callbacks: 4 → 0 |
| `llhttp` | libc | callbacks: 2 → 0; macros: 2 → 1 |
| `merge` | libc | structs: 3 → 2; functions: 20 → 19; callbacks: 4 → 0 |
| `ntlmclient` | libc | structs: 3 → 0 |
| `ntlmclient` | libcrypto | structs: 1 → 4 |
| `object` | libc | functions: 19 → 18; callbacks: 4 → 0 |
| `odb` | libc | structs: 10 → 9; callbacks: 7 → 0 |
| `refs` | libc | structs: 4 → 3; callbacks: 2 → 0 |
| `reftable` | libc | callbacks: 3 → 0 |
| `remote` | libc | functions: 8 → 7; callbacks: 5 → 0 |
| `repository` | libc | callbacks: 2 → 0 |
| `submodule` | libc | callbacks: 1 → 0 |
| `transport` | libc | structs: 19 → 7; callbacks: 17 → 0 |
| `transport` | libcrypto | structs: 2 → 12 |
| `util` | libc | structs: 18 → 12; functions: 114 → 113; callbacks: 8 → 0 |
| `util` | libcrypto | structs: 0 → 1 |
| `util` | libpcre2-8 | structs: 0 → 5 |
| `workdir` | libc | callbacks: 6 → 0 |
| `xdiff` | libc | structs: 1 → 0; callbacks: 3 → 0 |
| `xdiff` | libpcre2-8 | added: structs 1 |

## What moves downstream

The order of `subsystems.json` is cut from this graph; see
`crustify/reviews/planner/subsystems/order-from-rebuilt-graph/`. No schedule moves because of
these records: schedules come from each subsystem's own configuration.
