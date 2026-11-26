# 附录 B 标准库与常用 crate 速查

> **本章目标**：把「想做什么」映射到「用哪个类型/trait/crate」。写代码时最耗时的往往不是逻辑，而是想不起某个方法叫什么、某个能力标准库有没有、某个场景该选哪个 crate。这一章按用途组织，可以当成索引来翻。

## B.1 常用 trait 一页表

标准库里的 trait 有几百个，日常真正会用到的不到三十个。下表按用途分组。

| trait | 签名要点 | 什么时候用 | 章 |
| --- | --- | --- | --- |
| Display | fmt(&self, f) -> fmt::Result | 面向用户的文本输出，配合花括号 | 15 |
| Debug | fmt(&self, f) -> fmt::Result | 调试输出，配合问号占位符，可 derive | 8 |
| Clone | fn clone(&self) -> Self | 需要显式复制时 | 5 |
| Copy | 标记 trait，无方法 | 按位复制即可的类型，赋值不移动 | 5 |
| Default | fn default() -> Self | 需要「空值」构造时，配合结构体更新语法 | 8 |
| PartialEq | fn eq(&self, other: &Self) -> bool | 支持相等比较 | 9 |
| Eq | 标记 trait，要求全序等价 | 用作 HashMap 键的前提之一 | 12 |
| Hash | fn hash(&self, state) | 用作 HashMap/HashSet 的键 | 12 |
| PartialOrd | fn partial_cmp(&self, other) -> `Option<Ordering>` | 支持小于大于与 sort_by | 14 |
| Ord | fn cmp(&self, other) -> Ordering | 全序，支持 sort 与 BTreeMap 键 | 12 |
| From | fn from(T) -> Self | 无损转换，自动获得 Into | 2 |
| Into | fn into(self) -> T | 由 From 自动获得，少手写 | 2 |
| TryFrom | `fn try_from(T) -> Result<Self, E>` | 可能失败的转换，自动获得 TryInto | 10 |
| AsRef | fn as_ref(&self) -> &T | 参数接受多种「可当 T 用」的类型 | 7 |
| Borrow | fn borrow(&self) -> &T | HashMap 用 &str 查 String 键 | 12 |
| Deref | fn deref(&self) -> &T | 智能指针透明访问，驱动解引用强制转换 | 18 |
| Drop | fn drop(&mut self) | 离开作用域时的清理 | 20 |
| Iterator | fn next(&mut self) -> `Option<Item>` | 迭代协议，几十个适配器免费获得 | 13 |
| IntoIterator | fn into_iter(self) -> I | for 循环的底层协议 | 13 |
| FromStr | `fn from_str(&str) -> Result<Self, E>` | 支持 "x".parse() 这种调用 | 10 |
| Error | fn source(&self) -> Option<&dyn Error> | 错误类型，配合问号与 Box dyn Error | 10, 27 |
| Send | 标记 trait | 可以跨线程转移 | 21 |
| Sync | 标记 trait | 共享引用可以跨线程 | 21 |
| Fn / FnMut / FnOnce | 调用运算符重载 | 闭包作为参数时的三种约束 | 13 |
| Sized | 标记 trait，默认实现 | ?Sized 表示接受切片与 trait 对象 | 14 |
| ToOwned | fn to_owned(&self) -> Self::Owned | &str 到 String 的通用机制 | 7 |

几个容易混的点：

- **`Eq` 与 `PartialEq`**：浮点数只能 `PartialEq`（`NaN != NaN`），整数可以 `Eq`。`Eq` 是 `PartialEq` 的标记子 trait，承诺「等价关系成立」。
- **`Ord` 与 `PartialOrd`**：同理，浮点数只有 `PartialOrd`，所以 `f64` 不能直接当 `BTreeMap` 的键。
- **`From` 与 `Into`**：只实现 `From`，`Into` 由标准库的覆盖实现自动给你。反过来手写 `Into` 会失去 `From`。
- **`AsRef` 与 `Borrow`**：都提供「借用成另一种类型」的能力，但 `Borrow` 额外要求 `Hash`/`Eq`/`Ord` 行为一致，所以 `HashMap<String, V>` 可以用 `&str` 查，靠的是 `Borrow<str>`。

## B.2 常用类型与方法速查

### B.2.1 序列类型

| 类型 | 常用方法 | 备注 |
| --- | --- | --- |
| Vec | push/pop/insert/remove/len/is_empty/iter/retain/sort/dedup/split_off/drain | 连续内存，尾部增删 O(1) |
| VecDeque | push_back/push_front/pop_back/pop_front/make_contiguous | 环形缓冲，两端 O(1) |
| String | push_str/push/truncate/clear/replace/split/trim/parse/as_str | UTF-8，不可按字节索引 |
| &str | len/chars/bytes/split/lines/trim/starts_with/find/replace | 切片，长度是字节数 |
| [T] | len/iter/chunks/windows/sort/binary_search/contains | 切片方法对 Vec 与数组都可用 |
| [u8] | to_vec/escape_ascii | 与 String 互转需 from_utf8 |

`Vec::retain` 与 `Vec::drain` 是最容易被忽略的两个高效方法：前者原地按谓词删除（O(n)），后者取出一个区间并让剩下部分左移，常用于「处理完就清空」。

### B.2.2 映射与集合

| 类型 | 常用方法 | 备注 |
| --- | --- | --- |
| HashMap | insert/get/get_mut/entry/remove/contains_key/iter/keys/values | 无序，O(1)，默认 SipHash |
| BTreeMap | 同上 + range/first_key_value/last_key_value | 有序，O(log n) |
| HashSet | insert/contains/remove/union/intersection/difference | 去重 |
| BTreeSet | 同上 + range | 有序集合 |
| IndexMap | 需要保持插入顺序时 | 第三方 crate |

`entry` API 是必须掌握的一个：

```rust
use std::collections::HashMap;

fn main() {
    let text = "the quick brown fox jumps over the lazy dog the end";
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for w in text.split_whitespace() {
        *counts.entry(w).or_insert(0) += 1;
    }
    let mut pairs: Vec<_> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (w, c) in pairs.iter().take(3) {
        println!("{w}: {c}");
    }
}
```

`or_insert` / `or_insert_with` / `or_default` / `and_modify` 四个组合能覆盖九成的「先查再插」场景，避免两次哈希查找。

### B.2.3 Option 与 Result

| 方法 | 作用 |
| --- | --- |
| map / map_or / map_or_else | 变换内部值 |
| and_then | 链式返回 Option/Result 的操作（相当于扁平化 map） |
| ok_or / ok_or_else | Option 转 Result |
| unwrap_or / unwrap_or_default / unwrap_or_else | 提供兜底值 |
| filter | 条件不满足时变 None |
| take / replace | 取出并留下默认值 |
| as_ref / as_deref / as_mut | 借用内部值而不消耗 self |
| is_some_and | 判断 Some 且满足条件（1.70+） |
| transpose | Option 与 Result 嵌套互转 |

Result 额外有 `map_err`、`and`、`or_else`、`inspect`、`inspect_err`；Option 有 `get_or_insert`、`zip`、`flatten`、`xor`。

### B.2.4 智能指针与同步原语

| 类型 | 用途 | 章 |
| --- | --- | --- |
| Box | 堆分配、递归类型、trait 对象 | 18 |
| Rc / Weak | 单线程共享所有权、打破循环 | 18 |
| Arc / Weak | 多线程共享所有权 | 18, 22 |
| Cell / RefCell | 单线程内部可变性 | 19 |
| OnceLock / LazyLock | 一次性初始化 / 延迟初始化全局 | 19 |
| Mutex / RwLock | 跨线程互斥 / 读写锁 | 22 |
| AtomicUsize 等 | 无锁计数与标志 | 22 |
| Cow | 写时克隆，读多写少时省分配 | 18 |
| Pin | 保证值不被移动，异步与自引用结构需要 | 24 |

## B.3 内置宏速查

| 宏 | 用途 |
| --- | --- |
| println! / print! / eprintln! / eprint! | 格式化输出到 stdout / stderr |
| format! | 生成 String |
| write! / writeln! | 写入实现了 fmt::Write 或 io::Write 的目标 |
| vec! | 构造 Vec |
| assert! / assert_eq! / assert_ne! | 断言，失败 panic |
| debug_assert! 系列 | 只在 debug 构建里生效 |
| panic! / todo! / unimplemented! / unreachable! | 中止与占位 |
| matches! | 模式匹配返回 bool |
| dbg! | 打印表达式与值并返回它，调试神器 |
| include_str! / include_bytes! | 编译期嵌入文件内容 |
| env! / option_env! | 编译期读环境变量 |
| concat! / stringify! | 编译期拼接字面量 / 把代码变字符串 |
| cfg! | 编译期条件，返回 bool（不是条件编译） |

`dbg!` 值得单独记：

```rust
fn main() {
    let v = vec![1, 2, 3];
    let total: i32 = dbg!(v.iter().sum());
    assert_eq!(total, 6);
}
```

输出是 `[src/main.rs:3:22] v.iter().sum() = 6`——带文件名、行号、表达式原文和结果，而且会把值原样返回，所以可以直接插在表达式中间。它写 stderr，不会污染 stdout 的管道输出。

## B.4 标准库模块地图

`std` 的顶层模块不多，认清它们能省下大量搜索：

| 模块 | 内容 |
| --- | --- |
| std::collections | Vec、VecDeque、HashMap、BTreeMap、HashSet、BinaryHeap |
| std::env | 命令行参数、环境变量、当前目录 |
| std::fs | File、DirEntry、read/write/create_dir_all |
| std::io | Read/Write/BufRead/Seek、BufReader/BufWriter、stdin/stdout/stderr |
| std::net | TcpListener/TcpStream/UdpSocket、SocketAddr |
| std::path | Path/PathBuf、组件迭代 |
| std::process | Command、Child、ExitCode |
| std::sync | Arc、Mutex、RwLock、mpsc、OnceLock、Barrier、Condvar |
| std::sync::atomic | Atomic 类型与 Ordering |
| std::thread | spawn、scope、JoinHandle、sleep、available_parallelism |
| std::time | Duration、Instant、SystemTime |
| std::fmt | Display/Debug/Write、Formatter |
| std::mem | size_of、align_of、swap、take、replace、forget、MaybeUninit |
| std::ptr | null、null_mut、addr_of、read/write、copy |
| std::str | from_utf8、Split、pattern |
| std::num | NonZero 系列、Wrapping、Saturating、IntErrorKind |
| std::cmp | Ordering、min/max、Reverse |
| std::ops | Add/Sub/Deref/Index/Range 等运算符 |
| std::marker | Sized、Send、Sync、PhantomData、Copy |
| std::error | Error trait |

两个很少被用到但很有用的入口：`std::mem::take` 与 `std::mem::replace` 可以在不克隆的前提下把值从 `&mut` 里换出来；`std::cmp::Reverse` 让 `BinaryHeap` 变成小顶堆。

## B.5 常用 crate 按场景速查

### B.5.1 序列化与配置

| crate | 用途 | 备注 |
| --- | --- | --- |
| serde | 序列化框架本体 | 只提供 trait 与 derive |
| serde_json | JSON | 最常用，错误类型设计值得学 |
| toml | TOML 配置 | 支持保留格式的编辑 |
| serde_yaml | YAML | 已被官方归档，新项目看 serde_yml/serde_norway |
| bincode / postcard | 二进制紧凑格式 | 嵌入式常用 postcard |
| csv | CSV 读写 | 与 serde 集成 |

### B.5.2 错误处理

| crate | 用途 | 备注 |
| --- | --- | --- |
| thiserror | 为枚举自动实现 Error | 库首选，零运行期成本 |
| anyhow | 动态错误 + 上下文 | 应用首选，Error::context |
| eyre / color-eyre | anyhow 的增强版 | 漂亮的报告输出 |

### B.5.3 异步与并发

| crate | 用途 | 备注 |
| --- | --- | --- |
| tokio | 异步运行时 | 事实标准，特性开关多 |
| futures | Future 组合子 | Stream、join_all、select |
| async-trait | 在 trait 里写 async fn | 有分配开销，能用关联类型就别用 |
| crossbeam-channel | MPMC 通道 | 比 std mpsc 灵活 |
| rayon | 数据并行 | 把 iter 换成 par_iter |
| parking_lot | 更快的锁 | 无中毒、体积小 |
| dashmap | 并发 HashMap | 分片锁 |
| once_cell | 延迟初始化 | 标准库已有 OnceLock/LazyLock |

### B.5.4 网络与服务

| crate | 用途 | 备注 |
| --- | --- | --- |
| reqwest | HTTP 客户端 | 同步与异步两套 API |
| hyper | HTTP 底层 | 框架都建在它上面 |
| axum | Web 框架 | 与 tower 生态集成好 |
| actix-web | Web 框架 | 性能强，自带 actor 风格 |
| tower / tower-http | 中间件抽象 | 超时、限流、追踪 |
| tonic | gRPC | 基于 prost |

### B.5.5 命令行与终端

| crate | 用途 | 备注 |
| --- | --- | --- |
| clap | 参数解析 | derive 模式最省事 |
| argh | 轻量参数解析 | 编译快，功能少 |
| crossterm | 跨平台终端控制 | 贪吃蛇那类程序的基础 |
| ratatui | TUI 框架 | 基于 crossterm/termion |
| indicatif | 进度条 | 多进度条与模板 |
| console | 终端样式与宽度检测 | indicatif 的依赖 |

### B.5.6 时间、随机与文本

| crate | 用途 | 备注 |
| --- | --- | --- |
| chrono | 日期时间 | 功能全，注意本地时区处理 |
| time | 日期时间 | 更严格，编译期格式串检查 |
| rand | 随机数 | 0.9 起 API 有调整 |
| regex | 正则 | 编译一次复用，别在循环里编译 |
| uuid | UUID | v4 随机、v7 时间有序 |
| base64 / hex | 编解码 | 简单直接 |
| url | URL 解析 | 与 serde 集成 |
| glob / walkdir | 路径匹配与遍历 | 递归列目录用 walkdir |

### B.5.7 测试与工具

| crate | 用途 | 备注 |
| --- | --- | --- |
| tempfile | 临时文件与目录 | 自动清理，测试必备 |
| assert_cmd | 测试命令行程序 | 配合 env!("CARGO_BIN_EXE_x") |
| predicates | 断言输出内容 | 与 assert_cmd 搭配 |
| proptest / quickcheck | 属性测试 | 自动生成输入找反例 |
| criterion | 基准测试 | stable 上做统计化基准 |
| insta | 快照测试 | 输出对不上时生成 diff |
| cargo-expand | 查看宏展开 | 调试宏必装 |
| cargo-nextest | 更快的测试运行器 | 大型测试套件提速明显 |

### B.5.8 数据与系统

| crate | 用途 | 备注 |
| --- | --- | --- |
| rusqlite / sqlx | SQLite / 异步 SQL | sqlx 编译期校验 SQL |
| reqwest + serde | 抓取并解析 API | 最常见的组合 |
| image | 图像编解码 | 纯 Rust 实现多 |
| notify | 文件系统事件 | 跨平台 |
| sysinfo | 系统信息 | CPU、内存、进程 |

## B.6 怎么判断一个 crate 能不能用

挑 crate 有一条可操作的清单，按顺序检查：

1. **看最近更新时间与发版频率。** 两年没更新、issue 里堆着未回复的 bug，谨慎使用。
2. **看依赖树大小。** `cargo tree -p that-crate --depth 1` 数一数直接依赖；一个「小工具」拖进 40 个依赖是危险信号。
3. **看是否使用 unsafe。** 文档里搜 unsafe、或看 crate 是否有 `#![forbid(unsafe_code)]`。安全敏感的库（解析器、加密）要更严格。
4. **看 MSRV 策略。** 有没有明确的最低 Rust 版本，是否频繁提高 MSRV 逼你升级编译器。
5. **看 API 是否用类型表达约束。** 好的库把「必须先做什么」编译进类型（比如 `Builder` 的 typestate），差的库靠文档提醒你别忘了初始化。
6. **看错误类型。** 全是 `Box<dyn Error>` 或 `String` 的库，调用方很难针对性处理；有具体错误枚举加 `source()` 链的库值得信任。
7. **看 license。** MIT/Apache-2.0 双许可是生态惯例；GPL 类许可会影响你的分发方式。
8. **能不能只开需要的 feature？** 默认打开一堆重型依赖的库，会让你的编译时间长期受拖累。

一个现实的补充：**标准库能解决的事不要引 crate**。解析命令行参数、读配置文件、发 HTTP 请求确实需要第三方库；但字符串处理、集合操作、简单的日期计算往往标准库够用。每引一个依赖，都是把一部分构建风险与升级维护成本转交给别人。

## B.7 小结

- 常用 trait 不到三十个，掌握它们的语义分野（Eq/PartialEq、Ord/PartialOrd、From/Into、AsRef/Borrow）比背方法名重要。
- `entry` API、`retain`、`drain`、`mem::take` 是四个能显著简化代码的「非基础」方法。
- `dbg!` 写 stderr 并返回原值，是调试表达式最省事的手段。
- 认标准库的模块地图，能避免「这功能标准库有没有」的反复搜索。
- 错误处理：库用 thiserror，应用用 anyhow，这条分界线覆盖绝大多数项目。
- 异步生态基本被 tokio 统一；能用标准库线程解决的事不要为了异步而异步。
- 选 crate 要看更新频率、依赖树、unsafe 用量、错误类型设计与 license 五项，别只看下载量。
- 能用标准库解决就不要引依赖；每多一个依赖，构建时间与升级风险都会增加。
- 本附录只是索引。真正理解某个 API 的行为边界，仍然要回到正文对应章节去看它为什么这样设计。