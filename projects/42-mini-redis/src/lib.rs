//! mini-redis：一个只用标准库的 RESP 内存数据库（第 42 章项目四）。
//!
//! 分层与章节一一对应：
//! - parse_command：纯函数解析器，处理半包与粘包（42.4）；
//! - store：HashMap 加惰性过期（42.3）；
//! - reply：RESP 回复编码（42.5）；
//! - execute / handle_connection / serve：命令分发与每连接一线程（42.6、42.7）。

pub mod parse_command;
pub mod reply;
pub mod store;

pub use parse_command::parse_command;
pub use reply::Reply;
pub use store::{Store, StoreError};

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 命令分发（第 42.6 节）：命令名大小写不敏感，键名大小写敏感。
///
/// 参数个数用 match 守卫在模式里顺手校验；不合法的参数个数只回 RESP 错误、
/// 保持连接，只有解析层的 Err 才代表字节流不可信、必须断开。
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
                    // EX 后面不是数字是命令错误：悄悄当成「不过期」会让客户端以为 TTL 生效了。
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
        "DEL" => Reply::Error(String::from("DEL 需要恰好一个 key")),
        // 42.1 的验收表要求 EXISTS，42.6 的分发片段漏了它，这里按表补上。
        "EXISTS" if rest.len() == 1 => Reply::Integer(store.exists(&rest[0]) as i64),
        "EXISTS" => Reply::Error(String::from("EXISTS 需要恰好一个 key")),
        "INCR" if rest.len() == 1 => match store.incr(&rest[0]) {
            Ok(n) => Reply::Integer(n),
            Err(e) => Reply::Error(e.to_string()),
        },
        "INCR" => Reply::Error(String::from("INCR 需要恰好一个 key")),
        "KEYS" => Reply::Bulk(Some(store.keys().join(" "))),
        "EXPIRE" if rest.len() == 2 => match rest[1].parse::<u64>() {
            Ok(secs) => Reply::Integer(store.expire(&rest[0], secs) as i64),
            Err(_) => Reply::Error(String::from("EXPIRE 的秒数非法")),
        },
        "EXPIRE" => Reply::Error(String::from("EXPIRE 需要 key 和秒数")),
        "TTL" if rest.len() == 1 => Reply::Integer(store.ttl(&rest[0])),
        "TTL" => Reply::Error(String::from("TTL 需要恰好一个 key")),
        "FLUSHALL" => {
            // 整体替换比 map.clear() 更直白，以后 Store 加字段也不会漏掉。
            *store = Store::new();
            Reply::Simple(String::from("OK"))
        }
        "QUIT" => Reply::Simple(String::from("OK")),
        _ => Reply::Error(format!("未知命令 {}", args[0])),
    }
}

/// 一条连接的服务循环（第 42.7 节）。
///
/// 外层读、内层解析的嵌套是粘包的正解：一次 read 可能带回多条命令，
/// 必须在内层把缓冲区里的命令全部处理完，直到解析器返回 None 才回去再读。
pub fn handle_connection(mut stream: TcpStream, store: Arc<Mutex<Store>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) => return, // 对端关闭，不是错误；漏掉这个分支会空烧 CPU。
            Ok(n) => n,
            Err(_) => return,
        };
        buf.extend_from_slice(&chunk[..n]);
        loop {
            match parse_command(&buf) {
                Ok(Some((args, used))) => {
                    buf.drain(..used);
                    // 书上写 args[0]：但 *0 这种合法输入会解析出空参数列表，
                    // 于是这里先 panic，执行不到 execute 的空值检查。改用 first。
                    let quit = args
                        .first()
                        .is_some_and(|a| a.eq_ignore_ascii_case("QUIT"));
                    // 锁的作用域被一对花括号钉死在 execute 上，网络写操作在锁外。
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
                    // 协议错误：字节流已经不可信，回一个错误就断开。
                    let _ = stream.write_all(&Reply::Error(e).to_bytes());
                    return;
                }
            }
        }
    }
}

/// 每来一个连接丢给一个线程（第 42.7 节）。
pub fn serve(listener: TcpListener, store: Arc<Mutex<Store>>) -> std::io::Result<()> {
    for conn in listener.incoming() {
        let stream = conn?;
        // Arc::clone 必须在循环里面，每轮给新线程一个新的句柄。
        let store = Arc::clone(&store);
        std::thread::spawn(move || handle_connection(stream, store));
    }
    Ok(())
}

/// 测试用的小工具：读一行回复（只读头部，bulk 正文不读）。见 42.7 末尾。
///
/// 注意它每次都新建一个 BufReader 再丢掉，缓冲区里多读到的字节会一起丢，
/// 所以只适合「一条命令一条回复、不流水线」的场景；端到端测试里用的是
/// 一个长期存活的 BufReader，会把 bulk 正文完整读走。
pub fn read_reply(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim_end().to_string())
}
