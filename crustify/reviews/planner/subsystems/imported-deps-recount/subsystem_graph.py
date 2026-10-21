"""Subsystem-level view of the campaign-wide wavefront graph.

Run from the repository root:

    PYTHONHASHSEED=0 /opt/venv/bin/python <this file> deps    # imported_deps per subsystem, as JSON
    PYTHONHASHSEED=0 /opt/venv/bin/python <this file> order   # the order cut, its trace and cut edges
    PYTHONHASHSEED=0 /opt/venv/bin/python <this file> cut A B ...  # dependency records an order A, B, ... cuts

The graph is wavefront's default (body-deep) graph of
crustify/wavefront/configs/wavefront-config.json, T3 tables included. Each node is owned by the
subsystem whose implementation_files or api_headers list its defining file; the entities of a
header several subsystems list are owned by its first-scheduled sharer (SHARED below).
"""
import os, sys, json, collections
sys.path.insert(0, '/opt/venv/share/wavefront/src')
from pathlib import Path
from wavefront.layout import set_config_path, set_repo_root, Layout
from wavefront import dag as D
R = Path('/target')
CFG = R / 'crustify/wavefront/configs/wavefront-config.json'
SUBS = json.load(open(R / 'crustify/artifacts/subsystems.json'))['link_units'][0]['subsystems']

def load(api, cfg=CFG):
    set_repo_root(R); set_config_path(cfg)
    layout = Layout.discover(cfg)
    g = D.build(layout, cfg, stage='planner', api_headers_only=api)
    return g, D.load_nodes(g)

# owner of a file: the subsystem listing it; shared headers resolved by `shared_owner`
def file_owner(shared_owner):
    own = {}
    multi = collections.defaultdict(list)
    for s in SUBS:
        for f in s['implementation_files'] + s['api_headers']:
            multi[f].append(s['name'])
    for f, v in multi.items():
        own[f] = v[0] if len(v) == 1 else shared_owner[f]
    return own, multi

KIND = {'struct':'structs','union':'unions','enum':'enums','callback':'callbacks','macro':'macros'}
def kind_of(n):
    sk = n.subkind
    if sk in ('external','builtin') and n.node_kind == 'type': return 'structs'
    if sk.startswith('function') or sk in ('external','builtin','symbol'): return 'functions'
    if sk.startswith('global'): return 'global_variables'
    if sk.startswith('macro'): return 'macros'
    return KIND.get(sk, sk)

def sub_edges(by_key, own):
    """{(a,b): [edge count, {kind: set(dest entities)}]} for in-tree a!=b, and out-of-tree deps per subsystem."""
    E = collections.defaultdict(lambda: [0, collections.defaultdict(set)])
    for n in by_key.values():
        a = own.get(n.defined_in)
        if a is None: continue
        for k in set(n.dep_types + n.dep_syms + n.fallback):
            m = by_key.get(k)
            f = k[1]
            b = own.get(f)
            if b is None or b == a: continue
            e = E[(a, b)]; e[0] += 1
            e[1][kind_of(m) if m else 'functions'].add(k)
    return E

def sccs(nodes, adj):
    idx = {}; low = {}; st = []; on = set(); out = []; c = [0]
    sys.setrecursionlimit(10000)
    def sc(v):
        idx[v] = low[v] = c[0]; c[0] += 1; st.append(v); on.add(v)
        for w in sorted(adj.get(v, ())):
            if w not in nodes: continue
            if w not in idx: sc(w); low[v] = min(low[v], low[w])
            elif w in on: low[v] = min(low[v], idx[w])
        if low[v] == idx[v]:
            comp = []
            while True:
                w = st.pop(); on.discard(w); comp.append(w)
                if w == v: break
            out.append(sorted(comp))
    for v in sorted(nodes):
        if v not in idx: sc(v)
    return out  # reverse topological (producers first) by Tarjan

def order(nodes, W, trace=None):
    """W[(a,b)] = edge count a depends on b. Returns producers-first order."""
    adj = collections.defaultdict(set)
    for (a, b), w in W.items():
        if a in nodes and b in nodes and w: adj[a].add(b)
    res = []
    for comp in sccs(set(nodes), adj):   # Tarjan emits sinks (producers) first
        if len(comp) == 1: res += comp; continue
        rem = set(comp)
        dep_count = lambda m: sum(1 for x in rem if x != m and W.get((x, m)))
        best = max(dep_count(m) for m in rem)
        tied = [m for m in rem if dep_count(m) == best]
        net = lambda m: sum(W.get((t, m), 0) - W.get((m, t), 0) for t in tied if t != m)
        head = sorted(tied, key=lambda m: (-net(m), m))[0]
        if trace is not None: trace.append((sorted(comp), head, best, {m: net(m) for m in tied}))
        res.append(head)
        res += order(rem - {head}, W, trace)
    return res

def library(defined_in, name):
    f = defined_in or ''
    b = f.rsplit('/', 1)[-1]
    if '/openssl/' in f: return 'libssl' if b in ('ssl.h', 'ssl2.h', 'ssl3.h', 'tls1.h', 'dtls1.h', 'srtp.h', 'sslerr.h') else 'libcrypto'
    if b in ('zlib.h', 'zconf.h'): return 'libz'
    if b.startswith('pcre2'): return 'libpcre2-8'
    return 'libc'

def imported_deps(by_key, own, link_unit='libgit2'):
    E = sub_edges(by_key, own)
    O = collections.defaultdict(lambda: collections.defaultdict(lambda: collections.defaultdict(set)))
    for n in by_key.values():
        a = own.get(n.defined_in)
        if a is None: continue
        for k in set(n.dep_types + n.dep_syms + n.fallback):
            f = k[1]
            if f is not None and not f.startswith('/'): continue   # in tree
            m = by_key.get(k)
            O[a][library(f, k[0])][kind_of(m) if m else 'functions'].add(k)
    KINDS = ['structs', 'functions', 'global_variables', 'enums', 'unions', 'callbacks', 'macros']
    res = {}
    for s in SUBS:
        a = s['name']
        it = []
        for (x, b), (w, kinds) in E.items():
            if x != a: continue
            it.append({'link_unit': link_unit, 'subsystem': b, 'outgoing_edges': w,
                       'incoming_edges': E[(b, a)][0] if (b, a) in E else 0,
                       'counters': {k: len(kinds.get(k, ())) for k in KINDS}})
        ot = [{'library': lib, 'counters': {k: len(kinds.get(k, ())) for k in KINDS}}
              for lib, kinds in O[a].items()]
        res[a] = (it, ot)
    return res


SHARED = {'include/git2/types.h': 'core', 'include/git2/repository.h': 'repository',
          'include/git2/sys/diff.h': 'diff', 'include/git2/notes.h': 'index'}

if __name__ == '__main__':
    own, _ = file_owner(SHARED)
    _, (bk, _) = load(False)
    names = [s['name'] for s in SUBS if s['implementation_files'] or s['api_headers']]
    E = sub_edges(bk, own)
    W = {k: v[0] for k, v in E.items()}
    cmd = sys.argv[1]
    if cmd == 'deps':
        res = imported_deps(bk, own)
        print(json.dumps({n: {'in_tree': res[n][0], 'out_of_tree': res[n][1]} for n in names}, indent=1))
    elif cmd == 'order':
        tr = []
        o = order(set(names), W, tr)
        print('order:', ' '.join(o))
        for comp, head, best, net in tr:
            print(f'  component of {len(comp)}: {head} first, {best} members depend on it; tied net edges {net}')
        pos = {n: i for i, n in enumerate(o)}
        cut = sorted(((a, b), w) for (a, b), w in W.items() if pos[b] > pos[a])
        print(f'cut: {len(cut)} records, {sum(w for _, w in cut)} edges')
    elif cmd == 'cut':
        o = sys.argv[2:]
        pos = {n: i for i, n in enumerate(o)}
        cut = sorted(((a, b), w) for (a, b), w in W.items() if pos[b] > pos[a])
        print(f'cut: {len(cut)} records, {sum(w for _, w in cut)} edges')
        for (a, b), w in cut: print(f'  {a} -> {b}: {w}')
