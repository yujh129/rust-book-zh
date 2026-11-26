#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Start the book preview and the local Rust playground runner together.

    python tools/serve_book.py            # book on :3000, runner on :3030
    python tools/serve_book.py --no-serve # runner only

Press Ctrl-C to stop both.
"""
import argparse
import os
import subprocess
import sys
import threading

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def pump(tag, proc):
    for line in proc.stdout:
        sys.stdout.write('[%s] %s' % (tag, line))
        sys.stdout.flush()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=3000, help='mdbook serve port')
    ap.add_argument('--runner-port', type=int, default=3030)
    ap.add_argument('--no-serve', action='store_true')
    args = ap.parse_args()

    procs = []
    runner = subprocess.Popen(
        [sys.executable, os.path.join(HERE, 'playground_server.py'),
         '--port', str(args.runner_port)],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding='utf-8', errors='replace')
    procs.append(('runner', runner))

    if not args.no_serve:
        book = subprocess.Popen(
            ['mdbook', 'serve', '--hostname', '127.0.0.1', '--port', str(args.port)],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding='utf-8', errors='replace')
        procs.append(('book', book))
        print('book:     http://127.0.0.1:%d' % args.port)
    print('runner:   http://127.0.0.1:%d' % args.runner_port)
    print('Ctrl-C to stop.')
    try:
        threads = [threading.Thread(target=pump, args=(t, p), daemon=True)
                   for t, p in procs]
        for t in threads:
            t.start()
        for _, p in procs:
            p.wait()
    except KeyboardInterrupt:
        pass
    finally:
        for _, p in procs:
            if p.poll() is None:
                p.terminate()


if __name__ == '__main__':
    main()
