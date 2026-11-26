//! 端到端测试：真的走 TCP，验证 RESP 线格式与多客户端共享一份数据。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use miniredis::{serve, Store};

/// 在随机端口（127.0.0.1:0）起一个服务端线程，返回实际监听地址。
fn start_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口失败");
    let addr = listener.local_addr().expect("读取监听地址失败");
    let store = Arc::new(Mutex::new(Store::new()));
    std::thread::spawn(move || {
        let _ = serve(listener, store);
    });
    addr
}

/// 逐字节解析服务端回复，断言才能直接对着 RESP 线格式写。
#[derive(Debug, PartialEq, Eq)]
enum Wire {
    Simple(String),
    Bulk(Option<String>),
    Integer(i64),
    Error(String),
}

struct Client {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Client {
    fn connect(addr: SocketAddr) -> Client {
        let stream = TcpStream::connect(addr).expect("连接失败");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("设置读超时失败");
        let reader = BufReader::new(stream.try_clone().expect("复制句柄失败"));
        Client { stream, reader }
    }

    fn send_inline(&mut self, line: &str) {
        self.stream
            .write_all(format!("{line}\r\n").as_bytes())
            .expect("写 inline 命令失败");
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).expect("写原始字节失败");
    }

    /// 读一整条回复：返回首行的语义值，bulk 的正文一并读掉。
    fn read_reply(&mut self) -> Wire {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("读回复失败");
        assert!(!line.is_empty(), "服务端提前关闭了连接");
        let head = line.trim_end().to_string();
        assert!(!head.is_empty(), "回复首行是空的");
        let (tag, rest) = head.split_at(1);
        match tag {
            "+" => Wire::Simple(rest.to_string()),
            "-" => Wire::Error(rest.to_string()),
            ":" => Wire::Integer(rest.parse().expect("整数回复解析失败")),
            "$" => {
                let len: i64 = rest.parse().expect("bulk 长度头解析失败");
                if len < 0 {
                    return Wire::Bulk(None);
                }
                let n = len as usize;
                let mut body = vec![0u8; n + 2];
                self.reader.read_exact(&mut body).expect("读 bulk 正文失败");
                assert_eq!(&body[n..], b"\r\n".as_slice(), "bulk 正文后面必须是 CRLF");
                Wire::Bulk(Some(
                    String::from_utf8(body[..n].to_vec()).expect("bulk 不是 UTF-8"),
                ))
            }
            other => panic!("未知的回复首字节：{other}"),
        }
    }
}

#[test]
fn end_to_end_set_get() {
    let addr = start_server();
    let mut c = Client::connect(addr);
    c.send_inline("PING");
    assert_eq!(c.read_reply(), Wire::Simple(String::from("PONG")));
    c.send_inline("SET foo bar");
    assert_eq!(c.read_reply(), Wire::Simple(String::from("OK")));
    c.send_inline("GET foo");
    assert_eq!(c.read_reply(), Wire::Bulk(Some(String::from("bar"))));
    c.send_inline("GET nope");
    assert_eq!(c.read_reply(), Wire::Bulk(None));
    c.send_inline("DEL foo");
    assert_eq!(c.read_reply(), Wire::Integer(1));
    c.send_inline("EXISTS foo");
    assert_eq!(c.read_reply(), Wire::Integer(0));
}

#[test]
fn resp_array_over_tcp() {
    let addr = start_server();
    let mut c = Client::connect(addr);
    // 两条命令一次 write：既走 RESP 数组形式，又顺手验证粘包。
    c.send_raw(b"*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\nbar\r\n*2\r\n$3\r\nGET\r\n$3\r\nfoo\r\n");
    assert_eq!(c.read_reply(), Wire::Simple(String::from("OK")));
    assert_eq!(c.read_reply(), Wire::Bulk(Some(String::from("bar"))));
}

#[test]
fn unknown_command_returns_error() {
    let addr = start_server();
    let mut c = Client::connect(addr);
    c.send_inline("FOO bar");
    match c.read_reply() {
        Wire::Error(msg) => assert!(msg.contains("FOO"), "错误文案应回显命令名，实际 {msg}"),
        other => panic!("期望 RESP 错误，实际 {other:?}"),
    }
    // 业务错误不能断开连接：紧接着还能继续发命令。
    c.send_inline("PING");
    assert_eq!(c.read_reply(), Wire::Simple(String::from("PONG")));
}

/// 42.1 的验收标准 3：两个客户端同时连上，一个写一个读，读到同一份数据。
/// 书上列的 3 个端到端测试没有覆盖这一条，这里补一个。
#[test]
fn two_clients_share_state() {
    let addr = start_server();
    let mut writer = Client::connect(addr);
    let mut reader = Client::connect(addr);
    writer.send_inline("SET shared v1");
    assert_eq!(writer.read_reply(), Wire::Simple(String::from("OK")));
    reader.send_inline("GET shared");
    assert_eq!(reader.read_reply(), Wire::Bulk(Some(String::from("v1"))));
    writer.send_inline("INCR counter");
    assert_eq!(writer.read_reply(), Wire::Integer(1));
    reader.send_inline("INCR counter");
    assert_eq!(reader.read_reply(), Wire::Integer(2));
}
