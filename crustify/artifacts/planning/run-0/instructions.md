# Planner run: re-plan a resumed campaign under the current schemas

This campaign is resumed: it started under the schema-v1 plan and has landed nine
subsystems (`raw-lifetime-void`, `raw-lifetime-string`, `util`, `oid`, `core`,
`repository`, `object`, `config`, `index`), all promoted to `crustify/main`. The user
has decided how the plan carries that history forward. Four of the points below depart
from the planning instructions on purpose; apply them as written and do not reconcile
them away.

## Foundational tiers (departure)

Keep the two landed foundational tiers as they are: `raw-lifetime-void` and
`raw-lifetime-string`, with their names, their single `raw-lifetime` items and their
home `crustify/rust/libgit2/libgit2/raw.rs`. Do not introduce `void` or `string`
subsystems, select new foundational members, or submit lifecycle roles for them.
List the two tiers in `subsystems.json` as the first entries of the `libgit2` link unit,
at indices 0 and 1, and carry their landed schedule over to schema v2 unchanged,
`raw-lifetime` kind included, although it is not a kind of the current `task.json`.

## Composition is fixed (departure)

Keep every subsystem `subsystems.json` lists, and the composition of each:
its `implementation_files`, `api_headers` and objective. Do not split, merge, add or
remove subsystems, or move files between them. Bring the entries up to the current
schema (add `index` to the link unit and every subsystem, and anything else it lacks),
and correct counters and `imported_deps` that the evidence contradicts.

## Order and schedules follow the rebuilt DAG (departure, for done subsystems)

Write the T3 tables and rebuild the DAGs as your wavefront guidance describes. Then:

- order the subsystems from the campaign-wide graph, the two foundational tiers kept
  first; and
- emit every scheduled subsystem's `schema_version: 2` `schedule.json` from its
  rebuilt DAG, the landed subsystems included: their waves and batches may change.

The landed subsystems' schedules then describe their shape under the current graph. Their
work is not relaunched, and their landed artifact dirs under
`crustify/artifacts/libgit2/<subsystem>/wave-*/` stay as they are; do not edit or move
them. Name in your commit message each landed subsystem whose schedule now differs from
what landed. File the order changes and the schedule changes of the landed subsystems as
plan corrections, and state in each report what moved.

Remove the campaign-wide schema-v1 `crustify/artifacts/schedule.json` once every
scheduled subsystem has its own.

## Subsystems without a schedule (departure)

`llhttp`, `ntlmclient`, `xdiff` and `reftable` publish nothing on the public API (no
`api_headers`), and this is a wrap campaign scheduled with `--api-headers-only`. Keep them
in `subsystems.json`, with their counters and dependencies, and emit no `schedule.json`
for them.
