//! mini-redis 二进制入口：默认监听 127.0.0.1:6379。
//!
//! 用法：miniredis [端口] [--selftest]
//! --selftest 会自己起服务、自己连自己发几条命令，方便在没有 redis-cli
//! 的机器上验证（第 42.7 节）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use miniredis::{serve, Store};

const DEFAULT_PORT: u16 = 6379;

fn main() -> std::io::Result<()> {
    let mut port = DEFAULT_PORT;
    let mut selftest = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--selftest" => selftest = true,
            "-h" | "--help" => {
                println!("用法：miniredis [端口] [--selftest]");
                return Ok(());
            }
            other => match other.parse::<u16>() {
                Ok(p) => port = p,
                Err(_) => {
                    eprintln!("无法识别的参数：{other}");
                    std::process::exit(2);
                }
            },
        }
    }

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    println!("mini-redis 已启动：127.0.0.1:{port}");

    let store = Arc::new(Mutex::new(Store::new()));
    if selftest {
        let server_store = Arc::clone(&store);
        std::thread::spawn(move || {
            let _ = serve(listener, server_store);
        });
        selftest_run(port)
    } else {
        serve(listener, store)
    }
}

fn selftest_run(port: u16) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    for cmd in [
        "PING",
        "SET greeting hello",
        "GET greeting",
        "INCR hits",
        "KEYS",
    ] {
        stream.write_all(format!("{cmd}\r\n").as_bytes())?;
        let head = read_reply_head(&mut reader)?;
        println!("发送 \"{cmd}\" → 收到 \"{head}\"");
    }
    Ok(())
}

/// 读一条完整回复（bulk 的正文一起读掉，保证下一次读到的还是回复头），
/// 只返回首行。书上的 read_reply 只读一行，把正文留在缓冲区里，
/// 依赖缓冲区被丢弃时顺手吞掉，行为不稳定；selftest 用这个确定性版本。
fn read_reply_head(reader: &mut BufReader<TcpStream>) -> std::io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "服务端关闭了连接",
        ));
    }
    let head = line.trim_end().to_string();
    if let Some(len) = head.strip_prefix('$').and_then(|n| n.parse::<usize>().ok()) {
        let mut body = vec![0u8; len + 2];
        reader.read_exact(&mut body)?;
    }
    Ok(head)
}
