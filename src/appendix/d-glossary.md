# 附录 D 术语中英对照

> **本章目标**：把 Rust 里最容易被译名和术语绊住的概念集中成一张可查的表，覆盖所有权、类型系统、并发异步、工具链四个主题，并在最后给出「怎么用这张表」的建议。

Rust 的官方文档、错误信息、crate 文档几乎全是英文，而中文社区又常常混用「借用/引用」「所有权/拥有权」这类不统一的译法。查资料时最耗时的往往不是理解概念，而是猜「这个人说的『移动』到底是不是 move」。

这份表按主题分组，每行给出：中文常用译法、英文原词、一句话解释、以及本书里详细展开它的章节。建议的用法是——读英文资料卡住时回来查词，读完一章后回来扫一遍自己刚学的词。

几个翻译上的约定，本书从头到尾保持一致：

- **move** 译作「移动」，**copy** 译作「复制」，**clone** 译作「克隆」。三者是三个不同的操作，不要都叫「拷贝」。
- **borrow** 译作「借用」，**reference** 译作「引用」。`&T` 既是引用也是借用动作的结果，看语境。
- **binding** 译作「绑定」，`let x = 1;` 是「把 1 绑定到 x」，不是「给 x 赋值」。
- **trait** 不译，直接写 trait；**implementation** 译作「实现」，`impl` 块叫「实现块」。
- **lifetime** 译作「生命周期」，**scope** 译作「作用域」。两者相关但不同：生命周期是引用有效的区间，作用域是名字可见的区间。

## D.1 所有权与借用

| 中文 | English | 解释 | 章 |
| --- | --- | --- | --- |
| 所有权 | ownership | 每个值有且只有一个所有者，所有者离开作用域时值被销毁 | 5 |
| 移动 | move | 把值的所有权转给另一个绑定，原绑定失效 | 5 |
| 复制 | copy | 按位复制，原绑定仍可用，仅限实现了 Copy 的类型 | 5 |
| 克隆 | clone | 显式深拷贝，可能分配内存，可能很贵 | 5 |
| 部分移动 | partial move | 从结构体里移走一个字段，剩余字段仍可单独使用，整体不可再用 | 5 |
| 借用 | borrow | 不取得所有权地访问一个值，通过引用实现 | 6 |
| 共享引用 | shared reference | `&T`，只读，可以同时存在多个 | 6 |
| 可变引用 | mutable reference | `&mut T`，可写，同一时刻只能有一个 | 6 |
| 别名 | aliasing | 多个引用指向同一块内存；共享引用允许，可变引用不允许与其他引用共存 | 6 |
| 悬垂引用 | dangling reference | 指向已释放内存的引用，借用检查器在编译期禁止 | 6 |
| 借用检查器 | borrow checker | 编译期分析借用合法性的组件（NLL/Polonius） | 6 |
| 非词法生命周期 | non-lexical lifetimes (NLL) | 借用的有效期按实际使用点计算，而不是按词法块 | 6 |
| 生命周期 | lifetime | 引用有效的区间，写作 'a | 17 |
| 生命周期省略 | lifetime elision | 满足三条规则时可省略标注 | 17 |
| 生命周期约束 | lifetime bound | `'a: 'b` 表示 'a 至少和 'b 一样长 | 17 |
| 静态生命周期 | static lifetime | 'static，整个程序运行期都有效 | 17 |
| 切片 | slice | 对连续序列的一段视图，`&[T]` / `&str`，胖指针 | 7, 20 |
| 胖指针 | fat pointer | 带额外元数据的指针（切片带长度，trait 对象带 vtable），16 字节 | 20 |
| 解引用强制转换 | deref coercion | `&String` 自动当 `&str` 用，由 Deref 驱动 | 18 |
| 内部可变性 | interior mutability | 通过共享引用修改内容，Cell/RefCell/Mutex | 19 |
| 临时值 | temporary value | 表达式产生的匿名值，语句结束时销毁 | 6 |
| 临时值生命周期延长 | temporary lifetime extension | `let x = &temp();` 让临时值活到 x 的作用域结束 | 6 |
| 重借用 | reborrow | `&mut *r` 把可变引用借出去一段更短的时间 | 6 |
| 别名 XOR 可变性 | aliasing XOR mutability | Rust 的核心不变式：要么多个只读，要么一个可写 | 6 |

## D.2 类型、trait 与泛型

| 中文 | English | 解释 | 章 |
| --- | --- | --- | --- |
| trait | trait | 一组方法签名的集合，类似接口，但可以有默认实现与关联类型 | 15 |
| 实现 | implementation / impl | 为类型提供 trait 要求的方法 | 15 |
| 默认方法 | default method | trait 里带函数体的方法，实现者可以覆盖 | 15 |
| 关联类型 | associated type | trait 内部的类型占位符，如 Iterator 的 Item | 16 |
| 关联常量 | associated const | trait 内部的常量 | 16 |
| 泛型 | generic | 用类型参数写一份代码适配多种类型，编译期单态化 | 14 |
| 单态化 | monomorphization | 为每个具体类型生成一份机器码，零成本但有代码膨胀 | 14, 20 |
| trait 约束 | trait bound | `T: Clone`，限制类型参数必须实现什么 | 14 |
| where 子句 | where clause | 复杂约束的书写形式 | 14 |
| trait 对象 | trait object | `dyn Trait`，运行期通过 vtable 分派 | 16 |
| 对象安全 | object safety | 一个 trait 能否用作 dyn，方法不能返回 Self、不能有泛型参数 | 16 |
| 虚函数表 | vtable | trait 对象携带的函数指针表 | 16, 20 |
| 静态分派 | static dispatch | 编译期确定调用哪个实现，可内联 | 16 |
| 动态分派 | dynamic dispatch | 运行期查 vtable，有一次间接跳转 | 16 |
| 泛型关联类型 | generic associated types (GAT) | 带类型参数的关联类型 | 16, 36 |
| 类型别名 | type alias | `type Foo = Bar;` 只是别名，不产生新类型 | 8 |
| 新类型 | newtype | `struct Meters(u32);` 用元组结构体包一层，产生真正的新类型 | 8, 37 |
| 孤儿规则 | orphan rule | 只能为「本地类型」或「本地 trait」实现，防止冲突 | 15 |
| 相干性 | coherence | 保证任意类型对某个 trait 最多只有一个实现 | 15 |
| 覆盖实现 | blanket implementation | `impl<T: Display> ToString for T`，为所有满足约束的类型实现 | 15 |
| 标记 trait | marker trait | 没有方法的 trait，如 Copy/Send/Sync | 21 |
| 自动 trait | auto trait | 由编译器按字段自动推导的 trait，如 Send/Sync | 21 |
| 否定实现 | negative impl | `impl !Send for T`，显式声明不实现（不稳定，仅标准库可用） | 21 |
| 固有方法 | inherent method | 直接写在 impl Type 里的方法，不需要 trait | 8 |
| 关联函数 | associated function | 不带 self 的 impl 内函数，如 `String::new()` | 8 |
| 方法接收者 | receiver | self / &self / &mut self / self: `Box<Self>` 等 | 8 |
| 类型推断 | type inference | 编译器根据上下文推断类型，推断不出时要求标注 | 2 |
| 类型标注 | type annotation | `let x: i32`，帮助推断或明确意图 | 2 |
| 类型转换 | type cast | `as`，数值截断/重解释，不检查溢出 | 2 |
| 强制转换 | coercion | 编译器自动做的无损转换，如 &mut T 到 &T | 18 |
| 永真类型 | never type | `!`，表示不会正常返回，如 panic! 与 loop 的 break 值 | 3 |
| 单元类型 | unit type | `()`，零大小，函数默认返回它 | 4 |
| 零大小类型 | zero-sized type (ZST) | 不占空间的类型，如 `()`、PhantomData、空数组 | 20 |
| 幽灵类型 | PhantomData | 不占空间但参与类型检查的标记 | 20, 37 |
| 枚举判别式 | discriminant | 枚举内部标记当前变体的整数 | 20 |
| 空指针优化 | niche optimization | 用非法位模式表示 None，使 Option<&T> 与 &T 同大小 | 20 |
| 表示 | representation | repr(Rust) 默认可重排，repr(C) 固定布局 | 20 |
| 对齐 | alignment | 类型地址必须是 alignment 的倍数 | 20 |
| 填充 | padding | 为了对齐插入的无用字节 | 20 |
| 未定义行为 | undefined behavior (UB) | 编译器可以做任何事的状态，unsafe 代码必须避免 | 35 |
| 不安全 | unsafe | 允许解引用裸指针、调用 unsafe fn、访问 union 等 | 35 |
| 安全抽象 | safe abstraction | 用 unsafe 实现、对外暴露安全 API 的封装 | 35 |
| 外部函数接口 | foreign function interface (FFI) | 与其他语言（C）互操作 | 35 |

## D.3 并发与异步

| 中文 | English | 解释 | 章 |
| --- | --- | --- | --- |
| 线程 | thread | 操作系统调度的执行单元，Rust 用 1:1 模型 | 21 |
| 派生线程 | spawn | thread::spawn 启动新线程，返回 JoinHandle | 21 |
| 汇合 | join | 等待线程结束并取回返回值 | 21 |
| 作用域线程 | scoped thread | thread::scope，允许借用局部变量且保证 join | 21 |
| 恐慌传播 | panic propagation | 子线程 panic 后 join 返回 Err，可 downcast payload | 21 |
| 数据竞争 | data race | 多线程无同步的并发读写，Rust 在安全代码里杜绝 | 21 |
| 可发送 | Send | 值可以跨线程转移所有权 | 21 |
| 可共享 | Sync | 值的共享引用可以跨线程 | 21 |
| 互斥锁 | Mutex | 同一时刻只允许一个线程访问 | 22 |
| 读写锁 | RwLock | 多读单写，读多写少时更划算 | 22 |
| 锁中毒 | poisoning | 持锁线程 panic 后锁被标记为中毒，lock() 返回 Err | 22 |
| 死锁 | deadlock | 互相等待对方持有的锁 | 22 |
| 原子类型 | atomic | 无锁的原子读写，AtomicUsize 等 | 22 |
| 内存序 | memory ordering | Relaxed/Acquire/Release/AcqRel/SeqCst，规定可见性 | 22 |
| 松散序 | Relaxed | 只保证原子性，不建立同步关系 | 22 |
| 获取-释放 | Acquire / Release | 建立 happens-before 关系的一对 | 22 |
| 顺序一致 | SeqCst | 最强也最慢，全局单一顺序 | 22 |
| 内存屏障 | memory fence | 控制重排序的底层原语 | 22 |
| 通道 | channel | 线程间发消息，mpsc 多生产者单消费者 | 23 |
| 发送端 | sender (tx) | 发送端，clone 出多个生产者 | 23 |
| 接收端 | receiver (rx) | 接收端，只能有一个 | 23 |
| 有界通道 | sync_channel / bounded | 容量满时发送方阻塞，提供背压 | 23 |
| 汇合通道 | rendezvous channel | 容量 0，发送与接收必须同时就绪 | 23 |
| 背压 | backpressure | 消费者慢时反推生产者，防止内存无限增长 | 23 |
| 消息传递 | message passing | 通过通道共享数据，而不是共享内存 | 23 |
| 共享状态 | shared state | 通过 Mutex/原子共享内存，需 Arc 包裹 | 22 |
| 原子引用计数 | Arc | 线程安全的引用计数指针 | 18, 22 |
| actor 模型 | actor model | 每个实体一个任务加一个信箱，串行处理消息 | 23, 43 |
| 异步 | async / await | 不阻塞线程的并发，编译成状态机 | 24 |
| 未来 | Future | 表示尚未完成的计算，poll 驱动 | 24 |
| 轮询 | poll | 运行时询问 Future 是否就绪 | 24 |
| 唤醒器 | Waker | Future 就绪后通知运行时的机制 | 24 |
| 执行器 | executor | 调度并 poll 多个 Future 的组件 | 24 |
| 运行时 | runtime | 执行器 + 反应器 + 定时器，如 Tokio | 25 |
| 任务 | task | 运行时调度的单位，tokio::spawn 启动 | 25 |
| 取消 | cancellation | drop 一个 Future 即取消，取消点只在 await | 25 |
| 取消安全 | cancellation safety | 被取消后不留半完成状态 | 25 |
| 阻塞操作 | blocking operation | 会卡住线程的操作，异步里要 spawn_blocking | 25 |
| 工作窃取 | work stealing | 空闲线程从其他队列偷任务 | 25 |
| 选择 | select | 同时等待多个 Future，第一个就绪的胜出 | 23, 25 |
| 汇聚 | join | 同时等待多个 Future 全部完成 | 25 |
| 反应器 | reactor | 把 IO 事件转成 Waker 通知，如 epoll | 25 |

## D.4 工具链与生态

| 中文 | English | 解释 | 章 |
| --- | --- | --- | --- |
| 包 | crate | 编译单元，库或二进制 | 1, 11 |
| 包管理器 | Cargo | 构建、依赖、测试、发布一体化工具 | 1, 附录 A |
| 清单文件 | manifest | Cargo.toml | 1, 附录 A |
| 锁文件 | lock file | Cargo.lock，固定精确版本，应用应提交 | 附录 A |
| 工作区 | workspace | 多个 crate 共享一份 Cargo.lock 与 target | 11, 附录 A |
| 特性 | feature | 编译期开关，控制可选依赖与代码 | 附录 A |
| 编译配置档 | profile | dev/release，控制优化等级与调试信息 | 1, 附录 A |
| 依赖 | dependency | Cargo.toml 的 dependencies 表 | 附录 A |
| 版本要求 | version requirement | 插入符语义，1.2.3 等价于 >=1.2.3, <2.0.0 | 附录 A |
| 语义化版本 | semantic versioning | 主版本不兼容、次版本加功能、补丁修错误 | 附录 A |
| 注册表 | registry | crates.io 或镜像源 | 附录 A |
| 镜像 | mirror | 国内加速源，如 rsproxy | 00-setup |
| 文档注释 | doc comment | /// 与 //!，可被 cargo doc 渲染 | 32 |
| 文档测试 | doctest | 文档注释里的代码块会被当测试跑 | 32 |
| 单元测试 | unit test | cfg(test) 模块里的测试 | 26 |
| 集成测试 | integration test | tests 目录下只调用公开 API 的测试 | 26 |
| 基准测试 | benchmark | 测量性能，stable 上用 criterion | 26 |
| 断言 | assertion | assert!/assert_eq!/assert_ne! | 26 |
| 契约 | contract | 前置条件、后置条件、不变量 | 26 |
| 日志 | logging | 按级别输出诊断信息，log 门面 + tracing | 28 |
| 跨度 | span | 带上下文的区间，用于链路追踪 | 28 |
| 订阅者 | subscriber | tracing 的输出端 | 28 |
| 序列化 | serialization | 结构体转字节/文本，serde 的 Serialize | 29 |
| 反序列化 | deserialization | 字节/文本转结构体，Deserialize | 29 |
| 数据格式 | data format | JSON/TOML/YAML/MessagePack 等由具体 crate 提供 | 29 |
| 命令行参数解析 | argument parsing | clap/argh 等 | 30 |
| 子命令 | subcommand | git commit 里的 commit | 30 |
| 网络服务 | web service | axum/actix-web 等框架 | 31 |
| 处理器 | handler | 处理一次请求的函数 | 31 |
| 提取器 | extractor | 从请求里取出参数/正文的抽象 | 31 |
| 中间件 | middleware | 包裹处理器的横切逻辑 | 31 |
| 持续集成 | continuous integration (CI) | 每次提交自动构建测试 | 32 |
| 静态检查 | lint | clippy 提供的检查项 | 32 |
| 格式化 | formatting | rustfmt，cargo fmt | 32 |
| 未定义行为检查 | Miri | 解释执行 MIR，检出 UB | 35 |
| 卫生性 | hygiene | 宏引入的名字不与调用处冲突 | 33, 34 |
| 过程宏 | procedural macro | 编译期执行的函数，输入输出 TokenStream | 34 |
| 派生宏 | derive macro | #[derive(Foo)] 形式 | 34 |
| 属性宏 | attribute macro | #[foo] 修饰条目 | 34 |
| 函数式宏 | function-like macro | foo!(...) 形式 | 34 |
| 声明宏 | declarative macro | macro_rules! 定义 | 33 |
| 令牌流 | token stream | 词法单元序列，宏的输入输出 | 33, 34 |

## D.5 怎么用这张表

术语表最大的价值是消除「同一个词在不同资料里叫法不同」造成的假难点。三种典型情况：

1. **同一个英文词，中文译法不统一。** 例如 *binding* 有人译「绑定」也有人译「变量声明」；*trait object* 有人译「特征对象」也有人译「trait 对象」。看到陌生的中文词先想它的英文原词，再回表里对照。
2. **同一个中文词，指不同的英文词。** 「引用」既可能是 reference（`&T`）也可能是 borrow（借用这个动作）；「拷贝」既可能是 Copy 也可能被用来描述 clone。遇到歧义时优先信任英文原文。
3. **英文词在 Rust 里被赋予了专门含义。** *move* 在 C++ 里是一个可以重载、可以「移动后仍处于有效但未指定状态」的操作；在 Rust 里移动后原绑定直接不可用，是编译错误。*object* 在 Rust 里通常指 trait object，而不是类实例。这两处如果按 C++/Java 的直觉理解，会一直困惑。

使用建议：

- **第一遍读书时不要背表。** 术语在具体代码里见过两三次自然会记住，孤立背诵效率很低。
- **查词时连同章节号一起看。** 表里的「章」列告诉你这个词在哪里被真正展开讲；查词顺手点过去，比在原处反复琢磨更快。
- **读错误信息时回表。** 相当一部分「看不懂编译器在说什么」其实是某个名词不理解（lifetime / borrow / trait bound / object safety）。把错误信息里的关键词丢进这张表，往往立刻明白问题类别。
- **不要试图给所有词找中文对应。** trait、crate、impl、async 这类词在社区里已经默认不译，硬译反而增加沟通成本。本书正文也保留了这些词。

## D.6 小结

- 术语障碍是学习 Rust 的**表面障碍**，跨过去之后就只剩所有权、生命周期、并发这三类真正的难点。
- 遇到看不懂的中文资料，先把它翻译回英文关键词，再查官方文档或 crate 文档，准确率会高很多。
- 编译器错误信息是最权威的术语老师：它用的名词（borrow、lifetime、bound、vtable）就是社区的标准用语。
- 本书各章末尾的「典型错误案例」里那些真实报错，都特意保留了英文原文，就是为了让你熟悉这些词在错误里的实际用法。