# 环境搭建

> **本章目标**
> - 在 Windows、macOS 或 Linux 上装好 Rust 工具链，并配置国内镜像
> - 认识 `rustup`、`cargo`、`rustc` 三个命令的分工
> - 配好编辑器和三个必备工具：`rustfmt`、`clippy`、`rust-analyzer`
> - 能本地构建并预览这本书

## 1.1 三个命令，三种职责

| 命令 | 作用 | 你多久用一次 |
| --- | --- | --- |
| `rustup` | 管理工具链本身：安装、升级、切换 stable/beta/nightly、增删组件和目标平台 | 装一次，之后偶尔 |
| `cargo` | 项目管理：新建、编译、运行、测试、发布、拉依赖 | 每天几十次 |
| `rustc` | 单个文件的编译器，`cargo` 在背后调用它 | 几乎不直接调用 |

理解这一点很重要：**不要用 `rustc main.rs` 学 Rust**，从第一天起就用 `cargo`，否则你会错过依赖管理、测试、文档和 profile 配置。

## 1.2 安装

### Windows

1. 打开 <https://www.rust-lang.org/tools/install> 下载 `rustup-init.exe`。
2. 直接运行，默认选择 **1) Proceed with standard installation**。
3. 安装程序会检测 Visual Studio 的 C++ 生成工具。如果没有，它会提示你先装 **Visual Studio Build Tools**，勾选"使用 C++ 的桌面开发"即可。Rust 在 Windows 上默认使用 MSVC 工具链，链接阶段需要 `link.exe`。
4. 装完后**重开一个终端**，执行：

```console
$ rustc --version
rustc 1.75.0 (82e1608df 2024-01-03)
$ cargo --version
cargo 1.75.0 (1d8b05cdd 2023-11-20)
```

版本号不必和这里一致，能打印出来就说明 PATH 配好了。

> **不想装 Visual Studio？** 可以改用 GNU 工具链：`rustup default stable-x86_64-pc-windows-gnu`，它依赖 MinGW-w64。除非你有特殊理由，还是推荐 MSVC，调试体验更好。

### macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Linux 上如果编译时报 `linker 'cc' not found`，装一下构建工具：

```bash
sudo apt install build-essential   # Debian / Ubuntu
sudo dnf groupinstall "Development Tools"   # Fedora
```

## 1.3 国内镜像：先把下载速度解决

能正常访问官方源时可直接跳到 1.4。工具链下载和 crate 下载使用不同的服务，只需为遇到网络问题的那一项设置镜像。

### 工具链镜像（rustup）

Windows 在 PowerShell 里设置用户环境变量：

```powershell
[Environment]::SetEnvironmentVariable("RUSTUP_DIST_SERVER", "https://rsproxy.cn", "User")
[Environment]::SetEnvironmentVariable("RUSTUP_UPDATE_ROOT", "https://rsproxy.cn/rustup", "User")
```

macOS / Linux 写进 `~/.bashrc` 或 `~/.zshrc`：

```bash
export RUSTUP_DIST_SERVER="https://rsproxy.cn"
export RUSTUP_UPDATE_ROOT="https://rsproxy.cn/rustup"
```

### crates.io 镜像（cargo）

编辑（没有就新建）`~/.cargo/config.toml`，Windows 上是 `C:\Users\<你的用户名>\.cargo\config.toml`：

```toml
[source.crates-io]
replace-with = 'rsproxy-sparse'

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[registries.rsproxy]
index = "https://rsproxy.cn/crates.io-index"

[net]
git-fetch-with-cli = true
```

> 镜像地址会变，如果失效，搜索"rsproxy"或"字节跳动 Rust 镜像"获取最新的配置片段。`git-fetch-with-cli = true` 能避免某些网络环境下 git 依赖拉取卡死。

重开终端后，`rustup update` 使用工具链下载配置。进入有依赖的工程执行 `cargo fetch -v`，可以从 Cargo 输出核对使用的 registry。

## 1.4 编辑器

**VS Code + rust-analyzer** 是最主流的组合：

1. 安装扩展 **rust-analyzer**（注意不是旧的 Rust 扩展）。
2. 打开任意 `Cargo.toml` 所在目录，等右下角索引完成。
3. 推荐再装 **CodeLLDB**（调试）和 **crates**（在 `Cargo.toml` 里显示版本号与更新提示）。

rust-analyzer 提供的东西值得你花十分钟熟悉：保存时自动补全、`Ctrl+.` 快速修复（导入 trait、补 `mut`）、悬停显示类型与文档、`F12` 跳转定义、`Shift+F12` 查找引用、内联类型提示（inlay hints）。

**RustRover** 是 JetBrains 的独立 IDE，对重构和调试支持更好，学生可免费申请。用哪个不影响学习，别在工具上纠结超过一小时。

## 1.5 三个必备组件

```bash
rustup component add rustfmt clippy
```

- `cargo fmt`：统一格式。团队项目里它能让 diff 只包含真正的改动。
- `cargo clippy`：静态检查，能发现 `&vec[0]` 应该写 `vec.first()`、`clone()` 多余、`match` 可以简化等问题。**把 `cargo clippy -- -D warnings` 当成编译的一部分。**
- `rust-analyzer`：由编辑器扩展自动安装，不用手动装。

再加几个提升效率的工具（可选）：

```bash
cargo install cargo-watch cargo-expand
```

| 工具 | 用途 |
| --- | --- |
| `cargo watch -x run` | 文件变化就重新运行，省掉反复敲命令 |
| `cargo expand` | 展开宏之后的代码，理解 `derive` 和 `macro_rules!` 必备 |
| `cargo add serde` | Cargo 内置命令，把依赖版本要求写入清单 |
| `cargo tree` | 查看依赖树，排查版本冲突 |

`cargo-expand` 依赖 nightly 的 `rustc -Zunpretty`，它会自动提示你安装：`rustup toolchain install nightly`。

## 1.6 亲手验证工具链

新建一个项目，跑通完整循环：

```console
$ cargo new hello --bin
     Created binary (application) `hello` package
$ cd hello
$ cargo run
   Compiling hello v0.1.0 (/path/to/hello)
    Finished dev [unoptimized + debuginfo] target(s) in 0.31s
     Running `target/debug/hello`
Hello, world!
```

再试一下测试和 lint：

```console
$ cargo test
   Compiling hello v0.1.0 (/path/to/hello)
    Finished test [unoptimized + debuginfo] target(s) in 0.42s
     Running unittests src/main.rs (target/debug/deps/hello-...)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy
    Checking hello v0.1.0 (/path/to/hello)
    Finished dev [unoptimized + debuginfo] target(s) in 0.18s
```

三条命令都正常，环境就算就绪了。

## 1.7 构建这本书

本书是一个标准的 mdbook 项目，仓库根目录下有 `book.toml` 和 `src/`。

```bash
# 如果你还没装 mdbook
cargo install mdbook --version 0.5.4 --locked

# 在项目根目录启动本地服务器，改文件会自动刷新
mdbook serve --open

# 只生成静态站点到 book/ 目录
mdbook build
```

默认地址是 <http://localhost:3000>。生成出来的 `book/index.html` 可以直接双击打开，但搜索功能需要在 HTTP 环境下才正常，所以日常阅读用 `mdbook serve`。

## 1.8 在浏览器里改代码、跑代码

标为 `editable` 的错误案例带有编辑器；多文件工程及需要特定依赖的片段请在本地复现：那几行是真能改的，点 **Run** 就会看到真实的编译器输出。先跑一遍错误版，再照着修复版改一行，对比两次结果——这是全书最主要的学习方式。

默认情况下 Run 连的是官方 Playground（<https://play.rust-lang.org>），需要联网。如果你已经按前面的步骤装好 Rust，可以让代码在**自己机器上离线编译运行**：

```bash
# 首次使用时联网下载锁定的依赖；缓存齐备后可以断网
cargo fetch --locked --manifest-path tools/playground/Cargo.toml

# 在项目根目录：同时启动本地运行器和 mdbook 预览
python tools/serve_book.py

# 只启动运行器（127.0.0.1:3030），书用别的方式服务
python tools/serve_book.py --no-serve
```

Windows 上没有 `python` 命令就用 `py -3 tools/serve_book.py`。启动后打开 <http://127.0.0.1:3000>：左下角弹出绿色的「本地运行器 已连接 · 9 个 crate」，说明链路通了；如果显示橙色提示，说明运行器没起，Run 会自动退回官方 Playground。

本地运行器支持这些 crate：`anyhow`、`clap`、`rand`、`serde`、`serde_json`、`thiserror`、`tokio`、`tracing`、`tracing-subscriber`。本地编译阶段限时 90 秒；普通程序执行限时 4 秒，测试执行限时 20 秒。超时会终止对应的命令。

运行器启动时会在后台先把这 9 个依赖编译一遍（日志里出现 `[pg] warm-up ok in ...s`），所以第一次点 Run 可能需要等一会儿，之后每次都是秒级。浏览器对本地运行器的等待上限是 3 分钟，足够冷编译跑完。

在本地运行器中，带 `#[test]` 或 `#[cfg(test)]` 的代码块会被当成测试来跑（等价于 `cargo test`），所以你看到的是 `running 1 test`、`test tests::foo ... ok`、`test result: FAILED. 0 passed; 1 failed` 这样的输出，`.rs` 里的断言失败会连 `left` / `right` 一起打出来——第 26 章那几个测试案例就是靠它跑起来的。

| 现象 | 原因 | 解决 |
| --- | --- | --- |
| 点 Run 没反应，左下角橙色提示 | 本地运行器没启动 | 在项目根目录执行 `python tools/serve_book.py` |
| 改完之后 Run 报了个新错 | 改错了——这正是练习的意义 | 读报错，和紧跟代码后面的那几行 `error[...]` 对照 |
| 点 Run 转圈几十秒 | 第一次要编译整个运行器工程（启动时已在后台预热） | 等它跑完；之后的运行都是秒级 |

## 1.9 常见问题

| 现象 | 原因 | 解决 |
| --- | --- | --- |
| `error: linker 'link.exe' not found` | 没装 MSVC 生成工具 | 安装 Visual Studio Build Tools，勾选 C++ 桌面开发；或改用 GNU 工具链 |
| `rustup` 下载卡在 0% | 没配工具链镜像 | 按 1.3 设置 `RUSTUP_DIST_SERVER` |
| `cargo build` 拉依赖超时 | 没配 crates 镜像 | 按 1.3 配置 `config.toml` |
| 终端中文乱码 | Windows 控制台代码页不是 UTF-8 | `chcp 65001`，或在 Windows Terminal 里设置 |
| 项目放在中文路径下报奇怪的错 | 少数工具链组件对非 ASCII 路径支持不佳 | 把项目放到纯英文路径，例如 `D:\\code\\hello` |
| `cargo run` 说 `no targets specified` | Cargo 找到清单，但既没有自动发现目标也没有显式目标 | 创建 `src/main.rs` 或 `src/lib.rs`，或在清单中配置目标路径 |

---

环境就绪。下一章从 `cargo new` 开始，写出第一个真正属于你的 Rust 程序。
