# baseline-cov — `git2-rs`, the existing safe libgit2 bindings

The prior art this campaign is measured against: `git2`, the community-maintained
safe wrapper over `libgit2-sys`. This file records its tests, their Rust and C
line coverage, its API coverage of libgit2, and the unsafe surface its public
interface exports.

Measured `2026-10-12`.

## 0. Subject

| | `git2-rs` |
|---|---|
| checkout | `~/Workspace/git/crustify-rs/git2-rs` |
| upstream | `github.com/rust-lang/git2-rs` |
| revision | `f6f1116` (`2026-10-05`) |
| describe | `libgit2-sys-0.18.5+1.9.4-224-gf6f1116` |
| crate version | `git2 0.21.0`; edition `2021`, MSRV `1.87` |
| sys crate | `libgit2-sys 0.18.7+1.9.6` (path dep) |
| libgit2 | **`v1.9.6`**, the `libgit2-sys/libgit2` submodule, compiled by `libgit2-sys`'s `build.rs` through `cc` and linked `static=git2` |
| features measured | `https,ssh` (CI's fullest Linux job), unless stated |

The clone was not modified; every build used a separate target directory.

**Scope.** Only the `git2` crate is the subject. `libgit2-sys` (generated
bindings plus `build.rs`), `git2-curl` and `systest` are not, and none of their
tests are counted.

Toolchain: `rustc 1.99.0` stable (LLVM `23.1.1`), `cargo-llvm-cov 0.6.21`,
`Ubuntu clang 23.1.2` for the C, CodeQL `2.26.3`, in a container derived from
`crustify:latest` (Ubuntu noble).

## 1. Tests

From `cargo test --locked -p git2 [--features ...] -- --list`, then a real
run. Every test passes in every configuration.

`src/` holds `245` `#[test]` functions. Which of them compile depends on cargo
features and the platform: `30` are gated out of the default and `https,ssh`
builds — `28` behind the opt-in `unstable-sha256` feature (libgit2's
experimental SHA-256 object format: `smoke_init_sha256`, `diff_sha256`,
`insert_write_sha256`, `smoke_write_object_format`, …) and `2` behind
`cfg(windows)` (`path_to_repo_path_translate` and the Windows variant of
`path_to_repo_path_no_absolute`). The third column turns the SHA-256 tests on;
the Windows pair never runs on this Linux host.

| target | kind | default | `https,ssh` | `https,ssh,unstable-sha256` | result (`https,ssh,unstable-sha256`) |
|---|---|---:|---:|---:|---|
| `src/lib.rs` | lib unit tests (`#[cfg(test)]`) | `204` | `215` | `243` | `243` passed |
| `tests/add_extensions.rs` | integration | `1` | `1` | `1` | passed |
| `tests/compile_fail.rs` | integration (`trybuild`, `1` case: `remote_refspec.rs`) | `1` | `1` | `1` | passed |
| `tests/end_to_end.rs` | integration | `5` | `5` | `5` | passed |
| `tests/get_extensions.rs` | integration | `1` | `1` | `1` | passed |
| `tests/global_state.rs` | integration | `1` | `1` | `1` | passed |
| `tests/remove_extensions.rs` | integration | `1` | `1` | `1` | passed |
| doctests | doc | `13` | `13` | `13` | `12` passed, `1` ignored |
| **total** | | **`227`** | **`238`** | **`266`** | **`265` passed, `1` ignored, `0` failed** |

The `15` `examples/` are built by `cargo test` but contain no tests and are not
run. The `11` extra unit tests under `https,ssh` are the credential and
transport tests the features gate in; the `28` more under `unstable-sha256` are
the SHA-256 variants of existing commit, diff, index, indexer, oid,
packbuilder, repository and transaction tests. `unstable-sha256` is
experimental and ABI-breaking (git2's `Cargo.toml` tells library authors never
to enable it by default), so `https,ssh` remains the headline configuration
below and the SHA-256 run is reported beside it.

Test LoC (blank and `//` lines excluded): `5,341` in inline `#[cfg(test)]`
modules under `src/`, `476` under `tests/`. Non-test `src/` is `15,649` LoC over
`64` files.

## 2. Test coverage

One instrumented run of every test target per configuration (doctests excluded: their coverage
needs nightly `-Zinstrument-coverage` for rustdoc, and is not counted). Rust is
instrumented by rustc; the bundled libgit2 is compiled by clang with the same
LLVM `23` source-based coverage, so both land in the same `.profraw` files and
one `llvm-profdata` (rustup's `llvm-tools`, LLVM `23.1.1`) merges them.

```sh
CC=/usr/lib/llvm-23/bin/clang \
CFLAGS="-fprofile-instr-generate -fcoverage-mapping" \
cargo llvm-cov --no-report --locked -p git2 --features https,ssh --include-ffi
# and again, into its own target dir, with --features https,ssh,unstable-sha256
cargo llvm-cov report --include-ffi --json --summary-only   # per-file totals
cargo llvm-cov report --include-ffi --lcov                  # per-line, for the non-test cut
```

That the C really was the instrumented bundled copy was checked on the build
output: `libgit2-sys` linked `static=git2` from its own `OUT_DIR` (no system
libgit2 is installed), and the archive's `repository.o` carries
`__llvm_covmap` / `__llvm_covfun` / `__llvm_prf_cnts` and a
`clang version 23.1.2` `.comment`. Counters from both compilers read back
non-zero through the one merged profile, so the raw formats agree.

### Rust — `git2`'s own `src/`

| scope | files | LoC | regions | functions |
|---|---:|---:|---:|---:|
| `src/`, non-test (inline `#[cfg(test)]` excluded) | `64` | **`61.24%`** (`6303`/`10293`) | — | **`57.21%`** (`1083`/`1893`) |
| `src/`, whole files (llvm-cov summary) | `64` | `71.11%` (`10278`/`14454`) | `74.97%` (`17731`/`23651`) | `62.60%` (`1366`/`2182`) |
| `src/`, non-test, `+unstable-sha256` | `64` | `61.34%` (`6332`/`10322`) | — | `57.29%` (`1085`/`1894`) |
| `src/`, whole files, `+unstable-sha256` | `64` | `71.84%` (`10661`/`14839`) | `75.71%` (`18611`/`24581`) | `63.19%` (`1399`/`2214`) |

The first row is the one to read: it is the authored non-test source, the basis
`results.md` uses. It is cut from the LCOV export by brace-matching every
`#[cfg(test)]` item, which removes `3,906` instrumented lines; functions are
counted once per definition site, not per monomorphisation. Regions have no
per-line form in LCOV, so they are given for whole files only.

`5` files have `0%` line coverage: `src/cert.rs`, `src/email.rs`,
`src/patch.rs`, `src/revert.rs`, `src/version.rs`.

The SHA-256 rows are `https,ssh` plus the feature: the feature compiles in
`29` more non-test lines (the `ObjectFormat` paths) and the `28` tests lift the
non-test line figure by `+0.10`pp; the cut removes `4,260` instrumented test
lines instead of `3,906`.

### C — the bundled libgit2 `v1.9.6`

| scope | files | zero-cov files | LoC | regions | functions |
|---|---:|---:|---:|---:|---:|
| `src/libgit2/` | `169` | `42` | **`52.04%`** (`29869`/`57398`) | `52.42%` (`34415`/`65653`) | `66.77%` (`2200`/`3295`) |
| `src/util/` | `65` | `19` | **`52.09%`** (`4456`/`8555`) | `52.24%` (`5620`/`10758`) | `69.20%` (`391`/`565`) |
| **libgit2 proper** | **`234`** | **`61`** | **`52.04%`** (`34325`/`65953`) | **`52.39%`** (`40035`/`76411`) | **`67.12%`** (`2591`/`3860`) |
| `src/libgit2/`, `+unstable-sha256` | `169` | `42` | `52.36%` (`30133`/`57552`) | `52.66%` (`34648`/`65801`) | `67.06%` (`2211`/`3297`) |
| `src/util/`, `+unstable-sha256` | `65` | `19` | `52.10%` (`4457`/`8555`) | `52.19%` (`5615`/`10758`) | `69.38%` (`392`/`565`) |
| **libgit2 proper, `+unstable-sha256`** | **`234`** | **`61`** | **`52.33%`** (`34590`/`66107`) | **`52.59%`** (`40263`/`76559`) | **`67.40%`** (`2603`/`3862`) |
| `deps/xdiff/` | `10` | `5` | `42.21%` (`957`/`2267`) | `39.81%` | `57.58%` (`57`/`99`) |
| `deps/pcre2/` | `23` | `16` | `12.11%` (`2025`/`16728`) | `8.50%` | `19.57%` (`27`/`138`) |
| `deps/llhttp/` | `4` | `4` | `0.00%` (`0`/`9513`) | `0.00%` | `0.00%` (`0`/`123`) |

Files are `.c` and the headers they instantiate code from. The denominator is
what `libgit2-sys` compiles, not the whole tree: `120` of the `131` `.c` files
under `src/libgit2/` and `32` of the `53` under `src/util/` (the rest are other
platforms' and other backends' variants), and no `src/cli/`. libssh2, zlib and
OpenSSL are outside the workspace and not reported. `llhttp` is the HTTP
parser behind the `https` transport, which no test drives against a server.

Under `unstable-sha256` `libgit2-sys` compiles the bundled libgit2 with
`GIT_EXPERIMENTAL_SHA256=1` (confirmed in its build output:
`rustc-cfg=libgit2_experimental_sha256`, `libgit2_vendored`), which adds the
SHA-256 object-format paths: the denominator grows by `154` lines and `2`
functions in `src/libgit2/` and is unchanged in `src/util/` and `deps/` (whose
covered counts move by at most one line, so their rows are not repeated). The `28` SHA-256 tests reach `+265` C lines,
`+0.29`pp on libgit2 proper.

## 3. API coverage

### Denominator

wavefront over a CodeQL database of libgit2 `v1.9.6`, with the config modeled on
the campaign's (`impl_files: src/`, `api_headers: include/`); the database was
built from a copy of the submodule under its own cmake
(`-DBUILD_TESTS=OFF -DDEPRECATE_HARD=OFF`):

```sh
codeql database create wf/codeql/db --language=cpp --source-root . \
  --command 'cmake -S . -B build-cql -DBUILD_TESTS=OFF -DDEPRECATE_HARD=OFF && cmake --build build-cql'
wavefront <libgit2> --config wf-config.json extract-ql
wavefront <libgit2> --config wf-config.json query types   --api-only                  # 251
wavefront <libgit2> --config wf-config.json query symbols --api-only --targeted-only  # 1,013
```

`v1.9.6` publishes `251` types (`159` structs, `91` enums, `1` typedef), `961`
exported functions and `52` callbacks: **`1,264`** non-macro entities. Its `142`
macros are excluded, as `results.md` excludes them. For scale, the campaign's
libgit2 revision publishes `1,317` (`260` types, `996` functions, `61`
callbacks).

### Numerator

The method of `ffmpeg-wrap/crustify/baseline.md` ("Method — API coverage"),
unchanged: a C entity is covered when `git2`'s `src/` names it as a whole word,
after removing `#[cfg(test)]` items (brace-matched, per file) and `//` comments.
It is an **upper bound**: a mention scores whether or not the entity is
reachable from the safe API. Dropping the two filters adds exactly `3` names
(`git_diff_similarity_metric`, `git_error_set`, `git_tag_foreach_cb`), all
mentioned only in tests or comments.

| | API total | covered | % |
|---|---:|---:|---:|
| types | `251` | `148` | `59.0%` |
| functions | `961` | `626` | `65.1%` |
| callbacks | `52` | `14` | `26.9%` |
| **all** | **`1,264`** | **`788`** | **`62.3%`** |

Split the way `results.md` splits the campaign's remainder:

| header set | types | functions | callbacks | all |
|---|---:|---:|---:|---:|
| public (`include/git2/*.h`, `deprecated.h` aside) | `140`/`218` | `587`/`781` | `14`/`44` | **`741`/`1,043` (`71.0%`)** |
| `include/git2/sys/*` | `7`/`31` | `12`/`112` | `0`/`8` | `19`/`151` (`12.6%`) |
| `include/git2/deprecated.h` | `1`/`2` | `27`/`68` | — | `28`/`70` (`40.0%`) |

`git2` still binds a good part of `deprecated.h`, because its credential code
calls the old `git_cred_*` spellings: `credential.h`, their replacement, is
untouched.

### By header, touched

`67` of the `86` headers that publish an entity are touched. Columns are API
totals, then `git2`'s covered counts.

| header | API types | API syms | types | syms |
|---|---:|---:|---:|---:|
| `deprecated.h` | `2` | `68` | `1` | `27` |
| `remote.h` | `13` | `53` | `7` | `36` |
| `diff.h` | `21` | `37` | `17` | `28` |
| `repository.h` | `6` | `48` | `2` | `36` |
| `index.h` | `7` | `44` | `2` | `30` |
| `config.h` | `5` | `45` | `3` | `34` |
| `types.h` | `44` | `0` | `38` | `0` |
| `refs.h` | `1` | `43` | `0` | `35` |
| `odb.h` | `3` | `36` | `1` | `27` |
| `submodule.h` | `2` | `35` | `1` | `28` |
| `commit.h` | `2` | `34` | `0` | `27` |
| `tree.h` | `3` | `33` | `2` | `29` |
| `merge.h` | `9` | `16` | `8` | `15` |
| `tag.h` | `0` | `23` | `0` | `19` |
| `oid.h` | `3` | `19` | `2` | `6` |
| `sys/transport.h` | `6` | `15` | `5` | `2` |
| `blob.h` | `2` | `17` | `0` | `6` |
| `branch.h` | `1` | `18` | `1` | `17` |
| `notes.h` | `1` | `18` | `0` | `12` |
| `pack.h` | `1` | `18` | `1` | `18` |
| `rebase.h` | `3` | `16` | `3` | `13` |
| `revwalk.h` | `1` | `18` | `0` | `16` |
| `blame.h` | `5` | `13` | `3` | `6` |
| `worktree.h` | `3` | `15` | `3` | `15` |
| `patch.h` | `1` | `15` | `1` | `14` |
| `pathspec.h` | `3` | `13` | `2` | `13` |
| `stash.h` | `5` | `10` | `3` | `8` |
| `status.h` | `5` | `10` | `3` | `6` |
| `object.h` | `0` | `14` | `0` | `11` |
| `reflog.h` | `0` | `13` | `0` | `13` |
| `attr.h` | `2` | `10` | `0` | `2` |
| `refspec.h` | `0` | `12` | `0` | `9` |
| `checkout.h` | `4` | `7` | `3` | `3` |
| `indexer.h` | `3` | `8` | `2` | `7` |
| `describe.h` | `4` | `6` | `3` | `4` |
| `sys/repository.h` | `0` | `10` | `0` | `4` |
| `odb_backend.h` | `6` | `3` | `2` | `0` |
| `transaction.h` | `0` | `8` | `0` | `8` |
| `apply.h` | `3` | `4` | `3` | `3` |
| `cert.h` | `6` | `1` | `4` | `0` |
| `common.h` | `2` | `5` | `0` | `3` |
| `annotated_commit.h` | `0` | `7` | `0` | `6` |
| `mailmap.h` | `0` | `7` | `0` | `6` |
| `signature.h` | `0` | `7` | `0` | `6` |
| `clone.h` | `2` | `4` | `2` | `1` |
| `message.h` | `2` | `3` | `1` | `3` |
| `sys/odb_backend.h` | `1` | `4` | `1` | `0` |
| `sys/refdb_backend.h` | `2` | `3` | `1` | `0` |
| `revparse.h` | `2` | `3` | `1` | `3` |
| `sys/mempack.h` | `0` | `5` | `0` | `3` |
| `cherrypick.h` | `1` | `3` | `1` | `2` |
| `errors.h` | `3` | `1` | `3` | `1` |
| `reset.h` | `1` | `3` | `1` | `2` |
| `revert.h` | `1` | `3` | `1` | `3` |
| `sys/errors.h` | `0` | `4` | `0` | `2` |
| `refdb.h` | `0` | `4` | `0` | `2` |
| `email.h` | `2` | `1` | `2` | `1` |
| `proxy.h` | `2` | `1` | `2` | `0` |
| `trace.h` | `1` | `2` | `1` | `1` |
| `graph.h` | `0` | `3` | `0` | `2` |
| `ignore.h` | `0` | `3` | `0` | `3` |
| `buffer.h` | `1` | `1` | `1` | `1` |
| `net.h` | `2` | `0` | `2` | `0` |
| `oidarray.h` | `1` | `1` | `1` | `0` |
| `strarray.h` | `1` | `1` | `1` | `0` |
| `transport.h` | `0` | `2` | `0` | `1` |
| `sys/email.h` | `0` | `1` | `0` | `1` |

Headers are under `include/git2/`; an entity is attributed to the first public
header that declares it.

### Untouched headers

`19` headers, `143` entities, are named nowhere in `git2`'s non-test source
(entity count in parentheses):

`credential.h` (`24`), `sys/filter.h` (`17`), `filter.h` (`15`),
`sys/commit_graph.h` (`13`), `sys/index.h` (`13`), `sys/merge.h` (`12`),
`sys/config.h` (`7`), `sys/midx.h` (`7`), `sys/hashsig.h` (`6`),
`sys/stream.h` (`6`), `sys/diff.h` (`5`), `sys/alloc.h` (`3`), `sys/path.h` (`3`),
`sys/commit.h` (`3`), `credential_helpers.h` (`2`), `sys/remote.h` (`2`),
`global.h` (`2`), `sys/refs.h` (`2`), `sys/openssl.h` (`1`).

Most are the `sys/` backend-authoring interfaces the campaign also held back.
The public exceptions are `filter.h` (the filter API in full), `credential.h`
(bound through `deprecated.h` instead, above) and `global.h`.

## 4. Unsafe surface

`crustify scan-unsafe` (the rustc driver at crustify `HEAD` of `2026-10-12`).
`scan-unsafe` builds with default features, so the `https,ssh` figures come
from the same driver run under those features through the same
`driver._collect_emissions`, with the per-crate emission lines kept. Exactly one
crate emitted, `git2`: the workspace root is the `git2` package and declares no
members, so `libgit2-sys` and the workspace's other packages are never primary
and never measured.

| metric | `https,ssh` | default |
|---|---:|---:|
| `code_lines` (HIR denominator) | `14,178` | `13,915` |
| `fns_pub` / `fns_priv` | `2,234` / `939` | `2,229` / `930` |
| `unsafe_fns` | `124` | `124` |
| ...exported (`_pub_seam` + `_pub_smell`) | **`112`** (`90` + `22`) | `112` (`90` + `22`) |
| ...private | `12` | `12` |
| raw-pointer positions, args / rets / fields | `177` / `87` / `74` | same |
| ...exported (`raw_ptr_pub`) | **`136`** | `136` |
| ...private (`raw_ptr_priv`) | `202` | `202` |
| `*c_void` positions | `36` | `36` |
| `unsafe_blocks` | `949` | `932` |
| `unsafe_block_code_lines` | `3,505` (`24.7%` of `code_lines`) | `3,497` |
| `ffi_calls` | `672` | `672` |
| `unsafe_impls` | `8` | `8` |
| `raw_ptr_derefs` | `291` | `291` |
| `undocumented_unsafe` (lexical) | — | `929` of `940` sites |

`git2` writes almost no `// SAFETY:` comments (`11` of `940` lexical unsafe
sites), so `undocumented_unsafe` is close to the full count.

### Seam and smell, by `git2`'s own conventions

The seam/smell split rests on the seam names the scan is given
(`scan-unsafe --seams`); the scanner assumes none. These are `git2`'s own,
read off its code: every exported `unsafe fn` and raw-pointer position was
listed by name from a scan with no seam file, and the names that convert
between a safe wrapper and a libgit2 pointer were kept. The file
(`/data/marius.momeu/baseline-cov/seams/git2.txt`) lists, one per line:

| seam fn | what it is |
|---|---|
| `from_raw`, `from_raw_opt`, `raw` | the `Binding` trait (`src/util.rs`): adopt / expose the libgit2 pointer; implemented on every wrapper |
| `unwrap` | `Cred::unwrap`: surrender the raw `git_cred` |
| `configure` | `CheckoutBuilder::configure`: fill a raw `git_checkout_options` |

The C-ABI gateway (`extern "C"` / `#[no_mangle]` fns) is seam by structure,
whatever the file says.

| | `https,ssh` | default |
|---|---:|---:|
| exported `unsafe fn`: seam / smell | `90` / **`22`** | `90` / `22` |
| private `unsafe fn`: seam / smell | `5` / `7` | `5` / `7` |
| exported raw-pointer positions: seam / smell | `136` / **`0`** | `136` / `0` |
| private raw-pointer positions: seam / smell | `110` / `92` | `110` / `92` |
| exported `*c_void` smell | `0` | `0` |

Every exported raw pointer is behind `Binding`. The `22` exported `unsafe fn`
left as smell are not conversions but process-wide state the caller must
synchronise: the `21` `opts::*` getters and setters (`set_search_path`,
`set_mwindow_size`, `set_ssl_cert_file`, `set_server_timeout_in_milliseconds`,
…) and `transport::register`. That is the obligation `git2` exports.

## 5. Sanitizers

| sanitizer | tests targeting it |
|---|---:|
| ASan | `0` |
| MSan | `0` |
| TSan | `0` |
| UBSan | `0` |
| Miri | `0` |
| BorrowSanitizer | `0` |

Evidence: CI (`.github/workflows/main.yml`) runs only plain `cargo test` in
four feature combinations, `cargo test -p git2-curl`, `systest`, clippy and
rustfmt. A search of every `.rs`, `.yml`, `.toml`, `.sh` and `.md` file outside
the libgit2 submodule for `miri`, `sanitiz`, `asan`, `tsan`, `msan`, `ubsan`,
`bsan` and `-Zsanitizer` finds nothing: no `cfg(miri)`, no `-Zsanitizer`
build, no sanitizer job. The only sanitizer jobs in the tree are libgit2's own
C CI (`libgit2/.github/workflows/main.yml`, MSan / ASan / TSan / UBSan over its
C test suite), which exercises none of `git2`'s Rust.

## 6. Against this campaign

Figures from the earlier campaign's `libgit2-wrap/crustify/results.md` (branch `crustify/src-gpt-5.6-sol`), restated (not re-measured); this campaign's own `results.md` is not compared yet. The C differs: the campaign
wraps libgit2 at `0551dfd4` (a `main` revision, `1,317` published non-macro
entities); `git2` builds `v1.9.6` (`1,264`). The percentages are each against
their own revision's API.

| | crustify `libgit2-wrap` | `git2` |
|---|---:|---:|
| API coverage, non-macro | `1,129`/`1,317` (`85.7%`) | `788`/`1,264` (`62.3%`, upper bound) |
| types | `230`/`260` | `148`/`251` |
| functions | `854`/`996` | `626`/`961` |
| callbacks | `45`/`61` | `14`/`52` |
| non-test Rust LoC | `24,928` | `15,649` (`src/`) |
| test Rust LoC | `16,799` | `5,817` (`5,341` inline + `476` `tests/`) |
| `code_lines` (HIR) | `24,928` | `14,178` |
| `unsafe_fns` | `998` | `124` |
| raw-pointer positions | `1,166` | `338` |
| `ffi_calls` | `919` | `672` |
| tests targeting a sanitizer | see `results.md` | `0` |

The campaign's figure is anchor-based (a filled `Wraps:` / `Replaces:` on an
item), not a mention scan, so it is the stricter measure of the two. The
`unsafe_fns` and raw-pointer rows are scale, not quality: the campaign's
surface is mostly seam by construction (`781` of `998` unsafe fns, `1,147` of
`1,166` positions), which is the split section 4 says not to apply to `git2`.

## 7. Notes and limitations

- **Doctests are not in the coverage.** Their `13` tests run, but `cargo
  llvm-cov` on stable instruments only the test targets.
- **The C denominator is the compiled subset.** Files `libgit2-sys` never
  compiles (`src/cli/`, other platforms, other hash and HTTPS backends) are
  absent from the report, not counted as uncovered.
- **Feature- and platform-gated tests.** The headline `https,ssh` figures leave
  out the `28` `unstable-sha256` tests, reported in their own column and rows;
  the `2` `cfg(windows)` tests run in no configuration here.
- **Inline-test exclusion is textual.** `#[cfg(test)]` items are found by
  brace matching on source. A brace inside a string or char literal in a test
  module would mis-end it. The `3,906` instrumented lines removed and the
  `5,341` test LoC are consistent with the module sizes.
- **API coverage is an upper bound**, per the method. It is version-blind
  within `src/` (feature-gated code counts) and is scored against `v1.9.6`, not
  the campaign's revision.
- **Large state** (instrumented build, profiles, CodeQL database, scripts) is
  under `/data/marius.momeu/baseline-cov/git2/`: `apicov.py` (API scan),
  `lcov_nontest.py` (non-test coverage cut), `bucket.py` (per-directory
  coverage), `scan_features.py` (feature-aware scan).
