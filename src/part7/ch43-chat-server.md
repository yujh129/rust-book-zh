# 第 43 章 项目五：异步聊天服务器

> **本章目标**
>
> - 用 tokio 写出一个多客户端 TCP 聊天服务器：昵称、广播、/who、/quit 与带时间戳的消息。
> - 看清「每连接两条 task」与「中心 hub + mpsc」两种连接模型，说得出各自的代价。
> - 理解 broadcast 的滞后与容量问题，知道什么前提下它才是更简单的选择。
> - 用 select! 与 watch 实现优雅退出，并识别连接泄漏的几种成因。
> - 用 telnet/nc 手测、用单元测试与 #[tokio::test] 端到端测试验证行为。

## 43.1 需求与验收标准

先划边界。本章要交付的是一个能跑、能测、能把设计讲清楚的小服务器，正文代码 450 行上下（lib.rs 约 430 行，main.rs 30 行），分成两个文件：lib.rs 装服务器逻辑，main.rs 只做进程入口。这样测试可以直接调 lib 里的函数，不必启动一个真进程。

### 功能需求

1. R1 一个进程同时服务多个 TCP 客户端，默认监听 127.0.0.1:9000。
2. R2 客户端连上后第一行是昵称；昵称去掉空白与斜杠，最长 32 个字符。
3. R3 一条普通消息广播给房间里所有人，包括发送者自己：发送者看到自己的消息，等于收到了「服务器已接收」的回执。
4. R4 服务器给每条广播加上发送者昵称与服务器时间，格式固定为方括号时间、昵称、冒号、内容。
5. R5 /who 返回按昵称排序的在线名单；/quit 断开连接，并把离开事件广播给剩下的人。
6. R6 某个客户端读得慢，不能拖慢其他人。
7. R7 收到 Ctrl-C 后停止 accept、关闭所有连接、进程退出，不留监听端口。

### 明确不做

不做历史消息持久化、不做身份认证、不做私聊、不做 TLS。这四件事每一件都会让代码量翻倍，而它们和本章想讲的异步结构关系不大。把它们留给练习，正文保持能一口气读完的规模。

### 线协议

协议只有三句话：

- 一行一条消息，UTF-8 编码，以换行符结尾；
- 连接后的第一行是昵称，之后每一行才是一条消息或命令；
- 以斜杠开头的行是命令，目前只有 /who 和 /quit。

一次完整会话就是下面四行：

```text
ana
大家好，我是 ana
/who
/quit
```

选择行协议而不是第 42 章 RESP 那样的长度前缀，是因为聊天消息天然是文本、天然按行输入，而 telnet/nc 这类手测工具没法替你算长度前缀。代价是消息里不能出现换行；我们直接把「一行就是一条消息」定为约束，不为它设计转义。

### 验收标准

把下面五条当作完成定义，全部能手工复现才算做完：

1. 两个终端分别连上，A 发消息，B 立刻看到带时间和昵称的那一行，A 自己也看到同一条。
2. B 输入 /who，看到按字母序排列的在线名单。
3. A 输入 /quit，B 看到 A 离开的提示，A 的终端回到 shell。
4. 第三个客户端连上时，前两个都看到进入房间的提示，新客户端收到一句带当前名单的欢迎语。
5. 按 Ctrl-C，服务器打印关闭日志后退出；再查端口，9000 已经不在监听。

## 43.2 架构：一条 accept 循环，三类 task

整个服务器只有三类 task：

```text
                    accept 循环
                        │  每来一个连接，spawn 一个 handle_conn
                        │  mpsc::Sender<Cmd>
                        ▼
                     hub task
                        │  HashMap<u64, Client>，房间状态的唯一主人
                        │  每个客户端一个 mpsc::Sender<String>
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
   写 task(ana)     写 task(bob)    写 task(carol)
        ▲
        │ 读 task 从 socket 读行，转成 Cmd 发给 hub
```

三条规则值得先记住：

- 状态只有一个主人。HashMap 归 hub task 所有，别的 task 想读想写都只能发消息。于是整个项目不需要 Mutex，也不会出现「持锁跨 await」的问题。
- 谁拥有 socket 的哪一半，谁负责读或写。into_split 把 TcpStream 拆成读写两半，读半给读 task，写半给写 task。
- 连接任务不直接给别人发数据。它只把 Cmd 交给 hub，由 hub 决定发给谁、发什么。于是「谁在线」永远只有一个答案。

最容易想到的版本是「每连接一个 task，收到消息就遍历所有连接直接写」。客户端少、都很快时它能跑，但有两个毛病：写操作会阻塞读循环，一个慢客户端就能卡住整次广播；多个 task 同时改共享表又需要加锁。把写拆出去、把状态收进 hub，两个问题一起消失。

先看连接任务的骨架，它做四件事：读昵称、注册、循环读行、退出时注销。

```rust,noplayground
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

    // 注册、spawn 写 task、读循环、注销，下一节逐段展开
}
```

这里已经能看出一个设计选择：handle_conn 拿到的是 cmd_tx 的克隆，它自己从不碰 Hub。昵称不合法的连接在注册之前就返回了，hub 根本不知道它存在过——这比「先注册、再校验、再回滚」省一次状态变更。

## 43.3 用 broadcast 还是中心 hub

tokio 自带 broadcast 频道，看上去就是为聊天室准备的：一个发送者，多个接收者。每个连接订阅一份，收到就写进 socket。核心循环大概是这样：

```rust,noplayground
let (bcast_tx, _) = broadcast::channel::<Arc<str>>(64);

let mut rx = bcast_tx.subscribe();
tokio::spawn(async move {
    loop {
        match rx.recv().await {
            Ok(line) => {
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("这个客户端落后了 {} 条", n);
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
});
```

broadcast 的语义是环形缓冲区加每个订阅者一个游标：容量在创建时固定，写满后新消息覆盖最旧的，每个接收者自己记着读到哪。落后太多的接收者不会阻塞发送方，而是收到 Lagged(n)，游标被强行推到最旧的一条。这个设计对日志分发非常合适，放到聊天室却有下面三个具体麻烦。

1. 滞后不可控。慢客户端不是被丢几条，而是成批地跳：容量 64，客户端卡住时错过 200 条，它会一次性丢掉 200 条，且丢掉的内容无法重放。容量设小，偶尔卡一下就跳；设大，内存按客户端数乘容量乘单条消息大小增长。
2. 广播没有收件人概念。欢迎语只发给新人，/who 只回给提问者，离开通知要跳过离开者本人。用 broadcast 就得给每条消息加一个过滤字段，让每个接收者自己判断，等于把路由逻辑复制进每一份连接代码。
3. 昵称和在线表没地方放。谁在线、谁叫什么仍然需要一张共享表，于是又回到锁，以及持锁跨 await 的老问题。

hub 模型把这三件事一起收进一个 task：发送是定向的，背压是每客户端独立的，状态是独占的。

代价也要讲清楚：hub 是单点，所有消息都要排队经过它。聊天室消息小、频率低、人数少，单点吞吐远超需求；真要扩到几万人，按房间或按 id 分片成多个 hub 即可。反过来，broadcast 也并非错误答案，它在「所有订阅者都该收到全部消息、而且都跟得上」时更简单，配置热更新和日志扇出都是这种形状。聊天室偏偏不满足这两个前提。

两种设计对照如下，表格里只写名字，不写代码标记。

| 维度 | broadcast 方案 | 中心 hub 加 mpsc |
| --- | --- | --- |
| 路由 | 全员广播，接收者自行过滤 | hub 精确决定发给谁 |
| 慢客户端 | 整条频道滞后，Lagged 后成批丢 | 只有他自己的队列满，逐条丢 |
| 房间状态 | 仍需共享表加锁 | 归 hub 独占，无锁 |
| 扩展方式 | 多生产者容易 | 单点，需要分片 |
| 代码量 | 少，但过滤逻辑散落各处 | 稍多，逻辑集中、可单测 |

本章采用 hub。下面从它认识的数据结构开始。

## 43.4 实现 hub：Cmd、Client 与慢客户端策略

先定三个常量，后面所有循环的容量都来自它们：

```rust,noplayground
/// 一个客户端最多能欠服务器多少条消息。
pub const OUT_CAPACITY: usize = 32;
/// hub 的命令队列容量。
pub const CMD_CAPACITY: usize = 64;
/// 昵称最长 32 个字符。
pub const MAX_NAME_LEN: usize = 32;
```

hub 只认五种命令：

```rust,noplayground
pub enum Cmd {
    Join { id: u64, name: String, tx: mpsc::Sender<String> },
    Leave { id: u64 },
    Say { id: u64, text: String },
    Who { id: u64 },
    Notice { id: u64, text: String },
}

pub struct Client {
    pub name: String,
    pub tx: mpsc::Sender<String>,
}
```

三个设计点：

- id 由 accept 循环分配，单调递增，永不复用。复用一个刚断开的 id，会让「离开」和「加入」两条通知在某些交错顺序下变得难以推理。
- tx 存在 Client 里，而不是直接存 TcpStream。hub 只负责把一行塞进某个队列，不关心对端是 TCP 还是别的东西；单元测试里塞一个 mpsc 就能完整测试 hub，不需要真 socket。
- Join 同时带昵称和 tx，注册与拿到出站句柄是同一个原子步骤，不会出现「表里有这个人、却没有他的队列」的中间态。

hub 的完整实现只有一个 match、几个辅助函数：

```rust,noplayground
pub struct Hub {
    clients: HashMap<u64, Client>,
    dropped: u64,
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

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}
```

另外两个辅助函数：昵称清洗在连接建立时调用，时间戳在拼广播行时调用。

```rust,noplayground
use std::time::{SystemTime, UNIX_EPOCH};

/// R2：去掉空白与斜杠，最长 32 个字符。
pub fn sanitize_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '/')
        .take(MAX_NAME_LEN)
        .collect()
}

/// R4：取 UTC 的 [HH:MM:SS]。依赖清单里没有 chrono，想要当地时间得另加时区库。
pub fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}
```

### 慢客户端策略：为什么全是 try_send

发消息的三处地方一律用 try_send，不 await，这是本章最重要的一个取舍：

- hub 一旦 await 某个满队列，后面所有人的消息都会排队，一个慢客户端就能拖住整个房间。我们前面费劲把写拆成 task，就是为了不在 hub 里等待。
- 队列满只说明接收端暂时跟不上。聊天室的取舍是丢最新的：过时的消息比新消息更没用，而卡住所有人代价更大。
- dropped 计数器至少让运维知道有人在丢消息。但它区分不了「暂时慢」和「已经死了」。真实项目通常再加一条规则：连续丢 N 条就判定连接已死，给 hub 发一条 Leave 把它踢出去。本章保留计数，踢人留作练习。

还有一处必须说实话：try_send 的 Err 其实有两种，Full 表示队列满，Closed 表示写 task 已经退出、对端关闭。上面的代码把它们合并计数了。严格说 Closed 应该立刻把客户端从表里删掉，否则死连接会一直留在在线名单里，每次广播都白试一次——这就是 43.9 要讲的连接泄漏。先把主线跑通，练习里再修。

## 43.5 行协议：lines() 与 BufWriter

读行这条路上有两个必须知道的点。

第一，TcpStream 只实现了 AsyncRead 和 AsyncWrite，没有实现 AsyncBufRead，所以它本身没有 read_line 可用。必须先用 BufReader 包一层：BufReader 持有缓冲区，半行数据有地方暂存，read_line 才能在一次 read 拿回多行时只消费一行。忘记包 BufReader 的报错通常不是「方法不存在」，而是类型不满足 AsyncBufRead 的约束。

第二，lines() 是 AsyncBufReadExt 提供的便捷方法，返回一个 Lines，next_line 给出 `Option<String>`：读完了是 None，IO 错误是 Err。它内部同样是 read_line，只是替你管理了 String。注意这个 trait 必须在作用域里，否则编译器只会告诉你「没有名为 lines 的方法」，而不是「忘了导入」——这正是案例 3。

写出侧对应的是 BufWriter。一条消息至少两个 write：正文和换行。裸 TcpStream 上每次 write_all 都是一次系统调用；BufWriter 把小块攒进缓冲区，flush 决定什么时候真正推给内核。忘记 flush 的典型表现是本地测试碰巧能过、换台机器就「消息卡住」——数据还停在用户态缓冲里。

写 task 只有十几行：

```rust,noplayground
let (out_tx, mut out_rx) = mpsc::channel::<String>(OUT_CAPACITY);
let write_task = tokio::spawn(async move {
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
```

三个分之里的 break 是三条错误出口，收成同一个行为：对端不在了就停止。task 的返回值是单元类型，用 break 比用问号加 Result 返回更直接。循环外再 flush 一次，是最后尝试把缓冲区里剩下的半截推出去，失败就忽略。

OUT_CAPACITY 取 32，含义是「一个客户端最多能欠服务器 32 条消息」。它也是整个服务器的内存上界来源：每连接的内存约等于 32 条消息加两个缓冲区，跟在线人数成正比，与消息速率无关。

## 43.6 优雅退出：select! 与 watch

服务器要能被 Ctrl-C 停下来，而且停得干净：不再接新连接、通知所有连接退出、把 hub 收掉、端口释放。

关闭信号用 watch 频道广播。watch 的特点是只保留最新值：晚订阅的人也能立刻看到当前值，正适合这种开关量。与 broadcast 不同，它不存历史，没有滞后问题。

三个循环都要同时等两件事，于是都写成 select!：accept 循环等新连接或关闭信号，连接的读循环等下一行或关闭信号，hub 循环等命令或关闭信号。

先看 accept 循环：

```rust,noplayground
pub async fn run(listener: TcpListener, mut shutdown: watch::Receiver<bool>) -> std::io::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CAPACITY);
    let hub_shutdown = shutdown.clone();
    let hub = tokio::spawn(hub_loop(cmd_rx, hub_shutdown));

    let mut conns = tokio::task::JoinSet::new();
    let mut next_id: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let id = next_id;
                        next_id += 1;
                        conns.spawn(handle_conn(id, stream, cmd_tx.clone(), shutdown.clone()));
                    }
                    Err(_) => break,
                }
            }
            Some(_) = conns.join_next() => {}
        }
    }

    drop(cmd_tx);
    let _ = hub.await;
    conns.shutdown().await;
    Ok(())
}
```

四处细节：

- 第三个分支是任务回收。JoinSet 会把已完成的任务留着等你取，不取就一直占内存；join_next 在集合为空时返回 None，模式不匹配，这个分支自动禁用，不会空转。
- drop(cmd_tx) 扔掉的是 accept 循环手里这一份。hub 能退出并不依赖频道关闭——它自己的 select 里也等同一个关闭信号。
- 退出链条的顺序不能反：hub 先退出，退出时把表里的 Client 全部丢掉，每个出站队列的发送端随之消失，写 task 的 recv 返回 None 自行结束；连接任务里那句 write_task.await 才不会永远挂着。如果先等连接任务、再关 hub，一个正在等出站队列的写 task 就永远等不到 None。
- conns.shutdown().await 兜底：任何还没退出的连接任务在这一步被取消。正常情况下它们已经被信号叫停了，这一步只是保证不漏。

hub 循环本身很短：

```rust,noplayground
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
```

连接的读循环同样套一层 select!：

```rust,noplayground
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
```

这里藏着一个 tokio 里最常见的正确性陷阱：read_line 不是可取消安全的。select! 里另一个分支先完成时，read_line 的 future 会被丢掉；已经读进 BufReader 缓冲区的那部分数据还在，但已经追加进 line 的那部分会跟着 line 一起丢失。上面的写法之所以安全，是因为 line 每轮新建，而且取消只在「要退出了」时发生。如果同一个 select 里还有别的会先完成的分支，就必须换成手写状态机，或者把半行数据留在自己的缓冲区里。

把 43.2 的骨架、43.5 的写 task 和这段读循环按顺序接起来，就是完整的 handle_conn。还差最后三步：跳出循环后给 hub 发 Leave，再等写 task 收尾（hub 退出时会把 Client 连同出站队列的发送端一起丢掉，写 task 的 recv 这才返回 None）。

```rust,noplayground
// 三条退出路径（/quit、EOF、关闭信号）共用同一个出口。
let _ = cmd_tx.send(Cmd::Leave { id }).await;
let _ = write_task.await;
```

注意上面「未知命令」走的是 Cmd::Notice 而不是 Cmd::Say：这类提示只该回给发送者，广播给整个房间会让所有人都看到别人敲错的命令。hub 的 Notice 分支只调 send_to，不碰 broadcast。

最后是进程入口。为什么把 serve 包进 spawn 再 select：既要等 Ctrl-C，又要等服务器自己结束（比如端口被占用导致 bind 失败），直接 await serve 就没办法同时等信号。

```rust,noplayground
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = env::args().nth(1).unwrap_or_else(|| String::from("127.0.0.1:9000"));
    let (stop_tx, stop_rx) = watch::channel(false);
    println!("聊天服务器监听 {}", addr);

    let mut server = tokio::spawn(async move { chat_server::serve(&addr, stop_rx).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("收到 Ctrl-C，正在关闭");
            let _ = stop_tx.send(true);
        }
        res = &mut server => {
            res.unwrap()?;
            return Ok(());
        }
    }

    server.await.unwrap()?;
    println!("已退出");
    Ok(())
}
```

注意 res = &mut server 那一路借的是 JoinHandle 而不是消费它，所以信号分支结束后还能再 await 一次，把服务器真正跑完再打印「已退出」。

serve 只是「bind 再交给 run」这一层薄包装。单独拆出来是为了让测试能绕过固定端口：43.8 的端到端测试直接调 run，端口交给内核挑。

```rust,noplayground
pub async fn serve(addr: &str, shutdown: watch::Receiver<bool>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    run(listener, shutdown).await
}
```

## 43.7 手测：两个客户端对话

先启动服务器：

```console
$ cargo run --offline --bin chat-server
聊天服务器监听 127.0.0.1:9000
```

再开两个终端分别连上去。下面是本机的真实输出：ana 先连，0.7 秒后 bob 连上，两人各说一句、各自查一次在线名单，最后 ana 先退出。

ana 的终端：

```console
$ nc 127.0.0.1 9000
ana
*** 欢迎 ana，当前在线：ana
*** bob 进入了房间
大家好，我是 ana
[13:38:06] ana: 大家好，我是 ana
[13:38:06] bob: bob 报到
/who
*** 在线：ana, bob
/quit
```

bob 的终端：

```console
$ nc 127.0.0.1 9000
bob
*** 欢迎 bob，当前在线：ana, bob
[13:38:06] ana: 大家好，我是 ana
bob 报到
[13:38:06] bob: bob 报到
/who
*** 在线：ana, bob
*** ana 离开了房间
/quit
```

三处值得对照着看：

- bob 的欢迎语里已经有 ana，因为 ana 先连上；ana 那边看到的是「bob 进入了房间」，广播发生在注册之后，所以新人的名字已经在表里。
- 每条消息在两个终端都出现，包括发送者自己的终端。这是 R3，也是最便宜的回执：看到自己的话落在一行里，说明服务器收到了。
- ana 的终端里没有「ana 离开了房间」，离开通知只发给剩下的人；bob 的终端里有，而且排在他的 /quit 之前。

最后在服务器终端按 Ctrl-C：

```console
$ cargo run --offline --bin chat-server
聊天服务器监听 127.0.0.1:9000
^C
收到 Ctrl-C，正在关闭
已退出
```

顺序很重要：先打印关闭日志，等所有连接退出，再打印「已退出」。如果第二行之后卡住，几乎一定是 43.6 里那条退出链条的顺序被改坏了。

## 43.8 测试：hub 单测与端到端

hub 不碰 socket，所以可以直接单测：塞一条 mpsc 当客户端，把 Cmd 喂进去，再断言队列里收到了什么。

```rust,noplayground
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
```

五个 hub 测试各管一件事：

1. say_is_broadcast_with_name_and_time：断言消息以左方括号开头、以「ana: hi」结尾，且 dropped 为 0，把 R4 的格式钉死。
2. join_announces_to_others_and_who_lists_everyone：就是上面这个，同时覆盖欢迎语只发给新人、进入通知发给老人、/who 返回排序名单。
3. leave_removes_client_and_announces：离开后表里少一个，剩下的人收到通知，离开者自己的队列没有新消息。
4. names_are_sanitized：昵称「  an/a  」清洗成「ana」，超过 32 个字符被截断，全是空白或斜杠的昵称得到空串、连接被直接拒绝。
5. slow_client_drops_messages_and_counts_them：把队列容量设成 1，欢迎语先占满它，随后两条广播都发不出去，dropped 恰好是 2，队列里仍然只有一条。

端到端测试则真的起一个服务器，用 local_addr 拿内核分配的端口：

```rust,noplayground
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
```

三个细节：绑定 0 端口让内核挑一个空闲端口，并行跑测试不会互相打架；每个断言都读「下一行」，等于隐式检查了消息的顺序和数量，多一条少一条都会错位；最后必须发关闭信号并 await 服务器，否则测试函数结束时 task 被 runtime 直接丢弃，退出路径等于没测。

两个 helper 只是把建连和读行包了一层：

```rust,noplayground
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

async fn connect(addr: std::net::SocketAddr, name: &str) -> (BufReader<OwnedReadHalf>, OwnedWriteHalf) {
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
```

跑起来是六个测试全过：

```console
$ cargo test --offline
running 6 tests
test tests::join_announces_to_others_and_who_lists_everyone ... ok
test tests::leave_removes_client_and_announces ... ok
test tests::names_are_sanitized ... ok
test tests::say_is_broadcast_with_name_and_time ... ok
test tests::slow_client_drops_messages_and_counts_them ... ok
test tests::two_clients_chat_end_to_end ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 43.9 深入：四个容易被跳过的问题

### write 为什么要单独一个 task

因为背压不会消失，只会转移。要么转移到发送方，要么转移到缓冲区，异步系统设计的一大半工作就是选择把背压放在哪里。

如果 hub 直接写 socket，或者对满队列 await，那么一个慢客户端的网络延迟就会变成整个房间的延迟：所有人都得等他那一行写完，hub 才处理下一条命令。拆出写 task 之后，hub 对每次广播的代价是「在线人数次 try_send」，全部是同步的、O(1) 的，绝不等待。真正的 await 被挪进每个客户端自己的写 task，慢只慢他自己。

代价是队列长度成了新的旋钮。队列越长，能吸收的抖动越多，占用的内存也越多；队列越短，越早开始丢消息。32 这个数字没有魔法，它只是把「一个客户端最多能欠多少条」明确成了一个常量。

### broadcast 的 RingBuffer 语义

broadcast 的底层是一个固定容量的环形缓冲区，加每个接收者一个位置游标。send 永远不阻塞：写满就覆盖最旧的一条，然后把所有跟不上的接收者标记为「落后」。这些接收者下一次 recv 会拿到 Lagged(n)，n 是被覆盖掉的条数，之后从当前最旧的一条继续读。如果没有任何接收者，send 直接返回 Err。

这解释了三个日常现象：第一，broadcast 不会因为某个接收者慢而变慢，它只会丢；第二，Lagged 之后的补发是不可能的，数据已经被覆盖；第三，容量必须按「最坏情况下接收者会落后多少条」来估，而这个数在没有背压时没有上界。

下面这段纯 std 的小程序把同样的语义演了出来：容量 4 的环形缓冲区写进 6 个值，游标还停在 0，于是先报落后、再从最旧那条开始补读。

```rust
fn main() {
    let mut ring: Vec<u8> = vec![0; 4];
    let mut pos = 0usize;
    let mut total = 0usize;
    let mut cursor = 0usize;

    for value in [1u8, 2, 3, 4, 5, 6] {
        ring[pos] = value;
        pos = (pos + 1) % ring.len();
        total += 1;
    }

    if total - cursor > ring.len() {
        let lagged = total - cursor - ring.len();
        println!("落后 {} 条，从最旧一条继续", lagged);
        cursor = total - ring.len();
    }
    while cursor < total {
        println!("收到 {}", ring[cursor % ring.len()]);
        cursor += 1;
    }
}
```

```text
落后 2 条，从最旧一条继续
收到 3
收到 4
收到 5
收到 6
```

mpsc 恰好相反：点对点、有背压、无损，满了就让你等。聊天室要的是「每个客户端独立的、有界的、可丢弃的」通道，所以是「每人一条 mpsc」，而不是「所有人一条 broadcast」。

顺带一提，Lagged 这个分支不写会编译不过：match 不穷尽，编译器会直接指出来。这是 Rust 在 API 设计上帮忙的地方——有损语义被写进了类型，你没法假装它不存在。

### spawn 与 'static

tokio::spawn 的签名要求 F: Future + Send + 'static。两个约束各管一件事。

'static 意味着 future 不能借用栈上的局部变量。所以 handle_conn 里必须 into_split 把 stream 的所有权拆开，把 write_half move 进写 task 的闭包，cmd_tx 也必须 clone 一份。想共享只读数据就用 Arc，别想靠借用绕过；实在需要借用作用域，得用 async-scoped 这类库，tokio 本身不提供。

Send 意味着 future 内部跨 await 持有的每个值都必须是 Send，因为多线程 runtime 会把 task 在线程之间搬运。std::sync::MutexGuard 不是 Send，所以「拿到 guard，然后 await」的组合一定编译失败，这就是案例 4。修法有两条：把锁的作用域缩到 await 之前（复制出需要的数据，然后 drop guard），或者换成 tokio::sync::Mutex 并 await 它的 lock。前者通常更好：临界区里本来就不该有 IO。

### 连接泄漏与 Drop

一个连接的完整生命周期是「注册、读、写、注销」，泄漏发生在任何一步没接上：

1. 忘记发 Leave。表里留着已经断开的连接，在线名单是错的，每次广播都往一个没人读的队列里试一次，内存也不释放。本项目的 handle_conn 在跳出读循环之后一定会发 Leave，不管是 /quit、EOF 还是收到关闭信号，三条路径共用同一个出口。
2. 发了 Leave 但 hub 没 remove。Client 还在表里，它的 tx 就一直活着，写 task 永远阻塞在 recv 上。所以 Leave 的处理必须是 remove，而不是只发一条通知。
3. 只 spawn 不回收。task 结束后 JoinHandle 和它的输出还留在 JoinSet 里，连接多了就是稳定的内存增长。accept 循环里那个 join_next 分支干的就是这件事。
4. panic 被吞掉。spawn 出去的 task panic 不会让父任务失败，只有 await JoinHandle 才能看到 Err。conns.shutdown().await 的返回值被我们忽略了，生产代码至少应该记一条日志。

Drop 在这里扮演了收尾的角色：Client 被 remove 时 tx 被 drop，写 task 的 out_rx.recv 立刻返回 None，写 task 退出，write_half 被 drop，内核发出 FIN。一个 remove 动作同时完成了三件事。反过来说，任何地方多 clone 一份 tx 存着，写 task 就会多活一轮，泄漏就是这么来的。

至于「在 Drop 里自动发 Leave」这种写法，看着优雅，实际不行：Drop 不能 await，只能 try_send，队列满的时候照样丢。显式地在退出路径上 send 一次，比藏一个不可靠的 Drop guard 好。

## 43.10 典型错误案例

下面五个都是本章代码在真实编写过程中撞到的，报错原文来自本机 rustc 1.100.0，除文件名统一成 src/main.rs 之外未做删改。

### 案例 1：把 broadcast 的 recv 当成 Option——E0308

想用 broadcast 做广播，接收端照着 mpsc 的习惯写：

```rust,noplayground
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel::<String>(16);
    let mut rx = tx.subscribe();

    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            println!("{}", line);
        }
    });
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:9:19
  |
9 |         while let Some(line) = rx.recv().await {
  |                   ^^^^^^^^^^   --------------- this expression has type `Result<String, tokio::sync::broadcast::error::RecvError>`
  |                   |
  |                   expected `Result<String, RecvError>`, found `Option<_>`
  |
  = note: expected enum `Result<String, tokio::sync::broadcast::error::RecvError>`
             found enum `Option<_>`

For more information about this error, try `rustc --explain E0308`.
error: could not compile `chat-server` (bin "e1") due to 1 previous error
```

**原因**：broadcast 的 recv 有三种结局：收到一条、落后了、频道关闭。后两种必须能被区分，所以它返回 Result，Err 里恰好是 Lagged 和 Closed 两种。mpsc 的 recv 只可能「有」或「没了」，所以用 Option。把两套 API 混着记，就会写出 while let Some。

**修复**：三种结局分别处理，Lagged 不能只是忽略：

```rust,noplayground
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let (tx, _rx) = broadcast::channel::<String>(16);
    let mut rx = tx.subscribe();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(line) => println!("{}", line),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("落后 {} 条，已跳过", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
```

这也是本章最终没用 broadcast 的一个附带理由：它把「有损」写进了返回值，你每写一个接收端都得处理一次。hub 模型里这个决定被集中到一处，也就是那个 dropped 计数。

### 案例 2：广播时把 String 移动进了循环——E0382

```rust,noplayground
use tokio::sync::mpsc;

struct Client {
    tx: mpsc::Sender<String>,
}

fn broadcast(clients: &[Client], line: String) {
    for c in clients {
        c.tx.try_send(line).unwrap();
    }
}

#[tokio::main]
async fn main() {
    let (tx1, _r1) = mpsc::channel(4);
    let (tx2, _r2) = mpsc::channel(4);
    let clients = vec![Client { tx: tx1 }, Client { tx: tx2 }];
    broadcast(&clients, String::from("hello"));
}
```

```text
error[E0382]: use of moved value: `line`
 --> src/main.rs:9:23
  |
7 | fn broadcast(clients: &[Client], line: String) {
  |                                  ---- move occurs because `line` has type `String`, which does not implement the `Copy` trait
8 |     for c in clients {
  |     ---------------- inside of this loop
9 |         c.tx.try_send(line).unwrap();
  |                       ^^^^ value moved here, in previous iteration of loop
  |
help: consider cloning the value if the performance cost is acceptable
  |
9 |         c.tx.try_send(line.clone()).unwrap();
  |                           ++++++++

For more information about this error, try `rustc --explain E0382`.
error: could not compile `chat-server` (bin "e2") due to 1 previous error
```

**原因**：String 不实现 Copy，try_send 按值取走所有权。第一轮循环把 line 移进队列之后，第二轮就没有 line 可移了。编译器给的 help 直接把补丁写好：line.clone()。但要注意 clone 的代价：每个接收者一份完整拷贝，人数乘长度，房间一大就是明显的分配开销。

**修复**：两条路。消息短、人数少，直接 clone 最简单；消息会被很多人收到，就用 Arc 让所有人共享同一份：

```rust,noplayground
use std::sync::Arc;

use tokio::sync::mpsc;

struct Client {
    tx: mpsc::Sender<Arc<str>>,
}

fn broadcast(clients: &[Client], line: Arc<str>) {
    for c in clients {
        let _ = c.tx.try_send(Arc::clone(&line));
    }
}
```

Arc::clone 只加一次原子计数，消息内容全程只有一份。本章的 Hub 用了 String 加 clone，因为聊天消息只有几十字节、房间只有几十人；真要做万人房间，第一件事就是把出站类型换成 `Arc<str>`，这一改动会波及 Cmd、Client 和写 task 三处，正好是练习 5。

### 案例 3：lines() 找不到方法——E0599

```rust,noplayground
use tokio::io::BufReader;
use tokio::net::TcpStream;

#[tokio::main]
async fn main() {
    let stream = TcpStream::connect("127.0.0.1:9000").await.unwrap();
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines.next_line().await.unwrap() {
        println!("{}", line);
    }
}
```

```text
error[E0599]: no method named `lines` found for struct `tokio::io::BufReader<R>` in the current scope
   --> src/main.rs:7:44
    |
  7 |     let mut lines = BufReader::new(stream).lines();
    |                                            ^^^^^ method not found in `tokio::io::BufReader<tokio::net::TcpStream>`
    |
   ::: C:\Users\yujinhao\.cargo\registry\src\rsproxy.cn-e3de039b2554c837\tokio-1.54.1\src\io\util\async_buf_read_ext.rs:348:12
    |
348 |         fn lines(self) -> Lines<Self>
    |            ----- the method is available for `tokio::io::BufReader<tokio::net::TcpStream>` here
    |
    = help: items from traits can only be used if the trait is in scope
help: trait `AsyncBufReadExt` which provides `lines` is implemented but not in scope; perhaps you want to import it
    |
  1 + use tokio::io::AsyncBufReadExt;
    |
For more information about this error, try `rustc --explain E0599`.
error: could not compile `chat-server` (bin "e3") due to 1 previous error
```

**原因**：lines 不是 BufReader 的固有方法，而是 AsyncBufReadExt 这个扩展 trait 提供的方法。Rust 的方法解析只查找已引入作用域的 trait，所以 use tokio::io::BufReader 让你能构造类型，却不让你调用它的 trait 方法。报错里那句 the method is available here 加 help 已经把补丁写好了，照抄那一行 use 即可。

**修复**：把扩展 trait 一起引入。它和 BufReader 在同一个模块下，合成一条 use 就行：

```rust,noplayground
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() {
    let stream = TcpStream::connect("127.0.0.1:9000").await.unwrap();
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        println!("{}", line);
    }
}
```

注意顺手把 unwrap 换成了 while let Ok(Some(..))：服务器关掉时 next_line 会返回 Err，unwrap 会让客户端 panic 着退出，而这里只该安静地结束循环。

### 案例 4：跨 await 持有 std MutexGuard——future 不是 Send

```rust,noplayground
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

async fn dump(lock: Arc<Mutex<Vec<String>>>, mut stream: TcpStream) {
    let guard = lock.lock().unwrap();
    for line in guard.iter() {
        stream.write_all(line.as_bytes()).await.unwrap();
    }
}

#[tokio::main]
async fn main() {
    let lock = Arc::new(Mutex::new(vec![String::from("hi")]));
    let stream = TcpStream::connect("127.0.0.1:9000").await.unwrap();
    tokio::spawn(dump(lock, stream));
}
```

```text
error: future cannot be sent between threads safely
   --> src/main.rs:17:18
    |
 17 |     tokio::spawn(dump(lock, stream));
    |                  ^^^^^^^^^^^^^^^^^^ future returned by `dump` is not `Send`
    |
    = help: within `impl Future<Output = ()>`, the trait `Send` is not implemented for `std::sync::MutexGuard<'_, Vec<String>>`
note: future is not `Send` as this value is used across an await
   --> src/main.rs:9:43
    |
  7 |     let guard = lock.lock().unwrap();
    |         ----- has type `std::sync::MutexGuard<'_, Vec<String>>` which is not `Send`
  8 |     for line in guard.iter() {
  9 |         stream.write_all(line.as_bytes()).await.unwrap();
    |                                           ^^^^^ await occurs here, with `guard` maybe used later
note: required by a bound in `tokio::spawn`
   --> C:\Users\yujinhao\.cargo\registry\src\rsproxy.cn-e3de039b2554c837\tokio-1.54.1\src\task\spawn.rs:176:21
    |
174 |     pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
175 |     where
176 |         F: Future + Send + 'static,
    |                     ^^^^ required by this bound in `spawn`
error: could not compile `chat-server` (bin "e4") due to 1 previous error
```

**原因**：这条没有错误码，因为它是 trait 约束不满足，属于 E0277 那个家族。std 的 MutexGuard 不是 Send：它绑定在「加锁的那个线程」上，跨线程释放是非法的。多线程 runtime 会把 task 在线程之间搬运，所以一旦 future 在 await 时还持有 guard，编译器就拒绝把它交给 spawn。error 下面那三行 note 已经精确指出了是哪个值、跨了哪个 await。

**修复**：把锁的作用域缩到 await 之前，先把要用的数据拷出来，guard 在块结束时自动释放：

```rust,noplayground
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

async fn dump(lock: Arc<Mutex<Vec<String>>>, mut stream: TcpStream) {
    let lines: Vec<String> = {
        let guard = lock.lock().unwrap();
        guard.clone()
    };
    for line in lines {
        if stream.write_all(line.as_bytes()).await.is_err() {
            return;
        }
    }
}

#[tokio::main]
async fn main() {
    let lock = Arc::new(Mutex::new(vec![String::from("hi")]));
    let stream = TcpStream::connect("127.0.0.1:9000").await.unwrap();
    tokio::spawn(dump(lock, stream));
}
```

另一条路是换成 tokio::sync::Mutex 再 await 它的 lock，但那样临界区里就可以有 IO，锁会被一个慢 socket 长时间占住，通常比拷贝更糟。本章干脆不用锁：房间状态只归 hub 一个 task 所有，这条错误从根上不会出现。

### 案例 5：遍历 HashMap 的同时插入——E0502

```rust,noplayground
use std::collections::HashMap;

use tokio::sync::mpsc;

struct Client {
    name: String,
    tx: mpsc::Sender<String>,
}

fn add_bot(clients: &mut HashMap<u64, Client>) {
    for (&id, c) in clients.iter() {
        if c.name == "ana" {
            clients.insert(
                id + 100,
                Client { name: String::from("bot"), tx: c.tx.clone() },
            );
        }
    }
}

#[tokio::main]
async fn main() {
    let (tx, _rx) = mpsc::channel(4);
    let mut clients = HashMap::new();
    clients.insert(1, Client { name: String::from("ana"), tx });
    add_bot(&mut clients);
}
```

```text
error[E0502]: cannot borrow `*clients` as mutable because it is also borrowed as immutable
  --> src/main.rs:13:13
   |
11 |        for (&id, c) in clients.iter() {
   |        -               --------------
   |        |               |
   |        |               immutable borrow occurs here
   |  ______|               immutable borrow later used here
   | |
12 | |          if c.name == "ana" {
13 | |/             clients.insert(
14 | ||                 id + 100,
15 | ||                 Client { name: String::from("bot"), tx: c.tx.clone() },
16 | ||             );
   | ||_____________^ mutable borrow occurs here
17 | |          }
18 | |      }
   | |______- this for loop borrows `*clients` immutably, preventing mutation within its body
   |
   = help: consider using an index-based loop instead, or collecting modifications into a separate collection
For more information about this error, try `rustc --explain E0502`.
error: could not compile `chat-server` (bin "e5") due to 1 previous error
```

**原因**：for 循环拿着 clients 的不可变借用，循环体里又要可变借用同一张表。这不是编译器挑剔：插入可能触发扩容，扩容会搬动元素，正在迭代的引用立刻悬空，所以借用规则在这里挡住的是一个真实的内存安全问题。注意它和「先取一个引用、再插入」不同——那种写法靠两阶段借用能过，真正会报错的是「迭代器还活着的时候改容器」。

**修复**：先把要用的东西收集出来，结束借用，再统一写入：

```rust,noplayground
fn add_bot(clients: &mut HashMap<u64, Client>) {
    let targets: Vec<(u64, mpsc::Sender<String>)> = clients
        .iter()
        .filter(|(_, c)| c.name == "ana")
        .map(|(&id, c)| (id, c.tx.clone()))
        .collect();

    for (id, tx) in targets {
        clients.insert(id + 100, Client { name: String::from("bot"), tx });
    }
}
```

collect 这一步就把借用结束了，后面的循环拿的是自有数据。这个模式在 hub 里也会用到：任何「根据表里的内容决定怎么改表」的操作，都先收集，再修改。

## 完整源码

完整项目在 `projects/43-chat-server/`（<https://github.com/yujh129/rust-book-zh/tree/main/projects/43-chat-server>）：`lib.rs` 装全部服务器逻辑（含 5 个 hub 单测和 1 个 `#[tokio::test]` 端到端测试），`main.rs` 只有 31 行的进程入口。

```console
$ cd projects/43-chat-server
$ cargo run --offline -- 127.0.0.1:9000
聊天服务器监听 127.0.0.1:9000
# 另开两个终端，各自 nc 127.0.0.1 9000 并输入昵称
$ cargo test --offline
running 6 tests
test tests::join_announces_to_others_and_who_lists_everyone ... ok
test tests::leave_removes_client_and_announces ... ok
test tests::names_are_sanitized ... ok
test tests::say_is_broadcast_with_name_and_time ... ok
test tests::slow_client_drops_messages_and_counts_them ... ok
test tests::two_clients_chat_end_to_end ... ok
test result: ok. 6 passed; 0 failed
```

| 文件 | 内容 |
| --- | --- |
| `src/lib.rs` | 常量（`OUT_CAPACITY`/`CMD_CAPACITY`/`MAX_NAME_LEN`）、`Cmd`/`Client`/`Hub`、`sanitize_name`、`timestamp`、`hub_loop`、`handle_conn`、`run`、`serve` |
| `src/main.rs` | 参数、`watch` 关闭信号、`spawn(serve)` 加 `select!` 等 Ctrl-C |
| `Cargo.toml` | 唯一依赖是 tokio（`features = ["full"]`） |

43.2 给的是 `handle_conn` 的骨架，43.5 给写 task，43.6 给读循环——项目里能看到它们拼成的完整函数，以及退出时那句 `Leave` 与 `write_task.await`。

## 43.11 动手练习

### 练习 1：加一个 /nick 改名命令

新增 Cmd::Rename，hub 收到后校验新昵称（非空、不重名），成功则广播「旧名 改名为 新名」，失败只回给本人一句提示。难点不在命令解析，而在「重名检查」和「改名广播」必须发生在同一个 match 分支里，否则中间会有一个瞬间在线名单里出现两个同名的人。

验收：三个客户端同时在线的状态下改名，另外两人各看到一条改名通知，/who 的顺序随新名字变化。

### 练习 2：把慢客户端踢掉

给 Client 加一个连续丢弃计数。try_send 返回 Err 时加一，成功时清零；连续丢弃达到 8 条就给 hub 发 Leave 把它移除。同时把 try_send 的两种错误分开处理：Closed 表示对端已经关了，应当立刻移除，不必等阈值。

验收：写一个单元测试，把队列容量设成 1 再连发 10 条，断言该客户端被移除、其他人收到离开通知、hub.len() 减一。再想一下：为什么这个阈值不能太大，也不能太小。

### 练习 3：支持房间

把 `HashMap<u64, Client>` 换成「房间名到成员表」的嵌套结构，新增 /join 房间名 与 /rooms。要点：同一个连接在切换房间时，必须先离开旧房间再进入新房间，两步都要广播；不要在遍历成员表的时候改表，参照案例 5 的收集再修改。

验收：两个客户端在不同房间，A 说话 B 看不到，/rooms 能看到两个房间各自的人数。

### 练习 4：用 broadcast 重写一遍

保留昵称、欢迎语、/who 和 /quit，但底层换成 broadcast::channel。你会被迫给每条消息加收件人过滤（欢迎语只给新人、/who 只给提问者、离开通知跳过本人），并且要处理 Lagged。把容量设成 4，让一个客户端故意慢下来，记录它收到几次 Lagged、跳过了多少条。

验收：写一段说明，讲清在什么负载下 broadcast 版本比 hub 版本更简单，以及它在哪一刻开始变得难以推理。

### 练习 5：出站类型换成 `Arc<str>`

把 Cmd、Client、写 task 里的 String 全换成 `Arc<str>`，让每条广播只 clone 一次句柄而不是整条消息。改动会波及 Cmd 的定义、Hub 的五个分支、handle_conn 里的类型，以及测试里的断言（比较时用 &*line 或 line.as_ref()）。

验收：cargo test 全过；在 5000 条消息、10 个客户端的压测里，用全局分配计数器对比改造前后的分配次数，解释差在哪里。

## 43.12 小结

1. 状态独占优于共享加锁。房间表只归 hub 一个 task 所有，全项目没有一个 Mutex，案例 4 那类跨 await 持锁的错误从根上不会出现。
2. 每连接两条 task 是背压的落点。读 task 只产出 Cmd，写 task 独占 socket，hub 只做不等待的 try_send，「一个慢客户端拖住所有人」才真正被消除。
3. broadcast 是有损扇出，mpsc 是有界背压的点对点通道。聊天室要的是后者；broadcast 的 Lagged 不是 bug，是它的设计前提被违反时的表现。
4. 有界队列加丢最新，是「慢客户端不拖累别人」的可执行定义；dropped 计数是这条策略唯一的观测面，没有它就是在盲飞。
5. lines() 需要 BufReader 和 AsyncBufReadExt 同时到位，写侧需要 BufWriter 加显式 flush。忘记 flush 的典型症状是本机碰巧能过、换台机器就卡住。
6. select! 的正确性取决于被取消的 future 是否可取消安全。read_line 不是，所以每轮新建 String，并且只在完成分支里使用它。
7. 优雅退出是一条有顺序的链条：信号、accept 停、hub 退、丢掉 Client、写 task 收到 None、连接 task 结束、JoinSet 兜底。顺序反了就会卡在某个 await 上。
8. spawn 要求 Send 加 static，意味着所有权必须拆开：into_split 拆 socket，clone 发句柄，Arc 共享只读数据。它也意味着跨 await 不能持有非 Send 的 guard。
9. 泄漏有三种形态：忘记发 Leave、发了 Leave 但没 remove、只 spawn 不 join。Drop 是收尾机制，但别指望在 Drop 里 await。
10. 测试要分层：hub 用 mpsc 做单元测试，快且能覆盖策略；端到端用 127.0.0.1:0 起真服务器，覆盖协议、广播顺序和退出路径。
