#!/usr/bin/env python3
"""One-line-per-chapter progress table."""
import re, sys
from pathlib import Path

SRC = Path('src')
LINK = re.compile(r'\]\(([^)]+?\.md[^)]*)\)')

def files():
    out = []
    for line in (SRC / 'SUMMARY.md').read_text(encoding='utf-8').splitlines():
        m = LINK.search(line)
        if m:
            out.append(SRC / m.group(1).split('#')[0])
    return out

done = partial = stub = 0
for f in files():
    if not f.exists():
        print('MISSING ' + str(f))
        continue
    text = f.read_text(encoding='utf-8')
    n = text.count(chr(10)) + 1
    if '<!-- stub -->' in text:
        state = 'STUB'
        stub += 1
    elif '<!-- MORE -->' in text:
        state = 'PARTIAL'
        partial += 1
    else:
        state = 'OK'
        done += 1
    cases = text.count('**原因**')
    print('%-6s %4d  cases=%d  %s' % (state, n, cases, str(f).replace(chr(92), '/')))
print('')
print('OK %d / partial %d / stub %d / total %d' % (done, partial, stub, len(files())))
