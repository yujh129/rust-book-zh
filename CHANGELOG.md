# 更新日志

本项目的所有重要变更都记录在这里。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [1.0.1] - 2026-11-26

一次以正确性为目标的修补版本：不新增章节，把六个项目的源码补进仓库，
修正正文与源码里的事实错误，并让「错误案例」真的能在浏览器里跑起来。

### 新增

- **六个项目的完整源码入库**（`projects/`）：39 minigrep、40 JSON 解析器、41 HTTP 服务器、
  42 mini-redis、43 聊天室、44 贪吃蛇，clone 下来即可 `cargo run` / `cargo test`；
  只有聊天室用到 tokio，其余五个只依赖标准库。
- 各章末尾新增「## 完整源码」小节：文件职责表、真实运行输出、测试数量，并注明「正文与源码不一致时以 `projects/` 为准」。
- 新增 `tools/check_projects.py`：一条命令构建并测试六个工程（`--offline` 可在无网络时用）。
- 新增 [`CHANGELOG.md`](CHANGELOG.md) 与 `CONTRIBUTING` 说明之外的发布流程记录。
- **案例答案默认收起**：每章 5 个「典型错误案例」的原因与修复包进 `<details class="case-answer">` 面板，
  先自己判断错在哪，再点「查看答案」展开对照；打印或导出 PDF 时自动展开。全库 220 个案例面板。

### 修正

- **依赖声明与事实不符**：文档曾把贪吃蛇写成依赖 Tokio；六个项目里只有聊天室用到 tokio。
- **版本承诺过强**：删掉「Rust 1.75 就能编译全书」的说法，统一为「本书使用 Rust 2021 edition，
  本轮核验 rustc 1.100.0」，并补上相关 API 的稳定版本（`{name}` 内联参数 1.58、`unsafe extern` 1.82、
  `use<'a>` 1.82、`LazyLock` 1.80 等）；`book.toml` 增加 `[rust] edition = "2021"`。
- **示例会切坏中文**：ch16 的摘要由字节切片改为 `chars().take(20)`。
- **接收循环永不结束**：ch23 的发送端补上 `drop(tx)`。
- **死锁修复版仍可能挂住**：ch38 的两把锁改为在块作用域内释放，`join` 之前完成。
- **正文与项目源码不一致**：ch42 补齐参数与缓冲上限（1024 个参数 / 8 MiB 单个 bulk / 64 MiB 连接缓冲、
  `checked_add(2)`），ch43 改用 `JoinSet` 收拢写任务并明确退出顺序，ch44 增加棋盘尺寸校验与参数解析提示，
  ch41 修掉 `take` 的 E0507 写法。
- **正文泄漏本机路径**：5 处真实报错里的 `C:\Users\...` 换成 `<cargo registry>`、`<rustup toolchain>` 占位符。

### 交互

- **错误案例块全面可编辑**：313 个 `rust,editable` 块（1.0.0 时只有 284 个），
  先跑一遍看真实报错，再照「修复」改一行跑第二次对比。
- **本地运行器改为统一走离线 Cargo 工程**（`tools/playground/`）：带 `#[test]` 的代码按 `cargo test` 运行，
  能看到 `test ... ok` 与 `test result: FAILED`。
- **启动时后台预热依赖**，浏览器等待上限从 6 秒放宽到 3 分钟，左下角胶囊会提示「正在预编译依赖」，
  第一次点 Run 不再白等。
- **只监听本机**：非本机 `Host`/`Origin` 一律 403，非 JSON 请求 415。
- **折叠面板适配编辑器与打印**：面板展开后自动让内部 Ace 编辑器重新排版；打印前临时展开全部答案。

### 校验

| 检查项 | 命令 | 本版结果 |
| --- | --- | --- |
| 章节结构、必备小节、链接、每章五案例 | `python tools/check_book.py` | 50 页，0 处问题 |
| 正文普通代码块逐块编译 | `python tools/verify_blocks.py src` | 436 块，0 失败 |
| 错误案例报错与正文逐字比对 | `python tools/verify_cases.py` | 313 块：174 逐字对上、129 正常运行、10 预期运行期失败、0 不一致 |
| 案例答案面板完整 | `python tools/check_book.py`（每条案例必须含 `<details class="case-answer">`） | 220 个面板，0 缺失 |
| 六个项目工程构建并测试 | `python tools/check_projects.py --offline` | 6 个工程，42 个测试全过 |
| 整书构建 | `mdbook build` | 零警告 |

校验脚本本身也在本版收紧：`verify_blocks.py` 从类型检查改为逐块编译；`verify_cases.py` 从「前缀容忍」
改为严格比对首行报错，新增 `--local` 与运行期失败统计（`--local` 与正在运行的运行器互斥，避免两个进程
互相覆盖 `tools/playground/src/main.rs`）；`check_book.py` 增加链接检查、每章必须五个案例小标题、
未列入 `SUMMARY.md` 的文件报错。

### 说明

问题清单由另一位智能体（GPT6.2Astra）提供，逐条在本地复现后修复或确认；
本轮审计与修补由 DSH（DeepSeek Harness）编码智能体完成。

## [1.0.0] - 2026-11-26

第一个完整版本：44 章 + 4 个附录的中文 Rust 教程。在线阅读：<https://yujh129.github.io/rust-book-zh/>

### 内容

- **七部分 44 章**：起步（1–7）、抽象与数据建模（8–17）、内存与资源（18–20）、并发与异步（21–25）、
  工程实践（26–32）、深入语言（33–38）、项目实战（39–44）。
- **4 个附录**：Cargo 速查、标准库与常用 crate、学习路线图、术语表。
- **每章统一结构**：本章目标 → 正文（含一个「深入」小节讲编译器与运行时机制）→ 五个典型错误案例
  → 动手练习 → 小结；错误案例给的是真实 `rustc` 报错原文，并逐条给出原因与修复。

### 交互

- 错误案例可在浏览器里直接改、直接跑（284 个可编辑块）。
- 本地运行器 `python tools/serve_book.py`：书（:3000）与运行器（:3030）一起起，内置 9 个常用 crate。
- 章节导航放在正文末尾做成小胶囊，不再有贴边的巨型翻页条；←/→ 快捷键保留。
- 10 套配色：跟随系统、浅色、Rust、Coal、Navy、Ayu，外加纸墨、樱花、Nord、Gruvbox。

### 工具链

- Rust 2021 edition，核验使用 rustc / cargo 1.100.0；[mdBook](https://github.com/rust-lang/mdBook) 0.5.4。
- 内容资产：`STYLE.md`（写作与代码块规范）、`CHAPTER-PLAN.md`（章节规划）、`WRITING-RULES.md`（协作规则）。

[1.0.1]: https://github.com/yujh129/rust-book-zh/releases/tag/v1.0.1
[1.0.0]: https://github.com/yujh129/rust-book-zh/releases/tag/v1.0.0