# 第 10 章 错误处理：panic、Result 与 ?

> **本章目标**
>
> - 分清哪些失败该 `panic!`、哪些该返回 `Result`，并说得出判断依据
> - 用 `map`、`map_err`、`and_then`、`ok_or`、`?` 把嵌套的 `match` 压成一条链
> - 自己定义错误 `enum`，实现 `Display`、`Error`、`From`，让 `?` 自动转换错误
> - 按"库用具体类型、应用用 anyhow"给错误分层，说清 `thiserror` 与 `anyhow` 的取舍
> - 看懂 E0277、E0308、E0382、E0599、E0004 这五个与错误处理相关的报错

在 C++、Java、Python 里，函数会不会失败，签名上看不出来。异常（exception）把失败变成一条隐形的控制流：任何一层都能 `throw`，调用方可以完全不知情，编译器也不检查你有没有 `catch`。结果是"这个函数可能抛什么"只能写在文档里，而文档会过期。

Rust 把失败写进类型。可能失败的函数返回 `Result<T, E>`：`T` 是成功时的值，`E` 是失败时的值。签名即文档，`match` 强制你处理两个分支。代价是错误要一层层往上搬，本章一半的篇幅都在讲怎么搬得不啰嗦。

还有一类失败根本不该恢复：数组越界、除以零、内部不变量被破坏。继续执行只会把坏数据传得更远，这时 `panic!` 才是正确选择。所以先划清 panic 与 Result 的边界，再谈工具。

## 10.1 两种失败：panic! 与 Result 的分工

读一个配置文件可能失败的方式有很多：文件不存在、没有权限、内容不是 UTF-8、端口那一行不是数字。这些都不是程序写错了，而是外部世界不配合。调用方可能想换个路径重试，也可能想把原因显示给用户，所以函数必须把失败交回去：

```rust
use std::num::ParseIntError;

fn parse_port(text: &str) -> Result<u16, ParseIntError> {
    text.parse()
}

fn main() {
    match parse_port("8080") {
        Ok(port) => println!("端口 {}", port),
        Err(e) => println!("配置有误：{}", e),
    }
}
```

`Result` 是普通枚举，`Ok` 和 `Err` 都是值，可以存进变量、放进 `Vec`、跨线程传递。这一点和异常有本质区别：异常发生时栈已经在展开，错误对象只能顺着 `catch` 走；`Result` 只是一个返回值，怎么处理由调用方决定。

另一类失败长这样：代码假设"这个数组至少有 3 个元素"，实际只有 2 个。这不是外部世界的问题，是代码的假设错了。此时返回 `Result` 没有意义——调用方拿到 `Err` 也不知道该做什么，因为程序状态已经不对了。用 `panic!`：

```rust
fn main() {
    let values = [10, 20, 30];
    println!("{}", values[1]);

    // let _ = values[3]; // 打开这行会 panic：index out of bounds
    let sum = 255u8.checked_add(1);
    println!("{:?}", sum);
}
```

程序输出 `20` 和 `None`。把注释那行放开，程序会打印 `thread 'main' panicked at ...`，消息是 `index out of bounds: the len is 3 but the index is 3`，然后以退出码 101 结束。`panic!` 不是"更严重的错误"，而是"程序不该继续跑"的信号。

判断标准可以压成一句话：**这个失败是调用方能合理处理的输入，还是调用方违反了你定的契约？** 前者返回 `Result`，后者 `panic!`。

| 场景 | 选择 | 理由 |
| --- | --- | --- |
| 文件不存在、网络断开、解析用户输入 | Result | 预期之内，调用方可能重试或换方案 |
| 索引越界、整数溢出、状态机走到不可能的分支 | panic! | 代码有 bug，继续跑只会扩大污染 |
| 库函数的参数违反文档约定 | panic! | 契约违反，越早暴露越好 |
| 原型、示例、测试 | unwrap / expect | 让失败立刻可见，正式代码再换回 Result |

`unwrap` 和 `expect` 都是 panic 的快捷方式，区别只在信息量：`unwrap` 只说 "called `Result::unwrap()` on an `Err` value"，`expect("端口必须是数字")` 会把这句一起带上。生产代码里看到 `.unwrap()` 就该停下来问一句：这个失败真的不可能发生吗？

标准库还有一批函数在特定条件下 panic：切片索引、`Vec::remove` 越界、整数除法除以零、调试模式下的整数溢出。最后一条值得单独记：`255u8 + 1` 在 debug 构建里 panic，在 release 构建里回绕成 `0`。想让溢出变成可处理的值，用 `checked_add`、`checked_mul` 这类方法，它们返回 `Option`，而不是 `Result`——"溢出了"没有更多原因可讲。这也正好是下一节 `Option` 与 `Result` 分工的缩影。

## 10.2 Result 与组合子：把 match 压平

标准库里的 `Result` 就是一个普通枚举，只有 `Ok(T)` 和 `Err(E)` 两个变体。它不是语言内建类型，所以第 9 章学的 `match`、`if let` 全都适用。类型参数 `T` 是成功时的值，`E` 是失败时的值；`E` 放什么由你决定，可以是 `String`，可以是标准库的 `ParseIntError`，也可以是自定义的 `enum`。

最直白的处理方式还是 `match`。下面的函数把解析错误换成一个能直接展示给用户的句子：

```rust
fn parse_port(text: &str) -> Result<u16, String> {
    match text.parse::<u16>() {
        Ok(n) => Ok(n),
        Err(e) => Err(format!("端口 `{}` 不合法：`{}`", text, e)),
    }
}

fn main() {
    println!("{:?}", parse_port("8080"));
    println!("{:?}", parse_port("80x"));
}
```

这段代码没有错，但形状很啰嗦：`Ok(n) => Ok(n)` 什么都没做，`Err` 分支也只是换了个错误值。`Result` 自带一组组合子（combinator），专门消除这种"只改一边"的样板代码。上面的函数可以压成两行：

```rust,noplayground
fn parse_port(text: &str) -> Result<u16, String> {
    text.parse::<u16>()
        .map_err(|e| format!("端口 `{}` 不合法：`{}`", text, e))
}
```

`map_err` 只在 `Err` 上动手，`Ok` 原样穿过；对应的 `map` 只在 `Ok` 上动手。两者都不改变"成功还是失败"，只改变里面的值：

```rust
fn main() {
    let ok: Result<i32, String> = Ok(21);
    let bad: Result<i32, String> = Err(String::from("boom"));

    println!("{:?}", ok.map(|v| v * 2));
    println!("{:?}", bad.clone().map_err(|e| format!("上层：`{}`", e)));
    println!("{}", bad.unwrap_or(0));
}
```

输出：

```text
Ok(42)
Err("上层：boom")
0
```

当下一步操作本身也可能失败时，用 `and_then`：闭包返回一个新的 `Result`，成功才继续。想在失败时换一条路走，用 `or_else`：

```rust
fn half(n: i32) -> Result<i32, String> {
    if n % 2 == 0 {
        Ok(n / 2)
    } else {
        Err(format!("`{}` 是奇数", n))
    }
}

fn main() {
    println!("{:?}", half(10).and_then(half));
    println!("{:?}", half(14).and_then(half));

    let recovered: Result<i32, String> = half(3).or_else(|e| {
        println!("先记一笔：`{}`", e);
        Ok(0)
    });
    println!("{:?}", recovered);
}
```

输出是 `Err("5 是奇数")`、`Err("7 是奇数")`、`Ok(0)`。注意 `and_then` 的闭包返回 `Result`，`map` 的闭包返回普通值——这是两者唯一但关键的区别：**`map` 不会让成功变成失败，`and_then` 会**。

`unwrap_or` 和 `unwrap_or_else` 用于给失败兜底，前者直接给值，后者给闭包。区别在求值时机：

```rust
fn expensive() -> i32 {
    println!("计算了默认值");
    8080
}

fn main() {
    let a: Result<i32, String> = Ok(1);
    println!("{}", a.unwrap_or(expensive()));

    let b: Result<i32, String> = Ok(1);
    println!("{}", b.unwrap_or_else(|_| expensive()));
}
```

输出是 `计算了默认值`、`1`、`1`：`unwrap_or` 的参数**先求值再传进去**，哪怕 `a` 是 `Ok`，`expensive()` 也照跑；`unwrap_or_else` 只在真的失败时才调用闭包。默认值需要分配内存、读文件、加锁时，这个差别就是实打实的开销。

常用的组合子可以归成一张表：

| 方法 | 参数形态 | 作用 |
| --- | --- | --- |
| `map` | `FnOnce(T) -> U` | 变换 `Ok` 里的值，失败原样穿过 |
| `map_err` | `FnOnce(E) -> F` | 变换 `Err` 里的值，成功原样穿过 |
| `and_then` | `FnOnce(T) -> Result<U, E>` | 串联另一个可能失败的步骤 |
| `or_else` | `FnOnce(E) -> Result<T, F>` | 失败时换一条路 |
| `unwrap_or` | 一个 `T` | 失败时用给定值，立即求值 |
| `unwrap_or_else` | `FnOnce(E) -> T` | 失败时用闭包算，惰性求值 |
| `ok_or` | 一个 `E` | `Option` 转 `Result`，`None` 变成给定错误 |
| `ok_or_else` | `FnOnce() -> E` | 同上，错误惰性构造 |

最后两个方法只存在于 `Option` 上。这不是巧合：`Option` 只表示"没有值"，不解释为什么没有；`Result` 要求你给出原因。把 `Option` 升格成 `Result` 的动作就是 `ok_or("原因")`；不想提前构造错误就用 `ok_or_else(|| ...)`，闭包只在 `None` 时才跑。

反过来，`Result::ok()` 和 `Result::err()` 会把 `Result` 降级成 `Option`，代价是丢掉另一半信息。经验规则：**只在函数内部用 `Option`，跨函数边界、尤其是公开 API，用 `Result`**——调用方迟早需要知道"为什么没有"。

## 10.3 ?：让错误自动向上走

组合子解决了"在成功路径上做变换"，但没有解决"失败要提前返回"。假设要从两段文本里各解析一个端口，任何一段失败就立刻返回：

```rust
use std::num::ParseIntError;

fn parse_pair(a: &str, b: &str) -> Result<(u16, u16), ParseIntError> {
    let x = match a.trim().parse::<u16>() {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let y = match b.trim().parse::<u16>() {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    Ok((x, y))
}

fn main() {
    println!("{:?}", parse_pair(" 8080 ", "443"));
    println!("{:?}", parse_pair(" 8080 ", "44x"));
}
```

输出 `Ok((8080, 443))` 和 `Err(ParseIntError { kind: InvalidDigit })`。

两个 `match` 都只做一件事：成功就取值，失败就 `return Err(e)`。这正是 `?` 的语义，所以整段可以压成三行：

```rust
use std::num::ParseIntError;

fn parse_pair(a: &str, b: &str) -> Result<(u16, u16), ParseIntError> {
    let x = a.trim().parse::<u16>()?;
    let y = b.trim().parse::<u16>()?;
    Ok((x, y))
}

fn main() {
    println!("{:?}", parse_pair(" 8080 ", "443"));
}
```

读法：`?` 作用在 `Result` 上时，`Ok(v)` 就取出 `v` 继续往下走；`Err(e)` 则把 `e` 转成函数返回类型要求的错误类型，然后从函数里 `return Err(...)`。所以它只能出现在返回 `Result` 或 `Option` 的函数里：它的展开里有一个 `return`，而 `return` 的值必须对得上函数签名。

规则可以归成三条：

1. 所在函数的返回类型必须是 `Result` 或 `Option`（严格说，是实现了 `Try` 的类型；稳定版上可用的只有这两个）。
2. 被作用的值必须是 `Result<T, E>` 或 `Option<T>`，不能是 `T` 本身。
3. 若函数返回 `Result<_, E2>`，则 `E2` 必须实现 `From<E>`，`?` 会自动调用 `From::from` 完成转换。

第 3 条最容易被低估。它让不同来源的错误汇进同一种类型，而不需要你写任何转换代码。比如函数返回 `Box<dyn Error>`，标准库为它实现了 `From<E>`（只要 `E: Error + 'static`），于是 `ParseIntError` 可以自动变成 `Box<dyn Error>`：

```rust
use std::error::Error;

fn parse_pair(a: &str, b: &str) -> Result<(u16, u16), Box<dyn Error>> {
    let x = a.trim().parse::<u16>()?;
    let y = b.trim().parse::<u16>()?;
    Ok((x, y))
}

fn main() {
    match parse_pair("8080", "44x") {
        Ok(pair) => println!("{:?}", pair),
        Err(e) => println!("失败：`{}`", e),
    }
}
```

输出 `失败：invalid digit found in string`。错误的具体类型被 `Box<dyn Error>` 擦掉了，只留下 `Display` 能打印的内容和 `source` 能问出来的链条。

`?` 也能用在 `Option` 上，前提是函数返回 `Option`：遇到 `None` 就提前返回 `None`。

```rust
fn first_char(text: &str) -> Option<char> {
    let first = text.chars().next()?;
    Some(first.to_ascii_uppercase())
}

fn main() {
    println!("{:?}", first_char("rust"));
    println!("{:?}", first_char(""));
}
```

输出 `Some('R')` 和 `None`。`Option` 上的 `?` 不携带任何原因，所以只适合"有没有"本身就足够表达语义的场合。

反过来，在返回 `Result` 的函数里对 `Option` 用 `?`，编译器会报 E0277，并建议先 `ok_or("字符串为空")` 把 `Option` 变成 `Result`，再用 `?`。

关于 `?` 还有两个常见坑，后面各给一个完整案例：目标错误类型没有实现 `From` 时是 E0277（案例 1），函数体最后忘了把值包成 `Ok` 时是 E0308（案例 2）。至于 `?` 展开后到底长什么样、为什么老版本 Rust 不允许在 `main` 里用它，留到 10.7 讲。

`?` 让错误自动向上走，但它要求沿途所有函数的错误类型能互相转换。函数一多，"能转换"这件事本身就需要设计——这正是自定义错误类型的动机。

## 10.4 自定义错误：enum、Display 与 Error

上一节里 `Box<dyn Error>` 把错误的具体类型擦掉了。这在应用里图个方便，在库里却是麻烦：调用方想按错误种类分支处理（"端口非法就提示用户改配置，字段缺失就补默认值"），类型被擦掉之后只能靠字符串匹配，编译器再也帮不上忙。库应该给出具体的错误类型，让调用方 `match`。

做法是定义一个 `enum`，每个变体对应一种失败原因；需要保留原始错误时，就把原始错误装进变体。以解析配置为例：

```rust
use std::error::Error;
use std::fmt;
use std::num::ParseIntError;

#[derive(Debug)]
enum ConfigError {
    MalformedLine(String),
    InvalidPort(ParseIntError),
    MissingField(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MalformedLine(line) => write!(f, "无法解析的行：`{line}`"),
            ConfigError::InvalidPort(_) => write!(f, "端口不是合法整数"),
            ConfigError::MissingField(name) => write!(f, "缺少字段 `{name}`"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::InvalidPort(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ParseIntError> for ConfigError {
    fn from(e: ParseIntError) -> Self {
        ConfigError::InvalidPort(e)
    }
}

fn parse_config(text: &str) -> Result<(String, u16), ConfigError> {
    let mut host = None;
    let mut port = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| ConfigError::MalformedLine(line.to_string()))?;
        match key.trim() {
            "host" => host = Some(value.trim().to_string()),
            "port" => port = Some(value.trim().parse::<u16>()?),
            _ => {}
        }
    }
    let host = host.ok_or(ConfigError::MissingField("host"))?;
    let port = port.ok_or(ConfigError::MissingField("port"))?;
    Ok((host, port))
}

fn show(text: &str) {
    match parse_config(text) {
        Ok((host, port)) => println!("解析成功：{host}:{port}"),
        Err(e) => {
            println!("错误：{e}");
            let mut source = e.source();
            while let Some(inner) = source {
                println!("  原因：{inner}");
                source = inner.source();
            }
        }
    }
}

fn main() {
    show("port=8080\nhost=127.0.0.1");
    show("port=abc\nhost=127.0.0.1");
}

```

输出：

```text
解析成功：127.0.0.1:8080
错误：端口不是合法整数
  原因：invalid digit found in string
```

三个 `impl` 各司其职：

- `Display` 面向用户，写人话。它和 `Debug` 的分工要记牢：`{}` 打印 `Display`，`{:?}` 打印 `Debug`；终端提示和日志用前者，调试和测试断言用后者。
- `Error` 本身几乎是空 trait，只提供 `source`、`provide` 等有默认实现的方法。它真正的价值是让类型能放进 `Box<dyn Error>`。它的父 trait 要求同时实现 `Debug` 和 `Display`，所以 `#[derive(Debug)]` 不能省。
- `From<ParseIntError>` 是给 `?` 用的：`parse::<u16>()?` 失败时，`?` 自动调用它，把 `ParseIntError` 包成 `ConfigError::InvalidPort`。没有这个 `impl`，就会得到案例 1 里的 E0277。

`source` 返回"上一层原因"。`InvalidPort` 里装着 `ParseIntError`，所以返回 `Some(e)`；其他变体没有内层错误，返回 `None`。这就是错误链（error chain）的数据结构：每一层只说自己的直接原因，打印时顺着 `source` 一层层往下走。`main` 里那个 `while let` 循环就是干这个的，输出里的"原因：invalid digit found in string"来自 `ParseIntError` 自己的 `Display`。

这里也顺带说明 `Display` 该怎么写：只说本层，不要把内层错误再拼一遍。否则错误链上每一层都重复同一句话，用户会看到三行一模一样的原因。本层 `InvalidPort` 只说"端口不是合法整数"，"invalid digit"交给 `source`。

注意 `parse_config` 里 `?` 出现了两次，接的却是两种不同来源的错误：`parse::<u16>()` 的 `ParseIntError` 靠 `From` 转换，`split_once` 的 `None` 靠 `ok_or_else` 转成 `Result`。前者是"错误类型转换"，后者是"把缺失变成错误"，别混为一谈。

手写这些 `impl` 的样板量不小：一个变体一个 `match` 臂，一个来源错误一个 `From`。所以生态里有了 `thiserror` 来自动生成它们；而应用层常常懒得区分错误种类，于是有了 `anyhow`。下一节把三者放在一起比较。

## 10.5 错误分层：库用具体类型，应用用 anyhow

同一个问题在不同层有不同的答案。库的错误类型是公开契约的一部分：调用方要能 `match`、要能分辨种类，将来加一个变体还要考虑兼容性，所以它必须具体、稳定、可穷举。应用的错误最终只做两件事——打印给用户、写进日志——没有人会 `match` 它，大家要的是"当时在干什么"的上下文和一条完整的因果链，所以用一个能装下一切的类型更省事。

先看库这边。10.4 那三个 `impl` 是标准写法，也是纯样板，`thiserror` 把它们交给宏生成：

```rust,noplayground
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("无法解析的行：`{0}`")]
    MalformedLine(String),
    #[error("端口不是合法整数")]
    InvalidPort(#[from] std::num::ParseIntError),
    #[error("缺少字段 `{0}`")]
    MissingField(&'static str),
}
```

它需要加一个依赖：

```toml
[dependencies]
thiserror = "2"
```

`#[error("...")]` 生成 `Display`，字符串里的 `{0}` 取第 0 个字段；`#[from]` 生成 `From`，`?` 可以直接用；如果某个字段是内层错误、但又不该由 `from` 自动构造，用 `#[source]` 标出来，`source()` 就会返回它。生成的是和手写完全一样的代码，没有装箱、没有运行时开销，错误类型依旧具体。

应用这边就换一副面孔：

```rust,noplayground
use anyhow::{bail, Context, Result};

fn load_port(text: &str) -> Result<u16> {
    let line = text.lines().next().context("配置是空的")?;
    let (_, value) = line.split_once('=').context("第一行没有等号")?;
    let port: u16 = value.trim().parse().context("端口不是数字")?;
    if port == 0 {
        bail!("端口不能为 0");
    }
    Ok(port)
}
```

`anyhow::Result<T>` 是 `Result<T, anyhow::Error>` 的别名；`anyhow::Error` 能装下任何 `E: Error + Send + Sync + 'static`，还能外挂任意上下文。`context` 在失败时补一句"当时在做什么"，`bail!` 等价于 `return Err(anyhow!("..."))`。它的 `Debug` 输出会把错误链和 backtrace 一起打出来，这正是应用层想要的。代价是具体类型被擦掉，调用方只能 `downcast_ref` 猜；所以库的公开 API 不该用它。

取舍可以列成一张表：

| 位置 | 推荐 | 代价 |
| --- | --- | --- |
| 库的公开 API | 具体 enum，手写或 thiserror | 新增变体是破坏性变更，逼你想清楚 |
| 应用内部、bin crate | anyhow::Result | 调用方拿不回具体类型 |
| 示例、脚本、小工具 | `Box<dyn Error>` | 零依赖，但没有上下文能力 |
| 错误种类会长期增长 | enum + #[non_exhaustive] | 调用方必须留通配分支 |

一条实用规则：**错误类型的所有权跟着 crate 边界走**。跨 crate 暴露的类型必须具体，crate 内部随你用 anyhow。两者混用也常见——库定义 enum，应用用 anyhow 把它包起来，再补上"读哪个文件时出的错"。第 27 章会把这条规则展开成完整的 API 设计。

## 10.6 main 也能返回 Result

前面所有例子都在 `main` 里 `match`。其实 `main` 的返回类型可以是 `Result`，`?` 于是能一直用到程序入口：

```rust
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let port: u16 = "8080".parse()?;
    println!("端口 {}", port);
    Ok(())
}
```

输出 `端口 8080`。如果返回 `Err`，运行时会用 `Debug` 打印它——形如 `Error: ParseIntError { kind: InvalidDigit }`——然后以退出码 1 结束。注意是 `Debug` 而不是 `Display`，所以错误类型的 `Debug` 不能太潦草；`Box<dyn Error>` 的 `Debug` 会打印里面具体类型的 `Debug`，信息通常够用。

需要区分退出码时（比如 2 表示用法错误），让 `main` 返回 `ExitCode`：

```rust
use std::process::ExitCode;

fn run() -> Result<(), String> {
    Err(String::from("配置缺失"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误：{}", e);
            ExitCode::FAILURE
        }
    }
}
```

它在标准错误打印 `错误：配置缺失`，退出码是 1。`ExitCode` 的好处是把"打印什么"和"返回什么码"都握在自己手里，适合会被 shell 脚本或 CI 调用的程序。另一种写法是在 `main` 里调用 `std::process::exit(2)`，但它会立刻终止进程，不跑任何析构函数——持有文件锁、临时目录、后台线程时别这么干。

1.26 之前的 Rust 不允许 `main` 返回 `Result`，旧代码里常见 `fn main() { if let Err(e) = run() { ... } }` 这种包装。这个写法今天仍然有用：它让你决定错误怎么打印、退出码是什么，而不是交给运行时的默认格式。

## 10.7 深入：? 的展开、must_use 与 panic 策略

**`?` 展开成什么。** 教科书常说 `?` 是语法糖，但它的展开依赖一个 trait，看懂展开才能明白前面三条规则从哪来：

```rust
use std::num::ParseIntError;

#[derive(Debug)]
enum MyError {
    Parse(ParseIntError),
}

impl From<ParseIntError> for MyError {
    fn from(e: ParseIntError) -> Self {
        MyError::Parse(e)
    }
}

fn with_question(text: &str) -> Result<u16, MyError> {
    let port = text.parse::<u16>()?;
    Ok(port)
}

fn with_match(text: &str) -> Result<u16, MyError> {
    let port = match text.parse::<u16>() {
        Ok(v) => v,
        Err(e) => return Err(MyError::from(e)),
    };
    Ok(port)
}

fn main() {
    println!("{:?} {:?}", with_question("8080"), with_match("8080"));
    println!("{:?}", with_question("x"));
}
```

输出 `Ok(8080) Ok(8080)` 和 `Err(Parse(ParseIntError { kind: InvalidDigit }))`。两个函数做的事完全一样：`?` 展开后就是 `match`、`From::from` 和提前 `return` 三件事的组合。这也解释了为什么第三节要求"函数返回类型必须匹配"——`return` 出去的值必须对得上签名；为什么老版本不能再在 `main` 里用——那时 `main` 返回 `()`，`return Err(...)` 无处可去；以及为什么自定义类型不能自己实现 `?`——它依赖 `Try` trait，而在稳定版上只有 `Result` 和 `Option` 实现了它。

**must_use 与沉默的失败。** `Result` 带 `#[must_use]` 标注，意思是"这个值有意义，别悄悄丢掉"。`"8080".parse::<u16>();` 这样的语句能编译，但会得到一条警告：

```text
warning: unused `Result` that must be used
 --> src/main.rs:2:5
  |
2 |     "8080".parse::<u16>();
  |     ^^^^^^^^^^^^^^^^^^^^^
  |
  = note: this `Result` may be an `Err` variant, which should be handled
  = note: `#[warn(unused_must_use)]` (part of `#[warn(unused)]`) on by default
help: use `let _ = ...` to ignore the resulting value
  |
2 |     let _ = "8080".parse::<u16>();
  |     +++++++
```

三种处理方式按优先级排：能返回就加 `?`；不能在当前函数返回就 `match` 或 `if let`；确定要忽略，写 `let _ = ...` 并加一行注释说明为什么失败也无所谓。`let _ = ...` 不是"消除警告的开关"，它是你对读者和未来的自己做的承诺。同一个标注也能用在你自己的类型上：`#[must_use = "调用方必须消费这个句柄"]`。

**panic 展开与 abort。** `panic!` 默认走栈展开（unwind）：逐层退出栈帧、运行析构函数，最后让线程结束。整个过程可以被 `catch_unwind` 拦截，测试框架和 FFI 边界依赖这个能力。代价是二进制里要带展开所需的元数据。追求体积和启动速度的程序可以换策略：

```toml
[profile.release]
panic = "abort"
```

设为 `abort` 后，panic 直接终止进程，`catch_unwind` 不再有效（它拦不住 abort），析构函数也不会执行。嵌入式、wasm、部分游戏引擎会这么选。有一条底线要记住：`panic` 不能穿过 `extern "C"` 边界，否则是未定义行为；跨语言调用时必须在边界内侧 `catch_unwind` 把 panic 转成错误码。

**错误链与 backtrace 不是一回事。** `source()` 串起来的是**因果**：解析失败是因为输入是 `abc`，读配置失败是因为解析失败。backtrace 记录的是**调用栈**：错误发生时程序正走在哪些函数里。前者由你的错误类型和 `?` 一层层攒出来，后者由运行时在 panic 或 `Backtrace::capture()` 时抓取，通常要设 `RUST_BACKTRACE=1` 才有内容。`anyhow` 的 `Debug` 输出两者都给。给用户的提示应该只有因果链，backtrace 留给日志和开发者——把一屏栈帧丢给终端用户，除了吓人没有别的作用。

## 典型错误案例

### 案例 1：? 无法把 ParseIntError 转成自定义错误——E0277

```rust,editable
#[derive(Debug)]
struct MyError;

fn parse_port(text: &str) -> Result<u16, MyError> {
    let port = text.parse::<u16>()?;
    Ok(port)
}

fn main() {
    println!("{:?}", parse_port("8080"));
}
```

```text
error[E0277]: `?` couldn't convert the error to `MyError`
 --> src/main.rs:5:35
  |
4 | fn parse_port(text: &str) -> Result<u16, MyError> {
  |                              -------------------- expected `MyError` because of this
5 |     let port = text.parse::<u16>()?;
  |                     --------------^ the trait `From<ParseIntError>` is not implemented for `MyError`
  |                     |
  |                     this can't be annotated with `?` because it has type `Result<_, ParseIntError>`
  |
note: `MyError` needs to implement `From<ParseIntError>`
 --> src/main.rs:2:1
  |
2 | struct MyError;
  | ^^^^^^^^^^^^^^
  = note: the question mark operation (`?`) implicitly performs a conversion on the error value using the `From` trait
```

**原因**：`?` 在返回 `Result<_, MyError>` 的函数里，要求 `MyError: From<ParseIntError>`。`MyError` 没有这个 `impl`，编译器不会替你猜错误该怎么表示；`#[derive(Debug)]` 只满足别的约束，与 `From` 无关。

**修复**：补上 `From` 实现，`?` 就会自动调用它：

```rust,editable
#[derive(Debug)]
struct MyError;

impl From<std::num::ParseIntError> for MyError {
    fn from(_: std::num::ParseIntError) -> Self {
        MyError
    }
}

fn main() -> Result<(), MyError> {
    let port = "8080".parse::<u16>()?;
    println!("端口 {}", port);
    Ok(())
}
```

### 案例 2：Ok 分支忘了包 Ok——E0308

```rust,editable
fn check(n: i32) -> Result<i32, String> {
    if n > 0 {
        println!("正数");
    } else {
        return Err(String::from("必须是正数"));
    }
}

fn main() {
    println!("{:?}", check(3));
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:2:14
  |
2 |       if n > 0 {
  |  ______________^
3 | |         println!("正数");
4 | |     } else {
  | |_____^ expected `Result<i32, String>`, found `()`
  |
  = note:   expected enum `Result<i32, String>`
          found unit type `()`
```

**原因**：`if` 分支的最后一句是 `println!`，它的值是 `()`；`else` 分支里是 `return`，不参与求值。于是整个 `if/else` 求出 `()`，与签名要求的 `Result<i32, String>` 不符。`println!` 返回 `()` 这件事，是新手最常踩的坑之一。

**修复**：让 `Ok(n)` 成为 `if` 分支的值，同时把 `else` 里的 `return Err(...)` 改成表达式：

```rust,editable
fn check(n: i32) -> Result<i32, String> {
    if n > 0 {
        println!("正数");
        Ok(n)
    } else {
        Err(String::from("必须是正数"))
    }
}

fn main() {
    println!("{:?}", check(3));
    println!("{:?}", check(-1));
}
```

### 案例 3：把错误取出来后原值不能再用——E0382

```rust,editable
fn main() {
    let result: Result<i32, String> = Err(String::from("磁盘已满"));
    let error = result.err().unwrap();
    println!("错误：{}", error);
    println!("{:?}", result);
}
```

```text
error[E0382]: borrow of moved value: `result`
 --> src/main.rs:5:22
  |
2 |     let result: Result<i32, String> = Err(String::from("磁盘已满"));
  |         ------ move occurs because `result` has type `Result<i32, String>`, which does not implement the `Copy` trait
3 |     let error = result.err().unwrap();
  |                        ----- `result` moved due to this method call
4 |     println!("错误：{}", error);
5 |     println!("{:?}", result);
  |                      ^^^^^^ value borrowed here after move
  |
help: consider calling `.as_ref()` to borrow the value's contents
  |
3 |     let error = result.as_ref().err().unwrap();
  |                       +++++++++
```

**原因**：`Result::err` 的签名是 `fn err(self) -> Option<E>`，按值接收 `self`，调用它就把 `result` 整个移动走了。后面再用就是使用已移动的值。`String` 没有 `Copy`，编译器不会悄悄复制一份。

**修复**：借而不是搬——在 `&result` 上做 `if let Err(e)`：

```rust,editable
fn main() {
    let result: Result<i32, String> = Err(String::from("磁盘已满"));
    if let Err(e) = &result {
        println!("出错了：{}", e);
    }
    println!("{:?}", result);
}
```

### 案例 4：在 Result 上调用 ok_or——E0599

```rust,editable
fn main() {
    let result: Result<u16, std::num::ParseIntError> = "8080".parse();
    let value = result.ok_or("缺少端口");
    println!("{:?}", value);
}
```

```text
error[E0599]: no method named `ok_or` found for enum `Result<T, E>` in the current scope
 --> src/main.rs:3:24
  |
3 |     let value = result.ok_or("缺少端口");
  |                        ^^^^^ method not found in `Result<u16, ParseIntError>`
```

**原因**：`ok_or` 定义在 `Option` 上，作用是把"没有值"变成"有错误"。`Result` 已经是"要么值要么错误"，没有这个动作，所以方法表里找不到它。

**修复**：`Result` 上对应的是 `map_err`，用来替换错误一侧的类型：

```rust,editable
fn main() {
    let result: Result<u16, std::num::ParseIntError> = "8080".parse();
    let value = result.map_err(|_| String::from("缺少端口"));
    println!("{:?}", value);
}
```

### 案例 5：match 漏掉 Err 分支——E0004

```rust,editable
fn parse(text: &str) -> Result<u16, String> {
    match text.parse::<u16>() {
        Ok(n) => Ok(n),
    }
}

fn main() {
    println!("{:?}", parse("8080"));
}
```

```text
error[E0004]: non-exhaustive patterns: `Err(_)` not covered
 --> src/main.rs:2:11
  |
2 |     match text.parse::<u16>() {
  |           ^^^^^^^^^^^^^^^^^^^ pattern `Err(_)` not covered
  |
  = note: the matched value is of type `Result<u16, ParseIntError>`
help: ensure that all possible cases are being handled by adding a match arm with a wildcard pattern or an explicit pattern as shown
  |
3 ~         Ok(n) => Ok(n),
4 ~         Err(_) => todo!(),
  |
```

**原因**：`match` 必须穷尽所有变体。这里只写了 `Ok(n)`，漏掉 `Err(_)`，而 `Result` 只有这两个变体。编译器不会假设"这个 `parse` 一定成功"。

**修复**：补上 `Err` 分支，把标准库错误转成签名里的 `String`：

```rust,editable
fn parse(text: &str) -> Result<u16, String> {
    match text.parse::<u16>() {
        Ok(n) => Ok(n),
        Err(e) => Err(e.to_string()),
    }
}

fn main() {
    println!("{:?}", parse("8080"));
    println!("{:?}", parse("x"));
}
```

## 动手练习

1. 下面这行代码能编译，但会得到一条警告：`"8080".parse::<u16>();`。分别用 `match`、`?`、`let _ = ...` 三种方式处理它，说明三种写法各自适合什么场景。

2. 把 10.3 的 `parse_pair` 改成接收三个参数，任一失败都立刻返回；再用 `map_err` 把 `ParseIntError` 换成带字段名的 `String` 错误。

3. 定义 `enum MathError { DivByZero, Overflow }`，实现 `Display` 与 `Error`，写 `fn checked_div(a: i32, b: i32) -> Result<i32, MathError>`。在 `main` 里用 `?` 串联两次除法，并打印错误链；要求 `cargo run` 能看到两种不同的错误。

4. 给 10.4 的 `parse_config` 增加"端口必须在 `1..=65535` 内"的检查，新增 `PortOutOfRange(u32)` 变体；再故意构造三种坏输入，确认每个变体都能被打印出来。

5. 阅读题：`fn main() -> Result<(), Box<dyn Error>>` 返回 `Err` 时，终端打印的内容来自 `Debug` 还是 `Display`？如果错误类型只实现了 `Display` 而没有 `Debug`，还能编译吗？动手验证你的答案。

## 小结

- `panic!` 用于代码违反自身契约，`Result` 用于调用方能合理处理的失败。
- `map` 与 `map_err` 只变换一侧，`and_then` 才能串联另一个可能失败的步骤。
- `?` 等于 `match` + `From::from` + 提前 `return`，所在函数必须返回 `Result` 或 `Option`。
- 自定义错误要同时实现 `Debug`、`Display`、`Error`，需要自动转换时再补 `From`。
- `source()` 串成错误链；`Display` 只说本层，别把内层原因重复拼一遍。
- 库的公开 API 用具体错误类型（可配 `thiserror`），应用内部用 `anyhow` 或 `Box<dyn Error>`。
- `main` 可以返回 `Result` 或 `ExitCode`：前者用 `Debug` 打印并以 1 退出，后者让你控制格式和退出码。
- `Result` 带 `#[must_use]`，忽略它就是给未来的自己埋雷；`panic = "abort"` 换来更小的体积，代价是失去展开与捕获。
