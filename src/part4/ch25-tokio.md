# Tokio：工业级异步运行时

> **本章目标**
>
> - 理解「Future 只是描述，运行时负责驱动」这件事，知道 tokio 提供了什么
> - 用 `#[tokio::main]` 写第一个异步程序，掌握 `tokio::spawn` 与 `.await`
> - 熟悉 tokio 的核心原语：`oneshot`/`mpsc`/`watch`/`broadcast`/`Mutex`/`Semaphore`
> - 知道哪些操作会阻塞运行时，以及怎么用 `spawn_blocking` 兜住
> - 理解 work-stealing 调度器、current_thread 与 multi_thread 的区别
> - 避开「std 锁跨 await」「future 不是 Send」「忘了 .await」这些高频坑

第 24 章我们自己写了一个最小执行器：一个 `VecDeque<Arc<Task>>` 加一个 `Waker`。它能把 Future 跑起来，但没有 IO 驱动、没有定时器、没有多线程调度——真实项目不会用它。

这一章讲 tokio，它是 Rust 生态里事实标准的异步运行时。理解了第 24 章的 `poll` 和 `Waker`，这一章就只是「工程化的实现细节」。

## 25.1 为什么还需要一个运行时

`async`/`await` 是**语言**提供的语法：它把 `async fn` 编译成一台状态机，实现 `Future` trait。但语言**不提供**以下任何东西：

| 能力 | 语言 | tokio |
|---|---|---|
| 把 Future 推进到完成 | 只能手动 `poll` | 执行器自动驱动 |
| IO 就绪通知 | 无 | epoll/IOCP/kqueue 事件循环 |
| 定时器 | 无 | `tokio::time` |
| 多线程任务分发 | 无 | work-stealing 调度器 |
| 通道、锁、信号量 | 无（`std` 的是阻塞版） | `tokio::sync` |

所以「用 async 写代码」和「用 tokio 跑代码」是两件事。你也可以用 `async-std`、`smol`、`embassy`（嵌入式），它们的 API 不同，但都实现同一套 `Future` trait。

### 加依赖

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
```

`full` 打开所有特性。生产项目通常只开需要的，编译更快：

```toml
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "sync"] }
```

| feature | 提供 |
|---|---|
| `rt` | 单线程运行时 `Runtime::new()` |
| `rt-multi-thread` | 多线程运行时（默认 `#[tokio::main]` 用它） |
| `macros` | `#[tokio::main]`、`#[tokio::test]`、`select!` |
| `net` | `TcpStream`、`TcpListener`、`UdpSocket` |
| `time` | `sleep`、`timeout`、`interval` |
| `sync` | `Mutex`、`mpsc`、`oneshot`、`Semaphore` 等 |
| `fs` | 异步文件 API（内部其实是线程池） |

## 25.2 第一个 tokio 程序

```rust,noplayground
use std::time::Duration;

#[tokio::main]
async fn main() {
    let h1 = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        1
    });
    let h2 = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        2
    });

    let (a, b) = tokio::join!(h1, h2);
    println!("{} + {} = {}", a.unwrap(), b.unwrap(), a.unwrap() + b.unwrap());
}
```

`#[tokio::main]` 是一个属性宏，把 `async fn main()` 展开成：

```rust,noplayground
fn main() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // 原来的 async main 函数体
        })
}
```

这个展开解释了第 24 章的错误 `main function is not allowed to be async`：语言层面 `main` 必须是同步函数，是宏帮你套了一层 `block_on`。

### spawn 与 join

| 写法 | 含义 |
|---|---|
| `.await` | 等这个 Future 完成，**串行** |
| `tokio::spawn(fut)` | 把 Future 丢给运行时，**并行**执行，返回 `JoinHandle<T>` |
| `tokio::join!(a, b)` | 同时等多个，全部完成后一次性拿到结果 |
| `tokio::try_join!(a, b)` | 同上，但任一失败就提前返回 `Err` |
| `tokio::select!` | 谁先完成就先处理谁，其余被取消 |

```rust,noplayground
use std::time::Duration;

async fn fetch(id: u32) -> Result<String, String> {
    tokio::time::sleep(Duration::from_millis(10)).await;
    if id == 2 {
        Err(String::from("id 2 失败"))
    } else {
        Ok(format!("结果-{}", id))
    }
}

#[tokio::main]
async fn main() {
    // try_join!：任一 Err 立刻返回
    let res = tokio::try_join!(fetch(1), fetch(2));
    println!("try_join 结果：{:?}", res);

    // select!：谁先完成先处理
    tokio::select! {
        r = fetch(1) => println!("先完成的是 fetch(1)：{:?}", r),
        r = fetch(3) => println!("先完成的是 fetch(3)：{:?}", r),
    }
}
```

输出形如：

```text
try_join 结果：Err("id 2 失败")
先完成的是 fetch(1)：Ok("结果-1")
```

注意 `select!` 的分支是**竞争**的：两个分支同时开始，先完成的分支赢，另一个 Future 被 drop（即取消）。如果两个同时完成，默认随机选一个——这种不确定性在需要严格顺序时要小心。

### 串行 vs 并发：一个常见误解

```rust,noplayground
use std::time::{Duration, Instant};

async fn work(name: &str, ms: u64) -> String {
    tokio::time::sleep(Duration::from_millis(ms)).await;
    format!("{}-done", name)
}

#[tokio::main]
async fn main() {
    let t = Instant::now();
    // 串行：总耗时 100ms + 100ms
    let a = work("A", 100).await;
    let b = work("B", 100).await;
    println!("串行 {:?} {:?} 耗时 {:?}", a, b, t.elapsed());

    let t = Instant::now();
    // 并发：总耗时约 100ms
    let (a, b) = tokio::join!(work("A", 100), work("B", 100));
    println!("并发 {:?} {:?} 耗时 {:?}", a, b, t.elapsed());
}
```

输出：`串行 ... 耗时 200ms 左右`，`并发 ... 耗时 100ms 左右`。

`async` 不会自动让你变快：`.await` 就是顺序执行，只有 `join!`/`spawn` 才是并发。这一点和第 21 章的线程完全一致。

## 25.3 运行时提供的同步原语

`std::sync` 里的锁和通道是**阻塞**的，在异步代码里不能用（会卡住整个 worker 线程）。tokio 提供了一套异步版本：

| 需求 | 异步版 | 与 std 的区别 |
|---|---|---|
| 互斥锁 | `tokio::sync::Mutex` | `.lock().await`，允许跨 await 持有 |
| 读写锁 | `tokio::sync::RwLock` | `.read().await` / `.write().await` |
| 一次性通知 | `tokio::sync::oneshot` | 单个值，单次 |
| 多值通道 | `tokio::sync::mpsc` | `.send().await`，有界/无界 |
| 状态广播 | `tokio::sync::watch` | 只保留最新值，读者拿快照 |
| 事件广播 | `tokio::sync::broadcast` | 每个订阅者都收到所有消息 |
| 限流 | `tokio::sync::Semaphore` | 控制同时进行的数量 |
| 通知 | `tokio::sync::Notify` | 唤醒等待者，不带数据 |

### oneshot：一次请求-响应

```rust,noplayground
use tokio::sync::oneshot;

async fn compute() -> i32 {
    42
}

#[tokio::main]
async fn main() {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let v = compute().await;
        let _ = tx.send(v); // 接收端可能已经放弃
    });
    match rx.await {
        Ok(v) => println!("收到 {}", v),
        Err(_) => println!("发送方已关闭"),
    }
}
```

`oneshot` 的 `send` 返回 `Result<(), T>`：接收端 drop 后发送会失败。`rx.await` 返回 `Result<T, RecvError>`。

### mpsc：异步多生产者单消费者

```rust,noplayground
use std::time::Duration;

#[tokio::main]
async fn main() {
    // 容量 8：满了 send().await 会等待
    let (tx, mut rx) = tokio::sync::mpsc::channel::<i32>(8);

    for id in 0..3 {
        let tx = tx.clone();
        tokio::spawn(async move {
            for i in 0..3 {
                if tx.send(id * 10 + i).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }
    drop(tx);

    let mut sum = 0;
    while let Some(v) = rx.recv().await {
        sum += v;
    }
    println!("总和 {}", sum);
}
```

三个与 std 的差异：

1. `send` 是 async 的，队列满时 `.await` 而不是阻塞线程
2. `recv` 返回 `Option<T>`，`None` 表示所有发送者已 drop（和 std 的 `Err` 语义相同，写法更顺手）
3. 接收端方法需要 `&mut rx`，所以 `rx` 要声明成 `mut`

## 25.4 时间：sleep、timeout 与 interval

`tokio::time` 提供异步定时器。**不要**在异步代码里用 `std::thread::sleep`，它会卡住整个工作线程。

```rust,noplayground
use std::time::Duration;
use tokio::time::{interval, sleep, timeout, Instant};

async fn slow() -> &'static str {
    sleep(Duration::from_millis(300)).await;
    "完成"
}

#[tokio::main]
async fn main() {
    // sleep：让出线程，其他任务可以继续跑
    let t = Instant::now();
    sleep(Duration::from_millis(100)).await;
    println!("睡了 {:?}", t.elapsed());

    // timeout：超时返回 Err(Elapsed)，Future 被取消
    match timeout(Duration::from_millis(100), slow()).await {
        Ok(v) => println!("成功：{}", v),
        Err(_) => println!("超时了"),
    }

    // interval：周期性触发，第一次会立即 tick
    let mut ticker = interval(Duration::from_millis(50));
    for i in 0..3 {
        ticker.tick().await;
        println!("第 {} 次 tick", i);
    }
}
```

`timeout` 到期返回 `Elapsed`；丢弃包装器会丢弃其拥有的内部 future。计时器不能抢占一次迟迟不返回的 `poll`。若传入 `tokio::spawn` 返回的 `JoinHandle`，丢弃句柄只会分离任务；需要停止该任务时应保留句柄并显式 `abort`。

### 超时与重试的组合

```rust,noplayground
use std::time::Duration;
use tokio::time::{sleep, timeout};

async fn flaky(attempt: u32) -> Result<u32, String> {
    sleep(Duration::from_millis(20)).await;
    if attempt < 3 {
        Err(format!("第 {} 次失败", attempt))
    } else {
        Ok(attempt)
    }
}

async fn retry<F, Fut, T>(mut f: F, max: u32) -> Result<T, String>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last = String::new();
    for i in 1..=max {
        match timeout(Duration::from_millis(120), f(i)).await {
            Ok(Ok(v)) => return Ok(v),
            Ok(Err(e)) => last = e,
            Err(_) => last = format!("第 {} 次超时", i),
        }
        sleep(Duration::from_millis(10 * i as u64)).await; // 退避
    }
    Err(format!("{} 次后仍失败：{}", max, last))
}

#[tokio::main]
async fn main() {
    match retry(flaky, 4).await {
        Ok(v) => println!("第 {} 次成功", v),
        Err(e) => println!("最终失败：{}", e),
    }
}
```

输出：`第 3 次成功`。这个 `retry` 函数是异步版本的通用重试器，`F: FnMut(u32) -> Fut` 的写法值得记住——每次调用返回一个新 Future。

## 25.5 阻塞操作：`spawn_blocking` 与 `block_in_place`

运行时的线程数是有限的（默认等于 CPU 核数）。如果某个任务**同步阻塞**住线程，它占着的 worker 就没法跑其他任务。

```rust,noplayground
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let t = Instant::now();
    let ticker = tokio::spawn(async {
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            println!("tick at {:?}", t.elapsed());
        }
    });

    // 错误示范：阻塞了工作线程 200ms
    std::thread::sleep(Duration::from_millis(200));

    ticker.await.unwrap();
}
```

在单核机器或 `current_thread` 运行时上，输出会是三个 tick 全部挤在 200ms 之后——因为唯一的线程被 sleep 占住了。在多核机器上可能不容易复现，这正是这类 bug 危险的原因：**本机正常，线上偶发**。

正确做法有两种：

```rust,noplayground
use std::time::Duration;

#[tokio::main]
async fn main() {
    // 方案 1：spawn_blocking，把阻塞任务丢到专门的阻塞线程池
    let h = tokio::task::spawn_blocking(|| {
        std::thread::sleep(Duration::from_millis(200));
        42
    });
    println!("阻塞任务结果：{}", h.await.unwrap());

    // 方案 2：换成一个真正的异步 API
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("异步等待完成");
}
```

| 场景 | 做法 |
|---|---|
| CPU 密集计算（几百 ms 以上） | `spawn_blocking`，或干脆用线程池 + 通道 |
| 同步 IO（老库、文件、数据库驱动） | `spawn_blocking` |
| 已有异步 API | 直接用 `tokio::time`、`tokio::fs`、`tokio::net` |
| 在 `block_on` 里嵌套 `block_on` | 会 panic，改成 `spawn_blocking` |

`tokio::fs` 的所有函数内部都是 `spawn_blocking` 的包装——因为操作系统文件 API 本身就是阻塞的（Linux 的 io_uring 是例外，tokio 支持有限）。

## 25.6 深入：运行时是怎么调度的

### work-stealing 调度器

`#[tokio::main]` 默认创建 `multi_thread` 运行时：

```text
        全局任务队列
             │
   ┌─────────┼─────────┐
   ▼         ▼         ▼
worker0   worker1   worker2     ← 每个线程一个本地队列
   │         │         │
   └──── 偷任务 ───────┘        ← 本地队列空了就去别人那里偷
```

关键机制：

- 每个 worker 有自己的**本地队列**，`spawn` 先从本线程队列取，无竞争、很快
- 本地队列空了，去全局队列或其他 worker 的队列**偷一半**过来（work stealing）
- 任务被 `await` 挂起时，worker 立刻转去跑别的任务，不空转
- IO 就绪事件由 **epoll/IOCP** 收集，转成 `Waker::wake()` 把任务重新入队

这就是为什么单线程能同时「等待」成千上万个连接：等待不需要线程，只需要一个记录「谁在等什么」的表。

### 两种运行时

| | `current_thread` | `multi_thread` |
|---|---|---|
| 线程数 | 1 | CPU 核数 |
| `tokio::spawn` 的 future | 需要 `Send` | 需要 `Send` |
| `block_on` 的根 future | 可为 `!Send` | 可为 `!Send` |
| 适用 | 单连接、嵌入式、测试 | 服务端 |
| 创建 | `Builder::new_current_thread()` | `Builder::new_multi_thread()` |

```rust,noplayground
fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        println!("单线程运行时");
    });
}
```

`#[tokio::main(flavor = "current_thread")]` 等价于上面这个配置，很适合写测试和单线程工具。

### 为什么不能跨 await 持有 std 的锁

`std::sync::MutexGuard` 不是 `Send`。持有它跨过 `.await`，编译器就会报 `future cannot be sent between threads safely`。这不是限制，而是保护：

```text
线程 A 拿到 std 锁 → await 挂起（锁还在手里）
线程 B 被调度去跑同一个 Future 的后续 → 尝试拿同一把锁
→ 死锁（B 在等 A，但 A 已经不在跑这个任务了）
```

因为任务可以在任意线程上继续执行，跨 await 持有的 `MutexGuard` 会绑在「旧线程」上，而续跑的可能是「新线程」。tokio 的 `Mutex` 用异步等待队列解决了这个问题：拿不到锁就 `.await` 让出线程，而不是阻塞。

经验法则：

- 锁的临界区很短、**不跨 await** → 用 `std::sync::Mutex`（更快）
- 需要跨 await 持有 → 用 `tokio::sync::Mutex`

## 25.7 典型错误案例

### 案例 1：跨 await 持有 std 的 MutexGuard

```rust,editable
use std::sync::Mutex;

#[tokio::main]
async fn main() {
    tokio::spawn(async {
        let m = Mutex::new(0);
        let guard = m.lock().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        println!("{}", *guard);
    }).await.unwrap();
}
```

```text
error: future cannot be sent between threads safely
   --> src/main.rs:5:5
    |
  5 | /     tokio::spawn(async {
  6 | |         let m = Mutex::new(0);
  7 | |         let guard = m.lock().unwrap();
  8 | |         tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  9 | |         println!("{}", *guard);
 10 | |     }).await.unwrap();
    | |______^ future created by async block is not `Send`
    |
    = help: within `{async block@src/main.rs:5:18: 5:23}`, the trait `Send` is not implemented for `std::sync::MutexGuard<'_, i32>`
note: future is not `Send` as this value is used across an await
   --> src/main.rs:8:66
    |
  7 |         let guard = m.lock().unwrap();
    |             ----- has type `std::sync::MutexGuard<'_, i32>` which is not `Send`
  8 |         tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    |                                                                  ^^^^^ await occurs here, with `guard` maybe used later
note: required by a bound in `tokio::spawn`
   --> <cargo registry>/tokio-1.54.1/src/task/spawn.rs:176:21
    |
174 |     pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
175 |     where
176 |         F: Future + Send + 'static,
    |                     ^^^^ required by this bound in `spawn`
```

**原因**：`tokio::spawn` 要求提交的 future 实现 `Send`。这段代码在 `.await` 之后还使用 `guard`，状态机必须把它保存下来；标准库为兼容要求在原线程解锁的平台，把 `MutexGuard` 定义为 `!Send`。若仅在 `#[tokio::main]` 的根 future 中运行同样的代码，就没有这条 `spawn` 约束。

**修复**：缩小临界区，让 Guard 在 await 之前被 drop；或改用 `tokio::sync::Mutex`：

```rust,noplayground
use std::sync::Mutex;

#[tokio::main]
async fn main() {
    let m = Mutex::new(0);
    {
        let guard = m.lock().unwrap();
        println!("读到 {}", *guard);
    } // guard 在这里 drop

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // 需要跨 await 时用 tokio 的锁
    let tm = tokio::sync::Mutex::new(0);
    let mut g = tm.lock().await;
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    *g += 1;
    println!("tokio 锁：{}", *g);
}
```

### 案例 2：把 main 写成 async

```rust,editable
async fn main() {
    println!("hello");
}
```

```text
error[E0752]: `main` function is not allowed to be `async`
 --> src/main.rs:1:1
  |
1 | async fn main() {
  | ^^^^^^^^^^^^^^^ `main` function is not allowed to be `async`
```

**原因**：`main` 由 C 运行时调用，它只会调用同步函数；`async fn main` 返回的是一个 Future，没人去 poll 它。语法层面语言直接禁止。

**修复**：加 `#[tokio::main]`，让宏生成同步 `main` 并在里面 `block_on`：

```rust,noplayground
#[tokio::main]
async fn main() {
    println!("hello");
}
```

没有 tokio 时也可以手写：`fn main() { tokio::runtime::Runtime::new().unwrap().block_on(async_main()) }`。

### 案例 3：spawn 捕获了借用而不是所有权

```rust,editable
#[tokio::main]
async fn main() {
    let s = String::from("hi");
    let slice = s.as_str();
    tokio::spawn(async move {
        println!("{}", slice);
    })
    .await
    .unwrap();
}
```

```text
error[E0597]: `s` does not live long enough
 --> src/main.rs:4:17
  |
3 |     let s = String::from("hi");
  |         - binding `s` declared here
4 |     let slice = s.as_str();
  |                 ^ borrowed value does not live long enough
5 |     tokio::spawn(async move {
  |     ------------ argument requires that `s` is borrowed for `'static`
...
10| }
  | - `s` dropped here while still borrowed
  |
note: requirement that the value outlives `'static` introduced here
  |         F: Future + Send + 'static,
```

**原因**：`spawn` 把任务交给运行时，运行时可能在本函数返回之后才执行它，所以要求 `F: 'static`。`slice` 借用了局部的 `s`，`s` 会在函数结束时销毁，这个借用活不到 `'static`。

**修复**：把所有权移进任务（`move` + 拥有者一起移走），或用 `Arc<str>` 共享：

```rust,noplayground
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let s = Arc::new(String::from("hi"));
    let s2 = Arc::clone(&s);
    tokio::spawn(async move {
        println!("{}", s2);
    })
    .await
    .unwrap();
    println!("主线程还在用：{}", s);
}
```

### 案例 4：忘了 .await，直接解引用 Future

```rust,editable
#[tokio::main]
async fn main() {
    let lock = tokio::sync::Mutex::new(0);
    let guard = lock.lock();
    println!("{}", *guard);
}
```

```text
error[E0614]: type `impl Future<Output = tokio::sync::MutexGuard<'_, {integer}>>` cannot be dereferenced
 --> src/main.rs:5:20
  |
5 |     println!("{}", *guard);
  |                    ^^^^^^ can't be dereferenced
```

**原因**：`tokio::sync::Mutex::lock` 是 `async fn`，调用它只是**创建**一个 Future，并没有真的拿锁。Future 不能解引用，必须 `.await` 才会拿到 `MutexGuard`。

**修复**：加 `.await`（并且通常还要处理 poisoned 错误）：

```rust,noplayground
#[tokio::main]
async fn main() {
    let lock = tokio::sync::Mutex::new(0);
    let guard = lock.lock().await;
    println!("{}", *guard);
}
```

这条规则适用于所有 `async fn`：**调用只是「造了一个任务描述」，`.await` 才开始执行**。忘了 `.await` 时，编译器通常报「Future 不能做什么」，而不是「你忘了 await」——要学会认出 `impl Future<...>` 这个类型。

### 案例 5（运行期）：在运行时里再启动一个运行时

```rust,noplayground
#[tokio::main]
async fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        println!("inner");
    });
}
```

```text
thread 'main' panicked at tokio-1.54.1/src/runtime/scheduler/multi_thread/mod.rs:89:9:
Cannot start a runtime from within a runtime. This happens because a function (like `block_on`)
attempted to block the current thread while the thread is being used to drive asynchronous tasks.
If you are in an async function, use `.await` on the future instead of blocking on it.
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

**原因**：`block_on` 的语义是「阻塞当前线程直到 Future 完成」。而当前线程正被外层运行时用来驱动任务，再阻塞它就等于让运行时自己堵住自己，所以 tokio 直接 panic。

**修复**：在 async 函数里就老实 `.await`；确实需要跑一个「必须用独立运行时」的库函数时，用 `spawn_blocking` 把它挪到阻塞线程池：

```rust,noplayground
#[tokio::main]
async fn main() {
    // 直接 await 即可
    async { println!("inner"); }.await;

    // 必须阻塞的场景：挪到阻塞池
    tokio::task::spawn_blocking(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { println!("独立运行时里的工作"); });
    })
    .await
    .unwrap();
}
```

## 25.8 动手练习

1. **并发抓取**：写 `async fn fetch(id: u32) -> String`（内部 `sleep` 若 id 毫秒），用 `join!` 并发执行 5 个，打印总耗时；再用同样的函数串行 await 一遍，对比两个耗时。
2. **超时包裹**：实现 `async fn with_timeout<F: Future>(f: F, ms: u64) -> Option<F::Output>`，超时返回 `None`。用它包裹一个永远不返回的 `std::future::pending()`，验证超时生效。
3. **限流**：用 `tokio::sync::Semaphore::new(3)` 限制同时进行的任务数为 3；启动 10 个任务，每个任务开始时打印「开始」、结束前 `sleep(50ms)`、结束时打印「结束」。观察输出中同时进行的任务不超过 3 个。
4. **watch 广播状态**：用一个 `watch::channel(0u32)` 让 3 个任务循环读取最新值并打印；主任务每 100 ms 用 `send` 更新一次值，共 5 次。解释为什么每个订阅者可能跳过某些中间值。
5. **修 bug**：下面这段代码有两个问题（一个编译错误、一个隐性性能问题），指出并修复。

```rust,noplayground
use std::sync::Mutex;

#[tokio::main]
async fn main() {
    let m = std::sync::Arc::new(Mutex::new(0));
    let mut handles = Vec::new();
    for _ in 0..3 {
        let m = m.clone();
        handles.push(tokio::spawn(async move {
            let mut g = m.lock().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            *g += 1;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    println!("{}", m.lock().unwrap());
}
```

## 25.9 小结

- `async` 是语言能力，**运行时**负责驱动 Future；tokio 是事实标准，`#[tokio::main]` 只是 `Builder::new_multi_thread().enable_all().build().block_on(...)` 的语法糖。
- `.await` 是串行等待，`join!`/`spawn` 才是并发；`async` 本身不会让代码变快。
- `tokio::spawn` 要求 `Future + Send + 'static`，所以任务不能借用可能提前失效的局部变量；也可持有 `&'static T`（`move` + `Arc`/`clone`）。
- `timeout` 限制等待时间；后台任务的停止需要单独处理；`interval` 提供周期 tick。
- 长时间同步阻塞会占住当前 worker，反复或全面阻塞可能使其他任务饥饿，用 `spawn_blocking` 挪到阻塞线程池。
- 运行时用 work-stealing：每 worker 一个本地队列 + 全局队列 + 互相偷任务；`await` 挂起时立刻转去跑别的任务。
- 在 `current_thread` 上，`block_on` 可以驱动 `!Send` 的根 future，但 `tokio::spawn` 仍要求 `Send`。用 `LocalSet` 配合 `spawn_local` 可以执行 `!Send` 子任务。
- `std::sync::MutexGuard` 是 `!Send`，不能跨 `.await`；需要跨 await 就用 `tokio::sync::Mutex`，临界区短就直接缩到 await 之前。
- 调用 `async fn` 只是创建 Future，忘了 `.await` 时编译器会报「`impl Future` 不能解引用/不能显示」之类的怪错——看到 `impl Future` 先想 `.await`。
- 在运行时里再 `block_on` 会 panic（`Cannot start a runtime from within a runtime`）；需要嵌套就 `spawn_blocking`。