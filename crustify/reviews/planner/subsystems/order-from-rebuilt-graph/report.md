# Subsystem order recut from the rebuilt campaign-wide graph

## What the earlier plan decided

The schema-v1 `crustify/artifacts/schedule.json` ordered the `libgit2` link unit's
subsystems as (`index` in parentheses):

```text
raw-lifetime-void (0), raw-lifetime-string (1), util (2), oid (3), core (4), repository (5),
object (6), config (7), index (8), refs (9), odb (10), diff (11), attr (12), workdir (13),
history (14), remote (15), filter (16), merge (17), submodule (18), transport (19)
```

`subsystems.json` listed the 22 subsystems without indices, in the order `llhttp, ntlmclient,
util, oid, core, repository, object, config, index, refs, odb, diff, attr, workdir, history,
remote, xdiff, filter, merge, reftable, submodule, transport`.

## Evidence against it

The order is a DAG cut from the subsystem graph: within each strongly connected component,
the member the most other members depend on comes first, and net edges between tied
members break ties (`docs/planning.md`, Order). The campaign-wide default graph, with the T3
tables this run wrote, is one component of 20 subsystems that cuts as follows:

```sh
PYTHONHASHSEED=0 /opt/venv/bin/python \
  crustify/reviews/planner/subsystems/imported-deps-recount/subsystem_graph.py order
```

```text
order: llhttp ntlmclient util oid reftable xdiff core repository config object index attr filter refs diff odb workdir history merge remote transport submodule
  component of 20: util first, 19 members depend on it
  component of 16: core first, 15 members depend on it; tied net edges {'repository': -46, 'core': 46}
  component of 15: repository first, 14 members depend on it
  component of 14: config first, 12 members depend on it; tied net edges {'config': 10, 'object': -10}
  component of 13: object first, 12 members depend on it
  component of 12: index first, 8 members depend on it
  component of 9: refs first, 6 members depend on it
  component of 8: diff first, 5 members depend on it; tied net edges {'odb': 12, 'diff': 89, 'workdir': -101}
  component of 7: odb first, 4 members depend on it; tied net edges {'odb': 64, 'history': -71, 'workdir': 7}
  component of 6: workdir first, 4 members depend on it
  component of 2: history first, 1 members depend on it; tied net edges {'merge': -89, 'history': 89}
  component of 2: remote first, 1 members depend on it; tied net edges {'remote': 72, 'transport': -72}
cut: 43 records, 780 edges
```

The earlier order breaks that rule in several places, and cuts more of the same graph:

```sh
PYTHONHASHSEED=0 /opt/venv/bin/python \
  crustify/reviews/planner/subsystems/imported-deps-recount/subsystem_graph.py cut \
  llhttp ntlmclient util oid core repository object config index refs odb diff attr workdir \
  history remote xdiff filter merge reftable submodule transport
```

reports `cut: 51 records, 1006 edges`. The records only the earlier order cuts are
`refs → reftable` (142 edges), `diff → xdiff` (26), `workdir → filter` (18), `diff → filter`
(15), `diff → attr` (11), `object → config` (10), `history → xdiff` (9), `odb → xdiff` (6) and
`remote → merge` (1). The new order cuts only `diff → odb` (12) in their place. `reftable` and
`xdiff` depend on nothing but `util`, yet were placed after their consumers. `object` and `config` tie at 12 dependents, and `config` wins the tie by
net edges (`object → config` 10, `config → object` 0).

## The edit

`subsystems.json` now lists the link unit and every subsystem with its `index`, in this order:

```text
raw-lifetime-void (0), raw-lifetime-string (1), llhttp (2), ntlmclient (3), util (4), oid (5),
reftable (6), xdiff (7), core (8), repository (9), config (10), object (11), index (12),
attr (13), filter (14), refs (15), diff (16), odb (17), workdir (18), history (19), merge (20),
remote (21), transport (22), submodule (23)
```

The two foundational tiers stay first, as the run's instructions require. `llhttp`,
`ntlmclient`, `xdiff` and `reftable` have no schedule (they publish no API), so their place
orders nothing that runs.

## What moves downstream

- Among the landed subsystems, `config` now precedes `object`; they landed in the earlier
  order (`object`, then `config`). Their work is not relaunched, and neither schedule changes
  because of this move. Ownership changes do alter them; see the subsystems' own reports.
- Among the pending subsystems, the next to run are `attr`, `filter`, `refs`, `diff`, `odb`,
  `workdir`, `history`, `merge`, `remote`, `transport`, `submodule`. In the earlier order,
  `refs`, `odb` and `diff` ran before `attr`, `filter` moved from 16th to 14th place, and
  `transport` (last before) now precedes `submodule`.
- The ownership of the entities of the shared public headers follows the first-scheduled
  sharer. It is unchanged: `core` for `include/git2/types.h`, `repository` for
  `include/git2/repository.h`, `diff` for `include/git2/sys/diff.h`. `include/git2/notes.h`
  keeps its callback in `history`; see `crustify/wavefront/configs/libgit2/index/`.
- No schedule is re-emitted because of the order alone.
