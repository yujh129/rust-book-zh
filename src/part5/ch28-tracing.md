# 第 28 章 可观测性：log 与 tracing

> **本章目标**
>
> - 说清 `println!` 在生产环境失效的五个原因，能据此判断一段日志够不够用
> - 会用 `log` 门面加 `env_logger`，读懂 `RUST_LOG` 的匹配语法，知道静态最大级别如何把日志编译掉
> - 理解 tracing 的 span / event / subscriber 三层模型，能画出一次请求的 span 树
> - 会用 `#[instrument]`、`%` 与 `?` 两种字段记法、`in_scope` 与 `Instrument`，让 span 在 `.await` 前后都不跑偏
> - 会组合 `tracing-subscriber` 的 fmt 层与 `EnvFilter`，并用 `reload` 在运行期改级别
> - 能设计一条贯穿 `request_id`、带耗时与错误链的结构化日志

程序在开发机上跑得通，不等于上线之后还看得见它在干什么。开发时你开着终端，出错就往上翻两屏；上线之后进程在容器里，终端不存在，你手里只有一份日志文件，或者一个日志平台。这时候「打印一下」这件事的性质变了：它不再是给人眼睛看的输出，而是**唯一的事故现场记录**。

大多数人第一次踩坑，都是把 `println!` 原样带进了生产。打印量小的时候看不出问题，流量一上来，五个毛病会同时发作，而且每一个都不是「优化一下」能补救的。

本章走两条线。先看 `log`：它是 Rust 生态里最老的日志门面（facade），几乎所有库都在用它，生态成熟、开销可控，但它只有「事件流」，没有「调用树」。再看 `tracing`：它在事件之外引入了 span，把一次请求的整棵调用树变成可查询的结构，这是过去十年里日志领域最重要的一次模型升级，也是 Rust 异步服务的事实标准。

## 28.1 println 调试为什么会在生产环境爆炸

先看一段很典型的代码。它在本地跑起来毫无问题：

```rust
fn charge(order_id: u64, amount: u64) {
    println!("开始扣款 {}", order_id);
    println!("金额 {}", amount);
    println!("扣款完成 {}", order_id);
}

fn main() {
    charge(7, 100);
    charge(8, 0);
}
```

输出整齐、信息齐全，你甚至能一眼看出第二笔金额是 0。问题出在这段代码搬到线上的那一刻。

### 缺陷一：没有级别

`println!` 输出的每一行都是同样的权重。想让「扣款失败」这种要命的行在几万行正常日志里跳出来，你只能去 `grep` 关键字；想临时把某个模块的调试信息打开，你只能改代码、重新发布。

生产环境需要的是**按级别过滤**：默认只留 `info` 以上，出问题时把某个模块调到 `debug`，事后调回去。这要求每条日志自带级别，并且在**运行时**可以按级别决定是否输出——而不是编译期写死。

### 缺陷二：没有结构

`println!("金额 {}", amount)` 产出的是一个字符串 `金额 100`。对人友好，对机器极不友好：日志平台想做「统计所有金额大于 1000 的扣款」，得写正则去抠数字，格式一改就全崩。

结构化日志（structured logging）的要求是：日志是一条**记录**，字段有名字、有类型，`amount` 就是 `amount`，不依赖它在句子里排第几。

### 缺陷三：没有上下文

一次请求会经过网关、鉴权、业务逻辑、数据库四层，每层都打印。事故复盘时你面对的是：

```text
开始扣款 7
查询用户
开始扣款 8
查询用户
扣款失败
```

这条 `扣款失败` 属于 7 还是 8？并发一高，谁都答不上来。缺的是**上下文**：这条日志属于哪个请求、哪个用户、哪个 span。要把它补上，就必须让每一层都能拿到同一个标识，而不是每层各自打印。

### 缺陷四：没有采样

热点路径上的日志会被放大到不可思议的量级。一次下单 20 条日志，每秒 5000 单就是每秒 10 万行；一天 86 亿行。磁盘、网络、日志平台的账单都会先撑不住。

采样（sampling）不是「少打点日志」这么简单，它要求你能按请求、按比例、按级别分别控制，还能保证「出错的请求一定留全」。

### 缺陷五：多线程交错

`println!` 每一次调用都会锁住标准输出，所以单行不会被撕开，但**行的顺序**完全由调度器决定。下面这段两线程的代码几乎每次运行的交错都不一样：

```rust
use std::thread;
use std::time::Duration;

fn main() {
    let mut handles = Vec::new();
    for id in 0..2 {
        handles.push(thread::spawn(move || {
            for step in 0..3 {
                println!("worker {} step {}", id, step);
                thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}
```

一次真实运行的输出（第二次运行，顺序与第一次不同）：

```text
worker 0 step 0
worker 1 step 0
worker 1 step 1
worker 0 step 1
worker 1 step 2
worker 0 step 2
```

单看某一行是有序的，但「哪个 worker 在进行到第几步的时候另一个 worker 插了进来」这件事被彻底搅乱了。日志的价值在于重建时间线，交错一出现，时间线就碎了。

### 还有一笔隐藏成本

`println!` 的格式化参数在**调用点**求值，而且每次都要走一遍 `std::io::stdout()` 的锁。热点路径上，一个 `println!("{:?}", big_struct)` 能把 P99 拖高几毫秒——而这行日志在 99% 的情况下根本没人看。

理想的日志系统应该在「这条日志不会被输出」时**连参数都不算**。这一条后面会反复出现，它是 `log` 的静态最大级别和 `tracing` 的 Interest 缓存共同的设计目标。

## 28.2 log 门面：让库和实现解耦

Rust 没有内置日志。标准库只提供 `eprintln!` 和 `std::io`，剩下的交给生态。生态里最早、也是至今最通用的方案是 `log` crate，它只做一件事：定义日志的**接口**。

### 门面模式

`log` 里没有「写文件」「写终端」的代码，只有：

- 五个宏：`error!`、`warn!`、`info!`、`debug!`、`trace!`
- 一个 trait `Log`，谁实现它谁就是日志后端
- 一个全局注册点 `log::set_logger` / `log::set_boxed_logger`

库作者只依赖 `log`，在代码里写 `log::debug!`；最终的可执行程序选一个后端（`env_logger`、`flexi_logger`、`fern` 等）注册进去。这样库里不需要为「日志写到哪」做任何决定，也不会把某个后端拖进依赖树。这套做法叫**门面模式**（facade），和 `log` 同期的 `serde` 也是同一个思路：库定义数据形状，程序选序列化格式。

```rust,noplayground
use log::{debug, error, info, warn};

fn charge(order_id: u64, amount: u64) -> Result<(), String> {
    debug!(order_id, amount, "开始扣款");
    if amount == 0 {
        error!(order_id, "金额为零，拒绝扣款");
        return Err(String::from("金额为零"));
    }
    info!(order_id, amount, "扣款成功");
    Ok(())
}

fn main() {
    // 后端：读 RUST_LOG，把记录写到 stderr
    env_logger::init();

    warn!("服务启动，配置来自 config.toml");
    let _ = charge(7, 100);
    let _ = charge(8, 0);
}
```

对应的依赖（版本是本章实测用的）：

```toml
[dependencies]
log = "0.4"
env_logger = "0.11"
```

注意第二个参数位：`debug!(order_id, amount, "开始扣款")` 里的 `order_id, amount` 不是格式化参数，而是**结构化字段**。`log` 0.4 的宏支持 `字段 = 值` 和简写 `字段` 两种写法，字段会附在记录上，由后端决定怎么呈现（`env_logger` 的默认格式会把它们拼在消息后面）。

### RUST_LOG 的匹配语法

`env_logger` 读环境变量 `RUST_LOG`，语法是一串逗号分隔的指令：

```bash
RUST_LOG=info ./app                        # 全局 info 及以上
RUST_LOG=warn ./app                        # 全局只要 warn 及以上
RUST_LOG=my_app=debug ./app                # 只把 my_app 这个 target 调到 debug
RUST_LOG=warn,my_app::db=trace ./app       # 默认 warn，my_app::db 模块给到 trace
RUST_LOG=off ./app                         # 全关
```

`target` 是宏所在位置的**模块路径**，默认等于 `module_path!`，也就是 crate 名加模块层级。所以在 `my_app::db::pool` 里写的 `debug!`，target 是 `my_app::db::pool`。指令是「最长前缀匹配」：`my_app=info` 管住整个 crate，`my_app::db=trace` 只覆盖数据库子模块，后者更具体，优先。

这套语法的好处是**发布后仍然可调**：重启容器时改一个环境变量，就能把某个模块的日志开到 `trace`，不用重新编译。

### 静态最大级别：把日志编译掉

`log` 有两层级别控制，很容易混淆：

| 名称 | 何时决定 | 怎么改 |
| --- | --- | --- |
| 静态最大级别 STATIC_MAX_LEVEL | 编译期，由 Cargo feature 决定 | 改 Cargo.toml，重新编译 |
| 运行期最大级别 max_level() | 运行期，由 set_max_level 决定 | 改代码或读环境变量 |

对应的 feature 有 `max_level_off`、`max_level_error`、`max_level_warn`、`max_level_info`、`max_level_debug`、`max_level_trace`，每个还有一个 `release_max_level_*` 变体，只在 `--release` 构建时生效：

```toml
[dependencies]
log = { version = "0.4", features = ["release_max_level_info"] }
```

上面这一行等于宣布：release 构建里，`debug!` 和 `trace!` 不许存在。宏展开后的形状是

```rust,noplayground
if log::Level::Debug <= log::STATIC_MAX_LEVEL && log::Level::Debug <= log::max_level() {
    // 真正写日志的代码
}
```

`STATIC_MAX_LEVEL` 是常量，比较结果在编译期就定了。当静态级别是 `Info` 时，`Debug <= Info` 为 `false`，编译器把整个 `if` 块判成死代码删掉——**连格式化参数都不再求值**。这就是「编译期裁剪」的含义：不是运行到那一行再跳过，而是那一行根本不存在。

这带来两个后果。好的一面是热点路径上留 `debug!` 几乎不花钱。坏的一面是：`debug!` 里的表达式仍然要**通过类型检查**，写错照样编译不过；而且一旦裁剪，线上再怎么改 `RUST_LOG` 也变不出这些日志，只能重新发版。所以经验是：`debug!` 可以放心留，`trace!` 谨慎留，真正的诊断信息宁可放在 `debug!` 也不要只放在 `trace!`。

用 `log::set_max_level` 还能做运行期收敛，常见写法是启动时读环境变量：

```rust,noplayground
fn main() {
    env_logger::init();
    // env_logger 已经按 RUST_LOG 设过一次；
    // 想再压低上限（比如只留 error）可以手动覆盖
    log::set_max_level(log::LevelFilter::Error);
    log::error!("这行会输出");
    log::warn!("这行不会");
}
```

### log 的天花板

`log` 的设计非常克制：一条记录由「级别 + target + 消息 + 若干字段」构成，仅此而已。它没有「这条记录属于哪次请求」的概念，因为那需要**跨调用的状态**，而 `log` 刻意不碰状态。

于是上下文只能靠人手传：

```rust,noplayground
fn handle(request_id: &str) {
    log::info!(request_id = request_id, "收到请求");
    auth(request_id);            // 每一层都得带着这个参数
}

fn auth(request_id: &str) {
    log::info!(request_id = request_id, "鉴权通过");
    db(request_id);              // 继续往下传
}

fn db(request_id: &str) {
    log::info!(request_id = request_id, "查询用户");
}
```

三层还好，三十层就是灾难：`request_id` 会污染几乎所有函数签名，忘了传一个地方，日志就断链。而且一旦进了线程池或者 `async` 任务，你连「当前是哪个请求」都无从查起。

这就是 `tracing` 要解决的问题。

## 28.3 tracing 的三层模型：span、event、subscriber

`tracing` 把「日志」拆成三个概念，理解这三个词，整章就通了。

| 概念 | 含义 | 类比 |
| --- | --- | --- |
| event | 某一**时刻**发生的事，带级别和字段 | log 的一条记录 |
| span | 一段**时间区间**，有名字、字段、父子关系，可进入可退出 | 调用栈上的一帧 |
| subscriber | 消费 event 与 span 的后端 | log 的 Log 实现 |

关键在于 span。它不是一个字符串前缀，而是一个有生命周期的对象：创建时 `new_span`，进入时 `enter`，退出时 `exit`，结束时 `close`。subscriber 拿到这些回调，就能知道「此刻有哪些 span 是打开的、它们的嵌套关系是什么」。

### 为什么 span 能表达调用树

因为「进入」和「退出」是成对的，而当前打开的 span 构成一个栈。事件发生时，栈里从根到顶的那一串就是它的**上下文路径**。`tracing-subscriber` 的 fmt 层把这个路径直接拼在行首：

```rust,noplayground
use tracing::{info, info_span};

fn handle(id: u64) {
    let span = info_span!("handle", id);
    let _entered = span.enter();
    info!(step = "parse", "开始解析");
    let child = info_span!("db");
    let _child = child.enter();
    info!("查询完成");
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let root = info_span!("server");
    let _guard = root.enter();
    info!("listening on :8080");
    handle(42);
}
```

真实输出（每行开头是时间戳，中间是 span 路径，`d1_spans` 是 crate 名即 target）：

```console
2026-11-25T12:22:44.973081Z  INFO server: d1_spans: listening on :8080
2026-11-25T12:22:44.973276Z  INFO server:handle{id=42}: d1_spans: 开始解析 step="parse"
2026-11-25T12:22:44.973326Z  INFO server:handle{id=42}:db: d1_spans: 查询完成
```

第二行的前缀 `server:handle{id=42}:` 就是答案：这条日志发生在 `server` 里的 `handle` 里，而 `handle` 这个 span 带着字段 `id=42`。第三行多了一层 `db`，说明它在更深的嵌套里。

把它画成树：

```text
server                      (整个进程服务的生命周期)
└── handle{id=42}           (一次请求)
    └── db                  (一次数据库调用)
```

对比 28.1 里那段 `println!`：那里你只能看到「查询用户」四个字，这里你能看到它挂在哪个请求、哪个子调用下面。**这就是 span 相对 log 的全部价值**：它不是让日志更好看，而是让日志带上结构化的因果链。

### subscriber 是可替换的

span 和 event 只是**记录**，怎么处理由 subscriber 决定。同一个 `info!` 可以被：

- `fmt` 层打成人类可读的一行
- `json` 层打成一行 JSON 交给日志平台
- 一个自定义 subscriber 聚合成指标
- `EnvFilter` 在到达后端之前就丢掉

这也是 `tracing` 和 `log` 最大的架构差异：`log` 的后端只收到「记录」，看不到结构；`tracing` 的后端在 `new_span`、`enter`、`exit`、`event` 四个回调上都能插手，所以它能做采样、能算耗时、能重建树。

## 28.4 #[instrument]：把函数签名变成 span

28.3 的 span 是手写的：`info_span!` 建，`enter` 进，函数结束让守卫掉落。两三个函数还能忍受，三十个函数就开始走样——名字拼错、字段漏记、有人忘了 `enter`。异步函数更麻烦，手写的 `enter` 管不住 `.await`，28.6 会看到具体后果。

tracing 提供了属性宏 `#[instrument]`，把「按函数建 span」自动化。它来自 `tracing-attributes`，由 `tracing` 默认开启的 `attributes` feature 转出，所以 `use tracing::instrument;` 就能用。

```rust,noplayground
use tracing::instrument;

#[instrument(skip(password), fields(user = %user), err)]
fn login(user: &str, password: &str) -> Result<u64, String> {
    if password.is_empty() {
        return Err(String::from("空密码"));
    }
    Ok(user.len() as u64)
}

#[instrument(ret(level = "info"))]
fn parse(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    let _ = login("ana", "s3cret");
    let _ = login("bob", "");
    let _ = parse("42");
}
```

运行输出：

```console
$ cargo run
2026-11-25T13:36:32.814266Z ERROR login{user=bob}: error=空密码
2026-11-25T13:36:32.814349Z  INFO parse{s="42"}: return=42
```

第一行值得逐段拆开。`login` 是 span 名，默认取函数名；`{user=bob}` 是 span 字段；`error=空密码` 是 `err` 生成的事件字段，级别是 `ERROR`。`ana` 那次调用成功返回，`err` 不触发，所以没有输出——span 建了又关，fmt 层默认不打印 span 的开关，除非显式要求（28.7 会讲 `FmtSpan`）。

`skip(password)` 说明 `password` 不进 span。这不是可选的讲究：不写 `skip`，参数会按 `Debug` 全部记进去，密码、token、完整请求体都会落到日志盘上。

`fields(user = %user)` 覆盖默认记录。没有它，`user` 会按 `Debug` 记成 `user="ana"`（带引号）；有了 `%`，按 `Display` 记成 `user=ana`。两者的差别在 28.5 细说。

`#[instrument]` 的可配置项按用途分四组：

- 命名与位置：`name = "handle_order"` 改 span 名，`target = "order"` 改 target，`level = "debug"` 改 span 级别（默认 `INFO`）。
- 字段：`skip(a, b)` 跳过指定参数，`skip_all` 全跳过，`fields(k = v)` 追加或覆盖字段，`fields(k = field::Empty)` 先占位、之后再 `record`。
- 返回值：`ret` 在函数返回时打一条 `DEBUG` 事件，字段名 `return`；`ret(level = "info")` 抬高级别，`ret(display)` 改用 `Display`。
- 错误：`err` 在返回 `Err` 时打一条 `ERROR` 事件，字段名 `error`；`err(level = "warn")` 改级别，`err(Display)` 改渲染方式。

对 `async fn`，`#[instrument]` 生成的不再是普通函数体，而是被 `Instrumented` 包住的 future：每次 poll 前进入 span，poll 后退出。等待期间 span 不在任何线程上「挂着」，所以不会被别的任务蹭到，也不会因为任务换了线程而丢掉。这是它相对手写 `enter` 最重要的优势。

`#[instrument]` 覆盖不了两种场景：一是 span 的边界和函数边界不一致（比如一个循环里每次迭代一个 span），二是 span 需要跨多个函数手工传递。这两种仍然要写 `info_span!` 和 `Instrument`。

## 28.5 字段记法：% 与 ?

28.4 里出现了两种前缀：`%user` 和 `s="42"`（后者由默认的 `Debug` 记录产生）。tracing 的字段值有三条记录路径，选错就会撞上 E0277。

| 写法 | 记录方式 | 类型要求 |
| --- | --- | --- |
| 字段 = 表达式 | 直接把值交给订阅者 | 实现 Value |
| 字段 = %表达式 | 先按 Display 渲染成字符串 | 实现 Display |
| 字段 = ?表达式 | 先按 Debug 渲染成字符串 | 实现 Debug |
| 只写字段名 | 同名变量按第一条规则记录 | 实现 Value |

`Value` 是 `tracing-core` 里的对象安全记录接口（`&dyn Value`），标准库的整数、浮点、`bool`、`char`、`&str`、`String`、`&T`、`Box<T>` 以及 `dyn Error` 都实现了它。你自己的结构体不会自动实现，所以 `info!(order, "created")` 编译不过——这正是第一个错误案例。

`%` 和 `?` 是语法糖，分别等价于 `tracing::field::display(&expr)` 与 `tracing::field::debug(&expr)`。两者都只借用，不移动原值。下面这段能编译也能跑，`s` 和 `v` 在记录之后仍然可用：

```rust,noplayground
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    let s = String::from("abc");
    let v = vec![1, 2, 3];
    tracing::info!(%s, ?v, "不移动的借用");
    println!("s 还能用：{s}, v 长度={}", v.len());
}
```

```console
$ cargo run
2026-11-25T13:37:13.691279Z  INFO 不移动的借用 s=abc v=[1, 2, 3]
s 还能用：abc, v 长度=3
```

注意 `s=abc` 没有引号，`v=[1, 2, 3]` 也没有——`%` 走 `Display`，`?` 走 `Debug`，而 `Vec` 的 `Debug` 不带外层引号。字符串用 `?` 才会带引号，那是 `Debug` 给 `str` 加的行为。

### 字段先声明，后填值

span 的字段集合在创建时就固定。想事后补一个当时还不知道的值（比如数据库返回的 `request_id`），必须先占位：

```rust,noplayground
use tracing::{field, info_span};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    let span = info_span!("load", request_id = field::Empty, retries = 3u8);
    span.record("request_id", "req-7f3a");
    let _guard = span.enter();

    tracing::info!("loaded");
}
```

```console
$ cargo run
2026-11-25T13:37:15.845689Z  INFO load{retries=3 request_id="req-7f3a"}: loaded
```

`field::Empty` 声明了一个「暂时没有值」的字段，`record` 之后才填。没有声明的字段无法记录，值会被丢掉，所以占位不是可选项。`record` 的第二个参数同样遵循 Value 规则：`&str` 直接可用，结构体要加 `%` 或 `?`。

输出里 `retries=3` 在前、`request_id="req-7f3a"` 在后，是因为 `retries` 在创建时就记了，`request_id` 是后补的。字段顺序不稳定时，不要写依赖顺序的解析规则。

## 28.6 span 与 async：in_scope 与 Instrument

28.3 和 28.4 的 span 都是「进入—退出」成对的同步代码，赶上异步就不成立了。`.await` 会把执行权交还给运行时，函数体在这里切开成两半，而手写的 `span.enter()` 只对**进入的那一段**有效。

先看一段看起来没问题的代码：

```rust,noplayground
use tokio::time::{sleep, Duration};

async fn query(id: u64) {
    let span = tracing::info_span!("request", id);
    let _guard = span.enter();
    sleep(Duration::from_millis(5)).await;
    tracing::info!("查询完成");
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    let a = query(1);
    let b = query(2);
    tokio::join!(a, b);
}
```

它编译通过，跑起来的输出却是错的：

```console
$ cargo run
2026-11-25T13:36:40.907707Z  INFO request{id=1}:request{id=2}: 查询完成
2026-11-25T13:36:40.907964Z  INFO request{id=1}: 查询完成
```

`id=2` 的那条日志被挂在了 `id=1` 的 span 下面，而它自己的 `request{id=2}` 反而套在外层。这不是 fmt 层排版错，是 span 的当前值真的错了。

### 为什么会串

`enter` 做的事是：在当前线程上，把这个 span 压进「当前 span 栈」；守卫掉落时弹出来。关键在「当前线程」四个字：

- 任务 A 进入 `request{id=1}`，遇到 `.await` 挂起。挂起的路径上**没有**掉落守卫，所以条目还留在原来那个线程的栈上。
- 当前线程转去跑任务 B，B 进入 `request{id=2}`，它看到栈顶是 `request{id=1}`，于是把自己挂在 A 下面。
- 等 A 被唤醒，如果被调度到**另一个**线程，那个线程的栈里根本没有它的条目，日志就丢了上下文。

注意这里没有编译错误。`enter()` 返回的借用型守卫 `Entered<'_>` 是 `Send` 的，所以即便把任务 `tokio::spawn` 进去，编译器也不会拦你——这是最危险的一类 bug：类型检查过了，语义错了。

（`span.entered()` 返回的 `EnteredSpan` 是拥有所有权的守卫，它**不是** `Send`。拿它跨 `tokio::spawn` 会直接编译失败，这也是一种保护，但它挡不住 `join!` 这种同一运行时里的交错。）

### 正确做法：把 span 绑到 future 上

tracing 提供 `Instrument` trait，它把 span 和一个 future 组合成一个新的 future：每次 poll 前进入 span，poll 后退出。挂着的时候不进入，所以不会污染别人；被调度到哪个线程，就在哪个线程进入。

```rust,noplayground
use tokio::time::{sleep, Duration};
use tracing::Instrument;

async fn query(id: u64) {
    sleep(Duration::from_millis(5)).await;
    tracing::info!(id, "查询完成");
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    let a = query(1).instrument(tracing::info_span!("request", id = 1));
    let b = query(2).instrument(tracing::info_span!("request", id = 2));
    tokio::join!(a, b);
}
```

```console
$ cargo run
2026-11-25T13:36:38.107109Z  INFO request{id=2}: 查询完成 id=2
2026-11-25T13:36:38.107439Z  INFO request{id=1}: 查询完成 id=1
```

现在每条日志只带自己的 `request{id=N}`，即便两条日志的时间戳差了几百微秒、执行顺序相反，归属也不会弄错。28.4 的 `#[instrument]` 加在 `async fn` 上，生成的就是这种结构。

`.in_current_span()` 是同一个 trait 上的另一个方法：不新建 span，直接用**当前**那个。它适合「已经在一个 span 里，现在要把这段异步逻辑也纳入进来」的场合，比如在 `#[instrument]` 的函数体里 spawn 一个子任务。

### 同步闭包用 in_scope

如果一段逻辑是同步的，但不想为它单独写函数，可以用 `Span::in_scope`：

```rust,noplayground
let span = tracing::info_span!("compute", n = 3);
let total = span.in_scope(|| {
    tracing::info!("开始");
    (1..=3).sum::<u32>()
});
tracing::info!(total, "结束");
```

`in_scope` 接收一个闭包，进入 span、执行、退出，并原样返回闭包的结果，所以上面 `total` 是 `6`。它比手写守卫短，也杜绝了「忘了退出」。

但有一类误用值得点出来：

```rust,noplayground
// 错误：span 在闭包返回时就退出了，future 还没被 poll
let fut = span.in_scope(|| async { tracing::info!("在里面吗"); });
```

`in_scope` 的闭包返回一个 future，span 在闭包返回的瞬间就退出了；这个 future 之后被 await 时，span 早已不在。异步代码不要用 `in_scope`，用 `instrument`。

## 28.7 组装订阅者：registry、layer 与 EnvFilter

前面一直在用 `tracing_subscriber::fmt()` 和 `fmt::init()`。它们是个便捷入口：内部组合了一个 fmt 层和一个读 `RUST_LOG` 的 `EnvFilter`。真实项目往往需要更细的控制——同时打到终端和文件、给不同层配不同的过滤器、运行期换级别。这些都要自己组装。

三个词先分清：

| 组件 | 作用 |
| --- | --- |
| Registry | 只负责存储 span 数据和分发，不格式化任何东西 |
| Layer | 挂在 Registry 上的插件，加一种能力（fmt、json、过滤、采样） |
| Subscriber | 最终被安装的全局对象；Registry 加若干 Layer 之后就是一个 Subscriber |

`SubscriberExt::with` 负责叠加，`SubscriberInitExt::init`（或 `try_init`）负责安装为全局。下面是一份可以直接当模板用的组装：

```rust,noplayground
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn main() {
    // 有 RUST_LOG 就用它，没有就用 info
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_span_events(FmtSpan::CLOSE),
        )
        .init();

    let span = tracing::info_span!("job", name = "sync");
    let _guard = span.enter();
    tracing::info!("working");
}
```

```console
$ cargo run
2026-11-25T13:38:21.504797Z  INFO job{name="sync"}: working
2026-11-25T13:38:21.504929Z  INFO job{name="sync"}: close time.busy=154µs time.idle=56.6µs
```

第二行是 `FmtSpan::CLOSE` 的效果：span 结束时补一条事件，`time.busy` 是 span 内实际的执行时间，`time.idle` 是它开着但没在跑的时间。这两个数就是「这次请求耗时多少」的直接来源，比另起一个计时器省事。

`EnvFilter` 的指令比 `env_logger` 多一种：除了按 target，还能按 **span** 过滤。

```bash
RUST_LOG=info,[request]=debug ./app          # 名为 request 的 span 内部开 debug
RUST_LOG=info,[request{id=1}]=debug ./app    # 只给 id=1 的那个 request span 开 debug
```

方括号里写 span 名，花括号里按 span 字段继续收窄。第二条指令实测只放行 `id=1` 的那次，`id=2` 仍然是 `info` 级别——这对「线上盯一个特定请求」非常有用，代价是你要能事先知道 id。

### 过滤放在 registry 上还是 layer 上

`.with(filter)` 加在 `registry()` 后面，是**全局**过滤：事件在被各层看到之前就被丢掉，谁都不处理。

也可以把过滤器挂到某一层上（需要 `use tracing_subscriber::Layer;`）：

```rust,noplayground
use tracing_subscriber::Layer;

let subscriber = tracing_subscriber::registry()
    .with(
        tracing_subscriber::fmt::layer()
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
    );
```

区别是：全局过滤下，被丢掉的 span 根本不会被创建，Registry 里也没有记录；层内过滤下，span 仍然会被创建和存储，只是这一层不打印。如果同时挂了「全量存到文件」和「只把 warning 打到终端」两层，就必须用层内过滤器分别配置——用全局过滤会把文件那层的数据也一起丢掉。

### 运行期改级别：reload

发布之后最常见的需求是「不改代码、不重启，把线上级别调一下」。已经装进全局的订阅者换不掉，`tracing-subscriber` 的 `reload` 层解决这个问题：它把过滤器包成可替换的，给出一个句柄。

```rust,noplayground
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter};

fn main() {
    let filter = EnvFilter::new("info");
    let (filter_layer, handle) = reload::Layer::new(filter);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_target(false).with_ansi(false))
        .init();

    tracing::debug!("第一行：info 级别下看不见");

    // 运行期把过滤器换成 debug
    handle.modify(|f| *f = EnvFilter::new("debug")).unwrap();

    tracing::debug!("第二行：reload 之后可见");
    tracing::info!("第三行");
}
```

```console
$ cargo run
2026-11-25T13:36:43.657266Z DEBUG 第二行：reload 之后可见
2026-11-25T13:36:43.657450Z  INFO 第三行
```

`reload::Layer::new` 返回「层 + 句柄」两个值，句柄可以存进全局状态，或者交给一个 HTTP 管理接口，收到请求就改。

这里的 `modify` 为什么能立即生效？因为 tracing 的级别判断不是每次调用都问一遍订阅者，而是把结果缓存在**调用点（callsite）**上；`modify` 内部会告诉 `tracing-core` 把缓存全部作废重建。这正是 28.10 要讲的机制。

## 28.8 log 还是 tracing：取舍与开销

两套 API 都能用，选择取决于你在写什么。

| 维度 | log | tracing |
| --- | --- | --- |
| 依赖体积 | 很小 | 较大，含 tracing-core 与属性宏 |
| 上下文 | 无，只能逐层手传参数 | span 树，自动继承 |
| 异步 | 无感知 | 与 future 绑定，跨 await 不丢 |
| 过滤 | 全局级别加 target 前缀 | 级别、target、span 字段三者组合 |
| 生态 | 几乎所有库都在用 | 新库普遍支持，老库靠桥接 |
| no_std | 支持 | 核心部分支持 |

给库选型时，`log` 仍然是最稳的默认：依赖小、编译快、任何宿主都能接。应用层则应该用 `tracing`——上下文和异步这两件事，`log` 从设计上就不打算解决。

两者的桥接是现成的。`tracing-subscriber` 默认开启 `tracing-log` feature，装了 `LogTracer` 之后，依赖库里的 `log::info!` 会被转成 tracing 事件，享受同一套过滤和格式。`fmt::init()` 已经替你装了；手工组装时用 `tracing_log::LogTracer::init()`。

要注意全局只能有一个。`log::set_logger` 第二次调用返回 `Err`，`tracing::subscriber::set_global_default` 第二次调用同样返回 `Err`，而 `.init()` 内部直接 `unwrap`——重复初始化就是运行期 panic，这是第五个错误案例。测试代码里请用 `with_default` 开一个局部作用域，或者用 `try_init()` 忽略重复。

### 开销花在哪里

先给一组 release 构建下的本机实测，单位是每次调用：

| 场景 | 耗时 |
| --- | --- |
| info! 且没有任何订阅者 | 低于 1ns |
| span 的 enter 与 exit | 低于 1ns |
| info! 经 fmt 层写入 io::sink | 约 719ns |

差距全在「有没有人处理」。没有订阅者时，宏只做一次级别判断，字段表达式根本不求值。这一点可以直接验证：

```rust,noplayground
fn side() -> u64 {
    println!("SIDE-EFFECT");
    7
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_ansi(false)
        .with_target(false)
        .init();

    tracing::debug!(n = side(), "被过滤的 debug");
    tracing::info!(m = side(), "可见的 info");
    println!("enabled debug = {}", tracing::enabled!(tracing::Level::DEBUG));
}
```

```console
$ cargo run
SIDE-EFFECT
2026-11-25T13:40:36.923729Z  INFO 可见的 info m=7
enabled debug = false
```

`SIDE-EFFECT` 只打印了一次：`debug!` 被 `info` 过滤器挡掉时，`side()` 没有被调用。这不是优化技巧，而是宏展开顺序决定的——字段构造在「这条记录可能被处理」的判断之后。

但反过来，一旦有层可能处理它，参数就会求值。所以不要写 `info!(?huge_struct, "dump")` 这种把整个结构体塞进日志的代码：求值本身可能很贵，格式化则发生在层真正访问字段时，两者都是实打实的成本。

需要「先判断再算」的场合，用 `tracing::enabled!` 显式挡一层：

```rust,noplayground
if tracing::enabled!(tracing::Level::DEBUG) {
    let expensive = build_report();
    tracing::debug!(?expensive, "报告");
}
```

`enabled!` 走的是同一套 callsite 缓存，几乎不花钱，但能把 `build_report` 整个跳过。

还要留意 `#[instrument]` 的隐性成本：加在 `async fn` 上时，每次 poll 都要 enter 和 exit 一次。一个频繁被唤醒的任务，poll 次数远大于调用次数，这层开销会跟着放大——虽然单次是纳秒级，但在每秒几十万次 poll 的热点上仍值得用采样层压一压。

### 脱敏：日志一旦写出去就收不回来

结构化日志的字段会被采集、索引、长期保存。手机号、邮箱、密码、token、完整的 SQL 与连接串，一旦进了字段，删除的难度远大于当初不记。

第一道防线是**不记**。`#[instrument(skip(password, token))]` 或 `skip_all` 把敏感参数挡在 span 之外，需要定位问题时只记它们的哈希或长度。

第二道防线是**打码后再记**。有些值必须留一点信息（比如「用的是哪个 key」），这时包一层类型，把 `Debug` 改成脱敏输出：

```rust,noplayground
use std::fmt;

struct Secret<'a>(&'a str);

impl fmt::Debug for Secret<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted {} chars>", self.0.len())
    }
}

#[tracing::instrument(skip_all, fields(user = %user, token = ?token))]
fn login(user: &str, token: Secret<'_>) {
    tracing::info!("login");
}
```

`?token` 走的是自定义的 `Debug`，输出的只是长度，原值不会出现在任何一层。这比在每个调用点手写「记得别打 token」可靠，因为类型系统会替你兜底。

第三道防线才是出口过滤：在 Layer 里把字段改写或丢弃。它成本最高，也最容易漏——新加的字段默认是放行的，而前两道防线默认是拒绝的。

还有一类容易忽略的泄露源：错误值。`sqlx` 的 `Error` 的 `Display` 可能带上完整 SQL，`reqwest` 的错误可能带 URL 里的查询参数。用 `err` 记录错误之前，先确认它的 `Display` 里没有你不愿意落盘的东西。

## 28.9 深入：span 为什么能跨任务，过滤在哪一层短路

### span 跨任务，跨的是 id 不是栈

「当前 span」不是一个值，而是一个**线程局部的栈**。`enter` 往当前线程的栈里压一个 id，`exit` 弹出来。span 对象本身只持有两样东西：一个 id，和一个指向订阅者的 `Dispatch`。它是 `Clone` 的，克隆出来的副本共享同一个 id。

这就解释了两件事。第一，为什么 `Instrument` 能把 span 带过 `.await`：它把 `Span` 存进 future 的字段里，future 被移动到哪个线程，`Span` 就跟到哪个线程，poll 时在那条线程的栈上压入 id。跨任务传递的是 id，不是栈。

第二，为什么守卫必须和进入它的线程绑定。`EnteredSpan` 里的 `PhantomNotSend` 就是为此存在的，源码注释写得很直白：守卫必须在创建它的线程上掉落，否则它会在一个从未进入过这个 span 的线程上执行 exit，而那边栈顶是别人的东西。`enter()` 返回的 `Entered<'_>` 只含一个 `&Span`，自动推导出来是 `Send`，编译器不会拦你——这就是 28.6 那段交错代码能通过类型检查的原因。

### 过滤的三道闸门

一次 `info!` 调用，从宏到订阅者要过三道判断，任何一道说不，后面的都不执行。

第一道在编译期。`log` 是 `STATIC_MAX_LEVEL`，tracing 是 `tracing::level_filters::STATIC_MAX_LEVEL`，由 Cargo feature 决定。级别低于它的宏展开成空语句，连字段表达式都不存在。

第二道在调用点，靠 `Interest` 缓存。每个 `info!`、每个 `info_span!` 在编译期生成一个静态的 `Callsite` 结构；第一次执行到它时，tracing 向当前订阅者问一句「你要不要这类记录」，答案被规约成三档：

| Interest | 含义 | 后续行为 |
| --- | --- | --- |
| never | 没人要 | 直接返回，连字段都不构造 |
| sometimes | 有人可能要 | 每次调用再问一次订阅者的 enabled |
| always | 确定要 | 跳过 enabled 检查，直接发 |

多层的答案会合并：只要有一个层说 never，整体就是 never；全部说 always 才是 always；其余是 sometimes。结果缓存在那个静态 `Callsite` 上，之后每次调用只读缓存——这就是「关掉的日志几乎零成本」的全部秘密，成本是一次原子读加一个分支。

第三道在层内。`EnvFilter` 作为 Layer 时，它的 `enabled` 在事件真正分发时逐层判断，还会看当前 span 的字段（`[request{id=1}]` 就是在这一层匹配的）。到这一步 span 已经创建、事件已经构造，被丢弃只是不输出——这也是为什么层内过滤比全局过滤贵。

### 缓存什么时候失效

缓存不能永远有效：新装一个订阅者、换一个过滤器，旧答案就过时了。tracing 在两种时机重建：

- 有一个新的 `Dispatch` 成为当前订阅者（比如测试里 `with_default` 换了一个）；
- 有人显式调用 `callsite::rebuild_interest_cache()`。

28.7 的 `reload::Handle::modify` 做的就是第二件事。它在换掉过滤器之后立刻重建全部调用点的缓存，所以下一行 `debug!` 就能看见——不是那次调用特殊，而是缓存被重新问了一遍。

这也解释了为什么 `set_global_default` 之后不能再换：全局订阅者只能有一个，换就意味着所有调用点的缓存都要重建，而缓存里的 id 与订阅者实例绑定。与其支持热替换，tracing 选择让你用 `reload` 只换过滤器这一个组件。

## 28.10 设计一条贯穿请求的日志

到这里所有零件都齐了。把它们拼成一套能上线的方案，目标只有三个：给一个 `req_id` 能拉出完整时间线；失败时能看到错误链；敏感字段不落盘。

职责分三处：

- **入口**：从请求头 `x-request-id` 取 id，缺失就生成；建立根 span，放入 `req_id`、路由、用户。
- **传播**：子 span 自动继承父 span，不需要逐层传参，`#[instrument]` 或 `.instrument()` 负责接上。
- **出口**：决定格式与采样。容器里写 stdout 交给采集器，本地终端给人看。

一段完整的例子：

```rust,noplayground
use tokio::time::{sleep, Duration};
use tracing::{info, info_span, Instrument};

#[tracing::instrument]
async fn db() -> Result<(), String> {
    sleep(Duration::from_millis(2)).await;
    tracing::debug!("查询订单");
    Ok(())
}

async fn handle() -> Result<(), String> {
    info!("收到请求");
    db().await?;
    info!("完成");
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("debug")
        .with_ansi(false)
        .with_target(false)
        .init();

    let span = info_span!("request", req_id = %"req-7f3a", path = "/orders");
    handle().instrument(span).await.unwrap();
}
```

```console
$ cargo run
2026-11-25T13:42:46.643019Z  INFO request{req_id=req-7f3a path="/orders"}: 收到请求
2026-11-25T13:42:46.653972Z DEBUG request{req_id=req-7f3a path="/orders"}:db: 查询订单
2026-11-25T13:42:46.654196Z  INFO request{req_id=req-7f3a path="/orders"}: 完成
```

三行日志的归属一目了然。`db` 是 `handle` 内部的子 span，它没有接收任何 `req_id` 参数，却自动带上了父 span 的字段——这就是「上下文自动继承」。

几条落地经验：

- `req_id` 用字符串，不用自增整数：多实例部署时自增会撞号。常见做法是网关生成并透传。
- 字段命名统一成 snake_case 且语义固定（`req_id`、`user_id`、`elapsed_ms`），别一会儿 `id` 一会儿 `requestId`。日志平台按字段建索引，改名等于断档。
- 错误链要打全。`err` 与 `%e` 只记录错误的 `Display`，通常只有最外层一句话；用 `tracing::error!(error = ?e, "处理失败")`，`anyhow::Error` 的 `Debug` 会打印完整的因果链。
- 耗时优先用 `FmtSpan::CLOSE` 的 `time.busy`，或者在 span 上 `record` 一个 `elapsed_ms` 字段，不要各自起计时器。
- 采样按结果区分：成功请求按比例留，失败请求全留。入口处把采样决定写进 span 字段，再让过滤层分流；更彻底的尾部采样属于日志平台的能力。
- 容器里只写 stdout 或 stderr，不要自己写文件、自己轮转。
- 级别要有约定：`error` 是需要人介入，`warn` 是自动恢复了的异常，`info` 是状态变化，`debug` 是诊断细节，`trace` 是逐帧数据。什么都写成 `error`，等于没有级别。

## 典型错误案例

### 案例 1：字段值没有实现 Value——E0277

```rust,editable
use tracing::info;
struct Order {
    id: u64,
    amount: u64,
}
fn main() {
    let order = Order { id: 7, amount: 100 };
    info!(order, "created");
}
```

```text
error[E0277]: the trait bound `Order: Value` is not satisfied
 --> src/main.rs:8:5
  |
8 |     info!(order, "created");
  |     ^^^^^^^^^^^^^^^^^^^^^^^ unsatisfied trait bound
  |
help: the trait `Value` is not implemented for `Order`
 --> src/main.rs:2:1
  |
2 | struct Order {
  | ^^^^^^^^^^^^
  = help: the following other types implement trait `Value`:
            &'a T
            &'a mut T
            (dyn std::error::Error + 'static)
            (dyn std::error::Error + Send + 'static)
            (dyn std::error::Error + Send + Sync + 'static)
            (dyn std::error::Error + Sync + 'static)
            Arguments<'_>
            Box<T>
          and 35 others
  = note: required for the cast from `&Order` to `&dyn Value`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：简写字段 `info!(order, ...)` 等价于把这个值按原样交给订阅者，要求 `&Order` 能转成 `&dyn Value`。`Value` 只为基础类型和少数包装类型实现，自定义结构体不在其中。

**修复**：加 `?` 用 `Debug`，或加 `%` 用 `Display`；只想记其中几个字段就逐个取出来。下面三种都能编译：

```rust,editable
use tracing::info;

#[derive(Debug)]
struct Order {
    id: u64,
    amount: u64,
}

fn main() {
    let order = Order { id: 7, amount: 100 };
    info!(?order, "created");
    info!(order_id = order.id, amount = order.amount, "created");
    info!(amount_kind = %"整数", "created");
}
```

</details>

### 案例 2：`.entered()` 不是 `Span`——E0308

```rust,editable
fn main() {
    let _span: tracing::Span = tracing::info_span!("request").entered();
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:2:32
  |
2 |     let _span: tracing::Span = tracing::info_span!("request").entered();
  |                -------------   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Span`, found `EnteredSpan`
  |                |
  |                expected due to this
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`enter()` 返回借用型守卫 `Entered<'_>`，`entered()` 返回拥有所有权的守卫 `EnteredSpan`，两者都不是 `Span`。类型标注写成 `tracing::Span` 就把「守卫」和「span 句柄」搞混了。

**修复**：守卫交给编译器推导即可；`Span` 要长期保存时用 `clone` 或者 `.instrument()`。

```rust,editable
fn main() {
    let span: tracing::Span = tracing::info_span!("request");
    let _guard = span.enter();
    tracing::info!("done");
    let owned = span.clone();
    drop(owned);
}
```

</details>

### 案例 3：`with` 找不到——E0599

```rust,editable
use tracing_subscriber::fmt;
fn main() {
    let fmt_layer = fmt::layer();
    let subscriber = tracing_subscriber::registry().with(fmt_layer);
    tracing_subscriber::util::SubscriberInitExt::init(subscriber);
}
```

```text
error[E0599]: no method named `with` found for struct `Registry` in the current scope
 --> src/main.rs:4:53
  |
4 |     let subscriber = tracing_subscriber::registry().with(fmt_layer);
  |                                                     ^^^^ method not found in `Registry`
  |
 ::: tracing-subscriber-0.3.23/src/layer/mod.rs:1502:8
  |
  |     fn with<L>(self, layer: L) -> Layered<L, Self>
  |        ---- the method is available for `Registry` here
  |
  = help: items from traits can only be used if the trait is in scope
help: trait `SubscriberExt` which provides `with` is implemented but not in scope; perhaps you want to import it
  |
1 + use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`with` 不是 `Registry` 的固有方法，而是扩展 trait `SubscriberExt` 提供的组合方法；trait 不在作用域里，方法就不可见。这是 Rust 里「方法来自 trait」的常见表现，报错信息本身已经给出了提示。

**修复**：导入对应的 trait。日常写代码时直接导入 `prelude` 更省事：

```rust,editable
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() {
    let fmt_layer = fmt::layer();
    let subscriber = tracing_subscriber::registry().with(fmt_layer);
    subscriber.init();
}
```

</details>

### 案例 4：future 被移动后又使用——E0382

```rust,editable
use tracing::Instrument;
async fn work() {}
#[tokio::main]
async fn main() {
    let fut = work();
    fut.instrument(tracing::info_span!("job"));
    fut.await;
}
```

```text
error[E0382]: use of moved value: `fut`
 --> src/main.rs:7:5
  |
5 |     let fut = work();
  |         --- move occurs because `fut` has type `impl Future<Output = ()>`, which does not implement the `Copy` trait
6 |     fut.instrument(tracing::info_span!("job"));
  |     --- value moved here
7 |     fut.await;
  |     ^^^ value used here after move
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`.instrument(span)` 的签名是 `fn instrument(self, span: Span) -> Instrumented<Self>`，它按值取得 future，返回一个新的、被 span 包裹的 future。第 6 行把包装结果丢掉了，原来那个 `fut` 也已经不存在了。

**修复**：用返回值，把 `.instrument()` 接在 `.await` 前面：

```rust,editable
use tracing::Instrument;

async fn work() {}

#[tokio::main]
async fn main() {
    let fut = work().instrument(tracing::info_span!("job"));
    fut.await;
}
```

</details>

### 案例 5：全局订阅者装了两次——运行期 panic

```rust,editable
fn main() {
    let a = tracing_subscriber::fmt().finish();
    let b = tracing_subscriber::fmt().finish();
    tracing::subscriber::set_global_default(a).unwrap();
    tracing::subscriber::set_global_default(b).unwrap();
}
```

```text
thread 'main' (4032) panicked at src/main.rs:5:48:
called `Result::unwrap()` on an `Err` value: SetGlobalDefaultError("a global default trace dispatcher has already been set")
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：全局默认订阅者全进程只能设置一次，第二次调用必然返回 `Err(SetGlobalDefaultError)`。这段代码能编译，问题只在运行期出现，所以用 `unwrap` 会直接 panic。`init()` 内部同样会 panic（它把 `Err` 换成了带提示的 `expect`）。

**修复**：收敛到一处安装；测试或库代码里用 `with_default` 开临时作用域，或者用 `try_init()` 忽略重复初始化：

```rust,editable
fn main() {
    let subscriber = tracing_subscriber::fmt().finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
}

#[test]
fn scope() {
    let subscriber = tracing_subscriber::fmt().with_ansi(false).finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("只在本次作用域内生效");
    });
}
```

</details>

## 动手练习

**练习 1（把 println 换成门面）**

下面这段代码用 println 输出调试信息：

```rust
fn parse_port(s: &str) -> u16 {
    println!("parsing {}", s);
    match s.parse() {
        Ok(p) => {
            println!("parsed {}", p);
            p
        }
        Err(e) => {
            println!("failed: {}", e);
            8080
        }
    }
}

fn main() {
    let _ = parse_port("9090");
    let _ = parse_port("abc");
}
```

要求：改用 `log` 宏，失败走 `warn`，成功解析走 `debug`，入口走 `trace`；用 `env_logger` 初始化，并只用 `RUST_LOG=port=debug` 打开自己的模块。验收：默认运行没有输出，加环境变量后只看到 debug 与 warn 两行。

**练习 2（`#[instrument]` 与敏感字段）**

写一个 `fn login(user: &str, password: &str) -> Result<String, String>`，加 `#[tracing::instrument]`。要求：span 名是 `login`，`password` 不出现在 span 里，成功时记录 token 长度（`ret`），失败时记录错误（`err`）。验收：输出里只有 `user="alice"` 和 `token_len=32` 这类字段，搜不到密码明文。

**练习 3（复现 span 串线）**

用 tokio 起两个任务，各自 `let _guard = span.enter();` 然后 `sleep(Duration::from_millis(10)).await`，分别在 sleep 前后打一条 info。观察输出：两条日志的 span 前缀会互相嵌套。然后用 `.instrument(span)` 改写，让每条日志只带自己的 span。写一句话解释：为什么 `enter()` 的守卫在 await 期间会「借用」错了任务。

**练习 4（运行期改级别）**

组装 `registry().with(fmt_layer).with(EnvFilter::new("info"))`，在 `main` 里拿 `reload::Layer` 的 `Handle`，先在 info 级别打一条 `tracing::debug!`（应被过滤），再调用 `handle.modify` 把过滤器换成 `debug`，重打同一条 debug。验收：第二条出现，第一条不出现。

**练习 5（贯穿请求）**

实现 `async fn handle(req_id: &str)`，内部调用 `async fn db()`。要求：入口建 `request` span 并放入 `req_id` 与 `path`；`db` 用 `#[instrument]` 建子 span；错误统一用 `?e` 记录。验收：三行日志，第二行的前缀是 `request{req_id=... path=...}:db:`，并且不传任何参数给 `db` 也能看到 `req_id`。

## 小结

1. `println!` 在生产环境同时缺级别、结构、上下文与采样，多线程还会交错，只适合本地临时排查。
2. `log` 是门面：宏加 `Log` trait，实现由后端提供，换后端不用改调用点；`RUST_LOG` 是运行期过滤，`STATIC_MAX_LEVEL` 与 `release_max_level_*` 是编译期裁剪。
3. `log` 的记录是扁平的，没有 span；要上下文只能手动把 `request_id` 逐层传下去，这是它最大的天花板。
4. `tracing` 的三层是 event、span、subscriber：event 是瞬时事件，span 是有生命周期的上下文，subscriber 负责消费与格式化。
5. `#[instrument]` 自动建 span，`skip` 排除参数，`fields` 追加字段，`ret` 与 `err` 记录结果；用在 `async fn` 上时它生成的是被包裹的 future。
6. 字段记法：裸值要求实现 `Value`，`?` 走 `Debug`，`%` 走 `Display`；自定义类型不加修饰会得到 E0277。
7. `enter()` 与 `entered()` 的守卫都不能跨 await，跨 await 要用 `.instrument()` 或 `in_scope`，否则日志的 span 会串到别的任务上。
8. 组装用 `registry` 加多个 layer：`EnvFilter` 决定谁进来，`fmt` 决定长什么样，`reload` 让级别能在运行期修改。
9. 性能来自 callsite 的静态注册与 `Interest` 缓存：被过滤的日志在热路径上几乎零成本，改过滤器后要靠 `rebuild_interest_cache` 让缓存失效。
10. 生产设计：入口生成 `req_id`、字段命名固定、错误用 `?` 打全链路、容器只写 stdout、成功采样失败全留、敏感字段不进 span。





