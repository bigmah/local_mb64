import re, glob
CHECK = "if (recomp_should_preempt) recomp_preempt(rdram);"
func_re  = re.compile(r'^RECOMP_FUNC ')
label_re = re.compile(r'^(L_[0-9A-Fa-f]+):')
goto_re  = re.compile(r'^(\s*)goto (L_[0-9A-Fa-f]+);')
total = files = 0
for path in glob.glob('RecompiledFuncs/*.c'):
    lines = open(path).readlines()
    out, seen, changed = [], set(), False
    for line in lines:
        if func_re.match(line):
            seen = set()
        m = label_re.match(line)
        if m:
            seen.add(m.group(1))
        g = goto_re.match(line)
        if g and g.group(2) in seen:          # backward goto = loop back-edge
            out.append(g.group(1) + CHECK + "\n")
            total += 1; changed = True
        out.append(line)
    if changed:
        open(path, 'w').writelines(out); files += 1
print(f"inserted {total} preempt checks across {files} files")
