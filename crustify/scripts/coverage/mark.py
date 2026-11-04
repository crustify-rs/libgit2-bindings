"""mark.py <crate src dir> <crate root file>: put #[coverage(off)] on every test item."""
import re,sys,pathlib
src=pathlib.Path(sys.argv[1]); root=pathlib.Path(sys.argv[2])
ATTR=re.compile(r'^(\s*)#\[(test|cfg\((?:[^\]]*\btest\b[^\]]*)\))\]\s*$')
ITEM=re.compile(r'^\s*(pub(\([^)]*\))?\s+)?(const\s+)?(async\s+)?(unsafe\s+)?(extern\s+"C"\s+)?(mod|fn|impl)\b')
n=0
for f in src.rglob('*.rs'):
    L=f.read_text().split('\n'); out=[]; i=0
    while i<len(L):
        m=ATTR.match(L[i])
        if m:
            j=i+1
            while j<len(L) and (L[j].strip().startswith('#[') or L[j].strip().startswith('//')): j+=1
            prev=out[-1].strip() if out else ''
            if j<len(L) and ITEM.match(L[j]) and prev!='#[coverage(off)]':
                out.append(m.group(1)+'#[coverage(off)]'); n+=1
        out.append(L[i]); i+=1
    f.write_text('\n'.join(out))
r=root.read_text(); root.write_text('#![feature(coverage_attribute)]\n'+r)
print('marked',n)
