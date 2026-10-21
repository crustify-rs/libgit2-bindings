"""Counters of every subsystem, as docs/artifacts/subsystems.md defines them.

Run from the repository root, after decoding ql/aggregates.ql against the campaign's database:

    codeql query run --database crustify/wavefront/codeql/db --output /tmp/agg.bqrs \
        crustify/reviews/planner/subsystems/counters-recount/ql/aggregates.ql
    codeql bqrs decode --format=csv --output /tmp/agg.csv /tmp/agg.bqrs
    python3 crustify/reviews/planner/subsystems/counters-recount/counters.py /tmp/agg.csv

Structs, unions and enums are the aggregate definitions CodeQL locates (named and anonymous,
typedef'd or embedded); functions, globals, macros and callback typedefs come from the T1
tables. An entity a shared public header publishes counts once, in its owner: the subsystem
that defines it when that subsystem lists the header, else the header's first-scheduled
sharer. LoC is nonblank lines after comments are stripped; `gcc -fpreprocessed -dD -E -P`
gives the same per-file figures.
"""
import csv, json, collections, sys

def sloc(path):
    s=open(R+path,errors='replace').read()
    # strip comments, keep strings
    out=[];i=0;n=len(s)
    while i<n:
        c=s[i]
        if c=='"' or c=="'":
            q=c;j=i+1
            while j<n and s[j]!=q:
                if s[j]=='\\': j+=1
                j+=1
            out.append(s[i:j+1]);i=j+1
        elif s.startswith('//',i):
            j=s.find('\n',i); j=n if j<0 else j
            while j<n and s[j-1]=='\\':
                k=s.find('\n',j+1); j=n if k<0 else k
            i=j
        elif s.startswith('/*',i):
            j=s.find('*/',i+2); j=n if j<0 else j+2
            out.append('\n'*s.count('\n',i,j)); i=j
        else:
            out.append(c);i+=1
    return sum(1 for l in ''.join(out).split('\n') if l.strip())


T = '/target/crustify/wavefront/codeql/'
R = '/target/'
SUBS = json.load(open(R + 'crustify/artifacts/subsystems.json'))['link_units'][0]['subsystems']
ORDER = [s['name'] for s in SUBS]
pos = {n: i for i, n in enumerate(ORDER)}
impl_of = {}; api_listers = collections.defaultdict(list)
for s in SUBS:
    for f in s['implementation_files']: impl_of[f] = s['name']
    for f in s['api_headers']: api_listers[f].append(s['name'])
for f in api_listers: api_listers[f].sort(key=pos.get)
EXCEPT = {('git_note_foreach_cb', 'callback'): 'history'}

def api_owner(name, kind, def_file, decl_files):
    hs = [h for h in decl_files | ({def_file} if def_file else set()) if h in api_listers]
    if not hs: return None
    if (name, kind) in EXCEPT: return EXCEPT[(name, kind)]
    listers = {s for h in hs for s in api_listers[h]}
    if def_file in api_listers:            # defined in a public header: its first-scheduled sharer
        return api_listers[def_file][0]
    if impl_of.get(def_file) in listers:   # defined by a subsystem that publishes it
        return impl_of[def_file]
    return min(listers, key=pos.get)

agg = [r for r in csv.DictReader(open(sys.argv[1]))]
types = list(csv.DictReader(open(T + 't1/types.csv')))
funcs = list(csv.DictReader(open(T + 't1/functions.csv')))
globs = list(csv.DictReader(open(T + 't1/globals.csv')))
macros = list(csv.DictReader(open(T + 't1/macros.csv')))
split = lambda c: set(filter(None, (c or '').split('|')))

impl = collections.defaultdict(collections.Counter)
api = collections.defaultdict(collections.Counter)
for r in agg:
    if r['kind'] == 'callback': continue
    k = {'struct': 'structs', 'union': 'unions', 'enum': 'enums'}[r['kind']]
    if r['file'] in impl_of: impl[impl_of[r['file']]][k] += 1
    if r['file'] in api_listers: api[api_listers[r['file']][0]][k] += 1
for t in types:
    if t['kind'] == 'typedef' and t['unaliased_kind'] == 'callback':
        decl = split(t['decl_files'])
        home = t['def_file'] or sorted(decl)[0]
        if home in impl_of: impl[impl_of[home]]['callbacks'] += 1
        o = api_owner(t['name'], 'callback', t['def_file'], decl)
        if o: api[o]['callbacks'] += 1
    elif t['kind'] == 'struct' and t['def_file'] not in api_listers and not t['name'].startswith('('):
        o = api_owner(t['name'], 'struct', t['def_file'], split(t['decl_files']))
        if o: api[o]['opaque_structs'] += 1
for f in funcs:
    if f['def_file'] in impl_of: impl[impl_of[f['def_file']]]['functions'] += 1
    o = api_owner(f['name'], 'function', f['def_file'], split(f['decl_files']))
    if o: api[o]['functions'] += 1
for g in globs:
    if g['def_file'] in impl_of: impl[impl_of[g['def_file']]]['global_variables'] += 1
    o = api_owner(g['name'], 'global', g['def_file'], split(g['decl_files']))
    if o: api[o]['global_variables'] += 1
for m in macros:
    if m['def_file'] in impl_of: impl[impl_of[m['def_file']]]['macros'] += 1
    if m['def_file'] in api_listers: api[api_listers[m['def_file']][0]]['macros'] += 1

IMPL = ['files', 'loc', 'structs', 'functions', 'global_variables', 'enums', 'unions', 'callbacks', 'macros']
API = ['files', 'loc', 'structs', 'opaque_structs', 'functions', 'global_variables', 'enums', 'unions', 'callbacks', 'macros']
def counters(s):
    i = impl[s['name']]; a = api[s['name']]
    i['files'] = len(s['implementation_files']); i['loc'] = sum(sloc(f) for f in s['implementation_files'])
    a['files'] = len(s['api_headers']); a['loc'] = sum(sloc(f) for f in s['api_headers'])
    return {'implementation': {k: i[k] for k in IMPL}, 'api': {k: a[k] for k in API}}


if __name__ == '__main__':
    out = {s['name']: counters(s) for s in SUBS}
    print(json.dumps(out, indent=1))
