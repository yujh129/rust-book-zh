# 端到端冒烟：用真正的 socket 打 RESP 协议，逐字节核对回复。
# 用法：python smoke_test.py [端口]
import socket
import sys

HOST = "127.0.0.1"
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 6399
fails = []


def conn():
    s = socket.create_connection((HOST, PORT), timeout=5)
    s.settimeout(5)
    return s


def recv_exact(s, n):
    buf = b""
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk:
            raise EOFError("connection closed")
        buf += chunk
    return buf


def read_line(s):
    buf = b""
    while not buf.endswith(b"\r\n"):
        c = s.recv(1)
        if not c:
            raise EOFError("connection closed")
        buf += c
    return buf


def read_reply(s):
    head = read_line(s)
    if head[:1] == b"$":
        n = int(head[1:-2])
        if n < 0:
            return head
        return head + recv_exact(s, n + 2)
    return head


def send(s, payload):
    s.sendall(payload)
    return read_reply(s)


def check(label, got, want):
    ok = got == want
    if not ok:
        fails.append("%s: got %r want %r" % (label, got, want))
    print("%s %s: %r" % ("PASS" if ok else "FAIL", label, got))


def to_int(reply):
    assert reply[:1] == b":" and reply.endswith(b"\r\n"), reply
    return int(reply[1:-2])


a = conn()
check("PING", send(a, b"PING\r\n"), b"+PONG\r\n")
check("SET inline", send(a, b"SET greeting hello\r\n"), b"+OK\r\n")
check("GET inline", send(a, b"GET greeting\r\n"), b"$5\r\nhello\r\n")
check("SET RESP array", send(a, b"*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\nbar\r\n"), b"+OK\r\n")
check("GET RESP array", send(a, b"*2\r\n$3\r\nGET\r\n$3\r\nfoo\r\n"), b"$3\r\nbar\r\n")
check("GET missing -> $-1", send(a, b"GET nope\r\n"), b"$-1\r\n")
check("EXISTS 1", send(a, b"EXISTS foo\r\n"), b":1\r\n")
check("DEL 1", send(a, b"DEL foo\r\n"), b":1\r\n")
check("DEL 0", send(a, b"DEL foo\r\n"), b":0\r\n")
check("EXISTS 0", send(a, b"EXISTS foo\r\n"), b":0\r\n")
check("INCR new", send(a, b"INCR hits\r\n"), b":1\r\n")
check("INCR again", send(a, b"INCR hits\r\n"), b":2\r\n")
check("INCR non-int", send(a, b"INCR greeting\r\n"), b"-" + "值不是整数".encode() + b"\r\n")
check("EXPIRE missing", send(a, b"EXPIRE nope 10\r\n"), b":0\r\n")
check("EXPIRE ok", send(a, b"EXPIRE greeting 10\r\n"), b":1\r\n")
check("TTL persistent", send(a, b"TTL hits\r\n"), b":-1\r\n")
check("TTL missing", send(a, b"TTL nope\r\n"), b":-2\r\n")
ttl = to_int(send(a, b"TTL greeting\r\n"))
check("TTL 0..10", 0 <= ttl <= 10, True)
check("KEYS $13 greeting hits", send(a, b"KEYS\r\n"), b"$13\r\ngreeting hits\r\n")
check("unknown cmd", send(a, b"FOO bar\r\n"), b"-" + "未知命令 FOO".encode() + b"\r\n")
check("connection survives error", send(a, b"PING\r\n"), b"+PONG\r\n")
check("SET with EX", send(a, b"SET tmp v EX 100\r\n"), b"+OK\r\n")
ttl2 = to_int(send(a, b"TTL tmp\r\n"))
check("TTL after EX 100", 0 <= ttl2 <= 100, True)

# 验收标准 3：两个连接共享同一份数据。
b = conn()
check("client B sees A data", send(b, b"GET greeting\r\n"), b"$5\r\nhello\r\n")
check("client B INCR", send(b, b"INCR hits\r\n"), b":3\r\n")
check("client A sees B write", send(a, b"GET hits\r\n"), b"$1\r\n3\r\n")
check("client B FLUSHALL", send(b, b"FLUSHALL\r\n"), b"+OK\r\n")
check("client A sees flush", send(a, b"KEYS\r\n"), b"$0\r\n\r\n")

# QUIT：先回 +OK 再关连接。
check("QUIT", send(a, b"QUIT\r\n"), b"+OK\r\n")
check("QUIT closes conn", a.recv(1), b"")
b.close()

print()
if fails:
    print("SMOKE FAIL")
    for f in fails:
        print("  " + f)
    sys.exit(1)
print("SMOKE OK")
