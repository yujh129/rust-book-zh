#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Local Rust playground runner for the book.

Serves the two endpoints mdBook's book.js talks to on play.rust-lang.org:

    POST /meta/crates    -> {"crates": [{"id": "tokio"}, ...]}
    POST /evaluate.json  -> {"result": "<compiler errors or program output>"}
    GET  /health         -> {"ok": true, ...}

All snippets are built with the offline Cargo project in tools/playground/.

Usage:  python tools/playground_server.py [--port 3030]
"""
import argparse
import json
import os
import re
import subprocess
import sys
import threading
from urllib.parse import urlsplit
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.join(HERE, 'playground')
MAIN_RS = os.path.join(PROJ, 'src', 'main.rs')
BUILD_TIMEOUT = 90
RUN_TIMEOUT = 4.0
TEST_TIMEOUT = 20

# crates available in tools/playground/Cargo.toml, reported to book.js
CRATES = ['anyhow', 'clap', 'rand', 'serde', 'serde_json', 'thiserror',
          'tokio', 'tracing', 'tracing_subscriber']

BUILD_LOCK = threading.Lock()
# 启动时在后台把依赖编译一遍，浏览器第一次点 Run 就不用等冷编译
WARM = threading.Event()

# snippets that contain tests are compiled and run as a test harness
TEST_PATTERN = re.compile(
    r'#\[\s*test\s*\]|#\[\s*should_panic|#\[\s*cfg\s*\(\s*test\s*\)')

def is_test_program(code):
    return bool(TEST_PATTERN.search(code))


def run(cmd, cwd, timeout, env=None):
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, env=env,
                          capture_output=True, text=True, encoding='utf-8',
                          errors='replace')


def compile_snippet(cargo_args):
    """Read the executable path from Cargo, including custom target directories."""
    p = run(['cargo'] + list(cargo_args) + ['--offline', '--message-format=json',
             '--color', 'never'], PROJ, BUILD_TIMEOUT)
    executable = None
    errors = []
    for line in (p.stdout or '').splitlines():
        line = line.strip()
        if not line.startswith('{'):
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get('reason') == 'compiler-artifact' and msg['target']['name'] == 'playground' and msg.get('executable'):
            executable = msg['executable']
        if msg.get('reason') != 'compiler-message':
            continue
        inner = msg.get('message') or {}
        if inner.get('level') != 'error':
            continue
        rendered = inner.get('rendered')
        if not rendered:
            rendered = 'error: ' + str(inner.get('message', '')) + '\n'
        errors.append(rendered)
    if errors:
        return ''.join(errors), None
    if p.returncode != 0:
        tail = (p.stderr or '').strip().splitlines()[-12:]
        return 'error: cargo exited with %d\n%s\n' % (p.returncode,
                                                       '\n'.join(tail)), None
    return '', executable


def run_program(executable, timeout):
    if not executable:
        return 'error: build produced no executable\n', False
    started = time.time()
    try:
        p = subprocess.Popen([executable], cwd=PROJ, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, encoding='utf-8',
                             errors='replace', stdin=subprocess.DEVNULL)
        try:
            out, err = p.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
            text = (out or '') + (err or '')
            text += ('\nerror: program did not finish within %.0f seconds '
                     '(killed)\n' % timeout)
            return text, False
    except OSError as exc:
        return 'error: could not start the program: %s\n' % exc, False
    text = (out or '') + (err or '')
    if p.returncode not in (0, None):
        text += '\nnote: program exited with code %d after %.2fs\n' % (
            p.returncode, time.time() - started)
    return text, p.returncode == 0


def tidy(text):
    """Make the runner output look like rustc on unix: forward-slashed paths
    and no thread id in panic messages."""
    text = text.replace('src' + chr(92) + 'main.rs', 'src/main.rs')
    text = re.sub(r"thread '([^']*)' \(\d+\) panicked", r"thread '\1' panicked", text)
    return text


def warm_up():
    """Build the playground project once, so the first Run in the browser is fast."""
    with BUILD_LOCK:
        try:
            # 上一轮校验可能留下一个只有 #[test] 的 main.rs（没有 fn main，cargo build 会失败）
            with open(MAIN_RS, 'w', encoding='utf-8', newline='\n') as fh:
                fh.write('fn main() {}\n')
            started = time.time()
            p = run(['cargo', 'build', '--offline', '--quiet', '--color', 'never'],
                    PROJ, BUILD_TIMEOUT * 4)
            sys.stderr.write('[pg] warm-up %s in %.1fs\n' % (
                'ok' if p.returncode == 0 else 'failed (%d)' % p.returncode,
                time.time() - started))
        except subprocess.TimeoutExpired:
            sys.stderr.write('[pg] warm-up timed out\n')
        finally:
            WARM.set()


def evaluate(code):
    code = (code or '').replace('\r\n', '\n')
    if not code.strip():
        return 'error: no code to run\n', False
    with BUILD_LOCK:
        with open(MAIN_RS, 'w', encoding='utf-8', newline='\n') as fh:
            fh.write(code if code.endswith('\n') else code + '\n')
        test = is_test_program(code)
        args = ('test', '--no-run') if test else ('build',)
        errors, executable = compile_snippet(args)
        if errors:
            return tidy(errors), False
        out, ok = run_program(executable, TEST_TIMEOUT if test else RUN_TIMEOUT)
        return tidy(out), ok


class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    server_version = 'rustbook-playground/1.0'

    def log_message(self, fmt, *args):
        sys.stderr.write('[pg] %s %s\n' % (self.address_string(), fmt % args))

    def _cors(self):
        origin = self.headers.get('Origin')
        if origin and self.local_request():
            self.send_header('Access-Control-Allow-Origin', origin)
            self.send_header('Vary', 'Origin')
        self.send_header('Access-Control-Allow-Methods', 'POST, GET, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type')
        self.send_header('Access-Control-Max-Age', '86400')

    def _send(self, obj, code=200, ctype='application/json; charset=utf-8'):
        body = obj if isinstance(obj, bytes) else json.dumps(obj).encode('utf-8')
        self.send_response(code)
        self._cors()
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def local_request(self):
        local = ('localhost', '127.0.0.1', '::1')
        origin = self.headers.get('Origin')
        try:
            host = urlsplit('//' + self.headers.get('Host', '')).hostname
            page = urlsplit(origin) if origin else None
            return host in local and (page is None or (page.scheme in ('http', 'https') and page.hostname in local))
        except ValueError:
            return False

    def do_OPTIONS(self):
        if not self.local_request():
            self.send_error(403)
            return
        self.send_response(204)
        self._cors()
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_GET(self):
        path = self.path.split('?')[0]
        if path in ('/health', '/'):
            self._send({'ok': True, 'server': self.server_version,
                        'crates': CRATES, 'project': PROJ,
                        'warming': not WARM.is_set()})
        else:
            self._send({'error': 'not found'}, 404)

    def do_POST(self):
        if not self.local_request():
            self.send_error(403)
            return
        if self.headers.get_content_type() != 'application/json':
            self.send_error(415)
            return
        path = self.path.split('?')[0]
        length = int(self.headers.get('Content-Length') or 0)
        raw = self.rfile.read(length) if length else b'{}'
        try:
            payload = json.loads(raw.decode('utf-8'))
        except (ValueError, UnicodeDecodeError):
            payload = {}
        if path in ('/meta/crates', '/meta/crates.json'):
            self._send({'crates': [{'id': c} for c in CRATES]})
            return
        if path in ('/evaluate.json', '/execute'):
            code = payload.get('code', '')
            started = time.time()
            try:
                result, ok = evaluate(code)
            except subprocess.TimeoutExpired:
                result, ok = 'error: compilation timed out\n', False
            except Exception as exc:                      # noqa: BLE001
                result, ok = 'error: runner failure: %r\n' % (exc,), False
            mode = 'test' if is_test_program(code) else 'cargo'
            sys.stderr.write('[pg] %d bytes, %s mode, %.2fs\n' % (
                len(code), mode, time.time() - started))
            self._send({'result': result, 'success': ok})
            return
        self._send({'error': 'not found'}, 404)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=3030)
    ap.add_argument('--host', default='127.0.0.1')
    args = ap.parse_args()
    if not os.path.exists(os.path.join(PROJ, 'Cargo.toml')):
        sys.exit('missing cargo project: %s' % PROJ)
    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    print('local playground running at http://%s:%d  (project: %s)' % (
        args.host, args.port, PROJ))
    threading.Thread(target=warm_up, name='warm-up', daemon=True).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print('\nbye')
    finally:
        srv.server_close()


if __name__ == '__main__':
    main()
