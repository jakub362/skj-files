#!/usr/bin/env python3
"""Lists English strings in the Rust code that have no Polish entry in src/i18n_pl.rs.
Looks at ts("…"), tf!("…"), and the literal labels given to mi(), chk(), p() and toast()."""
import os, re, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
table = open(os.path.join(root, 'src', 'i18n_pl.rs'), encoding='utf-8').read()
keys = set(m.group(1) for m in re.finditer(r'^\s*\("((?:[^"\\]|\\.)*)", "', table, re.M))
pat = re.compile(r'(?:\bts\(|tf!\(|\bmi\(&?|\bchk\(|\bp\(&?|\btoast\()\s*"((?:[^"\\]|\\.)*)"')
missing = {}
for f in ('app.rs', 'fsx.rs', 'sysint.rs', 'winshell.rs'):
    for no, line in enumerate(open(os.path.join(root, 'src', f), encoding='utf-8'), 1):
        for m in pat.finditer(line):
            k = m.group(1)
            if k and re.search(r'[A-Za-z]{2}', k) and k not in keys:
                missing.setdefault(k, f'{f}:{no}')
for k, where in missing.items():
    print(f'{where}: "{k}"')
print(f'{len(keys)} Polish strings, {len(missing)} missing')
sys.exit(1 if missing else 0)
