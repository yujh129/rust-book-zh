# 第 24 章 async/await（一）：Future 与执行器

> **本章目标**
>
> - 说清 `async fn` 的返回类型是 `impl Future<Output = T>`，以及 future 为什么"不 poll 就不执行"
> - 把 `.await` 还原成状态机加 poll 循环，看懂编译器为 async 代码生成的结构
> - 只用标准库手写一个基于 `Waker` 的 `block_on` 执行器，并用它驱动真正的定时 future
> - 解释 `Pin` 与 `Send` 各自约束了什么：自引用结构不能移动，跨 await 的局部变量决定 future 能否跨线程
> - 说清 M:N 调度与 OS 线程的关系，能给出"用线程还是用 async"的判据

第 21 章算过一笔账：创建一个线程要 50–150 微秒，每个线程预留 2 MB 栈。如果服务器要同时维持一万个连接，一人一线程就是 20 GB 虚拟地址空间，外加大量上下文切换；而任意时刻，其中绝大多数连接都在等数据，什么也没干。

异步（async）的思路是把"等待"从线程里拿出来。连接没数据时，不占着线程睡，而是把"这个任务下次有进展时该做什么"记下来，线程转身去处理别的任务。等数据到了，再把任务放回队列。这样一来，少量线程就能撑住大量并发连接。

Rust 的做法和 Go、Python 都不同：标准库不提供运行时（runtime）。`async fn` 只被编译成一台状态机，`Future` 只是一个 trait，`Waker` 只是一个回调。没有运行时，future 就一步也不会走，这是很多初学者"代码写了却不执行"的根源。好处是这套机制能用在没有操作系统的环境（嵌入式、WASM），代价是你必须自己选一个运行时。

本章不装任何第三方库，只用标准库手写一个能跑的执行器，把 `poll`、`Waker`、`Pin` 这三样东西拆开看。第 25 章再换成 tokio 写真实服务。

## 24.1 async fn 返回的是 Future，不是结果

给函数加上 `async`，返回类型就变了。下面两种写法在类型层面等价：

```text
async fn fetch() -> String { ... }

fn fetch() -> impl Future<Output = String> { ... }
```

`Future` 是标准库里的 trait（`std::future::Future`），只要求实现一个方法：

```text
pub trait Future {
    type Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}
```

`Poll` 只有两个值：`Ready(T)` 表示算完了，`Pending` 表示"还没好，等会儿再来"。所以 future 不是"正在后台跑的任务"，而是一台需要别人摇动的机器：谁调用 `poll`，它才往前走一步。

下面这段程序手动 poll 一次，你能亲眼看到函数体是"被 poll 出来的"：

```rust
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

async fn fetch() -> String {
    println!("  [fetch] 函数体开始执行");
    String::from("data")
}

fn main() {
    println!("1. 调用 fetch()");
    let mut fut = pin!(fetch());
    println!("2. future 已创建，但函数体一行都没跑");

    let mut cx = Context::from_waker(Waker::noop());
    println!("3. 手动 poll 一次");
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => println!("4. poll 返回 Ready({})", v),
        Poll::Pending => println!("4. poll 返回 Pending"),
    }
}
```

```text
1. 调用 fetch()
2. future 已创建，但函数体一行都没跑
3. 手动 poll 一次
  [fetch] 函数体开始执行
4. poll 返回 Ready(data)
```

三行代码值得逐条解释：

- `pin!(fetch())` 把 future 固定在当前栈帧上，得到一个 `Pin<&mut impl Future>`。`Future::poll` 的接收者是 `Pin<&mut Self>` 而不是 `&mut self`，原因在 24.5 节。
- `Waker::noop()` 返回一个"被调用时什么也不做"的 waker。手动 poll 时用不到唤醒，拿它凑一个 `Context` 就行。
- 调用 `fetch()` 只做了一件事：把状态机结构体构造出来，参数被搬进它的字段。函数体里的 `println!` 要等到第一次 `poll` 才执行。

**future 是惰性的（lazy）。** 创建不等于启动，只有 `poll` 才推进。这条规则也决定了"取消"的语义：一个 future 被 `drop` 而没 poll 完，函数体就停在半路，已存在的局部变量按正常规则析构。第 25 章讲 tokio 的 `abort` 时会回到这一点。

`.await` 只是"poll 到 Ready 为止"的语法糖。忘了写 `.await`，你拿到的就是一个 future 而不是 `Output`，编译器会用各种方式提醒你，但提示未必直白（见案例 1）。

`async` 也能直接修饰一个块，得到匿名的 future：

```rust
fn main() {
    let a = async { 1 + 1 };            // 不捕获环境
    let msg = String::from("hi");
    let b = async move { msg.len() };   // 把 msg 移进 future

    println!("两个 future 已创建：{} {}", std::mem::size_of_val(&a), std::mem::size_of_val(&b));
}
```

`async fn` 可以理解成"参数进、async 块出"的语法糖：参数被复制或移动进 future，函数体成为这个块的尾巴。`async move` 与闭包的 `move` 规则相同，区别只在于搬进的是状态机而不是闭包环境。

## 24.2 .await 展开成什么：一台状态机

`.await` 常被误认为"阻塞等待"。它其实是一次 `match`：poll 一下子 future，`Ready` 就取走结果继续往下走，`Pending` 就把控制权还给执行器，函数就此返回；等下次被 poll 时，从刚才停下的位置接着跑。函数能在中间"返回又续上"，靠的是编译器把函数体改写成状态机。

看一个带两个 await 的函数：

```rust
async fn first() -> u32 {
    1
}

async fn second(a: u32) -> u32 {
    a + 1
}

async fn two_steps() -> u32 {
    let a = first().await;
    let b = second(a).await;
    a + b
}

fn main() {
    let fut = two_steps();
    println!("future 占 {} 字节", std::mem::size_of_val(&fut));
}
```

```text
future 占 16 字节
```

编译器生成的东西大致长这样（变体名与字段布局是实现细节，这里只画结构）：

```text
enum TwoSteps {
    Start,                    // 还没开始
    AwaitFirst,               // 停在 first().await，里面存着 first 的子 future
    AwaitSecond { a: u32 },   // 停在 second(a).await，a 要活过这个 await
    Done,
}
```

两个关键点：

- 每个 `.await` 都是一个可能的出口，所以是一个变体边界。函数被 poll 时从当前变体继续，遇到 `Pending` 就带着当前状态返回。
- 跨 await 还活着的局部变量必须存进状态机。上面 `a` 在第二个 await 之后还要用，于是变成 `AwaitSecond` 的字段。这也解释了为什么 future 的大小随函数体增长。

回到 24.1 的 `fetch`：它没有 await，状态机只有"开始"和"结束"两个状态，所以一次 poll 就能跑完。有 await 的 future 才需要被 poll 多次。

### 自己实现一个 Future

要理解 poll 的契约，最好的办法是手写一个。下面这个 `YieldOnce` 第一次 poll 返回 `Pending`，第二次返回 `Ready`，模拟"让出一次再继续"：

```rust
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();   // 告诉执行器：现在就可以再 poll 我
            Poll::Pending
        }
    }
}

fn main() {
    let mut fut = std::pin::pin!(YieldOnce { yielded: false });
    let mut cx = Context::from_waker(Waker::noop());

    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(()) => println!("第一次 poll: Ready(())"),
        Poll::Pending => println!("第一次 poll: Pending"),
    }
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(()) => println!("第二次 poll: Ready(())"),
        Poll::Pending => println!("第二次 poll: Pending"),
    }
}
```

```text
第一次 poll: Pending
第二次 poll: Ready(())
```

这段代码把 poll 的三条契约都摆了出来：

- **返回 `Pending` 之前必须设法唤醒自己。** 这里调 `wake_by_ref`；真实的 IO future 会把 waker 交给事件循环，等数据到了再调。忘了唤醒，执行器会一直睡下去，任务永远不完成。
- **同一个 future 会被 poll 很多次。** 每次调用都从上次的状态继续，所以 `yielded` 要记住进度。被多唤醒、多 poll 几次是允许的（称为虚假唤醒，spurious wakeup），poll 必须能容忍。
- **完成后不能再 poll。** 已经返回过 `Ready` 的 future 再被 poll 会 panic，信息是 `async fn resumed after completion`。这一条保护的是状态机里已经被移走的字段。

### future 有多大

状态机的大小完全由函数体决定：跨 await 的局部变量越多越大，没跨 await 的临时值不占空间。

```rust
async fn small() -> u32 {
    1
}

async fn big() -> u32 {
    let buf = [0u8; 64 * 1024];
    std::future::ready(()).await;
    buf[0] as u32
}

fn main() {
    println!("small: {} 字节", std::mem::size_of_val(&small()));
    println!("big:   {} 字节", std::mem::size_of_val(&big()));
}
```

```text
small: 1 字节
big:   65538 字节
```

`small` 的 future 只有一个状态字节；`big` 把 64 KB 的数组搬进了状态机，因为 `buf` 要活过那个 await。这带来两个实用推论：

- future 默认放在栈上或调用者给的位置。64 KB 的状态机放在栈上不算大事，但如果是递归的 async 函数，编译器无法算出固定大小，会直接报错 `E0733: recursion in an async fn requires boxing`，解法是用 `Box::pin` 把递归的那一层装箱。
- 状态机很大时，可以早一点把数据丢进 `Box` 或 `Vec`，让字段只存一个指针，future 立刻瘦身。

## 24.3 深入：poll、Context 与 Waker 的契约

前面两节已经用过 poll，但 `Context` 里装的是什么、唤醒为什么必须由 future 自己发起，还没交代。这一节把这条协议说完整。

`poll` 的签名有三处不寻常：

```text
fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
```

- `Pin<&mut Self>`：对于依赖地址稳定的 future，保证从固定起直到析构（包括两次 poll 之间）一直位于原处，理由在 24.5 节。
- `Context<'_>`：目前只提供 `waker()`，返回 `&Waker`。它是 future 与执行器之间唯一的联系。
- `Poll<Self::Output>`：`Ready` 或 `Pending`，没有第三种。

一次完整的往返是这样：

```text
执行器                          future                        事件源（定时器 / socket）
  |                              |                              |
  |-- poll(cx) ----------------->|                              |
  |                              |-- 把 cx.waker() 登记过去 ---->|
  |<---------------- Pending ----|                              |
  |  （线程转身去 poll 别的任务）  |                              |
  |<----------------------------- wake() -----------------------|
  |-- poll(cx) ----------------->|                              |
  |<---------------- Ready(v) ----|                              |
```

三方各有各的责任，缺一条协议就断：

- **执行器**维护一个就绪队列，对队列里的任务逐个 poll。它不知道 future 在等什么，也不关心。
- **future** 在返回 `Pending` 之前，必须把 waker 交给能决定"何时有进展"的那一方。拿不到数据就老老实实返回 `Pending`，不许在原地循环等待。
- **事件源**（定时器线程、epoll、IOCP）在条件满足时调用 `wake()`，把任务重新放回就绪队列。

这套设计叫拉模型（pull）：控制权始终在调用方，执行器主动来问"好了没"，future 不必保存自己的栈，也不必注册回调。代价是 future 必须自己管好唤醒，一旦漏掉，任务就永久挂起。这类 bug 不报错、不 panic，只是静静地卡住，所以调试异步代码时第一个问题永远是"谁负责唤醒我"。

### Waker 与 Wake

`Waker` 是"唤醒能力"的抽象，可以克隆、可以跨线程传递，并且必须同时满足 `Send + Sync`：任务可能在 A 线程被 poll，却在 B 线程被事件源唤醒。

手动构造 `Waker` 需要 `RawWaker` 和一张函数指针表，全是 `unsafe`。标准库提供了安全的替代品 `std::task::Wake`：你实现一个 trait，然后 `Waker::from(Arc<T>)` 就得到一个 waker。

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Wake, Waker};

struct Counter {
    woken: AtomicUsize,
}

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.woken.fetch_add(1, Ordering::SeqCst);
    }
    // wake_by_ref 有默认实现：克隆一份 Arc，再调用 wake
}

fn main() {
    let counter = Arc::new(Counter { woken: AtomicUsize::new(0) });
    let waker = Waker::from(Arc::clone(&counter));

    waker.wake_by_ref();
    waker.wake_by_ref();
    println!("被唤醒 {} 次", counter.woken.load(Ordering::SeqCst));
}
```

```text
被唤醒 2 次
```

`Wake` 的签名 `fn wake(self: Arc<Self>)` 需要 `Arc<Self>: Sized`，这正是它只能在 `Arc` 上实现的原因：waker 会被复制很多份，每份都要能独立地活到最后一刻。24.4 节的执行器会用同样的办法实现自己的 waker。

### 为什么 poll 要返回 Poll 而不是 Option

`Option` 只有"有"和"无"两种状态，而异步需要三种："算完了"、"还没好但以后会有"、"数据在里面"。`Poll<T>` 就是 `Ready(T) | Pending`，`Pending` 明确表示"以后还会有进展"，执行器据此把任务挂起而不是丢弃。

顺带说明一个常见误解：`Pending` 不是错误，也不表示"慢"。一个 future 在生命周期里返回几十次 `Pending` 完全正常，只要每次都有对应的唤醒。

## 24.4 手写一个最小执行器

现在把零件拼起来。下面的程序只用标准库，实现了一个能跑的 `block_on`，并用它驱动一个真正的定时 future：两次 80 毫秒的等待，总耗时约 160 毫秒，而不是空转。

```rust
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

// 唤醒信号：置位"就绪"标志，并通知阻塞等待的线程
struct Signal {
    woken: Mutex<bool>,
    condvar: Condvar,
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut woken = self.woken.lock().unwrap();
        *woken = true;
        self.condvar.notify_one();
    }
}

// 最小执行器：把一个 future 跑到完成
fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let signal = Arc::new(Signal {
        woken: Mutex::new(false),
        condvar: Condvar::new(),
    });
    let waker = Waker::from(Arc::clone(&signal));
    let mut cx = Context::from_waker(&waker);

    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => {
                let mut woken = signal.woken.lock().unwrap();
                while !*woken {
                    woken = signal.condvar.wait(woken).unwrap();
                }
                *woken = false;
            }
        }
    }
}

// 只用标准库实现的定时 future
struct Delay {
    ms: u64,
    done: Arc<AtomicBool>,
    started: bool,
}

impl Delay {
    fn new(ms: u64) -> Self {
        Delay {
            ms,
            done: Arc::new(AtomicBool::new(false)),
            started: false,
        }
    }
}

impl Future for Delay {
    type Output = u64;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u64> {
        if self.done.load(Ordering::SeqCst) {
            return Poll::Ready(self.ms);
        }
        if !self.started {
            self.started = true;
            let done = Arc::clone(&self.done);
            let waker = cx.waker().clone();
            let ms = self.ms;
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms));
                done.store(true, Ordering::SeqCst);
                waker.wake();
            });
        }
        Poll::Pending
    }
}

fn main() {
    let start = Instant::now();
    block_on(async {
        println!("开始第一个等待");
        Delay::new(80).await;
        println!("第一个等待结束，累计 {} ms", start.elapsed().as_millis());
        Delay::new(80).await;
        println!("第二个等待结束，累计 {} ms", start.elapsed().as_millis());
    });
    println!("总等待 160 ms，实际耗时 {} ms", start.elapsed().as_millis());
}
```

```text
开始第一个等待
第一个等待结束，累计 80 ms
第二个等待结束，累计 161 ms
总等待 160 ms，实际耗时 161 ms
```

一百来行，值得拆成四块看。

**`Signal`：把"唤醒"变成"能睡醒的条件变量"。** `wake` 把布尔标志置为 `true`，再 `notify_one` 叫醒执行器线程。这里没有用自旋，因为自旋会白烧一个 CPU 核；阻塞在条件变量上，等待期间线程完全让出 CPU。`woken` 初值为 `false`，配合锁使用才能不丢唤醒：唤醒可能在 poll 返回之后、加锁之前到达，标志位会把它记下来，`block_on` 加锁后先看到标志，就不会睡过去。

**`while !*woken` 而不是 `if`。** 条件变量可能被虚假唤醒，标准库明确允许 `wait` 无故返回；用 `while` 复查条件才是正确写法。同理，future 被多唤醒几次也无害，`block_on` 无非多 poll 一遍，拿到 `Pending` 再睡回去。

**`block_on`：poll、睡、再 poll。** `pin!` 把 future 固定在当前栈帧，循环的骨架只有两个分支：`Ready` 就带着结果返回，`Pending` 就锁住标志、睡到有人唤醒，醒来后清掉标志再 poll。真实运行时的循环长得一样，只是"任务"是队列里的一堆 future，"睡"是 epoll 或 IOCP 的系统调用。

**`Delay`：最笨的定时器。** 第一次 poll 时起一个线程，睡够时间后置位 `done` 并调用 `waker.wake()`；之后的每次 poll 只检查 `done`。这里"每个定时器一个线程"是为了不引入任何第三方库，代价很大：一万个定时器就是一万个线程。真实的运行时用一个 reactor 线程配合最小堆管理所有定时器，到点了只唤醒对应的任务，这也是 tokio 只需要几十个线程就能撑住大量连接的原因。

最后注意 `block_on<F: Future>` 的约束：**没有 `F: Send`**。它全程在调用线程上 poll，future 从来没离开过这个线程。`tokio::spawn` 则要求 `Send`，因为任务可能被工作窃取到别的线程上执行。同一个 future 能在前者跑、不能在后者跑，是很常见的困惑来源，24.6 节专门处理。

## 24.5 Pin：为什么 future 不能随便移动

24.2 节的状态机有个隐患：字段之间可能互相指。比如这样一段代码：

```text
let text = String::from("hello");
let view = text.as_str();   // view 指向 text 的缓冲区
io().await;                 // await 期间 text 与 view 都必须活着
println!("{}", view);
```

编译器会把 `text` 和 `view` 都放进状态机，而 `view` 里存的是 `text` 缓冲区的地址。于是这个结构体指向了自己的一部分：

```text
状态机（示意）
+-------------------------------+
| 状态: AwaitIo                  |
| text: String  <---------+      |
| view: &str --------------+      |  指向自己的 text
+-------------------------------+
```

Rust 的普通值随时可能被移动：赋值、放进 `Vec` 触发扩容、按值返回。一旦这个结构体被 memcpy 到新地址，`view` 还指着旧地址，就是一个悬垂指针。`Pin` 的存在就是为了堵住这个口子。

`Pin<P>` 是一个包装，保证它指向的值在"被固定"期间地址不变，`P` 通常是 `Box<T>` 或 `&mut T`。配套的 `Unpin` 是一个自动 trait，含义是"这个类型移动了也没关系"。规则可以压缩成四条：

- 绝大多数类型自动实现 `Unpin`，包括所有字段都 `Unpin` 的结构体。对它们，`Pin<&mut T>` 可以用 `get_mut` 拿回 `&mut T`，与普通引用没有区别。
- 使用编译器生成的 async 块时，应按它没有实现 `Unpin` 来写；即使源码中看不到自引用，也可以用 `pin!` 固定后再手动轮询。
- 固定一个值有三种常见方式：`Box::pin(v)` 固定在堆上；`pin!(v)` 固定在当前栈帧、直到作用域结束；`Pin::new(&mut v)` 只接受 `Unpin` 的类型。
- 投影（projection）可以取得未固定字段的 `&mut Field` 或固定字段的 `Pin<&mut Field>`。它不会把整个 `!Unpin` 值变回可以任意移动的 `&mut T`。

看一个能跑的例子。`PhantomPinned` 是一个零大小的标记类型，把它放进结构体，结构体就失去 `Unpin`，效果等同于 future 的自引用：

```rust
use std::marker::PhantomPinned;
use std::pin::Pin;

struct Sticky {
    value: u32,
    _pin: PhantomPinned, // 让 Sticky 不再是 Unpin
}

fn main() {
    let boxed = Box::pin(Sticky { value: 7, _pin: PhantomPinned });
    let addr_before = &*boxed as *const Sticky as usize;

    let mut moved = boxed; // 移动的只是 Box 这个指针
    let addr_after = &*moved as *const Sticky as usize;
    println!("移动前后地址相同：{}", addr_before == addr_after);

    let pinned: Pin<&mut Sticky> = moved.as_mut();
    println!("值仍然是：{}", pinned.value);
}
```

```text
移动前后地址相同：true
值仍然是：7
```

被移动的是 `Box` 本身，它指向的 `Sticky` 在堆上没有动，所以地址不变。如果换成 `&mut Sticky` 加 `Pin::new`，编译器会直接拒绝，因为 `Sticky` 不是 `Unpin`。

实际写业务代码时，你几乎不会直接碰到 `Pin`：`.await`、`Box::pin` 和执行器已经替你处理好了。只有在写执行器、手写 `Future`、实现 `select`/`join` 这类组合器时，才需要自己面对它。24.4 节的 `Delay` 全是 `Unpin` 字段，所以能直接写 `self.started = true`；换成持有自引用的 future，这样写就过不了编译。

## 24.6 Send 与 async：future 为什么会"不能跨线程"

future 能不能跨线程，取决于它的字段能不能跨线程。24.2 节说过，状态机的字段就是跨 await 存活的局部变量；只要其中一个不是 `Send`，整个 future 就不是 `Send`。

最典型的是 `Rc`：它的引用计数是普通整数，克隆时不是原子操作。案例 4 会看到完整诊断。修复只有三类：让值在 await 之前结束（缩小作用域或提前 `drop`）、换成 `Arc`、或者换一个不要求 `Send` 的执行器（24.4 节的 `block_on`、tokio 的 `LocalSet`）。

编译器的提示值得记住：`future is not Send as this value is used across an await`，并标出变量声明处与 await 处。措辞是"可能用到"，即使你后面没再用它，只要作用域跨过 await 就算。

### trait 里的 async fn

Rust 1.75 起，trait 里可以直接写 `async fn`（async fn in trait，AFIT）。但这里有个坑：trait 方法返回的 future 是不透明类型，trait 定义里没有地方声明它是否 `Send`。于是在泛型上下文里调用时，编译器只保证它实现了 `Future`，不保证 `Send`：

```rust,noplayground
fn require_send<T: Send>(_: T) {}

trait Repo {
    async fn count(&self) -> usize;
}

fn run<R: Repo>(repo: &R) {
    require_send(repo.count());
}

fn main() {}
```

```text
error[E0277]: `impl Future<Output = usize>` cannot be sent between threads safely
 --> src/main.rs:8:18
  |
8 |     require_send(repo.count());
  |     ------------ ^^^^^^^^^^^^ `impl Future<Output = usize>` cannot be sent between threads safely
  |
  = help: the trait `Send` is not implemented for `impl Future<Output = usize>`
note: required by a bound in `require_send`
 --> src/main.rs:1:20
  |
1 | fn require_send<T: Send>(_: T) {}
  |                    ^^^^ required by this bound in `require_send`
help: `Send` can be made part of the associated future's guarantees for all implementations of `Repo::count`
  |
4 -     async fn count(&self) -> usize;
4 +     fn count(&self) -> impl std::future::Future<Output = usize> + Send;
  |
```

注意最后一段 help：编译器直接给出了改写建议。把 `async fn` 换成返回 `impl Future + Send` 的普通函数，就能把这个约束写进 trait：

```rust
use std::future::Future;

trait Repo {
    fn count(&self) -> impl Future<Output = usize> + Send;
}

struct Mem(Vec<u8>);

impl Repo for Mem {
    async fn count(&self) -> usize {
        self.0.len()
    }
}

fn require_send<T: Send>(_: T) {}

fn main() {
    let repo = Mem(vec![1, 2, 3]);
    require_send(repo.count());
    println!("future 是 Send");
}
```

这种写法叫 RPITIT（return position impl trait in trait），不装箱、零分配，是首选。缺点是每个实现的返回类型都不同（不透明类型），trait 因此不是 dyn 兼容的，不能用 `Box<dyn Repo>`。

需要 dyn 兼容、或者要兼容老版本编译器时，用 `async-trait` 宏：

```toml
[dependencies]
async-trait = "0.1"
```

```rust,noplayground
use async_trait::async_trait;

#[async_trait]
trait Repo {
    async fn count(&self) -> usize;
}

struct Mem(Vec<u8>);

#[async_trait]
impl Repo for Mem {
    async fn count(&self) -> usize {
        self.0.len()
    }
}

fn main() {
    let repo = Mem(vec![1, 2, 3]);
    let fut = repo.count(); // 类型是 Pin<Box<dyn Future<Output = usize> + Send>>
    println!("装箱后的 future 占 {} 字节", std::mem::size_of_val(&fut));
}
```

宏把方法改写成 `fn count(&self) -> Pin<Box<dyn Future<Output = usize> + Send + '_>>`：每次调用在堆上分配一个装箱的 future，调用点变成动态分发。这解决了 dyn 兼容问题，代价是每次调用一次堆分配。

如果场景根本不需要跨线程（单线程运行时、WASM、`LocalSet`），就不该强迫 future 是 `Send`。`#[async_trait(?Send)]` 表达的就是这个意思：类似 `?Sized` 那样，显式要求"不要默认加 `Send`"，生成的装箱类型里去掉 `+ Send`。语言层面目前还没有给 trait 方法逐个标注或取消 `Send` 的稳定语法，这类需求仍要靠宏或手写返回类型来满足。

## 24.7 M:N 调度，以及什么时候该用 async

第 21 章讲过，`std::thread` 是 1:1 模型：一个任务占一个操作系统线程，线程由内核调度。async 运行时走的是另一条路，叫 M:N 模型：运行时启动 M 个 worker 线程，在上面跑 N 个任务，N 通常远大于 M。每个任务是一个 future 加一点状态，`spawn` 把它放进就绪队列，哪个 worker 空闲就 poll 哪个；worker 本地队列空了，还会去别人的队列里"偷"任务（work stealing），以此均衡负载。

这套模型里，任务只在 `poll` 返回 `Pending` 时让出 worker。所以有一个铁律：**不要在 async 任务里调用阻塞函数**。一次 `thread::sleep`、一次同步文件读写、一次等锁，卡住的不只是当前任务，而是整个 worker 线程上排队的其他任务。必须阻塞时，用 `spawn_blocking` 把它扔进专用线程池。

| 维度 | OS 线程（1:1） | async 任务（M:N） |
|---|---|---|
| 最小成本 | 50–150 微秒，2 MB 栈 | 一次分配，大小为状态机 |
| 切换方式 | 内核上下文切换 | 用户态函数返回 |
| 阻塞调用 | 只影响自己 | 卡住整个 worker |
| 抢占 | 内核抢占，相对公平 | 协作式，靠 await 让出 |
| 适合场景 | CPU 密集、少量长任务 | IO 密集、海量连接 |

Rust 的 future 是无栈协程：没有独立栈，局部变量就是状态机的字段，切换时不需要保存寄存器。Go 的 goroutine 有栈（初始很小、按需增长），切换要保存调用栈。前者更省内存、切换更便宜，后者写起来没有 `async` 的传染性。

选择的判据可以简化成三条：

- 需要同时撑住大量空闲连接（网络服务、代理、爬虫），用 async。
- 计算密集、要把 CPU 核跑满，用线程加 rayon 这类库，或在 async 服务里 `spawn_blocking`。
- 任务数量少、逻辑以阻塞调用为主（命令行工具、批处理脚本），线程更简单，别为了 async 而 async。

## 典型错误案例

下面五段代码都能用 rustc 1.100.0 复现，报错信息照抄自实际输出。

### 案例 1：忘了 .await，把 future 当结果用——E0369

```rust,editable
async fn answer() -> i32 {
    42
}

fn main() {
    let n = answer();
    println!("{}", n + 1);
}
```

```text
error[E0369]: cannot add `{integer}` to `impl Future<Output = i32>`
 --> src/main.rs:7:22
  |
7 |     println!("{}", n + 1);
  |                    - ^ - {integer}
  |                    |
  |                    impl Future<Output = i32>
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`answer()` 返回的是 future，不是 `i32`。`+` 没有为 future 实现，编译器于是把两边的类型都列了出来。

**修复**：不需要异步就去掉 `async`；需要异步就在 async 上下文里 `answer().await`，并让执行器驱动它。

```rust,editable
fn answer() -> i32 {
    42
}

fn main() {
    println!("{}", answer() + 1);
}
```

</details>

### 案例 2：在普通函数里 .await——E0728

```rust,editable
async fn answer() -> i32 {
    42
}

fn main() {
    let n = answer().await;
}
```

```text
error[E0728]: `await` is only allowed inside `async` functions and blocks
 --> src/main.rs:6:22
  |
5 | fn main() {
  | --------- this is not `async`
6 |     let n = answer().await;
  |                      ^^^^^ only allowed inside `async` functions and blocks
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`.await` 需要一块由编译器生成的状态机来保存进度，`main` 是普通函数，没有这台状态机。

**修复**：把 await 放进 async 函数，由执行器驱动（24.4 节的 `block_on`，或第 25 章的 `#[tokio::main]`）。

```rust,editable
async fn answer() -> i32 {
    42
}

async fn run() -> i32 {
    answer().await
}

fn main() {
    let _fut = run(); // 交给执行器；这里只演示类型能通过检查
}
```

</details>

### 案例 3：直接对 future 调 poll——E0599

```rust,editable
use std::future::Future;
use std::task::{Context, Waker};

async fn answer() -> i32 {
    42
}

fn main() {
    let fut = answer();
    let mut cx = Context::from_waker(Waker::noop());
    let _ = fut.poll(&mut cx);
}
```

```text
error[E0599]: no method named `poll` found for opaque type `impl Future<Output = i32>` in the current scope
  --> src/main.rs:11:17
   |
11 |     let _ = fut.poll(&mut cx);
   |                 ^^^^ method not found in `impl Future<Output = i32>`
   |
   = help: method `poll` found on `Pin<&mut impl Future<Output = i32>>`, see documentation for `std::pin::Pin`
help: consider pinning the expression
   |
11 ~     let mut pinned = std::pin::pin!(fut);
12 ~     let _ = pinned.as_mut().poll(&mut cx);
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`poll` 的接收者是 `Pin<&mut Self>`，不是 `&mut self`。一个没被固定的 future 连 poll 这个方法都找不到（24.5 节解释了原因）。

**修复**：用 `pin!` 或 `Box::pin` 固定，再通过 `as_mut()` 拿到 `Pin<&mut _>`。

```rust,editable
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

async fn answer() -> i32 {
    42
}

fn main() {
    let mut fut = pin!(answer());
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => println!("{}", v),
        Poll::Pending => println!("还没好"),
    }
}
```

</details>

### 案例 4：跨 await 持有 Rc，future 不再是 Send

```rust,editable
use std::rc::Rc;

fn require_send<T: Send>(_: T) {}

async fn task() {
    let shared = Rc::new(1);
    std::future::ready(()).await;
    println!("{}", shared);
}

fn main() {
    require_send(task());
}
```

```text
error: future cannot be sent between threads safely
  --> src/main.rs:12:18
   |
12 |     require_send(task());
   |                  ^^^^^^ future returned by `task` is not `Send`
   |
   = help: within `impl Future<Output = ()>`, the trait `Send` is not implemented for `Rc<i32>`
note: future is not `Send` as this value is used across an await
  --> src/main.rs:7:28
   |
 6 |     let shared = Rc::new(1);
   |         ------ has type `Rc<i32>` which is not `Send`
 7 |     std::future::ready(()).await;
   |                            ^^^^^ await occurs here, with `shared` maybe used later
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Rc` 的计数不是原子的，不能跨线程。`shared` 活过了 await，于是它成为状态机的字段，整个 future 跟着变成 `!Send`。

**修复**：换成 `Arc`，或者让 `Rc` 在 await 之前 `drop`（缩小作用域）。也可以换一个不要求 `Send` 的执行器。

```rust,editable
use std::sync::Arc;

fn require_send<T: Send>(_: T) {}

async fn task() {
    let shared = Arc::new(1);
    std::future::ready(()).await;
    println!("{}", shared);
}

fn main() {
    require_send(task());
}
```

</details>

### 案例 5：async fn 的返回类型不对——E0308

```rust,editable
async fn parse_port() -> u16 {
    "8080"
}

fn main() {
    let _fut = parse_port();
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:2:5
  |
1 | async fn parse_port() -> u16 {
  |                           --- expected `u16` because of return type
2 |     "8080"
  |     ^^^^^^ expected `u16`, found `&str`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`async` 只改变返回类型的外层（包一层 future），`Output` 仍然是 `u16`。函数体返回的是 `&str`，与声明不符。

**修复**：转换类型，让最后一个表达式的类型与 `Output` 一致。

```rust,editable
async fn parse_port() -> u16 {
    "8080".parse().unwrap()
}

fn main() {
    let _fut = parse_port();
    println!("解析函数已就绪");
}
```

</details>

## 动手练习

1. 用 `Waker::noop()` 手动 poll 一个只打印一行的 async 函数，确认第一次 poll 就返回 `Ready`；再 poll 一次，解释为什么 panic，以及 `async fn resumed after completion` 在保护什么。
2. 给 24.4 节的 `Delay` 写一个 `Timeout<F>` 组合器：在 `F` 完成之前超时，就返回 `None`。只用标准库，并用 `block_on` 验证「先完成」和「先超时」两条路径。
3. 写一个 `YieldNow` future，连续 `await` 三次，用 `block_on` 驱动并打印每次恢复的位置；再用 `size_of_val` 比较局部变量放在栈上与放进 `Box` 时 future 的大小。
4. 把案例 3 里的 `fut.poll(&mut cx)` 改成 `pin!` 加 `as_mut()` 的写法，打印 `Poll` 的两种结果；然后故意去掉 `pin!`，确认报错与书中一致。
5. 实现 `join2(a, b)`：要求 `A: Unpin`、`B: Unpin`，一次 poll 同时推进两个 future，保存已经 `Ready` 的分支结果，后续只轮询尚未完成的分支；两边都就绪时才返回结果对。用两个 80 毫秒的 `Delay` 验证总耗时约 80 毫秒，而不是 160 毫秒。

## 小结

- `async fn` 的返回类型是 `impl Future<Output = T>`，`T` 是原来的返回类型；调用只构造状态机，`poll` 才让它前进。
- `Future::poll` 的签名是 `poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>`；返回 `Pending` 之前必须安排好唤醒，否则任务永久挂起。
- `.await` 展开成状态机：每个 await 是一个变体边界，跨 await 存活的局部变量成为字段，future 的大小由它们决定。
- 执行器就是「就绪队列 + poll 循环 + 睡眠/唤醒」；`Waker` 由 future 交给事件源，事件源负责把任务放回队列。
- `Pin` 保证 `!Unpin` 的值不被移动，async 生成的 future 通常就是 `!Unpin`；用 `pin!` 或 `Box::pin` 固定，`Unpin` 类型则完全无感。
- future 是否 `Send`，由跨 await 存活的局部变量决定；`Rc` 会让整个 future 变成 `!Send`，可以换 `Arc`、缩小作用域，或改用单线程执行器。
- trait 里的 `async fn` 不保证返回的 future 是 `Send`；用 RPITIT 加 `Send` 约束或 `async-trait` 解决，不需要跨线程时用 `?Send`。
- M:N 调度用少量线程跑大量任务，切换是用户态的 `poll` 返回；阻塞调用会卡住整个 worker，必须 `spawn_blocking`，CPU 密集的任务则应该交回线程。








