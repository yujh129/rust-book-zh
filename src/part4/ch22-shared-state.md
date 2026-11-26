# 第 22 章 共享状态：Mutex、RwLock 与原子类型

> **本章目标**
>
> - 用 `Mutex<T>` 把数据和保护它的锁绑成一个值，说清 `lock()` 返回的守卫为什么能保证解锁
> - 理解 RAII 释放与锁中毒（poisoning），知道 `PoisonError` 该怎么处理而不是无脑 `unwrap`
> - 用 `Arc<Mutex<T>>` 写出多线程共享状态，并按临界区长度决定锁的粒度
> - 分清 `RwLock` 与 `Mutex` 的适用边界，不被"读写分离一定更快"误导
> - 用 `AtomicUsize`/`AtomicBool` 写无锁计数器，解释 `Relaxed`/`Acquire`/`Release`/`SeqCst` 的实际差别
> - 认出死锁的四种成因，知道标准库的锁在 Linux 上大致怎么实现

第 21 章把数据"分"给线程：`move`、`thread::scope`、`split_at_mut`，每个线程拿一块互不重叠的数据。这类并行很好写，因为不需要同步。

但有一类问题天生拆不开：四个线程要更新同一个计数器，十个请求要读写同一张缓存表，整个服务要共用同一个连接池。"共享"和"可变"凑在一起，就是数据竞争的温床。

其他语言的做法是给一段代码加锁：`synchronized`、`lock` 语句、`pthread_mutex_lock`。问题是锁和数据是分离的，编译器不知道哪块内存受哪把锁保护。忘记加锁照样能编译，然后在生产环境里偶发错误、偶发崩溃，复现不了。

Rust 换了个方向：把锁和数据放进同一个类型。`Mutex<T>` 里装着 `T`，不先 `lock()` 就拿不到 `&mut T`。于是"忘记加锁"从运行期 bug 变成编译错误，"锁释放了吗"从人工检查变成 `Drop` 的自动行为。

## 22.1 `Arc` 只解决"活得够久"，不解决"同时改"

第 21 章用过 `Arc` 把数据送进线程。`Arc::clone` 只把原子计数加一，两个线程看到的是同一块堆内存，子线程结束后计数回到 1——但 `Arc<T>` 给出的始终是 `&T`。

`Arc<T>` 通过 `Deref` 给你 `&T`，只读共享没问题。想改就不行了：`Arc::get_mut` 要求引用计数恰好为 1，多线程持有的时候永远返回 `None`。这不是实现偷懒，而是 `Arc` 的原子计数只负责"最后一个人释放内存"，它不提供互斥——两个线程同时拿到 `&mut T` 就是数据竞争。

所以"共享可变"需要另外两样东西：运行期的互斥，和编译期"必须先拿锁"的约束。`Mutex<T>` 一次给全。

## 22.2 `Mutex<T>`：锁和数据是同一个值

```rust
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(vec![1, 2, 3]);

    {
        let mut g = m.lock().unwrap();
        g.push(4);
        println!("临界区内：{:?}", *g);
    }   // g 在这里离开作用域，Drop 里解锁

    println!("临界区外：{:?}", m.lock().unwrap());
}
```

三件事值得停下来看。第一，`Mutex::new` 把数据**装进**锁里，没有独立的锁变量。你没法"绕过锁访问数据"，因为数据在锁里面。

第二，`lock` 的签名是 `fn lock(&self) -> LockResult<MutexGuard<'_, T>>`，接收 `&self` 而不是 `&mut self`。锁的内部状态由原子操作和系统调用修改，不需要外层可变。这正是 `Mutex` 能放在 `Arc` 后面被多个线程共享的前提。

第三，`LockResult<T>` 是 `Result<T, PoisonError<T>>` 的类型别名。绝大多数示例直接 `unwrap()`，下一节会讲这个 `unwrap` 在什么情况下会炸。

### 守卫：RAII 与 Deref

`MutexGuard<'a, T>` 是本章的主角，它同时做三件事：

1. **证明持有权**。生命周期 `'a` 绑在 `Mutex` 的借用上。守卫活着，`Mutex` 就被借走了，同一作用域里第二次 `lock()` 不会给你第二个守卫，而是阻塞等待——运行期自锁，编译期看不出来。
2. **提供访问**。它实现 `Deref<Target = T>` 和 `DerefMut`，所以 `g.push(4)`、`*g += 1`、`g.len()` 都能直接写，像在用 `&mut Vec<i32>`。
3. **自动释放**。`Drop` 里解锁，不需要 `unlock()`，标准库也没有提供这个方法。

第 2 条容易让人误以为守卫就是 `&mut T`。区别在于：守卫的创建和销毁有运行期副作用（抢锁、放锁），而 `&mut T` 是纯编译期概念。也因此，`let g = m.lock().unwrap(); *g += 1;` 会报 E0596——`DerefMut` 需要 `&mut g`，守卫本身必须是 `mut` 绑定。锁保护的是数据，可变性仍然服从借用规则，两套规则各管各的。

RAII 的价值在错误路径上。函数提前 `return`、`?` 传播错误、panic 展开栈，`Drop` 都会被调用，锁一定会释放。C 里手写 `pthread_mutex_unlock`，漏掉一条 `return` 路径就是死锁；这类 bug 在 Rust 里写不出来。

### 为什么不是 `T` 加一把锁

C++ 的 `std::mutex` 和它保护的数据是两个独立变量，靠命名约定关联：

```text
std::mutex mu;
std::vector<int> data;   // 谁都可以不碰 mu，直接改 data
```

`std::lock_guard` 只保证"这个作用域里拿了锁"，不保证"改 data 之前一定拿了锁"。两者对不上，编译器不会提醒。

Rust 的 `Mutex<T>` 把二者绑成一个类型，`lock()` 是拿到 `&mut T` 的唯一安全途径。另外两个出口都要求独占：`get_mut(&mut self)` 和 `into_inner(self)`；既然编译器已证明没有别人能碰它，这两条路不必加锁，适合"初始化阶段单线程填好数据、之后才共享"的场景。

代价是 `Mutex<T>` 只能整体加锁。想给不同字段配不同的锁，就得把字段拆成多个 `Mutex`，或者拆成多个结构体。这是 22.4 节粒度讨论的起点。

## 22.3 中毒：持锁 panic 之后

设想线程 A 拿着锁改一个 `Vec`，改到一半 `panic!` 了。栈展开时 `Drop` 会解锁，但数据结构可能停在"改了一半"的状态：长度字段更新了，元素还没写进去。线程 B 接着拿锁继续用，读到的就是逻辑上不一致的数据。

标准库不替你做决定，但把这件事记下来。守卫在 `Drop` 时检查当前线程是否正在 panic（`thread::panicking()`），是则把 `Mutex` 内部的 poison 标志置位。之后每次 `lock()` 都返回 `Err(PoisonError<MutexGuard<T>>)`。

```rust
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let m = Arc::new(Mutex::new(0));
    let m2 = Arc::clone(&m);

    let _ = thread::spawn(move || {
        let mut g = m2.lock().unwrap();
        *g += 1;
        panic!("worker 崩了");
    })
    .join();

    let g = m.lock().unwrap_or_else(|e| e.into_inner());
    println!("值仍然是 {}", *g);
}
```

```console
$ cargo run
thread '<unnamed>' panicked at src/main.rs:11:9:
worker 崩了
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
值仍然是 1
```

panic 信息走 stderr，最后一行是 stdout，所以真实终端里的顺序可能交错。

三条原则：

- poison 是一个**标志**，不是"锁坏了"。锁本身仍然可用，`PoisonError::into_inner()` 把守卫还给你，`Mutex::clear_poison()` 可以清除标志。
- `unwrap()` 的语义是"我不接受中毒"，于是 panic 扩散到当前线程。在服务端，这表现为"一个请求 panic 之后，所有后续请求都 panic"——不是锁坏了，是每一处 `unwrap()` 都在拒绝。
- 中毒只检测 **panic**，不检测逻辑错误；数据被写坏但没 panic，poison 不会报。

怎么选：不变量重要的数据（账户余额、索引结构）用 `unwrap()` 让问题尽早炸出来；可重建的缓存、统计量用 `unwrap_or_else(|e| e.into_inner())` 继续，或者丢掉重建；确信 panic 没破坏不变量才 `clear_poison()`，并在注释里写清为什么。全局替换成 `into_inner` 图省事，等于关掉中毒检测，把一个"可能不一致"的信号吞掉。

## 22.4 `Arc<Mutex<T>>`：经典组合

`Mutex<T>` 不能跨线程共享所有权，`Arc` 负责"多个线程各持一份所有权"，`Mutex` 负责"同一时刻只有一个线程能改"。两个叠起来，就是 Rust 里共享可变状态的默认答案。

```rust
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let total = Arc::new(Mutex::new(0usize));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let t = Arc::clone(&total);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                *t.lock().unwrap() += 1;
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    println!("{}", *total.lock().unwrap());
}
```

输出 `4000`。`Arc::clone` 只加计数，不复制 `Mutex` 里的数据；`move` 把每个 `Arc` 副本移进闭包。四个线程交替拿锁，每次只有一个在临界区里。

注意 `*t.lock().unwrap() += 1` 这一行：守卫是临时值，语句结束就释放，临界区只有一条指令那么长。如果写成 `let mut g = t.lock().unwrap(); *g += 1;` 再让 `g` 活到函数末尾，并发度就完全不同——别的线程全程在门外等。这是"锁的粒度"最微观的一层：**临界区只包住必须原子完成的那几行**。打印、格式化、IO、复杂计算都应该挪到锁外，必要时先在锁内把数据复制出来。

### 计数方案的实测

同一个任务：4 个线程各加 100 万次，比较 `Mutex<usize>` 与 `AtomicUsize`；另外单独测了每线程本地累加，作为"完全不共享"的下限。

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

const THREADS: usize = 4;
const N: usize = 1_000_000;

fn main() {
    let start = Instant::now();
    let m = Arc::new(Mutex::new(0usize));
    let mut hs = Vec::new();
    for _ in 0..THREADS {
        let m = Arc::clone(&m);
        hs.push(thread::spawn(move || {
            for _ in 0..N {
                *m.lock().unwrap() += 1;
            }
        }));
    }
    for h in hs { h.join().unwrap(); }
    println!("Mutex    {:>7.1} ms", start.elapsed().as_secs_f64() * 1000.0);

    let start = Instant::now();
    let a = Arc::new(AtomicUsize::new(0));
    let mut hs = Vec::new();
    for _ in 0..THREADS {
        let a = Arc::clone(&a);
        hs.push(thread::spawn(move || {
            for _ in 0..N {
                a.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for h in hs { h.join().unwrap(); }
    println!("Atomic   {:>7.1} ms", start.elapsed().as_secs_f64() * 1000.0);
}
```

```console
$ cargo run --release
Mutex      517.1 ms
Atomic      62.9 ms
本地计数（另一段程序） 0.5 ms
```

数字因机器而异，但比例有普遍性。`Mutex` 每次 lock/unlock 要一次 CAS，争用时还进内核睡眠；`fetch_add` 只有一条带 lock 前缀的加法。原子快约 8 倍，但两者都在抢同一条缓存行，线程越多越慢。

第三行要打个折扣：编译器看到 `local += 1` 循环 N 次且中间没有副作用，会直接算成 `local = N`，0.5 ms 里几乎没有循环。真实业务里本地累加不会这么容易被优化掉，但它确实**没有共享写**，这才是它快的根本原因。

选择判据：

- 只是计数、开关、状态标志 → 原子类型。
- 要维护一段多步不变量（两个字段必须一起改、`Vec` 的 push 与索引）→ `Mutex`。
- 能改成"各算各的，最后合并" → 先改结构，别共享。

### 粒度：一把大锁还是多把小锁

粗粒度是"整个状态一把锁"：写起来简单，正确性容易保证，但所有线程串行化。细粒度是"每个分片一把锁"：并发度高，但要处理多把锁的顺序、跨分片的一致性。

中间方案是**分段锁**（sharding）：按 key 的哈希把数据分到 N 个桶，每桶一把 `Mutex`。理想情况下竞争被分散到 N 条缓存行，吞吐接近线性；代价是遍历全表要依次锁 N 个桶，且"跨桶的原子操作"不再原子。

还有一个容易被忽略的坑：**伪共享**（false sharing）。两个不相干的 `AtomicUsize` 挨在一起，就落在同一条 64 字节缓存行上；两个线程各改一个，缓存行仍在两核之间来回弹，性能跟抢同一把锁差不多。真要隔离，用 `#[repr(align(64))]` 把热字段各自撑满一条缓存行。

## 22.5 `RwLock`：读多写少时再考虑

`Mutex` 的问题是独占：哪怕十个线程只是想读，也得排队。`RwLock<T>` 把访问分成两种：多个读者可以同时进入，写者必须独占。

```rust
use std::sync::RwLock;
use std::thread;

fn main() {
    let lock = RwLock::new(vec![1, 2, 3]);

    thread::scope(|s| {
        for _ in 0..3 {
            s.spawn(|| {
                let g = lock.read().unwrap();
                println!("读：{:?}", *g);
            });
        }
        s.spawn(|| {
            let mut g = lock.write().unwrap();
            g.push(4);
        });
    });

    println!("最终：{:?}", *lock.read().unwrap());
}
```

接口和 `Mutex` 对称：`read() -> LockResult<RwLockReadGuard<T>>`，`write() -> LockResult<RwLockWriteGuard<T>>`，两种守卫都实现 `Deref`，写守卫还实现 `DerefMut`，drop 时释放，panic 时同样会中毒。

三个边界必须知道：

**第一，读锁不比互斥锁便宜。** 每次 `read()` 仍要修改一个共享的原子计数，所有读者都在碰同一条缓存行。临界区只有"取一个字段"那么短时，`RwLock` 往往比 `Mutex` 慢；只有当临界区较长（比如遍历一个大表、做一次序列化）且读远多于写时，读的并行才划得来。

**第二，标准库不承诺公平。** 读者源源不断时，写者可能等很久；反过来，实现如果偏向写者，读者也可能被压住。具体策略随平台和版本变化，不能当成契约。要可预期的调度，用 `parking_lot` 或自己加队列。

**第三，读锁不能升级成写锁。** 没有 `upgrade()`。你必须先 `drop` 读守卫再 `write()`，而这两步之间别人可能已经改过数据，所以"读到就改"的逻辑要重新检查一遍。同一线程持有读锁再去 `write()` 会直接死锁。

选择顺序：先问能不能用原子或消息传递解决，不能再用 `Mutex`，确认读多写少且临界区不短才换 `RwLock`。

## 22.6 原子类型与 `Ordering`

原子类型是标准库给出的无锁基石：`AtomicBool`、`AtomicUsize`、`AtomicIsize`、`AtomicI32`/`AtomicU32`/`AtomicI64`/`AtomicU64`、`AtomicPtr<T>`。它们都不实现 `Clone`，只能通过 `&` 共享，或者放进 `Arc`。

核心 API 是一组"读—改—写"：

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

fn main() {
    let n = AtomicUsize::new(0);

    n.store(10, Ordering::Relaxed);
    n.fetch_add(5, Ordering::Relaxed);   // 返回旧值 10，现在是 15
    n.fetch_sub(3, Ordering::Relaxed);   // 返回 15，现在是 12

    let old = n.swap(100, Ordering::Relaxed);        // 返回 12
    println!("{} {} {}", n.load(Ordering::Relaxed), old, n.fetch_add(1, Ordering::Relaxed));
}
```

`fetch_add` 返回**旧值**，这一点和 `i += 1` 的直觉不同。要做"只在满足条件时更新"，用 `compare_exchange`：它在一条指令里完成"比较 + 交换"，失败时把当前值写回给你，让你重试。

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

fn update_max(max: &AtomicUsize, v: usize) {
    let mut cur = max.load(Ordering::Relaxed);
    while v > cur {
        match max.compare_exchange_weak(cur, v, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => cur = actual,
        }
    }
}

fn main() {
    let max = AtomicUsize::new(0);
    for v in [3, 9, 2, 7] { update_max(&max, v); }
    println!("最大值：{}", max.load(Ordering::Relaxed));
}
```

这是 CAS 循环（compare-and-swap loop）的标准形状：读、算、试、失败重读。`compare_exchange_weak` 允许"伪失败"（值没变也返回 `Err`），在循环里用它比 `compare_exchange` 更高效，因为它能映射到 LL/SC 架构（ARM、RISC-V）的单条指令。

### `Ordering` 到底在约束什么

每个原子操作都带一个 `Ordering`，它约束的不是"这个操作原子不原子"（原子性由类型保证），而是**这个操作前后的普通读写能不能被重排到它另一侧**。重排有两个来源：编译器优化和 CPU 乱序执行，两者都要防。

常用级别：

- `Relaxed`：只保证这个原子操作本身不可分割，不建立任何跨线程的先后关系。计数、统计、`AtomicBool` 的停止标志用它就够。
- `Acquire`（load）/ `Release`（store）是一对，分别禁止"之后的读写挪到前面"与"之前的读写挪到后面"：前者像进门先看门牌，后者像走之前把东西放好。
- `AcqRel`：读—改—写操作专用，load 部分按 Acquire，store 部分按 Release。
- `SeqCst`：在 Acquire/Release 之上，再要求所有 `SeqCst` 操作有一个全局一致的顺序。最贵，也最不容易写错。

光看定义不够，看一个真实的数据传递：

```rust
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

fn main() {
    let data = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(AtomicBool::new(false));

    let (d, r) = (Arc::clone(&data), Arc::clone(&ready));
    let writer = thread::spawn(move || {
        d.store(42, Ordering::Relaxed);      // 先写数据
        r.store(true, Ordering::Release);    // 再竖旗子
    });

    let (d, r) = (Arc::clone(&data), Arc::clone(&ready));
    let reader = thread::spawn(move || {
        while !r.load(Ordering::Acquire) {   // 看到旗子
            std::hint::spin_loop();
        }
        d.load(Ordering::Relaxed)            // 数据一定已经可见
    });

    writer.join().unwrap();
    println!("读到 {}", reader.join().unwrap());
}
```

`Release` 的 store 保证"42 的写入"不会被挪到"旗子置 true"之后；`Acquire` 的 load 保证"读 data"不会被挪到"检查旗子"之前。当读方的 Acquire 加载读到这次 Release 存储写入的 `true` 时，发布前对 `data` 的写入才与读方同步。这就是"release store 之前的一切，对 acquire load 之后的一切可见"。

如果把这两处都改成 `Relaxed`，程序依然能编译、依然原子，但读者可能先看到旗子为 true、再读到 0。x86 的硬件模型很强（load 自带 acquire、store 自带 release），这类 bug 在 x86 上常常跑一万次都不出现；换到 ARM 服务器上就偶发。测试可能暴露错误；测试通过仍需用同步关系解释为什么所有允许的执行都正确。

### 代价差在哪

在 x86-64 上：

- `Relaxed` 的 load/store 编译成普通 `mov`，和访问普通变量一样快。
- `Acquire` 的 load 也是 `mov`，因为 x86 的 load 天然有序。
- `Release` 的 store 还是 `mov`。
- `SeqCst` 的 store 需要 `xchg`（隐含 lock 前缀），代价取决于目标和缓存竞争。

所以"全都写 `SeqCst`"在很多程序里不会有明显性能问题，但会掩盖你对同步关系的理解。反之，"全用 `Relaxed` 图快"是在赌硬件和编译器不作妖。实用规则：

- 纯计数、统计、幂等的标志位 → `Relaxed`
- 用标志位传递"另一块数据已经准备好" → `Release` / `Acquire` 配对
- CAS 循环 → 成功用 `AcqRel`，失败用 `Relaxed`（失败时没有发布任何东西）
- 想不清楚、且不在热路径 → `SeqCst`

需要把同步点放在不是原子操作的位置时，用 `std::sync::atomic::fence(Acquire)` / `fence(Release)`。

## 22.7 深入：futex、自旋与标准库的实现

一把没人争的锁必须足够便宜。如果每次 `lock()` 都进一次内核，代价在微秒级，而临界区可能只有几条指令——同步开销会淹没业务逻辑。Linux 的 futex（fast userspace mutex）就是为此设计的：**无争用时完全在用户态完成，只有真需要睡觉才进内核**。

futex 的核心是一个 32 位整数（futex word），放在所有线程都能看到的共享内存里。它有三个状态：

```text
0   未锁
1   已锁，没有等待者
2   已锁，有等待者
```

加锁解锁的骨架：

```text
lock():
    if CAS(word, 0 -> 1) 成功: return      // 快路径：一条原子指令，无系统调用
    loop:
        if swap(word, 2) == 0: return      // 抢到了，顺手记下"有人在等"
        futex_wait(&word, 2)               // 睡，条件是 word 仍等于 2
unlock():
    if swap(word, 0) == 2:                 // 原来有人在等
        futex_wake(&word, 1)               // 只唤醒一个
```

两个细节决定了它的效率。**状态 1 和 2 的区分，是为了让解锁路径也不必进内核。** 解锁时 `swap` 返回 1，说明没有等待者，直接返回；只有返回 2 才调 `futex_wake`。绝大多数锁操作都是无争用的，因此绝大多数 unlock 只有一条原子指令。

**`futex_wait(&word, 2)` 的第二个参数是"期望值"。** 内核会先检查 `word` 是否仍等于 2，是才把线程挂起，否则立即返回。这个检查和挂起在内核里是原子的，堵住了"刚判断要睡、锁就被释放"的丢唤醒窗口。醒来后线程重新回到循环抢锁，而不是直接获得锁——这也是为什么 futex 只是"睡眠原语"，不是"锁"。

争用时还有一段**自旋**。线程从"发现抢不到"到"真正睡着"要走系统调用、调度器、上下文切换，成本在微秒级；如果锁只被持有几百纳秒，睡下去再被叫醒反而更亏。所以实现会在 `futex_wait` 之前用 `std::hint::spin_loop()`（x86 上是 `pause` 指令）空转若干次。标准库在 Linux 上的实现会先自旋约一百次，`parking_lot` 则用更精细的自适应策略：短时间内反复争用的锁，自旋次数会动态调整。

自旋不是免费的：空转的线程占着 CPU，如果线程数超过核心数，被自旋的锁持有者可能根本排不上 CPU，自旋纯属浪费。这就是"自旋锁只适合临界区极短、且线程数不超过核心数"的原因。

平台差异：Linux 上 `std::sync::Mutex` 自 1.62 起直接基于 futex，不再包一层 `pthread_mutex_t`；其他平台使用的等待原语取决于目标和标准库版本。所以"Rust 的锁有多快"没有统一答案，但快路径都是一条原子指令。

顺带说 `parking_lot`：不实现 poisoning（很多人认为 poison 弊大于利），守卫更小，通常比 std 快，还提供 `try_lock_for` 这类超时接口。代价是多一个依赖，标准库够用时不必换。

## 22.8 死锁：四种成因与避免

死锁的定义很朴素：每个线程都在等一个只有别人才能给的东西。Rust 的借用检查器管不了这个，因为锁的获取是运行期行为。

**成因一：同一线程重复加锁。** 标准库的 `Mutex` 不可重入，`let g = m.lock().unwrap(); let g2 = m.lock().unwrap();` 这两行编译通过，运行挂死。守卫 `g` 要等到作用域结束才释放，而 `g2` 在等 `g` 释放。修复方式不是换可重入锁，而是重新组织代码：把两次加锁合并成一次，或者先 `drop(g)` 再拿。可重入锁（`parking_lot::ReentrantMutex`）能把死锁变成不死锁，但通常掩盖了"临界区设计不清"的问题。

**成因二：多把锁，加锁顺序不一致。**

```rust,noplayground
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() {
    let a = Arc::new(Mutex::new(1));
    let b = Arc::new(Mutex::new(2));

    let (a1, b1) = (Arc::clone(&a), Arc::clone(&b));
    let t1 = thread::spawn(move || {
        let _x = a1.lock().unwrap();
        thread::sleep(Duration::from_millis(50));
        let _y = b1.lock().unwrap();   // 等 b
    });

    let (a2, b2) = (Arc::clone(&a), Arc::clone(&b));
    let t2 = thread::spawn(move || {
        let _x = b2.lock().unwrap();
        thread::sleep(Duration::from_millis(50));
        let _y = a2.lock().unwrap();   // 等 a
    });

    t1.join().unwrap();
    t2.join().unwrap();
}
```

线程 1 拿 a 等 b，线程 2 拿 b 等 a。经典命名是 ABBA 死锁。它不一定每次复现——只有两个线程恰好都拿到第一把锁时才发生。修复：**给所有锁规定一个全局顺序**（比如按地址、按 ID、按业务层级），任何线程都按同一顺序获取。实在需要同时持有时，把两把锁合并成一把，用数据结构的边界换掉锁的边界。

**成因三：持锁调用外部代码。** 你在锁里调了一个回调、一个 `Display` 实现、一个 trait 方法，而它的实现可能回头拿同一把锁。调用方无法约束被调用方，这类死锁最难查。规则是：**临界区里只碰数据，不调用未知代码**。需要回调时，先在锁内把数据复制出来，释放锁后再调。

**成因四：守卫活得比预期长。** 在循环外拿锁、在循环里处理，或者在 `async` 函数里跨 `.await` 持有 `std::sync::MutexGuard`。前者让其他线程长时间等待（严格说是饥饿而非死锁），后者在单线程执行器上会直接自锁，而且 `MutexGuard` 不是 `Send`，多线程运行时根本编译不过——第 25 章会展开。

还有一类跨组件的互等：线程 A 持锁等通道消息，线程 B 发完消息等这把锁。锁和任何"会阻塞的资源"（通道、`join`、IO）都不该嵌套持有。

工程上的防御手段，按性价比排序：

1. 缩小临界区，尽量只持有一把锁。
2. 必须多把时，定义全局顺序并写进注释或类型里。
3. 用 `try_lock()` 代替 `lock()` 做"探测"，拿不到就走备用路径，而不是傻等。
4. 需要等待上限或死锁检测时用 `parking_lot` 的 `try_lock_for` 与 `deadlock_detection` 特性。

`try_lock` 的返回类型是 `Result<Guard, TryLockError>`，错误分"被占用"和"已中毒"两种：

```rust
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    let g = m.lock().unwrap();

    match m.try_lock() {
        Ok(_) => println!("拿到了"),
        Err(std::sync::TryLockError::WouldBlock) => println!("被占用，不等了"),
        Err(std::sync::TryLockError::Poisoned(_)) => println!("锁已中毒"),
    }

    drop(g);
}
```

注意这里 `try_lock` 和 `lock` 在同一个线程——如果换成 `lock()`，就会变成成因一。`try_lock` 在这里既探测了占用，也顺带证明了不可重入。

## 典型错误案例

锁相关的错误分两层：借用检查器能拦下的（守卫的所有权、可变性、跨线程），和它拦不下的（死锁、中毒）。五个案例覆盖这两层。

### 案例 1：守卫忘了 mut——E0596

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    let g = m.lock().unwrap();
    *g += 1;
    println!("{}", *g);
}
```

```text
error[E0596]: cannot borrow `g` as mutable, as it is not declared as mutable
 --> src/main.rs:6:6
  |
6 |     *g += 1;
  |      ^ cannot borrow as mutable
  |
help: consider changing this to be mutable
  |
5 |     let mut g = m.lock().unwrap();
  |         +++
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`*g += 1` 调用的是 `DerefMut::deref_mut`，它要求 `g` 本身可以被可变借用。`g` 是一个变量，变量要可变借用，绑定就得是 `mut`。这里的 `mut` 修饰的是**守卫这个绑定**，不是 `Mutex` 里的数据——数据在锁后面，由 `Mutex` 保证独占，跟绑定是否 `mut` 无关。

**修复**：加 `mut`。

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    let mut g = m.lock().unwrap();
    *g += 1;
    println!("{}", *g);   // 1
}
```

一个容易混淆的点：`let m = Mutex::new(0);` 不需要 `mut`。`lock` 接收 `&self`，加锁本身就是"内部可变性"，不需要外层可变。

</details>

### 案例 2：守卫不能跨线程——E0277

```rust,editable
use std::sync::Mutex;
use std::thread;

fn main() {
    let m = Mutex::new(0);
    let g = m.lock().unwrap();
    thread::spawn(move || {
        println!("{}", *g);
    })
    .join()
    .unwrap();
}
```

```text
error[E0277]: `std::sync::MutexGuard<'_, i32>` cannot be sent between threads safely
   --> src/main.rs:7:19
    |
  7 |       thread::spawn(move || {
    |       ------------- ^------
    |       |             |
    |  _____|_____________within this `{closure@src/main.rs:7:19: 7:26}`
    | |     |
    | |     required by a bound introduced by this call
  8 | |         println!("{}", *g);
  9 | |     }).join().unwrap();
    | |_____^ `std::sync::MutexGuard<'_, i32>` cannot be sent between threads safely
    |
    = help: within `{closure@src/main.rs:7:19: 7:26}`, the trait `Send` is not implemented for `std::sync::MutexGuard<'_, i32>`
note: required because `g` is used within this closure
   --> src/main.rs:8:25
    |
  8 |         println!("{}", *g);
    |                         ^
...
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`MutexGuard` 从 1.71 起被明确标记为 `!Send`（可以验证：`fn assert_send<T: Send>() {}` 加 `assert_send::<std::sync::MutexGuard<'static, i32>>()` 会报同样的错）。这背后是平台事实：有些系统的解锁必须由加锁的线程完成（pthread 的默认互斥量就是如此），守卫被移到另一个线程再析构，解锁线程就错了。标准库选择用类型系统把这个限制变成编译错误，而不是留给运行期。

顺带一提，守卫是 `!Send` 但 `Mutex<T>` 在 `T: Send` 时是 `Send + Sync`——锁本身可以跨线程共享，锁的"持有权"不行。

**修复**：别把守卫送出去，把数据取出来再送，或者让每个线程自己加锁。

```rust,editable
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let m = Arc::new(Mutex::new(0));

    // 方案一：先读出来，只把值送进线程
    let value = *m.lock().unwrap();
    let t = thread::spawn(move || println!("值：{}", value));
    t.join().unwrap();

    // 方案二：共享 Arc，让线程自己加锁
    let m2 = Arc::clone(&m);
    let t = thread::spawn(move || {
        *m2.lock().unwrap() += 1;
    });
    t.join().unwrap();

    println!("{}", *m.lock().unwrap());   // 1
}
```

方案二就是 22.4 的经典模式。注意 `*m2.lock().unwrap() += 1;` 这一行：守卫是临时值，语句结束就释放，不需要显式 `drop`。

</details>

### 案例 3：守卫被 move 走——E0382

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    let g = m.lock().unwrap();
    let h = g;
    println!("{}", *g);
}
```

```text
error[E0382]: borrow of moved value: `g`
   --> src/main.rs:7:21
    |
  5 |     let g = m.lock().unwrap();
    |         - move occurs because `g` has type `std::sync::MutexGuard<'_, i32>`, which does not implement the `Copy` trait
  6 |     let h = g;
    |             - value moved here
  7 |     println!("{}", *g);
    |                     ^ value borrowed here after move
    |
    = note: borrow occurs due to deref coercion to `i32`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`MutexGuard` 不是 `Copy`，赋值 `let h = g` 把守卫的所有权移走了。`println!("{}", *g)` 通过 `Deref` 借用 `g`，此时 `g` 已失效。编译器还贴心地指出，这次借用是"deref coercion"——因为 `println!` 需要 `&`，而 `*g` 先解引用再取引用。

守卫为什么不能 `Copy`？因为它代表"锁被持有"这个独占状态，复制一份就等于两个守卫都认为自己负责解锁。

**修复**：要么用引用（`let h = &g;`），要么把守卫还给 `m`（`drop(g)`），要么干脆别复制。

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(0);
    let g = m.lock().unwrap();
    let h = &g;                 // 借用，不移动
    println!("{} {}", *g, *h);  // 0 0
}
```

如果确实想让两个作用域"先后"持有同一把锁，就在中间 `drop(g)`，别复制守卫。

</details>

### 案例 4：借用冲突——E0502

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(vec![1, 2, 3]);
    let mut g = m.lock().unwrap();
    let first = &g[0];
    g.push(4);
    println!("{} {:?}", first, *g);
}
```

```text
error[E0502]: cannot borrow `g` as mutable because it is also borrowed as immutable
 --> src/main.rs:7:5
  |
6 |     let first = &g[0];
  |                  - immutable borrow occurs here
7 |     g.push(4);
  |     ^ mutable borrow occurs here
8 |     println!("{} {:?}", first, *g);
  |                         ----- immutable borrow later used here
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`first` 是 `&g[0]`，借的是 `Vec` 里的元素；`g.push(4)` 可能重新分配缓冲区，让 `first` 悬空，所以借用检查器禁止在不可变借用存活期间可变借用。这跟锁无关——同样的代码在 `Vec` 上直接写也会报 E0502。锁只是把"可变借用"变成了"通过守卫的可变借用"，规则一个字没改。

**修复**：把需要的值复制出来，结束借用，再改容器。

```rust,editable
use std::sync::Mutex;

fn main() {
    let m = Mutex::new(vec![1, 2, 3]);
    let mut g = m.lock().unwrap();
    let first = g[0];
    g.push(4);
    println!("{} {:?}", first, *g);   // 1 [1, 2, 3, 4]
}
```

元素是 `i32` 时 `g[0]` 直接复制；元素是 `String` 时用 `g[0].clone()`，或者先算完再改。要"边读边改"，就得设计成两趟：第一趟只读收集结果，第二趟统一写。

</details>

### 案例 5：中毒后的第二次 panic——运行期 PoisonError

```rust,editable
use std::sync::{Arc, Mutex};
use std::thread;

struct Cache {
    data: Mutex<Vec<i32>>,
}

impl Cache {
    fn add(&self, v: i32) {
        self.data.lock().unwrap().push(v);
    }
}

fn main() {
    let c = Arc::new(Cache { data: Mutex::new(Vec::new()) });
    let c2 = Arc::clone(&c);

    let _ = thread::spawn(move || {
        let mut g = c2.data.lock().unwrap();
        g.push(1);
        panic!("写日志时崩了");
    })
    .join();

    c.add(2);
}
```

```text
thread '<unnamed>' panicked at src/main.rs:21:9:
写日志时崩了
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'main' panicked at src/main.rs:10:26:
called `Result::unwrap()` on an `Err` value: PoisonError { .. }
```

进程以 101 退出。注意这是**两个** panic：第一个是业务代码主动 panic，第二个是主线程 `unwrap` 了中毒的 `lock()`。

<details class="case-answer">
<summary>查看答案</summary>

**原因**：工作线程在持有守卫时 panic，栈展开会析构守卫；`Mutex` 在解锁时发现"这次解锁发生在 panic 展开过程中"，就把自己标记为 poisoned。之后任何 `lock()` 都返回 `Err(PoisonError)`，`unwrap` 把它变成第二个 panic。

poison 的用意：`Mutex` 保护的经常是"几个字段必须一起改"的不变量，持锁 panic 意味着改到一半，数据可能处于半成品状态；默认让后来者看到错误，比让它默默读到坏数据安全。

**修复**：明确决定"这份数据坏了要不要紧"。缓存、日志这类可重建的数据，取回守卫继续用：

```rust,editable
use std::sync::{Arc, Mutex};
use std::thread;

struct Cache {
    data: Mutex<Vec<i32>>,
}

impl Cache {
    fn add(&self, v: i32) {
        let mut g = self.data.lock().unwrap_or_else(|e| e.into_inner());
        g.push(v);
    }
}

fn main() {
    let c = Arc::new(Cache { data: Mutex::new(Vec::new()) });
    let c2 = Arc::clone(&c);

    let _ = thread::spawn(move || {
        let mut g = c2.data.lock().unwrap();
        g.push(1);
        panic!("写日志时崩了");
    })
    .join();

    c.add(2);
    let g = c.data.lock().unwrap_or_else(|e| e.into_inner());
    println!("{:?}", *g);   // [1, 2]
}
```

程序退出码变成 0，但第一个 panic 的消息仍会打印在 stderr——panic 本身没有被吞掉，`join` 返回的 `Err` 里还带着它。

反过来，如果数据的不变量很重要（账户余额、索引结构），正确做法是让错误继续传播，或者整体重建。`Mutex::clear_poison`（1.77 起）可以显式清除标记，但它只该在你**确认数据完好**之后调用，不是消除警告的开关。

</details>

## 动手练习

1. 把 22.4 的三个方案各写一遍，改成 8 线程、每线程 500 万次，记录耗时。再用 `std::hint::black_box` 防止本地计数被优化掉，看看"本地累加"到底快多少。
2. 用 `RwLock<HashMap<String, u64>>` 做词频统计：4 个线程各自统计一个文件片段并写入，主线程定期读快照。说明为什么这里 `RwLock` 比 `Mutex` 合适，以及写者会不会饿死。
3. 写出 22.8 的 ABBA 死锁，然后改成"用 `try_lock` 拿两把锁，任何一把失败就释放已拿到的、随机退避后重试"。解释为什么必须退避，不退避会得到什么。
4. 给 22.3 的 `Config` 加一个 `set(&self, v: i32)` 方法，要求中毒后仍能读写，并说明：什么情况下忽略 poison 是合理的，什么情况下必须重建数据。
5. 用 `AtomicBool` 和 `compare_exchange` 实现一个自旋锁，配一个 `SpinGuard` 实现 `Deref/DerefMut`，在 `Drop` 里 `store(false, Release)`。和 `std::sync::Mutex` 比 4 线程各 100 万次，解释差距来自哪里。

## 小结

- `Mutex<T>` 把数据和锁绑成一个值，拿到守卫才算拿到访问权；没有"忘了解锁"这回事，`Drop` 负责释放。
- 守卫通过 `Deref/DerefMut` 表现得像 `&T`/`&mut T`，借用检查器照常工作；守卫本身不可 `Copy`，1.71 起还不可 `Send`。
- poisoning 是运行期机制：持锁 panic 会让后续 `lock` 返回 `Err`。`into_inner` 可取回数据，但要不要取回是业务决定。
- `RwLock` 允许多读或单写，适合读远多于写的场景；写者可能饥饿，临界区长短决定它是否划算。
- 原子类型只保证单个操作的原子性，`Ordering` 管的是操作之间的可见性与重排，不是"原子/不原子"。
- `Relaxed` 适合纯计数；`Release`/`Acquire` 配对用于"发布数据 + 读方确认"；`SeqCst` 额外约束所有顺序一致操作的全局顺序，具体性能代价要测量。
- 锁粒度是性能与正确性的折中：临界区越小越好，但拆得太细会让不变量失去保护。先测量，再拆。
- 死锁的四种常见成因：同一线程重复加锁、多锁顺序不一致、持锁调用外部代码、守卫生命周期过长（含跨 `.await`）。
- futex 的启示：无争用走一条 CAS，争用才进内核睡眠，睡前先自旋；这也是"锁本身不慢，争用才慢"的底层原因。

共享可变状态能解决问题，但它把"谁在什么时候改了什么"变成了需要人工推理的事情。下一章换一个思路：不让线程共享内存，而是让它们传消息。
