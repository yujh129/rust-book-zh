
import argparse, io, json, os, re, sys
sys.stdout.reconfigure(encoding='utf-8')
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from wrap_answers import fence_mask, FENCE, HEAD, CASE, CLOSE
BT = chr(96)
FENCE3 = BT * 3
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))

def case_bounds(lines):
    mask = fence_mask(lines)
    out = []
    start = None
    for i, l in enumerate(lines):
        if not mask[i] and re.match(r'^## (?:\d+\.\d+ )?典型错误案例', l):
            start = i; break
    if start is None:
        return out, mask
    end = len(lines)
    for j in range(start + 1, len(lines)):
        if not mask[j] and lines[j].startswith('## '):
            end = j; break
    bounds = [k for k in range(start + 1, end) if not mask[k] and CASE.match(lines[k])]
    return list(zip(bounds, bounds[1:] + [end])), mask

def insert(path, case_key, code, info='rust,editable', label='修复后的完整代码：'):
    text = io.open(path, encoding='utf-8').read()
    lines = text.split(chr(10))
    bounds, mask = case_bounds(lines)
    hits = [(a, b) for (a, b) in bounds if case_key in lines[a]]
    if len(hits) != 1:
        return 'case-not-unique(%d)' % len(hits)
    a, b = hits[0]
    body = lines[a:b]
    if any(label in l for l in body):
        return 'already-has-fix'
    if any(FENCE3 in l for l in body) and len([l for l in body if FENCE.match(l) and FENCE.match(l).group(2).strip().startswith('rust')]) >= 2:
        return 'already-has-rust-fix'
    for l in code.split(chr(10)):
        if l.strip().startswith(BT * 3):
            return 'code-contains-fence'
    # find the panel closer of this case
    idx = None
    for k in range(b - 1, a - 1, -1):
        if lines[k].strip() == CLOSE:
            idx = k; break
    if idx is None:
        return 'no-answer-panel'
    block = ['', label, '', FENCE3 + info] + code.rstrip(chr(10)).split(chr(10)) + [FENCE3, '']
    lines[idx:idx] = block
    io.open(path, 'w', encoding='utf-8', newline='').write(chr(10).join(lines))
    return 'ok'

def main():
    ap = argparse.ArgumentParser(description='给案例答案面板补上「修复后的完整代码」块')
    ap.add_argument('json_file', help='[{file, case, code, info?, label?}, ...]')
    ap.add_argument('--apply', action='store_true')
    args = ap.parse_args()
    items = json.load(io.open(args.json_file, encoding='utf-8'))
    for it in items:
        path = it['file'] if os.path.isabs(it['file']) else os.path.join(ROOT, it['file'])
        if not args.apply:
            print('DRY  %-46s %s' % (it['file'], it['case']))
            continue
        r = insert(path, it['case'], it['code'], it.get('info', 'rust,editable'), it.get('label', '修复后的完整代码：'))
        print('%-20s %-46s %s' % (r, it['file'].split('/')[-1], it['case']))

if __name__ == '__main__':
    main()
