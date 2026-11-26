import argparse, io, os, re, sys
sys.stdout.reconfigure(encoding='utf-8')
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src')
BT = chr(96)
FENCE = re.compile(r'^(' + BT + r'{3,})(.*)$')
HEAD = re.compile(r'^(#{1,6})\s+(.*)$')
CASE = re.compile(r'^###\s+')
OPEN = '<details class="case-answer">'
SUMMARY = '<summary>查看答案</summary>'
CLOSE = '</details>'

VERBOSE = False

def fence_mask(lines):
    mask = [False] * len(lines)
    in_fence = False
    for k, line in enumerate(lines):
        if FENCE.match(line):
            mask[k] = True
            in_fence = not in_fence
        else:
            mask[k] = in_fence
    return mask

def section_bounds(lines):
    out = []
    mask = fence_mask(lines)
    i = 0
    while i < len(lines):
        m = None if mask[i] else HEAD.match(lines[i])
        if m and '典型错误案例' in m.group(2):
            level = len(m.group(1))
            j = i + 1
            while j < len(lines):
                h = None if mask[j] else HEAD.match(lines[j])
                if h and len(h.group(1)) <= level:
                    break
                j += 1
            out.append((i, j))
            i = j
        else:
            i += 1
    return out

def wrap_case(body):
    start = next((k for k, l in enumerate(body) if l.startswith('**原因**')), -1)
    if start < 0:
        return body, False
    if any(OPEN in l for l in body):
        return body, False
    rest = list(body[start:])
    trail = 0
    while rest and not rest[-1].strip():
        rest.pop()
        trail += 1
    if not rest:
        return body, False
    head = list(body[:start])
    while head and not head[-1].strip():
        head.pop()
    new = head + ['', OPEN, SUMMARY, ''] + rest + ['', CLOSE] + [''] * trail
    return new, True

def process(path, apply=False):
    text = io.open(path, encoding='utf-8').read()
    lines = text.split(chr(10))
    wrapped = skipped = 0
    mask = fence_mask(lines)
    for (s, e) in reversed(section_bounds(lines)):
        starts = [k for k in range(s + 1, e) if CASE.match(lines[k]) and not mask[k]]
        bounds = list(zip(starts, starts[1:] + [e]))
        for a, b in reversed(bounds):
            if b <= a:
                continue
            new, ok = wrap_case(lines[a:b])
            if ok:
                lines[a:b] = new
                wrapped += 1
            else:
                skipped += 1
                if VERBOSE:
                    print('    skip: %s' % (new[0][:70] if new else '<empty>'))
    if wrapped and apply:
        io.open(path, 'w', encoding='utf-8', newline='').write(chr(10).join(lines))
    return wrapped, skipped, lines

def main():
    ap = argparse.ArgumentParser(description='把每章案例的答案（原因/修复）包进可折叠面板')
    ap.add_argument('--only', help='只处理路径包含该子串的文件')
    ap.add_argument('--apply', action='store_true', help='真正写回；默认只报告')
    ap.add_argument('--show', action='store_true', help='打印处理后案例 2/3 之间的片段')
    ap.add_argument('--verbose', action='store_true', help='打印被跳过的案例')
    args = ap.parse_args()
    global VERBOSE
    VERBOSE = args.verbose
    files = []
    for dp, _, ns in os.walk(ROOT):
        for n in sorted(ns):
            if n.endswith('.md'):
                files.append(os.path.join(dp, n))
    files.sort()
    tw = ts = 0
    for f in files:
        if args.only and args.only not in f.replace(os.sep, '/'):
            continue
        w, s, lines = process(f, apply=args.apply)
        tw += w
        ts += s
        if w or s:
            print('%-46s wrapped=%-3d skipped=%-3d' % (os.path.relpath(f, ROOT).replace(os.sep, '/'), w, s))
        if args.show and w:
            idx = [k for k, l in enumerate(lines) if l.strip() == '</details>']
            k = idx[0] if idx else 0
            print('--- 片段（案例 1 结束 → 案例 2 开始）---')
            print(chr(10).join(lines[max(0, k - 6):k + 8]))
    print('TOTAL wrapped=%d skipped=%d apply=%s' % (tw, ts, args.apply))

if __name__ == '__main__':
    main()