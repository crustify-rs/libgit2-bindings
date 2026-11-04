# Remaining C surface of the wrapped types

Measured at `3db4b351da1` on `crustify/devel` (2026-11-04).

`ptr-positions.json` counts every position a wrapped C type, one with a `ty::` anchor in
`src/`, still holds in the public C surface the campaign has not wrapped:

- `signature_positions`: the parameters (`_arg`) and returns (`_ret`) of public functions
  without a `fn::` anchor;
- `field_positions_direct`: the fields of public structs and unions without a `ty::` anchor;
- `field_positions_nested`: the wrapped types reached through an unwrapped struct embedded by
  value, as `AVStream.attached_pic.buf` (`nested_examples` lists a few of each kind).

Each counter is split by the wrapped type's kind (`struct`, which covers unions, or `enum`)
and by how the position holds it: `ptr` behind a pointer, which a consumer crosses with a
seam (`as_ptr`, `from_ptr`) and an `unsafe` call; `val` by value, which needs a conversion;
and `cb`, a callback field whose signature names it. `per_type` gives the counts per wrapped
type, `distinct_fns` and `distinct_structs` how many functions and structs hold them, and
`unwrapped_api_fns` the public functions without an anchor.

Public means declared by a header the configuration lists in `api_headers`; a struct's
fields count only when it is defined there, so opaque types' fields are not counted. Pointers
are not followed: a field pointing to an unwrapped struct that holds a wrapped type is not
counted.

## Inputs

- `api-symbols.tsv` and `api-types.tsv`, wavefront's public inventory through
  `wavefront-config.json` here, the campaign-wide configuration converted to the installed wavefront's schema (the committed ones predate it):

  ```bash
  wavefront --config $PWD/crustify/artifacts/eval/wavefront-config.json $PWD/c query symbols --api-only > crustify/artifacts/eval/api-symbols.tsv
  wavefront --config $PWD/crustify/artifacts/eval/wavefront-config.json $PWD/c query types --api-only > crustify/artifacts/eval/api-types.tsv
  ```

- the T1/T2 tables under `crustify/wavefront/codeql/`, gitignored.

## Command

```bash
python3 -I crustify/scripts/eval/ptr_positions.py "$PWD" crustify/artifacts/eval/api-symbols.tsv \
    crustify/artifacts/eval/api-types.tsv crustify/artifacts/eval/wavefront-config.json crustify/artifacts/eval/ptr-positions.json
```
