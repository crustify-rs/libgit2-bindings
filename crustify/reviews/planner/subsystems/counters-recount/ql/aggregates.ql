import cpp
// Every struct, union and enum definition in the traced build, named or anonymous,
// with its defining file. T1's types.csv files unions as structs; this tells them apart.
from Element e, string kind, string name, string file, int line
where
  exists(Class c | c = e and exists(c.getDefinitionLocation()) |
    (if c instanceof Union then kind = "union" else kind = "struct") and
    name = c.getName() and file = c.getDefinitionLocation().getFile().getRelativePath() and
    line = c.getDefinitionLocation().getStartLine())
  or
  exists(Enum en, TypeDeclarationEntry d | en = e and d = en.getADeclarationEntry() and d.isDefinition() |
    kind = "enum" and name = en.getName() and
    file = d.getFile().getRelativePath() and line = d.getLocation().getStartLine())
select kind, name, file, line
