# 更新日志

本项目的所有重要变更都记录在这里。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [1.0.0] - 2026-11-26

第一个完整版本：44 章 + 4 个附录的中文 Rust 教程，加上六个可以直接跑起来的项目。
在线阅读：<https://yujh129.github.io/rust-book-zh/>

### 内容

- **七部分 44 章**：从环境搭建、所有权、trait、生命周期，一路讲到 Tokio、axum、过程宏与 unsafe。
- **4 个附录**：Cargo 速查、标准库与常用 crate、学习路线图、术语表。
- **每章统一结构**：本章目标 → 正文（含一个「深入」小节拆编译器与运行时机制）→ 五个典型错误案例
  → 动手练习 → 小结。错误案例给的是真实 `rustc` 报错原文，并逐条给出**原因**与**修复**。
- **六个项目实战**（`projects/`，都能独立构建）：

  | 目录 | 是什么 | 依赖 |
  | --- | --- | --- |
  | `projects/39-minigrep` | 命令行 grep（文件名搜索、行号、忽略大小写） | 只用标准库 |
  | `projects/40-json-parser` | 手写 JSON 词法与递归下降解析器 | 只用标准库 |
  | `projects/41-http-server` | 基于 `std::net` 的静态文件 HTTP 服务器 | 只用标准库 |
  | `projects/42-mini-redis` | RESP 协议兼容的迷你 Redis | 只用标准库 |
  | `projects/43-chat-server` | Tokio 多人聊天室（广播、昵称、JoinSet） | tokio |
  | `projects/44-snake` | 终端贪吃蛇（棋盘、食物、计分、胜负） | 只用标准库 |

### 交互

- **错误案例可以在浏览器里直接改、直接跑**：共 313 个标了 `rust,editable` 的代码块。
  先跑一遍看真实报错，再照着「修复」改一行跑第二次对比。
- **本地运行器** `python tools/serve_book.py`：书（:3000）与运行器（:3030）一起起。
  离线复用 Cargo 缓存，内置 anyhow / clap / rand / serde / serde_json / thiserror / tokio / tracing / tracing-subscriber
  共 9 个 crate；含 `#[test]` 的代码按 `cargo test` 跑；单程序 4 秒超时。
  启动时后台预热依赖，第一次点 Run 不用白等。
- **只监听本机**：非本机 `Host`/`Origin` 一律 403，非 JSON 请求 415。
- **章节导航**放在正文末尾，做成小胶囊，不再有贴边的巨型翻页条；←/→ 快捷键保留。
- **10 套配色**：跟随系统、浅色、Rust、Coal、Navy、Ayu，外加四套精选的「纸墨（暖白）」「樱花（浅粉）」
  「Nord（冷灰）」「Gruvbox（暖黑）」。深色主题会自动切换配套的代码高亮。

### 校验

仓库自带四个校验脚本，CI（`.github/workflows/check.yml`）每次都会跑：

| 检查项 | 命令 | 本版结果 |
| --- | --- | --- |
| 目录、行数、链接、每章五案例 | `python tools/check_book.py` | 50 页，0 处问题 |
| 正文普通代码块逐块编译 | `python tools/verify_blocks.py src` | 436 块，0 失败 |
| 错误案例块与文档中的报错逐字比对 | `python tools/verify_cases.py` | 313 块，0 不一致 |
| 六个项目工程构建并通过测试 | `python tools/check_projects.py --offline` | 6 个工程，42 个测试 |

`mdbook build` 零警告。书中围栏代码块共 1665 个（其中 `rust` 430、`rust,editable` 313、
`rust,noplayground` 347、`text` 410、`console` 66）。

### 本版修正的问题

1.0.0 定稿前做了一轮逐文件审计，修掉了下面这些问题（含另一位智能体报告的清单）：

- **依赖声明与事实不符**：文档曾把贪吃蛇写成依赖 Tokio，实际六个项目里只有聊天室用到 tokio。
- **版本承诺过强**：删掉「Rust 1.75 就能编译全书」的说法，统一为「本书使用 Rust 2021 edition，
  本轮核验 rustc 1.100.0」，并补上相关 API 的稳定版本（`{name}` 内联参数 1.58、`unsafe extern` 1.82、
  `use<'a>` 1.82、`LazyLock` 1.80 等）。`book.toml` 增加 `[rust] edition = "2021"`。
- **示例会切坏中文**：ch16 的摘要由字节切片改为 `chars().take(20)`。
- **接收循环永不结束**：ch23 的发送端补上 `drop(tx)`。
- **死锁修复版仍可能挂住**：ch38 的两把锁改为在块作用域内释放，`join` 之前完成。
- **正文与项目源码不一致**：ch42 补齐参数与缓冲上限（1024 个参数 / 8 MiB 单个 bulk / 64 MiB 连接缓冲、`checked_add(2)`），
  ch43 改用 `JoinSet` 收拢写任务并明确退出顺序，ch44 增加棋盘尺寸校验与参数解析错误提示。
- **正文泄漏本机路径**：5 处真实报错里的 `C:\Users\...` 换成 `<cargo registry>`、`<rustup toolchain>` 占位符。
- **校验脚本只能做类型检查**：`verify_blocks.py` 改为逐块编译；`verify_cases.py` 从「前缀容忍」改为严格比对首行报错，
  并新增 `--local`（运行器占用端口时会拒绝运行，避免两个进程互相覆盖 `tools/playground/src/main.rs`），
  同时新增「运行期失败」统计，识别 `panic`/`test result: FAILED` 这类预期报错。
- **运行器对第三方 crate 支持不稳**：`tools/playground_server.py` 从 rustc 直编改为统一走
  `tools/playground` 离线 Cargo 工程，带 `#[test]` 的代码改走 `cargo test`。
- **第一次点 Run 必失败/白等**：运行器启动时后台预热依赖，浏览器等待上限从 6 秒放宽到 3 分钟，
  左下角胶囊提示「正在预编译依赖」。

### 工具链

- Rust 2021 edition，核验使用 rustc / cargo 1.100.0。
- [mdBook](https://github.com/rust-lang/mdBook) 0.5.4。
- 本地运行器依赖 `tools/playground` 的 `Cargo.lock` 锁定版本，可离线构建。
- 另外两套内容资产：`STYLE.md`（写作与代码块规范）、`CHAPTER-PLAN.md`（章节规划与依赖顺序）。

### 说明

本书由 DSH（DeepSeek Harness）编码智能体产出并自查，规范、方向与最终审校由人工负责。
发现问题欢迎开 [Issue](https://github.com/yujh129/rust-book-zh/issues)。

[1.0.0]: https://github.com/yujh129/rust-book-zh/releases/tag/v1.0.0
