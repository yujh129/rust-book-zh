# 项目一：minigrep——一个真正的命令行搜索工具

> **本章目标**
>
> - 从一个模糊需求出发，把它拆成可执行的模块与接口
> - 手写参数解析（不依赖 clap），用 `Result` 表达「参数错误」
> - 写出带生命周期的搜索函数，理解为什么返回 `&'a str` 必须标注 `'a`
> - 用 `ExitCode` 让程序对脚本友好（grep 式的退出码语义）
> - 组织单元测试与集成测试，让功能可回归
> - 知道这个小工具还能往哪些方向长大（递归、多文件、高亮、二进制检测）

前面的章节是一个个知识点的切片。从这一章开始，我们把它们拼成**能交付的东西**。

第一个项目选 grep 的迷你版：`minigrep`。它足够小（一天能写完），又足够真实（涉及 IO、错误处理、生命周期、CLI 设计、测试、退出码）。

## 39.1 需求与验收标准

先写清楚要做什么，否则代码会越写越糊。

```text
minigrep [-i|--ignore-case] <模式> <文件>

行为：
  1. 逐行读取文件，输出包含模式的那些行，格式为「行号:行内容」
  2. 带 -i 时忽略大小写
  3. 找到 0 行时退出码 1（便于 shell 脚本判断）
  4. 参数错误退出码 2，文件读取失败退出码 3
```

为什么退出码要分这么细？因为命令行工具是**被别人脚本调用**的：

```bash
if minigrep -i error app.log; then
    echo "日志里有 error"
else
    echo "日志干净"
fi
```

如果所有情况都返回 0，「找没找到」就无法判断。grep 的约定是：0 = 找到，1 = 没找到，2 = 出错。我们沿用这套约定，再把「出错」拆成两类：**2 = 参数用法错误，3 = 文件读取失败**。脚本里 `if` 只看 0 与非 0，但需要区分「我没写对命令」和「文件不在了」时，3 这个码就有用。

## 39.2 项目骨架

```console
$ cargo new minigrep
$ cd minigrep
$ tree .
.
├── Cargo.toml
└── src
    ├── lib.rs      # 可测试的核心逻辑
    └── main.rs     # 薄薄的一层 CLI 壳
```

**为什么拆成 lib + bin**：`main.rs` 里的代码**没法被集成测试调用**（`tests/` 下的测试只能链接库 crate）。把所有逻辑放进 `lib.rs`，`main.rs` 只负责「解析参数、调用库、决定退出码」，这样核心逻辑能被完整测试。

```toml
[package]
name = "minigrep"
version = "0.1.0"
edition = "2021"
```

## 39.3 参数解析：手写 Config

最常见的错误写法是把解析逻辑堆在 `main` 里，用下标硬取：

```rust,noplayground
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let query = &args[1];
    let file_path = &args[2];
    println!("在 {} 里找 {}", file_path, query);
}
```

问题很明显：不带参数运行时直接 panic，报 `index out of bounds`。用户看到的是 Rust 的 panic 信息，而不是「用法：...」。

### 版本一：返回 Result 的构造函数

```rust
#[derive(Debug)]
pub struct Config {
    pub query: String,
    pub file_path: String,
    pub ignore_case: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    MissingArgument,
    UnknownFlag(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingArgument => write!(f, "用法：minigrep [-i] <模式> <文件>"),
            ConfigError::UnknownFlag(flag) => write!(f, "未知参数：{}", flag),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn build(args: &[String]) -> Result<Config, ConfigError> {
        if args.len() < 3 {
            return Err(ConfigError::MissingArgument);
        }

        let mut ignore_case = false;
        let mut positional = Vec::new();

        for arg in &args[1..] {
            match arg.as_str() {
                "-i" | "--ignore-case" => ignore_case = true,
                other if other.starts_with('-') => {
                    return Err(ConfigError::UnknownFlag(other.to_string()));
                }
                other => positional.push(other.to_string()),
            }
        }

        if positional.len() < 2 {
            return Err(ConfigError::MissingArgument);
        }

        Ok(Config {
            query: positional[0].clone(),
            file_path: positional[1].clone(),
            ignore_case,
        })
    }
}

fn main() {
    let args = vec![
        String::from("minigrep"),
        String::from("-i"),
        String::from("rust"),
        String::from("poem.txt"),
    ];
    let config = Config::build(&args).unwrap();
    println!("{:?}", config);
}
```

输出：

```text
Config { query: "rust", file_path: "poem.txt", ignore_case: true }
```

几个设计决策值得注意：

- `build` 接收 `&[String]` 而不是 `Vec<String>`：不夺走调用方的所有权，也方便测试时构造假参数
- 返回自定义的 `ConfigError` 而不是 `String`：调用方可以 `match` 具体错误，而字符串只能打印
- 实现了 `Display` + `Error`：这样它能被 `Box<dyn Error>` 装下、能被 `?` 传播
- 标志位和位置参数分开收集：`-i` 出现在模式之前或之后都能正确处理

### 为什么不直接用 clap

真实项目当然该用 `clap` 或 `bpaf`：它们自动生成 `--help`、支持 `--flag=value`、有 shell 补全。这个项目手写是为了看清**参数解析的本质**——无非是「遍历字符串、分类、校验」。第 30 章讲 clap，届时你会看到同样的事情用声明式写法只需 10 行。

## 39.4 主流程与退出码

`main.rs` 应该薄到「一眼看完」，它只做三件事：收集参数、调用库、把结果翻译成退出码。

```rust,noplayground
use std::env;
use std::process::ExitCode;

use minigrep::{run, Config};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let config = match Config::build(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("参数错误：{}", e);
            return ExitCode::from(2);
        }
    };

    match run(config) {
        Ok(0) => {
            eprintln!("没有找到匹配行");
            ExitCode::from(1)
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("读取错误：{}", e);
            ExitCode::from(3)
        }
    }
}
```

三个细节：

1. **`fn main() -> ExitCode`**：Rust 1.61 起 `main` 可以返回 `ExitCode`（或 `Result<(), E>`）。返回 `ExitCode::from(2)` 就是进程退出码 2，不需要 `std::process::exit`。
2. **错误信息走 `eprintln!`**：`println!` 是标准输出，`eprintln!` 是标准错误。脚本里 `minigrep ... > result.txt` 时，错误提示不该被混进结果文件。
3. **`Ok(0)` 单独处理**：找到了但结果为空，用退出码 1 表达「没有匹配」。这是 grep 的行为。

### 库里的 run

```rust,noplayground
use std::error::Error;
use std::fs;

pub fn run(config: Config) -> Result<usize, Box<dyn Error>> {
    let contents = fs::read_to_string(&config.file_path)?;

    let hits = if config.ignore_case {
        search_case_insensitive(&config.query, &contents)
    } else {
        search(&config.query, &contents)
    };

    for (line_no, line) in &hits {
        println!("{}:{}", line_no, line);
    }

    Ok(hits.len())
}
```

返回 `usize`（命中行数）让 `main` 能区分「0 命中」和「有命中」，同时 `Box<dyn Error>` 让 `?` 能把 `io::Error` 直接往上抛。

**为什么用 `Box<dyn Error>` 而不是自定义错误**：`run` 目前只可能失败一次（读文件），错误来源单一，用 trait 对象最省事。如果以后加上「解析二进制」「写入输出文件」等多种错误，就该换成自定义 enum（第 27 章）。

## 39.5 搜索函数与生命周期

这是全章最值得细看的地方。先写一个**返回引用**的搜索函数：

```rust
pub fn search<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)> {
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(query))
        .map(|(i, line)| (i + 1, line))
        .collect()
}

fn main() {
    let text = "第一行\n第二行有 rust\n第三行";
    for (n, line) in search("rust", text) {
        println!("{}: {}", n, line);
    }
}
```

输出：`2: 第二行有 rust`。

### 为什么必须写 `'a`

函数的输入有两个引用（`query` 和 `contents`），返回值里有引用。编译器必须知道**返回值里的引用来自哪个输入**——它不会替你猜。

```text
fn search(query: &str, contents: &str) -> Vec<(usize, &str)>
                                          ^^^^^^^^^^^^^^^^
                          这个 &str 是从 query 借的，还是从 contents 借的？
```

如果从 `query` 借，那么 `query` 改了返回值就失效；如果从 `contents` 借，那么 `contents` 活多久返回值就能活多久。语义完全不同，所以必须显式声明。

我们写的 `<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)>` 意思是：**返回值里的切片只与 `contents` 同寿**。`query` 的生命周期独立，因为函数内部只是拿它做比较，不会把它的一部分返回出去。

如果两个参数都标 `'a`（`contents: &'a str, query: &'a str`），编译器会把两者**取交集**，导致返回值被 `query` 的生命周期限制住——通常没必要，还可能让调用方提前报错。

### 大小写不敏感版本

朴素写法会遇到一个经典陷阱：

```rust,noplayground
pub fn search_case_insensitive<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)> {
    let lower_query = query.to_lowercase();
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&lower_query))
        .map(|(i, line)| (i + 1, line))
        .collect()
}
```

注意 `line.to_lowercase()` 创建了一个**临时 String**，但只在 `contains` 里用了一下就丢弃——返回的仍然是 `contents` 里的原始切片。所以生命周期没问题。

反过来，如果你想**返回小写后的行**，就会撞上 E0515：

```rust,noplayground
pub fn bad(s: &str) -> &str {
    let lower = s.to_lowercase();
    lower.split_whitespace().next().unwrap() // 返回借用临时变量的切片
}
```

`lower` 在函数结束时被销毁，返回的 `&str` 就成了悬垂引用，编译器直接拒绝。

### 一个小优化：查询串只小写一次

朴素写法是每次都把一行和查询串同时小写：每行都要分配一个 String。至少可以把查询串的小写提到循环外，只做一次：

```rust
pub fn search_case_insensitive<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)> {
    let query = query.to_lowercase();
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&query))
        .map(|(i, line)| (i + 1, line))
        .collect()
}

fn main() {
    let text = "Rust:\nsafe, fast, productive.\nRUST is also fast.";
    println!("{}", search_case_insensitive("rUsT", text).len());
}
```

输出 `2`。这里的改进只在查询串：它只小写一次。每行的小写仍是临时分配——这一步要彻底消除，得用 `eq_ignore_ascii_case`（只处理 ASCII）或 `chars().flat_map(char::to_lowercase)` 逐字符比较，那属于微优化，39.7 节的性能表里会给出取舍。

## 39.6 测试：单元 + 集成

### 单元测试放在 lib 里

```rust,noplayground
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Rust:\nsafe, fast, productive.\nPick three.\nTrust me.";

    #[test]
    fn case_sensitive_finds_one() {
        let hits = search("duct", SAMPLE);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 2);
    }

    #[test]
    fn case_insensitive_finds_two() {
        let hits = search_case_insensitive("rUsT", SAMPLE);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(search("zzz", SAMPLE).is_empty());
    }
}
```

`#[cfg(test)]` 让这个模块只在 `cargo test` 时编译，不会进发布产物。

### 集成测试放在 tests/ 目录

`tests/cli.rs` 里可以调用 `minigrep::Config::build`，但**不能**调用 `main`。想测真实进程行为（退出码、stderr），就要用 `std::process::Command` 启动编译好的二进制：

```rust,noplayground
use std::process::Command;

#[test]
fn exits_with_1_when_nothing_matches() {
    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["zzz", "tests/fixtures/poem.txt"])
        .output()
        .expect("启动失败");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("没有找到匹配行"));
}

#[test]
fn exits_with_2_on_bad_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["--bad", "poem.txt"])
        .output()
        .expect("启动失败");

    assert_eq!(output.status.code(), Some(2));
}
```

`env!("CARGO_BIN_EXE_minigrep")` 是 cargo 为集成测试注入的环境变量，指向编译好的二进制路径——不需要自己去猜 `target/debug` 在哪。

### 实际运行效果

准备一个 `poem.txt`；为了让集成测试也能读到同一份数据，把它复制到 `tests/fixtures/poem.txt`（`tests/cli.rs` 里的路径就是按这个位置写的）：

```text
Rust:
safe, fast, productive.
Pick three.
Trust me.
RUST is also fast.
```

```console
$ cargo run -q -- duct poem.txt
2:safe, fast, productive.
$ echo $?
0

$ cargo run -q -- -i rust poem.txt
1:Rust:
4:Trust me.
5:RUST is also fast.

$ cargo run -q -- --bad poem.txt
参数错误：未知参数：--bad
$ echo $?
2

$ cargo run -q -- zzz poem.txt
没有找到匹配行
$ echo $?
1
```

注意第二组的第 4 行 `Trust me.` 也命中了 `rust`——因为忽略大小写后 `Trust` 里含 `rust`。这不是 bug，是子串匹配的正常行为（想按单词匹配就得换正则）。

## 39.7 让它长大：五个可行的扩展方向

这个工具停在 200 行，但它有清晰的成长路径。下面每个方向都标注了需要的新知识。

**1. 多文件与目录递归**：把 `file_path` 换成 `Vec<PathBuf>`，用 `fs::read_dir` 递归。注意符号链接成环（用 `fs::metadata` 而不是 `symlink_metadata` 判断）。

```rust,noplayground
use std::fs;
use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let mut files = Vec::new();
    walk(Path::new("."), &mut files)?;
    println!("共 {} 个文件", files.len());
    Ok(())
}
```

**2. 二进制文件检测**：`fs::read_to_string` 遇到非 UTF-8 会返回 `InvalidData`。想优雅处理就读 `Vec<u8>`，先检查前 8KB 有没有 `0x00`，再决定是否当文本处理。

**3. 彩色高亮**：给匹配片段包上 ANSI 转义序列 `\x1b[31m ... \x1b[0m`。记得检测 `NO_COLOR` 环境变量和 `stdout().is_terminal()`，否则重定向到文件时会混入乱码。

**4. 正则支持**：换成 `regex` crate，`Regex::new(query)?` 一次编译、逐行 `is_match`。注意用户输入的正则可能非法，要捕获 `regex::Error`。

**5. 并行搜索**：文件多时用 `rayon` 的 `par_iter`，或按第 21 章的线程分片。搜索是天然的 map-reduce：每个文件独立搜索，最后合并结果。

### 性能：三个容易忽略的点

| 反模式 | 后果 | 改法 |
|---|---|---|
| 每行 `to_lowercase()` | 每行一次堆分配 | 只在需要时构造，或用 `eq_ignore_ascii_case` |
| 直接 `println!` 每行 | 每次调用都锁 stdout | 用 `BufWriter` 或 `stdout().lock()` |
| 先 collect 全部行再遍历 | 整个文件进内存 | 用迭代器流式处理 |

`println!` 的开销值得单独说：标准输出的 `println!` 每次都会获取一次全局锁。本章的 `run` 为了简短就是逐行 `println!`——搜索结果通常只有几十行，这个代价可以忽略；输出量大时才值得换成下面这种写法（注意要在所有分支都拿到结果）：

```rust,noplayground
use std::io::{self, BufWriter, Write};

fn print_all(lines: &[&str]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in lines {
        writeln!(out, "{}", line)?;
    }
    out.flush()
}

fn main() {
    let lines = vec!["a", "b", "c"];
    print_all(&lines).unwrap();
}
```

`stdout.lock()` 持有锁直到 `out` 被 drop，`BufWriter` 把多次写入攒成一次系统调用。对百万行的输出，这能把耗时降低一个数量级。

## 39.8 深入：三个「为什么」

### 为什么 search 返回 Vec 而不是迭代器

更 Rust 的写法是返回 `impl Iterator<Item = (usize, &'a str)> + 'a`：

```rust
pub fn search_iter<'a>(query: &'a str, contents: &'a str) -> impl Iterator<Item = (usize, &'a str)> + 'a {
    contents
        .lines()
        .enumerate()
        .filter(move |(_, line)| line.contains(query))
        .map(|(i, line)| (i + 1, line))
}

fn main() {
    let text = "a\nrust b\nc";
    let total = search_iter("rust", text).count();
    println!("{}", total);
}
```

这是零分配的惰性版本：不 collect，调用方决定消费多少。缺点有两个：

1. 返回类型只能写 `impl Iterator`，调用方看不到具体类型，也就无法再利用（比如想 collect 成 `Vec` 得自己写）
2. 闭包里捕获了 `query`，所以迭代器带着 `query` 的借用，两个参数因此都标成 `'a`，返回类型还要写 `+ 'a` 告诉编译器「迭代器活不过 `'a`」

**取舍**：搜索结果一般要全部输出，交给 `main` 消费，`Vec` 的直观性比零分配更值钱。库函数被高频调用、调用方可能只需要前几条时，才值得返回迭代器。

### 为什么 lines() 不包含行尾换行

`str::lines()` 按 `\n` 切分并**去掉**行尾的 `\r\n`（Windows 换行）。这意味着：

```rust
fn main() {
    let s = "a\r\nb\n";
    let v: Vec<&str> = s.lines().collect();
    println!("{:?}", v);
    println!("行数 {}", v.len());
}
```

输出 `["a", "b"]` 和 `行数 2`。最后那个空行不算一行——这与 `wc -l` 的行为一致。如果你的输出需要保留原始换行符，用 `split_inclusive('\n')`。

### 为什么 `Box<dyn Error>` 能装下所有错误

`?` 在错误类型不匹配时会尝试 `From::from` 转换。标准库写了两条 blanket impl：`E: Error + 'a` 可以转成 `Box<dyn Error + 'a>`，`E: Error + Send + Sync + 'a` 可以转成 `Box<dyn Error + Send + Sync + 'a>`。也就是说 `Box<dyn Error>` 这一侧**不要求** `Send + Sync`；只有你显式写了 `+ Send + Sync` 的类型才会去要求它们。`io::Error` 两条都满足，所以 `run` 里的 `?` 用哪一种签名都能编译。

注意 `Box<dyn Error>` 默认不是 `Send`：如果这个 Future 要跨线程（第 25 章的 `tokio::spawn`），就得写 `Box<dyn Error + Send + Sync>`。

## 39.9 典型错误案例

### 案例 1：返回引用却不标注生命周期

```rust,editable
use std::fs;

fn search<'a>(query: &str, contents: &'a str) -> Vec<&str> {
    contents.lines().filter(|l| l.contains(query)).collect()
}

fn main() {
    let c = fs::read_to_string("poem.txt").unwrap();
    println!("{:?}", search("duct", &c));
}
```

```text
error[E0106]: missing lifetime specifier
 --> src/main.rs:3:54
  |
3 | fn search<'a>(query: &str, contents: &'a str) -> Vec<&str> {
  |                      ----            -------         ^ expected named lifetime parameter
  |
  = help: this function's return type contains a borrowed value, but the signature does not say whether it is borrowed from `query` or `contents`
help: consider using the `'a` lifetime
  |
3 | fn search<'a>(query: &str, contents: &'a str) -> Vec<&'a str> {
  |                                                       +=
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：返回值里的 `&str` 有两个可能的来源，编译器不会替你猜。生命周期省略规则只对「单个输入引用」或「&self 方法」生效，两个引用参数的函数必须显式标注。

**修复**：把返回值的生命周期绑到 `contents`：

```rust
use std::fs;

fn search<'a>(query: &str, contents: &'a str) -> Vec<&'a str> {
    contents.lines().filter(|l| l.contains(query)).collect()
}

fn main() {
    let c = fs::read_to_string("poem.txt").unwrap();
    println!("{:?}", search("duct", &c));
}
```

</details>

### 案例 2：返回借用局部变量的切片

```rust,editable
fn first_word(s: &str) -> &str {
    let lower = s.to_lowercase();
    lower.split_whitespace().next().unwrap()
}

fn main() {
    println!("{}", first_word("Hello World"));
}
```

```text
error[E0515]: cannot return value referencing local variable `lower`
 --> src/main.rs:3:5
  |
3 |     lower.split_whitespace().next().unwrap()
  |     -----^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  |     |
  |     returns a value referencing data owned by the current function
  |     `lower` is borrowed here
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`lower` 是函数内的局部 `String`，函数返回时它被 drop，返回的切片会指向已释放的内存。Rust 用借用检查器把这种悬垂引用挡在编译期。

**修复**：返回拥有所有权的 `String`，或者不在函数内创建临时字符串：

```rust
fn first_word_owned(s: &str) -> String {
    let lower = s.to_lowercase();
    lower.split_whitespace().next().unwrap().to_string()
}

fn first_word(s: &str) -> Option<&str> {
    s.split_whitespace().next()
}

fn main() {
    println!("{}", first_word_owned("Hello World"));
    println!("{:?}", first_word("Hello World"));
}
```

</details>

### 案例 3：在返回 () 的 main 里用 ?

```rust,editable
fn main() {
    let contents = std::fs::read_to_string("poem.txt")?;
    println!("{}", contents);
}
```

```text
error[E0277]: the `?` operator can only be used in a function that returns `Result` or `Option` (or another type that implements `FromResidual`)
 --> src/main.rs:2:55
  |
1 | fn main() {
  | --------- this function should return `Result` or `Option` to accept `?`
2 |     let contents = std::fs::read_to_string("poem.txt")?;
  |                                                       ^ cannot use the `?` operator in a function that returns `()`
  |
help: consider adding return type
  |
1 ~ fn main() -> Result<(), Box<dyn std::error::Error>> {
2 |     let contents = std::fs::read_to_string("poem.txt")?;
3 |     println!("{}", contents);
4 +     Ok(())
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`?` 的语义是「失败就 return Err」。返回 `()` 的函数没法 return `Err`，所以编译器禁止。

**修复**：让 main 返回 `Result`（失败时退出码自动变成 1），或改用 `ExitCode` 手动处理：

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string("src/main.rs")?;
    println!("读到 {} 字节", contents.len());
    Ok(())
}
```

</details>

### 案例 4：ExitCode 与整数混用

```rust,editable
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return 0;
    }
    ExitCode::SUCCESS
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:7:16
  |
4 | fn main() -> ExitCode {
  |              -------- expected `ExitCode` because of return type
...
7 |         return 0;
  |                ^ expected `ExitCode`, found integer
  |
help: call `Into::into` on this expression to convert `{integer}` into `ExitCode`
  |
7 |         return 0.into();
  |                 +++++++
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`ExitCode` 是 `process` 模块里的类型，不会和 `i32` 自动互转。`ExitCode::SUCCESS` 是 0，`ExitCode::FAILURE` 是 1，其他值要 `ExitCode::from(n: u8)`。

**修复**：

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    let code: u8 = 2;
    if code == 2 {
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
```

</details>

### 案例 5：把 Vec 移进函数后又使用

```rust,editable
use std::env;

#[derive(Debug)]
struct Config {
    query: String,
}

fn build(args: Vec<String>) -> Config {
    Config { query: args[1].clone() }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let config = build(args);
    println!("{}", args.len());
    println!("{:?}", config);
}
```

```text
error[E0382]: borrow of moved value: `args`
  --> src/main.rs:14:20
   |
12 |     let args: Vec<String> = env::args().collect();
   |         ---- move occurs because `args` has type `Vec<String>`, which does not implement the `Copy` trait
13 |     let config = build(args);
   |                        ---- value moved here
14 |     println!("{}", args.len());
   |                    ^^^^ value borrowed here after move
   |
note: consider changing this parameter type in function `build` to borrow instead if owning the value isn't necessary
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`build(args)` 按值接收 `Vec`，所有权被移进函数；`main` 之后不能再碰 `args`。这个报错本身把修复方案都说出来了。

**修复**：改成借用 `&[String]`（也正是 39.3 里 `Config::build` 的签名）：

```rust
#[derive(Debug)]
struct Config {
    query: String,
}

fn build(args: &[String]) -> Config {
    Config { query: args[1].clone() }
}

fn main() {
    let args = vec![String::from("minigrep"), String::from("rust")];
    let config = build(&args);
    println!("参数个数 {}", args.len());
    println!("{:?}", config);
}
```

</details>

## 完整源码

正文里的每一段代码都在仓库的 `projects/39-minigrep/` 里落成了能直接跑的文件（<https://github.com/yujh129/rust-book-zh/tree/main/projects/39-minigrep>）。不想跟着正文一行行敲，可以直接读、直接跑：

```console
$ cd projects/39-minigrep
$ cargo run --offline -- fast poem.txt
safe, fast, productive.
RUST is also fast.
```

| 文件 | 内容 |
| --- | --- |
| `src/lib.rs` | `Config`、`ConfigError`、`run`、`search`、`search_case_insensitive`，对应 39.3–39.5，末尾带 3 个单元测试 |
| `src/main.rs` | 29 行外壳：只收参数、调库、把结果翻译成退出码（2 参数错误、1 无匹配、3 读取失败） |
| `tests/cli.rs` | 4 个集成测试，真的启动二进制、断言 stdout 与退出码 |
| `poem.txt`、`tests/fixtures/poem.txt` | 手测和集成测试共用的小文本 |

`cargo test --offline` 会跑 3 个单元测试加 4 个集成测试。正文与源码之间以 `projects/39-minigrep/` 为准：如果某段正文和项目里的实现对不上，就是正文漏了一次同步，欢迎开 issue 指出来。

## 39.10 动手练习

1. **加一个 -c/--count 开关**：只输出命中行数，不输出内容。要求 `Config` 增加字段、`run` 的行为随之切换，并补一个单元测试。
2. **加一个 -n/--no-line-number 开关**：默认带行号，加上后只输出行内容。思考：两个开关的组合有四种，`Config` 会不会开始臃肿？什么时候该改用位标志或子命令结构？
3. **支持从 stdin 读取**：文件参数为 `-` 时读标准输入。提示：用 `std::io::Read::read_to_string(&mut buf)?` 而不是按路径读文件。
4. **多模式搜索**：允许传多个模式，命中任一即输出，并在输出中标出命中的是哪个模式。
5. **写集成测试**：为「参数错误退出码 2」「找不到退出码 1」「命中退出码 0」「非 UTF-8 文件退出码 3」各写一个 `tests/cli.rs` 用例。提示：非 UTF-8 文件可以用 `fs::write("bad.bin", [0xff, 0xfe])` 造出来。

## 39.11 小结

- 需求先写成 text 块：输入形式、输出格式、退出码，三样定死之后代码就有了验收标准。
- `lib.rs` + `main.rs` 的组合让核心逻辑可测试；集成测试只能链接库，`main` 里的逻辑必须搬进库。
- `Config::build(&[String]) -> Result<Config, ConfigError>` 比「在 main 里下标硬取」好得多：接收借用、返回可 match 的错误。
- 自定义错误实现 `Display` + `Error` 后可以被 `Box<dyn Error>` 装下，从而被 `?` 传播。
- 返回引用的函数有两个以上引用参数时必须显式标注生命周期，通常把返回值绑到「数据源」那个参数上。
- 返回临时值的切片会被 E0515 拦住；要么返回拥有所有权的值，要么别在函数内造临时字符串。
- `fn main() -> ExitCode` 让退出码成为类型系统的一部分；`ExitCode::from(2)` 对应 shell 的 `$? = 2`。
- 错误信息走 `eprintln!`，正常结果走 `println!`；脚本会区分这两条流。
- `env!("CARGO_BIN_EXE_<name>")` 是集成测试启动自己二进制的标准方式，不用猜 target 目录在哪。
- 想继续长大：目录递归、二进制检测、ANSI 高亮、正则、并行化——每一个都对应前面章节的一个知识点。