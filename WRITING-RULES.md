# 写作规则（协作写作者必读）

你正在为一本**中文 Rust 教材**撰写其中一章。读者画像：有其他语言基础、英语不好、看不懂官方英文文档、想从入门一路学到能写生产代码。你的任务不是罗列语法，而是**讲清"为什么"**，并用真实的编译器报错教读者读懂 rustc。

## 1. 动笔之前

按顺序读三份材料，读完再写：

1. `STYLE.md`（本书硬规范）
2. 本文件（协作规则）
3. `src/part2/ch08-structs.md`（已完成的样板章，照着它的密度、语气、结构写）

## 2. 分块写，一次不超过 200 行

单次写 500 行会失败。按下面的节奏落盘：

- **第一次 `write`**：文件标题 `# 第 N 章 标题` + `> **本章目标**` 引用块（5 条）+ 开头引导段 + 前 2 个小节，文件最后单独一行 `<!-- MORE -->`
- **之后每次 `edit`**：`old_string` 传 `<!-- MORE -->`，`new_string` 传「下一块正文 + 空行 + `<!-- MORE -->`」
- **最后一块**：`new_string` 里不再带标记，把 `<!-- MORE -->` 彻底清掉

## 3. 必备结构

- `## N.M 小节标题` 共 5–8 个（N 是章号）
- 至少一个「深入」小节，讲编译器/运行时/内存机制（不是语法复述）
- `## 典型错误案例`：**恰好 5 个**，每个用三级标题 `### 案例 k：现象——E0xxx`
- `## 动手练习`：4–5 题，从易到难，至少一题需要动手跑代码验证
- `## 小结`：6–10 条，每条一句话，能被单独摘出来当检查表
- 全章 450–800 行（含空行与代码块）

## 4. 错误案例的严格格式

每个案例四段，缺一不可：

```markdown
### 案例 1：一句话描述现象——E0507

```rust,noplayground
<能触发该错误的完整代码>
```

```text
<rustc 真实输出，含 error[Exxxx]、 --> src/main.rs:行:列、^ 指示符、help/note 行>
```

**原因**：为什么会报错，指向编译器检查的规则。

**修复**：怎么改，附一段可运行的 ```rust 正确写法。
```

**报错必须是真实输出，不许编造**。验证方法（在 rust-book 目录下）：

```bash
mkdir -p .scratch/chNN
# 把错误代码存成 .scratch/chNN/e1.rs，然后：
cd .scratch/chNN && rustc --edition 2021 --crate-type bin -o /dev/null e1.rs 2>&1 | head -20
```

把真实 stderr 粘进 ```text 块。行号要和代码块里的行号对得上；展示时把文件名写成 `src/main.rs`。

## 5. 代码块标记规则

| 标记 | 用途 |
|---|---|
| ```rust | 必须自包含、可编译、可运行（含 `fn main`） |
| ```rust,noplayground | 错误演示、片段、需要第三方 crate 的代码 |
| ```text | rustc 报错输出、内存布局图、纯文本示意 |
| ```console | 终端会话（含命令与输出） |
| ```toml / ```bash / ```json | 配置文件与命令行 |

## 6. 禁止事项

- 禁止 `TODO`、省略号占位、`<!-- MORE -->` 残留
- 禁止"下一章我们将……"式的预告段落
- 禁止正文里写 HTML
- 禁止表格单元格里嵌套反引号（会触发 mdBook 的 HTML 标签警告）
- 禁止机翻腔、空话（"Rust 是一门优秀的语言"这类）
- 禁止把 `cargo run` 的输出说成编译输出

## 7. 自检（必须做）

在 `D:\code\dshwork\rust-book` 目录下：

```bash
python tools/verify_blocks.py src --only chNN 2>&1 | tail -6
```

必须输出 `0 failed`。有失败块时：要么修代码让它真能编译；要么若该块本来就是故意报错的演示，把标记改成 `rust,noplayground`（可用 `python tools/fix_playground_tags.py <文件>` 自动处理）。

```bash
PYTHONIOENCODING=utf-8 python tools/check_book.py 2>&1 | grep -A8 chNN
```

没有输出即通过。若提示 `missing section`、`error cases reason/fix`、`lines outside`，按提示修正。

## 8. 边界

只改你这一个章节文件，以及 `.scratch/chNN/` 下的临时文件。**不要**运行 `mdbook build`（会与其他写作者冲突），不要修改 `SUMMARY.md`、`book.toml`、`theme/`、其他章节。

## 9. 交付回复

写完后回复四行：文件路径 · 行数 · `verify_blocks` 结果 · 五个错误码（如 E0004/E0308/E0509/E0382/E0599）。
