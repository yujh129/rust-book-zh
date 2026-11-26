#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Local Rust playground runner for the book.

Serves the two endpoints mdBook's book.js talks to on play.rust-lang.org:

    POST /meta/crates    -> {"crates": [{"id": "tokio"}, ...]}
    POST /evaluate.json  -> {"result": "<compiler errors or program output>"}
    GET  /health         -> {"ok": true, ...}

Code is compiled with rustc (no external crate) or with the offline cargo
project in tools/playground/ when a third-party crate is referenced.

Usage:  python tools/playground_server.py [--port 3030]
"""
import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.join(HERE, 'playground')
MAIN_RS = os.path.join(PROJ, 'src', 'main.rs')
EXE = os.path.join(PROJ, 'target', 'debug', 'playground.exe' if os.name == 'nt' else 'playground')
BUILD_TIMEOUT = 90
RUN_TIMEOUT = 4.0
TEST_TIMEOUT = 20

# crates available in tools/playground/Cargo.toml, reported to book.js
CRATES = ['anyhow', 'clap', 'rand', 'serde', 'serde_json', 'thiserror',
          'tokio', 'tracing', 'tracing_subscriber']

# identifiers that mean "this snippet wants a third-party crate"
CRATE_PATTERN = re.compile(
    r'\b(?:tokio|serde|serde_json|clap|rand|anyhow|thiserror|tracing|'
    r'tracing_subscriber|futures|bytes|chrono|uuid|regex)::')
DERIVE_PATTERN = re.compile(r'#\[\s*derive\s*\([^)]*(?:Serialize|Deserialize|'
                            r'Parser|Subcommand|Args|ValueEnum|Error)')
EXTERN_PATTERN = re.compile(r'\bextern\s+crate\s+(\w+)')

BUILD_LOCK = threading.Lock()

# snippets that contain tests are compiled and run as a test harness
TEST_PATTERN = re.compile(
    r'#\[\s*test\s*\]|#\[\s*should_panic|#\[\s*cfg\s*\(\s*test\s*\)')

# cargo's own bookkeeping warnings, not part of what the reader should see
CARGO_NOISE_START = ('warning: unused dependency', 'warning: unused manifest key',
                     'warning: unused patch', 'warning: unused config key')


def needs_cargo(code):
    return bool(CRATE_PATTERN.search(code) or DERIVE_PATTERN.search(code)
                or EXTERN_PATTERN.search(code))


def is_test_program(code):
    return bool(TEST_PATTERN.search(code))


def strip_cargo_noise(text):
    """Drop cargo bookkeeping warnings (unused dependencies and friends)."""
    kept = []
    skipping = False
    for line in text.splitlines():
        stripped = line.strip()
        if line.startswith(CARGO_NOISE_START):
            skipping = True
            continue
        if skipping:
            if not stripped:
                skipping = False
                continue
            if (stripped.startswith(('-->', '|', '=', 'help:', 'note:'))
                    or re.match(r'^\d+\s*\|', stripped)
                    or line.startswith((' ', chr(9)))):
                continue
            skipping = False
        kept.append(line)
    while kept and not kept[0].strip():
        kept.pop(0)
    while kept and not kept[-1].strip():
        kept.pop()
    return ('\n'.join(kept) + '\n') if kept else ''


def run(cmd, cwd, timeout, env=None):
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, env=env,
                          capture_output=True, text=True, encoding='utf-8',
                          errors='replace')


def cargo_errors(cargo_args=('build',)):
    """Compile only, return (errors_text, ok)."""
    p = run(['cargo'] + list(cargo_args) + ['--offline', '--message-format=json',
             '--color', 'never'], PROJ, BUILD_TIMEOUT)
    errors = []
    for line in (p.stdout or '').splitlines():
        line = line.strip()
        if not line.startswith('{'):
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
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
        return ''.join(errors), False
    if p.returncode != 0:
        tail = (p.stderr or '').strip().splitlines()[-12:]
        return 'error: cargo exited with %d\n%s\n' % (p.returncode,
                                                       '\n'.join(tail)), False
    return '', True


def cargo_build_errors():
    return cargo_errors(('build',))


def run_tests():
    """Run the snippet as a test harness and return what cargo test prints."""
    try:
        p = run(['cargo', 'test', '--offline', '--quiet', '--color', 'never'],
                PROJ, TEST_TIMEOUT)
    except subprocess.TimeoutExpired:
        return ('note: the test binary did not finish within %.0f seconds (killed)\n'
                % TEST_TIMEOUT)
    text = (p.stdout or '') + strip_cargo_noise(p.stderr or '')
    return text


def run_program():
    """Run the built binary and return its combined output."""
    if not os.path.exists(EXE):
        return 'error: build produced no executable\n', False
    started = time.time()
    try:
        p = subprocess.Popen([EXE], cwd=PROJ, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, encoding='utf-8',
                             errors='replace', stdin=subprocess.DEVNULL)
        try:
            out, err = p.communicate(timeout=RUN_TIMEOUT)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
            text = (out or '') + (err or '')
            text += ('\nerror: program did not finish within %.0f seconds '
                     '(killed)\n' % RUN_TIMEOUT)
            return text, False
    except OSError as exc:
        return 'error: could not start the program: %s\n' % exc, False
    text = (out or '') + (err or '')
    if not text:
        return '', True
    if p.returncode not in (0, None):
        text += '\nnote: program exited with code %d after %.2fs\n' % (
            p.returncode, time.time() - started)
    return text, True


def tidy(text):
    """Make the runner output look like rustc on unix: forward-slashed paths
    and no thread id in panic messages."""
    text = text.replace('src' + chr(92) + 'main.rs', 'src/main.rs')
    text = re.sub(r"thread '([^']*)' \(\d+\) panicked", r"thread '\1' panicked", text)
    return text


def evaluate(code):
    code = (code or '').replace('\r\n', '\n')
    if not code.strip():
        return 'error: no code to run\n'
    with BUILD_LOCK:
        with open(MAIN_RS, 'w', encoding='utf-8', newline='\n') as fh:
            fh.write(code if code.endswith('\n') else code + '\n')
        if is_test_program(code):
            errors, ok = cargo_errors(('test', '--no-run'))
            if not ok:
                return tidy(errors)
            return tidy(run_tests())
        errors, ok = cargo_build_errors()
        if not ok:
            return tidy(errors)
        out, _ = run_program()
        return tidy(out)


class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    server_version = 'rustbook-playground/1.0'

    def log_message(self, fmt, *args):
        sys.stderr.write('[pg] %s %s\n' % (self.address_string(), fmt % args))

    def _cors(self):
        self.send_header('Access-Control-Allow-Origin', '*')
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

    def do_OPTIONS(self):
        self.send_response(204)
        self._cors()
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_GET(self):
        path = self.path.split('?')[0]
        if path in ('/health', '/'):
            self._send({'ok': True, 'server': self.server_version,
                        'crates': CRATES, 'project': PROJ})
        else:
            self._send({'error': 'not found'}, 404)

    def do_POST(self):
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
                result = evaluate(code)
            except subprocess.TimeoutExpired:
                result = 'error: compilation timed out\n'
            except Exception as exc:                      # noqa: BLE001
                result = 'error: runner failure: %r\n' % (exc,)
            mode = 'test' if is_test_program(code) else (
                'cargo' if needs_cargo(code) else 'rustc')
            sys.stderr.write('[pg] %d bytes, %s mode, %.2fs\n' % (
                len(code), mode, time.time() - started))
            self._send({'result': result, 'success': True})
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
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print('\nbye')
    finally:
        srv.server_close()


if __name__ == '__main__':
    main()
