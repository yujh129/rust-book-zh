# 线程：std::thread 与 Send/Sync

> **本章目标**
>
> - 会用 `thread::spawn` 启动线程，并用 `JoinHandle` 等待结果
> - 理解闭包为什么必须 `move`，以及 `'static` 约束从哪来
> - 掌握 `join` 的返回值：子线程 panic 时如何拿到 `Err`
> - 用 `thread::scope` 借用局部变量，避免 `Arc` 和无谓的 `clone`
> - 理解 `Send` 与 `Sync` 是 auto trait，掌握"什么类型能跨线程"
> - 知道线程创建的真实成本，以及什么时候该用线程池

前面二十章都是单线程的。从这一章开始，代码会同时在多处执行——这也是 Rust 最能体现价值的地方：**数据竞争在编译期就是错误**。

```rust
use std::thread;
use std::time::Duration;

fn main() {
    let handle = thread::spawn(|| {
        for i in 1..=3 {
            println!("子线程：{}", i);
            thread::sleep(Duration::from_millis(10));
        }
        42
    });

    for i in 1..=2 {
        println!("主线程：{}", i);
        thread::sleep(Duration::from_millis(15));
    }

    // join 阻塞直到子线程结束，返回 Result<T, Box<dyn Any + Send>>
    let result = handle.join().unwrap();
    println!("子线程返回：{}", result);
}
```

输出顺序不固定——两次运行可能不同。这是并发程序的第一个特征：**你不该依赖线程间的执行顺序**。

## 21.1 `spawn` 与 `JoinHandle`

`thread::spawn` 接受一个 `FnOnce() -> T` 闭包，返回 `JoinHandle<T>`：

```rust
use std::thread;

fn main() {
    let h = thread::spawn(|| 1 + 1);
    let v = h.join().expect("子线程 panic 了");
    println!("{}", v);
}
```

关键点：

- 闭包必须有 `'static` 生命周期——线程可能在函数返回后才结束，所以它不能借用栈上的临时数据
- 闭包的返回值 `T` 也必须 `Send`，因为它要跨线程传回
- `JoinHandle::join` 消耗 `self`（`fn join(self)`），一个句柄只能 join 一次

如果不管子线程，直接 drop `JoinHandle`——线程会继续跑（detach），主线程退出时进程结束，所有线程被强杀：

```rust
use std::thread;
use std::time::Duration;

fn main() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(10));
        println!("这一行永远不会打印");
    });

    println!("主线程结束，进程退出");
}
```

## 21.2 为什么必须 `move`

```rust,noplayground
use std::thread;

fn main() {
    let msg = String::from("hi");
    thread::spawn(|| {
        println!("{}", msg);
    });
}
```

这段代码编译不过，报 `error[E0373]: closure may outlive the current function, but it borrows msg, which is owned by the current function`。

原因：`|| { ... }` 是一个**借用**闭包，它借了 `msg`；而 `spawn` 要求闭包是 `'static` 的——线程活得可能比 `main` 的栈帧更久，那 `msg` 早就被销毁了。

加上 `move` 就解决了：闭包**按值捕获** `msg`，所有权转移进线程。

```rust
use std::thread;

fn main() {
    let msg = String::from("hi");
    let h = thread::spawn(move || {
        println!("{}", msg);
    });
    h.join().unwrap();
}
```

注意：`move` 之后 `msg` 在 `main` 里就不能再用了（所有权已经移走）。如果需要两边都用，先 clone：

```rust
use std::thread;

fn main() {
    let msg = String::from("hi");
    let msg2 = msg.clone();

    let h = thread::spawn(move || {
        println!("子线程看到：{}", msg2);
    });

    println!("主线程仍然持有：{}", msg);
    h.join().unwrap();
}
```

### `move` 只是"按值捕获"，不代表一定复制

对一个 `Copy` 类型（`i32`、`bool`），`move` 是复制；对 `String`，`move` 是转移；对 `Arc<T>`，`move` 是移动那个指针（引用计数不变，clone 才加）。所以 `move` 的语义要结合类型看。

## 21.3 `join`：等待与 panic 传播

`join` 返回 `Result<T, Box<dyn Any + Send + 'static>>`。子线程**正常结束**返回 `Ok(T)`；子线程 **panic** 时返回 `Err(payload)`：

```rust
use std::thread;

fn main() {
    let h = thread::spawn(|| {
        panic!("子线程炸了");
    });

    match h.join() {
        Ok(v) => println!("正常结束：{:?}", v),
        Err(e) => {
            // payload 通常是 panic! 的格式化字符串
            let msg = e.downcast_ref::<&str>().map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| String::from("未知 payload"));
            println!("子线程 panic：{}", msg);
        }
    }

    println!("主线程仍然活着");
}
```

关键事实：**一个线程的 panic 不会杀死整个进程**（除非是 `panic = "abort"` 配置，或栈溢出）。panic 只展开该线程的栈，把 payload 通过 `join` 交回来。这让"任务隔离"成为可能：一个 worker 崩了，主线程可以继续。

### 线程 panic 的影响范围

```rust
use std::thread;

fn main() {
    let h1 = thread::spawn(|| panic!("任务 A 失败"));
    let h2 = thread::spawn(|| 2 + 2);

    println!("A: {:?}", h1.join().is_err());
    println!("B: {:?}", h2.join().unwrap());
}
```

输出 `A: true`、`B: 4`。两个线程互不影响。

但要注意：如果子线程持有锁时 panic，锁会被标记为"中毒"（poisoned），后续 `lock()` 返回 `Err`——详见第 22 章。

### 线程名与调试

```rust
use std::thread;

fn main() {
    let builder = thread::Builder::new().name("worker-1".into());
    let h = builder.spawn(|| {
        let cur = thread::current();
        format!("我是 {}", cur.name().unwrap_or("无名"))
    }).unwrap();

    println!("{}", h.join().unwrap());
}
```

线程名会出现在 panic 信息、`/proc/<pid>/task/` 和调试器里。生产代码建议给每个线程起名，否则崩溃日志里只有 `unnamed`。

## 21.4 `thread::scope`：借用局部变量

`spawn` 要求 `'static`，这导致一个很烦的后果：想共享局部数据就得 `Arc` 包一层，或者到处 clone。Rust 1.63 引入的 `thread::scope` 解决了这个问题——**作用域内启动的线程保证在 `scope` 返回前结束**，因此可以安全地借用局部变量。

```rust
use std::thread;

fn main() {
    let data = vec![1, 2, 3];
    let name = String::from("scoped");

    thread::scope(|s| {
        s.spawn(|| println!("{} 的长度 {}", name, data.len()));
        s.spawn(|| println!("{} 的首元素 {}", name, data[0]));
    });

    // 两个线程都结束了，data 和 name 的借用已经归还
    println!("作用域结束，data 仍然可用：{:?}", data);
}
```

**不需要 `move`，不需要 `Arc`，也不需要 clone。** 编译器通过作用域的生命周期，证明"线程不会活过 `data`"。

### 拆分可变借用：并行的经典用法

```rust
use std::thread;

fn main() {
    let mut data = vec![0i32; 10];

    thread::scope(|s| {
        // split_at_mut 把切片拆成两个不重叠的可变借用
        let (left, right) = data.split_at_mut(5);

        s.spawn(move || {
            for x in left.iter_mut() {
                *x = 1;
            }
        });

        s.spawn(move || {
            for x in right.iter_mut() {
                *x = 2;
            }
        });
    });

    println!("{:?}", data);
}
```

输出 `[1, 1, 1, 1, 1, 2, 2, 2, 2, 2]`。这是数据并行（分块处理）最简洁的写法：`split_at_mut` 在编译期证明了两个切片不重叠，每个线程拿一块。

如果两个线程需要访问**同一块**可变数据，编译器会拒绝——那说明你需要 `Mutex` 或原子类型，而不是"想办法绕过"。

### 作用域线程的 panic 处理

`s.spawn` 也返回 `ScopedJoinHandle`，可以 join。但如果一个作用域线程 panic 而你没 join，`scope` 会在返回时重新 panic：

```rust
use std::thread;

fn main() {
    let result = std::panic::catch_unwind(|| {
        thread::scope(|s| {
            s.spawn(|| panic!("子线程失败"));
        });
    });
    println!("捕获到 panic：{}", result.is_err());
}
```

这个设计是刻意为之：作用域线程的失败不应该被静默忽略。要自己处理，就显式 `join` 每个 handle 并检查 `Result`。

## 21.5 线程的成本与线程池

```rust
use std::thread;
use std::time::Instant;

fn main() {
    let n = 100;
    let start = Instant::now();
    let mut handles = Vec::with_capacity(n);

    for _ in 0..n {
        handles.push(thread::spawn(|| 1 + 1));
    }
    for h in handles {
        h.join().unwrap();
    }

    println!("创建并回收 {} 个线程耗时：{:?}", n, start.elapsed());
}
```

在我的机器上大约 5–15 ms，也就是**每个线程 50–150 微秒**。其中大部分是 `pthread_create` 的系统调用和内核调度，而不是你的任务。

线程还有别的成本：

- **栈空间**：每个线程默认预留 2 MB 虚拟内存（Linux 常见 8 MB），1000 个线程就是 2–8 GB 虚拟地址空间
- **调度开销**：线程数远超 CPU 核心数时，上下文切换会吃掉大量 CPU
- **内存同步**：跨核共享数据需要缓存一致性协议

所以"每个任务开一个线程"只在任务数很少时可行。任务多的时候用**线程池**：固定数量的工作线程，从队列里取任务。

标准库没有线程池（`std::thread` 只提供原始线程），生产代码常用：

- `rayon`：数据并行的 `par_iter`，自动分治
- `tokio`：异步运行时的任务调度（不一定一任务一线程）
- `crossbeam`：提供 `scope` 的增强版、无锁队列等

```rust
use std::thread;

fn main() {
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    println!("可用并行度：{}", cores);
}
```

`available_parallelism`（1.59+）返回"当前进程实际可用的 CPU 数"，考虑 cgroup 配额，比 `num_cpus` 更准确。

### 什么时候该开线程

- 任务是 **CPU 密集**且能切分成互不依赖的块（图像分块处理、矩阵分区计算）
- 或任务是 **阻塞 IO** 且并发数不高（几十个连接）
- 不适合：成千上万的短小任务（用线程池），或高并发 IO（用 async，见第 24 章）

## 21.6 深入：Send 与 Sync

这两个 trait 是所有并发安全的基础，但它们的定义极其简洁：

```text
pub unsafe auto trait Send {}
pub unsafe auto trait Sync {}
```

- `T: Send`：`T` 的**所有权**可以安全地转移到另一个线程
- `T: Sync`：`&T` 可以安全地在多个线程间共享（等价于 `&T: Send`）

它们都是 **auto trait**：编译器根据字段自动推导，不需要你手动 impl。规则：

- 所有字段都是 `Send` → 结构体是 `Send`
- 所有字段都是 `Sync` → 结构体是 `Sync`
- 有 `PhantomData<T>` 就按 `T` 推导

"不安全"的类型用**负实现**（negative impl）标记：

```text
impl<T: ?Sized> !Send for Rc<T> {}
impl<T: ?Sized> !Sync for RefCell<T> {}
impl<T: ?Sized> !Sync for Cell<T> {}
impl<T: ?Sized> !Send for *const T {}
impl<T: ?Sized> !Send for *mut T {}
```

这解释了常见现象：

| 类型 | Send | Sync | 原因 |
|---|---|---|---|
| `i32`, `String`, `Vec<T>`（T: Send） | ✅ | ✅ | 纯数据，无共享可变 |
| `Rc<T>` | ❌ | ❌ | 引用计数非原子 |
| `Arc<T>`（T: Send + Sync） | ✅ | ✅ | 计数是原子的 |
| `Cell<T>` | ✅ | ❌ | 可 `&` 修改，非线程安全 |
| `RefCell<T>` | ✅ | ❌ | 借用计数非原子 |
| `Mutex<T>`（T: Send） | ✅ | ✅ | 内部有锁保护 |
| `*const T`, `*mut T` | ❌ | ❌ | 裸指针，编译器无法验证 |

注意 `Cell` 和 `RefCell` 是 `Send` 但**不是** `Sync`：可以把整个 `Cell` 移到另一个线程（独占使用是安全的），但不能让两个线程同时持有 `&Cell`。这正是下面这个错误的来源：

```rust,noplayground
use std::cell::Cell;
use std::thread;

fn main() {
    let c = Cell::new(0);
    thread::scope(|s| {
        s.spawn(|| c.set(1));
        s.spawn(|| c.set(2));
    });
    println!("{}", c.get());
}
```

`error[E0277]: Cell<i32> cannot be shared between threads safely`，`help: the trait Sync is not implemented for Cell<i32>`。

### 手动实现 `Send`/`Sync` 意味着什么

你可以对自己定义的类型写：

```rust
struct MyBox<T> {
    ptr: *mut T,
}

// 只有在你确实知道它在语义上安全时才能这么做
unsafe impl<T: Send> Send for MyBox<T> {}
unsafe impl<T: Send> Sync for MyBox<T> {}

fn main() {
    let b = MyBox { ptr: std::ptr::null_mut::<i32>() };
    let _ = b.ptr;
}
```

`unsafe impl Send` 的承诺是："我保证这个类型的所有权转移和共享引用不会导致数据竞争"。如果这个承诺是错的，你得到的是 UB——编译器不会再帮你检查。标准库里的 `Arc`、`Mutex`、`AtomicXxx` 都是这样实现的。

**实践建议**：除非你在写底层同步原语，否则不要手写 `unsafe impl Send/Sync`。

### 1:1 线程模型

`std::thread` 是 **1:1 模型**：每个 Rust 线程对应一个 OS 线程，由内核调度。优点是简单、可以直接用操作系统的所有能力（优先级、CPU 亲和性、`/proc` 观测）；缺点是创建贵、切换贵、数量受限。

对比之下，Go 的 goroutine 和 Rust 的 `async` 都是 **M:N 模型**：用户态的任务由运行时调度到少量 OS 线程上。第 24 章会详细讲。

### 数据竞争的三个必要条件

Rust 的并发安全来自一个事实：**数据竞争需要同时满足三件事**——两个线程访问同一块内存、至少一个在写、且没有同步。`Send`/`Sync`/`&`/`&mut` 和 `Mutex` 一起，在编译期排除了第三种可能性。这就是"The Rust Programming Language"里那句著名的口号：

> 无畏并发（Fearless concurrency）。


## 典型错误案例

### 案例 1：闭包没有 `move`——E0373

```rust,editable
use std::thread;

fn main() {
    let msg = String::from("hi");
    thread::spawn(|| {
        println!("{}", msg);
    });
}
```

```text
error[E0373]: closure may outlive the current function, but it borrows `msg`, which is owned by the current function
 --> src/main.rs:5:19
  |
5 |     thread::spawn(|| {
  |                   ^^ may outlive borrowed value `msg`
6 |         println!("{}", msg);
  |                        --- `msg` is borrowed here
  |
note: function requires argument type to outlive `'static`
 --> src/main.rs:5:5
  |
5 | /     thread::spawn(|| {
6 | |         println!("{}", msg);
  | |______________________^
  |
help: to force the closure to take ownership of `msg` (and any other referenced variables), use the `move` keyword
  |
5 |     thread::spawn(move || {
  |                   ++++

error: aborting due to 1 previous error
```

**原因**：`spawn` 的签名是 `fn spawn<F, T>(f: F) -> JoinHandle<T> where F: FnOnce() -> T + Send + 'static`。闭包默认按"能借就借"捕获，于是 `msg` 被借用，闭包带着 `&msg`——但线程可能活过栈帧，`'static` 约束因此被违反。

**修复**：闭包前加 `move`，让闭包按值捕获：

```rust,editable
use std::thread;

fn main() {
    let msg = String::from("hi");
    let h = thread::spawn(move || {
        println!("{}", msg);
    });
    h.join().unwrap();
}
```

或者不用 `spawn` 而用 `thread::scope`（21.4），在作用域里借用是合法的。

### 案例 2：move 之后再用原变量——E0382

```rust,editable
use std::thread;

fn main() {
    let mut v = vec![1, 2, 3];
    let h = thread::spawn(move || {
        v.push(4);
        v
    });
    v.push(5);
    println!("{:?}", h.join().unwrap());
}
```

```text
error[E0382]: borrow of moved value: `v`
 --> src/main.rs:9:5
  |
4 |     let mut v = vec![1, 2, 3];
  |         ----- move occurs because `v` has type `Vec<i32>`, which does not implement the `Copy` trait
5 |     let h = thread::spawn(move || {
  |                           ------- value moved into closure here
6 |         v.push(4);
  |         - variable moved due to use in closure
...
9 |     v.push(5);
  |     ^ value borrowed here after move
  |
  = note: ...

error: aborting due to 1 previous error
```

**原因**：`move` 把 `v` 的所有权整体转移到闭包里，主线程不能再碰它。这是所有权规则在线程边界的直接体现——如果两边都能改同一个 `Vec`，就出现了数据竞争（`Vec` 扩容时指针会重新分配，另一个线程可能读到悬垂指针）。

**修复**：要么 clone 一份给线程，要么用 `Arc<Mutex<Vec<i32>>>` 真正共享：

```rust,editable
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let v = Arc::new(Mutex::new(vec![1, 2, 3]));

    let v2 = Arc::clone(&v);
    let h = thread::spawn(move || {
        v2.lock().unwrap().push(4);
    });

    v.lock().unwrap().push(5);
    h.join().unwrap();

    println!("{:?}", v.lock().unwrap());
}
```

### 案例 3：把局部变量的引用交给线程——E0597

```rust,editable
use std::thread;

fn main() {
    let s = String::from("hello");
    let slice = &s[..];
    let h = thread::spawn(move || {
        println!("{}", slice);
    });
    h.join().unwrap();
}
```

```text
error[E0597]: `s` does not live long enough
 --> src/main.rs:5:18
  |
4 |     let s = String::from("hello");
  |         - binding `s` declared here
5 |     let slice = &s[..];
  |                    ^ borrowed value does not live long enough
6 |     let h = thread::spawn(move || {
  |  _____________-
7 | |         println!("{}", slice);
8 | |     });
  | |______- argument requires that `s` is borrowed for `'static`
9 |       h.join().unwrap();
  | }
  | -
  | |
  | `s` dropped here while still borrowed

error: aborting due to 1 previous error
```

**原因**：`move` 移动的是 `slice`（一个 `&str`），但 `slice` 指向的 `s` 仍在 `main` 的栈上。线程活过 `main` 的栈帧，引用就悬垂了。

**修复**：让线程拥有数据本身（`String` 而非 `&str`），或者用 `thread::scope`：

```rust,editable
use std::thread;

fn main() {
    let s = String::from("hello");

    // 方案一：把 String 移进线程
    let s2 = s.clone();
    let h = thread::spawn(move || {
        println!("{}", s2);
    });
    h.join().unwrap();

    // 方案二：作用域线程可以直接借用 s
    thread::scope(|scope| {
        scope.spawn(|| println!("作用域线程看到：{}", s));
    });
}
```

### 案例 4：裸指针不能跨线程——E0277

```rust,editable
use std::thread;

fn main() {
    let p: *const i32 = &1;
    thread::spawn(move || {
        unsafe { println!("{}", *p); }
    });
}
```

```text
error[E0277]: `*const i32` cannot be sent between threads safely
 --> src/main.rs:5:19
  |
5 |     thread::spawn(move || {
  |     ------------- ^------
  |     |             |
  |     |             within this `{closure@src/main.rs:5:19: 5:26}`
  |     |             required by a bound introduced by this call
6 |         unsafe { println!("{}", *p); }
7 |     });
  |     ^ `*const i32` cannot be sent between threads safely
  |
  = help: the trait `Send` is not implemented for `*const i32`
note: required because `p` is used within this closure

error: aborting due to 1 previous error
```

**原因**：裸指针没有所有权语义，编译器无法判断它指向的数据是否有效、是否被别名。`*const T` 和 `*mut T` 都**没有** `Send`/`Sync`。这是	extbf{有意的保守}：宁可拦住正确代码，也不放过数据竞争。

**修复**：用安全的包装类型表达意图，例如把指针包进 `Arc`/`Box`，或者用 `usize` 传递地址（在 `unsafe` 块里再转回指针，并自己承担正确性）：

```rust,editable
use std::sync::Arc;
use std::thread;

fn main() {
    let data = Arc::new(1i32);
    let d = Arc::clone(&data);

    thread::spawn(move || {
        println!("{}", *d);
    }).join().unwrap();

    println!("{}", *data);
}
```

如果确实需要跨线程共享裸指针（例如 FFI 回调），可以写一个 `struct SendPtr(*mut T); unsafe impl Send for SendPtr {}`，但必须自己保证生命周期与同步。

### 案例 5：`Cell` 不是 `Sync`——E0277

```rust,editable
use std::cell::Cell;
use std::thread;

fn main() {
    let c = Cell::new(0);
    thread::scope(|s| {
        s.spawn(|| c.set(1));
        s.spawn(|| c.set(2));
    });
    println!("{}", c.get());
}
```

```text
error[E0277]: `Cell<i32>` cannot be shared between threads safely
 --> src/main.rs:7:17
  |
7 |         s.spawn(|| c.set(1));
  |           ----- ^^^^^^^^^^ `Cell<i32>` cannot be shared between threads safely
  |           |
  |           required by a bound introduced by this call
  |
  = help: the trait `Sync` is not implemented for `Cell<i32>`
  = note: if you want to do aliasing and mutation between multiple threads, use `std::sync::RwLock` or `std::sync::atomic::AtomicI32` instead
  = note: required for `&Cell<i32>` to implement `Send`
note: required because `c` is used within this closure

error: aborting due to 1 previous error
```

**原因**：`Cell` 允许通过 `&` 修改内容（内部可变性），但它的读写没有任何同步。两个线程同时 `c.set()` 就是数据竞争（在 `Cell` 的语义里等于 UB）。所以 `Cell: Send` 但 `!Sync`，而这里两个闭包都借了 `&c`，需要 `&Cell: Send`，即 `Cell: Sync`。

**修复**：按 rustc 的提示换成 `AtomicI32`（无锁、最轻量），或者 `Mutex<i32>`：

```rust,editable
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

fn main() {
    let c = AtomicI32::new(0);
    thread::scope(|s| {
        s.spawn(|| { c.fetch_add(1, Ordering::Relaxed); });
        s.spawn(|| { c.fetch_add(2, Ordering::Relaxed); });
    });
    println!("{}", c.load(Ordering::Relaxed));
}
```

## 动手练习

**练习 1**：起 4 个线程，每个计算 `1..=250000` 中落在自己区间的数之和，主线程分别 `join` 后求和。先写 `move` 版本（每个线程拿 `(start, end)`），再用 `thread::scope` 重写，对比代码量。

**练习 2**：用 `thread::scope` + `chunks_mut` 把一个 `Vec<i32>` 分给 4 个线程并行乘 2。要求不 clone 数据、不用 `Mutex`。

**练习 3**：写一个函数 `fn reliable_task() -> Result<i32, String>`，内部起一个可能 panic 的子线程，用 `join()` 捕获 panic 并从 payload 里取出错误信息，返回 `Err(String)`。

**练习 4**：测量 1、10、100、1000 个空线程的创建+join 时间，画出大致曲线（用 `Instant` 打印毫秒数即可）。解释为什么曲线不是线性的。

**练习 5**：用 `thread::available_parallelism()` 和 `thread::scope`，写一个 `par_map<T, U>(v: &[T], f: impl Fn(&T) -> U + Sync) -> Vec<U>`，把 `v` 切成 N 份并行处理（不引第三方 crate）。注意 `U: Send`。

## 小结

- `thread::spawn` 要求闭包 `Send + 'static`；返回值必须 `Send`
- 闭包默认借用捕获，配合 `spawn` 必须加 `move`；`move` 之后原变量不可再用
- `join` 返回 `Result<T, Box<dyn Any + Send>>`；子线程 panic 只影响自己，主线程可以继续
- 不 join 的线程会被 detach，主线程退出时进程结束，所有线程被强杀
- `thread::scope`（1.63+）允许借用局部变量，配合 `split_at_mut` 能优雅地做数据并行
- 线程创建约 50–150 µs，且有 2–8 MB 栈开销；任务多时用线程池（`rayon` 等）
- `Send` 表示所有权可跨线程转移，`Sync` 表示 `&T` 可跨线程共享；两者都是 auto trait
- `Rc`/`Cell`/`RefCell`/裸指针不是 `Sync`（`Rc` 连 `Send` 都不是），`Arc`/`Mutex`/`Atomic` 才是并发安全的工具
- `std::thread` 是 1:1 模型（一线程一 OS 线程），async 是 M:N 模型，各有适用场景
- 数据竞争 = 多线程 + 同一块内存 + 至少一个写 + 无同步；Rust 用类型系统在编译期排除了它
