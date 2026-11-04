"""report.py <cov dir> <name>=<dir>[:<dir>...][@<compilation dir>],... --c <C object>...

Merges each suite's profiles under <cov dir>/prof/, the four suites' union as TOTAL and every
suite's as ALL, and prints the TOTAL line of `llvm-cov report` for each named scope, with the C
objects and every test and doctest binary as objects for every suite. A scope's @<dir>
resolves the relative paths a C build recorded. MERGED=1 reuses the merged profiles, and ONLY=<suite> reports that one alone."""
import subprocess,sys,pathlib,json,glob,os
out0=pathlib.Path(sys.argv[1])
nightly=os.environ.get("NIGHTLY") or next(l.split('"')[1] for l in open(out0/"tree"/"Cargo.toml") if l.startswith("nightly = "))
sysroot=subprocess.run(['rustc','+'+nightly,'--print','sysroot'],capture_output=True,text=True,check=True).stdout.strip()
B=sysroot+'/lib/rustlib/x86_64-unknown-linux-gnu/bin/'
out=pathlib.Path(sys.argv[1]); scopes=dict(s.split('=',1) for s in sys.argv[2].split(','))
cobjs=sys.argv[sys.argv.index('--c')+1:]
bins=[l.split()[2] for l in (out/'bins.txt').read_text().splitlines()]
docs=[p for p in glob.glob(str(out/'doctests'/'**'/'rust_out'),recursive=True)]
objs=cobjs+bins+docs
suites=sorted(p.name for p in (out/'prof').iterdir() if any(p.glob('*.profraw')))
def merge(name,dirs):
    raws=[str(r) for d in dirs for r in (out/'prof'/d).glob('*.profraw')]
    prof=out/f'{name}.profdata'
    subprocess.run([B+'llvm-profdata','merge','-sparse','-o',str(prof)]+raws,check=True)
    return prof
def total(prof,srcs):
    cd=[]
    if '@' in srcs[-1]: srcs[-1],c=srcs[-1].split('@'); cd=['-compilation-dir='+c]
    args=[B+'llvm-cov','report','-instr-profile',str(prof)]+cd+[objs[0]]+sum((['-object',o] for o in objs[1:]),[])+srcs
    r=subprocess.run(args,capture_output=True,text=True)
    if r.returncode: sys.exit(r.stderr[-2000:])
    t=[l for l in r.stdout.splitlines() if l.startswith('TOTAL')][-1].split()
    # TOTAL regions missed cover functions missed exec lines missed cover branches missed cover
    return dict(lines=(int(t[7])-int(t[8]),int(t[7]),t[9]),functions=(int(t[4])-int(t[5]),int(t[4]),t[6]),regions=(int(t[1])-int(t[2]),int(t[1]),t[3]))
res={}
skip=os.environ.get('MERGED')
four=[x for x in suites if x!='doc']
only=os.environ.get('ONLY')
for s in ([only] if only else suites+['TOTAL','ALL']):
    prof=out/f'{s}.profdata' if skip else merge(s,four if s=='TOTAL' else suites if s=='ALL' else [s])
    res[s]={n:total(prof,src.split(':')) for n,src in scopes.items()}
    print(s,json.dumps(res[s]))
(out/(f'figures-{only}.json' if only else 'figures.json')).write_text(json.dumps(res,indent=1))
