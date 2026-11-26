# 附录 A Cargo 常用命令与配置

> **本章目标**：把散落在正文各章的 Cargo 知识集中成一份可以随时查阅的手册。日常写代码时你只需要记住 `cargo run` 和 `cargo test`，但一旦项目变复杂——多个 crate、可选依赖、不同的构建配置、CI 里要可复现的构建——就会撞上 Cargo.toml 里的各种字段。这一章按「日常用什么」「配置怎么写」「报错怎么修」三段组织。

## A.1 一个项目的完整生命周期

从零到发布，一条命令一个阶段：

```console
$ cargo new hello            # 创建二进制项目
$ cd hello && cargo run      # 编译并运行（debug）
$ cargo check                # 只做类型检查，比 build 快 3-5 倍
$ cargo test                 # 编译测试并运行
$ cargo fmt                  # 格式化
$ cargo clippy -- -D warnings # 静态检查，警告当错误
$ cargo build --release      # 优化构建
$ cargo doc --open           # 生成并打开文档
```

`cargo check` 值得单独强调：它跳过代码生成，只跑前端（解析、类型检查、借用检查），所以在一个中等项目上常常是 1 秒 vs 8 秒的差别。写代码时的高频循环应该是 `check`，而不是 `build`。

### A.1.1 命令速查

| 命令 | 作用 | 备注 |
| --- | --- | --- |
| cargo new NAME | 新建项目，默认 git init | `--lib` 建库 |
| cargo init | 在已有目录里初始化 | |
| cargo build | 编译 | `--release` 走优化档 |
| cargo run | 编译并运行 | `-- args` 传参数 |
| cargo check | 只做检查 | 最快的内环 |
| cargo test | 编译并跑测试 | `-- --nocapture` 显示输出 |
| cargo bench | 跑基准测试 | 需要 nightly 或 criterion |
| cargo fmt | 格式化 | `--check` 只检查不修改 |
| cargo clippy | 静态检查 | `--fix` 自动修可修项 |
| cargo doc | 生成文档 | `--no-deps` 只文档本项目 |
| cargo add | 添加依赖并写进清单 | 需要 cargo-edit |
| cargo remove | 移除依赖 | 需要 cargo-edit |
| cargo update | 更新 lock 里的版本 | `-p name` 只更新一个 |
| cargo tree | 打印依赖树 | `-d` 只看重复版本 |
| cargo metadata | 输出项目元数据 JSON | 工具链集成用 |
| cargo clean | 删除 target | `-p name` 只清一个包 |
| cargo publish | 发布到注册表 | `--dry-run` 先演练 |
| cargo install | 安装二进制 | `--path .` 装本地 |
| cargo generate-lockfile | 只生成锁文件 | CI 常用 |
| cargo vendor | 把依赖复制进项目 | 离线构建用 |

### A.1.2 三个容易忽略的参数

```bash
# 1. 只编译某一个包（workspace 里非常有用）
cargo build -p my-server

# 2. 编译所有 feature 组合，找出只在某个组合下才失败的情况
cargo check --all-features
cargo check --no-default-features

# 3. 显示详细的编译时间，找出拖慢构建的依赖
cargo build --timings
```

`--timings` 会在 `target/cargo-timings/` 下生成一个 HTML，横轴是时间、纵轴是 crate，一眼就能看出是哪个依赖吃掉了 40 秒。当你觉得「改一行要等一分钟」时，第一件该做的事就是看这个图——通常是某个永远不重建的重型依赖（比如 `syn`）在首次编译，或者你不小心开了 `--all-features`。

## A.2 Cargo.toml 解剖

一个功能完整的清单长这样：

```toml
[package]
name = "myapp"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"
authors = ["You <you@example.com>"]
description = "一个示例项目"
license = "MIT OR Apache-2.0"
readme = "README.md"
keywords = ["cli", "demo"]
categories = ["command-line-utilities"]

[lib]
name = "myapp"
path = "src/lib.rs"

[[bin]]
name = "myapp"
path = "src/main.rs"
required-features = ["cli"]

[dependencies]
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"

[features]
default = ["cli"]
cli = ["dep:clap"]

[profile.release]
lto = true
codegen-units = 1
```

几个字段值得单独解释：

- **`edition`** 决定语言版本（2015/2018/2021/2024）。它不影响编译器版本，只影响语法与语义细节（比如 `async` 是不是关键字、数组的 `IntoIterator` 行为、闭包捕获规则）。不写 edition 时 Cargo 会警告并默认 2015——这是很多「教程里的代码在我这儿编译不过」的根源。
- **`rust-version`** 声明最低支持的 Rust 版本（MSRV）。它不会阻止你在更新的编译器上构建，但依赖解析器会用它来挑选兼容的依赖版本。
- **`required-features`** 让某个 target 只在特定 feature 打开时才编译。常见用法是把命令行部分藏在 `cli` feature 后面，库用户不需要拖进 `clap`。
- **`license`** 与 **`description`** 在 `cargo publish` 时是必需的；`license` 推荐用 SPDX 表达式（`MIT OR Apache-2.0` 是 Rust 生态的惯例双许可）。

### A.2.1 自动发现规则

Cargo 在你不写 `[lib]`/`[[bin]]` 时会按约定查找：

| 路径 | 目标 |
| --- | --- |
| src/lib.rs | 库，名字等于包名（连字符转下划线） |
| src/main.rs | 二进制，名字等于包名 |
| src/bin/*.rs | 每个文件一个二进制 |
| src/bin/NAME/main.rs | 每个目录一个二进制 |
| tests/*.rs | 每个文件一个集成测试 crate |
| benches/*.rs | 每个文件一个基准测试 |
| examples/*.rs | 每个文件一个示例，`cargo run --example NAME` |

这套约定带来一个很实用的结构：库里放逻辑（可测试），`src/main.rs` 只做参数解析和调用，`src/bin/` 下放辅助工具，`examples/` 放给人看的用法演示。第 39 章的 minigrep 就是这个结构。

## A.3 依赖的六种写法

```toml
[dependencies]
# 1. 版本号：插入符语义，1.2.3 表示 >=1.2.3, <2.0.0
serde = "1"

# 2. 精确版本：只在万不得已时用，会阻碍整个依赖图的升级
pinned = "=1.2.3"

# 3. 路径依赖：workspace 内部或本地开发
my-core = { path = "../my-core" }

# 4. Git 依赖：cargo 会锁到具体 commit
my-lib = { git = "https://github.com/me/my-lib", branch = "main" }

# 5. 可选依赖：只有打开 feature 才编译
tokio = { version = "1", optional = true }

# 6. 平台限定依赖
[target.'cfg(windows)'.dependencies]
winapi = "0.3"
```

关于版本号，有一个必须搞清楚的细节：**`"1"` 不是「版本 1」，而是「任何 1.x」**。Cargo 用插入符语义：

| 写法 | 等价范围 |
| --- | --- |
| 1.2.3 | >=1.2.3, <2.0.0 |
| 0.2.3 | >=0.2.3, <0.3.0 |
| 0.0.3 | >=0.0.3, <0.0.4 |
| =1.2.3 | 恰好 1.2.3 |
| >=1.2, <1.5 | 显式区间 |
| * | 任意版本（不推荐） |

注意 `0.x` 的特殊规则：0.x 版本被视为「不稳定的主版本」，所以 `0.2` 只接受 0.2.*。这就是为什么 0.x 的 crate 升级经常是破坏性的。

`Cargo.lock` 记录实际使用的精确版本。**应用要提交 lock，库不要提交**——应用需要可复现的构建，库需要让下游能自由选择版本。

## A.4 features：编译期开关

feature 是 Cargo 最容易被误用的机制。它本质是「打开一组可选依赖 + 打开一组 cfg 分支」，**不是运行期开关，也不是条件编译的通用工具**。

```toml
[features]
# default 是默认打开的集合
default = ["std"]

# 直接打开一个 feature
std = []

# feature 可以携带依赖：dep: 前缀表示「只启用这个可选依赖，不产生同名 feature」
json = ["dep:serde_json"]

# feature 叠加：打开 json 时也打开 serde 的 derive
serde-support = ["dep:serde", "serde/derive"]
```

配套的代码用 `cfg` 分支：

```rust,noplayground
#[cfg(feature = "json")]
pub fn to_json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

#[cfg(not(feature = "json"))]
pub fn to_json<T>(_v: &T) -> String {
    panic!("请打开 json feature")
}
```

三条实践建议：

1. **feature 必须是可加的**（additive）。打开一个 feature 只能「增加」能力，不能「替换」行为。如果两个 feature 互斥，说明设计有问题——因为 Cargo 会把同一个 crate 的所有使用者请求的 feature 取并集（feature unification），你无法保证用户只打开其中一个。
2. **默认 feature 要保守**。默认打开的东西会出现在每一个依赖它的项目里，拖慢编译。重型可选依赖（数据库驱动、异步运行时）默认关闭。
3. **用 `dep:` 语法**避免意外产生同名 feature。`json = ["serde_json"]` 会隐式创建一个叫 `serde_json` 的 feature，用户可以用 `features = ["serde_json"]` 打开它，语义混乱；写成 `json = ["dep:serde_json"]` 就只保留 `json` 这个名字。

测试 feature 组合是 CI 的必备项：

```bash
cargo check --no-default-features
cargo check --all-features
cargo check --no-default-features --features json
```

## A.5 profile：控制优化与调试信息

```toml
[profile.dev]
opt-level = 1        # 0 最容易调试，1 能让数值循环快很多
debug = true         # 保留调试符号

[profile.dev.package."*"]
opt-level = 2        # 依赖总是优化，自己的代码保持可调试

[profile.release]
opt-level = 3
lto = "thin"         # 链接期优化，thin 在编译时间与体积间折中
codegen-units = 1    # 更好的优化，更慢的编译
panic = "abort"      # 去掉展开表，体积更小（但 catch_unwind 失效）
strip = "symbols"    # 去掉符号表，二进制小很多

[profile.bench]
debug = true         # 基准测试保留符号，方便 perf 分析
```

`[profile.dev.package."*"]` 配 `opt-level = 2` 是一个很实用的技巧：你的代码保持未优化（断点不跳、变量可看），但依赖（正则引擎、解析器、加密库）用优化后的版本，整体运行速度能有数倍提升。

自定义 profile 需要 nightly 或较新的 Cargo：

```toml
[profile.profiling]
inherits = "release"
debug = true
```

用 `cargo build --profile profiling` 使用它，配合 `perf` 或 `samply` 做性能分析。

## A.6 workspace：多 crate 项目

一个 workspace 是「一组共享一份 `Cargo.lock` 和一份 `target/` 的 crate」。它解决两个具体问题：编译产物不重复（所有 crate 的依赖只编译一次）、版本统一（同一依赖不会出现两个不兼容版本）。

根目录的 `Cargo.toml` 只放 `[workspace]`：

```toml
[workspace]
members = ["core", "server", "cli"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
```

成员 crate 用 `workspace = true` 继承：

```toml
[package]
name = "myapp-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
```

`[workspace.dependencies]` 的价值在于**版本只写一处**。如果三个成员各自写 `serde = "1"` 也没问题（会解析到同一个版本），但只要有人写成 `"1.0.100"`、有人写 `"1.0.200"`，就可能出现两个 serde 副本，表现为「trait 不匹配」的诡异错误——`A` 实现了 `serde::Serialize`，但 `B` 期望的是另一个 serde 版本的 `Serialize`。

常用命令：

```bash
cargo build -p myapp-server   # 只构建一个成员
cargo test --workspace        # 全部测试
cargo tree -d                 # 找出重复的依赖版本
cargo metadata --format-version 1 | jq '.workspace_members'
```

`cargo tree -d` 是排查「trait 不匹配」的第一件工具：如果同一个 crate 出现两个版本，几乎必然是某个依赖没跟上。

### A.6.1 单一仓库、多个二进制

不是所有多目标项目都需要 workspace。如果只是「一个库 + 几个命令行工具」，用 `src/bin/` 就够了：

```text
myproject/
├── Cargo.toml
├── src/
│   ├── lib.rs        # 逻辑，可测试
│   ├── main.rs       # 主程序
│   └── bin/
│       ├── tool-a.rs # cargo run --bin tool-a
│       └── tool-b.rs
```

当各工具需要**不同的依赖集合**（比如一个要 tokio、另一个要 clap），或者需要分别发布到 crates.io，才值得拆成 workspace。

## A.7 环境变量与配置优先级

Cargo 的配置可以来自四处，优先级从高到低：命令行参数 → 环境变量 → `.cargo/config.toml`（从当前目录向上查找）→ `~/.cargo/config.toml`。

| 变量 | 作用 |
| --- | --- |
| CARGO_TARGET_DIR | 改 target 目录位置，CI 里共享缓存常用 |
| CARGO_BUILD_JOBS | 并行编译任务数，默认等于 CPU 核数 |
| CARGO_INCREMENTAL | 0 关闭增量编译，发布构建建议关闭 |
| RUSTFLAGS | 传给 rustc 的参数，如 -C target-cpu=native |
| RUST_BACKTRACE | 1 打印 panic 的完整调用栈 |
| RUST_LOG | tracing/log 的过滤规则，见第 28 章 |
| CARGO_NET_OFFLINE | true 等价于 --offline |
| CARGO_HOME | 修改 cargo 的缓存与配置目录 |

配置文件形式（`.cargo/config.toml`）：

```toml
[build]
target-dir = "target"
jobs = 8

[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=native"]

[alias]
b = "build"
rr = "run --release"
```

`[alias]` 定义后 `cargo b` 就等于 `cargo build`，`cargo rr` 就是发布构建并运行。团队里把常用组合写进仓库的 `.cargo/config.toml`（提交它），能显著减少「每个人记的参数不一样」导致的问题。

## A.8 build.rs：构建脚本

`build.rs` 是 Cargo 在编译你的 crate **之前**执行的 Rust 程序，用途只有三类：生成代码、编译 C 代码、探测环境。

```rust,noplayground
// build.rs
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // 1. 告诉 Cargo：这些文件变了才重新跑我
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto/schema.txt");

    // 2. 生成代码到 OUT_DIR，源码里用 include! 引入
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let gen = "pub const BUILD_TAG: &str = \"local\";\n";
    fs::write(out.join("build_info.rs"), gen).unwrap();

    // 3. 探测平台并设置 cfg
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-cfg=has_epoll");
    }
}
```

代码侧：

```rust,noplayground
include!(concat!(env!("OUT_DIR"), "/build_info.rs"));

#[cfg(has_epoll)]
fn backend() { /* epoll 实现 */ }

#[cfg(not(has_epoll))]
fn backend() { /* 其他实现 */ }
```

`build.rs` 的三个陷阱：

1. **忘了 `rerun-if-changed`**，Cargo 会在任何文件变动时都重跑脚本，或者反过来永远不重跑。显式声明是必须的。
2. **`build.rs` 在宿主机上编译和运行**，不是目标平台。交叉编译时它产生的代码必须与目标平台无关。
3. **它让构建变慢且难缓存**。能用 `const` 或 `cfg!` 解决的事不要用 `build.rs`。

## A.9 离线构建与国内镜像

在国内网络环境下，`cargo build` 卡在下载是常态。两个对策。

**对策一：换源。** 在 `~/.cargo/config.toml` 里：

```toml
[source.crates-io]
replace-with = "rsproxy-sparse"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[registries.rsproxy]
index = "https://rsproxy.cn/crates.io-index"
```

`sparse+` 前缀很关键：它让 Cargo 用 HTTP 直接取索引文件，而不是克隆整个 git 索引（后者要几百 MB 且极慢）。

**对策二：完全离线。** 如果依赖已经下载过，`--offline` 让 Cargo 只用本地缓存：

```bash
cargo build --offline
cargo test --offline
```

离线模式下的典型报错是：

```text
error: failed to select a version for the requirement `serde = "^99"`
candidate versions found which didn't match: 1.0.229
location searched: `rsproxy-sparse` index (which is replacing registry `crates-io`)
required by package `p4 v0.1.0 (D:\code\dshwork\rust-book\.scratch\cprobe\p4)`
note: perhaps a crate was updated and forgotten to be re-vendored?
note: offline mode (via `--offline`) can sometimes cause surprising resolution failures
help: if this error is too confusing you may wish to retry without `--offline`
```

注意 `location searched` 里的 `rsproxy-sparse`——它告诉你「这个源里没有你要的版本」，而不是「你写错了」。如果确实需要新版本，就必须联网，或先在有网环境 `cargo fetch`。

要给完全无网的环境做构建（比如 Docker 镜像里），用 `cargo vendor`：

```bash
cargo vendor vendor/            # 把所有依赖源码复制到 vendor/
# 按输出提示在 .cargo/config.toml 里加 source 替换，然后
cargo build --offline
```

## A.10 发布到 crates.io

流程是固定的一套：

```bash
cargo login                 # 一次性，写入 ~/.cargo/credentials.toml
cargo publish --dry-run     # 打包并检查，不上传
cargo package --list        # 看哪些文件会进包
cargo publish               # 真正上传
```

发布前必须满足的硬条件：`license` 或 `license-file`、`description`、唯一的包名、版本号未发布过。**版本号一旦发布就不能删除**（只能用 `cargo yank` 标记为不可用，已下载的项目仍能构建）。

`cargo publish --dry-run` 最常见的失败原因是**漏文件**：Cargo 只打包被 git 追踪的文件（有 `.gitignore` 时），所以忘记 `git add` 的新文件不会进包。用 `cargo package --list` 核对，或者用 `include`/`exclude` 字段显式控制。

## A.11 常见报错与修复

下面每一条都是本机 `cargo 1.100.0` 的真实输出。

### A.11.1 清单文件里写重复的键

```text
error: duplicate key
 --> Cargo.toml:8:1
  |
8 | serde = "1"
  | ^^^^^
```

**原因**：同一个表里同一个键出现了两次（常见于合并分支时的残留）。

**修复**：删掉重复行。如果两行想表达不同的配置，把它们合并成一个内联表。

### A.11.2 键名写错：只是警告

```text
warning: Cargo.toml: unused manifest key: profile.release.opt_levelx
warning: `p2` (manifest) generated 1 warning
```

**原因**：`opt-level` 拼成了 `opt_levelx`。Cargo 对未知键只发警告，不报错。

**修复**：修正拼写。**这类警告必须当成错误处理**——否则你以为开了 `lto`，其实根本没生效，发布出来的二进制又大又慢。CI 里的 `-D warnings` 不覆盖清单警告，要人工盯。
### A.11.3 依赖不存在

```text
error: no matching package named `this-crate-does-not-exist-xyz` found
location searched: `rsproxy-sparse` index (which is replacing registry `crates-io`)
required by package `p3 v0.1.0 (D:\code\dshwork\rust-book\.scratch\cprobe\p3)`
```

**原因**：包名拼错，或者包不在当前配置的源里。

**修复**：去 crates.io 搜准确名字（注意连字符与下划线是两种不同字符）；确认 `[source]` 替换指向的镜像同步完整。

### A.11.4 版本要求不可能满足

```text
error: failed to select a version for the requirement `serde = "^99"`
candidate versions found which didn't match: 1.0.229
```

**原因**：写了一个不存在的大版本。`^99` 等价于 `>=99.0.0, <100.0.0`。

**修复**：改成实际存在的版本范围。如果是从别处抄来的，注意被抄的项目可能用了私有 fork。

### A.11.5 锁文件与清单不一致

```text
error: cannot update the lock file D:\code\dshwork\rust-book\.scratch\cprobe\p6\Cargo.lock because --locked was passed to prevent this
help: to generate the lock file without accessing the network, remove the `--locked` flag and use `--offline` instead.
```

**原因**：改了 Cargo.toml（加了依赖）但 Cargo.lock 还是旧的，而 `--locked` 要求「不许改锁文件」。这正是 CI 想要的行为——防止构建时悄悄改变依赖版本。

**修复**：本地跑一次 `cargo build` 更新锁文件并提交；CI 保留 `--locked`。

### A.11.6 库与二进制同名导致产物冲突

```text
warning: output filename collision at D:\code\dshwork\rust-book\.scratch\cprobe\p5\target\debug\p5.pdb
  = note: the bin target `p5` in package `p5 v0.1.0` has the same output filename as the lib target `p5` in package `p5 v0.1.0`
  = note: this may become a hard error in the future; see <https://github.com/rust-lang/cargo/issues/6313>
```

**原因**：一个包里同时有 `src/lib.rs` 和 `src/main.rs`，且都用包名作为目标名，在 Windows 上调试符号文件会互相覆盖。

**修复**：用 `[[bin]] name = "p5-cli"` 给二进制换个名字，或者把二进制拆到 `src/bin/` 下用不同文件名。

### A.11.7 忘记写 edition

```text
warning: Cargo.toml: `package.edition` is unspecified, defaulting to `2015` while the latest is `2024`
```

**原因**：清单里缺 `edition` 字段。

**修复**：加 `edition = "2021"`（本书统一用 2021）。不写的后果是异步语法、`dyn` 关键字、数组迭代语义全都退回 2015 的规则，会出现一堆莫名其妙的报错。

## A.12 小结

- 日常内环用 `cargo check`，它比 `cargo build` 快好几倍；只在要运行或要产物时才 build。
- Cargo.toml 的未知键只是警告，**不会阻止构建**，所以拼错的配置会静默失效——见到清单警告必须修。
- 版本号是范围不是精确值；`0.x` 有特殊规则；`Cargo.lock` 应用提交、库不提交。
- feature 必须是可加的，因为 Cargo 会对所有使用者取并集；重型可选依赖默认关闭。
- `[profile.dev.package."*"]` 优化依赖、不优化自己的代码，是开发期性价比最高的一个配置。
- 多 crate 项目用 workspace 共享锁文件与 target；`cargo tree -d` 是排查 trait 不匹配的第一把工具。
- `build.rs` 只在确实需要生成代码或编译 C 时使用，并且一定要写 `rerun-if-changed`。
- 网络不稳时用 `sparse+` 镜像；完全无网时用 `cargo vendor` 加 `--offline`。
- 发布前跑 `cargo publish --dry-run` 与 `cargo package --list`，重点确认没有漏文件。