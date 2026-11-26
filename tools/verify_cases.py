#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Run every editable case block through the local playground runner and compare
what rustc says with the error text documented right below the block.

    python tools/serve_book.py --no-serve &     # or tools/playground_server.py
    python tools/verify_cases.py                # all chapters
    python tools/verify_cases.py --only ch05    # one chapter

Every block marked "rust,editable" inside a 典型错误案例 section is sent to the
runner.  The block right after it (if it is a text block) documents the error the
reader is supposed to see; the first error[...] line must match.

  NOEXP   no documented error below the block (it should compile and run)
  MISM    the first error line differs from the documented one
  SLOW    the runner needed more than 3 seconds (the browser gives up at 6)
"""
import argparse
import io
import json
import os
import re
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
Q = chr(96)
FENCE = re.compile('^(' + Q + '{3,})([^' + Q + ']*)$')


def parse(path):
    """Split a chapter into fenced blocks, remembering the last ## heading."""
    text = io.open(path, encoding='utf-8').read().split(chr(10))
    out, section, i = [], '', 0
    while i < len(text):
        m = FENCE.match(text[i])
        if m:
            fence, info = m.group(1), m.group(2).strip()
            first = i + 1
            i += 1
            chunk = []
            while i < len(text) and not text[i].startswith(fence):
                chunk.append(text[i])
                i += 1
            out.append({'section': section, 'info': info, 'line': first,
                        'code': chr(10).join(chunk)})
            i += 1
            continue
        if text[i].startswith('## '):
            section = text[i][3:].strip()
        i += 1
    return out


def first_error_line(text):
    for ln in text.split(chr(10)):
        ln = re.sub(r'\s+', ' ', ln.strip())
        if ln.startswith('error'):
            return ln
    return ''


def post(base, code, timeout):
    data = json.dumps({'version': 'stable', 'optimize': '0', 'code': code,
                       'edition': '2021'}).encode('utf-8')
    req = urllib.request.Request(base + '/evaluate.json', data=data,
                                 headers={'Content-Type': 'application/json'})
    start = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        payload = json.loads(r.read().decode('utf-8'))
    return payload.get('result', ''), time.time() - start


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--base', default='http://127.0.0.1:3030',
                    help='local playground runner (default 127.0.0.1:3030)')
    ap.add_argument('--only', default=None, help='only chapters whose path contains this')
    ap.add_argument('--report', default=None, help='write the per-block report here')
    ap.add_argument('--timeout', type=float, default=60.0)
    args = ap.parse_args()

    files = []
    for root, _, names in os.walk(os.path.join(ROOT, 'src')):
        for name in sorted(names):
            if not name.endswith('.md'):
                continue
            path = os.path.join(root, name)
            if args.only and args.only not in path.replace(os.sep, '/'):
                continue
            files.append(path)
    if not files:
        print('no chapters matched')
        return 2

    report, stats = [], {'match': 0, 'mismatch': 0, 'no_expected': 0, 'slow': 0}
    for path in files:
        rel = os.path.relpath(path, os.path.join(ROOT, 'src')).replace(os.sep, '/')
        blocks = parse(path)
        for n, block in enumerate(blocks):
            if block['section'] != '典型错误案例' or 'editable' not in block['info']:
                continue
            expected = ''
            for nxt in blocks[n + 1:]:
                if nxt['info'].startswith('rust') or nxt['section'] != '典型错误案例':
                    break
                if nxt['info'].startswith('text'):
                    expected = first_error_line(nxt['code'])
                    break
                if nxt['info'].startswith('console'):
                    break
            result, elapsed = post(args.base, block['code'], args.timeout)
            got = first_error_line(result)
            if elapsed > 3:
                stats['slow'] += 1
            if not expected:
                stats['no_expected'] += 1
                report.append(('NOEXP', rel, block['line'], '',
                               result.strip()[:200].replace(chr(10), ' / ')))
            elif got == expected or (got and expected and
                                     (got.startswith(expected) or expected.startswith(got))):
                stats['match'] += 1
            else:
                stats['mismatch'] += 1
                report.append(('MISM', rel, block['line'], expected,
                               got or result.strip()[:200]))
            sys.stdout.write('.')
            sys.stdout.flush()
    print('')
    for kind, rel, line, exp, got in report:
        print('%s %s:%d' % (kind, rel, line))
        if exp:
            print('  expected: ' + exp)
        print('  got:      ' + got)

    target = args.report or os.path.join(ROOT, '.scratch', 'case_verify.txt')
    try:
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with io.open(target, 'w', encoding='utf-8') as fh:
            for kind, rel, line, exp, got in report:
                fh.write('%s %s:%d' % (kind, rel, line) + chr(10))
                if exp:
                    fh.write('  expected: ' + exp + chr(10))
                fh.write('  got:      ' + got + chr(10))
    except OSError as exc:
        print('could not write report: ' + str(exc))
    print('')
    print('%d editable case blocks: %d match, %d mismatch, %d without documented error, %d slow'
          % (sum(stats.values()), stats['match'], stats['mismatch'],
             stats['no_expected'], stats['slow']))
    return 0 if stats['mismatch'] == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
