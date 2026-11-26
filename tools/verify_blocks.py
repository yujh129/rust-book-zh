#!/usr/bin/env python3
"""Extract every rust fenced block from the book and compile it with rustc.

Blocks whose info string is exactly 'rust' must be self-contained programs
(mdBook playground style). Hidden lines (mdBook hidelines, prefix '##') are
unhidden before compiling. Third-party/noplayground blocks are skipped.
"""
import argparse, subprocess, sys
from pathlib import Path
import re

FENCE = chr(96) * 3
PATTERN = re.compile(r'^[ \t]*' + FENCE + r'rust[ \t]*\n(.*?)^[ \t]*' + FENCE + r'[ \t]*$', re.M | re.S)


def blocks_of(text):
    for m in PATTERN.finditer(text):
        body = m.group(1)
        lines = [ln[2:] if ln.startswith('##') else ln for ln in body.splitlines()]
        start = text[: m.start()].count(chr(10)) + 1
        yield start, chr(10).join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('src', nargs='?', default='src')
    ap.add_argument('--only', default=None)
    args = ap.parse_args()

    files = sorted(Path(args.src).rglob('*.md'))
    if args.only:
        files = [f for f in files if args.only in str(f)]
    work = Path('.scratch/rustbook-check')
    work.mkdir(parents=True, exist_ok=True)
    tasks = []
    for f in files:
        for lineno, code in blocks_of(f.read_text(encoding='utf-8')):
            tag = f.stem + '-' + str(lineno)
            rs = work / (tag + '.rs')
            rs.write_text(code, encoding='utf-8')
            tasks.append((tag, str(f), lineno, rs))

    failures = []
    for tag, path, lineno, rs in tasks:
        out = work / (tag + '.o')
        proc = subprocess.run(
            ['rustc', '--edition', '2021', '--crate-type', 'bin',
             '--emit=obj', '-A', 'warnings', '-o', str(out), str(rs)],
            capture_output=True, text=True, encoding='utf-8', errors='replace')
        if proc.returncode != 0:
            head = [l for l in proc.stderr.splitlines() if l.startswith('error')][:3]
            failures.append((path + ':' + str(lineno), head, rs))

    print('checked ' + str(len(tasks)) + ' rust blocks in ' + str(len(files)) + ' files, '
          + str(len(failures)) + ' failed')
    for where, head, rs in failures:
        print('')
        print('--- FAIL ' + where)
        for line in head:
            print('    ' + line)
    if failures:
        print('')
        print('block sources kept in ' + str(work))
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
