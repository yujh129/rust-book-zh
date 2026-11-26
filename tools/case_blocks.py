#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""Analyze code blocks inside '## 典型错误案例' sections of the book.

For every block: does it type-check with rustc, does it have fn main, which
error codes does it produce. Writes a TSV report to .scratch/case_blocks.tsv.
"""
import io, os, re, subprocess, sys, json, tempfile, hashlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, 'src')
WORK = os.path.join(ROOT, '.scratch', 'casecheck')
os.makedirs(WORK, exist_ok=True)

FENCE = re.compile(r'^(\x60{3,})([^\x60]*)$')

def iter_md(root):
    for dirpath, _, files in os.walk(root):
        for f in sorted(files):
            if f.endswith('.md'):
                yield os.path.join(dirpath, f)

def blocks(path):
    lines = io.open(path, encoding='utf-8').read().split('\n')
    out = []
    i = 0
    section = ''
    while i < len(lines):
        line = lines[i]
        m = FENCE.match(line)
        if m:
            fence = m.group(1)
            info = m.group(2).strip()
            start = i + 1
            i += 1
            body = []
            while i < len(lines) and not lines[i].startswith(fence):
                body.append(lines[i])
                i += 1
            out.append({'section': section, 'info': info, 'start': start + 1,
                        'code': '\n'.join(body)})
            i += 1
            continue
        if line.startswith('## '):
            section = line[3:].strip()
        i += 1
    return out

def check(code, tag):
    """Return (ok, errcodes, has_main, errtext)."""
    src = os.path.join(WORK, tag + '.rs')
    io.open(src, 'w', encoding='utf-8', newline='\n').write(code + '\n')
    p = subprocess.run(['rustc', '--edition', '2021', '--crate-type', 'bin',
                        '--emit=metadata', '-A', 'warnings', '-o',
                        os.path.join(WORK, tag), src],
                       capture_output=True, text=True, encoding='utf-8',
                       errors='replace', cwd=WORK)
    err = (p.stderr or '')
    codes = sorted(set(re.findall(r'error\[(E\d{4})\]', err)))
    return (p.returncode == 0, codes, 'fn main' in code, err)

rows = []
for path in iter_md(SRC):
    rel = os.path.relpath(path, SRC).replace('\\', '/')
    bs = blocks(path)
    in_case = False
    for b in bs:
        if b['section'] == '典型错误案例':
            in_case = True
        elif b['section'] in ('动手练习', '小结'):
            in_case = False
        if not in_case or not b['info'].startswith('rust'):
            continue
        tag = hashlib.md5((rel + ':' + str(b['start'])).encode()).hexdigest()[:10]
        ok, codes, main, err = check(b['code'], tag)
        rows.append({'file': rel, 'line': b['start'], 'info': b['info'],
                     'ok': ok, 'codes': ','.join(codes), 'main': main,
                     'len': len(b['code'].split('\n'))})

out = os.path.join(ROOT, '.scratch', 'case_blocks.tsv')
with io.open(out, 'w', encoding='utf-8') as f:
    f.write('file\tline\tinfo\tok\tcodes\tmain\tlines\n')
    for r in rows:
        f.write('%s\t%d\t%s\t%s\t%s\t%s\t%d\n' % (r['file'], r['line'], r['info'],
                'ok' if r['ok'] else 'FAIL', r['codes'], 'main' if r['main'] else '-', r['len']))

print('blocks:', len(rows))
print('playground(rust):', sum(1 for r in rows if r['info'] == 'rust'))
print('noplayground:', sum(1 for r in rows if 'noplayground' in r['info']))
print('ok:', sum(1 for r in rows if r['ok']), 'fail:', sum(1 for r in rows if not r['ok']))
print('with main:', sum(1 for r in rows if r['main']))
print('no main:', sum(1 for r in rows if not r['main']))
print('report:', out)
