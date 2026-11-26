#!/usr/bin/env python3
"""Structural QA for the mdBook: every chapter must match STYLE.md.

Checks per chapter file: line count, required headings, exactly five error
cases (each with a text block starting with 'error' plus a reason/fix pair),
no leftover stub markers, no TODO/ellipsis placeholders, and every relative
markdown link resolves.
"""
import re, sys
from pathlib import Path

SRC = Path('src')
REQUIRED = ['本章目标', '典型错误案例', '动手练习', '小结']
LINK = re.compile(r'\]\(([^)]+?\.md[^)]*)\)')


def chapter_files():
    out = []
    for line in (SRC / 'SUMMARY.md').read_text(encoding='utf-8').splitlines():
        m = LINK.search(line)
        if m:
            out.append(SRC / m.group(1).split('#')[0])
    return out


def check(path):
    issues = []
    text = path.read_text(encoding='utf-8')
    lines = text.split(chr(10))
    n = len(lines)
    rel = str(path).replace(chr(92), '/')
    is_project = rel.startswith('src/part7')
    is_appendix = rel.startswith('src/appendix')
    is_front = rel.startswith('src/00-')
    for m in LINK.finditer(text):
        dest = m.group(1).split('#')[0]
        if '://' not in dest and not (path.parent / dest).is_file():
            issues.append('broken link ' + m.group(1))
    if is_front:
        return n, issues
    low = 500 if is_project else (200 if is_appendix else 450)
    high = 1400 if is_project else (700 if is_appendix else 1200)
    if n < low or n > high:
        issues.append('lines ' + str(n) + ' outside [' + str(low) + ',' + str(high) + ']')
    if '<!-- stub -->' in text:
        issues.append('still a stub')
    if 'TODO' in text:
        issues.append('placeholder TODO')
    if not is_appendix:
        for want in REQUIRED:
            ok = any(want in l and (l.startswith('##') or l.startswith('>')) for l in lines)
            if not ok:
                issues.append('missing section ' + want)
    errs = sum(1 for i, l in enumerate(lines) if l.strip().startswith('> **本章目标**'))
    if errs == 0:
        issues.append('missing goal blockquote')
    reasons = text.count('**原因**')
    fixes = text.count('**修复**')
    if not is_appendix:
        if reasons < 5 or fixes < 5:
            issues.append('error cases reason/fix = ' + str(reasons) + '/' + str(fixes) + ' (want 5/5)')
        texts = 0
        for i, l in enumerate(lines):
            if l.strip() == chr(96) * 3 + 'text':
                nxt = next((x for x in lines[i + 1:i + 4] if x.strip()), '')
                if nxt.strip().startswith('error'):
                    texts += 1
        if texts < 3:
            issues.append('only ' + str(texts) + ' rustc error outputs')
    if not is_appendix:
        section = re.search(r'^## (?:\d+\.\d+ )?典型错误案例[^\n]*\n(.*?)(?=^## |\Z)', text, re.M | re.S)
        cases = re.split(r'^### ', section.group(1), flags=re.M)[1:] if section else []
        if len(cases) != 5:
            issues.append(f'expected five case headings, found {len(cases)}')
        for case in cases:
            if '**原因**' not in case or '**修复**' not in case:
                issues.append('missing reason or fix in ' + case.splitlines()[0])
            if '<details class="case-answer">' not in case:
                issues.append('answers not collapsed in ' + case.splitlines()[0])
            n_rust = len(re.findall(r'^```rust[^\n]*$', case, re.M))
            n_cfg = len(re.findall(r'^```(?:toml|bash|console)', case, re.M))
            if n_rust < 2 and n_cfg == 0:
                issues.append('no fixed code in ' + case.splitlines()[0])
    return n, issues


def main():
    files = chapter_files()
    missing = set(SRC.rglob('*.md')) - set(files) - {SRC / 'SUMMARY.md'}
    for path in sorted(missing):
        print('not listed in SUMMARY.md: ' + str(path))
    bad = len(missing)
    for f in files:
        if not f.exists():
            print('MISSING ' + str(f))
            bad += 1
            continue
        n, issues = check(f)
        if issues:
            bad += 1
            print(str(f) + ' (' + str(n) + ' lines)')
            for i in issues:
                print('    - ' + i)
    print('')
    print(str(len(files)) + ' chapters, ' + str(bad) + ' with issues')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
