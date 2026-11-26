# 第 19 章 内部可变性

> **本章目标**
>
> - 说清 `&T` 为什么默认不可变，以及 `UnsafeCell<T>` 凭什么成为所有内部可变性类型的唯一基石
> - 面对 `Cell`、`RefCell`、`OnceLock`、`LazyLock`、`Mutex`、原子类型时，能按"是否跨线程、冲突时 panic 还是阻塞"选对工具
> - 会用 `OnceLock` 写全局配置、用 `LazyLock` 写惰性全局表，不再为此引入 `lazy_static`
> - 会用 `thread_local!` 保存每线程一份的状态，并说清它为什么不需要 `Sync`
> - 能解释 `Fn` 闭包为什么必须靠 `Cell`/`RefCell` 才能计数，以及 memoization 的借用边界

前面说过 Rust 没有字段级可变性，`&T` 是共享借用。两条规则合起来是一句硬话：**只要还有 `&T` 活着，`T` 就不会变**。

编译器拿这句话做优化，借用检查器拿它做证明。但真实程序里有一整类需求跟它正面冲突：缓存要在 `&self` 方法里写入，全局配置要在第一次使用时才初始化，回调要在被反复调用的同时记数。

第 18 章已经用过 `Cell` 和 `RefCell`，也见过它们 panic 的样子。本章不重复"怎么借、怎么改"，而是回答三个更底层的问题：这些类型靠什么合法地绕过 `&` 的承诺；它们的 `Send`/`Sync` 是怎么摆的；什么场景该选哪一个。

## 19.1 为什么 `&` 不能改：UnsafeCell 是唯一的基石

`&T` 在类型层面只承诺"共享"，但 Rust 额外把"不可变"也塞进了这个承诺里。编译器据此做两件事：把循环里重复的读合并成一次；假定 `&mut T` 与任何其它引用都不重叠（LLVM 里给 `&mut` 加 `noalias`）。

这两条优化都建立在"没人会在背后写"之上。一旦语言允许你通过 `&T` 改数据，优化后的代码就可能读到过期值——这不是逻辑错误，是未定义行为。

所以规则不能普遍开口子。唯一的口子开在一个类型上，标准库里的定义简化后是这样：

```rust,noplayground
// std::cell::UnsafeCell 的定义（简化）
#[repr(transparent)]
pub struct UnsafeCell<T: ?Sized> {
    value: T,
}
```

`#[repr(transparent)]` 意味着 `UnsafeCell<T>` 与 `T` 的布局完全一样，运行时零开销。编译器对它做了特例处理：**只有位于 `UnsafeCell` 内部的字节，才允许在共享引用存在时被写入**。

它的 `get` 返回裸指针，而不是引用：

```rust,noplayground
impl<T: ?Sized> UnsafeCell<T> {
    pub const fn get(&self) -> *mut T { /* 省略实现 */ }
}
```

返回 `*mut T` 而不是 `&mut T`，是因为前者不建立"独占借用"这层语义。解引用裸指针必须写 `unsafe`，等于把"此刻确实没有别人在读"的证明责任交给了调用者。

### 手写一个 Cell

`Cell` 的全部秘密就是这个包装。自己写一遍，能看清 `unsafe` 到底用在哪几行：

```rust
use std::cell::UnsafeCell;

struct MyCell<T> {
    value: UnsafeCell<T>,
}

impl<T: Copy> MyCell<T> {
    fn new(value: T) -> Self {
        MyCell { value: UnsafeCell::new(value) }
    }

    fn get(&self) -> T {
        unsafe { *self.value.get() }
    }

    fn set(&self, value: T) {
        unsafe { *self.value.get() = value }
    }
}

fn main() {
    let c = MyCell::new(1);
    c.set(c.get() + 1);
    println!("{}", c.get());
}
```

这段代码之所以安全，是因为它遵守了两条纪律：`get` 返回 `T` 的副本（要求 `T: Copy`），没有让 `&T` 逃逸；`set` 不经过任何已经存在的引用，直接覆盖内存。

更要紧的是最后一点：`MyCell` 自动是 `!Sync`。你一行 `unsafe impl` 都没写，编译器仍然不会让你把它跨线程共享——因为它含有 `UnsafeCell`。

### `!Sync`：把承诺收回，而不是提供保护

`UnsafeCell` 的定义里有一行负向实现：

```rust,noplayground
impl<T: ?Sized> !Sync for UnsafeCell<T> {}
```

`Sync` 的含义是"`&T` 可以安全地送到别的线程"。`UnsafeCell` 明确取消这个资格：它自己不知道谁在写、有没有同步，于是拒绝被共享。

注意它仍然是 `Send`（当 `T: Send` 时）。把整个 cell 移动给另一个线程没有问题，出问题的是"两个线程同时持有 `&cell`"。

这条 `!Sync` 是所有安全包装类型的地基：

```text
                  UnsafeCell<T>          唯一能 & -> *mut 的类型，!Sync
                       |
     +-----------------+------------------+------------------+
     |                 |                  |                  |
   Cell<T>         RefCell<T>        OnceCell<T>         Mutex<T>
   整体读写         运行期借用计数      只写一次             加锁
   !Sync            !Sync              !Sync               Sync
     |                 |                  |                  |
     |                 |                  +--> OnceLock<T>    +--> RwLock<T>
     |                 |                       只写一次，Sync      读写锁
     |                 |
     +-----------------+--> 这两个补不回 Sync，只能留在单线程

   AtomicUsize 等原子类型：内核同样是 UnsafeCell + 硬件原子指令，
   靠"读改写不可分割"自己实现 Sync
```

包装类型要做的事只有一件：把 `UnsafeCell` 丢掉的那份承诺，用别的机制补回来。

- `Mutex` 用锁保证同一时刻只有一个线程拿到 `&mut T`
- 原子类型用硬件指令保证读改写不可分割
- `OnceLock` 在共享访问下只接受一次成功初始化；`get_or_init` 的闭包若 panic，值仍未初始化，可以重试
- `Cell`/`RefCell` 补不回来，于是保持 `!Sync`，代价是只能单线程使用

**内部可变性不是"绕过借用检查"，而是"换一种检查方式"**：把编译期的证明换成运行期的计数、锁或原子指令。`unsafe` 只出现在标准库那一层，使用者拿到的仍然是安全接口。

## 19.2 选型对照表：八个类型怎么挑

这一节先给结论表，再给判断顺序。表中的"通过 `&` 写"指只拿到 `&self` 时，能不能改内部数据。

| 类型 | 通过 `&` 读 | 通过 `&` 写 | 冲突时 | `Sync` | 典型场景 |
| --- | --- | --- | --- | --- | --- |
| `Cell<T>` | `get`，要求 `T: Copy` | `set`、`replace`、`take` | 没有检查，后写覆盖 | 否 | 计数器、标志位、`&` 方法里的 `Copy` 字段 |
| `RefCell<T>` | `borrow` 得到 `Ref<T>` | `borrow_mut` 得到 `RefMut<T>` | panic，或 `try_borrow` 返回 `Err` | 否 | 单线程共享可改、缓存、树 |
| `OnceCell<T>` | `get` 得到 `Option<&T>` | `set` 一次 | 第二次 `set` 返回 `Err(T)` | 否 | 结构体里只初始化一次的字段 |
| `OnceLock<T>` | `get` 得到 `Option<&T>` | `set` 一次 | 第二次 `set` 返回 `Err(T)` | 是 | 全局配置、单例 |
| `LazyLock<T>` | `Deref` 得到 `&T` | 只能经内部可变性 | 初始化 panic 后永久不可用 | 是 | 全局惰性表、正则、字典 |
| `Mutex<T>` | `lock` 得到 `MutexGuard` | 同一个 `lock` | 阻塞，或 `try_lock` 返回 `Err` | 是 | 跨线程共享可改 |
| `RwLock<T>` | `read` 得到读守卫 | `write` 得到写守卫 | 读读并行，写者等待 | 是 | 读多写少的共享表 |
| `AtomicUsize` | `load` | `store`、`fetch_add` | 硬件原子，不阻塞 | 是 | 计数器、状态标志 |

判断顺序是四步：

1. **跨不跨线程**。单线程：`Cell` → `RefCell` → `OnceCell`。跨线程：`OnceLock`/`LazyLock` → `Atomic*` → `Mutex` → `RwLock`。
2. **需不需要 `&T`**。`Cell` 拿不到内部引用，只能整体读写；要借用内部（`Vec` 的元素、`HashMap` 的 entry），只能用 `RefCell` 或锁。
3. **冲突时想怎样**。`RefCell` 默认 panic，想优雅处理就用 `try_borrow`；`Mutex` 默认阻塞，想快速失败就用 `try_lock`；原子类型永不冲突。
4. **访问频率**。原子操作是一条带锁语义的机器指令；无竞争的 `Mutex` 也是一次原子交换，一旦竞争线程会被挂起、陷入内核，两者差几个数量级。热路径优先原子，复杂不变量优先锁。

`Cell` 的边界值得单独说一句：它没有 `borrow`，`get` 要求 `T: Copy`。所以 `Cell<Vec<T>>` 既不能读元素，也不能取 `&T`，只能 `take` 出来、改完再 `set` 回去。要"只读一个元素"，选 `RefCell<Vec<T>>`。

## 19.3 `OnceLock<T>`：只写一次的全局

全局配置是最典型的场景：进程启动时不想读文件，第一次用到时才读，读完不再变。

老写法是 `static mut`。它的问题不只是"要写 `unsafe`"，而是 `unsafe` 也兜不住：只要有两处代码同时拿到 `&mut`，就是未定义行为，而编译器不会帮你查（案例 5 会看到它现在连编译都过不去）。

`OnceLock<T>` 从 1.70 起稳定，把这件事收敛成一个原子状态机：未初始化 → 正在初始化 → 已初始化。

```rust
use std::sync::OnceLock;

#[derive(Debug)]
struct Config {
    workers: usize,
    verbose: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let workers = std::env::var("WORKERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4);
        Config {
            workers,
            verbose: std::env::var("VERBOSE").is_ok(),
        }
    })
}

fn main() {
    println!("{:?}", config());
    println!("两次拿到的是同一份：{}", std::ptr::eq(config(), config()));
}
```

关键签名都在 `&` 上操作，所以能直接放进 `static`：

```text
pub const fn new() -> OnceLock<T>
pub fn set(&self, value: T) -> Result<(), T>
pub fn get(&self) -> Option<&T>
pub fn get_or_init<F: FnOnce() -> T>(&self, f: F) -> &T
pub fn get_mut(&mut self) -> Option<&mut T>
```

`set` 返回 `Result<(), T>` 而不是 `bool`：第二次写入失败时，把值原样退回给你，不静默丢弃。这个设计逼你处理失败，虽然大多数代码会用 `let _ =` 把它吞掉（案例 3 就是这个问题）。

`get_or_init` 的语义比 `set` 更常用：闭包最多执行一次。多个线程同时调用时，只有一个线程真正运行闭包，其余线程阻塞等待；闭包返回后，所有线程拿到指向同一块内存的 `&T`。初始化完成之后，读取就是普通的内存读，不再加锁。

想让 `OnceLock<T>` 是 `Sync`，需要 `T: Send + Sync`：值会被多个线程共享读，所以 `T` 必须 `Sync`；初始化可能发生在另一个线程，所以 `T` 必须 `Send`。

单线程版本是 `std::cell::OnceCell<T>`，API 一样，只是 `!Sync`。它适合放进结构体，做一个"用到才算、算完不变"的字段——这正是 `RefCell` 做不到的那件事：`get_or_init` 返回的 `&T` 生命周期跟 cell 一样长，可以直接从方法里返回；`RefCell` 的 `Ref` 守卫是局部变量，函数返回时就得还回去。

## 19.4 `LazyLock<T>`：第一次解引用时才初始化

`OnceLock` 每次读都要 `get().unwrap()`，写起来啰嗦。如果一个全局值"一定会有"，只是不想在启动时构造，`LazyLock<T>` 更顺手：它实现 `Deref<Target = T>`，用起来跟普通 `static` 一样。

`LazyLock` 从 1.80 起稳定：

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

static TABLE: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("one", 1);
    m.insert("two", 2);
    m
});

fn main() {
    println!("{}", TABLE["two"]);
    println!("{}", TABLE.len());
}
```

`LazyLock::new` 是 `const fn`，所以能直接初始化 `static`。第一次解引用时执行闭包，之后每次解引用只读一次状态位，不再执行闭包——这也是它的固定成本：每次访问都有一次原子读。热路径上如果一秒访问上百万次，先 `let table = &*TABLE;` 取一次引用，循环里用这个引用。

初始化函数 panic 会毒化（poison）整个 cell。第一次 panic 原样抛给你，之后每次访问都 panic：

```text
thread 'main' panicked at src/main.rs:4:49:
初始化失败

thread 'main' panicked at library/std/src/sync/lazy_lock.rs:475:5:
LazyLock instance has previously been poisoned
```

这里的毒化不可恢复。`Mutex` 的毒化还能用 `PoisonError::into_inner` 把数据抢回来，`LazyLock` 没有对应手段——初始化函数要么不 panic，要么你得接受这个全局量此后不可用。

### 与 lazy_static、once_cell 的关系

在 `LazyLock` 稳定之前，社区的标准做法是 `lazy_static!` 宏或 `once_cell` crate。它们的对应关系很整齐：

- `once_cell::sync::Lazy` 对应 `std::sync::LazyLock`
- `once_cell::sync::OnceCell` 对应 `std::sync::OnceLock`
- `once_cell::unsync::OnceCell` 对应 `std::cell::OnceCell`
- `lazy_static!` 生成的隐藏类型，本质也是"静态存储 + 首次 `Deref` 时初始化"

`LazyLock` 是 `lazy_static!` 的类型化版本：它是个类型而不是宏，所以能放进结构体、能当函数参数、报错时不会甩出一堆宏展开。新代码不需要再引这两个 crate，只有工具链低于 1.80 时才需要。

`LazyLock` 本身不提供修改能力。要"惰性初始化 + 之后可改"，得把内部可变性套进去，比如 `LazyLock<Mutex<Vec<String>>>`——外层负责"只初始化一次"，内层负责"改"。第 22 章会讲 `Mutex` 的细节。

## 19.5 `thread_local!`：每个线程一份

有些状态天然属于线程：递归深度、请求 id、随机数发生器、解析器的临时缓冲。它们不需要跨线程共享，因此也不需要 `Sync`——可 `static` 偏偏要求 `Sync`，于是用不了。

`thread_local!` 声明的是"每个线程各有一份"的静态量：

```rust
use std::cell::Cell;
use std::thread;

thread_local! {
    static DEPTH: Cell<u32> = Cell::new(0);
}

fn enter() -> u32 {
    DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get()
    })
}

fn main() {
    println!("主线程 {}", enter());
    println!("主线程 {}", enter());

    let h = thread::spawn(enter);
    println!("子线程 {}", h.join().unwrap());
    println!("主线程 {}", enter());
}
```

输出是 `1`、`2`、`1`、`3`：子线程从 0 开始，主线程的计数没有被子线程影响。

`with` 的闭包不是多余的包装，它禁止 `&T` 逃逸出闭包。线程退出时 TLS 里的值会被 drop，如果允许返回 `&T`，这个引用立刻可能悬垂——编译器没法在类型层面表达"只在这个线程活着时有效"，于是干脆不让你把引用带出去。

几个容易忽略的细节：

- **惰性初始化**。第一次 `with` 才执行初始化表达式；从没访问过的线程不会构造这份值。
- **线程结束时 drop**。带析构的值在线程退出时按注册的逆序 drop。在线程局部值的 `Drop` 里再访问同一个 TLS 会 panic。
- **不需要 `Sync`**。每个线程只碰自己那份，所以 `Cell`、`RefCell` 在这里正合适，也不会有任何锁开销。
- **1.73 起的快捷方法**。`LocalKey<Cell<T>>` 提供 `get`/`set`/`take`，`LocalKey<RefCell<T>>` 提供 `with_borrow`/`with_borrow_mut`，省掉一层 `with` 闭包。

成本上，每次访问要走线程局部存储查表（ELF 平台上是 `__tls_get_addr`，Windows 走 TEB），比普通 `static` 慢。热循环里先把值 `with` 出来存进局部变量，再反复用。

## 19.6 trait 与闭包：`Fn` 为什么需要内部可变性

`Fn` 的调用签名是 `fn call(&self, args) -> R`。两个事实叠在一起：`&` 是共享借用；`Fn` 可以被调用任意多次。结论是闭包捕获的环境在调用期间只能被只读访问。

```rust,noplayground
let mut n = 0;
let f = move || {
    n += 1;
    n
};
```

这个闭包是 `FnMut` 而不是 `Fn`——它需要 `&mut` 环境。想把它塞进要求 `Fn` 的位置，编译器会拒绝。

把可变状态装进 `Cell` 就解决了：

```rust
use std::cell::Cell;

fn call_twice<F: Fn() -> u32>(f: F) {
    println!("{} {}", f(), f());
}

fn make_counter() -> impl Fn() -> u32 {
    let count = Cell::new(0);
    move || {
        count.set(count.get() + 1);
        count.get()
    }
}

fn main() {
    let next = make_counter();
    call_twice(&next);
    call_twice(&next);
}
```

`count` 被 move 进闭包，闭包体只拿到 `&count`（因为 `call` 接收 `&self`），而 `Cell::set` 正好只需要 `&self`。

这不是语法技巧，而是很多设计的硬约束。要把回调存成 `Rc<dyn Fn()>` 或 `Arc<dyn Fn()>` 时，`Fn` 是唯一能用的 trait：`dyn FnMut` 调用时需要独占借用，而多个持有者没法同时给出独占借用。GUI 事件回调、观察者列表、插件注册表都卡在这一条上。

同样的道理出现在 trait 方法上。`fn describe(&self) -> String` 只拿 `&self`，如果实现要缓存结果，缓存字段就得是 `RefCell` 或 `OnceCell`。

### 缓存字段的写法与借用边界

```rust
use std::cell::RefCell;
use std::collections::HashMap;

struct Fib {
    cache: RefCell<HashMap<u64, u64>>,
}

impl Fib {
    fn new() -> Self {
        Fib { cache: RefCell::new(HashMap::new()) }
    }

    fn get(&self, n: u64) -> u64 {
        if n < 2 {
            return n;
        }
        if let Some(&v) = self.cache.borrow().get(&n) {
            return v;
        }
        let v = self.get(n - 1) + self.get(n - 2);
        self.cache.borrow_mut().insert(n, v);
        v
    }
}

fn main() {
    let fib = Fib::new();
    println!("fib(40) = {}", fib.get(40));
    println!("缓存了 {} 项", fib.cache.borrow().len());
}
```

这段代码有三处刻意的安排，每一处都对应一条纪律：

1. **查询的借用只活在一条语句里**。`self.cache.borrow()` 产生的 `Ref` 在 `if let` 语句结束时释放，之后才递归。如果写成 `let mut cache = self.cache.borrow_mut();` 再递归，第二次 `borrow` 就会 panic——这正是案例 2 的机制，只是换了场景。
2. **只在写入时短暂 `borrow_mut`**。插入一行就结束，不把守卫跨过任何函数调用。
3. **不返回内部引用**。`Ref` 是局部守卫，函数返回时必须 drop，所以 `RefCell` 只能返回克隆或 `Copy` 值。需要返回 `&T` 时选 `OnceCell`：`get_or_init` 的引用指向 cell 内部，生命周期和 cell 一样长。

并发版本的缓存把 `RefCell` 换成 `Mutex`，结构不变；只写一次的键值对可以用 `OnceLock` 按 key 存，避免锁。

## 19.7 深入：`RefCell` 的借用计数只是一个 `isize`

标准库里 `RefCell` 的定义（略去调试用的字段，`BorrowCounter` 就是 `isize` 的别名）：

```rust,noplayground
pub struct RefCell<T: ?Sized> {
    borrow: Cell<isize>,
    value: UnsafeCell<T>,
}
```

所有借用状态压在这一个整数上：

```text
borrow =  0     没有借用
borrow =  n     n 个共享借用（n > 0，每个 Ref 加一）
borrow = -1     一个独占借用（RefMut）
borrow = -n     n 个互不重叠的独占借用（RefMut::map_split 拆出来的）
```

`try_borrow_mut` 和 `try_borrow` 的全部逻辑就是对这个整数的读改写：

```text
try_borrow_mut：读到 0 就写入 -1；否则返回 Err
try_borrow    ：读到 >= 0 就加一；读到负数返回 Err
```

释放发生在守卫的 `Drop` 里：`Ref::drop` 减一，`RefMut::drop` 把 -1 加回 0。所以"借用期"就是"守卫的生命周期"，没有别的机制。

这个实现带来三个可观察的后果。

**一、它不是原子操作。** 两个线程同时对 0 执行"读到 0，写入 -1"，都会成功，同一块内存于是出现两个 `&mut T`——未定义行为。`!Sync` 是唯一挡在中间的东西：安全代码里根本拿不到跨线程的 `&RefCell<T>`。

**二、`mem::forget` 守卫会永久偏移计数。** 借用不释放，之后每次 `borrow_mut` 都失败。这不是 UB，只是这个 cell 从此只能读。

**三、panic 展开时计数会恢复。** 守卫随栈展开被 drop，所以 `catch_unwind` 之后 `RefCell` 仍然可用。`RefCell` 没有 `Mutex` 那种毒化状态：panic 的来源是你自己的逻辑，不是数据被写坏。

内存布局上，64 位平台的 `RefCell<u32>` 是 16 字节：

```text
+----------------+----------------+------------+
| borrow: isize  | value: u32     | padding    |
| 8 字节          | 4 字节          | 4 字节      |
+----------------+----------------+------------+
```

字段顺序不保证（`repr(Rust)` 允许重排），这是标准库当前的声明顺序。

`Cell<T>` 连这个整数都没有。它是 `#[repr(transparent)]` 包着一个 `UnsafeCell<T>`，所以 `Cell<u32>` 就是 4 字节，`get`/`set` 编译成一次普通的内存读写，不可能 panic，也没有任何计数开销。代价是拿不到 `&T`，只能整体进出。

### 为什么 `Cell` 是 `Send` 却不是 `Sync`

`Send` 说的是"值可以移动到另一个线程"。移动之后原线程不再持有它，任何时刻只有一个线程能访问，所以非原子的读写是安全的。

`Sync` 说的是"`&T` 可以送到另一个线程"。这时候两个线程可能同时 `set`，而 `set` 就是一句非原子的写——数据竞争。编译器不会替你插入原子指令，于是只能拒绝。

```rust
use std::cell::{Cell, RefCell};

fn assert_send<T: Send>() {}

fn main() {
    assert_send::<Cell<i32>>();
    assert_send::<RefCell<i32>>();
    println!("Cell 和 RefCell 都能移动到别的线程");
}
```

把 `assert_send` 换成 `assert_sync`，两行都会变成 E0277。

标准库在 `Cell` 上写了显式的 `impl<T: ?Sized> !Sync for Cell<T> {}`。源码注释说明：`Cell` 包着 `UnsafeCell`，本来就不是 `Sync`，单独写这一行是为了错误信息更清楚——rustc 会直接告诉你 `Cell<i32>` 不满足 `Sync`，而不是让你顺着一串 `UnsafeCell` 嵌套去猜。

## 典型错误案例

下面五个案例都来自真实编译或运行输出。前两个是编译期拒绝，第三个是运行期返回失败值，后两个一个把单线程类型塞进多线程，一个是跨线程的借用计数。

### 案例 1：把 `&Cell` 交给第二个线程——E0277

`Cell` 不是 `Sync`，所以 `&Cell<T>` 不能跨线程共享。`thread::scope` 的闭包会自动借用外部变量，这个错误很容易在毫不知情时踩中。

```rust,editable
use std::cell::Cell;
use std::thread;

fn main() {
    let counter = Cell::new(0);
    thread::scope(|s| {
        s.spawn(|| counter.set(counter.get() + 1));
        s.spawn(|| println!("{}", counter.get()));
    });
    println!("{}", counter.get());
}
```

```text
error[E0277]: `Cell<i32>` cannot be shared between threads safely
   --> src/main.rs:7:17
    |
  7 |         s.spawn(|| counter.set(counter.get() + 1));
    |           ----- ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `Cell<i32>` cannot be shared between threads safely
    |           |
    |           required by a bound introduced by this call
    |
    = help: the trait `Sync` is not implemented for `Cell<i32>`
    = note: if you want to do aliasing and mutation between multiple threads, use `std::sync::RwLock` or `std::sync::atomic::AtomicI32` instead
    = note: required for `&Cell<i32>` to implement `Send`
note: required because `counter` is used within this closure
   --> src/main.rs:7:20
    |
  7 |         s.spawn(|| counter.set(counter.get() + 1));
    |                    ^^^^^^^
```

**原因**：闭包按引用捕获 `counter`，闭包要满足 `Send`，于是 `&Cell<i32>` 也必须 `Send`。而 `&T: Send` 的前提是 `T: Sync`，`Cell<i32>` 没有实现 `Sync`，链条就断在这里。编译器甚至直接给了替代品：`AtomicI32` 或 `RwLock`。

**修复**：计数用原子类型。原子操作在硬件层面保证不撕裂，所以 `AtomicUsize` 是 `Sync`，连 `&AtomicUsize` 都能随便共享。

```rust,editable
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

fn main() {
    let counter = AtomicUsize::new(0);
    thread::scope(|s| {
        s.spawn(|| { counter.fetch_add(1, Ordering::Relaxed); });
        s.spawn(|| { counter.fetch_add(1, Ordering::Relaxed); });
    });
    println!("{}", counter.load(Ordering::Relaxed));
}
```

输出是 `2`。注意闭包不再需要 `move`：`&AtomicUsize` 本身就可以安全共享。

### 案例 2：遍历借用还没结束就申请独占借用——运行期 panic

这是 `RefCell` 最典型的翻车方式：在迭代器还有效时修改容器。

```rust,editable
use std::cell::RefCell;

fn main() {
    let queue = RefCell::new(vec![1, 2, 3]);

    for x in queue.borrow().iter() {
        if *x < 3 {
            queue.borrow_mut().push(x + 10);
        }
    }

    println!("{:?}", queue.borrow());
}
```

```text
thread 'main' (33200) panicked at src/main.rs:8:19:
RefCell already borrowed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

**原因**：`for` 循环里的迭代器持有 `Ref`，整个循环期间 `borrow` 计数是 1。循环体里的 `borrow_mut` 看到计数非零，直接 panic。即使没有 panic，往 `Vec` 里 push 也可能让迭代器持有的缓冲区失效，所以这条规则不是形式主义。

**修复**：先把要改的内容收集出来，结束借用之后再一次性写入。

```rust,editable
use std::cell::RefCell;

fn main() {
    let queue = RefCell::new(vec![1, 2, 3]);

    let extra: Vec<i32> = queue
        .borrow()
        .iter()
        .filter(|&&x| x < 3)
        .map(|x| x + 10)
        .collect();
    queue.borrow_mut().extend(extra);

    println!("{:?}", queue.borrow());
}
```

输出 `[1, 2, 3, 11, 12]`。`collect` 结束时 `Ref` 已经释放，后面的 `borrow_mut` 才合法。这条"先读后写、两段分开"的模式能解决绝大多数 `RefCell` 借用冲突。

### 案例 3：`OnceLock::set` 调用两次——第二次静默失败

`set` 返回 `Result<(), T>`，不 panic，所以失败很容易被 `let _ =` 吞掉。

```rust,editable
use std::sync::OnceLock;

static CONFIG: OnceLock<String> = OnceLock::new();

fn main() {
    println!("{:?}", CONFIG.set(String::from("dev")));
    println!("{:?}", CONFIG.set(String::from("prod")));
    println!("{}", CONFIG.get().unwrap());
}
```

```text
Ok(())
Err("prod")
dev
```

**原因**：`OnceLock` 的语义是"第一次写入生效，之后永远只读"。第二次 `set` 失败时会把没能写进去的值原样还给你（这里是 `"prod"`），值并没有丢，但也不会覆盖。这种设计让运行期配置无法被偷偷改掉。

**修复**：用 `get_or_init` 表达"没有就初始化，有就直接用"，把"只初始化一次"交给类型系统保证。

```rust,editable
use std::sync::OnceLock;

static CONFIG: OnceLock<String> = OnceLock::new();

fn config() -> &'static str {
    CONFIG.get_or_init(|| String::from("dev"))
}

fn main() {
    println!("{}", config());
    println!("{}", config());
}
```

两次调用都输出 `dev`。如果确实需要保留 `set` 的"重复设置要报警"语义，就把返回值用 `if let Err(existing) = CONFIG.set(v)` 接住并处理，别用 `let _ =` 丢掉。

### 案例 4：`Arc<RefCell<T>>` 扔进线程池——E0277

`Arc` 让多个线程共享同一个值，但 `Arc<T>: Send` 要求 `T: Send + Sync`。`RefCell` 的借用计数是非原子的，这一条直接卡死。

```rust,editable
use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

fn main() {
    let total = Arc::new(RefCell::new(0));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let t = Arc::clone(&total);
        handles.push(thread::spawn(move || {
            *t.borrow_mut() += 1;
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    println!("{}", total.borrow());
}
```

```text
error[E0277]: `RefCell<i32>` cannot be shared between threads safely
   --> src/main.rs:11:36
    |
 11 |           handles.push(thread::spawn(move || {
    |  ______________________-------------_^
    | |                      |
    | |                      required by a bound introduced by this call
 12 | |             *t.borrow_mut() += 1;
 13 | |         }));
    | |_________^ `RefCell<i32>` cannot be shared between threads safely
    |
    = help: the trait `Sync` is not implemented for `RefCell<i32>`
    = note: if you want to do aliasing and mutation between multiple threads, use `std::sync::RwLock` instead
    = note: required for `Arc<RefCell<i32>>` to implement `Send`
note: required because `t` is used within this closure
   --> src/main.rs:12:14
    |
 12 |             *t.borrow_mut() += 1;
    |              ^
```

**原因**：`Arc<T>` 把同一份数据交给多个线程，所以 `T` 必须是 `Sync`——否则两个线程可以同时拿到 `&RefCell<i32>`，同时读到借用计数 0，同时写入 1，然后各自拿到 `&mut i32`。这不是理论风险，只是被 `!Sync` 提前拦住了。

**修复**：把 `RefCell` 换成 `Mutex`。锁保证同一时刻只有一个线程进入临界区，借用检查随之变成加锁与解锁。

```rust,editable
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let total = Arc::new(Mutex::new(0));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let t = Arc::clone(&total);
        handles.push(thread::spawn(move || {
            *t.lock().unwrap() += 1;
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    println!("{}", total.lock().unwrap());
}
```

输出 `4`。`MutexGuard` 在语句结束时释放锁，多个线程依次累加。

### 案例 5：没有 unsafe 就读写 `static mut`——E0133

`static mut` 是全局可变状态，编译器无法证明同一时刻只有一个线程在访问它。

```rust,editable
static mut COUNTER: u32 = 0;

fn main() {
    COUNTER += 1;
    println!("{}", COUNTER);
}
```

```text
error[E0133]: use of mutable static is unsafe and requires unsafe function or block
 --> src/main.rs:4:5
  |
4 |     COUNTER += 1;
  |     ^^^^^^^ use of mutable static
  |
  = note: mutable statics can be mutated by multiple threads: aliasing violations or data races will cause undefined behavior

error[E0133]: use of mutable static is unsafe and requires unsafe function or block
 --> src/main.rs:5:20
  |
5 |     println!("{}", COUNTER);
  |                    ^^^^^^^ use of mutable static
  |
  = note: mutable statics can be mutated by multiple threads: aliasing violations or data races will cause undefined behavior
```

**原因**：读取和写入都算"使用可变静态"，各自要一个 `unsafe` 块。编译器把责任交给你：你得自己证明没有别的线程同时在碰它。而 edition 2024 把 `static_mut_refs` 提升为 deny，连 `&mut COUNTER` 这种引用都拿不到，靠 `unsafe` 也补不回来——这说明 `static mut` 在现代 Rust 里基本不该出现。

**修复**：换成原子类型，读写都不需要 `unsafe`，也不会有数据竞争。

```rust,editable
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn main() {
    COUNTER.fetch_add(1, Ordering::Relaxed);
    println!("{}", COUNTER.load(Ordering::Relaxed));
}
```

输出 `1`。如果全局状态是一个结构体而不只是一个整数，用 `static CONFIG: OnceLock<Config>` 或 `static CACHE: Mutex<HashMap<u32, u32>>`，同样不需要 `unsafe`。

## 动手练习

1. **限流闭包**：用 `Cell<usize>` 写一个 `fn limiter(max: usize) -> impl Fn() -> Option<usize>`，每次调用返回序号 `Some(1)`、`Some(2)`，超过 `max` 次后返回 `None`。要求返回的闭包满足 `Fn`，能被 `&self` 方法调用，并与 `FnMut` 版本对比调用点的差异。

2. **给缓存加清理**：19.6 的 `Fib` 加一个 `fn clear(&self)`。先写出 `let cache = self.cache.borrow_mut();` 之后又调用 `self.get(n)` 的版本，观察 panic 的触发点，再改成不跨语句持有守卫的写法。

3. **只读一次的配置**：定义 `static LIMIT: OnceLock<usize>` 和 `fn limit() -> usize`，从环境变量 `APP_LIMIT` 读取，解析失败或缺失时用 100。启动三个线程同时调用 `limit()`，说明为什么环境变量只会被读一次，以及 `get_or_init` 里放 `panic!` 会发生什么。

4. **静态查找表**：用 `LazyLock<HashMap<u8, u32>>` 建一张 `0..=9` 的平方表，再用 `thread_local!` 记录每个线程查表的次数。主线程查三次，另起两个线程各查一次，打印四个计数，确认线程之间互不干扰。

5. **改错**：下面这段代码有三处问题，指出编译器会先报哪一处，然后改成无锁版本。

   ```rust,noplayground
   use std::cell::Cell;
   use std::sync::Arc;
   use std::thread;

   fn main() {
       let hits = Arc::new(Cell::new(0u32));
       for _ in 0..2 {
           let h = Arc::clone(&hits);
           thread::spawn(move || h.set(h.get() + 1)).join().unwrap();
       }
       println!("{}", hits.get());
   }
   ```

## 小结

- `&` 默认只读，唯一的例外是 `UnsafeCell<T>`。它用 `!Sync` 把"保证不冲突"的责任交给使用者，其余所有内部可变性类型都是它的包装。
- `Cell<T>` 整体进出、不 panic、不提供 `&T`；`RefCell<T>` 提供引用，用运行期计数换编译期检查，冲突时 panic。
- 选型顺序：单线程且只需要读写值就用 `Cell`，需要引用就用 `RefCell`，只初始化一次就用 `OnceLock`/`OnceCell`，惰性全局就用 `LazyLock`，跨线程用 `Mutex`/`RwLock`，纯计数用原子类型。
- `OnceLock::set` 只成功一次，失败时把值还给你；`get_or_init` 把"只初始化一次"写进类型，多个线程同时首次调用也只有一个闭包会执行。
- `LazyLock` 从 1.80 起进入标准库，替代 `lazy_static!` 与 `once_cell::sync::Lazy`；初始化 panic 之后它会永久毒化。
- `thread_local!` 给每个线程一份独立实例，初始化是惰性的，`with` 不允许引用逃逸。
- `Fn` 的 `call` 只拿 `&self`，所以回调、缓存这类"看起来只读、实际要写"的场景必须借助内部可变性。
- `RefCell` 的借用状态就是一个 `isize`：正数是共享借用数，负数是独占借用数，panic 来自这个整数的读改写，`!Sync` 来自它不是原子操作。
- `Send` 允许移动，`Sync` 允许共享引用。`Cell` 两样只占前者，所以它能进线程但不能被两个线程同时持有。
- 借用冲突的通用解法是"先读后写、两段分开"：把需要的数据收集出来，结束共享借用，再申请独占借用。
