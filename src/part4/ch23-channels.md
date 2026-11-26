# 通道：线程之间传消息

> **本章目标**
>
> - 掌握 `std::sync::mpsc` 的 `Sender`/`Receiver` 模型，能写多生产者单消费者
> - 理解 `send` 是**所有权转移**，这正是通道比共享内存更安全的原因
> - 用**有界通道** `sync_channel` 实现背压（backpressure）
> - 知道通道关闭的语义：`recv` 返回 `Err` 意味着所有发送者都已销毁
> - 用通道实现 pipeline 和 actor 模式
> - 理解通道与锁的分工：**共享内存 vs 消息传递**

第 21 章解决的是"怎么开一个线程"，第 22 章解决的是"多个线程怎么共享同一份数据"。这一章讲另一条经典路线：**不共享，改传消息**。

这是并发编程的两大流派：

```text
共享内存：  线程 A ──┐
                     ├──> [同一块内存]  ← 需要锁/原子
            线程 B ──┘

消息传递：  线程 A ──> [通道] ──> 线程 B   ← 所有权随消息转移
```

Rust 官方文档有句名言："Do not communicate by sharing memory; instead, share memory by communicating."（不要通过共享内存来通信，而要通过通信来共享内存。）通道就是这个理念的载体。

## 23.1 第一个通道程序

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    // mpsc = multi-producer, single-consumer
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let values = vec![1, 2, 3, 4, 5];
        for v in values {
            tx.send(v).unwrap();
        }
        // tx 在这里被 drop，通道关闭
    });

    // rx 是迭代器：不断接收直到所有 Sender 被销毁
    for received in rx {
        println!("收到：{}", received);
    }
}
```

几个关键点：

- `mpsc::channel()` 返回 `(Sender<T>, Receiver<T>)` 二元组
- `tx` 被 `move` 进子线程，`rx` 留在主线程——所有权把"发送方"和"接收方"分到了两边
- 当所有 `Sender` 都被 drop，`rx` 的迭代自动结束（`recv()` 返回 `Err(RecvError)`）
- `send` 返回 `Result<(), SendError<T>>`：失败时会把消息**还给你**

### send 失败时消息去哪了

```rust
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel::<String>();
    drop(rx);

    match tx.send(String::from("你好")) {
        Ok(()) => println!("发送成功"),
        Err(e) => {
            // SendError<String> 里带着原始消息
            let msg = e.0;
            println!("发送失败，拿回消息：{}", msg);
        }
    }
}
```

`SendError<T>` 包含 `T`，所以消息不会丢。这个设计很重要：如果接收端已经没了，你能把数据拿回来做重试或落盘。

## 23.2 多个生产者

`Sender` 实现了 `Clone`，`Receiver` **没有**。这从类型层面保证了"多生产者、单消费者"：

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::channel();

    for id in 0..3 {
        let tx = tx.clone();
        thread::spawn(move || {
            for i in 0..3 {
                tx.send((id, i)).unwrap();
            }
        });
    }
    // 关键：必须 drop 原始的 tx，否则 for 循环永远不会结束
    drop(tx);

    let mut results: Vec<(i32, i32)> = rx.iter().collect();
    results.sort();
    println!("{:?}", results);
    println!("共收到 {} 条", results.len());
}
```

**最容易踩的坑**：忘了 `drop(tx)`。原始的 `tx` 还在主线程活着，通道就永远不算"所有发送者都关闭"，`rx.iter()` 会一直阻塞。

### 用通道汇总结果

多线程计算的经典模式：每个 worker 算一部分，通过通道把结果送回主线程。

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::channel();
    let data: Vec<i32> = (1..=10).collect();

    for chunk in data.chunks(2) {
        let tx = tx.clone();
        let chunk = chunk.to_vec();
        thread::spawn(move || {
            let sum: i32 = chunk.iter().sum();
            tx.send(sum).unwrap();
        });
    }
    drop(tx);

    let total: i32 = rx.iter().sum();
    println!("总和：{}", total);
}
```

这个模式的好处：worker 之间完全隔离，主线程只做汇总。**没有锁，没有共享状态**，所有数据流都是单向的。

## 23.3 有界通道与背压

`mpsc::channel()` 是**无界**的：`send` 永不阻塞，如果生产速度远超消费速度，队列会无限增长直到 OOM。

`sync_channel(n)` 创建**有界**通道：容量满时 `send` 阻塞，直到消费者取走一个。这就是**背压**（backpressure）。

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    // 容量为 2：最多缓存 2 条消息
    let (tx, rx) = mpsc::sync_channel(2);

    let producer = thread::spawn(move || {
        for i in 0..5 {
            println!("生产 {}", i);
            tx.send(i).unwrap();
            println!("  {} 已入队", i);
        }
    });

    thread::sleep(Duration::from_millis(200));
    for v in rx {
        println!("消费 {}", v);
        thread::sleep(Duration::from_millis(200));
    }
    producer.join().unwrap();
}
```

观察输出：前两条 `send` 立刻返回，之后每条都要等消费者取走一条。生产者的速度自动被"限流"到消费者的速度。

### sync_channel(0)：会合（rendezvous）

容量 0 是一个特例：`send` 会一直阻塞到**接收方准备好接收**为止。两个线程在这一点"会合"，没有任何缓冲。

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::sync_channel(0);

    let h = thread::spawn(move || {
        println!("发送方：准备发送");
        tx.send("hello").unwrap();
        println!("发送方：对方收到了");
    });

    println!("接收方：先干点别的");
    let msg = rx.recv().unwrap();
    println!("接收方：收到 {}", msg);
    h.join().unwrap();
}
```

`sync_channel(0)` 适合"确认对方已经接住"的场景，但容易死锁，实践中用 1–100 的容量更常见。

### 非阻塞尝试

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        tx.send(42).unwrap();
    });

    // try_recv：立刻返回，没有消息时是 Err(TryRecvError::Empty)
    match rx.try_recv() {
        Ok(v) => println!("马上拿到 {}", v),
        Err(e) => println!("暂时没有：{:?}", e),
    }

    // recv_timeout：最多等 1 秒
    match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(v) => println!("等到 {}", v),
        Err(e) => println!("超时：{:?}", e),
    }
}
```

三个接收方法的选择：

| 方法 | 行为 | 适用 |
|---|---|---|
| `recv()` | 一直阻塞 | 后台 worker 的主循环 |
| `try_recv()` | 立即返回 `Result` | 轮询、不想阻塞时 |
| `recv_timeout(d)` | 最多阻塞 d | 需要定期做别的事（心跳、超时） |

## 23.4 关闭语义：recv 返回 Err 意味着什么

通道的关闭不是显式调用，而是**所有 Sender 被 drop 时自动发生**。

```rust
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel::<i32>();

    tx.send(1).unwrap();
    tx.send(2).unwrap();
    drop(tx);

    // 已经缓冲的消息还能取出来
    while let Ok(v) = rx.recv() {
        println!("拿到 {}", v);
    }
    println!("通道已关闭，迭代结束");
}
```

语义细节：

- 缓冲里的消息**不会丢**，recv 会把它们全部交出
- 缓冲空了且所有发送者已 drop，recv 立刻返回 Err(RecvError)
- `for v in rx` 就是 `while let Ok(v) = rx.recv()` 的语法糖，所以会正常结束

接收端也可以主动关闭：drop(rx) 之后发送方 send 会失败。

```rust
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel::<i32>();
    drop(rx);
    match tx.send(1) {
        Ok(()) => println!("没人接收也能发"),
        Err(e) => println!("发送失败，消息被退回：{}", e.0),
    }
}
```

### 检测「所有 worker 已结束」

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();

    for id in 0..3 {
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            tx.send(id).unwrap();
        }));
    }
    drop(tx);

    let mut done = 0;
    for _ in 0..3 {
        if rx.recv().is_ok() {
            done += 1;
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    println!("{} 个 worker 完成", done);

    match rx.recv_timeout(Duration::from_millis(50)) {
        Ok(v) => println!("还有消息 {}", v),
        Err(e) => println!("结束：{:?}", e),
    }
}
```

## 23.5 用通道实现 actor 模式

actor 是一种并发模型：每个 actor 有自己的状态，**只能通过消息修改**，内部串行处理，天然无锁。

```rust
use std::sync::mpsc::{self, Sender};
use std::thread;

enum Command {
    Add(i64),
    Get(Sender<i64>),
    Quit,
}

struct Counter {
    rx: mpsc::Receiver<Command>,
}

impl Counter {
    fn run(self) {
        let mut total = 0i64;
        for cmd in self.rx {
            match cmd {
                Command::Add(n) => total += n,
                Command::Get(reply) => {
                    reply.send(total).unwrap();
                }
                Command::Quit => break,
            }
        }
    }
}

fn main() {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || Counter { rx }.run());

    tx.send(Command::Add(10)).unwrap();
    tx.send(Command::Add(32)).unwrap();

    let (reply_tx, reply_rx) = mpsc::channel();
    tx.send(Command::Get(reply_tx)).unwrap();
    println!("当前总数：{}", reply_rx.recv().unwrap());

    tx.send(Command::Quit).unwrap();
    handle.join().unwrap();
    println!("counter 线程已退出");
}
```

这个模式的价值：

- 状态（total）只在 actor 线程内被访问，**不需要任何锁**
- 外部只能发命令，不能直接改状态，天然避免了竞态
- Command::Get 里带一个回复通道，实现了「请求-响应」
- actor 可以扩展成 worker 池（每个 worker 一个通道）

tokio 的 mpsc、actix 框架都是这个思路的工业化版本。

### pipeline：多级流水线

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx1, rx1) = mpsc::channel::<i32>();
    let (tx2, rx2) = mpsc::channel::<i32>();

    let stage1 = thread::spawn(move || {
        for i in 1..=5 {
            tx1.send(i).unwrap();
        }
    });

    let stage2 = thread::spawn(move || {
        for v in rx1 {
            tx2.send(v * v).unwrap();
        }
    });

    let stage3 = thread::spawn(move || {
        let sum: i32 = rx2.iter().sum();
        println!("平方和：{}", sum);
    });

    stage1.join().unwrap();
    stage2.join().unwrap();
    stage3.join().unwrap();
}
```

每一级只关心「从上游拿、处理后交给下游」，级与级之间通过通道解耦。这是 Unix 管道思想在进程内的复刻。

## 23.6 深入：通道的内部实现与 crossbeam

### 标准库 mpsc 是怎么实现的

标准库的 mpsc::channel 本质是一个**加锁的队列**：内部有互斥锁保护的队列加一个条件变量（Condvar）负责唤醒等待的接收者。

这意味着：

- send 会短暂持锁，有竞争时可能排队
- recv 在空队列上阻塞时在 Condvar 上等待，被唤醒后要重新抢锁
- 吞吐量受锁限制，但通常能达到每秒百万级消息

对绝大多数应用这已经够用。高频小消息场景（每秒几十万条以上）下锁才会成为瓶颈。

### crossbeam-channel：更快的 MPMC

crossbeam-channel 提供了：

- **MPMC**（多消费者；标准库只有单消费者）
- select! 宏：同时等待多个通道，谁先来先处理
- 更好的性能（无锁数据结构 + 批量操作）

```rust,noplayground
use crossbeam_channel::{select, unbounded};
use std::thread;
use std::time::Duration;

fn main() {
    let (tx1, rx1) = unbounded();
    let (tx2, rx2) = unbounded();

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        tx1.send("慢通道").unwrap();
    });
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        tx2.send("快通道").unwrap();
    });

    select! {
        recv(rx1) -> msg => println!("{:?}", msg),
        recv(rx2) -> msg => println!("{:?}", msg),
    }
}
```

这段依赖第三方 crate，所以标了 noplayground；本地 cargo add crossbeam-channel 之后可以直接运行，输出「快通道」。

select! 就是「多路复用」：tokio 里叫 tokio::select!，Go 里是 select 语句。

### 通道 vs 锁：怎么选

| 场景 | 推荐 |
|---|---|
| 任务分发（一个队列多个 worker） | 通道（标准库 mpsc，多消费者用 crossbeam） |
| 共享只读配置 | Arc&lt;T&gt;，不用锁也不用通道 |
| 高频读、低频写 | RwLock 或 ArcSwap |
| 计数器/标志位 | 原子类型 |
| 请求-响应、状态机 | 通道（actor） |
| 大量随机访问同一结构 | Mutex |

经验法则：**数据流是单向的，用通道；需要随机访问的共享状态，用锁。** 通道天然避免了「多把锁的获取顺序」这类死锁，但可能因为忘关通道而永久阻塞。

### 为什么通道能避免数据竞争

send 会**移动**消息进通道：

```rust,noplayground
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel();
    let v = String::from("hello");
    tx.send(v).unwrap();
    println!("{}", v); // 编译错误：v 已经被移走
    drop(rx);
}
```

从这个角度看，通道是「所有权的传递管道」：任何时刻一条消息只有一个所有者（在发送方、在队列里、或在接收方）。**数据竞争的必要条件「两个线程同时访问同一块内存」从根上就不成立**，所以不需要锁。

这也是为什么 T 必须满足 Send：消息要跨线程，所有权要转移。

## 23.7 典型错误案例

### 案例 1：send 之后继续使用被移动的值

```rust,editable
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let v = String::from("hello");
        tx.send(v).unwrap();
        println!("{}", v);
    });
    println!("{:?}", rx.recv().unwrap());
}
```

```text
error[E0382]: borrow of moved value: `v`
 --> src/main.rs:9:24
  |
7 |         let v = String::from("hello");
  |             - move occurs because `v` has type `String`, which does not implement the `Copy` trait
8 |         tx.send(v).unwrap();
  |                 - value moved here
9 |         println!("{}", v);
  |                        ^ value borrowed here after move

error: aborting due to 1 previous error
```

**原因**：send 的签名是 fn send(&self, t: T)，参数按值接收，所以 v 的所有权被移动进通道。移动之后 v 在编译期就失效了，不能再用。

**修复**：在发送前先用完，或者发送克隆：

```rust
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel();
    let v = String::from("hello");
    tx.send(v.clone()).unwrap();
    println!("本地还留着：{}", v);
    println!("通道里收到：{:?}", rx.recv().unwrap());
}
```

### 案例 2：发送的类型与通道的元素类型不一致

```rust,editable
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel::<i32>();
    tx.send(String::from("oops")).unwrap();
    println!("{}", rx.recv().unwrap());
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:5:13
  |
5 |     tx.send(String::from("oops")).unwrap();
  |        ---- ^^^^^^^^^^^^^^^^^^^^ expected `i32`, found `String`
  |        |
  |        arguments to this method are incorrect
  |
help: the return type of this call is `String` due to the type of the argument passed
  |
5 |     tx.send(String::from("oops")).unwrap();
  |     ^^^^^^^^--------------------^
  |             |
  |             this argument influences the return type of `send`
note: method defined here
  |     pub fn send(&self, t: T) -> Result<(), SendError<T>>
```

**原因**：`mpsc::channel::<i32>()` 把 `T` 固定成了 `i32`，`Sender<i32>::send` 只能收 `i32`。通道的元素类型在创建时就确定了，之后不能改。

**修复**：统一类型。如果确实要发多种类型，用一个 enum 把它们包起来：

```rust
use std::sync::mpsc;

enum Msg {
    Number(i32),
    Text(String),
}

fn main() {
    let (tx, rx) = mpsc::channel::<Msg>();
    tx.send(Msg::Number(1)).unwrap();
    tx.send(Msg::Text(String::from("hi"))).unwrap();
    drop(tx); // 最后一个发送端离开后，迭代器读完队列即可结束
    for m in rx {
        match m {
            Msg::Number(n) => println!("数字 {}", n),
            Msg::Text(s) => println!("文本 {}", s),
        }
    }
}
```

### 案例 3：把 Vec 直接 println! 出来

```rust,editable
use std::sync::mpsc;

fn main() {
    let (tx, rx) = mpsc::channel::<Vec<i32>>();
    tx.send(vec![1, 2, 3]).unwrap();
    let v = rx.recv().unwrap();
    println!("{}", v);
}
```

```text
error[E0277]: `Vec<i32>` doesn't implement `std::fmt::Display`
 --> src/main.rs:7:20
  |
7 |     println!("{}", v);
  |               --   ^ `Vec<i32>` cannot be formatted with the default formatter
  |               |
  |               required by this formatting parameter
  |
  = help: the trait `std::fmt::Display` is not implemented for `Vec<i32>`
  = note: in format strings you may be able to use `{:?}` (or `{:#?}` for pretty-print) instead
```

**原因**：Display 要求「只有一种人类可读的表示形式」。Vec 有多种合理表示（[1, 2, 3]　或换行列表），所以只实现了 Debug。这个错误与通道无关，是通道让 Vec 变成了打印对象才暴露出来。

**修复**：用 {:?} 或 `{:#?}`，或者用 join 自己格式化：

```rust
fn main() {
    let v = vec![1, 2, 3];
    println!("{:?}", v);
    println!("{}", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "));
}
```

### 案例 4（运行期）：sync_channel(0) 双方互相等待

```rust,noplayground
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::sync_channel(0);
    let h = thread::spawn(move || {
        tx.send(1).unwrap();
        // 这里等主线程接收
    });

    // 主线程先 join，子线程永远发不出去
    h.join().unwrap();
    println!("{}", rx.recv().unwrap());
}
```

```text
（程序不会报错，也不会结束：子线程阻塞在 send 上等接收，
  主线程阻塞在 join 上等子线程。Ctrl+C 才能停。）
```

**原因**：容量 0 意味着 send 必须与 recv 会合。这里 join 在 recv 之前，顺序反了，两个线程永远等不到对方。这类错误编译器帮不了你，只能靠理解语义。

**修复**：先收再 join，或者给通道一个非零容量：

```rust
use std::sync::mpsc;
use std::thread;

fn main() {
    let (tx, rx) = mpsc::sync_channel(0);
    let h = thread::spawn(move || {
        tx.send(1).unwrap();
    });
    let v = rx.recv().unwrap();
    println!("收到 {}", v);
    h.join().unwrap();
}
```

### 案例 5（运行期）：忘记 drop 原始 Sender，for 循环永不结束

```rust,noplayground
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    thread::spawn(move || {
        tx2.send(1).unwrap();
    });
    // 忘了 drop(tx)

    for v in rx {
        println!("收到 {}", v);
    }
    println!("结束了");
}
```

```text
收到 1
（然后永远卡住：主线程自己还持有一个 Sender，通道永远不算关闭）
```

**原因**：for v in rx 只在「缓冲空且所有 Sender 已 drop」时结束。原始的 tx 还在 main 的作用域里活着，所以 rx 认为「可能还有消息要来」，一直阻塞。

**修复**：在开始接收之前 drop(tx)；或者用 recv_timeout 避免无限等待：

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    thread::spawn(move || {
        tx2.send(1).unwrap();
    });
    drop(tx);

    while let Ok(v) = rx.recv_timeout(Duration::from_millis(500)) {
        println!("收到 {}", v);
    }
    println!("结束了");
}
```

## 23.8 动手练习

1. **基本通道**：主线程 spawn 一个线程计算 1 到 100 的和，用通道把结果送回来并打印。要求不用 `JoinHandle::join` 的返回值。
2. **worker 池**：创建 4 个 worker 线程，共享一个任务通道；主线程发送 10 个任务（每个任务是「计算 n 的平方」），worker 用结果通道送回结果。统计所有结果的和。注意要让所有 worker 正常退出。
3. **有界背压**：用 `sync_channel(4)` 写一个生产者-消费者程序，生产者在 10 条消息里每发一条打印时间戳（`std::time::Instant`），消费者每 100 ms 取一条。观察生产者的速度被限制，并解释为什么前 4 条几乎是瞬间发出的。
4. **actor 升级**：把 23.5 的 `Counter` 扩展成支持 `Mul(i64)`、`Reset` 和 `Stats(Sender<String>)`（返回形如 `总数=xx 操作数=yy` 的字符串）。要求 `Stats` 里再嵌套一次请求-响应。
5. **超时与重试**：写一个函数 `fn wait_for(rx: &Receiver<i32>, timeout: Duration) -> Option<i32>`，内部用 `recv_timeout`，超时返回 `None`。再写一段代码模拟「3 次发送都延迟 200 ms、超时设成 100 ms」的场景，打印每次的结果。

## 23.9 小结

- `mpsc::channel()` 返回 `(Sender<T>, Receiver<T>)`；`Sender` 可 clone，`Receiver` 不可，这就是「多生产者单消费者」的类型级保证。
- `send` 按值接收消息，所有权随消息转移；失败时 `SendError<T>` 把消息还给你，不丢数据。
- 所有 `Sender` 被 drop 后通道自动关闭；`recv` 返回 `Err(RecvError)`，`for v in rx` 正常结束。
- 缓冲中的消息在关闭后仍能取完，关闭不等于丢弃。
- `sync_channel(n)` 是有界通道：队列满时 `send` 阻塞，用容量换内存安全，这就是背压。
- `sync_channel(0)` 是会合通道：`send` 必须等到 `recv`，顺序写反就是死锁。
- 接收三件套：`recv()` 阻塞、`try_recv()` 立即返回、`recv_timeout(d)` 限时等待。
- actor 模式用「命令 enum + 回复通道」把状态封装进单线程，天然免锁；`Get(Sender<T>)` 是最常用的请求-响应样式。
- 通道让「数据流单向」的程序免锁，锁适合「随机访问共享状态」的程序；二者不是替代关系，而是分工。
- 忘 `drop(tx)`、容量写 0 后顺序颠倒、`send` 后继续用被移动的值，是通道代码里最高频的三个坑。
