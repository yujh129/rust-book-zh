use std::fs;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

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
