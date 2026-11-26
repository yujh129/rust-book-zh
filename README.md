# Rust 学习指南：从入门到进阶

一本写给中文读者的 Rust 系统教程：从第一行 `println!` 讲到 Tokio 异步、unsafe/FFI，最后带你做完七个完整项目。全书 **44 章 + 4 个附录**，约 **3.8 万行 / 106 万字符**，用 [mdBook](https://rust-lang.github.io/mdBook/) 构建。

书里的每个「典型错误案例」都是真实的 `rustc` 输出，而且可以在浏览器里直接改代码、点 **Run**，亲手对比错误写法与正确写法。

## 这本书有什么

- **讲「为什么」，不只是语法清单。** 所有权、借用检查、生命周期、trait 分派、Future 状态机、内存布局……每个机制都从编译器和内存模型的角度解释，而不是让你背规则。
- **每章 5 个真实报错。** 错误代码（E0382、E0502、E0596……）+ `rustc` 完整输出（含 `-->` 位置、`^` 指示符、`help` / `note`）+ 原因 + 修复，训练你先学会读编译器。
- **错误案例可以上手改。** 案例块默认就是可编辑的：先跑一次看报错，再照修复方案改一行、跑第二次，直接看到通过的结果。带 `#[test]` 的代码块按 `cargo test` 运行，能看到 `test ... ok` 和 `test result: FAILED`。
- **从入门一直到能写生产代码。** 后半程覆盖 tracing、serde、clap、axum、CI 与发布，以及七个项目实战：minigrep、JSON 解析器、HTTP 服务器、mini-redis、多人聊天室、贪吃蛇。
- **四套精选配色。** 除 mdBook 自带的主题外，另有纸墨、樱花、Nord、Gruvbox；正文、链接与代码的对比度按 WCAG AA 测过。

## 目录

| 部分 | 章节 | 主题 |
| --- | --- | --- |
| 起步 | 1–7 | Cargo、变量与类型、函数与控制流、复合类型、所有权、借用、切片 |
| 抽象与数据建模 | 8–17 | 结构体、枚举与匹配、错误处理、模块、集合、迭代器与闭包、泛型、trait、生命周期 |
| 内存与资源 | 18–20 | 智能指针、内部可变性、内存布局 |
| 并发与异步 | 21–25 | 线程、共享状态、通道、Future、Tokio |
| 工程实践 | 26–32 | 测试、API 与错误设计、tracing、serde、clap、axum、文档与发布 |
| 深入语言 | 33–38 | 声明宏、过程宏、unsafe/FFI、类型系统、设计模式、错误速查 |
| 项目实战 | 39–44 | minigrep、JSON 解析器、HTTP 服务器、mini-redis、多人聊天室、贪吃蛇 |

## 快速开始

### 1. 构建与阅读

需要 mdBook **0.5.x**（`book.toml` 用的是 0.5 的字段，0.4 不兼容）与 Rust 工具链：

```bash
cargo install mdbook --version 0.5.4 --locked
git clone <仓库地址>
cd rust-book-zh
mdbook serve --open     # http://localhost:3000，改 md 自动刷新
```

也可以 `mdbook build` 之后直接打开 `book/index.html` 离线阅读。

### 2. 在浏览器里跑书里的代码（可选）

mdBook 默认把代码块发给官方 Playground（play.rust-lang.org）运行，需要联网。仓库自带一个本地运行器，装好 Rust 之后可以让代码在你自己的机器上离线编译运行：

```bash
python tools/serve_book.py              # 同时启动书(:3000)与运行器(:3030)
python tools/serve_book.py --no-serve   # 只启动运行器
```

需要 Python 3.8+。页面左下角出现绿色的「本地运行器 已连接」，说明 Run 走的是本机 `rustc`/`cargo`；否则会自动退回官方 Playground。运行器内置 9 个常用 crate（anyhow、clap、rand、serde、serde_json、thiserror、tokio、tracing、tracing-subscriber），单次运行 4 秒超时。详细说明见书里 [1.8 在浏览器里改代码、跑代码](src/00-setup.md)。

## 仓库结构

```text
.
├── book.toml            # mdBook 配置：主题、搜索、代码块行号隐藏标记
├── README.md            # 本文件
├── STYLE.md             # 全书硬规范：章节结构、代码块标记、禁止事项
├── WRITING-RULES.md     # 写作协作规则（分块落盘、自检流程、交付格式）
├── CHAPTER-PLAN.md      # 每章规格：知识点、深入小节、五个错误码、综合作品
├── LICENSE
├── theme/               # extra.css（排版/导航/配色）、head.hbs（运行器转发）、index.hbs、book.js
├── src/                 # SUMMARY.md、前言与环境搭建、part1~part7、appendix A~D
├── tools/
│   ├── serve_book.py         # 一条命令同时启动书与本地运行器
│   ├── playground_server.py  # 本地 Rust 运行器：离线编译并运行书里的代码块
│   ├── playground/           # 运行器用的离线 cargo 小工程
│   ├── verify_blocks.py      # 用 rustc 逐块类型检查普通 rust 示例
│   ├── verify_cases.py       # 把可编辑案例块送进运行器，核对正文里的报错
│   ├── check_book.py         # 章节结构、必备小节与交叉引用检查
│   └── ...                   # 其余为写作期辅助脚本
└── .github/workflows/   # Pages 自动发布 + 代码块校验
```

## 质量把关

| 检查项 | 命令 | 当前结果 |
| --- | --- | --- |
| 章节结构、必备小节、交叉引用 | `python tools/check_book.py` | 50 个条目，0 告警 |
| 普通 rust 代码块能否编译 | `python tools/verify_blocks.py src` | 436 块，0 失败 |
| 可编辑案例的报错是否与正文一致 | `python tools/verify_cases.py` | 284 块：145 块逐字对上，139 块是可运行的修复版或测试，0 不一致 |
| 整书构建 | `mdbook build` | 零警告 |

## 这本书是怎么写出来的：DSH

这本书不是一句一句手敲的，而是在 **DSH（DeepSeek Harness）** 里完成的。DSH 是一个把大模型、工具调用（读写文件、搜索、执行命令）、后台作业和子代理组织在一起的编码智能体环境：模型不只输出文本，还能真的建工程、跑 `rustc`、读构建产物、修问题，并把长任务拆给多个子代理并行推进。

具体是这么用的：

1. **先写规格，再写正文。** [CHAPTER-PLAN.md](CHAPTER-PLAN.md) 把每一章拆成四件事：知识点清单、一个「深入」小节的主题、五个具体错误码、一个综合小作品；[STYLE.md](STYLE.md) 与 [WRITING-RULES.md](WRITING-RULES.md) 规定章节结构、代码块标记与禁止事项。规格定死之后，正文才有统一的密度。
2. **子代理并行、按块落盘。** 一次写 500 行容易失败，所以每章由子代理分块写：先落文件头与前两节并留一个 `<!-- MORE -->` 标记，之后每次用「下一块 + 标记」替换它，最后清掉标记。主代理负责样板章、验收和修补。
3. **机器验收闭环。** 写完不靠感觉，靠脚本：`verify_blocks.py` 把每个 `rust` 块单独丢给 `rustc` 做类型检查；`verify_cases.py` 把可编辑块送进本地运行器，取真实输出的第一行 `error[...]` 与正文里贴的报错逐字比对；`check_book.py` 校验结构、必备小节与链接；最后 `mdbook build` 必须零警告。正文里的报错因此都是真跑出来的。
4. **交互式 Playground 也是在同一套环境里搭的。** 先读 mdBook 生成的 `book.js`，搞清它怎么请求 `play.rust-lang.org`、什么时候创建 Ace 编辑器和 Run 按钮，再用 `theme/head.hbs` 包一层 `fetch` 把请求转给本地运行器；本地不可达就自动退回官方 Playground。

一句话：**AI 负责产出和自查，人负责定方向、定规范、做最终审校。** 观点与取舍由编者负责，代码与报错由脚本保证。

## 贡献

欢迎勘误、补充与讨论。改章节前请先读 [STYLE.md](STYLE.md)，改完至少跑一遍：

```bash
python tools/verify_blocks.py src --only <章名>
python tools/check_book.py
```

## 许可证

[MIT](LICENSE)。代码、正文与配图都可以自由使用、修改、转载，注明出处即可。

## 致谢

书中的示例、报错与结论均以 Rust 稳定版工具链（开发时为 `rustc 1.100`、`mdbook 0.5.4`）的实际输出为准。
