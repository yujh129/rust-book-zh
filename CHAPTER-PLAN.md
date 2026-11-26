# 章节写作计划（ch11 起）

每章一份规格：知识点、深入小节主题、五个错误码、综合作品。写作者按此展开，可增补但不删减。
格式约定见 WRITING-RULES.md；样板章 `src/part2/ch08-structs.md`。

## ch11 模块、包与 crate（src/part2/ch11-modules-crates.md）

- 知识点：`mod` 的树形组织；`pub` 与可见性（`pub(crate)`、`pub(super)`、`pub(in path)`）；`use` 与 `as` 重命名、`self`/`super`/`crate` 路径；`mod name;` 与文件/目录的对应关系（`name.rs` 或 `name/mod.rs`）；包（package）、crate、模块三者的关系；工作空间 `[workspace]` 与 `Cargo.toml` 的 `[dependencies]`、`features`
- 深入：单态化与 crate 边界、`pub` 只控制可见性不控制"能否被链接"、编译单元与增量编译、为什么 Rust 没有头文件
- 错误：E0432 unresolved import、E0433 failed to resolve（路径写错）、E0603 module is private、E0583 file not found for module、E0255 name defined multiple times
- 作品：把第 8 章的 Rectangle 拆成 `geometry::{shape, rect}` 多文件小 crate，演示 `pub use` 重导出

## ch12 常用集合（src/part2/ch12-collections.md）

- 知识点：`Vec<T>` 的增长策略与 `with_capacity`；`String` vs `&str` vs `str`、`push_str`/`format!`/`+ ` 的所有权代价；`HashMap` 的 `entry` API 与 `or_insert`；`BTreeMap`/`HashSet` 选择；`VecDeque`；迭代集合时增删导致的借用冲突
- 深入：`Vec` 的三元组（ptr/len/cap）与扩容时的 realloc、`HashMap` 的 SipHash 与哈希洪水攻击、为什么 `HashMap` 默认不是有序的、`String` 的 UTF-8 不变式与 `chars`/`bytes` 的差异
- 错误：E0502 边遍历边改（borrow of moved/immutable borrow）、E0382 use of moved value（`for s in v` 消费了 v）、E0277 `String: Index` 不支持切片索引、E0599 no method found（`push` 用在 HashMap）、E0507 cannot move out of index（`v[0]` 取值）
- 作品：词频统计器（`HashMap<&str, usize>` + `entry`），再按频次排序输出 Top N

## ch13 迭代器与闭包（src/part2/ch13-iterators-closures.md）

- 知识点：`Iterator` trait 的 `next` 与 `Item`；惰性求值；适配器 `map/filter/take/skip/chain/zip/flat_map/enumerate`；消费者 `collect/sum/fold/count/any/all/find/position`；闭包的三种捕获（`Fn`/`FnMut`/`FnOnce`）与 `move`；闭包类型无法命名、`impl Fn` 参数
- 深入：迭代器在 LLVM 下如何被优化成和 `for` 循环一样的机器码（零成本抽象）、`collect` 如何利用 `size_hint` 预分配、`Fn`/`FnMut`/`FnOnce` 的调用语法糖 `FnMut()` 实为 `FnMut<Args>`
- 错误：E0382 closure 捕获后变量被移动、E0507 cannot move out of captured variable in Fn closure（`move` 与 `FnMut` 混用）、E0277 `&mut` 与 `Fn` 不兼容、E0599 no method named `iter`（对 `&` 用错方向）、E0308 迭代器类型不一致（`if` 两个分支返回不同适配器）
- 作品：实现自己的迭代器 `Fibonacci`（`impl Iterator for Fibonacci`）与一个 `Chunks` 适配器

## ch14 泛型（src/part2/ch14-generics.md）

- 知识点：`fn largest<T: PartialOrd>(list: &[T]) -> &T`；结构体/枚举泛型；trait bound 的三种写法（`<T: Trait>`、`where`、返回位置 `impl Trait`）；多参数与默认类型参数；单态化的代价（编译时间、二进制体积）
- 深入：单态化如何生成代码（每个具体类型一份），与 Java 类型擦除、C++ 模板的对比；`impl Trait` 与 `dyn Trait` 的静态/动态分派差异、vtable 布局
- 错误：E0277 trait bound not satisfied、E0369 cannot add（未加 Add bound）、E0308 泛型参数不一致、E0282 type annotations needed（无法推断 T）、E0599 method not found on generic type（缺 bound）
- 作品：写 `Stack<T>` 与 `Pair<A, B>`，再加一个 `fn max_by_key<T, K: Ord, F: Fn(&T) -> K>`

## ch15 trait 基础（src/part2/ch15-traits-basics.md）

- 知识点：定义与实现；默认方法；trait 作为参数（`impl Trait`）与返回值；`dyn Trait` 与对象安全（object safety）；`&dyn Trait`/`Box<dyn Trait>`；trait 与泛型的组合；孤儿规则
- 深入：vtable 的实际内容（drop/size/align/方法指针）、对象安全的具体条件（无 Self: Sized 的方法、无关联常量）、`impl Trait` 返回位置的静态分派
- 错误：E0277 trait not implemented、E0038 trait cannot be made into an object、E0117 only traits defined in current crate（孤儿规则）、E0599 no method found（trait 未导入）、E0223 ambiguous associated type
- 作品：`trait Shape { fn area(&self) -> f64; fn name(&self) -> String { ... } }` + 三个实现 + `Vec<Box<dyn Shape>>` 多态绘制

## ch16 trait 进阶（src/part2/ch16-traits-advanced.md）

- 知识点：关联类型 `type Item` vs 泛型参数（为什么 `Iterator` 用关联类型）；关联常量；supertrait（`trait A: B`）；`where Self: Sized`；trait 对象与泛型方法；`impl Trait for &T`、为新类型实现外部 trait；`Deref` 强制转换
- 深入：`Deref` 的自动解引用链、`DerefMut` 与可变的坑、一致性（coherence）与 blanket impl 的冲突、`#[fundamental]`
- 错误：E0277 associated type not specified、E0220 associated type not found、E0046 not all trait items implemented、E0053 method has incompatible type for trait、E0119 conflicting implementations
- 作品：`trait Repo<T> { type Id; fn get(&self, id: Self::Id) -> Option<&T>; }` + 内存实现 + 一个 `Box<dyn Repo<...>>`

## ch17 生命周期（src/part2/ch17-lifetimes.md）

- 知识点：借用检查器的三条规则；显式标注 `<'a>` 的语法与位置；函数签名里的生命周期省略三条规则；结构体持有引用 `struct Parser<'a> { input: &'a str }`；`'static` 的真实含义；生命周期与泛型 bound `T: 'a`
- 深入：NLL（非词法生命周期）与 Polonius 现状、生命周期是"区域推理"不是"标记存活时间"、`&'a mut T` 的不变性（invariance）与协变/逆变、为什么 `fn f<'a>(x: &'a mut Vec<&'a u8>)` 会锁死
- 错误：E0106 missing lifetime specifier、E0621 explicit lifetime required、E0499 cannot borrow as mutable more than once、E0515 returning reference to local、E0597 borrowed value does not live long enough、E0308 lifetime mismatch
- 作品：写一个 `Tokenizer<'a>`，逐词切分 `&'a str` 并返回 `Vec<&'a str>`，再写 `fn longest<'a>(x: &'a str, y: &'a str) -> &'a str`

## ch18 智能指针（src/part3/ch18-smart-pointers.md）

- 知识点：`Box<T>` 的堆分配与递归类型；`Deref`/`Drop` 如何让智能指针"像引用"；`Rc<T>` 引用计数与 `Rc::clone`；`Arc<T>` 与线程安全；`Weak<T>` 打破循环引用；`Cow<'a, str>`
- 深入：`Box<T>` 的内存布局（裸指针、newtype 的 Deref）、`Rc` 的 strong/weak 计数布局与 `Rc::make_mut` 的写时复制、`Drop` 的调用顺序与 `std::mem::drop`、`Rc` 循环引用泄漏的实测
- 错误：E0072 recursive type has infinite size、E0382 use of moved value（Box 被移动）、E0596 cannot borrow as mutable（Rc 内部不可变）、E0277 `Rc<T>` cannot be sent between threads、E0505 cannot move out of `Rc`（`Rc::try_unwrap` 用法）
- 作品：用 `Rc<RefCell<Node>>` + `Weak` 实现双向链表，验证 `Weak::upgrade` 与内存释放

## ch19 内部可变性（src/part3/ch19-interior-mutability.md）

- 知识点：`Cell<T>` 的 `get/set/take`（Copy 类型）；`RefCell<T>` 的 `borrow/borrow_mut` 与运行时 panic；`Rc<RefCell<T>>` 组合；`OnceCell`/`OnceLock`/`LazyLock`；`Mutex`/`RwLock` 速览（第 22 章展开）
- 深入：`UnsafeCell` 是唯一合法突破不变性的底层原语、`RefCell` 的 borrow flag 实现与 panic 时机（运行期 vs 编译期）、`Cell` 为什么不需要运行时检查（值语义替换）
- 错误：E0502 borrow conflict（RefCell 的编译期版本对比）、运行期 `already borrowed: BorrowMutError` panic、E0596 cannot borrow as mutable（忘记 RefCell）、E0277 `Cell<T>`: Clone 要求 T: Copy、E0499 双 `borrow_mut`
- 作品：用 `Rc<RefCell<Vec<String>>>` 实现可以被多个"观察者"修改的共享日志，并演示越界借用 panic 的捕获

## ch20 内存布局与性能（src/part3/ch20-memory-layout.md）

- 知识点：栈/堆/静态区；`size_of`/`align_of`/`offset_of!`；`Vec` 与 `Box` 的间接层；缓存友好性（顺序访问 vs 指针追逐）；`enum` 的 tag 开销；`&[T]` 的胖指针（ptr+len）
- 深入：L1/L2 缓存行与预取、AoS vs SoA 的实测、`Box<[T]>` 省一层间接、`SmallVec` 思路、分配器（`#[global_allocator]`）与 `jemalloc`/`mimalloc` 的取舍、`with_capacity` 的性能差异实测
- 错误：E0507 cannot move out of index（`v[0]`）、E0499 双重可变借用（同时 `&mut v[0]` 与 `&mut v[1]`，用 `split_at_mut`）、E0596 通过 `&` 修改、E0382 大结构体被意外移动（用 `&T`）、E0277 `[T]` 大小未知（`size_of::<[T]>` 不允许）
- 作品：写 benchmark（`std::time::Instant`）对比 `Vec<Point>` 与 `(Vec<f64>, Vec<f64>)` 求和的耗时差异

## ch21 线程（src/part4/ch21-threads.md）

- 知识点：`thread::spawn` 与 `JoinHandle`；`move` 闭包搬数据；`thread::scope`（借用栈变量）；`Send`/`Sync` 的含义与自动派生；线程 panic 与 `join()` 返回 `Err`
- 深入：1:1 线程模型（OS 线程）与 `std::thread` 的实现、`Send`/`Sync` 的 auto trait 推导规则、`Rc` 为何不是 `Send`、线程栈大小与 `Builder::stack_size`
- 错误：E0373 closure may outlive（忘记 `move`）、E0277 `Rc<T>` cannot be sent between threads safely、E0499 多个线程同时可变借用（应改用 Mutex）、E0597 borrowed value does not live long enough（跨线程借用）、E0716 temporary value dropped whil
e borrowed
- 作品：并行计算 π（多个线程分片求和）+ `thread::scope` 版本对比

## ch22 共享状态与原子（src/part4/ch22-shared-state.md）

- 知识点：`Arc<Mutex<T>>` 标准组合；`Mutex` 的 `lock()` 返回 `LockResult<MutexGuard>`；死锁与锁顺序；`RwLock`；`AtomicUsize/AtomicBool` 与 `Ordering`（`Relaxed/Acquire/Release/SeqCst`）；`Arc::strong_count`
- 深入：`Ordering` 到底在约束什么（编译器重排与 CPU 重排、内存序模型）、`Mutex` 在 Linux 上的 futex 实现、锁中毒（poisoning）与 `into_inner`、伪共享实测（`#[repr(align(64))]`）
- 错误：E0277 `MutexGuard` cannot be sent between threads（guard 跨 await 被 move）、E0596 cannot borrow as mutable（忘记 `lock()`）、运行期死锁（同一线程重复 lock）、E0502 借用冲突（guard 生命周期过长导致 `lock` 阻塞自己）、E0382 use of moved value（`Arc` 忘了 clone）
- 作品：多线程计数器，对比 `Mutex<usize>`、`AtomicUsize`、每线程独立计数再合并三种方案的耗时

## ch23 通道与消息传递（src/part4/ch23-channels.md）

- 知识点：`mpsc::channel` 的 `Sender`/`Receiver`；`send`/`recv`/`try_recv`；所有权随消息转移（"不要通过共享内存通信"）；`clone` Sender 形成多生产者；`sync_channel` 背压；`crossbeam` 的 `select!`（noplayground 提一下）
- 深入：channel 内部是 `Arc<Mutex<VecDeque>>` + 条件变量的近似实现、`recv` 阻塞与 `try_recv` 轮询的取舍、`Sender` 的 `Send`/`Sync` 边界、drop 所有 Sender 后 `recv` 返回 `Err`
- 错误：E0382 use of moved value（Receiver 移进线程后主线程还想用）、E0277 `Receiver` cannot be shared、E0599 没有 `clone`（Receiver 不能 clone）、E0502 借用冲突（在循环里持有 receiver）、E0005 模式不匹配（`while let Ok(msg)`）
- 作品：生产者-消费者流水线（3 个生产者 → 1 个消费者统计词频），再用 `sync_channel` 演示背压

## ch24 async/await 基础（src/part4/ch24-async-basics.md）

- 知识点：`async fn` 返回 `impl Future<Output = T>`；`Future` 是惰性的，必须被 poll/await；`async` 块；`.await` 只能在 async 上下文；`Pin` 与自引用；`Send` 约束（跨 await 持有非 Send 值）
- 深入：`Future::poll` 与 `Context`/`Waker` 的协作式调度、状态机大小与 `async` 代码膨胀、`Pin` 为什么存在（自引用结构体的地址不能变）、`block_on` 手写一个最小执行器
- 错误：E0728 `await` is only allowed inside async、E0277 `impl Future` is not Send（跨 await 持有 Rc）、E0601 main function not found（忘记 `#[tokio::main]`）、E0708 async closure 参数、E0277 `Pin<&mut T>` 相关或 `Future` 未实现
- 作品：手写一个 4 行执行器 `block_on`（用 `std::task` 的 RawWaker），再写 async 的计数器

## ch25 Tokio 与异步实战（src/part4/ch25-tokio.md）

- 知识点：`#[tokio::main]` 与 `Runtime`；`tokio::spawn` 要求 `Send + 'static`；`tokio::time::sleep` 与 `select!`；`tokio::sync::{mpsc, Mutex, oneshot}`；`JoinHandle` 与取消；并发 vs 并行（`join!`）
- 深入：多线程运行时的 work-stealing 调度、任务不是线程（绿色线程成本）、`spawn_blocking` 的使用边界、为什么 `std::sync::Mutex` 不能跨 await 持有
- 错误：E0277 `tokio::spawn` 要求 `'static`（借用局部变量）、E0277 future is not Send（跨 await 持有 `MutexGuard`）、E0599 无 `shutdown` 方法、E0308 `join!` 返回元组类型不匹配、E0505 任务被 move 后原变量失效
- 作品：并发抓取多个 URL 的骨架（用 `tokio::time::sleep` 模拟网络，避免真实网络），带超时与取消
