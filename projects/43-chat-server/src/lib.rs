//! 第 43 章 项目五：异步聊天服务器 —— 服务器逻辑（lib.rs）
//!
//! 架构：一条 accept 循环 + 三类 task（hub task、每连接的读 task、每连接的写 task）。
//! 房间状态（HashMap<u64, Client>）只归 hub task 所有，全项目无锁。
//!
//! 本文件把章节 43.2–43.6、43.8 里分节给出的片段组装成完整实现，并补上章节
//! 引用却未给出实现的部分（sanitize_name / timestamp / Hub::len / serve /
//! CMD_CAPACITY / OUT_CAPACITY）。每处补充都标了「章节未给出」。

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

/// 出站队列容量：一个客户端最多能欠服务器多少条消息（43.5）。
pub const OUT_CAPACITY: usize = 32;
/// 命令队列容量。章节 43.6 只写了常量名，未给出数值（章节未给出）。
pub const CMD_CAPACITY: usize = 64;
/// R2：昵称最长 32 个字符。
pub const MAX_NAME_LEN: usize = 32;
/// R1：默认监听地址。
pub const DEFAULT_ADDR: &str = "127.0.0.1:9000";

/// hub 只认五种命令（43.4）。
pub enum Cmd {
    Join { id: u64, name: String, tx: mpsc::Sender<String> },
    Leave { id: u64 },
    Say { id: u64, text: String },
    Who { id: u64 },
    /// 只回给发送者的提示（例如敲了不存在的命令）。
    Notice { id: u64, text: String },
}

pub struct Client {
    pub name: String,
    pub tx: mpsc::Sender<String>,
}

pub struct Hub {
    clients: HashMap<u64, Client>,
    dropped: u64,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        Self { clients: HashMap::new(), dropped: 0 }
    }

    pub fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Join { id, name, tx } => {
                self.clients.insert(id, Client { name: name.clone(), tx });
                let roster = self.roster();
                self.send_to(id, format!("*** 欢迎 {}，当前在线：{}", name, roster));
                self.broadcast_except(id, format!("*** {} 进入了房间", name));
            }
            Cmd::Leave { id } => {
                if let Some(c) = self.clients.remove(&id) {
                    self.broadcast(format!("*** {} 离开了房间", c.name));
                }
            }
            Cmd::Say { id, text } => {
                let line = match self.clients.get(&id) {
                    Some(c) => format!("[{}] {}: {}", timestamp(), c.name, text),
                    None => return,
                };
                self.broadcast(line);
            }
            Cmd::Who { id } => {
                let roster = self.roster();
                self.send_to(id, format!("*** 在线：{}", roster));
            }
            Cmd::Notice { id, text } => {
                self.send_to(id, text);
            }
        }
    }

    fn roster(&self) -> String {
        let mut names: Vec<&str> = self.clients.values().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        names.join(", ")
    }

    fn send_to(&mut self, id: u64, line: String) {
        let delivered = match self.clients.get(&id) {
            Some(c) => c.tx.try_send(line).is_ok(),
            None => true,
        };
        if !delivered {
            self.dropped += 1;
        }
    }

    fn broadcast_except(&mut self, skip: u64, line: String) {
        let mut failed = 0;
        for (&id, c) in &self.clients {
            if id != skip && c.tx.try_send(line.clone()).is_err() {
                failed += 1;
            }
        }
        self.dropped += failed;
    }

    fn broadcast(&mut self, line: String) {
        let mut failed = 0;
        for c in self.clients.values() {
            if c.tx.try_send(line.clone()).is_err() {
                failed += 1;
            }
        }
        self.dropped += failed;
    }

    // 章节 43.8 的测试调用了 hub.len()，但 43.4 的 Hub 实现里没有这个方法（章节未给出）。
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// 43.8 的 slow_client 测试要断言「dropped 恰好是 2」（章节未给出）。
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// R2：昵称去掉空白与斜杠，最长 32 个字符。章节 43.2 调用了它，但未给出实现（章节未给出）。
pub fn sanitize_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '/')
        .take(MAX_NAME_LEN)
        .collect()
}

/// R4：[HH:MM:SS]。章节 43.4 调用了它，但未给出实现（章节未给出）。
/// 依赖清单里没有 chrono，这里取 UTC；换成当地时间需要额外的时区库。
pub fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// hub 循环（43.6）。
async fn hub_loop(mut cmd_rx: mpsc::Receiver<Cmd>, mut shutdown: watch::Receiver<bool>) {
    let mut hub = Hub::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => hub.handle(cmd),
                    None => break,
                }
            }
        }
    }
}

/// 连接任务：读昵称、注册、spawn 写 task、读循环、注销（43.2 + 43.5 + 43.6）。
///
/// 章节 43.2 只给了骨架并说「下一节逐段展开」，展开后的完整形态如下。
async fn handle_conn(
    id: u64,
    stream: TcpStream,
    cmd_tx: mpsc::Sender<Cmd>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    let mut first = String::new();
    if !matches!(reader.read_line(&mut first).await, Ok(n) if n > 0) {
        return;
    }
    let name = sanitize_name(first.trim());
    if name.is_empty() {
        return;
    }

    // 出站队列与写 task（43.5）。
    let (out_tx, mut out_rx) = mpsc::channel::<String>(OUT_CAPACITY);
    if cmd_tx.send(Cmd::Join { id, name, tx: out_tx }).await.is_err() {
        return;
    }

    let mut writers = JoinSet::new();
    writers.spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if writer.write_all(b"\n").await.is_err() {
                break;
            }
            if writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.flush().await;
    });

    // 读循环（43.6）。
    loop {
        let mut line = String::new();
        tokio::select! {
            _ = shutdown.changed() => break,
            read = reader.read_line(&mut line) => {
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text.eq_ignore_ascii_case("/quit") {
            break;
        }
        let cmd = if text.eq_ignore_ascii_case("/who") {
            Cmd::Who { id }
        } else if text.starts_with('/') {
            Cmd::Notice { id, text: format!("未知命令：{}", text) }
        } else {
            Cmd::Say { id, text: text.to_string() }
        };
        if cmd_tx.send(cmd).await.is_err() {
            break;
        }
    }

    // 三条退出路径（/quit、EOF、关闭信号）共用同一个出口。
    let _ = cmd_tx.send(Cmd::Leave { id }).await;
    let _ = writers.join_next().await;
}

/// accept 循环（43.6）。
pub async fn run(listener: TcpListener, mut shutdown: watch::Receiver<bool>) -> std::io::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CAPACITY);
    let hub_shutdown = shutdown.clone();
    let hub = tokio::spawn(hub_loop(cmd_rx, hub_shutdown));

    let mut conns = JoinSet::new();
    let mut next_id: u64 = 0;

    let result = loop {
        tokio::select! {
            _ = shutdown.changed() => break Ok(()),
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let id = next_id;
                        next_id += 1;
                        conns.spawn(handle_conn(id, stream, cmd_tx.clone(), shutdown.clone()));
                    }
                    Err(err) => break Err(err),
                }
            }
            Some(_) = conns.join_next() => {}
        }
    };

    drop(cmd_tx);
    conns.shutdown().await;
    let _ = hub.await;
    result
}

/// 进程入口用的一层包装：章节 43.6 的 main 调用了 chat_server::serve，
/// 但正文从未给出它的实现（章节未给出）。
pub async fn serve(addr: &str, shutdown: watch::Receiver<bool>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    run(listener, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

    fn drain(rx: &mut mpsc::Receiver<String>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(line) = rx.try_recv() {
            out.push(line);
        }
        out
    }

    #[test]
    fn join_announces_to_others_and_who_lists_everyone() {
        let mut hub = Hub::new();
        let (a_tx, mut a_rx) = mpsc::channel(8);
        let (b_tx, mut b_rx) = mpsc::channel(8);
        hub.handle(Cmd::Join { id: 1, name: String::from("ana"), tx: a_tx });
        assert_eq!(drain(&mut a_rx).len(), 1);

        hub.handle(Cmd::Join { id: 2, name: String::from("bob"), tx: b_tx });
        let announcement = drain(&mut a_rx);
        assert_eq!(announcement.len(), 1);
        assert!(announcement[0].contains("bob 进入了房间"));
        assert_eq!(drain(&mut b_rx).len(), 1);

        hub.handle(Cmd::Who { id: 2 });
        let who = drain(&mut b_rx);
        assert!(who[0].contains("ana") && who[0].contains("bob"));
        assert_eq!(hub.len(), 2);
    }

    #[test]
    fn leave_removes_client_and_announces() {
        let mut hub = Hub::new();
        let (a_tx, mut a_rx) = mpsc::channel(8);
        let (b_tx, mut b_rx) = mpsc::channel(8);
        hub.handle(Cmd::Join { id: 1, name: String::from("ana"), tx: a_tx });
        hub.handle(Cmd::Join { id: 2, name: String::from("bob"), tx: b_tx });
        let _ = drain(&mut a_rx);
        let _ = drain(&mut b_rx);

        hub.handle(Cmd::Leave { id: 1 });
        assert_eq!(hub.len(), 1);
        // 离开者自己的队列没有新消息。
        assert!(drain(&mut a_rx).is_empty());
        let left = drain(&mut b_rx);
        assert_eq!(left.len(), 1);
        assert!(left[0].contains("ana 离开了房间"));
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("  ana  "), "ana");
        assert_eq!(sanitize_name("an a"), "ana");
        assert_eq!(sanitize_name("a/b"), "ab");
        assert_eq!(sanitize_name(" / "), "");
        assert_eq!(sanitize_name("").is_empty(), true);
        assert_eq!(sanitize_name(&"x".repeat(40)).chars().count(), MAX_NAME_LEN);
    }

    #[test]
    fn say_is_broadcast_with_name_and_time() {
        let mut hub = Hub::new();
        let (a_tx, mut a_rx) = mpsc::channel(8);
        let (b_tx, mut b_rx) = mpsc::channel(8);
        hub.handle(Cmd::Join { id: 1, name: String::from("ana"), tx: a_tx });
        hub.handle(Cmd::Join { id: 2, name: String::from("bob"), tx: b_tx });
        let _ = drain(&mut a_rx);
        let _ = drain(&mut b_rx);

        hub.handle(Cmd::Say { id: 1, text: String::from("hi") });
        for rx in [&mut a_rx, &mut b_rx] {
            let got = drain(rx);
            assert_eq!(got.len(), 1);
            assert!(got[0].starts_with('['));
            assert!(got[0].ends_with("ana: hi"));
        }
        assert_eq!(hub.dropped(), 0);
    }

    #[test]
    fn slow_client_drops_messages_and_counts_them() {
        let mut hub = Hub::new();
        // 容量 1：欢迎语先占满它。
        let (a_tx, mut a_rx) = mpsc::channel(1);
        hub.handle(Cmd::Join { id: 1, name: String::from("ana"), tx: a_tx });
        hub.handle(Cmd::Say { id: 1, text: String::from("one") });
        hub.handle(Cmd::Say { id: 1, text: String::from("two") });
        assert_eq!(hub.dropped(), 2);
        let got = drain(&mut a_rx);
        assert_eq!(got.len(), 1, "队列里仍然只有欢迎语那一条: {:?}", got);
        assert_eq!(hub.len(), 1);
    }

    async fn connect(
        addr: std::net::SocketAddr,
        name: &str,
    ) -> (BufReader<OwnedReadHalf>, OwnedWriteHalf) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        write_half.write_all(format!("{}\n", name).as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
        (BufReader::new(read_half), write_half)
    }

    async fn next_line(reader: &mut BufReader<OwnedReadHalf>) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        line.trim_end().to_string()
    }

    #[tokio::test]
    async fn two_clients_chat_end_to_end() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop_tx, stop_rx) = watch::channel(false);
        let server = tokio::spawn(run(listener, stop_rx));

        let (mut a_rx, mut a_tx) = connect(addr, "ana").await;
        assert!(next_line(&mut a_rx).await.contains("ana"));

        let (mut b_rx, mut b_tx) = connect(addr, "bob").await;
        assert!(next_line(&mut a_rx).await.contains("bob 进入了房间"));
        assert!(next_line(&mut b_rx).await.contains("bob"));

        a_tx.write_all(b"hello\n").await.unwrap();
        a_tx.flush().await.unwrap();
        assert!(next_line(&mut a_rx).await.ends_with("ana: hello"));
        assert!(next_line(&mut b_rx).await.ends_with("ana: hello"));

        b_tx.write_all(b"/who\n").await.unwrap();
        b_tx.flush().await.unwrap();
        let who = next_line(&mut b_rx).await;
        assert!(who.contains("ana") && who.contains("bob"));

        b_tx.write_all(b"/quit\n").await.unwrap();
        b_tx.flush().await.unwrap();
        assert!(next_line(&mut a_rx).await.contains("bob 离开了房间"));

        stop_tx.send(true).unwrap();
        server.await.unwrap().unwrap();
    }
}
