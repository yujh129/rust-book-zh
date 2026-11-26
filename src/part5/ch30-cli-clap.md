# 第 30 章 命令行工具实战：clap

> **本章目标**
>
> - 说清手写 `std::env::args` 的解析在第三个参数之后为什么开始失控，能用一张表判断何时该引入 clap
> - 会用 `#[derive(Parser)]` 把一个结构体变成命令行接口，让字段类型直接决定参数类型
> - 掌握短选项、长选项、位置参数、`Option<T>`、`default_value_t`、`num_args`、`value_delimiter`、`required` 与 `ArgAction::Count` 的行为差异
> - 会用 `#[derive(Subcommand)]` 组织子命令，用 `ValueEnum`、`value_parser` 和环境变量控制取值来源与合法性
> - 能读懂 clap 的 `ErrorKind`，区分「用法错误退出 2」和「程序自身失败退出 1」，并用 `try_parse` 接管错误
> - 理解 argv 到词法切分、再到 `ArgMatches`、最后到结构体这四个阶段的边界，知道 `--`、短选项聚簇、外部子命令分别在哪一层生效

很多 Rust 程序的 `main` 第一行是 `let args: Vec<String> = std::env::args().collect();`。参数只有一两个时这样写没问题；一旦有默认值、可选值、类型转换、子命令，这段代码就会变成 `main` 里最长的一块，而且每加一个参数要改三个地方：解析分支、帮助文本、错误处理。

clap 是 Rust 生态里的命令行解析库，本机使用的版本是 4.6.7。它把「参数长什么样」和「解析出来是什么类型」合成了一件事：你声明一个结构体，字段名就是参数名，字段类型就是取值方式。这不是语法糖的堆砌，而是把定型的解析逻辑交给库，把业务逻辑留在你自己的代码里。

本章按「先能跑，再讲边界」的顺序展开：先用 clap 重写 30.1 的手写解析器，再逐个补齐参数形态，接着讲子命令与取值控制，最后深入到 argv 到结构体的四个阶段，并给出五个真实错误案例。

## 30.1 为什么不要手写 std::env::args

先用标准库写一个只有三个参数的程序：`--port` 带值、`-v` 可重复、其它一律报错。这是教学示例里最常见的手写解析器。

```rust
use std::env;

fn main() {
    let mut port: u16 = 8080;
    let mut verbose = 0u8;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--port" => {
                let raw = args.next().unwrap_or_else(|| {
                    eprintln!("error: --port 需要一个值");
                    std::process::exit(2);
                });
                port = raw.parse().unwrap_or_else(|_| {
                    eprintln!("error: 端口不是合法数字: {raw}");
                    std::process::exit(2);
                });
            }
            "-v" => verbose += 1,
            other => {
                eprintln!("error: 未知参数 {other}");
                std::process::exit(2);
            }
        }
    }

    println!("port = {port}, verbose = {verbose}");
}
```

这段代码能跑，但它的能力边界很快就会撞到：

- `--port=8080` 会落到「未知参数」分支，因为 `match` 比的是整个字符串；
- `-p8080` 同样失败，短选项和值黏在一起是绝大多数 CLI 都支持的写法；
- `-vvv` 失败，用户习惯把重复的短开关连写；
- 没有 `--help`，也拿不到自动生成的用法说明；
- 用户把 `--prot` 敲错时，程序只能说「未知参数」，不能提示「你是不是想写 --port」。

要继续补齐这些，需要再写几百行：切分 `=`、识别聚簇、按前缀匹配长选项、算编辑距离做拼写建议、维护帮助文本的排版。这些都是「已知答案」的机械工作，正是应该交给库的部分。

为了把选择讲清楚，下面这张表把需求拆开对比。手写一列的「代价」按需要额外写的代码量估计。

| 需求 | 手写 std::env::args | clap 的写法 |
| --- | --- | --- |
| 长选项带值 | match 分支加取下一个 | #[arg(long)] |
| --port=8080 | 手工按 = 切分 | 内建支持 |
| -p8080 | 手工识别前缀 | 内建支持 |
| 短选项聚簇 -vvv | 逐字符循环 | action = ArgAction::Count |
| 类型转换与报错 | parse 加重写错误信息 | 字段类型 u16 自动完成 |
| 缺参、未知参数提示 | 每个分支手写 | 内建，退出码固定为 2 |
| --help 与 -V | 手写文本，手写退出 | #[command(version)] 自动生成 |
| 子命令 | 用第一个位置参数模拟 | #[derive(Subcommand)] |
| shell 补全脚本 | 几乎没人手写 | clap_complete 生成 |

代价同样要说清楚。引入 clap 会带来一棵依赖树（本机锁定后是 21 个包），首次编译多花十几秒，以及二进制体积的增长：同一个「只解析一个 --file 参数」的程序，纯标准库版本在 `opt-level = "z"` 下是 121 KB，换成 clap 是 446 KB，多出约 325 KB。具体测量在 30.7 节。

判据因此可以写得很具体：**参数超过三个、有子命令、需要发布给别人的工具，用 clap；一两个固定位置参数的内部脚本，手写更省事。** 手写不是错误，只是在需求增长时必然要重写。

## 30.2 第一个 clap 程序

在 `Cargo.toml` 里加依赖。`derive` 是派生宏支持，`env` 是读取环境变量所需的特性，两个都要显式打开：

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }
```

`version = "4"` 是语义化版本要求，允许 4.x 的任意小版本；本章实测锁定到 4.6.7。不打开 `derive` 的话 `#[derive(Parser)]` 根本不存在，不打开 `env` 的话 `env =` 会变成编译错误——第 30.8 节的案例 1 就是前者的真实报错。

下面是一个完整可跑的最小程序：

```rust,noplayground
use clap::Parser;
use std::path::PathBuf;

/// 一个最小的命令行笔记工具
#[derive(Parser, Debug)]
#[command(name = "notes", version, about)]
struct Cli {
    /// 数据文件位置
    #[arg(long, default_value = "notes.json")]
    file: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    println!("{:#?}", cli);
}
```

三处信息合起来定义了接口：

1. `#[derive(Parser)]` 为 `Cli` 生成 `impl Parser for Cli`，其中包含 `parse`、`try_parse`、`command`、`parse_from` 四组关联函数；
2. 字段上的文档注释 `///` 变成该参数的帮助文本，结构体上的文档注释变成整个程序的 `about`；
3. `#[arg(...)]` 描述参数形态，`#[command(...)]` 描述命令本身。`version` 会自动读取 `Cargo.toml` 里的版本号，不需要手写。

运行 `notes --help` 得到：

```text
一个最小的命令行笔记工具

Usage: notes [OPTIONS]

Options:
      --file <FILE>  数据文件位置 [default: notes.json]
  -h, --help         Print help
  -V, --version      Print version
```

运行 `notes -V` 得到一行版本号：

```text
notes 0.1.0
```

在 Windows 上，`Usage:` 那行会显示成 `notes.exe`，因为可执行文件名带扩展名——书里为了简洁统一写成 `notes`，其余输出完全一致。

两者都会以退出码 0 结束，并且写到标准输出；这一点很重要，因为「帮助是正常输出」和「报错是错误输出」在脚本里的处理方式不同。

**字段类型决定取值方式**，这是 derive 写法最需要记的一点。同名的 `#[arg(long)]` 配上不同类型，行为完全不同：

| 字段类型 | 参数行为 | 不给时 |
| --- | --- | --- |
| bool | 布尔开关，出现即为真 | false |
| String / PathBuf | 必须跟一个值 | 报错，除非有默认值 |
| `Option<T>` | 可省略 | None |
| `Vec<T>` | 可重复出现，值累积 | 空向量 |
| u16 / f64 等 | 解析并校验，失败即用法错误 | 报错，除非有默认值 |
| 派生了 ValueEnum 的枚举 | 限定在候选值内 | 报错，除非有默认值 |

到这里接口就算立起来了。但真实工具的参数总要多几个，下一节把常用的参数形态一次讲完。

## 30.3 参数形态：从开关到变长列表

### 短选项与长选项

```rust,noplayground
#[derive(Parser)]
struct Cli {
    /// 长短都有：自动得到 -f 和 --file
    #[arg(short, long)]
    file: String,

    /// 显式指定短名
    #[arg(short = 'o', long)]
    output: String,

    /// 只有长选项：--dry-run
    #[arg(long)]
    dry_run: bool,

    /// 只有短选项，注意是大写 O
    #[arg(short = 'O')]
    open: bool,
}
```

四条命名规则决定了用户最终看到什么：

- `short` 不给值，取字段名的首字母；`long` 不给值，取字段名的 kebab-case，所以 `dry_run` 是 `--dry-run`；
- 短名大小写敏感，`-o` 和 `-O` 是两个不同的参数，同一程序里可以并存；
- `bool` 字段自动是「开关」语义（内部的 `ArgAction::SetTrue`），出现即真，不需要也不接受 `= true`；
- 想改名用 `short = 'f'`、`long = "file"`、`value_name = "PATH"`，其中 `value_name` 只影响帮助文本里的占位符。

同一个带值的选项有四种等价写法，clap 都认：

```console
$ notes --file notes.json
$ notes --file=notes.json
$ notes -f notes.json
$ notes -fnotes.json
```

第四种是「值黏在短选项后面」，实测 `-p8080` 会被解析成 `port: 8080`。这也是手写解析器最容易漏掉的一种。

### 位置参数

不加 `short`/`long` 的字段就是位置参数，字段的声明顺序就是用户的输入顺序：

```rust,noplayground
#[derive(Parser)]
struct Cli {
    /// 源路径
    source: PathBuf,
    /// 目标路径
    dest: PathBuf,
    /// 可选模式
    #[arg(value_name = "MODE")]
    mode: Option<String>,
}
```

它的用法说明是 `notes <SOURCE> <DEST> [MODE]`：必填的用尖括号，可选的用方括号。位置参数不能带默认值以外的花样——一旦顺序变了，用户的命令行就变了，所以 CLI 的位置参数一般不超过两个。

### Option 与两种默认值

```rust,noplayground
#[derive(Parser, Debug)]
struct Cli {
    /// 不给就是 None
    #[arg(long)]
    name: Option<String>,

    /// 不给就用 4
    #[arg(long, default_value_t = 4)]
    jobs: usize,

    /// default_value 收一段字符串，内部再做解析
    #[arg(long, default_value = "notes.json")]
    file: PathBuf,
}
```

`default_value_t` 和 `default_value` 的区别是前者用 `Display` 把 Rust 值渲染成字符串，后者直接给字符串再走和用户输入相同的解析路径。因此 `PathBuf` 这类不实现 `Display` 的类型只能用 `default_value`。两者的共同点是：一旦有默认值，这个参数在帮助里会显示 `[default: ...]`，并且永远不会触发「缺少参数」错误。

`Option<T>` 和默认值不要同时用。`Option<String>` 加 `default_value` 时，用户没给也会得到 `Some("...".to_string())`，`None` 这个分支永远不会出现，读代码的人会误判。

### 重复、定长与分隔符

```rust,noplayground
#[derive(Parser, Debug)]
struct Cli {
    /// 可重复，也可用逗号一次给多个
    #[arg(short, long, value_delimiter = ',')]
    tag: Vec<String>,

    /// 一次固定吃两个值
    #[arg(long, num_args = 2)]
    pair: Option<Vec<String>>,

    /// 贪婪地吃一个或多个
    #[arg(long, num_args = 1..)]
    ids: Vec<u64>,

    /// 类型可空，但命令行上必须给
    #[arg(long, required = true)]
    name: Option<String>,
}
```

实测 `notes add milk -t home,shop -t urgent` 得到的 `tag` 是三项：`["home", "shop", "urgent"]`。`value_delimiter = ','` 让一个参数内部再切分，配合 `Vec` 就能同时支持「重复给」和「逗号分隔」。

`num_args = 2` 要求这个选项后面恰好跟两个值，帮助文本里会写成 `--pair <PAIR> <PAIR>`。`num_args = 1..` 是上不封顶，但它**遇到下一个以短横线开头的 token 就停**，所以 `remove --ids 1 2 3 --yes` 里 `--yes` 不会被吞进 `ids` 向量。

`required = true` 加在 `Option<T>` 上看起来矛盾，实际用途是：参数必须由用户显式提供，但程序内部仍用 `Option` 表达「可能有值」的接口形态。帮助文本里它会出现在 Usage 行，例如实测的 `Usage: shapes [OPTIONS] --port <PORT> --name <NAME>`。

### 计数开关 -vvv

用户习惯用 `-v`、`-vv`、`-vvv` 表示日志级别，这需要 `ArgAction::Count`：

```rust,noplayground
#[arg(short, long, action = ArgAction::Count)]
verbose: u8,
```

帮助文本会渲染成 `-v, --verbose...`，那个省略号就是「可重复」的提示。实测 `-vvv` 得到 `verbose: 3`。如果 `verbose` 写成 `bool`，行为是 `SetTrue`，`-vvv` 会被当成第二个 `-v` 而报错。

`ArgAction` 还有几个常用值：`SetTrue`（bool 的默认）、`SetFalse`（出现即置假）、`Append`（Vec 的默认）、`Set`（重复出现时以最后一次为准）。当你看到「重复给同一个选项，后一个覆盖前一个」的需求时，答案就是 `Set`。

短选项可以聚簇：`-vvv` 是三个 `-v`。聚簇的切分规则和 POSIX getopt 一致——**遇到需要取值的选项时，同一 token 里剩下的字符就是这个值**。实测 `notes add -tv home` 得到 `tag: ["v"]` 和 `text: "home"`，而不是把 `v` 当作开关。

## 30.4 子命令：把多个工具装进一个二进制

当命令行形态从「一堆选项」变成「先说要做什么，再说怎么做」时，就该用子命令。声明方式是给一个字段加上 `#[command(subcommand)]`：

```rust,noplayground
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "notes", version)]
struct Cli {
    /// 全局选项，写在子命令前后都可以
    #[arg(short, long, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 新增一条笔记
    Add {
        text: String,
        #[arg(short, long, value_delimiter = ',')]
        tag: Vec<String>,
    },
    /// 列出笔记
    List {
        /// 只看某个标签
        #[arg(short, long)]
        tag: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    println!("{:#?}", cli);
}
```

实测 `notes --help` 的输出：

```text
一个最小的命令行笔记工具

Usage: notes [OPTIONS] <COMMAND>

Commands:
  add     新增一条笔记
  list    列出笔记
  help    Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose...   输出更详细的日志，可重复：-v、-vv、-vvv
      --file <FILE>  数据文件位置 [default: notes.json]
  -h, --help         Print help
  -V, --version      Print version
```

三件事是自动的：枚举变体名转成 kebab-case 子命令名（`AddItem` 会变成 `add-item`，要改就写 `#[command(name = "...")]`）；每个变体的字段成为该子命令自己的参数；`help` 子命令自动生成，`notes help add` 与 `notes add --help` 等价。

注意 `Commands` 区域里没有 `help` 之外的任何内置命令，`help` 是 clap 自己加的。`global = true` 的选项可以写在子命令之前或之后，实测 `notes -v add ...` 与 `notes add ... -v` 都能得到 `verbose: 1`。

子命令的参数一多，就地写在枚举变体里会很长。这时把参数抽成一个结构体，让变体变成元组形式：

```rust,noplayground
#[derive(clap::Args, Debug)]
struct AddArgs {
    text: String,
    #[arg(short, long, value_delimiter = ',')]
    tag: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 新增一条笔记
    Add(AddArgs),
}
```

`#[derive(Args)]` 与 `#[derive(Parser)]` 的字段属性完全相同，区别只是前者不会生成顶层解析入口。真实项目通常再用 `#[command(flatten)]` 把一组共享选项（日志、超时、输出格式）嵌进每个子命令。

## 30.5 枚举、自定义解析与环境变量

### ValueEnum：让枚举直接成为可选值

上一节的 `Priority` 已经展示过最省事的做法：给枚举派生 `ValueEnum`，字段上加 `value_enum`，帮助文本就自动带上候选值，解析也自动完成。

```rust,noplayground
#[derive(ValueEnum, Clone, Debug)]
enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Parser, Debug)]
struct Cli {
    #[arg(short, long, value_enum, default_value_t = Priority::Normal)]
    priority: Priority,
}
```

三个细节值得记住。第一，`ValueEnum` 要求类型同时实现 `Clone`，只派生 `ValueEnum` 会编译报错，所以习惯上写 `#[derive(ValueEnum, Clone, Copy, Debug)]`。第二，变体名同样转成 kebab-case，`VeryHigh` 就是 `very-high`，要改就写 `#[value(name = "critical")]`。第三，帮助里的候选值来自类型本身，所以实测的错误提示是：

```text
error: invalid value 'urgent' for '--priority <PRIORITY>'
  [possible values: low, normal, high]

For more information, try '--help'.
```

用户把 `-p urgent` 写错时，程序不需要自己列候选值，clap 已经把 `possible values` 打出来了。

### value_parser：控制字符串怎么变成值

clap 4 的解析统一走 `ValueParser`。对实现了 `ValueParserFactory` 的类型（`u16`、`usize`、`PathBuf` 等），派生宏会自动接上，所以 `--offset` 收到一个超出 `usize` 的数字时，报错是类型层面给出的：

```text
error: invalid value '99999999999999999999' for '--offset <OFFSET>': number too large to fit in target type

For more information, try '--help'.
```

需要额外约束时用 `value_parser!(T)` 拿到的构建器再加方法。下面这个例子把端口限制在 1 以上：

```rust,noplayground
#[derive(Parser)]
struct Cli {
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}
```

实测 `--port 0` 的输出是：

```text
error: invalid value '0' for '--port <PORT>': 0 is not in 1..=65535

For more information, try '--help'.
```

自定义规则则写一个普通函数：

```rust,noplayground
fn ratio(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("{s} 不是数字"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{s} 超出 0..=1"))
    }
}

#[derive(Parser)]
struct Cli {
    #[arg(long, value_parser = ratio)]
    ratio: f64,
}
```

实测 `--ratio 1.5` 得到 `error: invalid value '1.5' for '--ratio <RATIO>': 1.5 超出 0..=1`。注意 `value_parser = ratio` 后面不带圆括号：属性要的是函数本身，clap 自己去调用它。函数的返回值只要 `Err` 部分能转成 `Box<dyn Error>`（`String` 可以），错误文本就会原样出现在提示里。

### env =：让选项也能从环境变量来

给字段加上 `env = "NOTES_FILE"`，帮助文本里就多出一行 `[env: NOTES_FILE=]`：

```rust,noplayground
#[arg(long, default_value = "notes.json", env = "NOTES_FILE")]
file: PathBuf,
```

实测 `NOTES_FILE=/tmp/x.json notes list` 得到 `file: "/tmp/x.json"`；而同时给命令行和环境变量时，命令行赢：

```console
$ NOTES_FILE=/tmp/env.json notes --file cli.json list
$ # file: "cli.json"
```

优先级是「命令行 > 环境变量 > 默认值」：显式敲出来的参数压过环境。`env` 是 cargo feature（在依赖里漏掉它，编译期会报 E0599），而环境变量的值同样走 `value_parser`，所以 `NOTES_PORT=abc` 会被和命令行输入一样地拦下，比手写 `std::env::var` 再各自 `parse` 一致得多。

## 30.6 解析失败与退出码

### parse 与 try_parse

`Cli::parse()` 是 `Cli::try_parse()` 的语法糖，两者只在出错时的行为上不同：

```rust,noplayground
fn main() {
    // parse(): 出错时自己打印并退出
    // 等价于 match try_parse() { Ok(c) => c, Err(e) => e.exit() }
    let cli = Cli::parse();
    println!("{:?}", cli);
}
```

`try_parse()` 返回 `Result<Cli, clap::Error>`，不打印、不退出，把决定权交还给你。需要「先看参数再决定是否真的跑」的程序（比如测试、库代码、或者要在解析失败前后做清理的工具）应该用它。

### clap::Error 的形状

`clap::Error` 上最常用的三个方法决定了错误怎么被消费：

```rust,noplayground
use clap::error::ErrorKind;

fn main() {
    match Cli::try_parse() {
        Ok(cli) => println!("{cli:?}"),
        Err(e) => {
            let kind: ErrorKind = e.kind();
            let code: i32 = e.exit_code();
            let to_stderr: bool = e.use_stderr();
            eprintln!("kind = {kind:?}, exit_code = {code}, use_stderr = {to_stderr}");
            std::process::exit(code);
        }
    }
}
```

实测三种输入的取值：

- `--port abc`：`kind = ValueValidation`，`exit_code = 2`，`use_stderr = true`；
- `-h`：`kind = DisplayHelp`，`exit_code = 0`，`use_stderr = false`；
- `--prot`：`kind = UnknownArgument`，`exit_code = 2`，`use_stderr = true`。

这解释了为什么 `--help` 的输出走 stdout 且退出码为 0：它不是错误，而是一次「用户请求的帮助」，管道里可以正常接 `less`、`grep`。真正的错误走 stderr 且退出码为 2。

### 退出码 2 与自定义

Unix 惯例里，退出码 1 表示运行期失败，2 表示用法错误，clap 遵循后者。这个分工让 shell 脚本能区分「我参数写错了」和「程序跑起来但业务失败」：

```bash
if notes list --offset 1; then
    echo "正常"
else
    case $? in
        2) echo "用法错了，检查参数" ;;
        *) echo "业务失败" ;;
    esac
fi
```

如果你想用 `exit_code()` 之外的语义（比如批量执行时「有子项失败」），在 `try_parse` 的 `Err` 分支里自己决定即可，但别为此改变用法错误的码——调用者的脚本会依赖它。

## 30.7 深入：clap 的分词、定型与取舍

### 两阶段解析：先分词，再定型

clap 的内部可以分成两个阶段。第一阶段是**构建与分词**：`#[derive(Parser)]` 生成的代码把你的结构体翻译成一棵 `Command` 描述（每个字段变成 `Arg`），`get_matches()` 再拿 `std::env::args_os()` 去匹配这棵描述，得到 `ArgMatches`——一个「参数 id 到原始值」的字典。第二阶段是**定型**：派生宏为结构体实现 `FromArgMatches`，把字典里的字符串按字段类型取出来、解析、装进结构体。

理解这两阶段，就能解释很多现象：为什么 `--help` 在字段类型写错之前就能打印（构建阶段已经完成）；为什么 `Cli::parse()` 不等于 `std::env::args()` 加手写循环（它中间隔着一次按描述匹配）；为什么派生方式与 builder 方式可以互换。

第二阶段是公开的。你可以在派生结构体上取出原始字典：

```rust,noplayground
use clap::{CommandFactory, FromArgMatches, Parser};

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    port: u16,
}

fn main() {
    let matches = Cli::command().get_matches();
    if let Some(port) = matches.get_one::<u16>("port") {
        println!("原始字典里的端口：{port}");
    }
    let cli = Cli::from_arg_matches(&matches).unwrap();
    println!("定型后的端口：{}", cli.port);
}
```

`Cli::command()` 返回的 `Command` 描述和 `from_arg_matches` 都是公开接口：纯 builder 写法（`Command::new` 加 `Arg::new`）与派生完全等价，派生宏只是它的语法糖。分割点暴露出来以后，你可以写「一次构建描述、多次解析」的测试，也可以把这棵描述交给补全脚本生成器复用。

### `--` 分隔与参数透传

双短横线 `--` 告诉分词阶段「后面不再是选项」。实测 `notes add -- -t` 得到 `text: "-t"`：这个 `-t` 是位置参数的值，不会被当成标签选项。

需要把一长串参数原样交给子进程的场合（`cargo run -- ...` 就是这个形态），用两个属性配合：

```rust,noplayground
#[derive(Parser)]
struct Cli {
    /// 剩余参数原样收集，允许其中出现短横线开头的内容
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}
```

实测 `notes run -- --help -x` 收集到 `["--help", "-x"]`，程序自己的帮助没有被触发。反过来说，`notes run --help` 仍然会打印 `run` 子命令的帮助——`--help` 的识别发生在 `--` 之前。

### 短选项聚簇发生在分词阶段

上一节看到的 `-tv home` 行为，本质是分词器把 `-tv` 按字符切开：先遇到 `v`（开关，继续），再遇到需要取值的 `t`，于是同一 token 里剩下的字符或下一个 token（`home`）成为它的值。所以 `-tv home` 等价于 `-t v home`，而 `-vt home` 相当于 `-v -t home`，位置参数就缺了。手写解析器通常只处理「选项和值分开写」这一种形态，聚簇、`=` 连接、黏连值全都要自己补。

### allow_external_subcommands：留一个后门

插件式 CLI（把未知子命令交给外部可执行文件）可以用 `allow_external_subcommands`：

```rust,noplayground
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(allow_external_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Status,
    #[command(external_subcommand)]
    External(Vec<String>),
}

fn main() {
    match Cli::parse().command {
        Command::Status => println!("ok"),
        Command::External(args) => println!("外部子命令：{args:?}"),
    }
}
```

注意 `external_subcommand` 的变体必须收 `Vec<String>`，并且第一个元素是子命令名本身。实测 `external.exe foo --bar 1` 得到 `External(["foo", "--bar", "1"])`——连 `--bar` 这种未知选项也一并交给你，是否处理由你决定。这比 `trailing_var_arg` 更「靠前」：它作用在子命令这一层。

### 补全脚本与二进制体积

clap 生态里的 `clap_complete` 能从同一棵 `Command` 描述生成各 shell 的补全脚本，不需要你维护第二份参数表：

```rust,noplayground
use clap::CommandFactory;
use clap_complete::{generate, Shell};

fn print_completion(shell: Shell) {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "notes", &mut std::io::stdout());
}
```

把它挂到一个隐藏子命令上，用户执行 `notes completion bash > /etc/bash_completion.d/notes` 即可。补全脚本的内容来自运行时构建的那棵描述，所以新增参数后补全自动跟上。

代价是体积。实测一个「只解析一个 `--file` 参数」的 clap 程序与纯 `std::env::args` 手写程序对比：默认 release 配置下 724480 字节对 140288 字节，多约 584 KB；加上下面的裁剪配置后是 445952 字节对 120832 字节，仍多约 325 KB。

```toml
[profile.release]
strip = true
lto = true
codegen-units = 1
opt-level = "z"
```

另外，clap 会带来 21 个依赖包、首次开发构建约 13.7 秒。结论不是「不要用」，而是**按场景选**：几十行的一次性脚本、或体积敏感的工具，手写 `args()` 更划算；参数超过五六个、或有子命令和补全需求时，clap 省下的解析逻辑与错误提示远超这点成本。

## 典型错误案例

下面五段报错都是本机 `rustc 1.100.0` 加 `clap 4.6.7` 原样跑出来的，只保留与问题直接相关的部分；代码块就是被编译的那份 `src/main.rs`，行号与报错中的箭头一一对应。

### 案例 1：忘了 use clap::Parser——E0599

```rust,editable
#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    port: u16,
}

fn main() {
    let cli = Cli::parse();
    println!("{}", cli.port);
}
```

```text
error: cannot find derive macro `Parser` in this scope
 --> src/main.rs:1:10
  |
1 | #[derive(Parser)]
  |          ^^^^^^

error: cannot find attribute `arg` in this scope
 --> src/main.rs:3:7
  |
3 |     #[arg(short, long)]
  |       ^^^

error[E0599]: no associated function or constant named `parse` found for struct `Cli` in the current scope
 --> src/main.rs:8:20
  |
2 | struct Cli {
  | ---------- associated function or constant `parse` not found for this struct
...
8 |     let cli = Cli::parse();
  |                    ^^^^^ associated function or constant not found in `Cli`
  |
  = help: items from traits can only be used if the trait is implemented and in scope
  = note: the following traits define an item `parse`, perhaps you need to implement one of them:
          candidate #1: `clap::Parser`
          candidate #2: `clap::builder::TypedValueParser`

For more information about this error, try `rustc --explain E0599`.
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Parser` 在这里不是类型而是派生宏，`#[arg]` 也只是派生宏认识的辅助属性。两者都必须先在作用域里，编译器才知道 `#[derive(Parser)]` 要展开成什么、`#[arg]` 该交给谁处理。而 `parse()` 是 `clap::Parser` trait 上的方法，trait 不在作用域里就找不到——所以同一个根因被报了三次：宏找不到、属性找不到、方法找不到。

**修复**：加一行 `use`。

```rust,editable
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    port: u16,
}

fn main() {
    println!("{}", Cli::parse_from(["demo", "--port", "8080"]).port);
}
```

</details>

### 案例 2：子命令枚举漏派生——E0277

```rust,editable
use clap::Parser;

#[derive(Parser)]
#[command(name = "demo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug)]
enum Command {
    Add { text: String },
}

fn main() {
    let cli = Cli::parse();
    println!("{:?}", cli.command);
}
```

```text
error[E0277]: the trait bound `Command: FromArgMatches` is not satisfied
   --> src/main.rs:7:14
    |
  7 |     command: Command,
    |              ^^^^^^^ unsatisfied trait bound
    |
help: the trait `FromArgMatches` is not implemented for `Command`
   --> src/main.rs:11:1
    |
 11 | enum Command {
    | ^^^^^^^^^^^^

error[E0277]: the trait bound `Command: Subcommand` is not satisfied
   --> src/main.rs:7:14
    |
  7 |     command: Command,
    |              ^^^^^^^ unsatisfied trait bound
    |
help: the trait `Subcommand` is not implemented for `Command`
   --> src/main.rs:11:1
    |
 11 | enum Command {
    | ^^^^^^^^^^^^
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`#[command(subcommand)]` 本身不生成任何解析逻辑，它只是声明「这个字段的类型要能充当子命令」。于是编译器要求 `Command` 同时实现 `Subcommand`（提供命令描述，也就是帮助里那个 `add`）和 `FromArgMatches`（把匹配结果定型成枚举）。这两个 trait 正是 `#[derive(Subcommand)]` 一起生成的东西，漏掉派生就只剩「字段声明了一个不满足约束的类型」。

**修复**：给枚举补上派生，并把 `Subcommand` 引入作用域。

```rust,editable
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "demo")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Add { text: String },
}

fn main() {
    let cli = Cli::parse_from(["demo", "add", "hello"]);
    println!("{:?}", cli.command);
}
```

</details>

### 案例 3：ValueEnum 少了 Clone——E0277

```rust,editable
use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Debug)]
enum Mode {
    Fast,
    Slow,
}

#[derive(Parser)]
struct Cli {
    #[arg(long, value_enum)]
    mode: Mode,
}

fn main() {
    let cli = Cli::parse();
    println!("{:?}", cli.mode);
}
```

```text
error[E0277]: the trait bound `Mode: Clone` is not satisfied
   --> src/main.rs:4:6
    |
  4 | enum Mode {
    |      ^^^^ the trait `Clone` is not implemented for `Mode`
    |
note: required by a bound in `ValueEnum`
   --> clap_builder-4.6.7/src/derive.rs:293:30
    |
293 | pub trait ValueEnum: Sized + Clone {
    |                              ^^^^^ required by this bound in `ValueEnum`
help: consider annotating `Mode` with `#[derive(Clone)]`
    |
  4 + #[derive(Clone)]
  5 | enum Mode {
    |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`ValueEnum` 的声明是 `pub trait ValueEnum: Sized + Clone`，父 trait 里带着 `Clone`。生成候选值列表、把匹配到的值交还给你，都要复制枚举值，所以只派生 `ValueEnum` 不满足约束。同一份报错会在字段处再出现一次（`ArgMatches::remove_one` 也要求 `Clone`），根因只有一个。

**修复**：`#[derive(ValueEnum, Clone, Debug)]`——习惯上顺手加上 `Copy`，让枚举在两处使用时不必 `.clone()`。

```rust,editable
use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Mode {
    Fast,
    Slow,
}

#[derive(Parser)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Fast)]
    mode: Mode,
}

fn main() {
    let cli = Cli::parse();
    println!("{:?}", cli.mode);
}
```

</details>

### 案例 4：env = 需要打开 cargo feature——E0599

```rust,noplayground
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[arg(long, env = "NOTES_FILE", default_value = "notes.json")]
    file: String,
}

fn main() {
    let cli = Cli::parse();
    println!("{}", cli.file);
}
```

```text
error[E0599]: no method named `env` found for struct `Arg` in the current scope
 --> src/main.rs:5:17
  |
5 |     #[arg(long, env = "NOTES_FILE", default_value = "notes.json")]
  |                 ^^^ method not found in `Arg`

For more information about this error, try `rustc --explain E0599`.
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`#[arg(env = "...")]` 展开后调用的是 `Arg::env`，而这个方法被 cargo feature 门控：`clap = { version = "4", features = ["derive"] }` 只开了 `derive`，`env` 方法在编译期根本不存在，于是派生宏展开出的一行普通方法调用找不到目标。报错指向属性所在的行，是因为展开后的代码锚定在那里。

**修复**：在依赖里补上 `env` feature。Rust 代码一行都不用改。

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }
```

改完依赖后，上面那段 `src/main.rs` 原样即可编译运行。

</details>

### 案例 5：default_value_t 用在没有 Display 的类型上——E0277

```rust,editable
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value_t = PathBuf::from("notes.json"))]
    file: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    println!("{}", cli.file.display());
}
```

```text
error[E0277]: the trait bound `PathBuf: ToString` is not satisfied
 --> src/main.rs:7:17
  |
7 |     #[arg(long, default_value_t = PathBuf::from("notes.json"))]
  |                 ^^^^^^^^^^^^^^^ the trait `std::fmt::Display` is not implemented for `PathBuf`
  |
  = note: required for `PathBuf` to implement `ToString`

For more information about this error, try `rustc --explain E0277`.
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`default_value_t` 的语义是「把一个 Rust 值渲染成字符串塞进默认值」，因此要求类型实现 `Display`（经由 `ToString`），而 `PathBuf` 故意不实现 `Display`——路径不保证是合法 UTF-8。`String`、`u16`、枚举这类类型没有这个问题，所以错误通常在换成路径、字节串时才冒出来。

**修复**：路径类默认值改用字面量形式的 `default_value`，它接收字符串并在解析时再转成 `PathBuf`。

```rust,editable
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "notes.json")]
    file: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    println!("{}", cli.file.display());
}
```

</details>

## 动手练习

1. 给 `notes` 增加 `search` 子命令：一个必填的位置参数 `keyword`、可重复的 `--tag`、以及 `--limit`（`default_value_t = 10`）。写出完整代码，并预测缺 `keyword` 时的退出码。
2. 用自定义 `value_parser` 实现 `--config`：值必须是一个已存在的文件路径，否则报错信息里要出现「找不到配置文件 <路径>」。用 `try_parse` 验证错误文本。
3. 把输出格式做成 `ValueEnum`（`json`/`table`/`csv`），`--format` 支持 `env = "NOTES_FORMAT"`，默认 `table`。写出 `--help` 中这个选项会显示的行。
4. 为「`notes add` 缺少位置参数」写一个单元测试：用 `Cli::try_parse_from(["notes", "add"])` 断言 `kind()` 是 `ErrorKind::MissingRequiredArgument`、`exit_code()` 是 2。
5. 用 `clap_complete` 为 `notes` 生成 bash 补全，并说明为什么补全脚本不需要随着参数增加而手工维护。

## 小结

- clap 把「解析命令行」变成「描述参数形状」：派生宏读字段类型，类型即参数类型，解析失败由库统一给出带上下文的提示。
- `Cli::parse()` 出错就打印并退出；需要自己处理错误时用 `try_parse()`，从 `clap::Error` 上取 `kind()`、`exit_code()`、`use_stderr()`。
- 帮助走 stdout 且退出码为 0，用法错误走 stderr 且退出码为 2，调用者的脚本可以据此区分两类失败。
- `Option<T>` 表示可省略，`default_value_t` 要求 `Display`，路径类默认值用 `default_value`，`required = true` 表达「必须由用户给出」。
- `Vec<T>` 配 `value_delimiter` 同时支持重复与分隔符，`num_args` 控制取值个数，`ArgAction::Count` 实现 `-vvv`。
- 短选项可以聚簇，遇到需要取值的选项时，同一 token 剩下的字符就是值；`--` 之后的一切都是位置参数。
- 子命令用一个 `#[derive(Subcommand)]` 的枚举表达，参数多了就抽成 `#[derive(Args)]` 结构体，选项想全局可用加 `global = true`。
- `ValueEnum` 让枚举成为候选值（记得连带 `Clone`），`value_parser` 承接自定义校验，`env =` 让环境变量成为第二来源，优先级是命令行 > 环境变量 > 默认值。
- 内部是「先按描述分词、再按字段类型定型」两阶段，所以派生写法与 builder 写法可以混用。
- 代价是约 325 KB 体积与 21 个依赖：脚本和体积敏感的工具可以考虑手写 `args()`，参数一多就该交给 clap。
