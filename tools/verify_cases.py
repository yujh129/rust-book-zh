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

  MISM    the compiler diagnostic, runtime failure, or exit status differs
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
            section = re.sub(r'^\d+(?:\.\d+)*\s+', '', text[i][3:].strip())
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
    return payload['result'], payload['success'], time.time() - start


def runner_alive(base):
    """True when a playground server already owns tools/playground/."""
    try:
        with urllib.request.urlopen(base + '/health', timeout=1.5) as r:
            return json.loads(r.read().decode('utf-8')).get('ok') is True
    except Exception:  # noqa: BLE001 - any failure means "not running"
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--local', action='store_true', help='use the local Cargo runner directly, without HTTP')
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
    # --local 用同一个 Cargo 工程编译，若同时还有运行器在跑，两个进程会互相覆盖
    # src/main.rs，拿到别人的输出。这种情况直接退出，让调用方走 HTTP。
    if args.local and runner_alive(args.base):
        print('a playground runner is already listening on %s; drop --local '
              '(check through HTTP) or stop the runner first' % args.base)
        return 2

    report, stats = [], {'match': 0, 'mismatch': 0, 'no_expected': 0, 'slow': 0,
                         'error': 0, 'runtime': 0}
    for path in files:
        rel = os.path.relpath(path, os.path.join(ROOT, 'src')).replace(os.sep, '/')
        blocks = parse(path)
        for n, block in enumerate(blocks):
            if block['section'] != '典型错误案例' or 'editable' not in block['info']:
                continue
            expected = ''
            runtime = ''
            for nxt in blocks[n + 1:]:
                if nxt['info'].startswith('rust') or nxt['section'] != '典型错误案例':
                    break
                if nxt['info'].startswith('text'):
                    transcript = nxt['code']
                    if 'test result: FAILED' in transcript:
                        runtime = 'test result: FAILED'
                    elif 'panicked at ' in transcript:
                        panic = re.search(r'panicked at [^\n]*\n([^\n]+)', transcript)
                        runtime = panic.group(1).strip() if panic else 'panicked at '
                    else:
                        expected = first_error_line(transcript)
                    break
                if nxt['info'].startswith('console'):
                    break
            try:
                if args.local:
                    from playground_server import evaluate
                    start = time.monotonic()
                    result, success = evaluate(block['code'])
                    elapsed = time.monotonic() - start
                else:
                    result, success, elapsed = post(args.base, block['code'], args.timeout)
            except Exception as exc:  # 运行器没响应也不该中断整轮校验
                stats['error'] += 1
                report.append(('ERR', rel, block['line'], expected,
                               'request failed: ' + str(exc)))
                sys.stdout.write('!')
                sys.stdout.flush()
                continue
            got = first_error_line(result)
            if elapsed > 3:
                stats['slow'] += 1
            if runtime and not success and runtime in result:
                stats['runtime'] += 1
            elif not runtime and not expected and success:
                stats['no_expected'] += 1
            elif expected and not success and got == expected:
                stats['match'] += 1
            else:
                stats['mismatch'] += 1
                report.append(('MISM', rel, block['line'], expected or runtime or 'successful exit',
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
        os.makedirs(os.path.dirname(target) or '.', exist_ok=True)
        with io.open(target, 'w', encoding='utf-8') as fh:
            for kind, rel, line, exp, got in report:
                fh.write('%s %s:%d' % (kind, rel, line) + chr(10))
                if exp:
                    fh.write('  expected: ' + exp + chr(10))
                fh.write('  got:      ' + got + chr(10))
    except OSError as exc:
        print('could not write report: ' + str(exc))
    print('')
    print('%d editable case blocks: %d match, %d mismatch, %d request errors, %d successful runs, %d expected runtime failures, %d slow'
          % (sum(stats.values()) - stats['slow'], stats['match'], stats['mismatch'],
             stats['error'], stats['no_expected'], stats['runtime'], stats['slow']))
    return 0 if stats['mismatch'] == 0 and stats['error'] == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
