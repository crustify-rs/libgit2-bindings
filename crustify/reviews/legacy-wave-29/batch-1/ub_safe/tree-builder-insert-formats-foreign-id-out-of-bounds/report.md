# `TreeBuilderMut::insert` hands libgit2 an id of another type than the repository's, which its object lookup formats out of bounds

- **item**: `git_treebuilder_insert` (symbol), `crustify/rust/libgit2/libgit2/object/tree.rs`
- **affected**: `TreeBuilderMut::insert` and `TreeBuilderMut::insert_entry`, for every
  file mode libgit2 looks the object up for (`Blob`, `BlobExecutable`, `Link`, `Tree`)
- **affected commit**: `84c213038` (libgit2/object wave 1 batch 7; reviewed at the
  review-wave base `38e6d1b3e`)
- **class**: UB reachable from the safe API — a heap buffer overflow (AddressSanitizer),
  a write in `git_oid_pathfmt`; the converse case reads uninitialised heap bytes
- **reproducer**: `src/main.rs` (`#![forbid(unsafe_code)]`; ASan, run as described in its
  header; aborts on the affected commit, exits 0 after the fix)

## Finding

`git_treebuilder_insert` checks every entry with `check_entry` (`src/libgit2/tree.c:484`):
the file mode, the name, the zero id, then, for any mode but `GIT_FILEMODE_COMMIT`
and while strict object creation is on (the default), `git_object__is_valid(repo, id,
type)`, which reads the object's header from the object database. Nothing on that
path compares the id's type with the repository's. The loose backend then formats the
id into the object's path (`object_file_name`, `src/libgit2/odb_loose.c:82`): it grows
the path buffer by `be->oid_hexsize + 2` bytes, the repository's hex size, but
`git_oid_pathfmt` writes as many digits as the *id's* type has. For a SHA-256 id in a
SHA-1 repository that is 65 bytes into room for 42; whenever the objects directory's
path is shorter than ~80 bytes (any temporary directory, so the buffer's 1.5× growth
slack does not absorb the excess), the write runs past the allocation. For a SHA-1 id
in a SHA-256 repository, 23 bytes of the path stay unwritten and are read by `stat`.

The crate refuses ids of another type than the repository's everywhere else an id
reaches an object lookup (`object::object::id_fits`: `Object::lookup*`,
`Tree/Tag/Blob/Commit::lookup*`, `TreeEntryRef::to_object`, `extract_signature`), and
the module docs of `object.rs` describe exactly this out-of-bounds formatting. Batch 7
did not apply it to the builder: its docs said "the id is stored as it is, so an id of
another type than the repository's makes the builder write a malformed tree", which is
true only of the modes libgit2 does not look up. One safe call reaches it:

```rust
let repo = Repository::init(c"/tmp/tbi", true)?;              // SHA-1
let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256)?;
TreeBuilder::new(repo.as_ref())?.as_mut().insert(c"file", wide.as_ref(), FileMode::Blob);
```

```
==1601315==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x6e17ba9f96d8 at pc 0x71a7bc259572 bp 0x7ffff9048b10 sp 0x7ffff9048b08
WRITE of size 1 at 0x6e17ba9f96d8 thread T0
    #0 0x71a7bc259571 in git_oid_fmt_substr /target/src/libgit2/oid.h:133:10
    #1 0x71a7bc259756 in git_oid_pathfmt /target/src/libgit2/oid.c:153:2
    #2 0x71a7bc24d836 in object_file_name /target/src/libgit2/odb_loose.c:94:2
    #3 0x71a7bc24b49f in locate_object /target/src/libgit2/odb_loose.c:466:14
    #4 0x71a7bc24b49f in loose_backend__read_header /target/src/libgit2/odb_loose.c:608:6
    #5 0x71a7bc24555a in odb_read_header_1 /target/src/libgit2/odb.c:1007:11
    #6 0x71a7bc24508e in git_odb__read_header_or_object /target/src/libgit2/odb.c:1050:10
    #7 0x71a7bc244cf6 in git_odb_read_header /target/src/libgit2/odb.c:968:10
    #8 0x71a7bc23ed0b in git_object__is_valid /target/src/libgit2/object.c:598:12
    #9 0x71a7bc33ec9e in check_entry /target/src/libgit2/tree.c:496:7
    #10 0x71a7bc33e7af in git_treebuilder_insert /target/src/libgit2/tree.c:791:15
    #11 0x5903bb683000 in <libgit2::libgit2::object::tree::TreeBuilderMut>::insert_entry libgit2/libgit2/object/tree.rs:1661:13
    #12 0x5903bb683317 in <libgit2::libgit2::object::tree::TreeBuilderMut>::insert libgit2/libgit2/object/tree.rs:1622:14
    #13 0x5903bb67f6f9 in tree_builder_insert_formats_foreign_id_out_of_bounds::main src/main.rs:47:35
    ...
0x6e17ba9f96d8 is located 0 bytes after 72-byte region [0x6e17ba9f9690,0x6e17ba9f96d8)
allocated by thread T0 here:
    #0 0x5903bb64c061 in realloc
    #1 0x71a7bc10158a in git__realloc /target/src/util/alloc.h:29:12
    #2 0x71a7bc10158a in git_str_try_grow /target/src/util/str.c:86:12
    #3 0x71a7bc24d7af in object_file_name /target/src/libgit2/odb_loose.c:91:6
    ...
SUMMARY: AddressSanitizer: heap-buffer-overflow (/target/crustify/builds/asan-ubsan/libgit2.so.1.9+0x2cb571)
```

The existing `tests/ub_safe/tree.rs` suites inserted only SHA-1 ids into SHA-1
repositories, and seeded foreign-type trees through `from_tree`, which copies entries
without a lookup, so none reached the path.

## Fix

`TreeBuilderMut::insert_entry` (and so `insert`) now runs `id_fits` against the
builder's repository before calling libgit2 whenever libgit2 would look the object up:
any mode but `Commit` and `Unreadable` (refused by libgit2 first), and a non-zero id
(likewise refused first), so libgit2's own refusals keep their errors. The refusal is
libgit2's wording for the mismatch, code `-1`, `GIT_ERROR_INVALID` "specified object ID
type does not match repository object ID type", as every other lookup wrapper gives.

libgit2 publishes no accessor for a builder's repository, so the new crate-private
`TreeBuilderRef::repository` reads `git_treebuilder`'s first member, `git_repository
*repo` (`src/libgit2/tree.h:33`, the revision the crate links), and returns the
`RepositoryRef<'r>` every builder is built from. The unit test
`builder_routines::builders_name_the_repository_they_were_built_for` pins that layout
assumption for empty and seeded builders of both id types.

Divergences, documented on `insert` and in `tests/equiv/tree.rs`: a foreign-type id is
refused before libgit2's check of the entry's name (an invalid name with such an id
reports the type mismatch), and also when strict object creation is turned off, where
libgit2 would store it unchecked (and write a malformed tree). A `Commit` entry's id
keeps libgit2's behaviour: stored as it is, never looked up.

Regression tests:

- `tests/ub_safe/tree.rs::routines::ids_of_another_type_are_refused_before_the_lookup`
  (ASan): every looked-up mode, both insert forms, a new and an existing name, the
  builder left as it was; the mode and zero-id refusals and a `Commit` entry keep
  libgit2's answers; the converse through a SHA-256 repository's builder. With the
  check removed it aborts with the heap-buffer-overflow above; with it the tree suites
  are ASan-clean.
- The reproducer was also run alone under BSan on the fixed tree (clean).
