# 第 42 章 项目四：mini-redis——内存数据库与 RESP 协议

> **本章目标**
>
> - 读懂 RESP（REdis Serialization Protocol）的五种回复形态与两类请求，明白为什么数据库协议要用长度前缀而不是换行分隔
> - 写一个能处理 TCP 粘包与半包的状态无关解析器：`parse_command(&[u8]) -> Result<Option<(Vec<String>, usize)>, String>`
> - 用 `HashMap<String, Entry>` 加每次读写时的惰性过期，实现 SET/GET/DEL/EXISTS/INCR/KEYS/EXPIRE/TTL
> - 把内存结构包进 `Arc<Mutex<Store>>`，用「每连接一个线程」的模型同时服务多个客户端
> - 用同一个 crate 同时产出库与二进制，让 9 个单元测试与 4 个端到端测试都能 `cargo test` 一键跑通

前三个项目（minigrep、JSON 解析器、HTTP 服务器）处理的都是「一问一答就结束」的连接。这一章要做的东西不一样：客户端连着不断开，一条连接上可能连续发出几百条命令，服务端要一直记住状态。这就是数据库的样子。

我们照着 Redis 的线协议做一个精简版：`mini-redis`。它不支持主从、不支持集群、不支持持久化，但协议解析、过期语义、并发共享这几块是真的，代码大约 600 行（不含测试），全部只用标准库。

## 42.1 需求与验收标准

先钉死范围。一个数据库最小的可用集合是「读、写、删、计数、查过期」：

| 命令 | 语法 | 回复 | 说明 |
| --- | --- | --- | --- |
| PING | PING | +PONG | 探活，客户端启动时先发它 |
| SET | SET key value [EX 秒] | +OK | 覆盖写；带 EX 时同时设过期 |
| GET | GET key | $len 换行后的值，或 $-1 | 键不存在返回空 bulk |
| DEL | DEL key | :1 / :0 | 返回真正删掉的个数 |
| EXISTS | EXISTS key | :1 / :0 | 过期的键视为不存在 |
| INCR | INCR key | :新值 | 键不存在按 0 处理；非整数报错 |
| KEYS | KEYS | $len 后跟空格分隔的键 | 排序后返回，方便测试断言 |
| EXPIRE | EXPIRE key 秒 | :1 / :0 | 只对已存在的键生效 |
| TTL | TTL key | :秒 / :-1 / :-2 | -1 永久，-2 不存在 |
| FLUSHALL | FLUSHALL | +OK | 清空，测试之间复位用 |
| QUIT | QUIT | +OK | 回复后服务端主动关连接 |

验收标准写成三条，后面每写完一层就对照一次：

1. `cargo test` 全绿：9 个单元测试（协议解析 4 个、存储语义 4 个、回复编码 1 个）加 4 个端到端测试，多出来的那个专门盯第 3 条验收标准。
2. 真实客户端能连上来发命令，收到逐字节符合 RESP 的回复。
3. 两个客户端同时连上，一个写一个读，读到的是同一份数据——这就逼出了 `Arc<Mutex<..>>`。

## 42.2 RESP：为什么要长度前缀

最容易想到的协议是「一行一条命令，用换行分隔」，HTTP/1.1 的请求行就是这样。但数据库要存的值里可以有换行：`SET log "line1\nline2"` 用换行分隔就彻底没法解析了。

RESP 的答案是**先把长度告诉我，再给我这么多字节**。这样值里出现什么字符都不影响切分，解析器只需要数字节，不需要猜。

回复一共五种形态，首字节就是类型标签：

| 首字节 | 名字 | 例子 | 含义 |
| --- | --- | --- | --- |
| + | Simple String | +OK 回车换行 | 状态，不能含 CRLF |
| - | Error | -未知命令 FOO | 错误，客户端应按异常处理 |
| : | Integer | :42 | 64 位整数 |
| $ | Bulk String | $5 换行 hello | 二进制安全字符串；$-1 表示不存在 |
| * | Array | *2 后跟两个 bulk | 请求基本都是这种 |

注意 `$-1` 这个空值：它不能和「空字符串」混为一谈。`$0` 加一个空行是「存在但是空」，`$-1` 是「不存在」。很多手写客户端在这里翻车，把两者都当成空串，于是 `EXISTS` 的判断永远为真。

请求有两种写法，服务端都要支持：

```text
# inline 形式：一行，空格分隔，方便用 telnet/nc 手测
PING

SET greeting hello


# RESP 数组形式：redis-cli 和所有真实客户端都用这个
*3
$3
SET
$8
greeting
$5
hello

```

inline 形式不能存带空格的值，也不能存二进制，所以只用来调试。真正要保证的是数组形式。

### TCP 不保证「一次读一条命令」

这是网络编程最容易被忽略的一点。`TcpStream::read` 的语义是「有多少字节先给我多少」，它可能：

- 只读到半条命令（`*3` 加 `$3` 加 `SE`）——**半包**；
- 一次读到两条命令（`PING` 换行 `PING` 换行）——**粘包**；
- 把一条命令拆成三次到。

所以解析函数不能用「读一行，处理一行」的写法，必须做到两点：

1. **纯函数**：只吃 `&[u8]`，不碰 socket，这样单元测试可以不建连接。
2. **返回消耗了多少字节**：成功时给出 `(args, used)`，调用方从缓冲区里切掉这 `used` 个字节，剩下的留给下一轮；数据还不够时返回 `Ok(None)`，表示「合法但没读完」，绝不报错。

签名因此长这样：

```rust,noplayground
pub fn parse_command(buf: &[u8]) -> Result<Option<(Vec<String>, usize)>, String>;
```

三层含义正好对应三种网络状况：`Ok(Some(..))` 拿到一条完整命令，`Ok(None)` 数据不够继续读，`Err(..)` 客户端发的东西根本不符合协议（这时候只能回一个错误然后断开，因为已经无法定位下一条命令的边界了）。

## 42.3 存储层：Entry 与惰性过期

存什么？一个键对应一个字符串，外加一个可选的到期时刻。

```rust,noplayground
struct Entry {
    value: String,
    expires_at: Option<Instant>,
}

pub struct Store {
    map: HashMap<String, Entry>,
}
```

`Instant` 而不是 `SystemTime`，这不是随手选的：`SystemTime` 会被 NTP 校时、时区设置、用户手动改钟影响，可能往回跳，导致「过期时间在未来某刻」的键突然变回未过期，也可能一下子全过期。`Instant` 是单调时钟，只会往前走，`Instant::now() + Duration::from_secs(30)` 的语义稳定。代价是它不能序列化（重启后没有参照），所以我们不做持久化。

过期有两种实现策略：

| 策略 | 做法 | 优点 | 缺点 |
| --- | --- | --- | --- |
| 惰性过期 | 每次访问某键时检查它是否过期，过期就删 | 零额外线程，CPU 只花在被访问的键上 | 没人访问的键会一直占内存 |
| 定时清扫 | 后台线程每隔一段时间扫一遍全表 | 内存能回收 | 扫全表在大库上很贵，还要处理并发 |

真实 Redis 两者都用（惰性加定期抽样）。我们的 mini 版本只做惰性过期，因为代码少一半而且语义正确：过期的键对任何客户端都表现为不存在。唯一「漏」的地方是 `KEYS`——它会遍历全表，所以那一处要顺手把过期项剔掉，否则用户会看到一个 `GET` 拿不到的键。

所有读写路径共用同一个私有函数，保证语义只有一份：

```rust,noplayground
fn live(&mut self, key: &str) -> Option<&mut Entry> {
    let expired = match self.map.get(key) {
        Some(e) => matches!(e.expires_at, Some(t) if t <= Instant::now()),
        None => return None,
    };
    if expired {
        self.map.remove(key);
        return None;
    }
    self.map.get_mut(key)
}
```

这段代码有个值得注意的地方：`self.map.get(key)` 拿到的 `Option<&Entry>` 借着 `self`，紧接着 `self.map.remove(key)` 需要 `&mut self`。如果先 `let entry = self.map.get(key)` 再在 `if` 里 remove，编译器会直接拒绝，报 `cannot borrow self.map as mutable because it is also borrowed as immutable`。我们只把「是否过期」这个 `bool` 取出来，借用就在 `match` 结束时释放了，后面的 `remove` 与 `get_mut` 才不会打架。这是第 6 章借用规则在真实代码里的样子。

`TTL` 的三个返回值是 Redis 的约定，别自己发明：

- `-2`：键不存在（或已过期）；
- `-1`：键存在但没有过期时间；
- 非负：还剩多少秒，向下取整。

## 42.4 解析器：一次只切一条命令

先写一个找 CRLF 的小工具。注意搜索范围是 `0..len-1`：CR 后面必须还有字节才可能凑成一对，否则取下一个字节就下标越界——这是手写解析器最常见的 panic 来源。

```rust,noplayground
fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    (from..buf.len().saturating_sub(1)).find(|i| buf[*i] == b'\r' && buf[*i + 1] == b'\n')
}
```

解析主体分两条路：首字节是 `*` 走数组形式，否则走 inline。数组形式的流程是：读元素个数 → 循环读每个 `$len` 头 → 按 len 取正文 → 校验结尾的 CRLF → 返回总消耗。

```rust,noplayground
pub fn parse_command(buf: &[u8]) -> Result<Option<(Vec<String>, usize)>, String> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf[0] != b'*' {
        let Some(end) = find_crlf(buf, 0) else {
            return Ok(None);
        };
        let line = std::str::from_utf8(&buf[..end]).map_err(|_| String::from("非法 UTF-8"))?;
        let parts: Vec<String> = line.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            return Err(String::from("空命令"));
        }
        return Ok(Some((parts, end + 2)));
    }
    let Some(end) = find_crlf(buf, 0) else {
        return Ok(None);
    };
    let count: usize = std::str::from_utf8(&buf[1..end])
        .map_err(|_| String::from("非法 UTF-8"))?
        .parse()
        .map_err(|_| String::from("数组长度非法"))?;
    let mut pos = end + 2;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        if pos >= buf.len() {
            return Ok(None);
        }
        if buf[pos] != b'$' {
            return Err(String::from("期望 bulk string"));
        }
        let Some(hdr_end) = find_crlf(buf, pos) else {
            return Ok(None);
        };
        let len: usize = std::str::from_utf8(&buf[pos + 1..hdr_end])
            .map_err(|_| String::from("非法 UTF-8"))?
            .parse()
            .map_err(|_| String::from("bulk 长度非法"))?;
        let body_start = hdr_end + 2;
        // len 也是客户端给的，直接相加可能整数溢出。
        let Some(body_end) = body_start.checked_add(len) else {
            return Err(String::from("bulk 长度非法"));
        };
        if buf.len() < body_end + 2 {
            return Ok(None);
        }
        let s = std::str::from_utf8(&buf[body_start..body_end])
            .map_err(|_| String::from("非法 UTF-8"))?;
        args.push(s.to_string());
        pos = body_end + 2;
    }
    Ok(Some((args, pos)))
}
```

几处细节：

- 判断用 `buf.len() < body_end + 2` 而不是 `< body_end`：正文后面还有两个字节的 CRLF，少一个字节都算没读完。写成后者会在下一步 `pos = body_end + 2` 时越界。
- `body_start + len` 用 `checked_add` 而不是直接相加：`len` 同样是客户端说了算，发一个 `$18446744073709551615` 的头就能让加法在 debug 构建下整数溢出、直接 panic。溢出当协议错误处理，连复用连接的机会都不给。
- 所有 `Err` 都是**协议错误**，不是**命令错误**。`SET` 少了一个参数是命令错误，应该回一个 RESP 错误但仍保持连接；而 `*3` 后面跟 `XX` 这种是协议错误，说明字节流已经没法继续切了，只能断开。
- 用 `String` 存参数而不是 `&[u8]`：省掉生命周期标注，代价是不能存二进制值。真实 Redis 的键值都是字节串，如果要做二进制安全，把 `String` 换成 `Vec<u8>`，把 utf8 校验删掉即可。

四个单元测试覆盖了三种网络状况和一个粘包场景：

```rust,noplayground
#[test]
fn parses_inline_command() {
    let (cmd, used) = parse_command(b"PING\r\n").unwrap().unwrap();
    assert_eq!(cmd, vec![String::from("PING")]);
    assert_eq!(used, 6);
}

#[test]
fn parses_resp_array() {
    let raw = b"*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\nbar\r\n";
    let (cmd, used) = parse_command(raw).unwrap().unwrap();
    assert_eq!(cmd, vec![String::from("SET"), String::from("foo"), String::from("bar")]);
    assert_eq!(used, raw.len());
}

#[test]
fn incomplete_input_returns_none() {
    assert!(parse_command(b"*2\r\n$3\r\nSE").unwrap().is_none());
    assert!(parse_command(b"*2\r\n$3\r\nSET\r\n$3\r\nfo").unwrap().is_none());
}

#[test]
fn two_commands_in_one_buffer() {
    let raw = b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPING\r\n";
    let (first, used) = parse_command(raw).unwrap().unwrap();
    assert_eq!(first, vec![String::from("PING")]);
    let (second, _) = parse_command(&raw[used..]).unwrap().unwrap();
    assert_eq!(second, vec![String::from("PING")]);
}
```

运行 `cargo test` 时这四条会输出 `test result: ok. 9 passed`（同一批里还有存储层的测试），说明「半包返回 `None`、粘包能切第二次」这两个最要命的行为是对的。

## 42.5 回复编码：一个枚举管住五种形态

回复侧同样不需要为每种命令写自己的字符串拼接。把 RESP 的五种形态映射成一个枚举，编码只有一处实现，命令逻辑只负责挑一个变体：

```rust,noplayground
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Simple(String),
    Bulk(Option<String>),
    Integer(i64),
    Error(String),
}

impl Reply {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Reply::Simple(s) => {
                out.extend_from_slice(format!("+{}\r\n", s).as_bytes());
            }
            Reply::Bulk(Some(s)) => {
                out.extend_from_slice(format!("${}\r\n{}\r\n", s.len(), s).as_bytes());
            }
            Reply::Bulk(None) => out.extend_from_slice(b"$-1\r\n"),
            Reply::Integer(n) => out.extend_from_slice(format!(":{}\r\n", n).as_bytes()),
            Reply::Error(e) => out.extend_from_slice(format!("-{}\r\n", e).as_bytes()),
        }
        out
    }
}
```

`Bulk(Some(s))` 里的 `s.len()` 是**字节数**不是字符数——RESP 的长度单位是字节，而 Rust 的 `String::len()` 恰好也返回字节数，所以这里对上了。如果哪天把值换成 `Vec<char>` 或者用了 `chars().count()`，中文值就会立刻错位：`你好` 是 2 个字符却是 6 个字节，客户端会读 2 个字节然后拿后面 4 个字节去当下一条回复的头部，整个连接从此错乱。第 7 章讲 `&str` 切片时强调过这件事，在这里它有直接的后果。

`Error` 走的是 RESP 的 `-` 类型，客户端会把它当异常抛出来，所以错误文本里不能出现 CRLF，否则一条回复会被切成两条。我们的错误文案都是单行中文，天然满足。

用 `#[derive(PartialEq, Eq)]` 不是为了功能，是为了测试里能直接断言：

```rust,noplayground
#[test]
fn replies_have_correct_wire_format() {
    assert_eq!(Reply::Simple(String::from("OK")).to_bytes(), b"+OK\r\n".to_vec());
    assert_eq!(Reply::Bulk(None).to_bytes(), b"$-1\r\n".to_vec());
    assert_eq!(Reply::Bulk(Some(String::from("hi"))).to_bytes(), b"$2\r\nhi\r\n".to_vec());
    assert_eq!(Reply::Integer(42).to_bytes(), b":42\r\n".to_vec());
    assert_eq!(Reply::Error(String::from("bad")).to_bytes(), b"-bad\r\n".to_vec());
}
```

## 42.6 命令分发：一个 match 加守卫

分发就是「取第一个参数转大写，剩下的叫 rest，然后按命令名 match」。参数个数用 `match` 守卫（`if rest.len() == 1`）在模式里顺手校验，不合法时落到下一个分支返回错误：

```rust,noplayground
pub fn execute(store: &mut Store, args: &[String]) -> Reply {
    if args.is_empty() {
        return Reply::Error(String::from("未知命令"));
    }
    let cmd = args[0].to_uppercase();
    let rest = &args[1..];
    match cmd.as_str() {
        "PING" => Reply::Simple(String::from("PONG")),
        "SET" if rest.len() >= 2 => {
            if rest.len() >= 4 && rest[2].eq_ignore_ascii_case("EX") {
                match rest[3].parse::<u64>() {
                    Ok(secs) => store.set(&rest[0], &rest[1], Some(Duration::from_secs(secs))),
                    // EX 后面不是数字：这是命令错误，不能悄悄当成「不过期」。
                    Err(_) => return Reply::Error(String::from("EX 的秒数非法")),
                }
            } else {
                store.set(&rest[0], &rest[1], None);
            }
            Reply::Simple(String::from("OK"))
        }
        "SET" => Reply::Error(String::from("SET 需要 key 和 value")),
        "GET" if rest.len() == 1 => Reply::Bulk(store.get(&rest[0])),
        "GET" => Reply::Error(String::from("GET 需要恰好一个 key")),
        "DEL" if rest.len() == 1 => Reply::Integer(store.del(&rest[0]) as i64),
        "EXISTS" if rest.len() == 1 => Reply::Integer(store.exists(&rest[0]) as i64),
        "INCR" if rest.len() == 1 => match store.incr(&rest[0]) {
            Ok(n) => Reply::Integer(n),
            Err(e) => Reply::Error(e.to_string()),
        },
        "KEYS" => Reply::Bulk(Some(store.keys().join(" "))),
        "EXPIRE" if rest.len() == 2 => match rest[1].parse::<u64>() {
            Ok(secs) => Reply::Integer(store.expire(&rest[0], secs) as i64),
            Err(_) => Reply::Error(String::from("EXPIRE 的秒数非法")),
        },
        "TTL" if rest.len() == 1 => Reply::Integer(store.ttl(&rest[0])),
        "FLUSHALL" => {
            *store = Store::new();
            Reply::Simple(String::from("OK"))
        }
        "QUIT" => Reply::Simple(String::from("OK")),
        _ => Reply::Error(format!("未知命令 {}", args[0])),
    }
}
```

三个设计决定值得说清楚：

**命令名大小写不敏感，键名大小写敏感。** `cmd.to_uppercase()` 只作用于第一个参数，`SET Foo bar` 存的就是 `Foo`。真实 Redis 也是这样，客户端和服务端的约定是「命令大写、键原样」。

**`as i64` 转换 bool。** `store.del(key)` 返回 `bool`，协议要的是 `:1` 或 `:0`。`as` 在 `bool → i64` 上是精确定义的：`true` 变 1、`false` 变 0，不会截断（第 2 章那套 `as` 截断的担忧只针对数值型之间）。

**未知命令返回错误但不断连接。** 这是 `Err` 与「`Reply::Error`」的分工：前者是协议层面的灾难（字节流没法继续切），后者只是一个业务错误，客户端还能接着发下一条命令。typo 打错命令不该让连接断掉，`redis-cli` 里连续敲错几次仍然能继续就是这个原因。

`FLUSHALL` 用 `*store = Store::new()` 而不是 `store.map.clear()`：`Store` 里只有一个 `HashMap`，整体替换更直白，而且以后加了别的字段（比如统计计数）也不会漏掉。这是测试里复位用的，客户端真跑业务时用不到。

## 42.7 连接循环与并发共享

现在到了最难写对的一段：把字节从 socket 读进来、切出命令、执行、把回复写回去，而且在一条连接上循环反复。

```rust,noplayground
pub fn handle_connection(mut stream: TcpStream, store: Arc<Mutex<Store>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        buf.extend_from_slice(&chunk[..n]);
        loop {
            match parse_command(&buf) {
                Ok(Some((args, used))) => {
                    buf.drain(..used);
                    // *0 是合法输入，会解析出空参数表；直接取 args[0] 会先 panic。
                    let quit = args.first().is_some_and(|a| a.eq_ignore_ascii_case("QUIT"));
                    let reply = {
                        let mut guard = store.lock().unwrap();
                        execute(&mut guard, &args)
                    };
                    if stream.write_all(&reply.to_bytes()).is_err() {
                        return;
                    }
                    if quit {
                        return;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = stream.write_all(&Reply::Error(e).to_bytes());
                    return;
                }
            }
        }
    }
}
```

这段只有 30 行，但每一行都有理由：

- `Ok(0)` 是 TCP 的「对端关闭」，不是错误。漏掉这个分支会让读循环永远转下去，在一个已经死掉的连接上空烧 CPU。
- 外层读、内层解析的嵌套结构，是粘包的正解：一次 `read` 可能带回三条命令，所以必须在内层把缓冲区里的命令**全部**处理完，直到 `parse_command` 返回 `None` 才回去再读。
- `buf.drain(..used)` 把已消费的字节从头部删掉。这里用 `Vec<u8>` 而不是 `VecDeque`，因为命令通常很小、`drain` 一次就清空，`Vec` 的连续内存更快；真遇到「客户端狂发命令而我们处理得慢」的极端情况，`Vec::drain` 的搬移会成为瓶颈，那时换成环形缓冲区。
- **锁的作用域被一对花括号钉死在 `execute` 上**，`write_all` 在锁外。如果把 `write_all` 放进锁里，一个慢客户端（读得慢、发送缓冲区满）就能拖住所有其他客户端——它们的命令排在同一个 Mutex 后面，全局串行化。抢锁写数据是网络服务最容易犯的性能错误之一。
- `Err` 分支回一个错误就 `return`，理由前面说过：字节流已经不可信。

`QUIT` 的处理有个次序问题：先回复 `+OK` 再关闭，还是直接关掉？我们按 Redis 的语义先回复再返回，因为客户端可能正等着读这一条。

服务端主体就是把每来一个连接丢给一个线程：

```rust,noplayground
pub fn serve(listener: TcpListener, store: Arc<Mutex<Store>>) -> std::io::Result<()> {
    for conn in listener.incoming() {
        let stream = conn?;
        let store = Arc::clone(&store);
        std::thread::spawn(move || handle_connection(stream, store));
    }
    Ok(())
}
```

`Arc::clone(&store)` 必须在循环**里面**，每轮产生一个新的句柄交给新线程。如果先 clone 一次再在循环里用那个变量，`move` 闭包会在第一次迭代就把它移走，第二次迭代时编译器报 `the type Arc does not implement Copy`——这正是本章错误案例里最经典的一条。

测试侧再给一个小工具，让端到端测试能读一行回复：

```rust,noplayground
pub fn read_reply(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim_end().to_string())
}
```

`read_line` 来自 `BufRead` trait，只 `use std::io::BufReader` 是不够的，必须同时 `use std::io::BufRead`——这个报错我在写这一章时真的踩了一次，报错原文见下面的错误案例。`try_clone()` 复制的是文件描述符（Windows 上是对应的句柄），两个 `TcpStream` 指向同一个连接，读写互不影响。

但上面这个 `read_reply` 只适合测试里**读一次**：它每次调用都新建一个 `BufReader`，函数一返回缓冲区就被丢掉，而 `BufReader` 会一次从 socket 读一大块——如果它顺手把「下一条回复」的字节也读进了自己的缓冲区，那些字节就永久丢了。要连续读多条回复，客户端必须让同一个 `BufReader` 活到对话结束，并且把 bulk 的正文也读走：

```rust,noplayground
/// 长期存活的读取器：缓冲区不丢，bulk 正文也一起读掉。
pub fn read_reply(reader: &mut BufReader<TcpStream>) -> std::io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let head = line.trim_end().to_string();
    if let Some(len) = head.strip_prefix('$').and_then(|n| n.parse::<usize>().ok()) {
        let mut body = vec![0u8; len + 2];
        reader.read_exact(&mut body)?;
    }
    Ok(head)
}
```

`--selftest` 用的就是这个版本：读一整个回复，但只把头部返回去打印，所以输出里 `$5` 后面看不到 `hello`。

`cargo test` 的实际输出（`--offline` 因为本机没网，依赖只有标准库，不影响结果）：

```console
running 9 tests
test parse_command::tests::incomplete_input_returns_none ... ok
test parse_command::tests::parses_inline_command ... ok
test parse_command::tests::parses_resp_array ... ok
test parse_command::tests::two_commands_in_one_buffer ... ok
test store::tests::expired_key_disappears ... ok
test store::tests::incr_rejects_non_integer ... ok
test store::tests::set_get_del_roundtrip ... ok
test store::tests::ttl_reports_missing_and_persistent ... ok
test reply::tests::replies_have_correct_wire_format ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 4 tests
test end_to_end_set_get ... ok
test resp_array_over_tcp ... ok
test two_clients_share_state ... ok
test unknown_command_returns_error ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

二进制侧还要支持一个 `--selftest` 模式，自己起服务、自己连自己发几条命令，方便在没有 `redis-cli` 的机器上验证。实际运行输出：

```console
$ cargo run --offline -- 6399 --selftest
mini-redis 已启动：127.0.0.1:6399
发送 "PING" → 收到 "+PONG"
发送 "SET greeting hello" → 收到 "+OK"
发送 "GET greeting" → 收到 "$5"
发送 "INCR hits" → 收到 ":1"
发送 "KEYS" → 收到 "$13"
```

`GET greeting` 只打印 `$5`，因为 `--selftest` 用的读取函数会把 `hello` 的正文一起读掉（否则下一次读到的就是正文而不是回复头），但只把头部返回、打印出来。断言时我们只关心头部类型与长度：`$5` 已经说明「值存在且长 5 字节」，而长度的正确性由单元测试 `replies_have_correct_wire_format` 逐字节保证。真实客户端会把 `5 + 2` 个字节的正文交给业务层。

## 42.8 深入：这 600 行背后的取舍

### 每连接一线程能撑多少

`std::thread::spawn` 不是免费的。一个线程在 Linux 上默认预留 8 MiB 栈（虚拟地址，不是物理内存），创建本身要 50–150 微秒，还要进内核登记调度实体。一千个空转的连接线程就已经让调度器开始吃力，一万个基本没戏。

| 模型 | 每连接成本 | 适合的连接数 | 代码复杂度 |
| --- | --- | --- | --- |
| 每连接一线程（本章） | 一个线程 + 8 MiB 虚拟栈 | 数百到数千 | 最低 |
| 线程池 + 阻塞 IO | 固定线程数，连接排队 | 数千 | 中（要自己写队列） |
| 事件循环 + 非阻塞 IO（Tokio） | 一个 task 几十字节到几 KB | 十万级 | 高（要处理状态机） |

真实 Redis 走的是第三条路的「单线程版」：一个线程用 `epoll`/`kqueue` 同时盯着几万个 socket，谁有数据就处理谁，命令执行本身是串行的。所以 Redis 的所有命令都是原子的，不需要给数据结构加锁——它的「锁」就是「只有一个线程在跑命令」。

我们选了第一条路，因为它让 `Store` 的并发问题退化成「加一把大锁」，不用写任何状态机。代价是连接数上不去，但作为教学项目和本地缓存够用了。真要换成 Tokio，`handle_connection` 里每个 `read`/`write` 变成 `.await`，`Mutex` 换成 `tokio::sync::Mutex`（或者在锁内不 await 就仍用 `std::sync::Mutex`），其余结构几乎不变——这正是第 25 章想说明的：异步改的是等待方式，不是业务逻辑。

### 为什么是 Mutex 而不是 RwLock

直觉上「读多写少」应该用 `RwLock`。但这个服务的读写比例取决于客户端，`SET` 和 `GET` 一样常见；而 `std::sync::RwLock` 的实现（Windows 上是 SRWLock）在读锁竞争时开销比 `Mutex` 高，写者还可能被持续到达的读者饿死。更关键的是**临界区极短**：`execute` 里只是查一次 `HashMap`，纳秒级。锁的开销几乎全在抢锁那一瞬间，读写分离带来的收益抵不过复杂度。

判断标准可以记成一句话：**临界区短且读写混合，用 Mutex；临界区里明显是纯读且耗时长（比如遍历大表做统计），才考虑 RwLock。**

### 缓冲区没有上限，就是个 DoS 洞

`buf` 是个会无限增长的 `Vec`。客户端只要发一个 `*100000000` 的头，然后慢慢喂数据，我们的 `Vec::with_capacity(count)` 会立刻尝试分配 8 亿个 `String` 的位置——直接 abort 或者被 OOM killer 干掉。真实 Redis 对 `*` 后面的数字有上限（1M），对单个 bulk 长度也有上限（512 MB）。

修法很简单，在 `execute` 之前加两道闸：

```rust,noplayground
const MAX_ARGS: usize = 1024;
const MAX_BULK: usize = 8 * 1024 * 1024;
const MAX_BUFFER: usize = 64 * 1024 * 1024;

// parse_command 里读 count 之后：
if count > MAX_ARGS {
    return Err(format!("参数过多：{}", count));
}
// 读 len 之后：
if len > MAX_BULK {
    return Err(format!("单个参数过长：{}", len));
}
// handle_connection 的读循环里：
if buf.len() > MAX_BUFFER {
    let _ = stream.write_all(&Reply::Error(String::from("请求过大")).to_bytes());
    return;
}
```

另外 `Vec::with_capacity(count)` 本身就危险：`count` 来自客户端，在还没收到任何字节时就按它预分配。稳妥的写法是先 `Vec::new()`，让 push 自己增长，或者 `count.min(MAX_ARGS)`。这是「不要相信外部输入的长度字段」的典型例子。

### Nagle 算法：小包为什么慢

TCP 默认开着 Nagle 算法：小数据先攒着，等收到对方的 ACK 或者攒够一个 MSS 再发。这对批量传输是好事，对「发一条命令等一条回复」的请求响应模式是灾难——每条 `PING` 可能白等 40 毫秒的延迟确认。

所以真实的客户端和服务器几乎都会：

```rust,noplayground
stream.set_nodelay(true)?;
```

它关掉 Nagle，每个 `write` 立刻发包。代价是如果应用层自己一条一条发小数据，网络上的包会变多。对数据库这种「一问一答」的协议，这个交换绝对划算。

### 惰性过期的内存账

一个键设了 60 秒过期，然后再也没有人访问它。它会在 `HashMap` 里躺到进程退出，占着键和值的内存。如果业务是「每次请求生成一个临时 token 并设 5 分钟过期」，一天下来内存曲线就是一条斜线。

三个层次的对策：

1. **定期抽样**（Redis 的做法）：每秒随机取 20 个带过期时间的键，删掉过期的；如果超过 1/4 都过期了，就在这一轮里继续抽。抽样而不是全扫，保证 CPU 摊还。
2. **上限 + LRU**：键数超过阈值时按最近使用时间淘汰。要在 `Entry` 里加 `last_used: Instant`，并在每次 `get` 时更新——注意这会让 `get` 也需要 `&mut self`。
3. **定时全扫**：实现最简单，但大库上一次扫描的停顿会很明显，只在键数很少时可用。

我们的实现停在「惰性 + `KEYS` 时清理」这一步，够用于教学；如果你要把它用到真实场景，`KEYS` 本身也该改成 `SCAN` 那样的游标式遍历，否则一次 `KEYS` 就能阻塞所有客户端（因为它在锁里做全表遍历）。

## 42.9 典型错误案例

### 案例 1：忘了引入 BufRead trait

```rust,noplayground
use std::io::BufReader;
use std::net::TcpStream;

fn main() {
    let stream = TcpStream::connect("127.0.0.1:6379").unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    println!("{}", line);
}
```

```text
error[E0599]: no method named `read_line` found for struct `BufReader<R>` in the current scope
   --> src/main.rs:8:12
    |
  8 |     reader.read_line(&mut line).unwrap();
    |            ^^^^^^^^^ method not found in `BufReader<TcpStream>`
    |
   ::: C:\Users\yujinhao\.rustup\toolchains\stable-x86_64-pc-windows-msvc\lib/rustlib/src/rust\library/alloc/src/io/buf_read.rs:342:8
    |
342 |     fn read_line(&mut self, buf: &mut String) -> Result<usize> {
    |        --------- the method is available for `BufReader<TcpStream>` here
    |
    = help: items from traits can only be used if the trait is in scope
help: trait `BufRead` which provides `read_line` is implemented but not in scope; perhaps you want to import it
    |
  1 + use std::io::BufRead;
    |
```

**原因**：`read_line`、`lines`、`read_until` 这些方法不是 `BufReader` 自带的，而是 `BufRead` trait 提供的方法。Rust 的方法解析只查找「已引入作用域的 trait」，所以 `use std::io::BufReader` 让你能构造类型，却不让你调用 trait 方法。报错里的 `the method is available for BufReader<TcpStream> here` 已经把话说得很清楚。

**修复**：把 trait 也引入。注意 rustc 给的补丁行号是 `1 +`，意思是插在第一行，实际写的时候两条 `use` 可以合并：

```rust,no_run
use std::io::{BufRead, BufReader};
use std::net::TcpStream;

fn main() -> std::io::Result<()> {
    let stream = TcpStream::connect("127.0.0.1:6379")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    println!("{}", line.trim_end());
    Ok(())
}
```

这类错误的识别特征是「方法明明存在，rustc 却在 `:::` 里指着标准库某个 `fn`」。看到 `items from traits can only be used if the trait is in scope` 就直接按 help 加 `use`。

### 案例 2：把 bool 当计数返回

```rust,noplayground
fn main() {
    let mut store = miniredis::Store::new();
    let removed: i64 = store.del("k");
    println!("{}", removed);
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:3:24
  |
3 |     let removed: i64 = store.del("k");
  |                  ---   ^^^^^^^^^^^^^^ expected `i64`, found `bool`
  |                  |
  |                  expected due to this
```

**原因**：`Store::del` 的签名返回 `bool`（「到底删掉了没有」），因为惰性过期让「键不存在」和「键已过期」都表现为 `false`。而 RESP 的 `:` 类型要的是整数。两者语义相关但类型不同，编译器不会替你隐式转换——Rust 没有 C 那样的 bool 到整数的隐式提升。

**修复**：在分发层显式转换。`bool as i64` 是 Rust 明确定义的行为（`true` 变 1、`false` 变 0），比写 `if x { 1 } else { 0 }` 更短且没有歧义：

```rust,noplayground
"DEL" if rest.len() == 1 => Reply::Integer(store.del(&rest[0]) as i64),
```

顺带一提：反过来 `i64 as bool` 是不允许的（会报 `cannot cast `i64` as `bool`），因为非 0 值到 true 的映射在 C 里就是无数 bug 的来源，Rust 拒绝了这个转换。要用 `n != 0`。

### 案例 3：给只读引用改数据

```rust,noplayground
fn bump(store: &miniredis::Store) {
    store.set("k", "v", None);
}

fn main() {
    let mut s = miniredis::Store::new();
    bump(&s);
    println!("{:?}", s.get("k"));
}
```

```text
error[E0596]: cannot borrow `*store` as mutable, as it is behind a `&` reference
 --> src/main.rs:2:5
  |
2 |     store.set("k", "v", None);
  |     ^^^^^ `store` is a `&` reference, so it cannot be borrowed as mutable
  |
help: consider changing this to be a mutable reference
  |
1 | fn bump(store: &mut miniredis::Store) {
  |                 +++
```

**原因**：`set` 需要 `&mut self`（它要往 `HashMap` 里写），而参数是 `&Store`。共享引用背后是只读视图，Rust 不允许从只读视图借出可变引用。`main` 里那个 `s` 虽然声明成了 `mut`，但 `bump` 的签名决定了传进去的只是个只读借用。

**修复**：把签名改成 `&mut`，调用处改成 `bump(&mut s)`。如果这个函数确实拿不到独占引用（比如它从 `Arc<Store>` 里来），那说明设计有问题，应该改用 `Mutex`/`RwLock` 或者内部可变性——这也是我们整个服务用 `Arc<Mutex<Store>>` 而不是 `Arc<Store>` 的原因：所有客户端共享的是「一把锁」，锁的 `lock()` 方法只需要 `&self`，它自己负责在内部拿到独占权。

```rust,noplayground
fn bump(store: &mut miniredis::Store) {
    store.set("k", "v", None);
}

fn main() {
    let mut s = miniredis::Store::new();
    bump(&mut s);
    println!("{:?}", s.get("k"));
}
```

### 案例 4：在循环里移动 Arc

```rust,noplayground
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let store = Arc::new(Mutex::new(miniredis::Store::new()));
    for conn in listener.incoming() {
        let stream = conn.unwrap();
        std::thread::spawn(move || miniredis::handle_connection(stream, store));
    }
}
```

```text
error[E0382]: the type `Arc` does not implement `Copy`
  --> src/main.rs:9:28
   |
 6 |     let store = Arc::new(Mutex::new(miniredis::Store::new()));
   |         ----- this move could be avoided by cloning the original `Arc`, which is inexpensive
 7 |     for conn in listener.incoming() {
   |     ------------------------------- inside of this loop
 8 |         let stream = conn.unwrap();
 9 |         std::thread::spawn(move || miniredis::handle_connection(stream, store));
   |                            ^^^^^^^                                      ----- use occurs due to use in closure
   |                            |
   |                            value moved into closure here, in previous iteration of loop
   |
   = note: consider using `Arc::clone`
help: consider cloning the value before moving it into the closure
   |
 9 ~         let value = store.clone();
10 ~         std::thread::spawn(move || miniredis::handle_connection(stream, value));
   |

For more information about this error, try `rustc --explain E0382`.
error: could not compile `miniredis` (example "e4b") due to 1 previous error
```

**原因**：`move` 闭包会把捕获的变量**移动**进去，而 `store` 在循环外只声明了一次。第一轮迭代把它移进第一个线程后，第二轮就没有可移的了。`Arc` 名字里带「引用计数」，很容易让人误以为它是 Copy 的——不是。`Arc<T>` 只是一个含指针和计数的结构体，按值传递时照样是移动。想共享就要显式增加计数，那个动作就是 `Arc::clone`。

**修复**：在循环体内 clone 一个新句柄给线程，原句柄留在外面继续用：

```rust,noplayground
for conn in listener.incoming() {
    let stream = conn.unwrap();
    let store = Arc::clone(&store);
    std::thread::spawn(move || miniredis::handle_connection(stream, store));
}
```

注意 clone 出来的变量**同名遮蔽**了外层的 `store`，这是有意的：闭包里那份是线程自己的，外层那份下一轮继续 clone。`Arc::clone(&x)` 比 `x.clone()` 更推荐，因为它明确表达「我要的是 Arc 的克隆（加计数），不是里面 T 的克隆」——如果哪天 `Store` 实现了 `Clone`，`store.clone()` 会被解析成克隆整个哈希表，那是个巨大的性能陷阱。

### 案例 5：用 `Rc<RefCell>` 共享给线程

```rust,noplayground
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn main() {
    let store: Rc<RefCell<HashMap<String, String>>> = Rc::new(RefCell::new(HashMap::new()));
    let shared = Rc::clone(&store);
    std::thread::spawn(move || {
        shared.borrow_mut().insert(String::from("k"), String::from("v"));
    });
    println!("{}", store.borrow().len());
}
```

```text
error[E0277]: `Rc<RefCell<HashMap<String, String>>>` cannot be sent between threads safely
   --> src/main.rs:8:24
    |
  8 |       std::thread::spawn(move || {
    |       ------------------ ^------
    |       |                  |
    |  _____|__________________within this `{closure@src/main.rs:8:24: 8:31}`
    | |     |
    | |     required by a bound introduced by this call
  9 | |         shared.borrow_mut().insert(String::from("k"), String::from("v"));
 10 | |     });
    | |_____^ `Rc<RefCell<HashMap<String, String>>>` cannot be sent between threads safely
    |
    = help: within `{closure@src/main.rs:8:24: 8:31}`, the trait `Send` is not implemented for `Rc<RefCell<HashMap<String, String>>>`
note: required because `shared` is used within this closure
   --> src/main.rs:9:9
    |
  9 |         shared.borrow_mut().insert(String::from("k"), String::from("v"));
    |         ^^^^^^
note: required by a bound in `spawn`
   --> C:\Users\yujinhao\.rustup\toolchains\stable-x86_64-pc-windows-msvc\lib/rustlib/src/rust\library/std\src\thread\functions.rs:128:8
    |
125 | pub fn spawn<F, T>(f: F) -> JoinHandle<T>
...
128 |     F: Send + 'static,
    |        ^^^^ required by this bound in `spawn`

For more information about this error, try `rustc --explain E0277`.
error: could not compile `miniredis` (example "e7") due to 1 previous error
```

**原因**：`Rc` 的引用计数是**普通整数**，不是原子操作。两个线程同时 clone 或 drop，计数就会错乱：要么提前归零导致 use-after-free，要么永远不归零导致泄漏。所以 `Rc<T>` 没有实现 `Send`。`RefCell` 更糟，它的借用标志也是普通 `Cell<isize>`，两个线程同时 `borrow_mut` 会双双通过检查，直接造成数据竞争。`Rc<RefCell<T>>` 这个组合在单线程里是神器（第 18、19 章的主角），一旦跨线程就是双重违规。

**修复**：换成 `Arc<Mutex<T>>`。`Arc` 的计数用原子指令，`Mutex` 用系统锁保证同一时刻只有一个线程能进临界区：

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    let store = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let shared = Arc::clone(&store);
    std::thread::spawn(move || {
        shared.lock().unwrap().insert(String::from("k"), String::from("v"));
    })
    .join()
    .unwrap();
    println!("{}", store.lock().unwrap().len());
}
```

这段会打印 `1`。判断一个类型能不能跨线程，看两个 auto trait 就够：`Send`（能把所有权移到别的线程）和 `Sync`（能被多个线程同时 `&` 引用）。`Rc`/`RefCell`/裸指针都不是 `Send`；`Arc<T>` 是 `Send` 当且仅当 `T: Send + Sync`。第 21、22 章有完整的对照表。

## 完整源码

完整实现按层次拆在 `projects/42-mini-redis/`（<https://github.com/yujh129/rust-book-zh/tree/main/projects/42-mini-redis>），零依赖。没有 `redis-cli` 也能验证：

```console
$ cd projects/42-mini-redis
$ cargo run --offline -- --selftest
mini-redis 已启动：127.0.0.1:6379
发送 "PING" → 收到 "+PONG"
发送 "SET greeting hello" → 收到 "+OK"
发送 "GET greeting" → 收到 "$5"
发送 "INCR hits" → 收到 ":1"
发送 "KEYS" → 收到 "$13"
```

不带 `--selftest` 就是普通服务器：`cargo run --offline -- 6380`，再用 `redis-cli -p 6380` 或 `nc 127.0.0.1 6380` 连上去。`cargo test --offline` 跑 9 个单元测试加 4 个端到端测试。

| 文件 | 内容 |
| --- | --- |
| `src/parse_command.rs` | RESP 命令的纯函数解析器，处理半包与粘包，含 `*0`、`$` 长度溢出这些边界 |
| `src/store.rs` | `Entry` 与 `Store`，惰性过期 |
| `src/reply.rs` | 五种回复形态的编码（`+` `-` `:` `$` `*`） |
| `src/lib.rs` | 命令分发（含 `SET … EX` 的校验）、连接循环、`serve` |
| `src/main.rs` | 端口参数与 `--selftest` |
| `tests/end_to_end.rs` | 4 个端到端测试，含 `two_clients_share_state` |
| `smoke_test.py` | 30 项逐字节的协议冒烟测试，自己起服务、自己连 |

## 42.10 动手练习

1. **加计数命令**：实现 `INCRBY key n` 与 `DECR key`，要求 `n` 不是整数、键值不是整数、加减溢出（用 `i64::checked_add`）三种情况都返回 RESP 错误而不是 panic。再补三个单元测试。
2. **补资源上限**：按 42.8 末尾的片段加上 `MAX_ARGS`/`MAX_BULK`/`MAX_BUFFER`，并写一个测试：向 `parse_command` 传入 `*2000` 开头的缓冲区，断言它返回 `Err` 而不是试图分配。
3. **改造 KEYS**：现在的 `KEYS` 在锁内做全表遍历并 `join`，键一多就会长时间持锁。改成返回 `Vec<String>` 的 `keys_snapshot()`，在锁内只做 clone，把排序和拼接移到锁外。测一下十万键时的差异。
4. **换成线程池**：把「每连接一线程」改成「固定 4 个 worker 线程 + `mpsc` 任务队列」，连接到来时把 `TcpStream` 塞进队列。用 1000 个并发连接压一下，比较两种模型的成功连接数与内存占用（任务提示：工作线程从队列取到一个连接后要一直服务到它断开，这其实退化成了「4 个线程轮流被一个长连接占死」，想一想为什么这是错的，正确的做法是什么）。
5. **测 Nagle 的影响**：写一个循环发 1000 次 `PING` 并计时的基准程序，分别在服务端 `set_nodelay(true)` 与默认设置下各跑一次。在本地回环上差异可能只有几毫秒，但把客户端放到另一台机器上（或者用 `tc`/`clumsy` 注入 40ms 延迟）差异会非常明显——解释为什么。

## 42.11 小结

- RESP 用「长度前缀 + 定长正文」解决「值里含分隔符」的问题，五种回复各由一个首字节标识：`+` 状态、`-` 错误、`:` 整数、`$` bulk、`*` 数组。
- `$-1` 是「不存在」，`$0` 加空行是「存在且为空」，两者语义不同，客户端不能都当空串。
- TCP 是字节流：必须假设半包与粘包都会发生。解析器要写成纯函数，返回 `Ok(Some((值, 消耗字节)))`/`Ok(None)`/`Err` 三态。
- 过期时间用 `Instant`（单调时钟）而不是 `SystemTime`（会被校时回拨）。
- 惰性过期实现简单、语义正确，但没人访问的键不会释放内存；生产环境要么定期抽样，要么加上限与淘汰。
- 所有共享状态用 `Arc<Mutex<Store>>`：`Arc` 负责跨线程共享所有权，`Mutex` 负责临界区互斥。锁的作用域要卡在最短的 `execute` 上，绝不能让网络 IO 持锁。
- `Arc` 不是 `Copy`：循环里给每个线程发句柄必须 `Arc::clone(&store)`，并且要放在循环体内。
- 每连接一线程的模型简单但扩展性差（8 MiB 虚拟栈/线程），上万连接要换事件循环；这正是 Tokio 存在的理由。
- 任何来自外部的长度字段（`*N`、`$N`）都必须先校验上限再分配内存，否则就是一个远程 OOM 漏洞。
- `cargo test` 的 9 个单元测试 + 4 个端到端测试覆盖了协议、存储、并发三条主线；写网络程序时，把「解析」和「IO」拆开是可测试性的关键。