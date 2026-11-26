# 第 41 章 项目三：多线程 HTTP 服务器

> **本章目标**
>
> - 说清一个 HTTP/1.1 请求在字节层面由哪三部分组成，能手工写出一个合法请求
> - 只用 std::net、std::thread、std::fs 写出能响应 200、404、400、405 的静态文件服务器
> - 用 BufReader 按行读请求，并用头部字节上限与读超时挡住慢速攻击
> - 把 URL 映射成磁盘路径时挡住目录穿越与符号链接逃逸
> - 用 mpsc 加固定 worker 组成线程池，并说清 Ctrl-C 为什么绕过了清理逻辑

第 25 章用 tokio 写过服务端，框架把协议细节全包住了：你写一个路由函数，解析、连接复用、超时都由库负责。代价是出问题时只能猜——客户端发来的到底是哪一行不合法，响应为什么少一个字节就会卡住。这一章把框架拆开，只用标准库实现 HTTP/1.1 的一个最小子集。

限制很明确：不引入 axum、hyper，也不引入任何第三方 crate。全部依赖只有 std::net::TcpListener、std::thread 和 std::fs。这样做不是排斥框架，而是先看清框架替你做了什么，之后才有依据判断什么时候不该用它。

最终产物三百行上下（去掉空行与注释约 240 行）：能并发处理请求、能防目录穿越、能正确区分 404 与 400。后面各小节的代码块按顺序拼起来就是完整的 main.rs。

## 41.1 需求、验收标准与 HTTP/1.1 最小子集

先把「做完」定义清楚。程序启动后在 127.0.0.1:7878 监听，以当前目录下的 public 为根，满足下面五条就算通过：

```bash
curl -i http://127.0.0.1:7878/            # 200，返回 public/index.html
curl -i http://127.0.0.1:7878/style.css   # 200，Content-Type: text/css
curl -i http://127.0.0.1:7878/missing     # 404
curl -i "http://127.0.0.1:7878/%ZZ"       # 400，百分号转义非法
curl -i -X POST http://127.0.0.1:7878/    # 405，带 Allow: GET, HEAD
```

### 报文长什么样

HTTP/1.1 是文本协议，一个请求就是一段按行排列的 ASCII 字节。把 \r\n 显式写出来，一个最小的请求是：

```text
GET /index.html HTTP/1.1\r\n
Host: 127.0.0.1:7878\r\n
Accept: */*\r\n
\r\n
```

三部分，顺序固定：请求行（request line）、零到多个头部行（header field）、一个空行。请求行由方法、请求目标、协议版本三段组成，用单个空格分隔。头部行的格式是名字、冒号、值，冒号后面允许有空格。空行是分隔符，不是可选项——没有它，服务端无法判断头部在哪里结束。

HTTP/1.1 的 Host 头是必需的，因为同一个 IP 上可能托管多个域名。本章的实现里，1.1 请求缺 Host 直接回 400，这是最省事也最符合规范的做法。

响应也是同样的三段结构：状态行、头部、空行，然后是响应体。状态行写成 HTTP/1.1 加三位状态码加原因短语。本章只产出下面五种：

| 状态码 | 原因短语 | 本章触发条件 |
| --- | --- | --- |
| 200 | OK | 文件存在且读取成功 |
| 400 | Bad Request | 请求行字段数不对、版本不支持、缺 Host、百分号转义非法 |
| 404 | Not Found | 文件不存在、路径越界、目标是目录 |
| 405 | Method Not Allowed | 方法不是 GET 或 HEAD，响应带 Allow 头 |

### 为什么只做子集

完整的 HTTP/1.1 还要处理分块传输（chunked）、连接复用（keep-alive）、管道化、100-continue、压缩协商。每多一项，状态机就多一层。本章的选择是每响应一个请求就关闭连接，固定发 Connection: close，于是：

- 不需要解析请求体。GET 与 HEAD 没有体，其它方法直接 405，省掉 Content-Length 与 chunked 的读取逻辑。
- 不需要在连接上保存状态。读一个请求、写一个响应、关掉，循环简单到不会错。
- 代价是每个请求一次 TCP 三次握手。静态文件服务器上这个开销真实存在，41.6 会算这笔账。

## 41.2 服务端骨架：bind、incoming 与每连接一线程

技术选型只有一句话：零依赖。Cargo.toml 里 [dependencies] 留空，edition 用 2021。

```toml
[package]
name = "http-server"
version = "0.1.0"
edition = "2021"

[dependencies]
```

目录结构如下。public 是文档根目录（document root），服务器只在这个目录里找文件：

```text
http-server/
├── Cargo.toml
├── public/
│   ├── index.html
│   └── style.css
└── src/
    └── main.rs
```

public 里放两个最小的静态文件，后面 console 里的 `Content-Length` 就是按它们算的：

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <title>http-server</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <h1>Hello from std::net</h1>
  <p>只用标准库的多线程 HTTP/1.1 静态文件服务器。</p>
</body>
</html>
```

```css
body {
  margin: 2rem auto;
  max-width: 40rem;
  font-family: system-ui, sans-serif;
}

h1 {
  color: #1f6feb;
}
```

先写一个能跑通的最小版本：接受连接、读一次、回一段固定内容。它离成品很远，但验证了 bind 和 spawn 这两步。

```rust
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:7878")?;
    println!("listening on http://127.0.0.1:7878");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                thread::spawn(move || {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);

                    let body = b"hello from std\n";
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                });
            }
            Err(err) => eprintln!("accept failed: {}", err),
        }
    }

    Ok(())
}
```

几处细节值得停一下。

TcpListener::bind 的返回值是 Result，端口被占用时返回 Err。这里用 ? 直接向上抛，main 返回 io::Result 时打印的是 Debug 形式，够用。incoming() 返回一个阻塞迭代器：没有新连接时线程停在 accept 系统调用里，不占 CPU。

每个连接 spawn 一个线程，闭包必须加 move。stream 是 TcpStream，读写方法都要 &mut self，所以闭包里要声明 mut stream。闭包捕获的是所有权，主循环下一轮拿到的已经是另一个 stream，两者互不影响。

accept 返回 Err 时不能 unwrap。常见的一种是文件描述符耗尽（EMFILE）：进程打开的 fd 达到上限，accept 立刻失败，但监听套接字还活着。如果这里 panic，整个服务就退出；如果继续循环，下一轮很可能又失败，于是变成忙等。正确做法是记录错误、短暂退避，本章为了短就只记录。

这个版本的缺陷是线程数没有上限。一万个并发连接就是一万个 OS 线程，每个线程默认预留 8 MiB 栈空间（虚拟地址，按需提交），光是切换上下文就能把 CPU 吃满。41.5 换成固定大小的线程池。

## 41.3 解析请求：Request、parse 与路径安全

解析器要回答两个问题：这次请求合法吗，它想要哪个文件。第一个问题决定 400，第二个问题决定 200 还是 404。

### Request 为什么持有 String

最省事的写法是让 Request 借用读进来的缓冲区：

```rust,noplayground
struct Request<'a> {
    method: &'a str,
    target: &'a str,
}
```

这个设计在有生命周期的解析器里很常见，但它要求缓冲区活得比 Request 长。而实际调用链是 handle_connection 在栈上创建 BufReader，解析函数还想借用自己那行临时 String 返回 Request，函数一返回缓冲区就销毁，引用立刻悬空。这正是 41.7 案例 3 的报错。所以这里让 Request 拥有自己的 String：多一次分配，换掉一整类生命周期问题。

```rust
use std::fmt;
use std::io::{self, BufRead, BufReader, Cursor};

const MAX_HEADER_BYTES: usize = 8 * 1024;

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug)]
enum ParseError {
    Io(io::Error),
    BadRequest(&'static str),
}

impl From<io::Error> for ParseError {
    fn from(err: io::Error) -> Self {
        ParseError::Io(err)
    }
}

// 两个字段都真的被读到，日志里才有可用的信息（只派生 Debug 会被 dead_code 判为未读）。
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Io(err) => write!(f, "读取失败：{}", err),
            ParseError::BadRequest(reason) => write!(f, "请求不合法：{}", reason),
        }
    }
}

impl Request {
    fn parse<R: BufRead>(reader: &mut R) -> Result<Request, ParseError> {
        // 给整个请求头（含请求行）一个 8 KiB 的预算：Take 在预算用尽时让
        // read_line 返回 0，单行超长也会被截住，不会先把 String 撑大再检查。
        let mut limited = std::io::Read::take(reader, MAX_HEADER_BYTES as u64);
        let mut line = String::new();
        let n = limited.read_line(&mut line)?;
        if n == 0 {
            return Err(ParseError::BadRequest("请求行为空或头部超过 8 KiB"));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        let mut parts = line.split(' ');
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("");
        let version = parts.next().unwrap_or("");
        if method.is_empty() || target.is_empty() || parts.next().is_some() {
            return Err(ParseError::BadRequest("请求行字段数不对"));
        }
        if version != "HTTP/1.1" && version != "HTTP/1.0" {
            return Err(ParseError::BadRequest("不支持的协议版本"));
        }

        let mut headers = Vec::new();
        loop {
            let mut raw = String::new();
            let n = limited.read_line(&mut raw)?;
            if n == 0 {
                // 预算耗尽时 Take 也返回 0，把两种情况一起报出来。
                return Err(ParseError::BadRequest("头部未以空行结束或超过 8 KiB"));
            }
            if raw == "\r\n" || raw == "\n" {
                break;
            }
            let raw = raw.trim_end_matches(['\r', '\n']);
            let (name, value) = raw
                .split_once(':')
                .ok_or(ParseError::BadRequest("头部缺少冒号"))?;
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }

        Ok(Request {
            method: method.to_string(),
            target: target.to_string(),
            version: version.to_string(),
            headers,
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn main() {
    let raw = b"GET /index.html HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n";
    let mut reader = BufReader::new(Cursor::new(raw.to_vec()));
    match Request::parse(&mut reader) {
        Ok(request) => println!(
            "{} {} {} ({} 个头)",
            request.method,
            request.target,
            request.version,
            request.headers.len()
        ),
        Err(err) => println!("拒绝: {:?}", err),
    }
}
```

parse 的签名是泛型的：`fn parse<R: BufRead>(reader: &mut R)`。它不关心底层是 TcpStream 还是内存里的 Cursor，于是可以脱离网络测试——上面 main 就是干这个的。这也是第 15 章那条规则的应用：参数用 trait bound，不用具体类型。

几个容易写错的地方。

read_line 把换行符也读进来，返回值是读到的字节数，0 表示对端在写任何字节前就关闭了连接。这里选择宽松处理：用 trim_end_matches 把尾部的 \r 和 \n 全剥掉，于是 LF 结尾的请求也能接受。严格实现应该要求 CRLF，但现实里有手写的客户端只发 LF。

请求行用 split(' ') 而不是 split_whitespace()。规范里三段之间是单个空格，用 split(' ') 时空字段会立刻暴露出来，比如两个连续空格会让 parts.next() 拿到空串，进而被 method.is_empty() 拦住。

头部用一个 `Take` 限制在 8 KiB，请求行算在同一个预算里：`let mut limited = std::io::Read::take(reader, MAX_HEADER_BYTES as u64);`。这里要用 UFCS 写出接收者：`reader` 的类型是 `&mut R`，直接写 `reader.take(n)`（或 `reader.by_ref().take(n)`）时方法解析会落到 `R` 自己头上，编译器要把 `R` 从可变借用后面移出来，报 E0507（cannot move out of a mutable reference）；写成 `std::io::Read::take(reader, n)` 则是把 `&mut R` 这个引用本身交给 `take`，得到 `Take<&mut R>`，既限了额又没动 `R`。这一步必须放在读之前——如果在 `read_line` 返回之后再累计字节数，单行不写 CRLF 时 `String` 仍会一路涨到内存耗尽，上限形同虚设。8 KiB 是个折中：浏览器的请求头通常在 1 KiB 以内，带上 Cookie 也很少超过 4 KiB。没有上限时，一个只发 `X-Fill: a` 再逐行续写的客户端就能耗住线程，这类攻击叫慢速头部攻击。

头部存成 Vec 而不是 HashMap，是为了保留顺序并允许同名头重复（Set-Cookie 就会重复）。比较时统一转小写，HTTP 头部名本身大小写不敏感。

### 路径安全检查：字符串规则加前缀比对

拿到 target 后先去掉查询串。`/index.html?v=2` 里 ? 之后的内容不参与文件定位。然后是百分号解码：%2e%2e 就是 ..，如果先按字符串找 .. 再解码，攻击者只要编码一下就能绕过。

```rust
use std::path::{Path, PathBuf};

fn percent_decode(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(());
            }
            let hi = hex_value(bytes[i + 1]).ok_or(())?;
            let lo = hex_value(bytes[i + 2]).ok_or(())?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let mut full = root.to_path_buf();
    for segment in url_path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        full.push(segment);
    }
    if full == root {
        full.push("index.html");
    }

    let canonical = full.canonicalize().ok()?;
    if !canonical.starts_with(root) {
        return None;
    }
    if !canonical.is_file() {
        return None;
    }
    Some(canonical)
}

fn main() {
    let root = std::env::current_dir().expect("当前目录不可用");
    let root = root.canonicalize().expect("无法解析当前目录");
    for target in ["/..%2f..%2fsecret.txt", "/%ZZ", "/src/main.rs"] {
        match percent_decode(target) {
            Ok(path) => match resolve(&root, &path) {
                Some(file) => println!("{} -> {}", target, file.display()),
                None => println!("{} -> 404", target),
            },
            Err(()) => println!("{} -> 400", target),
        }
    }
    println!("root = {}", root.display());
}
```

函数分成两道防线，缺一不可。

第一道是逐段检查：把解码后的路径按 / 切开，遇到 .. 直接拒绝，遇到空段和 . 跳过。这一步不碰磁盘，代价接近零。

第二道是 canonicalize 之后比前缀。canonicalize 会展开 ..、解析符号链接、返回绝对路径，然后把结果与已经规范化的 root 做 starts_with 比较。为什么在已经拒绝 .. 之后还要这一步？因为字符串规则总有缝隙。Windows 上反斜杠也是路径分隔符，一个形如 ..\\..\\secret 的段不会被 segment == ".." 拦住，push 进去之后却会向上跳两层；Unix 上则是符号链接，public/link 指向 /etc 时，字符串检查看不出问题。canonicalize 之后路径已经唯一，前缀比对是最终判据。

starts_with 按路径分量比较，不是字符串前缀。root 是 /srv/public 时，/srv/public2/x 不会被误判为内部路径。

失败一律返回 None，上层映射成 404，而不是 403。403 会告诉攻击者「这个文件存在但不让你看」，等于免费提供一次探测结果。目录也返回 None：本章不做目录列表，那是练习 1 的事。

## 41.4 响应：手动拼状态行、头与体

响应结构体的字段就是状态行与头部的全部内容，body 用 `Vec<u8>` 而不是 `String`。静态文件可能是 PNG、WOFF2，任何一步把它当文本处理都会损坏字节。

```rust
use std::io::{self, Write};
use std::path::Path;

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    allow: Option<&'static str>,
}

impl Response {
    fn ok(content_type: &'static str, body: Vec<u8>) -> Response {
        Response {
            status: 200,
            reason: "OK",
            content_type,
            body,
            allow: None,
        }
    }

    fn plain(status: u16, reason: &'static str) -> Response {
        Response {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: format!("{} {}\n", status, reason).into_bytes(),
            allow: None,
        }
    }

    fn bad_request() -> Response {
        Response::plain(400, "Bad Request")
    }

    fn not_found() -> Response {
        Response::plain(404, "Not Found")
    }

    fn method_not_allowed() -> Response {
        Response {
            status: 405,
            reason: "Method Not Allowed",
            content_type: "text/plain; charset=utf-8",
            body: b"405 Method Not Allowed\n".to_vec(),
            allow: Some("GET, HEAD"),
        }
    }

    fn write_to<W: Write>(&self, out: &mut W, head_only: bool) -> io::Result<()> {
        write!(out, "HTTP/1.1 {} {}\r\n", self.status, self.reason)?;
        write!(out, "Content-Type: {}\r\n", self.content_type)?;
        write!(out, "Content-Length: {}\r\n", self.body.len())?;
        write!(out, "Connection: close\r\n")?;
        if let Some(allow) = self.allow {
            write!(out, "Allow: {}\r\n", allow)?;
        }
        write!(out, "\r\n")?;
        if !head_only {
            out.write_all(&self.body)?;
        }
        out.flush()
    }
}

fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn main() -> io::Result<()> {
    let response = Response::ok(
        "text/html; charset=utf-8",
        "<h1>你好</h1>".as_bytes().to_vec(),
    );
    let mut wire = Vec::new();
    response.write_to(&mut wire, false)?;
    print!("{}", String::from_utf8_lossy(&wire));
    println!("总字节数 = {}", wire.len());
    Ok(())
}
```

四个细节决定这个响应对不对。

Content-Length 必须是响应体的字节数。String::len() 返回的就是字节数，一个汉字占三字节，所以「你好」的体长是 6，不是 2。如果这里写成字符数，客户端会少读一半，然后卡在等待剩余字节上直到超时。

写体必须用 write_all。Write::write 只保证写进去一部分，返回实际写入的字节数。内核发送缓冲区满时它会只写几百字节，剩下的要你自己循环。write_all 就是那个循环，它返回 Ok 意味着整个切片都进了缓冲区。

write_to 的参数是泛型 W: Write，而不是 TcpStream。这让你能把它写到 `Vec<u8>` 里做断言（上面 main 就是这样），也让 41.7 的修复示例可以脱离网络运行。这是第 15 章「用 trait bound 而不是具体类型」的直接收益。

HEAD 请求只发头部。路由层把 head_only 传进来，Content-Length 照样写真实长度，但不写体。客户端靠这个头判断资源大小，不必下载。注意此时不能省 Content-Length，否则 HEAD 就失去了意义。

Connection: close 是本实现的核心简化：每个响应都声明连接到此为止，服务端写完就关，不解析后续请求。代价是每个请求一次三次握手，收益是不需要处理 keep-alive 的状态与超时。

## 41.5 并发：固定线程池与优雅退出

每连接一线程的问题是成本随连接数线性增长。线程池把并发度固定下来：主线程只做 accept，把连接交给固定数量的 worker，多出来的连接在通道里排队。

第 23 章讲过 mpsc 的所有权转移，这里正好用上。Job 是装箱的闭包，类型写成 `Box<dyn FnOnce() + Send + 'static>`：FnOnce 因为每个任务只执行一次，Send 因为要跨线程，'static 因为线程可能比创建它的栈帧活得久。

```rust
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct ThreadPool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ThreadPool {
    fn new(size: usize) -> ThreadPool {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);
        for _ in 0..size {
            let receiver = Arc::clone(&receiver);
            workers.push(thread::spawn(move || loop {
                let job = {
                    let guard = receiver.lock().unwrap();
                    guard.recv()
                };
                match job {
                    Ok(job) => job(),
                    Err(_) => break,
                }
            }));
        }
        ThreadPool {
            sender: Some(sender),
            workers,
        }
    }

    fn execute<F: FnOnce() + Send + 'static>(&self, job: F) {
        if let Some(sender) = &self.sender {
            if sender.send(Box::new(job)).is_err() {
                eprintln!("线程池已关闭");
            }
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn main() {
    let pool = ThreadPool::new(2);
    for id in 0..4 {
        pool.execute(move || println!("处理第 {} 个连接", id));
    }
}
```

Receiver 不能 clone，所以用 `Arc<Mutex<Receiver>>` 让多个 worker 共享。锁的范围只包住 recv，取到 job 之后立刻释放——上面用花括号把 guard 限制在语句里。如果写成 `while let Ok(job) = receiver.lock().unwrap().recv() { job() }` 并在 job() 期间一直持有 guard，所有 worker 会串行执行，线程池退化成单线程。

Drop 里的顺序也有讲究：先 drop(sender) 让所有 worker 的 recv 返回 Err 并退出循环，再逐个 join。如果先 join，worker 永远等不到通道关闭，程序就挂住了。

把前面几块拼起来：main 负责 bind、算好文档根目录、把连接丢进池；handle_connection 负责读请求、路由、写响应。下面这段引用了 41.3 与 41.4 的类型，单独贴出来不能编译，所以标成 noplayground。

```rust,noplayground
fn route(request: &Request, root: &Path) -> Response {
    if request.method != "GET" && request.method != "HEAD" {
        return Response::method_not_allowed();
    }
    if request.version == "HTTP/1.1" && request.header("host").is_none() {
        return Response::bad_request();
    }
    let raw_path = request.target.split('?').next().unwrap_or("/");
    let url_path = match percent_decode(raw_path) {
        Ok(path) => path,
        Err(()) => return Response::bad_request(),
    };
    match resolve(root, &url_path) {
        Some(file) => match fs::read(&file) {
            Ok(body) => Response::ok(mime_of(&file), body),
            Err(_) => Response::not_found(),
        },
        None => Response::not_found(),
    }
}

fn handle_connection(mut stream: TcpStream, root: &Path) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut reader = BufReader::new(&stream);
    let parsed = Request::parse(&mut reader);
    drop(reader);

    let (response, head_only) = match parsed {
        Ok(request) => (route(&request, root), request.method == "HEAD"),
        Err(err) => {
            eprintln!("请求被拒绝：{}", err);
            (Response::bad_request(), false)
        }
    };

    if let Err(err) = response.write_to(&mut stream, head_only) {
        eprintln!("write failed: {}", err);
    }
}
```

主程序只有十几行，文档根目录用 Arc 共享给各 worker：

```rust,noplayground
fn main() -> io::Result<()> {
    let root = Arc::new(fs::canonicalize("public")?);
    let listener = TcpListener::bind("127.0.0.1:7878")?;
    println!("serving {} at http://127.0.0.1:7878", root.display());

    let pool = ThreadPool::new(4);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let root = Arc::clone(&root);
                pool.execute(move || handle_connection(stream, &root));
            }
            Err(err) => eprintln!("accept failed: {}", err),
        }
    }
    Ok(())
}
```

把上面两个文件放进 public 后启动，实测输出如下。三行命令分别对应 200、404、400：

```console
$ curl -i http://127.0.0.1:7878/
HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8
Content-Length: 274
Connection: close

$ curl -i http://127.0.0.1:7878/missing
HTTP/1.1 404 Not Found
Content-Type: text/plain; charset=utf-8
Content-Length: 14
Connection: close

$ curl -i "http://127.0.0.1:7878/%ZZ"
HTTP/1.1 400 Bad Request
Content-Type: text/plain; charset=utf-8
Content-Length: 16
Connection: close
```

200 的响应体就是 index.html 的 274 字节，console 里为省空间省略了。404 与 400 的体分别是 `404 Not Found\n` 与 `400 Bad Request\n`，长度 14 与 16 对得上。

### Ctrl-C 与优雅退出

按 Ctrl-C 时，终端给进程发 SIGINT。标准库没有信号处理 API，默认行为是进程立刻终止：main 不会返回，ThreadPool 的 Drop 不会执行，正在写响应的 worker 被一起杀掉。

要让 Ctrl-C 触发优雅退出，需要接收停止信号，并让监听循环观察到它。例如用 `ctrlc` 的回调设置标志，再让非阻塞监听器在 `WouldBlock` 时检查标志并短暂休眠。主线程正常退出循环后才能运行线程池的析构逻辑。仅把 `accept` 改为非阻塞不会改变 Ctrl-C 的默认处理。

对静态文件服务器来说，硬退出通常可以接受：最坏情况是某个客户端收到半截响应，它会看到连接被重置并重试。真正需要优雅退出的场景是长连接与流式响应，那已经超出本章的范围。

## 41.6 深入：缓冲、超时与协议边界

**为什么必须用 BufReader。** TCP 是字节流，不保留消息边界。一次 read 可能只返回请求行的前三个字节，也可能把整个请求连同下一个请求一起给你。如果直接在 TcpStream 上实现按行读，你只有两个选择：每次 read 一个字节（每个字节一次系统调用，一次上下文切换的成本在微秒级，一个 300 字节的头部就是 300 次），或者自己维护一块缓冲和游标。BufReader 就是后者，默认 8 KiB 缓冲把系统调用摊薄到每 8 KiB 一次，read_line 只在这个缓冲里找换行，找不到才回内核再要一块。这就是 41.3 里 parse 的签名要求 R: BufRead 而不是 R: Read 的原因——按行读需要「能回退的缓冲」，裸 Read 不提供。

**慢速攻击与读超时。** 每连接一个线程的模型下，一个连接占一个线程。攻击者发完 `GET /` 之后每隔十秒补一个字节，线程就一直卡在 read_line 里，几百个这样的连接就能耗尽线程池。set_read_timeout 让 read 在超时后返回错误（Unix 上是 WouldBlock，Windows 上是 TimedOut），handle_connection 里把它记成 400 并关连接。注意这个超时是「每次 read」而不是「整个请求」：攻击者每次都在超时前发一个字节，依然能拖很久。所以还要有 41.3 的头部字节上限，必要时再加一个基于 Instant 的总时限。两个限制针对的是不同的攻击面，不能互相替代。

**HTTP 为什么是无状态的。** 协议本身不要求服务端记住上一个请求：每个请求必须自带全部上下文，身份放在 Cookie 或 Authorization 里，缓存策略放在头部里。keep-alive 只是在同一条 TCP 连接上连续发送多个请求，节省握手，并不改变「每个请求独立」这个语义。理解了这一点，就能明白为什么本章每响应一次就关连接仍然是对的——它只损失性能，不损失正确性。反过来，如果你的处理逻辑依赖「上一个请求是谁」，那是应用状态，必须自己存（内存表、Redis、数据库），协议不会帮你保存。

**keep-alive 与 Content-Length 的绑定关系。** 客户端判断响应体在哪里结束，只有三种依据：Content-Length 给出的字节数、chunked 编码的终止块、或者连接关闭。第三条一旦复用连接就失效，所以开启 keep-alive 的服务器必须给每条响应一个确定的长度，否则客户端会把下一个响应当成上一个的体。本章固定发 Connection: close，等于让客户端用「连接关闭」判断结束；即便这样，我们仍然写 Content-Length，因为 HEAD 请求与代理缓存都需要它。

**零拷贝。** 常规的静态文件发送路径是 read 把文件读进用户缓冲，再 write 把缓冲送进内核 socket，数据被拷贝两次，还多一次系统调用。Linux 的 sendfile(2) 让内核直接把页缓存里的数据送进 socket，省掉用户态那一趟；Windows 对应 TransmitFile，FreeBSD 也叫 sendfile。标准库没有封装这个调用，要用得引入 libc 或换 tokio 的异步文件接口；这也是 hyper、nginx 这类实现处理大文件时明显更快的原因之一。本章的实现每次读整个文件到 Vec，对小文件（几十 KB）无所谓，对几百 MB 的视频就会把内存吃满，正确做法是分块读、分块 write_all。

## 41.7 典型错误案例

前面几节把设计决定摆完了，现在换成编译器视角。下面五个错误都是这个项目里真会写出来的代码，报错取自本机 rustc 1.100.0，行号与代码块一一对应。看完这五段，41.3 到 41.5 里那些「为什么非得这样」就有了依据：不是作者偏好，是编译器只放这一条路过去。

### 案例 1：想从 &Request 里把 String 拿走——E0507

```rust,editable
struct Request {
    method: String,
    target: String,
}

fn route(request: &Request) -> String {
    request.target
}

fn main() {
    let request = Request {
        method: String::from("GET"),
        target: String::from("/index.html"),
    };
    println!("{}", route(&request));
}
```

```text
error[E0507]: cannot move out of `request.target` which is behind a shared reference
 --> src/main.rs:7:5
  |
7 |     request.target
  |     ^^^^^^^^^^^^^^ move occurs because `request.target` has type `String`, which does not implement the `Copy` trait
  |
help: consider cloning the value if the performance cost is acceptable
  |
7 |     request.target.clone()
  |                   ++++++++
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`route` 收到的是 `&Request`，它只被允许读。`request.target` 出现在返回值位置上，需要一个真正的 `String`，而 `String` 没有实现 `Copy`，唯一能交出值的方式是把堆缓冲的所有权搬走。从借来的数据里搬走所有权，调用者手里的 `request` 立刻变成悬空的壳，编译器用 E0507 在这里截住。

**修复**：两条路，按语义选。真要把字段交出去，就让 `route` 接收所有权；只想读，就传 `&request.target` 或 `request.target.as_str()`。41.4 的 `route(&request, root)` 之所以成立，是因为响应体来自 `fs::read` 新分配的 `Vec<u8>`，根本没有移动请求的字段。

```rust
struct Request {
    method: String,
    target: String,
}

fn route(request: Request) -> (String, String) {
    (request.method, request.target)
}

fn main() {
    let request = Request {
        method: String::from("GET"),
        target: String::from("/index.html"),
    };
    let (method, target) = route(request);
    println!("{} {}", method, target);
}
```

</details>

### 案例 2：在 &self 的处理函数里往 Vec 里塞日志——E0596

```rust,editable
struct Server {
    log: Vec<String>,
}

impl Server {
    fn handle(&self, target: &str) {
        self.log.push(target.to_string());
    }
}

fn main() {
    let server = Server { log: Vec::new() };
    server.handle("/index.html");
    println!("{}", server.log.len());
}
```

```text
error[E0596]: cannot borrow `self.log` as mutable, as it is behind a `&` reference
 --> src/main.rs:7:9
  |
7 |         self.log.push(target.to_string());
  |         ^^^^^^^^ `self` is a `&` reference, so it cannot be borrowed as mutable
  |
help: consider changing this to be a mutable reference
  |
6 |     fn handle(&mut self, target: &str) {
  |                +++
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Vec::push` 的接收者是 `&mut self`，而方法拿到的 `self` 是 `&Server`。共享借用不能凭空升级成可变借用，否则「同一时刻要么多个只读、要么一个可写」这条规则就破了。注意报错指在方法体内的 `self.log`，而不是调用点——这类错误的根在签名，改调用处没用。

**修复**：单线程下把接收者改成 `&mut self`。但 41.5 的线程池要求处理函数能被多个 worker 共享，那时 `&mut self` 传不进 `FnOnce` 闭包，得把可变状态关进 `Mutex`：

```rust
use std::sync::Mutex;

struct Server {
    log: Mutex<Vec<String>>,
}

impl Server {
    fn handle(&self, target: &str) {
        self.log.lock().unwrap().push(target.to_string());
    }
}

fn main() {
    let server = Server { log: Mutex::new(Vec::new()) };
    server.handle("/index.html");
    println!("{}", server.log.lock().unwrap().len());
}
```

</details>

### 案例 3：让 Request 借用 parse 里的局部 String——E0515

```rust,editable
use std::io::{BufRead, BufReader};

struct Request<'a> {
    target: &'a str,
}

fn parse<R: BufRead>(reader: &mut R) -> Request<'_> {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    Request {
        target: line.split(' ').nth(1).unwrap(),
    }
}

fn main() {
    let mut reader = BufReader::new("GET /index.html HTTP/1.1\r\n\r\n".as_bytes());
    println!("{}", parse(&mut reader).target);
}
```

```text
error[E0515]: cannot return value referencing local variable `line`
  --> src/main.rs:10:5
   |
10 | /     Request {
11 | |         target: line.split(' ').nth(1).unwrap(),
   | |                 ---- `line` is borrowed here
12 | |     }
   | |_____^ returns a value referencing data owned by the current function
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`line` 是 `parse` 的局部变量，函数返回时它被 drop，堆缓冲归还分配器。返回的 `Request` 里那个 `&str` 指向的正是这块内存。编译器不判断调用者之后还会不会用，只要「返回值的生命周期不长于 `line`」这条约束无法证明，就拒绝。

这正是 41.3 里那个 `struct Request<'a>` 版本的结局。要让借用版成立，缓冲区必须由调用者提供并活得比 `Request` 长；而 41.5 的 `handle_connection` 先在栈上建 `BufReader`，解析完还要跨过 `drop(reader)` 去写响应，两条命都保不住。所以最终选择了「Request 拥有 String」。

**修复**：在离开函数前把切片变成独立的值。多一次分配，换掉一整类生命周期问题；后面写响应的代码也不必再被 `'a` 传染。

```rust
use std::io::{BufRead, BufReader};

struct Request {
    target: String,
}

fn parse<R: BufRead>(reader: &mut R) -> Request {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let target = line.split(' ').nth(1).unwrap_or("/").to_string();
    Request { target }
}

fn main() {
    let mut reader = BufReader::new("GET /index.html HTTP/1.1\r\n\r\n".as_bytes());
    println!("{}", parse(&mut reader).target);
}
```

</details>

### 案例 4：线程池任务捕获了 Rc——E0277

```rust,editable
type Job = Box<dyn FnOnce() + Send + 'static>;

fn execute(job: Job) {
    job();
}

fn main() {
    let root = std::rc::Rc::new(String::from("public"));
    execute(Box::new(move || println!("{}", root)));
}
```

```text
error[E0277]: `Rc<String>` cannot be sent between threads safely
 --> src/main.rs:9:13
  |
9 |     execute(Box::new(move || println!("{}", root)));
  |             ^^^^^^^^^-------^^^^^^^^^^^^^^^^^^^^^^
  |             |        |
  |             |        within this `{closure@src/main.rs:9:22: 9:29}`
  |             `Rc<String>` cannot be sent between threads safely
  |
  = help: within `{closure@src/main.rs:9:22: 9:29}`, the trait `Send` is not implemented for `Rc<String>`
note: required because `root` is used within this closure
 --> src/main.rs:9:45
  |
9 |     execute(Box::new(move || println!("{}", root)));
  |                                             ^^^^
  = note: required for the cast from `Box<{closure@src/main.rs:9:22: 9:29}>` to `Box<(dyn FnOnce() + Send + 'static)>`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Job` 的 trait bound 要求 `Send`，因为闭包会被送到 worker 线程执行。`Rc` 的引用计数是一个普通整数，两个线程同时 clone 会把它数错，计数变小就提前释放、变大就永远泄漏，所以 `Rc<T>` 永远不实现 `Send`。编译器把 `Job` 的约束一路推导到闭包捕获的 `root` 上，报错末尾那句 required for the cast 说的就是这次检查：装箱后要擦成 `dyn FnOnce() + Send`，而捕获了 `Rc` 的闭包做不到。

**修复**：跨线程共享所有权用 `Arc`，它的计数是原子操作。41.5 主程序里 `Arc<PathBuf>` 正是这个原因——每个 worker 都要读文档根目录，但谁都不拥有它。

```rust
type Job = Box<dyn FnOnce() + Send + 'static>;

fn execute(job: Job) {
    job();
}

fn main() {
    let root = std::sync::Arc::new(String::from("public"));
    execute(Box::new(move || println!("{}", root)));
}
```

顺带记住同一条规则的另外两个成员：`RefCell<T>` 也不是 `Sync`，`*mut T` 两者都不是。想让一个类型能进线程池，它的每个字段都得先过 `Send` 这一关。

</details>

### 案例 5：直接在 TcpStream 上按行读——E0599

```rust,editable
use std::io::BufRead;
use std::net::TcpStream;

fn handle(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut line = String::new();
    stream.read_line(&mut line)?;
    println!("{}", line);
    Ok(())
}

fn main() {
    if let Ok(mut stream) = TcpStream::connect("127.0.0.1:7878") {
        let _ = handle(&mut stream);
    }
}
```

```text
error[E0599]: the method `read_line` exists for mutable reference `&mut TcpStream`, but its trait bounds were not satisfied
  --> src/main.rs:6:12
   |
 6 |     stream.read_line(&mut line)?;
   |            ^^^^^^^^^ method cannot be called on `&mut TcpStream` due to unsatisfied trait bounds
   |
   = note: the following trait bounds were not satisfied:
           `TcpStream: BufRead`
           which is required by `&mut TcpStream: BufRead`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`read_line` 是 `BufRead` 的方法，`TcpStream` 只实现了 `Read`。E0599 的措辞值得细读：方法名找得到、接收者类型也对，卡住的是 trait bound。为什么标准库不给 `TcpStream` 实现 `BufRead`：TCP 是字节流，内核读走多少就是多少，昨天的半行不会替你留着；按行读要求的「多读一点、下次接着用」需要一块由调用者管理的缓冲，而这块缓冲一旦存在，就必须始终由同一个对象持有。

**修复**：套一层 `BufReader`。它从 socket 一次读 8 KiB 进自己的缓冲，`read_line` 在这块缓冲里找换行，找不到才回内核再要。代价是缓冲里的字节只能由这个 `BufReader` 消费，后续再绕过它直接读 `stream` 就会丢数据——41.6 讲的正是这一点。

```rust
use std::io::{BufRead, BufReader};
use std::net::TcpStream;

fn handle(stream: &TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    println!("{}", line);
    Ok(())
}

fn main() {
    if let Ok(stream) = TcpStream::connect("127.0.0.1:7878") {
        let _ = handle(&stream);
    }
}
```

</details>

## 完整源码

纯标准库实现就在 `projects/41-http-server/`（<https://github.com/yujh129/rust-book-zh/tree/main/projects/41-http-server>）：单文件 338 行，零依赖，`public/` 是它要服务的静态目录。

```console
$ cd projects/41-http-server
$ cargo run --offline
serving <项目目录>/public at http://127.0.0.1:7878
# 另开一个终端
$ curl -i http://127.0.0.1:7878/
HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8
Content-Length: 274
$ curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:7878/missing
404
```

本机实测：`/` 返回 274 字节的 `index.html`，`/style.css` 返回 114 字节，`/missing` 返回 404，带一个 9.5 KB 头部的请求返回 400（8 KiB 预算生效）。

| 文件 | 内容 |
| --- | --- |
| `src/main.rs` | `Request`/`parse`/`ParseError`、路径安全、响应拼装、固定线程池、析构时排空队列、`main` |
| `public/index.html` | 274 字节的演示页，`/` 与 `/index.html` 都指向它 |
| `public/style.css` | 114 字节，用来验证 `Content-Type` 分支 |

## 41.8 动手练习

1. **基础：把 HEAD 弄对。** 启动服务器，分别执行 `curl -i http://127.0.0.1:7878/` 和 `curl -I http://127.0.0.1:7878/`。两次的 `Content-Length` 必须相同，HEAD 那次响应体必须为零字节。排查方向提示：`route` 里 `request.method == "HEAD"` 算出的布尔值有没有一路传进 `write_to`；`write_to` 中写体的那几行在什么条件下跳过。改完再用 `curl -sI ... | wc -c` 确认头部长度稳定。

2. **基础：给 MIME 表补几个类型。** 现在 `mime_of` 是一条长的 `match`，每来一个请求就按顺序比较字符串。先补上 `woff2`、`mp4`、`ico`，再把它重构成 `HashMap<&'static str, &'static str>`：在 `main` 里构造一次并用 `Arc` 分发给 worker，或者用 `std::sync::OnceLock` 让它在首次调用时惰性初始化。跑同一组 `curl -i`，确认每个扩展名的 `Content-Type` 与改之前 byte 级一致。

3. **进阶：加上 keep-alive。** 把 `handle_connection` 改成循环：用同一个 `BufReader` 连续调用 `Request::parse`；读请求头 `Connection`，值为 `close` 或版本是 HTTP/1.0 时，响应写 `Connection: close` 后返回，否则写 `Connection: keep-alive` 并继续读下一个请求。两个硬约束：每个响应都必须带正确的 `Content-Length`，否则客户端无法定位响应边界；两次请求之间要靠 `set_read_timeout` 断开空闲连接。用 `curl -v http://127.0.0.1:7878/ http://127.0.0.1:7878/style.css` 观察是否只握手一次。

4. **进阶：线程池优雅退出。** 给 `ThreadPool` 加一个显式方法 `fn shutdown(&mut self)`：先 drop 发送端让 `recv` 返回 `Err`、worker 退出循环，再逐个 `join`；同时用一个主线程置位、worker 轮询的 `Arc<AtomicBool>` 作为运行标志。要求：收到停止信号后，已经在通道里排队的任务全部执行完才返回，没有任务丢失，也没有 worker 永久阻塞。提示：`accept` 阻塞期间收不到标志，得先 `listener.set_nonblocking(true)`，遇到 `WouldBlock` 时检查标志并短暂 `sleep`，否则会变成忙等。

5. **挑战：分块发送大文件。** 当前实现用 `fs::read` 把整个文件读进 `Vec<u8>`。改成分块：用 `File::open` 加 `metadata()?.len()` 得到长度写进 `Content-Length`，再用一个 64 KiB 的栈缓冲循环 `read` 与 `write_all`。准备一个 512 MiB 的测试文件，发送时用任务管理器（Windows）或 `/usr/bin/time -v`（Linux）对比两版的峰值内存：旧版约等于文件大小，新版应当稳定在几百 KB。注意不要因为分块就改用 chunked 编码——长度已知时普通响应就够了。

## 41.9 小结

- HTTP/1.1 请求由请求行、头部、空行三部分组成；Host 在 1.1 中必需，缺失或协议版本无法识别都对 400。
- 本章只支持 GET 与 HEAD，其它方法返回 405 并用 `Allow` 头声明可用方法；响应体的字节数写进 `Content-Length`。
- 解析必须建立在 `BufRead` 上：TCP 不保留消息边界，按行读需要一块能回退的缓冲，`BufReader` 就是它。
- 头部按字节数上限截断，配合每连接的读超时，分别挡住慢速头部攻击与慢速连接攻击，两者不能互相替代。
- `Request` 拥有 `String` 而不是借用缓冲区，用一次分配换掉一整类生命周期错误，这是案例 3 的结论。
- 路径安全是两道防线：逐段拒绝 `..`，`canonicalize` 之后再做按路径分量的前缀比对；失败返回 404 而不是 403，避免泄露文件是否存在。
- 响应体是 `Vec<u8>`，`Content-Length` 是字节数而非字符数，写体用 `write_all`，HEAD 只发头部但保留真实长度。
- 固定大小线程池用 `mpsc` 加 `Arc<Mutex<Receiver>>`：锁只包住 `recv`，`Drop` 里先关发送端再 `join`，顺序反了就挂住。
- 任务类型是 `Box<dyn FnOnce() + Send + 'static>`，跨线程共享用 `Arc` 而不是 `Rc`，这是案例 4 的约束。
- Ctrl-C 不会触发 `Drop`，静态文件服务器的硬退出可以接受；要优雅退出需要停止信号和能退出的监听循环。





