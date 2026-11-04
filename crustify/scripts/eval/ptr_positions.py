"""ptr_positions.py <repo> <api-syms.txt> <api-types.txt> <wavefront config> <out.json>: the positions of the C types
the Rust tree wraps (a `ty::` anchor) that remain in the public C surface it has not wrapped:
parameters and returns of public functions without a `fn::` anchor, and fields of public structs
without a `ty::` anchor. Pointer positions (depth >= 1) need a seam to cross; by-value ones a
conversion."""
import csv,re,sys,pathlib,collections,json
repo=pathlib.Path(sys.argv[1]); T=repo/'crustify/wavefront/codeql'
anch=collections.defaultdict(set)
for p in (repo/'src').rglob('*.rs'):
    for m in re.finditer(r'/// (?:Wraps|Replaces) `(fn|ty)::(\w+)`',p.read_text()): anch[m[1]].add(m[2])
W=anch['ty']; wfn=anch['fn']
api={};
for l in open(sys.argv[2]):
    f=l.rstrip('\n').split('\t')
    if len(f)>3 and f[1].startswith('function'): api[f[0]]=f[3].split(',')[0]
hdrs=set(json.load(open(sys.argv[4]))['api_headers']['included'])
pubstruct={l.split('\t')[0] for l in open(sys.argv[3]) if l.split('\t')[1] in ('struct','union') and l.split('\t')[2] in hdrs}
rd=lambda n: csv.DictReader(open(T/n))
pdepth={}
for r in rd('t1/params.csv'): pdepth[(r['function_name'],int(r['position']))]=int(r['ptr_depth'])
rdepth={}
for r in rd('t1/functions.csv'):
    s=r['signature']; i=s.find(r['name']+'(')
    if i>0: rdepth[r['name']]=s[:i].count('*')
fn_pos=collections.Counter(); seen=set(); per_type=collections.defaultdict(lambda: collections.Counter()); fns=collections.defaultdict(set)
for r in rd('t2/signature_type_uses.csv'):
    f,t,pos=r['function_name'],r['type_name'],r['position']
    if t not in W or f not in api or f in wfn or (f,t,pos) in seen: continue
    seen.add((f,t,pos))
    d=rdepth.get(f,0) if pos=='return' else pdepth.get((f,int(pos.split('_')[1])),0)
    k=('ptr' if d else 'val')+('_ret' if pos=='return' else '_arg')
    fn_pos[k]+=1; per_type[t][k]+=1; fns[k].add(f)
fdepth={};fcb=set()
for r in rd('t1/fields.csv'):
    fdepth[(r['struct_name'],r['field_name'])]=int(r['ptr_depth'])
    if '(*)' in r['field_type']: fcb.add((r['struct_name'],r['field_name']))
fld=collections.Counter(); seenf=set(); structs=collections.defaultdict(set)
for r in rd('t2/field_type_uses.csv'):
    s,fl,t=r['struct_name'],r['field_name'],r['type_name']
    if t not in W or s in W or s not in pubstruct or (s,fl,t) in seenf: continue
    seenf.add((s,fl,t)); k='ptr_fld' if fdepth.get((s,fl),0) else 'val_fld'
    fld[k]+=1; per_type[t][k]+=1; structs[k].add(s)
out=dict(wrapped_types=len(W),unwrapped_api_fns=len(set(api)-wfn),positions=dict(fn_pos),fields=dict(fld),
         distinct_fns={k:len(v) for k,v in fns.items()},distinct_structs={k:len(v) for k,v in structs.items()},
         per_type={t:dict(c) for t,c in sorted(per_type.items(),key=lambda x:-sum(x[1].values()))})
print(json.dumps({k:v for k,v in out.items() if k!='per_type'}))
for t,c in list(out['per_type'].items())[:12]: print(f"  {t:28} {c}")
# --- struct/enum split, and nested fields: wrapped types reached through structs embedded by value
kind={}
for r in rd('t1/types.csv'): kind.setdefault(r['name'],r['unaliased_kind'] or r['kind'])
isenum=lambda t: kind.get(t,'').startswith('enum')
embed=collections.defaultdict(list)          # S -> [(field, T, depth)] over public structs
for r in rd('t2/field_type_uses.csv'):
    s=r['struct_name']
    if s in pubstruct: embed[s].append((r['field_name'],r['type_name'],'cb' if (s,r['field_name']) in fcb else fdepth.get((s,r['field_name']),0)))
for s in embed: embed[s]=sorted(set(embed[s]))
def walk(s,path,stack):
    for f,t,d in embed.get(s,()):
        p=path+[f]
        if t in W: yield '.'.join(p),t,d
        elif d==0 and t in pubstruct and t not in stack: yield from walk(t,p,stack|{t})
nest=collections.Counter(); nestex=collections.defaultdict(list); direct=collections.Counter()
for s in pubstruct-W:
    for path,t,d in walk(s,[],{s}):
        k=('enum' if isenum(t) else 'struct')+('_cb' if d=='cb' else '_ptr' if d else '_val')+('_nested' if '.' in path and path.split('.')[0] and len(path.split('.'))>1 and not any((s,path) == (s,f) for f,_,_ in embed[s]) else '')
        (nest if k.endswith('_nested') else direct)[k]+=1
        if k.endswith('_nested') and len(nestex[k])<6: nestex[k].append(f'{s}.{path}: {t}')
sig=collections.Counter()
for (f,t,pos) in seen:
    d=rdepth.get(f,0) if pos=='return' else pdepth.get((f,int(pos.split('_')[1])),0)
    sig[('enum' if isenum(t) else 'struct')+('_ptr' if d else '_val')+('_ret' if pos=='return' else '_arg')]+=1
print('signature positions in unwrapped public fns:',dict(sorted(sig.items())))
print('fields of unwrapped public structs, direct:',dict(sorted(direct.items())))
print('fields reached through embedded unwrapped structs (nested):',dict(sorted(nest.items())))
for k,v in nestex.items(): print(' ',k,v)
out.update(signature_positions=dict(sorted(sig.items())),field_positions_direct=dict(sorted(direct.items())),
           field_positions_nested=dict(sorted(nest.items())),nested_examples=nestex)
pathlib.Path(sys.argv[5]).write_text(json.dumps(out,indent=1)+'\n')
