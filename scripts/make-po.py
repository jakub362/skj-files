#!/usr/bin/env python3
"""Writes lang/pl/LC_MESSAGES/skj-files.po from the Slint string table below it
(scripts/slint_pl.py) and checks that every @tr("…") in ui/app.slint has a translation."""
import os, re, sys
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(here)
sys.path.insert(0, here)
from slint_pl import PL, PLURALS

ui = open(os.path.join(root, 'ui', 'app.slint'), encoding='utf-8').read()
unesc = lambda t: re.sub(r'\\(.)', lambda m: m.group(1), t)
singles = set(unesc(m.group(1)) for m in re.finditer(r'@tr\("((?:[^"\\]|\\.)*)"\s*[,)]', ui))
plurals = set((unesc(a), unesc(b)) for a, b in re.finditer(r'@tr\("((?:[^"\\]|\\.)*)"\s*\|\s*"((?:[^"\\]|\\.)*)"', ui) and
              [(m.group(1), m.group(2)) for m in re.finditer(r'@tr\("((?:[^"\\]|\\.)*)"\s*\|\s*"((?:[^"\\]|\\.)*)"', ui)])

def key(k):  # app.slint escapes literal braces as {{ }}
    return k if re.search(r'\{(?!\}|\d+\}|n\})[^}]*\}', k) is None else k.replace('{', '{{').replace('}', '}}')
plain = {key(k): v for k, v in PL.items()}
plain_val = {key(k): (key(v) if key(k) != k else v) for k, v in PL.items()}
missing = [s for s in singles if s not in plain]
missing += [p for p in plurals if p not in PLURALS]
if missing:
    print('missing Polish for:', *missing, sep='\n  ')
    sys.exit(1)

po = lambda s: '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'
out = ['msgid ""', 'msgstr ""', '"Content-Type: text/plain; charset=UTF-8\\n"', '"Language: pl\\n"',
       '"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\\n"', '']
for k in sorted(singles):
    out += ['msgid ' + po(k), 'msgstr ' + po(plain_val[k]), '']
for (a, b) in sorted(plurals):
    f = PLURALS[(a, b)]
    out += ['msgid ' + po(a), 'msgid_plural ' + po(b)] + ['msgstr[%d] %s' % (i, po(x)) for i, x in enumerate(f)] + ['']
dst = os.path.join(root, 'lang', 'pl', 'LC_MESSAGES', 'skj-files.po')
os.makedirs(os.path.dirname(dst), exist_ok=True)
open(dst, 'w', encoding='utf-8', newline='\n').write('\n'.join(out))
print('wrote', dst, len(singles), 'strings +', len(plurals), 'plural')
