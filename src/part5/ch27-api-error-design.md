# 第 27 章 错误处理与 API 设计

> **本章目标**
>
> - 手写一个只依赖标准库的错误类型，正确实现 `Display`、`Error` 与 `source`
> - 按 crate 边界划分库错误与应用错误，不让内部类型和内部细节泄进公开 API
> - 在枚举、结构体、不透明错误、`Box<dyn Error>` 与 `&dyn Error` 之间做出有依据的选择
> - 用 `From` 与 `?` 设计传播链，知道什么时候该改用 `map_err` 补上下文
> - 用 `#[non_exhaustive]` 给错误枚举留演进空间，并从签名层面减少失败的可能

第 10 章把 `Result`、`?` 和自定义错误 `enum` 的语法讲完了。会写不等于会设计：同样一个失败，有人返回 `Result<T, String>`，有人返回 `Box<dyn Error>`，有人定义一个带 `source` 链的枚举。三种写法都能编译，但调用方拿到的信息、你能做出的兼容性承诺、半年后排查问题的难度完全不同。

错误类型是 API 的一部分，而且是最容易被忽略的一部分。函数签名里的参数和返回值会被 review，错误类型却常常被随手写成 `String`。等到调用方想区分"文件不存在"和"没有权限"时，才发现字符串里除了一句给人看的话，什么结构都没有。

这一章不再重复 `?` 怎么用，而是回答三个设计问题：错误类型该长什么样，错误该在哪一层被转换，以及怎样让 API 的签名本身减少失败的可能。

## 27.1 错误类型是 API 的一部分

先看一个能跑、但不好用的版本。下面的函数读取文件里的端口号：

```rust
use std::fs;

fn load_port(path: &str) -> Result<u16, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    text.trim().parse::<u16>().map_err(|e| e.to_string())
}

fn main() {
    match load_port("port.txt") {
        Ok(port) => println!("port {port}"),
        Err(e) => println!("{e}"),
    }
}
```

它能打印系统给出的错误文本，但信息到此为止：

- 调用方拿到的是一句话，无法区分"文件不存在"（可以创建）和"内容不是数字"（该提示用户改配置）。
- `source` 链断了。`String` 没有原因可讲，日志里既看不到 `io::Error`，也看不到 `ParseIntError`。
- 错误类型无法演进。想加错误码、重试提示或字段名，只能修改字符串格式，而字符串格式没有兼容性约定。
- 测试只能断言字符串包含某段文字，改一个标点就会失败。

错误的归属跟着 crate 边界走，这条规则比任何具体类型都重要：库的公开 API 返回具体错误，应用层才用 `Box<dyn Error>` 这类不透明类型，内部的数据库驱动或 HTTP 客户端不写进公开签名。具体分层在 27.4 展开。

**三种最常见的反模式。**

**反模式一：unwrap 满天飞。** `unwrap` 是断言，不是错误处理。它表达的是"这里失败说明程序有 bug"。库代码里尤其不能用：调用方的非法输入不该让你的库 panic。确实不可能失败的地方，用 `expect` 写清假设；其余地方用 `?` 或 `ok_or`。

**反模式二：把错误转成 String 就丢信息。** `.map_err(|e| e.to_string())` 只要在上游出现一次，后面所有层都只能拿到一句话。类型、`source` 链、可匹配的变体全部消失。只有最外层的打印和日志可以压平成字符串。

**反模式三：吞掉错误。** `let _ = fs::remove_file(path);`、没有 `else` 的 `if let Ok(v) = ...`、`.ok()` 之后直接丢掉，都属于"把不想处理写成没有发生"。如果失败真的无害，写一行注释说明为什么，并至少记一条日志。

三种反模式的共同点是把错误当麻烦，而不是信息。接下来的小节从类型开始，把信息留住。

## 27.2 手写错误类型：Display、Error 与 source

库对外给出的错误类型，最少要实现三件事：`Debug` 用 `derive` 生成，给开发者和测试看；`Display` 给最终用户看；`Error` 让类型能进入 `Box<dyn Error>`，并通过 `source` 串起因果链。三者不是可选项——`Error` 的父 trait 就是 `Debug + Display`，少一个都编译不过。

下面这个例子只依赖标准库，解析 `"1500ms"`、`"3s"` 这样的时长字符串：

```rust
use std::error::Error;
use std::fmt;
use std::num::ParseIntError;
use std::time::Duration;

#[derive(Debug)]
enum DurationError {
    Empty,
    MissingUnit { input: String },
    InvalidNumber { input: String, source: ParseIntError },
    UnknownUnit { unit: String },
}

impl fmt::Display for DurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DurationError::Empty => write!(f, "duration is empty"),
            DurationError::MissingUnit { input } => write!(f, "missing unit in `{input}`"),
            DurationError::InvalidNumber { input, .. } => write!(f, "invalid number in `{input}`"),
            DurationError::UnknownUnit { unit } => write!(f, "unknown unit `{unit}`"),
        }
    }
}

impl Error for DurationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DurationError::InvalidNumber { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn parse_duration(input: &str) -> Result<Duration, DurationError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DurationError::Empty);
    }
    let split = input
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| DurationError::MissingUnit { input: input.to_string() })?;
    let (number, unit) = input.split_at(split);
    let value: u64 = number.parse().map_err(|source| DurationError::InvalidNumber {
        input: input.to_string(),
        source,
    })?;
    let millis = match unit {
        "ms" => value,
        "s" => value * 1000,
        _ => return Err(DurationError::UnknownUnit { unit: unit.to_string() }),
    };
    Ok(Duration::from_millis(millis))
}

fn main() {
    for input in ["1500ms", "3s", "12", "5m", ""] {
        match parse_duration(input) {
            Ok(d) => println!("{input} -> {d:?}"),
            Err(e) => {
                println!("{input} -> {e}");
                if let Some(source) = e.source() {
                    println!("    caused by: {source}");
                }
            }
        }
    }
}
```

输出：

```text
1500ms -> 1.5s
3s -> 3s
12 -> missing unit in `12`
5m -> unknown unit `m`
 -> duration is empty
```

这个类型有几个刻意的设计选择。

**变体携带上下文，不只携带种类。** `MissingUnit` 存了原始输入，`UnknownUnit` 存了单位，`InvalidNumber` 同时存输入和底层 `ParseIntError`。调用方打印时不需要再回头找"当时输入是什么"，错误自己带着证据。代价是每个变体更大：`String` 占三个机器字，错误值不再便宜。错误是罕见路径，这点空间通常值得。

**`Display` 只写本层。** 它说的是"这个错误是什么"，不说底层为什么失败。`InvalidNumber` 只打印 "invalid number in `abc`"，`invalid digit found in string` 交给 `source`。如果本层把内层消息也拼进去，打印完整链时同一句话会出现两次。格式上全部小写、结尾不加句号、用反引号包住用户输入，原因在 27.5 展开。

**`source` 返回借用而不是拥有。** 签名 `Option<&(dyn Error + 'static)>` 有两层含义：返回引用，所以遍历错误链不需要任何分配；`'static` 约束，所以被擦除的具体类型不能借用在栈上的临时数据。错误类型通常自己拥有字符串和数字，天然满足这个约束；如果错误里存的是 `&'a str`，它就没法做成 `Box<dyn Error>`。

**这里没有 `impl From<ParseIntError>`，是有意的。** `From` 的函数签名只能拿到 `ParseIntError`，拿不到 `input`；而 `InvalidNumber` 需要两者。硬写一个 `From` 只能把 `input` 填成空字符串，等于丢掉刚想保留的上下文。所以这里用 `map_err` 在调用点补上下文。什么样的错误适合 `From`，27.4 专门讨论。

编译这段代码时，如果只写了 `#[derive(Debug)]` 而漏掉 `Display`，会先卡在 `impl Error` 上，因为父 trait 不满足。错误案例 2 展示了缺 `Display` 时更早暴露的一种报错。

## 27.3 枚举、结构体还是不透明错误

错误类型没有唯一正确的形状，只有适合当前契约的形状。常见的有三种。

**公开枚举：把失败原因列成有限集合。** 每个变体是一种可以分支处理的情况，调用方能穷尽匹配。它适合失败集合稳定、调用方确实要按原因做不同事情的库，比如"解析失败"要区分空输入、非法字符和越界。代价是公开枚举加变体通常是破坏性变更，27.6 会给出缓解手段。

**公开结构体加原因分类。** 标准库的 `std::io::Error` 是代表：结构体本身不透明，内部可以装系统错误码或任意内层错误；调用方通过 `kind()` 拿到 `ErrorKind` 枚举来分类，通过 `source()` 看底层原因。这种"kind 加私有载荷"的模式适合内部原因很多、又不想把每一种都写进公开 API 的场景。`std::num::ParseIntError` 则更彻底，是完全不透明的结构体，除 `Display`、`Debug`、`Error` 之外几乎不承诺任何东西。

**不透明错误：只承诺能显示、能串链。** 公开一个结构体，内部字段私有，装一个 `Box<dyn Error + Send + Sync>`。调用方不能 `match`，只能在必要时 `downcast_ref` 猜具体类型。它的好处是内部实现可以随便换、随便加错误种类，都不会破坏兼容性；坏处是调用方失去了编译期分类能力。应用层的内部错误、错误分类还在快速变化的库，都适合这种形状。

| 形态 | 调用方能穷尽匹配 | 新增原因是否破坏兼容 | 适用位置 |
| --- | --- | --- | --- |
| 公开枚举 | 能 | 是，除非加 non_exhaustive | 失败集合稳定的库 |
| 公开结构体加 kind | 能按 kind 分类 | kind 加变体也要 non_exhaustive | 原因多但要分类的库 |
| 不透明结构体 | 不能 | 否 | 应用边界、内部实现常变 |

**借用还是拥有：`&dyn Error` 与 `Box<dyn Error>`。**

两种 trait 对象经常被混着用，但它们的约束完全不同。先看一个只借用、不拥有错误的报告函数：

```rust
use std::error::Error;

fn report(err: &dyn Error) {
    println!("error: {err}");
    let mut source = err.source();
    while let Some(inner) = source {
        println!("  caused by: {inner}");
        source = inner.source();
    }
}

fn main() {
    let err: Box<dyn Error> = "42x".parse::<u8>().unwrap_err().into();
    report(&*err);
}
```

输出：

```text
error: invalid digit found in string
```

`&dyn Error` 不拥有数据、不分配内存，适合日志、上报、诊断这类"看一眼就走"的函数。它的限制来自借用本身：生命周期跟着被借用的值，不能返回指向局部错误的引用（错误案例 5），也没法在借用期间给它补上下文。注意 `report(&err)` 不能直接传 `&Box<dyn Error>`，要写 `&*err` 或 `err.as_ref()`，先把盒子拆成里面的 trait 对象。

`Box<dyn Error>` 拥有错误，堆分配一次，可以从函数返回、存进结构体、跨层传递。标准库为它实现了 `From<E>`（`E: Error + 'static`），所以 `?` 能把任何标准库或自定义错误转进去。它没有实现 `Send` 和 `Sync`，因为 trait 对象默认不携带这两个自动 trait；要跨线程或进 async 任务，必须写 `Box<dyn Error + Send + Sync>`（错误案例 4）。这也是不依赖 anyhow 时最实用的应用错误形状。

| 写法 | 分配 | 能从函数返回 | 能跨线程 | 能 match 具体类型 |
| --- | --- | --- | --- | --- |
| &dyn Error | 无 | 需要生命周期 | 取决于被借用类型 | downcast |
| `Box<dyn Error>` | 一次 | 能 | 否 | downcast |
| `Box<dyn Error + Send + Sync>` | 一次 | 能 | 是 | downcast |
| 具体枚举 | 无 | 能 | 取决于字段 | 是 |

这张表里最容易被忽略的是最后一列。trait 对象把类型擦掉了，调用方只能 `downcast_ref::<T>()` 去猜；猜不中就返回 `None`，编译器帮不上忙。库的公开 API 一旦返回 `Box<dyn Error>`，就等于承诺"调用方不需要按错误种类处理"。如果这个承诺不成立，就该换成具体枚举。

## 27.4 From 与 ? 的传播链

`?` 的规则只有一条：表达式返回 `Result<T, E>`，函数返回 `Result<U, F>`，那么必须存在 `F: From<E>`。编译器把 `expr?` 展开成大致这样的代码：

```rust,noplayground
match expr {
    Ok(value) => value,
    Err(e) => return Err(From::from(e)),
}
```

同类型转换不需要你写任何东西，标准库有反身的 `impl<T> From<T> for T`。跨类型才需要 `From`。下面把 `ParseIntError` 接进自己的错误类型：

```rust
use std::error::Error;
use std::fmt;
use std::num::ParseIntError;

#[derive(Debug)]
enum ConfigError {
    NotANumber(ParseIntError),
    OutOfRange(u32),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotANumber(_) => write!(f, "port is not a number"),
            ConfigError::OutOfRange(port) => write!(f, "port {port} is out of range"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::NotANumber(source) => Some(source),
            ConfigError::OutOfRange(_) => None,
        }
    }
}

impl From<ParseIntError> for ConfigError {
    fn from(source: ParseIntError) -> Self {
        ConfigError::NotANumber(source)
    }
}

fn parse_port(input: &str) -> Result<u16, ConfigError> {
    let port: u32 = input.parse()?;
    if port == 0 || port > u32::from(u16::MAX) {
        return Err(ConfigError::OutOfRange(port));
    }
    Ok(port as u16)
}

fn main() {
    for input in ["8080", "70000", "abc"] {
        match parse_port(input) {
            Ok(port) => println!("listening on {port}"),
            Err(e) => println!("{e:?}"),
        }
    }
}
```

输出：

```text
listening on 8080
OutOfRange(70000)
NotANumber(ParseIntError { kind: InvalidDigit })
```

有了这个 `impl`，`input.parse()?` 就能自动把 `ParseIntError` 变成 `ConfigError`，函数体里没有任何手写的转换。注意 `From` 只搬运错误，不添加信息：`NotANumber` 里除了 `ParseIntError` 什么都没有，所以 `Display` 只能说"不是数字"，说不出"哪个字段不是数字"。这正是 `From` 的边界。

**`From` 与 `map_err` 的分工。** 一对一、不需要额外上下文时用 `From`，让 `?` 自动完成；需要路径、操作名、输入值时用 `map_err`，在调用点补上下文。两者不要叠加：既然写了 `impl From<ParseIntError>`，就别在调用点再 `map_err` 成同一个变体，那是重复代码。反过来，如果某个转换需要上下文，就不要为它实现 `From`，否则会诱导调用方写出丢上下文的代码。

**一个 `From` 只能有一个目标。** 同一个 `io::Error` 在你的错误类型里只能映射到一个变体；想让"读配置失败"和"写日志失败"落到不同变体，只能靠 `map_err`，或者包成两个 newtype。这不是缺陷，而是 `?` 的设计前提：它只看类型，不看调用位置。

**孤儿规则决定了谁能实现 `From`。** 你可以在自己的 crate 里为 `ConfigError` 实现 `From<ParseIntError>`，因为 `ConfigError` 是本地类型；但不能给 `Box<dyn Error>` 写 `impl From<MyError>`，因为两者都来自外部。标准库已经替 `Box<dyn Error>` 和 `Box<dyn Error + Send + Sync>` 写了覆盖所有 `E: Error` 的 `From`，所以 `?` 可以直接把自定义错误送进盒子。给外部错误加语义时，正确做法是包一层本地 newtype，再为 newtype 实现需要的 trait。

**库错误与应用错误。**

错误分层不是术语游戏，它决定谁能看到什么。

库的错误类型应该具体、可匹配、不包含展示文案之外的策略。它只回答"哪里失败了、为什么"，不决定重试、不打印日志、不退出进程。库的 `Display` 写给最终用户看，`source` 写给诊断看，两者都不该出现 "please contact support" 这种应用层话术。

应用的错误类型可以把多个库错误收进一个不透明类型，再加上下文。应用负责策略：哪些错误重试、哪些直接退出、给用户显示哪一句、日志里记多详细。常见的写法是 `type AppError = Box<dyn Error + Send + Sync>`，函数返回 `Result<T, AppError>`，在边界处统一打印 `Display`、遍历 `source` 记日志、按 `downcast_ref` 决定退出码。

内部错误不外泄，指的是公开 API 的签名里不出现内部实现细节。如果仓储层直接返回数据库驱动的错误类型，调用方就被迫依赖那个驱动，换数据库就是破坏性变更；如果错误消息里带 SQL 语句和连接串，日志和报错页面还会泄露内部结构。正确做法是在边界处转换：内部记录原始 `source` 供日志使用，对外只暴露"存储失败"这类稳定分类。错误案例 1 的反面教材就是把底层错误先格式化成 `String`，再想用 `?` 转回自己的类型——信息在格式化那一刻就已经丢了。

## 27.5 错误消息的写法

`Display` 的输出会被嵌进更大的句子、写进日志、贴到终端。它有三条约定。

**小写开头，结尾不加标点。** 标准库写 `invalid digit found in string`，不写 `Invalid digit found in string.`。因为调用方常写 `failed to load config: {e}`，如果 `{e}` 自带大写和句号，句子会断裂成两句。中文消息同理不加句号，也不要在 `Display` 里再写"错误："前缀——外层已经说明这是错误了。

**包含上下文，至少回答"对什么做了什么"。** `invalid input`、`parse failed`、`something went wrong` 都没有信息量。好的消息带动作、对象和值：

```rust,noplayground
// 避免
"Error: Failed to parse!"
"something went wrong"
"invalid input"

// 推荐
"failed to parse config `config.toml`"
"invalid port `70000`: expected 1-65535"
"failed to connect to `127.0.0.1:8080`: connection refused"
```

动态值用反引号或引号包住，用户输入里有空格时也不会歧义。值太长就截断，不要把整个请求体回显到日志里。

**区分诊断与展示。** `Display` 面向读日志的人，`Debug` 面向开发者，时间戳、级别、模块名归日志框架管。不要在 `Display` 里加 `[ERROR]`、时间戳、重试建议或表情符号。库的消息保持中立，应用层再决定对最终用户说什么。

还有一条容易忽略：**消息留在 `Display`，原因留在 `source`。** 如果 `Display` 把底层错误也拼进去，遍历错误链时同一句话会出现两遍；如果为了拼消息把底层错误转成 `String`，类型和 `source` 就都丢了。27.2 的 `InvalidNumber` 是正确分工：本层说"哪个输入不合法"，底层说"为什么"。

写 `expect` 时也一样。它的消息描述被破坏的不变量，比如 `expect("default config was validated")`，而不是 `expect("should never happen")`。`unwrap` 和 `expect` 的 panic 是开发者诊断，不是错误处理，别让它们出现在可恢复的路径上。

## 27.6 #[non_exhaustive] 与错误枚举的演进

公开枚举有一个隐蔽的兼容性问题：下游用 `match` 穷尽了所有变体，你下个版本新增一个变体，下游全部编译失败。对库来说，这等于"加错误种类"成了破坏性变更。

`#[non_exhaustive]` 就是为此设计的：

```rust
use std::fmt;

#[non_exhaustive]
#[derive(Debug)]
pub enum ApiError {
    NotFound,
    RateLimited { retry_after: u64 },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotFound => write!(f, "resource not found"),
            ApiError::RateLimited { retry_after } => {
                write!(f, "rate limited, retry after {retry_after}s")
            }
        }
    }
}

fn main() {
    println!("{}", ApiError::NotFound);
}
```

输出：

```text
resource not found
```

标记之后，**其他 crate** 的 `match` 必须带一个 `_` 臂，否则编译报错；因此你以后新增 `ApiError::Timeout` 时，下游的 `_` 会自动接住，不用改代码。**定义它的 crate 内部不受限制**：上面的 `Display` 实现仍可穷尽匹配，新增变体时编译器照样提醒你补分支。这个不对称正是它的价值——维护者保留穷尽检查，使用者获得向前兼容。

代价也要说清楚。下游从此失去穷尽性保证，新增变体不会在他们的代码里报错，只会静默落进 `_` 臂，可能被当成"未知错误"处理。所以 `#[non_exhaustive]` 适合"失败集合还会增长"的错误枚举；如果集合是封闭的、调用方必须逐个处理，就不该加。加了之后，也要在文档里写清楚默认分支的语义。

同样的属性可以用在结构体上：

```rust,noplayground
#[non_exhaustive]
pub struct ConfigError {
    pub kind: ErrorKind,
    pub path: String,
}
```

外部代码不能用结构体字面量构造它，也不能用模式匹配拆开所有字段，只能通过构造函数和访问器。这样你以后加字段、改字段可见性都不会破坏兼容。错误结构体如果字段是公开的，加上它几乎总是划算的。

## 27.7 API 设计原则：让错误没有机会发生

错误处理的一半工作在签名上。类型选对了，很多错误根本不会出现在运行期。

**参数用 `&str` 而不是 `String`。** 除非函数要存储或转移所有权，否则借用就够了：

```rust,noplayground
fn greet(name: &str) -> String {
    format!("hello, {name}")
}
```

`&String` 能通过解引用强制转换传给 `&str`，字符串字面量也能直接传。反过来，参数写成 `String` 会强迫持有 `&str` 的调用方分配一份拷贝。返回值同理：能从输入借用就返回 `&str`，需要新数据才返回 `String`。这条规则减少的是分配失败和所有权纠缠，不是错误类型本身。

**返回 `impl Iterator` 而不是 `Vec`。** 调用方常常只需要前几项，或者只想 `sum` 一下：

```rust,noplayground
fn evens(limit: u32) -> impl Iterator<Item = u32> {
    (0..limit).filter(|n| n % 2 == 0)
}

fn main() {
    let first: Vec<_> = evens(10).take(3).collect();
    println!("{first:?}");
}
```

这样不分配中间 `Vec`，调用方还能 `take` 提前停止。代价是返回类型不透明，两个不同分支返回不同迭代器时不能直接写 `impl Iterator`，要么 `Box<dyn Iterator>`，要么用 `Either` 一类的枚举。API 设计就是这种取舍：把灵活性留给调用方，把复杂度留给自己。

**避免 `bool` 参数。** `parse(input, true)` 在调用处完全读不出 `true` 的含义，而且两个布尔参数就有四种组合，其中不少是无意义的。用枚举表达模式：

```rust,noplayground
enum Mode {
    Strict,
    Lenient,
}

fn parse(input: &str, mode: Mode) -> Result<Config, ConfigError> {
    // ...
}
```

调用处变成 `parse(input, Mode::Strict)`，自解释，新增模式也不用改签名。返回 `bool` 也有类似问题：调用方通常需要知道"为什么失败"，那就返回 `Result<(), Error>`，而不是 `false`。

**用 newtype 表达不变式。** 校验一次，之后类型本身就保证合法：

```rust,noplayground
#[derive(Debug)]
enum PortError {
    Zero,
}

struct Port(u16);

impl Port {
    fn new(value: u16) -> Result<Self, PortError> {
        if value == 0 {
            return Err(PortError::Zero);
        }
        Ok(Port(value))
    }

    fn get(self) -> u16 {
        self.0
    }
}
```

字段私有，外部只能通过 `Port::new` 构造，所以 `Port` 存在即合法。下游函数接收 `Port` 而不是 `u16`，既不用重复校验，也不会把端口号和超时时间搞混。newtype 是零成本的，编译后就是一个 `u16`。

**参数多了用 Builder。** 六个参数的函数，调用处全是位置参数，加一个可选参数就得改所有调用点。Builder 把"可选"写进类型，把校验集中到 `build`：

```rust
#[derive(Debug)]
struct Request {
    url: String,
    retries: u32,
}

#[derive(Debug, Default)]
struct RequestBuilder {
    url: Option<String>,
    retries: Option<u32>,
}

#[derive(Debug)]
enum BuildError {
    MissingUrl,
}

impl RequestBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    fn retries(mut self, retries: u32) -> Self {
        self.retries = Some(retries);
        self
    }

    fn build(self) -> Result<Request, BuildError> {
        let url = self.url.ok_or(BuildError::MissingUrl)?;
        Ok(Request {
            url,
            retries: self.retries.unwrap_or(3),
        })
    }
}

fn main() {
    let req = RequestBuilder::new()
        .url("https://example.com")
        .retries(1)
        .build();
    println!("{req:?}");

    let bad = RequestBuilder::new().build();
    println!("{bad:?}");
}
```

输出：

```text
Ok(Request { url: "https://example.com", retries: 1 })
Err(MissingUrl)
```

注意 `build` 返回 `Result`：缺 URL 是调用方可以处理的失败，不是 panic 的理由。`unwrap_or(3)` 则表达"缺省值"，和"缺失即错误"是两回事。把这两种情况分开，是 Builder 设计的核心。

## 27.8 深入：`Box<dyn Error>` 的内存布局

`dyn Error` 是 trait 对象，而 trait 对象是胖指针：一个机器字指向数据，另一个指向虚表。虚表由编译器为每个具体类型生成一份，放在只读数据段，里面记录大小、对齐、析构函数和各方法的地址。

```rust
use std::error::Error;
use std::io;
use std::mem::size_of;

fn main() {
    println!("Box<io::Error> = {}", size_of::<Box<io::Error>>());
    println!("Box<dyn Error> = {}", size_of::<Box<dyn Error>>());
    println!("&dyn Error = {}", size_of::<&dyn Error>());
    println!(
        "Box<dyn Error + Send + Sync> = {}",
        size_of::<Box<dyn Error + Send + Sync>>()
    );
}
```

输出（64 位平台）：

```text
Box<io::Error> = 8
Box<dyn Error> = 16
&dyn Error = 16
Box<dyn Error + Send + Sync> = 16
```

几个结论可以直接从这四个数字读出来。

第一，`Box<dyn Error>` 比 `Box<io::Error>` 大一倍，多出来的是虚表指针；具体错误类型无论多大，装进盒子后 `Err` 分支都只占 16 字节。这正是"公开 API 返回 `Box<dyn Error>` 能让 `Result` 保持小"的原因，代价是每次转换都要堆分配一次。

第二，`&dyn Error` 和 `Box<dyn Error>` 一样大。借用不复制数据，只复制两个指针；`source()` 返回 `&dyn Error` 因此不需要任何分配，遍历错误链是纯读操作。

第三，加 `Send + Sync` 不改变大小。自动 trait 不往虚表里加方法，只改变 trait 对象的类型约束；编译器据此拒绝把不含这两个约束的对象送进线程。错误案例 4 的报错就是这条约束在起作用。

第四，`downcast_ref` 依赖 `TypeId`，而 `TypeId` 只对 `'static` 类型可用，所以 trait 对象必须满足 `'static`。这也解释了为什么 `source` 的返回类型写着 `&(dyn Error + 'static)`。

错误路径上的动态派发不值得优化：一次间接调用相对于一次 I/O 或一次解析可以忽略。但在每秒调用百万次的热路径里，把错误装进盒子会带来可观的分配开销，这时具体枚举更合适。

## 典型错误案例

下面五个案例都出在错误类型的设计上，而不是语法本身。行号与代码块一致，输出摘自真实的 `rustc`，省略号表示删去了无关行。

### 案例 1：把错误格式化成 String 再 ? ——E0277

`LoadError` 用 `String` 存消息，`?` 找不到可用的转换：

```rust,editable
use std::fs;

#[derive(Debug)]
struct LoadError(String);

fn load(path: &str) -> Result<String, LoadError> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {path}: {e}"))?;
    Ok(text)
}

fn main() {}
```

```text
error[E0277]: `?` couldn't convert the error to `LoadError`
 --> main.rs:8:60
  |
8 |         .map_err(|e| format!("failed to read {path}: {e}"))?;
  |          --------------------------------------------------^ the trait `From<String>` is not implemented for `LoadError`
  |
note: `LoadError` needs to implement `From<String>`
  = note: the question mark operation (`?`) implicitly performs a conversion on the error value using the `From` trait
...
```

**原因**：`?` 只看类型，通过 `From` 转换。`String` 不是错误类型，`LoadError` 也没有实现 `From<String>`。更根本的问题是 `format!` 已经把 `io::Error` 的类型和 `source` 销毁了，即使补一个 `From<String>`，也只能得到一个没有原因的字符串。

**修复**：不要在中途字符串化，让变体携带路径和原始错误：

```rust,noplayground
#[derive(Debug)]
enum LoadError {
    Io { path: String, source: std::io::Error },
}

// 调用点
.map_err(|source| LoadError::Io { path: path.to_string(), source })?;
```

`Display` 打印路径，`source` 返回 `io::Error`，诊断链完整。

### 案例 2：没实现 Display 就调用 to_string——E0599

`to_string` 不是枚举自带的方法，它来自 `ToString` 的 blanket 实现：

```rust,editable
#[derive(Debug)]
enum ConfigError {
    Missing,
}

fn main() {
    let err = ConfigError::Missing;
    println!("{}", err.to_string());
}
```

```text
error[E0599]: `ConfigError` doesn't implement `std::fmt::Display`
    --> main.rs:8:24
     |
   8 |     println!("{}", err.to_string());
     |                        ^^^^^^^^^ method cannot be called on `ConfigError` due to unsatisfied trait bounds
     |
     = note: the following trait bounds were not satisfied:
             `ConfigError: std::fmt::Display`
             which is required by `ConfigError: ToString`
...
```

**原因**：`ToString` 对一切 `T: Display` 自动实现，所以调用 `to_string` 等价于要求 `ConfigError: Display`。只写了 `#[derive(Debug)]` 不满足这个条件。编译器不会提示"实现 Display"，而是报"方法找不到"，因为 trait 方法必须 trait 在作用域内且类型满足约束。

**修复**：实现 `Display`，`ToString` 自动就有了，不要手写：

```rust,noplayground
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "missing configuration")
    }
}

impl std::error::Error for ConfigError {}
```

### 案例 3：Err 分支塞了另一种错误——E0308

函数声明返回 `ParseError`，`Err` 里却直接放了 `io::Error`：

```rust,editable
#[derive(Debug)]
enum ParseError {
    NotANumber(std::num::ParseIntError),
}

fn parse(input: &str) -> Result<u32, ParseError> {
    if input.is_empty() {
        return Err(std::io::Error::other("empty input"));
    }
    input.parse().map_err(ParseError::NotANumber)
}

fn main() {}
```

```text
error[E0308]: mismatched types
   --> main.rs:8:20
    |
  8 |         return Err(std::io::Error::other("empty input"));
    |                --- ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `ParseError`, found `Error`
    |                |
    |                arguments to this enum variant are incorrect
...
help: the type constructed contains `std::io::Error` due to the type of the argument passed
```

**原因**：`return Err(e)` 是直接构造，`Err` 的类型由函数签名固定为 `ParseError`，不会调用 `From`。只有 `?` 才会隐式转换。这里少了一个能装 `io::Error` 的变体。

**修复**：给枚举补上 `Io` 变体，显式包装：

```rust,noplayground
enum ParseError {
    NotANumber(std::num::ParseIntError),
    Io(std::io::Error),
}

return Err(ParseError::Io(std::io::Error::other("empty input")));
```

如果写 `Err(std::io::Error::other(...).into())`，则要求 `ParseError: From<io::Error>`，同样得先补 `impl`。

### 案例 4：`Box<dyn Error>` 不能跨线程——E0277

`thread::spawn` 要求闭包的返回类型满足 `Send`，trait 对象默认不带这个自动 trait：

```rust,editable
use std::error::Error;
use std::thread;

fn run() -> Result<(), Box<dyn Error>> {
    let handle = thread::spawn(|| -> Result<(), Box<dyn Error>> { Ok(()) });
    handle.join().unwrap()?;
    Ok(())
}

fn main() {}
```

```text
error[E0277]: `dyn std::error::Error` cannot be sent between threads safely
   --> main.rs:5:18
    |
  5 |     let handle = thread::spawn(|| -> Result<(), Box<dyn Error>> { Ok(()) });
    |                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `dyn std::error::Error` cannot be sent between threads safely
    |
    = help: the trait `Send` is not implemented for `dyn std::error::Error`
...
    = note: required by a bound in `spawn`
```

**原因**：`Box<dyn Error>` 只擦除了 `Error` 这一层，没有承诺背后的具体类型能安全地跨线程移动。`spawn` 的约束是 `T: Send + 'static`，而 `dyn Error` 不在 `Send` 的自动实现之列。

**修复**：把约束写进 trait 对象类型：

```rust,noplayground
fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let handle = thread::spawn(|| -> Result<(), Box<dyn Error + Send + Sync>> { Ok(()) });
```

标准库为所有 `E: Error + Send + Sync` 实现了到这种盒子的 `From`，所以 `?` 照常可用。

### 案例 5：返回借用错误的引用——E0106

函数想返回一个 `&dyn Error`，但没有说明借用的来源：

```rust,editable
use std::error::Error;

fn last_error() -> &dyn Error {
    &std::io::Error::other("boom")
}

fn main() {
    let _ = last_error();
}
```

```text
error[E0106]: missing lifetime specifier
 --> main.rs:3:20
  |
3 | fn last_error() -> &dyn Error {
  |                    ^ expected named lifetime parameter
  |
  = help: this function's return type contains a borrowed value, but there is no value for it to be borrowed from
help: consider using the `'static` lifetime, but this is uncommon unless you're returning a borrowed value from a `const` or a `static`
```

**原因**：`&` 必须能回答"借用了谁、活多久"。这里的 `io::Error` 是函数内的临时值，函数返回时就被销毁，引用会悬空。编译器拒绝推断，要求显式生命周期。

**修复**：返回拥有所有权的盒子，把错误移出函数：

```rust,noplayground
fn last_error() -> Box<dyn Error> {
    Box::new(std::io::Error::other("boom"))
}
```

只有错误本身是 `static` 时才适合返回 `&'static dyn Error`；一般情况下，返回 `Box<dyn Error>` 才是想要的语义。

## 27.9 动手练习

1. **手写一条完整错误链**：为下面的场景定义 `ConfigError` 枚举（`Io`、`Missing(String)`、`Invalid { key: String, value: String }` 三个变体），手写 `Display`、`Error`（`source()` 只在 `Io` 上返回 `Some`），实现 `From<std::io::Error>`，然后写 `fn load(path: &str) -> Result<Config, ConfigError>`，内部用 `?` 把三层错误串起来。最后写一个测试，沿 `source()` 走到底并断言链条长度是 2。
2. **两种签名的对照**：把同一个函数分别写成 `fn run() -> Result<(), Box<dyn Error>>` 和 `fn run() -> Result<(), AppError>`，各写一个调用方。列出「调用方能不能 match 具体变体」「二进制体积」「编译时间」「向下游承诺了什么」四个维度的差异，说明为什么库应该用具体类型、应用可以用 `Box<dyn Error>`。
3. **端口解析的四种失败**：实现 `parse_port(&str) -> Result<u16, PortError>`，要求空串、含非数字、等于 0、大于 65535 各有一个独立变体，且 `Display` 输出四句不同的人话（例如「端口不能为空」「端口必须是数字」「端口不能为 0」「端口超出范围 1-65535」）。给枚举加 `#[non_exhaustive]`，并解释加了之后下游代码会多出哪个编译错误。
4. **Option 还是 Result**：把 `fn find(id: u64) -> Option<User>` 改成 `Result<User, NotFound>`，再写两个调用方比较。然后列一张判据表回答：「找不到」对调用方来说是正常分支还是异常分支？需要携带原因吗？调用方通常立刻 `unwrap_or_default` 还是必须处理？
5. **模仿一个真实的错误类型**：读 `serde_json::Error` 的文档（或用 `cargo doc --offline` 看本地源码），回答它为什么把内部表示藏起来、为什么要提供 `classify()` 与 `line()`/`column()`、为什么 `impl From<io::Error>` 而不是暴露 `Io` 变体。然后照着写一个 `ParserError`：内部私有枚举 + 公开的 `kind()` + 位置信息，并说明这样做的代价（下游无法穷尽匹配，只能按 kind 分类处理）。

## 27.10 小结

- 错误类型是公开 API 的一部分：一旦发布，变体名、字段、`Display` 文案都会被下游依赖，改动的破坏性和改函数签名一样大。
- 库用具体的错误枚举（可 match、可判断），应用用 `Box<dyn Error + Send + Sync>`（省事、能跨线程）；这条分界线比「统一用 anyhow」更耐用。
- 手写错误类型的三件套是 `Display`、`Error`、`From`；`source()` 只该指向「更低层的原因」，不要把同一个错误同时放进消息和 source 里，否则日志里会出现重复的两行。
- `?` 的传播依赖 `From`：每一层都要有从下层错误到本层错误的转换，否则编译直接报 `the trait From<X> is not implemented for Y`。
- 错误消息写给两类读者：终端用户看「发生了什么、该怎么办」，运维看「哪个文件、哪个键、哪个偏移」。两者需要的细节不同，必要时应分开（`Display` 与 `Debug`/结构化字段）。
- `#[non_exhaustive]` 让下游的 `match` 必须带 `_` 分支，从而允许你在小版本里新增变体；代价是下游永远无法穷尽匹配。
- 最好的错误处理是让错误不可能发生：用 newtype 把「已校验」编码进类型（`Port(u16)` 而不是 `u16`），比在每个调用点重复校验更可靠。
- `Box<dyn Error>` 是胖指针（数据指针 + vtable 指针，16 字节），`Box<dyn Error + Send + Sync>` 还会多一层自动 trait 的 vtable 组合；它换来的是「不用为每种错误写一个变体」。
- 错误信息里不要放大段输入原文（可能有隐私、可能几个 GB），只放键名、偏移、长度这类定位信息。
- 判断 API 好坏的标准之一：**调用方能不能在不读实现的情况下，仅凭类型签名就知道所有失败模式。** 能做到就是好设计。
