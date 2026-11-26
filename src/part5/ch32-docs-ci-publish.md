# 第 32 章 文档、lint、CI 与发布

> **本章目标**
>
> - 会写 `///` 与 `//!` 两种文档注释，并用 `cargo doc --open` 看到它们变成什么样的 HTML
> - 能说清文档测试（doctest）被 `cargo test --doc` 挑出来、编译、运行的完整过程，以及 `no_run`、`ignore`、`should_panic`、隐藏行 `#` 各自的作用
> - 会用 `#![deny(missing_docs)]`、`RUSTDOCFLAGS="-D warnings"`、`#[doc(hidden)]` 把文档质量变成 CI 门禁
> - 能按语义化版本（SemVer）给 crate 定版本号，说清 `0.x` 的特殊性、MSRV 与 `Cargo.lock` 该不该提交
> - 会写一条 fmt + clippy + test 矩阵（stable/beta 乘三个操作系统）的 GitHub Actions 流水线
> - 走一遍 `cargo package --list` 到 `cargo publish --dry-run` 的发布检查，知道发错了只能 yank、不能删

代码能不能跑，测试会告诉你。但一个 crate 还有第二个使用者：读文档的人。他们不读你的测试，只读你写的文档，照着示例粘贴运行。文档和代码一旦分家，示例就成了最隐蔽的 bug——编译器不检查注释，测试也不检查注释。

Rust 的应对办法很直接：把文档放进编译器能看见的地方。文档注释不是普通注释，而是属性的语法糖；文档里的代码块不是排版素材，而是独立的测试 crate。于是"文档过期"这件事从"靠自觉"变成了"靠 CI"：示例编译不过，流水线就红。

这一章把这条链路走完：写文档注释、让文档测试跑起来、用 lint 和格式统一风格、把这一切挂到 CI 上，最后把 crate 发到 crates.io。它偏流程和配置，没有新的类型系统概念，但每一节都对应一个真实会让你在发布当天手忙脚乱的问题。

## 32.1 文档注释：///、//! 与 rustdoc

Rust 有两种文档注释，区别只在"注释管辖谁"：

- `///` 是**外层**文档注释，作用于紧随其后的那一项——函数、结构体、字段、模块都行。
- `//!` 是**内层**文档注释，作用于包含它的那一项，所以通常写在文件最顶部（对应 crate 根）或模块文件的开头。

```rust
//! 一组温度换算函数。
//!
//! 这段 `//!` 写在文件顶部，会出现在 crate 文档的首页。

/// 摄氏度转华氏度。
///
/// # 示例
///
/// ```
/// assert_eq!(celsius_to_fahrenheit(0.0), 32.0);
/// ```
pub fn celsius_to_fahrenheit(c: f64) -> f64 {
    c * 9.0 / 5.0 + 32.0
}

/// 华氏度转摄氏度。
pub fn fahrenheit_to_celsius(f: f64) -> f64 {
    (f - 32.0) * 5.0 / 9.0
}

fn main() {
    println!("{}", celsius_to_fahrenheit(37.0));
}
```

在编译器眼里，`/// 文本` 就是 `#[doc = "文本"]`，`//! 文本` 就是 crate 根上的 `#[doc = "文本"]`。理解这层等价关系，就能理解为什么文档能做属性做的事：`#[doc(hidden)]` 把项从文档里藏起来，`#[doc(alias = "length")]` 给搜索加别名（用户搜 length 也能找到 `len`），`#[doc(cfg(feature = "tls"))]` 标注这一项只在某个 feature 下存在。

注释正文按 Markdown 解析：标题、列表、表格、图片、链接都可以用。rustdoc 还认几个约定小节名，社区和 lint 都按它们找内容：

| 小节 | 写什么 |
| --- | --- |
| 示例 | 最常见的用法，代码块会变成 doctest |
| Panics | 什么条件下 panic，调用方据此决定要不要提前检查 |
| Errors | 返回 Result 时，什么情况是 Err |
| Safety | unsafe 函数的调用者必须保证什么 |

生成 HTML 用 `cargo doc`：

```console
$ cargo doc --no-deps
 Documenting doc_demo v0.1.0 (/path/to/doc_demo)
    Finished dev profile [unoptimized + debuginfo] target(s) in 0.67s
   Generated /path/to/doc_demo/target/doc/doc_demo/index.html
```

`--no-deps` 只给你自己的 crate 生成文档，依赖的文档网上已经有了；`--open` 生成后自动用浏览器打开首页。加 `--document-private-items` 连私有项一起生成，内部工具 crate 常用。

产物落在 `target/doc`，结构比想象中简单：

```text
target/doc/
├── doc_demo/
│   ├── index.html             # crate 首页，即 //! 的内容
│   ├── struct.Counter.html    # 每一个公开项一个页面
│   ├── fn.helper.html
│   ├── all.html               # 所有项汇总在一页
│   └── sidebar-items.js       # 侧栏目录数据
├── search.index/              # 搜索索引，按类型切成多个 js 分片
└── help.html
```

文件名是"种类 + 名字"：`struct.Counter.html`、`fn.helper.html`。这意味着改类型名会改文件名，外部链接会断；真要分享链接，用 crate 名加项名的锚点，别写死某个 HTML 文件名。`search.index` 目录说明搜索是纯前端 JS 做的：页面加载后按需拉分片，所以离线打开 `index.html` 也能搜。

## 32.2 文档测试：注释里的代码会被编译运行

文档里最容易过期的就是示例。Rust 的解法是让它们参与编译：`cargo test` 会在跑完单元测试和集成测试之后，再跑一遍文档测试（doc test），把每个文档代码块当作独立的小程序。

先看机制。rustdoc 抽取每个代码块时会做三件事：

1. 新建一个匿名 crate，把代码块内容放进去；
2. 在开头注入 `extern crate 你的库名;`，所以块里能直接用库的公开 API；
3. 如果块里没有 `fn main` 这一项，就在外面套一层 `fn main` 函数。

第 3 条解释了为什么文档示例可以写裸语句。下面这种代码在普通 crate 里编译不过，在 doctest 里天经地义：

```rust,noplayground
/// 统计不同单词的个数。
///
/// ```
/// let mut counts = HashMap::new();
/// counts.insert("a", 1);
/// assert_eq!(counts.len(), 1);
/// ```
pub fn count() {}
```

rustdoc 检测的是"有没有 `fn main` 这一项"，不是字符串匹配。写在注释或字符串里的 `fn main` 不影响判断，所以块里可以放心谈论 `main` 函数。第 3 条还有反向前提：包必须有 lib 目标，纯二进制 crate 没有文档测试，32.3 会给出真实报错。

代码块可以加属性决定怎么处理：

| 属性 | 编译 | 运行 | 典型用途 |
| --- | --- | --- | --- |
| 无 | 是 | 是 | 默认，纯计算示例 |
| no_run | 是 | 否 | 要联网、读写文件、耗时很长 |
| ignore | 否 | 否 | 依赖不存在的环境，或只是伪代码 |
| should_panic | 是 | 是，且必须 panic | 演示 panic 契约 |
| compile_fail | 是，且必须编译失败 | 否 | 演示类型系统挡住了某种写法 |

`no_run` 和 `ignore` 的区别常被搞混：前者仍然会**编译**，API 一改名它就报错，只是不执行；后者连编译都跳过，等于把示例从测试里摘出去。能用 `no_run` 就别用 `ignore`。

下面这个库把四种属性都用了一遍：

```rust,noplayground
//! 文档属性演示。

/// 只编译不运行。
///
/// ```no_run
/// let v: Vec<u8> = (0..1000).map(|x| x as u8).collect();
/// assert_eq!(v.len(), 1000);
/// ```
pub fn heavy() {}

/// 需要外部服务，标记忽略。
///
/// ```ignore
/// let body = reqwest::blocking::get("https://example.com").unwrap().text().unwrap();
/// assert!(!body.is_empty());
/// ```
pub fn fetch() {}

/// 应当 panic。
///
/// ```should_panic
/// panic!("boom");
/// ```
pub fn explode() {}

/// 隐藏辅助行。
///
/// ```
/// # let secret = 41;
/// let answer = secret + 1;
/// assert_eq!(answer, 42);
/// ```
pub fn hidden() {}
```

`cargo test --doc` 的输出会直接把每个块的状态列出来：

```console
$ cargo test --doc
   Doc-tests attr_demo

running 4 tests
test src/lib.rs - fetch (line 13) ... ignored
test src/lib.rs - heavy (line 5) - compile ... ok
test src/lib.rs - explode (line 21) ... ok
test src/lib.rs - hidden (line 28) ... ok

test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.63s
```

读法：`heavy` 后面多出的 `- compile` 就是 `no_run` 的标记；`ignored` 表示 `ignore`；`hidden` 那块虽然少了一行，照样编译运行。名字里的 `(line 13)` 是代码块起始行在源文件中的行号，改动上方文档时它会变，不要把它当稳定标识写进脚本。

**隐藏行**是写文档示例最实用的技巧。行首写 `# `，这一行不会出现在渲染后的 HTML 里，但参与编译；单独一个 `#` 渲染成空行；要显示字面的 `#`，写成 `##`。它一般用来藏三样东西：`use` 语句、错误处理的 `unwrap`、以及让示例能跑起来的样板代码。上面 `hidden` 那个例子里，读者只看到两行计算，而 `secret` 的来源被藏了起来。

最后是代价。每个文档块都是一个独立 crate：库本身编译一次，块有多少个就要再编译多少次。示例多的大库，`cargo test --doc` 比单元测试慢一个数量级很正常，所以 CI 里值得单独跑、单独看耗时。

## 32.3 文档测试的三个坑

文档测试跑的是"别人眼里的你的库"，所以它踩的坑和下游用户踩的坑是同一批。

### 坑一：doctest 是一个独立 crate

代码块里写的名字，必须能从"外部"看见。`crate::` 在 doctest 里指向的是那个临时 crate，不是你的库；要引用库里的项，只能写库名。库名默认等于包名，但如果 `Cargo.toml` 里有 `[lib] name = "wordtools"`，或者依赖被 `package =` 重命名过，就得用真正的 lib 名——案例 1 会给出这条的逐字报错。

第二个高频问题是漏写 `use`。doctest 的预导入（prelude）和你平时写代码时一样，只有 `std::prelude` 那几十个名字，`HashMap`、`Arc`、`Duration` 都不在里面：

```rust,noplayground
/// 统计不同单词的个数。
///
/// ```
/// let mut counts = HashMap::new();
/// counts.insert("a", 1);
/// assert_eq!(counts.len(), 1);
/// ```
pub fn count() {}
```

```text
error[E0433]: cannot find type `HashMap` in this scope
 --> src/lib.rs:6:18
  |
6 | let mut counts = HashMap::new();
  |                  ^^^^^^^ use of undeclared type `HashMap`
  |
help: consider importing this struct
  |
5 + use std::collections::HashMap;

error: aborting due to 1 previous error

Couldn't compile the test.
```

修法有两种：在示例里老实写 `use std::collections::HashMap;`，或者用隐藏行 `# use std::collections::HashMap;` 藏起来。后者更好——读者关心的是 API 怎么用，不是导入清单。注意报错里的行号是**源文件行号**，`--> src/lib.rs:6:18` 指向的正是 `///` 里那一行，照着改就行。

### 坑二：只能看到公开 API

doctest 链接的是编译产物，权限和下游用户完全一样。`pub(crate)` 的函数、私有模块里的类型，在示例里都摸不到。这不是限制，而是提醒：文档示例天然就是对外契约的一部分，示例里出现的东西，读者会当成公开用法。

确实需要演示内部机制时，别为了文档把 API 改成 `pub`。两条正路：给内部逻辑写 `#[cfg(test)]` 单元测试（第 26 章），或者单独维护一份开发者文档。把内部项 `pub` 出去只为了写示例，等于给未来的自己加了一份 semver 债务。

### 坑三：二进制 crate 没有文档测试

这条最容易被忽略，因为 `cargo test` 平时不报错——它只是不跑文档测试：

```console
$ cargo test --doc
error: no library targets found in package `bin_demo`
```

doctest 需要链接一个库目标，纯 `src/main.rs` 的包没有。所以"把逻辑放 `src/lib.rs`、`main` 只做参数解析和调用"不只是为了可测试性（第 26 章），也是为了让文档示例能被验证。命令行工具通常都这么分：`src/lib.rs` 放实现，`src/main.rs` 十行以内。

## 32.4 把文档质量变成门禁

写作规范靠人记不住，靠工具才记得住。rustdoc 自带一组 lint，其中两个最值得开：

- `missing_docs`：公开项没有文档就报警。
- `broken_intra_doc_links`：文档里的 `[Foo]` 链接解析不到目标就报警。

在 crate 根加一行，把第一个升级成错误：

```rust
//! 全部公开项都必须有文档。
#![deny(missing_docs)]

/// 把字节数格式化成人类可读的字符串。
pub fn human_bytes(n: u64) -> String {
    format!("{} B", n)
}

fn main() {
    println!("{}", human_bytes(1024));
}
```

`#![deny(missing_docs)]` 这类 crate 级属性必须写在文件最前面、`//!` 之后。只想对某个模块放宽时，在模块上加 `#[allow(missing_docs)]`，别把整条 lint 关掉。

两个 lint 同时触发时，`cargo doc` 的输出是这样：

```text
 Documenting doc_demo v0.1.0 (/path/to/doc_demo)
error: missing documentation for a function
  --> src/lib.rs:24:1
   |
24 | pub fn helper() {}
   | ^^^^^^^^^^^^^^^
   |
note: the lint level is defined here
  --> src/lib.rs:2:9
   |
 2 | #![deny(missing_docs)]
   |         ^^^^^^^^^^^^

error: unresolved link to `Self::zero`
  --> src/lib.rs:18:16
   |
18 |     /// 清零，参见 [Self::zero]。
   |                     ^^^^^^^^^^ the struct `Counter` has no field or associated item named `zero`
   |
note: the lint level is defined here
  --> src/lib.rs:3:9
   |
 3 | #![deny(rustdoc::broken_intra_doc_links)]
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

error: could not document `doc_demo`
```

注意最后一行 `error: could not document`：文档生成失败，命令退出码非 0，CI 直接红。这条链路比"文档写得不好"重要得多——它是可执行的。

CI 里还有一层保险：`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`。它把 rustdoc 的**所有**警告都当错误，包括你还没见过的新 lint。官方 crate 常用的做法是本地只开 `deny`，CI 用 `-D warnings` 收口。

最后说 `#[doc(hidden)]`。它只影响渲染，不影响可见性：被隐藏的项仍然是 `pub`，下游仍然能调用，仍然受 semver 约束。它正确的用途是"公开但不想让人依赖"的实现细节，比如宏展开后必须可见的辅助项、`__private` 模块。把它当成"私有"来用，迟早会被下游的 `use` 打脸。

## 32.5 版本号是承诺：SemVer、0.x、MSRV 与 Cargo.lock

版本号不是流水号，是契约：主版本变，说明有破坏性改动；次版本变，说明只加了东西；修订号变，说明只修了 bug。Rust 生态把这套语义化版本（Semantic Versioning，SemVer）写进了 Cargo 的依赖解析规则里。

Cargo 的版本要求默认带插入符（caret）语义，含义随主版本号变化：

| 写在 Cargo.toml 里 | 实际接受的范围 | 含义 |
| --- | --- | --- |
| 1.2.3 | >=1.2.3, <2.0.0 | 主版本相同就兼容 |
| 0.2.3 | >=0.2.3, <0.3.0 | 0.x 时次版本当主版本用 |
| 0.0.3 | >=0.0.3, <0.0.4 | 几乎等于精确锁定 |

这就是 `0.x` 的特殊性：**API 还没定型时，次版本号承担主版本的职责**。`0.1` 升到 `0.2` 允许破坏兼容，而 `0.2.3` 升到 `0.2.4` 不允许。很多库在 `0.1` 上待很久，就是在攒这个"可以破坏"的窗口；也有人一上来就发 `1.0`，等于承诺此后只做加法。

哪些改动算破坏性？判断标准只有一个：**下游已有的代码会不会编译不过**。

- 删掉或重命名公开项、改函数签名、收紧泛型约束：破坏。
- 给 trait 加一个没有默认实现的方法：破坏，所有实现者都要补。
- 给枚举加变体、给结构体加公开字段：破坏，除非标了 `#[non_exhaustive]`。
- 新增一个公开函数、给已有函数加 `impl Trait` 参数（不改变调用方式）：兼容。
- 只改内部实现、改 panic 的触发条件：语义上兼容，但要在 CHANGELOG 里写清楚。

`#[non_exhaustive]` 是给未来留门的属性：

```rust
#[non_exhaustive]
pub enum Status {
    Ok,
    Retry,
}

fn main() {
    let s = Status::Ok;
    match s {
        Status::Ok => println!("ok"),
        Status::Retry => println!("retry"),
    }
}
```

在定义它的 crate 内部，穷尽匹配仍然合法；对下游来说，编译器会强制 `match` 带上 `_` 分支，构造带该属性的结构体时必须写 `..Default::default()`。代价是下游少了一点编译期检查，换来的是你能在下个次版本里加变体而不算破坏。

**MSRV**（Minimum Supported Rust Version，最低支持的 Rust 版本）写在 `[package]` 里：

```toml
[package]
name = "wordtools"
version = "0.2.3"
edition = "2021"
rust-version = "1.82"
```

`rust-version` 不是装饰。cargo 1.84 起，解析依赖时会优先挑选与这个版本兼容的依赖版本，避免"我的库声称支持 1.82，却拉进一个要求 1.85 的依赖"。但**没有任何工具会替你验证它**：你必须在 CI 里用 1.82 的工具链真跑一遍，否则这行就是一句没人核对的声明。

`Cargo.lock` 该不该提交，按"谁读这个 lock"判断：

- 二进制、应用、工作区：提交。部署要可复现，两次构建必须拉到同一批依赖。
- 纯库：不提交也无所谓，因为下游只用你的版本要求，不会用你的 lock。但团队 CI 想稳定，提交也没坏处。
- 想强制使用 lock 时，`cargo build --locked` 会在 lock 缺失或需要更新时直接失败，Docker 镜像和 CI 里常用。

## 32.6 格式与 lint：rustfmt、clippy 与 allow 的克制

格式争论不值得开会。rustfmt 把"怎么排版"变成一条命令，clippy 把"哪种写法更容易出错"变成一组可执行的检查。两者都不改变语义，只统一风格。

### cargo fmt

`cargo fmt` 直接改写源文件，`cargo fmt --check` 只检查、不改写，CI 用后者。不通过时的输出是标准的 diff：

```text
Diff in src/main.rs:1:
 fn main() {
-    let nums = vec![1,2,3];
-  let total:i32 = nums.iter().sum();
-        println!("{}",total);
-    if total > 0 { println!("positive"); }
+    let nums = vec![1, 2, 3];
+    let total: i32 = nums.iter().sum();
+    println!("{}", total);
+    if total > 0 {
+        println!("positive");
+    }
 }
```

`-` 是当前文件，`+` 是 rustfmt 想要的样子，照着改或者直接跑一次 `cargo fmt`。几个要点：

- rustfmt 的规则可以调，配置文件叫 `rustfmt.toml`，放在 crate 根或工作区根。常用的只有 `max_width`、`edition`；其余选项大多是 nightly-only，别在稳定版项目里配。
- 确实有排版不能动的代码（对齐的矩阵、生成的表），在项上加 `#[rustfmt::skip]`，别整个文件排除。
- `cargo fmt` 会大范围改文件，和别人的分支必然冲突。团队约定通常是"格式化单独一次提交，不和逻辑改动混在一起"，这样 code review 的 diff 才有意义。

### cargo clippy

clippy 是 rustc 的 lint 插件，内置几百条规则，按组划分：

| 组 | 默认级别 | 内容 |
| --- | --- | --- |
| clippy::correctness | deny | 几乎肯定是 bug 的写法 |
| clippy::suspicious | warn | 看起来可疑，多半写错了 |
| clippy::style | warn | 更符合习惯的等价写法 |
| clippy::complexity | warn | 可以写得更简单 |
| clippy::perf | warn | 有性能代价的写法 |
| clippy::pedantic | allow | 主观、严格，按需开启 |

前五组合起来就是 `clippy::all`，也是 `cargo clippy` 默认会报的部分。跑一段典型的新手代码：

```console
$ cargo clippy
warning: the loop variable `i` is only used to index `v`
 --> src/main.rs:4:14
  |
4 |     for i in 0..v.len() {
  |              ^^^^^^^^^^
  |
note: for this index operation
 --> src/main.rs:5:16
  |
5 |         sum += v[i];
  |                ^^^^
  = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.100.0/index.html#needless_range_loop
  = note: `#[warn(clippy::needless_range_loop)]` on by default
help: consider using an iterator
  |
4 -     for i in 0..v.len() {
4 +     for <item> in &v {
  |

warning: equality checks against true are unnecessary
 --> src/main.rs:8:8
  |
8 |     if flag == true {
  |        ^^^^^^^^^^^^ help: try: `flag`
  |
  = note: `#[warn(clippy::bool_comparison)]` on by default

warning: `clippy_demo` (bin "clippy_demo") generated 3 warnings (run `cargo clippy --fix --bin "clippy_demo" -p clippy_demo -- ` to apply 2 suggestions)
```

每条警告都带三样东西：问题在哪、为什么（`note`）、怎么改（`help` 给出可以直接替换的代码）。第三条 `useless_vec` 没贴出来，但意思一样。改完是这样：

```rust
fn main() {
    let v = [1, 2, 3, 4];
    let sum: i32 = v.iter().sum();
    if sum > 0 {
        println!("{}", sum);
    }
}
```

CI 里的标准写法是 `cargo clippy --all-targets --all-features -- -D warnings`：`--all-targets` 连测试和示例一起检查，`--all-features` 打开所有 feature，`--` 之后的 `-D warnings` 交给 rustc，把警告升级成错误。

想让人在本地就红，可以在 crate 根写 `#![deny(clippy::all)]`。它和 CI 的 `-D warnings` 有区别：前者只针对 clippy 的这五组，后者把 rustc 自己的警告也算了进去。两个都开最稳。

`#[allow]` 要克制。它最常见的坏用法是"警告消不掉就关掉"，然后把整组 lint 关死：`#![allow(clippy::all)]` 一写，这个 crate 从此不受检查。正确做法是精确到一条 lint，并且写清理由。Rust 1.81 起更推荐 `#[expect]`：

```rust,noplayground
struct Conn;

#[expect(clippy::too_many_arguments, reason = "参数与协议字段一一对应，拆成结构体反而更难核对")]
fn new_conn(host: &str, port: u16, tls: bool, timeout_ms: u64, retries: u32, keepalive: bool) -> Conn {
    let _ = (host, port, tls, timeout_ms, retries, keepalive);
    Conn
}
```

`#[expect]` 比 `#[allow]` 多一条保证：如果这条 lint 后来不再触发（代码改了、clippy 规则变了），`unfulfilled_lint_expectations` 会报一条警告，提醒你删掉这条过期的豁免。`#[allow]` 做不到——它会悄悄留几十年。

还有一个现实问题：clippy 每个版本都会加新规则。`-D warnings` 配上不固定的工具链，某天早上一行代码没改，CI 突然红了。解决办法是固定 CI 的 Rust 版本（写死 `1.100.0` 而不是 `stable`），或者让 clippy 只在 stable 上做门禁、nightly 只做提示。

## 32.7 CI：一条覆盖 fmt、clippy、test 的流水线

到这一步，前面所有命令都还只在你的机器上跑过。CI 的价值是把它们变成"合并前必须通过"的条件，并且把"只在我的机器上能跑"的风险摊到三个操作系统、两个工具链上。

GitHub Actions 的配置文件放在 `.github/workflows/ci.yml`：

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  test:
    name: ${{ matrix.rust }} on ${{ matrix.os }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        rust: [stable, beta]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.rust }}
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test --all-features
      - run: cargo test --no-default-features
      - run: cargo doc --no-deps --all-features
        env:
          RUSTDOCFLAGS: -D warnings
```

逐块读：

`on` 决定什么时候触发。推送到 `main` 和所有 `pull_request` 都跑，PR 上的结果才能作为合并条件。

`permissions: contents: read` 是最小权限。工作流默认可能拿到比需要的更多权限，显式声明只读能把"被投毒的依赖在 CI 里改仓库"这类风险压到最低。缓存不受这个字段影响，32.9 会解释原因。

`fmt` 单独一个 job，只在 Ubuntu 上跑一次。格式化结果与操作系统无关，放进矩阵纯属浪费五分钟。

`test` 用矩阵跑 2 乘 3 共 6 个组合。`fail-fast: false` 让一个组合失败后其余继续跑完——你需要知道是"Windows 特有"还是"全都挂了"。beta 工具链提前暴露下一个稳定版会破坏什么，如果嫌吵，可以给它加 `continue-on-error: true`。

缓存放第三步。`Swatinem/rust-cache@v2` 会缓存 `~/.cargo/registry`、`~/.cargo/git` 和 `target`，键里带 `Cargo.lock` 的哈希和 rustc 版本；依赖没变时命中，依赖一变自动失效。手写 `actions/cache` 也能做，但键的设计（要包含 rustc 版本，否则跨工具链复用会拿到不兼容的产物）容易写错。

`cargo test --all-features` 和 `cargo test --no-default-features` 是两条线。前者验证"功能全开时能编译能跑"，后者验证"最小依赖集也成立"。只跑其中一条，另一条路径上的 `#[cfg]` 代码就永远没人编译过。

最后一条把文档也纳入门禁：`RUSTDOCFLAGS` 是 rustdoc 专属的环境变量，`-D warnings` 让文档警告变成错误。这一条在 32.4 讲过，放进 CI 才算真正生效。

工作流之外还有两件常被忘的事：一是 Linux 上缺系统库（`openssl`、`pkg-config`），要在 `steps` 里先 `sudo apt-get install`；二是私有仓库的分钟数要计费，6 个 job 每次跑满并不便宜，矩阵该按实际需要收缩。

## 32.8 发布：package、publish 与 yank

`cargo publish` 只有一条命令，但发布是一串检查。原因在于 crates.io 的规则：**版本号一旦上传就不能删除，也不能用同一个号重发**。所以发布前的每一步都是在避免"把错误永久留在网上"。

### 第一步：补齐元数据

`Cargo.toml` 里除了 name 和 version，还有一批 crates.io 会展示、下游会看的字段：

```toml
[package]
name = "wordtools"
version = "0.2.3"
edition = "2021"
rust-version = "1.82"
description = "Word counting and text normalization helpers."
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/wordtools"
documentation = "https://docs.rs/wordtools"
keywords = ["text", "words", "unicode"]
categories = ["text-processing"]

[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

`description` 会出现在搜索结果里，一句话说清"这个 crate 干什么"，别写 "A Rust library"。`license` 用 SPDX 表达式，Rust 生态的惯例是 `MIT OR Apache-2.0` 双许可；如果许可证全文要单独放文件，用 `license-file`，两者只能选一个。`keywords` 最多五个，`categories` 必须是 crates.io 的固定分类名。

`[package.metadata.docs.rs]` 控制 docs.rs 怎么给你建文档。`all-features = true` 让文档包含所有 feature 下的 API；没有这一行，docs.rs 默认只开默认 feature，一半的页面会是空的。

### 第二步：看清单

`cargo package --list` 打印将要打进 `.crate` 的文件，不上传、不编译：

```console
$ cargo package --list
.cargo_vcs_info.json
Cargo.lock
Cargo.toml
Cargo.toml.orig
README.md
src/lib.rs
```

前三行是 cargo 自己加的：`.cargo_vcs_info.json` 记录 git 提交号（方便追溯这个版本对应的代码），`Cargo.toml.orig` 是你原始的清单，`Cargo.toml` 是规范化后的版本。`Cargo.lock` 也会被打进去，但下游构建时不会用它。`target/`、`.git/` 默认排除。

清单不对时用 `[package]` 里的 `include` / `exclude` 调整。常见事故是把测试数据、几 MB 的样例文件、内部文档一起发上去——`.crate` 有大小限制，而且这些东西会永久留在那个版本里。

### 第三步：干跑

`cargo publish --dry-run` 会完整走一遍打包和编译验证，只是最后不上传。它是发布前唯一能验证"这个 `.crate` 真能编译"的手段——注意它验证的是**打包后的内容**，不是你的工作目录，少一个文件在这里就会暴露。

本机如果配了 crates.io 镜像，这一步会直接停住：

```text
error: crates-io is replaced with non-remote-registry source registry `rsproxy-sparse`;
include `--registry crates-io` to use crates.io
```

这条报错本身有用：它说明 `publish` 必须走真正的远端 registry，镜像只服务下载。要发布就得在没被替换的环境里做，或者显式指定 registry。

### 第四步：发布与补救

`cargo login` 存好 token，`cargo publish` 上传。之后如果发现版本有严重问题，唯一的补救是 **yank**：

```console
$ cargo yank --version 0.2.3
    Yanking wordtools@0.2.3
$ cargo yank --undo --version 0.2.3
    Unyanking wordtools@0.2.3
```

yank 的语义要说准：它**不是删除**。新项目解析依赖时会跳过被 yank 的版本，但已经写在别人 `Cargo.lock` 里的版本仍然能下载、能构建——这保证了别人的构建不会因为你 yank 而突然坏掉。所以正确处理是"yank 掉坏版本 + 立刻发一个修订号更高的修复版"，而不是指望它消失。

### 工作区：发布顺序

工作区里多个 crate 互相依赖时，发布顺序由依赖图决定：先发被依赖的，再发依赖它的。Cargo 不会替你排序，也不会等索引同步。

两个硬性要求：

- 路径依赖必须同时写 `version`：`wordtools = { path = "../wordtools", version = "0.2.3" }`。只写 path 的依赖在打包时会被剥掉路径，剩下一个没有版本约束的依赖，`cargo package` 会直接报错。
- 发完被依赖的 crate 后，索引要几秒到几十秒才可见。脚本里连着发两个，第二个可能报"找不到刚发的版本"。用 `cargo publish -p 名字` 逐个发，中间留出重试。

版本号多的项目一般用 `cargo-release` 或 `release-plz` 这类工具串起"改版本号、打 tag、发布、推送"，但底层的检查仍是上面这四步。

## 32.9 深入：链接解析、令牌权限与 feature 组合

这一节往下走三层：文档里的链接是怎么解析的、CI 里那些令牌到底谁管什么、以及 `--all-features` 为什么不是"测了所有情况"。

### 32.9.1 intra-doc link 的解析规则

文档里的 `[Counter::bump]` 不是普通 Markdown 链接，rustdoc 会把它当路径解析，解析成功才生成 `<a>`，失败则触发 `rustdoc::broken_intra_doc_links`。解析顺序大致是：

1. 先在当前项的模块作用域里找，和写代码时的名字解析一致；
2. 找不到就到 crate 根找；
3. 还找不到就当作外部 crate 的项，用依赖的元数据去解析（docs.rs 上能跳到其他 crate 的文档，靠的就是这一步）。

`Self` 在 `impl` 块里可用，指向当前类型；`crate::`、`super::`、`self::` 的语义和代码里一样。名字有歧义时（同名 struct 和函数），用消歧前缀指定种类：`[struct@Config]`、`[fn@config]`、`[mod@config]`、`[macro@config]`。

链接到私有项会触发另一条 lint `private_intra_doc_links`——文档里点不过去，读者会困惑。修法是改成公开路径，或者删掉链接只留文字。

这套机制的关键在于：**链接是编译期检查的**。所以它能进 CI，而普通 Markdown 的死链检查只能靠爬虫。

### 32.9.2 CI 缓存与 GITHUB_TOKEN 各管什么

很多教程会让你在 workflow 里加 `permissions: write-all` 才能用缓存，这是误解。两套凭据要分开看：

- **GITHUB_TOKEN** 是仓库级的短期令牌，权限由 `permissions` 字段决定。`actions/checkout` 拉代码需要 `contents: read`；创建 release、上传构建产物需要 `contents: write`。默认权限取决于仓库设置，显式写成最小集合是稳妥做法。
- **缓存服务**用的是每次 job 运行时注入的运行时令牌（`ACTIONS_RUNTIME_TOKEN`），作用域限于本次工作流运行。`actions/cache` 和 `Swatinem/rust-cache` 都用它，所以 `permissions: contents: read` 完全不影响缓存命中。

缓存还有一个安全边界：来自 fork 的 PR，令牌是只读的，缓存也只能读自己分支的，写不进主分支的缓存。这正是设计意图——不能让一个未合并的 PR 往主分支缓存里塞东西。反过来说，`pull_request_target` 触发器会在基分支上下文运行、还能拿到写权限，除非你完全清楚在做什么，否则不要用它来跑 PR 里的代码。

### 32.9.3 --all-features 与组合爆炸

N 个 feature 有 2 的 N 次方种开关组合，`--all-features` 只覆盖其中**全开**这一种。它甚至不是"最严格"的那种：有一类 feature 是互斥的。

```rust,noplayground
#[cfg(feature = "tls")]
pub fn connect(addr: &str) -> std::io::Result<()> {
    let _ = addr;
    Ok(())
}

#[cfg(not(feature = "tls"))]
pub fn connect(addr: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "tls feature 未开启",
    ))
}
```

如果再加一个 `runtime-tokio` 和 `runtime-async-std`，两个都开就会撞车：两套运行时初始化代码同时存在，符号重复或者逻辑互斥。这时 `--all-features` 编译失败，而 `--features runtime-tokio` 和 `--features runtime-async-std` 各自都能过。所以 CI 里至少要两条：全开一条，最小一条（`--no-default-features`）。

更麻烦的是"默认看不见"：`#[cfg(feature = "x")]` 包住的代码，在默认构建里根本不参与编译，语法错了都没人发现，直到某个用户打开那个 feature。feature 越多，这种死角越多。

实践上的三条约束：

- feature 数量控制在个位数，每一个都要有 CI 覆盖，覆盖不了的就别加。
- feature 名表达**能力**而不是**实现**：`tls` 比 `openssl` 好，因为将来换实现时用户不用改 `Cargo.toml`。
- 用 `dep:` 语法避免隐式 feature。`serde = { version = "1", optional = true }` 会自动生成一个叫 `serde` 的 feature，用户 `features = ["serde"]` 时你不知道他想要的是"启用 serde 支持"还是"顺手打开了这个可选依赖"。写成 `features = ["dep:serde"]` 就不会生成隐式 feature。

需要真的穷举组合时，用 `cargo hack --feature-powerset`（第三方子命令）。它按 2 的 N 次方逐个构建，只适合 feature 很少的库。

## 典型错误案例

下面五条报错全部来自本机的 rustc 1.100.0 与 cargo 1.100.0，代码与命令都经过实际运行，输出只做了路径分隔符统一（反斜杠改成斜杠）和无关行的删减。

### 案例 1：文档测试里用了包名而不是库名——E0432

包名和库名不一致时，doctest 必须用库名：

```toml
[package]
name = "rename_demo"
version = "0.1.0"
edition = "2021"

[lib]
name = "wordtools"
```

```rust,noplayground
//! 文档示例会被 `cargo test --doc` 编译运行。

/// 把两个整数相加。
///
/// ```
/// use rename_demo::add;
/// assert_eq!(add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 3));
}
```

```text
error[E0432]: unresolved import `rename_demo`
 --> src/lib.rs:6:5
  |
6 | use rename_demo::add;
  |     ^^^^^^^^^^^ use of unresolved module or unlinked crate `rename_demo`
  |
  = help: if you wanted to use a crate named `rename_demo`, use `cargo add rename_demo` to add it to your `Cargo.toml`

error: aborting due to 1 previous error

Couldn't compile the test.
```

**原因**：doctest 被编译成一个独立 crate，链接的是 **lib 目标**，名字由 `[lib] name` 决定，这里是 `wordtools`。`rename_demo` 只是包名，在代码里不是一个可导入的 crate。编译器看不出你的意图，于是按常规给出"用 cargo add 加上它"的建议——方向正好相反。

**修复**：把 `use` 改成库名。这也解释了为什么给库改名（`[lib] name`）是一次破坏性变更：所有下游的 `use` 都要跟着改。

```rust,noplayground
//! 文档示例用库名导入。

/// 把两个整数相加。
///
/// ```
/// use wordtools::add;
/// assert_eq!(add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 3));
}
```

### 案例 2：文档示例引用了没声明的 crate——E0463

```rust,noplayground
//! 文档示例会被 `cargo test --doc` 编译运行。

/// 生成一个随机字节。
///
/// ```
/// extern crate rand;
///
/// fn main() {
///     println!("{}", rand::random::<u8>());
/// }
/// ```
pub fn pick() {}

fn main() {
    println!("demo");
}
```

```text
error[E0463]: can't find crate for `rand`
 --> src/lib.rs:6:1
  |
6 | extern crate rand;
  | ^^^^^^^^^^^^^^^^^^ can't find crate

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0463`.
Couldn't compile the test.

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s

error: doctest failed, to rerun pass `--doc`
```

**原因**：doctest 的编译环境就是这个 crate 的依赖环境，`Cargo.toml` 里没写 `rand`，编译时自然找不到。常见的来源是抄了一段别处的示例、或者把示例当成了"文档，不用管"。注意报错发生在编译阶段，比断言失败更早，也更值得优先修——它意味着读者复制这段示例必然失败。

**修复**：要么真的加上依赖，要么把示例改成只依赖标准库。给一个需要联网或大依赖的示例，加 `no_run` 也无法绕开"crate 必须存在"，只能加依赖或改写。

```rust,noplayground
//! 文档示例只依赖标准库。

/// 用一个种子生成伪随机字节。
///
/// ```
/// let b = pseudo_byte(42);
/// assert_eq!(b, pseudo_byte(42));
/// ```
pub fn pseudo_byte(seed: u64) -> u8 {
    let mixed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (mixed >> 33) as u8
}

fn main() {
    println!("{}", pseudo_byte(42));
}
```

### 案例 3：文档测试断言失败——doctest failed

```rust,noplayground
//! 文档示例会被 `cargo test --doc` 编译运行。

/// 把两个整数相加。
///
/// ```
/// assert_eq!(2 + 2, 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 2));
}
```

```text
---- src/lib.rs - add (line 5) stdout ----
Test executable failed (exit code: 101).

stderr:

thread 'main' panicked at src/lib.rs:3:1:
assertion `left == right` failed
  left: 4
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.34s

error: doctest failed, to rerun pass `--doc`
```

**原因**：代码编译通过，运行时断言失败。`left` 是实际值 4，`right` 是期望值 5，两者矛盾。这里的价值正在于示例和实现都是"真的"：示例写错、实现写错、或者两者对同一个 API 的理解不一致，都会在这里爆出来。注意 panic 位置 `src/lib.rs:3:1` 指向的是源文件里文档块附近，不是生成代码的行号，照着附近看就行。

**修复**：确认哪边才是对的。`2 + 2` 是 4，所以改断言；如果实现错了，就改实现。

```rust,noplayground
//! 文档示例会被 `cargo test --doc` 编译运行。

/// 把两个整数相加。
///
/// ```
/// assert_eq!(2 + 2, 4);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 2));
}
```

### 案例 4：依赖版本写成浮点数——manifest 解析失败

```toml
[package]
name = "manifest_demo"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = 1.0
```

```text
error: invalid type: floating point `1.0`, expected a version string like "0.9.8" or a detailed dependency like { version = "0.9.8" }
 --> Cargo.toml:7:9
  |
7 | serde = 1.0
  |         ^^^
```

**原因**：TOML 把 `1.0` 解析成浮点数，而 cargo 要的是版本字符串。`"1.0"` 和 `1.0` 在 TOML 里是两种类型，加不加引号决定了它是字符串还是 f64。同一个坑还有更狠的版本：`version = 0.1.0` 会得到 `error: invalid float, expected nothing`，因为 TOML 不接受两个小数点。这类错误发生在解析清单阶段，编译都还没开始。

**修复**：所有版本号都加引号。

```toml
[package]
name = "manifest_demo"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
```

### 案例 5：工作区有未提交改动——cargo package 拒绝

```rust,noplayground
/// 加一。
pub fn add_one(x: i32) -> i32 {
    x + 1
}
```

刚改过 `README.md` 和 `src/lib.rs`，都没提交，直接打包：

```text
warning: manifest has no documentation, homepage or repository
  |
  = note: see https://doc.rust-lang.org/cargo/reference/manifest.html#package-metadata for more info
error: 2 files in the working directory contain changes that were not yet committed into git:

README.md
src/lib.rs

to proceed despite this and include the uncommitted changes, pass the `--allow-dirty` flag
```

**原因**：`cargo package`（以及内部调用它的 `cargo publish`）默认只打包 git 里已提交的内容，这样 `.crate` 与 tag 指向的代码一一对应。工作区有改动时它拒绝继续，并列出涉及的文件。上面那条 `warning` 是另一件事：清单里缺 `repository` 之类的元数据，crates.io 页面会显得很空，但不阻止打包。

**修复**：先提交，再打包。真要在脏工作区验证打包结果，加 `--allow-dirty`；正式发布不要加这个参数，否则网上那个版本可能对应不到任何一次提交。

```bash
git add -A
git commit -m "docs: 补 README 说明"
cargo package --list
cargo publish --dry-run
```

## 动手练习

**练习 1**：新建一个库 crate，给它加 `#![deny(missing_docs)]`，写五个公开项后跑 `cargo doc --no-deps`，把报出的缺失文档逐个补上。最后用 `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` 再跑一遍，确认零警告。留意 `#[allow(missing_docs)]` 加在模块上时，子模块里是否也被放宽。

**练习 2**：写一个文档块分别使用四种属性：`no_run`、`ignore`、`should_panic`、隐藏行 `#`。跑 `cargo test --doc`，对照输出里的 `- compile`、`ignored` 标记，解释为什么 `no_run` 仍能在 API 改名时报错，而 `ignore` 不能。

**练习 3**：写一段带 `for i in 0..v.len()`、`if flag == true`、`let v = vec![1, 2, 3]` 的代码，跑 `cargo clippy` 记下三条 lint 名。然后只给其中一条加 `#[expect(lint名, reason = "此处写清豁免理由")]`，把代码改成触发不到它，再跑一次，解释 `unfulfilled_lint_expectations` 是在提醒什么。

**练习 4**：把 32.7 的 workflow 放进一个真实仓库，依次制造三种失败：`cargo fmt` 改成不合规、引入一条 clippy 警告、在 doctest 里写一个必然失败的断言。确认三种情况 CI 都会红，并说明分别由哪个步骤拦住。

**练习 5**：建一个两 crate 的工作区：`core` 库和依赖它的 `app` 二进制。给 `core` 补全 `Cargo.toml` 元数据，跑 `cargo package --list -p core`，指出哪些文件是 cargo 自动加的；然后把 `app` 对 `core` 的依赖只写 `path` 不写 `version`，跑 `cargo package -p app`，解释它为什么拒绝。

## 小结

- `///` 是外层文档注释，`//!` 是内层；它们分别是 `#[doc = "文本"]` 的语法糖，所以能配合 `#[doc(hidden)]`、`#[doc(alias)]` 使用。
- 文档代码块会被 `cargo test` 当作独立 crate 编译运行；没有 `fn main` 时 rustdoc 自动套壳，有 `crate::` 时它指向的是临时 crate 而不是你的库。
- `no_run` 只编译不运行，`ignore` 连编译都跳过，`should_panic` 要求 panic，`compile_fail` 要求编译失败；行首 `# ` 的隐藏行参与编译但不显示。
- doctest 看到的世界和下游用户完全一样：只有公开 API、只有已声明的依赖、必须有 lib 目标；纯二进制 crate 跑不了 `--doc`。
- `#![deny(missing_docs)]` 与 `rustdoc::broken_intra_doc_links` 把文档质量变成编译期错误，CI 再用 `RUSTDOCFLAGS="-D warnings"` 收口；`#[doc(hidden)]` 只是不展示，不改变 semver 义务。
- Cargo 的版本要求默认是插入符语义；`0.x` 时次版本号承担主版本职责，破坏性变更的判断标准是"下游代码会不会编译不过"。
- `rust-version` 声明的 MSRV 必须由 CI 用对应工具链真跑一遍才算数；`Cargo.lock` 对应用提交、对纯库可选，`--locked` 用于需要强一致构建的场景。
- CI 的标准配置是 fmt 单独一个 job，加上 stable/beta 乘多操作系统的矩阵，缓存用 `Swatinem/rust-cache`，并在 `--all-features` 之外补一条 `--no-default-features`。
- 发布前依次做：补元数据、`cargo package --list` 看清单、`cargo publish --dry-run` 验证打包结果、提交干净工作区；发错只能 yank，不能删除或重发同一个版本号。
