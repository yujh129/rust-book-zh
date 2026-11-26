#!/usr/bin/env python3
"""Retag deliberate-error blocks: a rust block that fails to compile and is
immediately followed by a text block starting with 'error' becomes
rust,noplayground so mdBook's playground does not offer a broken Run button.
"""
import subprocess, sys, tempfile
from pathlib import Path

FENCE = chr(96) * 3


def parse(lines):
    blocks, i = [], 0
    while i < len(lines):
        s = lines[i].strip()
        if s.startswith(FENCE):
            info = s[3:].strip()
            j = i + 1
            while j < len(lines) and lines[j].strip() != FENCE:
                j += 1
            if j < len(lines):
                blocks.append({'open': i, 'close': j, 'info': info, 'body': lines[i + 1:j]})
                i = j + 1
                continue
        i += 1
    return blocks


def compiles(code, work):
    rs = work / 'probe.rs'
    rs.write_text(code, encoding='utf-8')
    p = subprocess.run(['rustc', '--edition', '2021', '--crate-type', 'bin',
                        '--emit=metadata', '-A', 'warnings', '-o', str(work / 'probe.rmeta'), str(rs)],
                       capture_output=True, text=True, encoding='utf-8', errors='replace')
    return p.returncode == 0


def main(paths):
    work = Path('.scratch/tagfix')
    work.mkdir(parents=True, exist_ok=True)
    total = 0
    for path in paths:
        text = Path(path).read_text(encoding='utf-8')
        lines = text.split(chr(10))
        blocks = parse(lines)
        changed = []
        for idx, b in enumerate(blocks):
            if b['info'] != 'rust':
                continue
            code = chr(10).join(l[2:] if l.startswith('##') else l for l in b['body'])
            if compiles(code, work):
                continue
            nxt = blocks[idx + 1] if idx + 1 < len(blocks) else None
            if not nxt or nxt['info'] != 'text':
                print(path + ':' + str(b['open'] + 1) + ' rust block fails and is not an error demo')
                continue
            first = next((l for l in nxt['body'] if l.strip()), '')
            if not first.strip().startswith('error'):
                print(path + ':' + str(b['open'] + 1) + ' fails, next text block is not an error message')
                continue
            lines[b['open']] = lines[b['open']].replace(FENCE + 'rust', FENCE + 'rust,noplayground', 1)
            changed.append(b['open'] + 1)
        if changed:
            Path(path).write_text(chr(10).join(lines), encoding='utf-8')
            print(str(path) + ' retagged: ' + ', '.join(map(str, changed)))
            total += len(changed)
    print('retagged ' + str(total) + ' blocks')


if __name__ == '__main__':
    main(sys.argv[1:] or ['src'])
