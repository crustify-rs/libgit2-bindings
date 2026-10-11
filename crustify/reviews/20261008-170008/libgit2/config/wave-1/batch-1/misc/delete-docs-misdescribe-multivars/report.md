# The delete and multivar wrappers' docs described the wrong operations

- **items**: `git_config_delete_entry`, `git_config_delete_multivar`,
  `git_config_set_multivar` (symbols, wave 1 batch 1, `5a0f495b6`)
- **affected commit**: `d79771f6f`
- **class**: documentation defect

## Finding

The three wrappers shared one copy-pasted doc comment:

- `delete_entry` said "Deletes the entry or matching multivar values in the writer
  backend ... replacement values are copied". It deletes no multivar values: the file
  backend's `config_file_delete` looks the key up with `git_config_list_get_unique`. On
  a multivar that fails with "entry is not unique due to being a multivar", and nothing
  is deleted. It also takes no replacement value.
- `delete_multivar` said "Deletes the entry or matching multivar values" and also
  mentioned replacement values, which it doesn't take.
- `set_multivar` said "Replaces matching values or appends when none match", which is
  correct, but not which value is appended or that the regexp selects it.

A caller relying on the `delete_entry` doc would expect it to clear a multivar and get an
error instead.

## Fix

Each doc now describes its own operation. `delete_entry` deletes a single-valued entry
and points to `delete_multivar` for multivars. `delete_multivar` deletes the values of a
multivar that match `regexp`. `set_multivar` replaces the values that match `regexp` or
appends `value`. Both delete docs also describe the read-only failure; see
`../delete-readonly-error-is-stale`. The unit regression for that report also checks
that `delete_entry` refuses a multivar and that `delete_multivar` deletes one of its
values.
