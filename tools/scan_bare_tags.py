import re, pathlib

root = pathlib.Path('src')
pat = re.compile(r'<[A-Za-z][^<>]{0,30}>')
code_span = re.compile(r'(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)')

for path in sorted(root.rglob('*.md')):
    rel = path.relative_to(root).as_posix()
    if rel == 'SUMMARY.md':
        continue
    in_fence = False
    for i, line in enumerate(path.read_text(encoding='utf-8').split(chr(10)), 1):
        if line.lstrip().startswith('```'):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        stripped = code_span.sub('', line)
        hits = pat.findall(stripped)
        if hits:
            print(rel + ':' + str(i) + ': ' + str(hits) + '  ||  ' + line.strip()[:150])