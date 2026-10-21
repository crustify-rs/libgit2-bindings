"""Emit one subsystem's schema-v2 schedule.json the way planner run 0 did.

Run from the repository root:

    python3 crustify/reviews/planner/subsystems/repository-schedule-type-ownership/emit_schedule.py <subsystem> [out]

It runs wavefront on the subsystem's own configuration, selecting every file the subsystem
lists, with --transitive --api-headers-only and the campaign's workload caps, then converts the
result: one `wrap` batch per wavefront batch; an item keeps the home the schema-v1 plan gave it
(read from a8b28cbd0:crustify/artifacts/schedule.json), and a new item homes in the module of the
subsystem's public header that publishes it. The callable API macros, which wavefront never makes
units, keep the v1 batch and wave, unless the graph puts their dependencies later.
"""
import csv, json, collections, subprocess, sys, tempfile, os
R = '/target/'
V1_COMMIT = 'a8b28cbd0'
T='/target/crustify/wavefront/codeql/t1/'
decl=collections.defaultdict(set)
for r in csv.DictReader(open(T+'functions.csv')):
    decl[(r['name'],r['def_file'])] |= set(filter(None,r['decl_files'].split('|'))) | {r['def_file']}
for r in csv.DictReader(open(T+'types.csv')):
    k=(r['name'],r['def_file'] or r['decl_files'].split('|')[0])
    decl[k] |= set(filter(None,r['decl_files'].split('|'))) | ({r['def_file']} if r['def_file'] else set())
for r in csv.DictReader(open(T+'globals.csv')):
    decl[(r['name'],r['def_file'])] |= set(filter(None,r['decl_files'].split('|')))|{r['def_file']}
BASE='crustify/rust/libgit2/libgit2/'
def header_mod(h):
    assert h.startswith('include/git2/') and h.endswith('.h'), h
    return h[len('include/git2/'):-2]
def home(sub, api_headers, name, defined_in):
    """Home: the module of the subsystem's public header that publishes the item."""
    hs = sorted(h for h in decl.get((name, defined_in), set()) | {defined_in} if h in api_headers)
    if not hs: return None, []
    # prefer the header defining it, then a non-sys header, then the first
    if defined_in in hs: h = defined_in
    else:
        non_sys=[h for h in hs if '/sys/' not in h]
        h = (non_sys or hs)[0]
    return BASE + sub + '/' + header_mod(h) + '.rs', hs

SUBS = {s['name']: s for s in json.load(open(R + 'crustify/artifacts/subsystems.json'))['link_units'][0]['subsystems']}
v1 = json.loads(subprocess.run(['git', '-C', R, 'show', V1_COMMIT + ':crustify/artifacts/schedule.json'],
                               capture_output=True, text=True, check=True).stdout)
V1 = {s['name']: s for s in v1['link_units'][0]['subsystems']}
v1home = {(it['name'], it['defined_in']): it['home'] for s in V1.values() for w in s['waves'] for b in w['batches'] for it in b['items']}
# Symbols libgit2.so does not export (nm -D), which the v1 plan left out too.
UNEXPORTED = ['git_stdalloc_init_allocator', 'git_win32_crtdbg_init_allocator', 'git_config_backend_from_string',
              'git_config_backend_from_values', 'git_blame_file_from_buffer', 'git_transport_dummy']
# Callbacks wavefront also schedules in a config whose API struct fields name them, although
# another subsystem owns them (a wavefront defect); skipped in the borrowing subsystem.
BORROWED = {'merge': ['git_commit_create_cb', 'git_repository_fetchhead_foreach_cb', 'git_repository_mergehead_foreach_cb'],
            'remote': ['git_packbuilder_progress'], 'index': ['git_note_foreach_cb']}

def wavefront(sub):
    s = SUBS[sub]
    out = tempfile.mktemp(suffix='.json')
    subprocess.run(['/opt/venv/bin/wavefront', R, '--config', f'crustify/wavefront/configs/libgit2/{sub}/wavefront-config.json',
                    'schedule', '--output', out, '--file', *s['implementation_files'], *s['api_headers'],
                    '--transitive', '--api-headers-only', '--max-types', '2', '--min-fields', '20',
                    '--max-syms', '25', '--max-loc', '500', '--skip', *UNEXPORTED, *BORROWED.get(sub, [])],
                   check=True, cwd=R, stdout=subprocess.DEVNULL)
    return json.load(open(out))

# callable API macros wavefront never makes units; each is placed after the units its body names
MACROS = {
    'attr': (['GIT_ATTR_HAS_VALUE', 'GIT_ATTR_IS_FALSE', 'GIT_ATTR_IS_TRUE', 'GIT_ATTR_IS_UNSPECIFIED'], 'include/git2/attr.h', ['git_attr_value', 'git_attr_value_t']),
    'index': (['GIT_INDEX_ENTRY_STAGE', 'GIT_INDEX_ENTRY_STAGE_SET'], 'include/git2/index.h', ['git_index_entry']),
    'submodule': (['GIT_SUBMODULE_STATUS_IS_INDEX_UNMODIFIED', 'GIT_SUBMODULE_STATUS_IS_UNMODIFIED', 'GIT_SUBMODULE_STATUS_IS_WD_DIRTY', 'GIT_SUBMODULE_STATUS_IS_WD_UNMODIFIED'], 'include/git2/submodule.h', ['git_submodule_status_t']),
}

def item(sub, it):
    k = (it['name'], it['defined_in'])
    h = v1home.get(k) or home(sub, set(SUBS[sub]['api_headers']), *k)[0]
    assert h, k
    return {'name': it['name'], 'defined_in': it['defined_in'], 'kind': it['kind'],
            'field_anchors': list(it['field_anchors']) if it['kind'] == 'type' else [], 'home': h}

def emit(sub):
    wf = wavefront(sub)
    waves = [[{'objective': 'wrap', 'items': [item(sub, it) for it in b['items']]} for b in w['batches']] for w in wf['waves']]
    if sub in MACROS:
        names, hdr, deps = MACROS[sub]
        after = max(i for i, w in enumerate(waves) for b in w for it in b['items'] if it['name'] in deps) + 1
        # v1's batch of these macros, in its wave and order, unless the graph now needs it later
        v1b = [(w['index'], b) for w in V1[sub]['waves'] for b in w['batches'] if any(it['name'] in names for it in b['items'])]
        (v1w, b), = v1b
        at = max(after, v1w)
        while at >= len(waves): waves.append([])
        waves[at].append({'objective': 'wrap', 'items': [dict(it) for it in b['items']]})
    return {'schema_version': 2, 'waves': [{'index': i, 'batches': w} for i, w in enumerate(waves)]}


if __name__ == '__main__':
    sub = sys.argv[1]
    doc = json.dumps(emit(sub), indent=2) + '\n'
    if len(sys.argv) > 2: open(sys.argv[2], 'w').write(doc)
    else: sys.stdout.write(doc)
