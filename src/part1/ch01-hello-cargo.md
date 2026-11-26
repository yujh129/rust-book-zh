# 第 1 章 Hello, Cargo：第一个 Rust 项目

> **本章目标**
> - 用 `cargo new` 创建项目，读懂 `Cargo.toml` 里每个字段的作用
> - 说清一次 `cargo run` 背后发生了什么：从源码到可执行文件经过哪几步
> - 掌握 `println!` 的完整格式化语法，理解它为什么必须是宏
> - 分清 `cargo check`/`build`/`run` 与 debug/release 的差异及代价
> - 写出一个能读输入、做计算、输出结果的命令行程序，并读懂它报的错

## 1.1 为什么第一件事是 Cargo

几乎每种语言都有一句"Hello, world"，但用什么方式写这句问候，决定了你后面怎么组织代码。直接调 `rustc main.rs` 可以编译一个 crate（也能加载其他模块文件），一旦要引第三方库、拆分模块、跑测试，就得手工管理一堆编译参数。

Cargo 把这些事情收进一个配置文件 `Cargo.toml`，剩下的交给命令。它不只是"包管理器"，而是 Rust 的**构建系统 + 依赖管理器 + 测试运行器 + 发布工具**。官方也把它当作事实标准，所以从第一个程序开始就用它。

```console
$ cargo new hello --bin --edition 2021
     Created binary (application) `hello` package
$ cd hello
$ tree .
.
├── Cargo.toml
└── src
    └── main.rs
```

两个文件的分工：

| 文件 | 作用 |
| --- | --- |
| `Cargo.toml` | 项目的"身份证 + 依赖清单"：名字、版本、edition、依赖、编译配置 |
| `src/main.rs` | 二进制 crate 的入口，必须有 `fn main` |

`Cargo.toml` 生成出来长这样：

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "2021"

[dependencies]
```

逐字段说明，这些字段你在真实项目里都会改：

| 字段 | 含义 | 注意 |
| --- | --- | --- |
| `name` | crate 名，也是生成的二进制名 | crates.io 接受 ASCII 字母、数字、`_`、`-`，常用小写；库代码中以 `_` 代替 `-` |
| `version` | 语义化版本 | 每次发布必须使用该包尚未用过的版本号 |
| `edition` | 语言版本，不是编译器版本 | 本书选 2021；当前 Cargo 新建项目默认选 2024 |
| `rust-version` | 支持的最低 Rust 版本（MSRV） | 写库时建议填，例如 `rust-version = "1.75"` |
| `authors` | 作者列表 | 仅展示用，crates.io 已不强制 |
| `license` | SPDX 许可证标识 | 发布 crates.io 必填，例如 `"MIT OR Apache-2.0"` |
| `description` | 一句话简介 | 发布必填，会显示在 crates.io 搜索结果里 |

依赖写法有几种形态，第 11 章会细讲，先认个脸：

```toml
[dependencies]
serde = "1"                                   # 版本区间，实际解析到最新 1.x
rand = { version = "0.8", features = ["small_rng"] }  # 开启可选特性
my_lib = { path = "../my_lib" }               # 本地路径依赖
tokio = { git = "https://github.com/tokio-rs/tokio" } # git 依赖

[dev-dependencies]
criterion = "0.5"                             # 只在测试/示例中可用
```

> **edition 是什么**：同一个编译器可以按不同版本的语法规则解析代码。2021 版把闭包捕获改成按字段捕获、允许 `array.into_iter()` 按值迭代；2024 版把 `gen` 变成关键字、要求 `unsafe extern`。同一依赖图可以混用 edition。迁移已有代码要检查不兼容之处，2024 需要 Rust 1.85 或更高版本。`{name}` 内联格式参数从 Rust 1.58 起在各 edition 均可使用。

## 1.2 第一行 Rust 代码

`src/main.rs` 的默认内容：

```rust
fn main() {
    println!("Hello, world!");
}
```

三个细节：

- `fn` 定义函数，`main` 是程序入口，签名为"无参数、返回 `()`"。C 语言里 `main` 要返回 `int`，Rust 不需要，进程退出码默认 0。
- `println!` 后面的感叹号表示**宏调用**，不是函数调用。
- 语句以分号结尾。表达式与语句的区别在第 3 章展开，现在先记住：写在一行末尾的调用后面要有分号。

运行它：

```console
$ cargo run
   Compiling hello v0.1.0 (/path/to/hello)
    Finished dev [unoptimized + debuginfo] target(s) in 0.31s
     Running `target/debug/hello`
Hello, world!
```

第一次 `cargo run` 会编译，之后再运行只花几十毫秒——Cargo 只在源文件或依赖变化时重新编译。

### 深入：一次 cargo run 背后发生了什么

```text
cargo run
  ├─ 读取 Cargo.toml，解析依赖图（Cargo.lock 锁定版本）
  ├─ 检查 target/debug/.fingerprint/ 里的哈希：源码、依赖、编译参数、rustc 版本
  │    └─ 都没变 → 直接跳到运行，这就是"二次运行几十毫秒"的原因
  ├─ 为每个依赖 crate 调 rustc，产出 .rlib（静态库）
  └─ 为自己的 crate 调 rustc：
       lexer → AST → HIR → 类型检查 → MIR → 借用检查 → LLVM IR → 目标文件 → 链接
                                                              ↑ 这一格最慢
```

几个值得知道的点：

- **Cargo 按目标决定是否调用 rustc**。改动库后，依赖它的目标也可能重编；启用增量编译时，rustc 能复用一个 crate 内部未变的查询与代码生成结果。这就是为什么 Rust 项目习惯拆成多个小 crate：编译可以并行，改动只影响局部。
- `target/debug/deps/` 里是编译产物与依赖，`target/debug/incremental/` 是增量缓存，`target/debug/.fingerprint/` 记录"什么输入产生了什么输出"。`cargo clean` 删的就是整个 `target/`。
- **借用检查发生在 MIR 上**（`mir borrowck`），比类型检查晚，所以有些报错会先出现类型错误、修完才暴露借用错误。

## 1.3 宏与函数：为什么 println! 带感叹号

`println!` 支持变参，而且格式串在**编译期**就被检查：

```rust
fn main() {
    let name = "世界";
    let pi = 3.14159;
    let n = 42;

    println!("你好，{}！", name);            // 位置参数
    println!("{name} 的圆周率是 {pi:.2}");   // 内联变量 + 两位小数
    println!("{n:>8}|{n:<8}|{n:^8}|");       // 右对齐、左对齐、居中，总宽 8
    println!("{n:08} {n:b} {n:o} {n:x} {n:X}"); // 补零、二进制、八进制、十六进制
    println!("{pi:e} {n:+}", );              // 科学计数法、强制带符号
    println!("{{}} 是字面量大括号");          // 转义
    println!("{:?}", vec![1, 2, 3]);          // Debug 输出
    println!("{:#?}", (1, "two"));            // 美化 Debug
}
```

```console
$ cargo run
你好，世界！
世界 的圆周率是 3.14
      42|42      |   42   |
00000042 101010 52 2a 2A
3.14159e0 +42
{} 是字面量大括号
[1, 2, 3]
(
    1,
    "two",
)
```

普通函数做不到"参数个数可变 + 格式串编译期校验"同时成立，所以标准库把它实现成宏。写成 `println(...)`（没有 `!`）会得到：

```text
error[E0423]: expected function, found macro `println`
 --> src/main.rs:2:5
  |
2 |     println("hi");
  |     ^^^^^^^ not a function
  |
help: use `!` to call the macro
  |
2 |     println!("hi");
  |            +
```

编译器连改法都告诉你了。Rust 报错里的 `help` 段经常直接给出可复制的修改建议，读报错时优先看它。

**输出走哪条流**：`println!` 写标准输出，`eprintln!` 写标准错误。终端里两者都显示，但重定向时不一样：`cargo run > out.txt` 只会把 stdout 写进文件，错误信息仍然打在屏幕上。调试时还有个专门给开发者的宏：

```rust
fn main() {
    let x = 10;
    let y = dbg!(x * 2) + 1;   // 打印 "[src/main.rs:3] x * 2 = 20" 到 stderr，并返回 20
    println!("{y}");
}
```

`dbg!` 会带上文件名、行号和表达式原文，比手写 `println!("x = {x}")` 更省事，而且**返回值是参数本身**，可以直接嵌进表达式。

## 1.4 编译、检查与优化

| 命令 | 产物 | 什么时候用 |
| --- | --- | --- |
| `cargo check` | 只做类型检查，不生成可执行文件 | 写代码过程中反复验证，比 build 快数倍 |
| `cargo build` | `target/debug/hello` | 需要运行或调试时 |
| `cargo run` | 先 build 再运行 | 日常开发 |
| `cargo build --release` | `target/release/hello` | 交付、压测、基准测试 |
| `cargo clean` | 删除 `target/` | target 占了几个 G 的时候 |

debug 与 release 不只是快慢：

| | debug | release |
| --- | --- | --- |
| 优化级别 | `opt-level = 0` | `opt-level = 3` |
| 整数溢出 | 运行时 panic | 静默回绕（wrapping） |
| 调试符号 | 有 | 默认无 |
| 断言 `debug_assert!` | 生效 | 被移除 |
| 编译速度 | 快 | 慢 |
| 运行速度 | 慢（常见 5～20 倍差距） | 快 |

第二行是容易踩的坑：**debug 下 `i32::MAX + 1` 会 panic，release 下会变成负数**。涉及边界值的逻辑要么用 `checked_add`，要么在 `Cargo.toml` 里固定行为：

```toml
[profile.release]
overflow-checks = true
```

顺带认识一下 profile：它决定"用哪套优化与调试参数编译"。常用的是 `dev`（`cargo run` 默认）和 `release`。可以自定义一个更快的开发配置：

```toml
[profile.dev]
opt-level = 1        # 自己代码轻度优化，依赖全优化
[profile.dev.package."*"]
opt-level = 3
```

这对游戏、图像处理这类"debug 下慢到没法调"的项目很有用。

## 1.5 变量默认不可变

先把最容易让新手困惑的一点摆出来。Rust 的 `let` 默认创建**不可变**绑定：

```rust,noplayground
fn main() {
    let x = 5;
    println!("x = {x}");
    x = 6; // 这里会编译失败
}
```

```text
error[E0384]: cannot assign twice to immutable variable `x`
 --> src/main.rs:4:5
  |
2 |     let x = 5;
  |         - first assignment to `x`
  |         |
  |         help: consider making this binding mutable: `mut x`
4 |     x = 6;
  |     ^^^^^ cannot assign twice to immutable variable
```

加上 `mut` 就能改：

```rust
fn main() {
    let mut x = 5;
    x = 6;
    println!("x = {x}"); // 6
}
```

为什么默认不可变？因为"这个值之后不会被改"是一个**能被编译器验证的承诺**。读代码时不用追踪它被谁改过；并发时也不用担心数据竞争。需要改时显式写 `mut`，等于在类型层面标注"这里是有状态的"。第 2 章会展开可变性与遮蔽（shadowing）的区别。

## 1.6 main 也可以有返回值

`main` 的签名不是必须写 `-> ()`。只要返回类型实现了 `std::process::Termination`，就可以直接返回：

```rust
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string("Cargo.toml")?;
    println!("读到了 {} 字节", content.len());
    Ok(())
}
```

失败时进程退出码是 1，并打印 `Error: ...`。这比在 `main` 里到处 `match` 再 `process::exit(1)` 干净得多，第 27 章会系统讲这种写法。

需要精确控制退出码时，仍然用 `std::process::exit`：

```rust
fn main() {
    let ok = false;
    if !ok {
        eprintln!("出错了");
        std::process::exit(2);   // 之后的代码不会执行，也不会运行析构函数
    }
}
```

注意 `exit` 会**直接终止进程，不执行栈上变量的 Drop**。需要清理资源时，让 `main` 正常返回而不是调 `exit`。

## 1.7 第一个有用的程序：BMI 计算器

下面这个程序把输入、解析、计算、输出串起来，正好用到后面几章的核心概念。先看代码，不必每个细节都懂。

```rust
use std::io;

fn main() {
    println!("BMI 计算器");

    println!("请输入身高（米），例如 1.75：");
    let mut height = String::new();
    io::stdin()
        .read_line(&mut height)
        .expect("读取身高失败");
    let height: f64 = height.trim().parse().expect("身高必须是数字");

    println!("请输入体重（千克），例如 68：");
    let mut weight = String::new();
    io::stdin()
        .read_line(&mut weight)
        .expect("读取体重失败");
    let weight: f64 = weight.trim().parse().expect("体重必须是数字");

    let bmi = weight / (height * height);
    let level = if bmi < 18.5 {
        "偏瘦"
    } else if bmi < 24.0 {
        "正常"
    } else if bmi < 28.0 {
        "偏胖"
    } else {
        "肥胖"
    };

    println!("你的 BMI 是 {bmi:.1}，属于{level}");
}
```

```console
$ cargo run
BMI 计算器
请输入身高（米），例如 1.75：
1.75
请输入体重（千克），例如 68：
68
你的 BMI 是 22.2，属于正常
```

逐段解释，每一条都会在后面的章节展开：

1. `use std::io;` 把标准库的 `io` 模块引入作用域。Rust 默认只预导入很少的东西，`io` 不在其中（第 11 章讲模块系统）。
2. `String::new()` 创建空的、可增长的 UTF-8 字符串。`mut` 是必须的，因为下一行要往里读数据。
3. `read_line(&mut height)` 要求**可变引用**，因为它要修改这个字符串。`&` 与 `&mut` 的区别是第 6 章的主题。
4. `read_line` 返回 `Result`，表示"可能失败"。`.expect(...)` 失败时 panic 并打印消息。第 10 章讲怎样优雅处理而不是 panic。
5. `height.trim()` 去掉行尾换行符（`read_line` 会把 `\n` 一起读进来），`.parse()` 把字符串转成数字，同样返回 `Result`。
6. `let height: f64 = ...` 用类型标注告诉编译器 `parse` 转成什么。`parse` 是泛型的，不标注就不知道目标类型。
7. `if` 在这里是**表达式**，能直接产生值赋给 `level`。这是 Rust 与 C 系语言的明显差异：不需要三元运算符。

## 典型错误案例

### 案例 1：漏了分号

```rust,editable
fn main() {
    let x = 5
    println!("{x}");
}
```

```text
error: expected `;`, found `println`
 --> src/main.rs:3:5
  |
2 |     let x = 5
  |              - help: add `;` here: `;`
3 |     println!("{x}");
  |     ^^^^^^^
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：Rust 的语句必须以分号结束，而 `let` 是语句不是表达式。`-` 标记的位置往往就是漏分号的地方。
**修复**：`let x = 5;`。如果只是想让一个表达式的结果被丢弃，最后一行不带分号才是对的（`{` 块的值）。


修复后的完整代码：

```rust,editable
fn main() {
    let x = 5;
    println!("{x}");
}
```

</details>

### 案例 2：宏少了感叹号

```rust,editable
fn main() {
    println("hello");
}
```

```text
error[E0423]: cannot find function `println` in this scope
 --> src/main.rs:2:5
  |
2 |     println("hello");
  |     ^^^^^^^ not found in this scope
  |
  = note: a macro named `println` exists in another namespace
help: use `!` to invoke the macro
  |
2 |     println!("hello");
  |            +
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`println` 是宏不是函数，宏调用必须带 `!`。同类错误还有忘记给 `vec!`、`format!`、`assert!` 加感叹号。
**修复**：`println!("hello");`。


修复后的完整代码：

```rust,editable
fn main() {
    println!("hello");
}
```

</details>

### 案例 3：main 的返回类型不对

```rust,editable
fn main() -> i32 {
    println!("hi");
}
```

```text
error[E0277]: `main` has invalid return type `i32`
 --> src/main.rs:1:14
  |
1 | fn main() -> i32 {
  |              ^^^ `main` can only return types that implement `Termination`
  |
  = help: consider using `()`, or a `Result`

error[E0308]: mismatched types
 --> src/main.rs:1:14
  |
1 | fn main() -> i32 {
  |    ----      ^^^ expected `i32`, found `()`
  |    |
  |    implicitly returns `()` as its body has no tail or `return` expression
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`main` 只能返回 `()`、`Result<(), E>` 或实现了 `Termination` 的类型（如 `ExitCode`）。
**修复**：返回 `()` 并用 `process::exit` 设置退出码，或改成 `fn main() -> Result<(), Box<dyn std::error::Error>>`。


修复后的完整代码：

```rust,editable
fn main() {
    println!("hi");
}
```

</details>

### 案例 4：不在项目根目录

```console
$ cargo run
error: could not find `Cargo.toml` in `/home/me` or any parent directory
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：Cargo 需要从当前目录向上找 `Cargo.toml`。你在项目外，或者 `cd` 进了子目录但子目录不是独立 crate。
**修复**：`cd` 到含 `Cargo.toml` 的目录。注意：如果子目录里也有一个 `Cargo.toml`（比如 workspace 成员），那就是"另一个项目"了，Cargo 会用最近的那个。


修复：回到含 `Cargo.toml` 的目录再跑

```console
$ cd my-first-project    # 换成含 Cargo.toml 的那一层
$ cargo run
```

</details>

### 案例 5：一个文件两个 main

```rust,editable
fn main() {
    println!("first");
}

fn main() {
    println!("second");
}
```

```text
error[E0428]: the name `main` is defined multiple times
 --> src/main.rs:5:1
  |
2 | fn main() {
  | --------- previous definition of `main` here
5 | fn main() {
  | ^^^^^^^^^ `main` redefined here
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：同一个 crate 里入口只能有一个。常见于复制粘贴示例代码时把两段 `main` 都留下了。
**修复**：只保留一个 `main`，另一个改名为普通函数并用 `#[allow(dead_code)]` 或直接删掉。


修复后的完整代码：

```rust,editable
fn main() {
    println!("first");
    second();
}

fn second() {
    println!("second");
}
```

</details>

## 动手练习

1. 改 `hello` 程序，读入你的名字并打印 `你好，<名字>！`。复用 BMI 程序里 `read_line` 的写法，记得 `trim()`。
2. 给 BMI 程序加单位换算：允许输入厘米（如 `175`），数值大于 3 时按厘米处理。想一想，"大于 3"这个阈值为什么合理？
3. 把 BMI 的分级阈值和文案挪进一个函数，让 `main` 只负责输入输出。现在还没学结构体和枚举，用 `if` 和切片完成即可。
4. 用 `dbg!` 和 `println!("{:#?}")` 观察一个嵌套元组 `(1, ("a", [true; 2]))` 的输出差异。

## 小结

- `cargo new` 生成的项目由 `Cargo.toml`（元数据与依赖）和 `src/main.rs`（入口）组成。
- 一次编译的单位是 crate；`target/debug/.fingerprint` 记录增量依据，改动库也可能触发下游目标重编。
- `println!` 是宏，格式串在编译期检查；`{}` 走 Display，`{:?}` 走 Debug，`dbg!` 打印带位置信息到 stderr。
- `cargo check` 只做类型检查，开发时比 `build` 快；交付用 `--release`，普通整数加法的溢出检查由 profile 配置决定。
- `let` 默认不可变，需要修改时写 `let mut`，这是编译器能验证的承诺。
- `main` 可以返回 `Result` 或 `ExitCode`；`process::exit` 不执行析构。
