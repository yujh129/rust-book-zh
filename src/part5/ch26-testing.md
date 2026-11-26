# 第 26 章 测试：单元、集成与文档测试

> **本章目标**
>
> - 会给一个 crate 写 `#[test]` 函数，用名字过滤、`--list`、`--ignored` 只跑想看的那部分
> - 会用 `assert!`、`assert_eq!`、`assert_ne!` 和自定义断言，从失败信息里直接读出左右两边的值
> - 会把测试放进 `#[cfg(test)]` 模块，说清它为什么能测私有项、又为什么不进入发布二进制
> - 会写 `#[should_panic(expected = ...)]` 和返回 `Result` 的测试，并把单元测试、`tests/` 集成测试、文档测试放到各自该在的位置
> - 会用 trait 手写替身代替 mock 框架，并知道 `#[bench]` 为什么只在 nightly、稳定版该用什么

在 `main` 里塞一串 `println!`，改一次跑一次，用眼睛比对输出——这是多数人验证程序的第一步。函数涨到十几个之后这个办法就失效了：输出混在一起，跑一次要几十秒，上次的正确值没人记得。

测试框架解决的正是这三件事：把期望值写成代码、让每条检查独立执行、失败时指出是哪一条、差多少。Java 有 JUnit，Python 有 pytest，Go 把 `testing` 放进标准库；Rust 的做法是让 `#[test]` 成为语言属性，把运行器 `libtest` 随工具链一起发布，再由 `cargo test` 统一驱动。你不需要往 `Cargo.toml` 里加任何依赖，就能写出生产项目规模的测试。

代价是这套工具刻意做得很薄：没有内置 mock，没有稳定的基准测试入口，没有 fixture 机制。Rust 更希望你把依赖抽象成 trait，用普通代码搭替身，26.7 会给出完整例子。先写第一个测试。

## 26.1 第一个测试：#[test] 与 cargo test

新建一个库 crate：

```console
$ cargo new demo --lib
```

把 `src/lib.rs` 写成这样：

```rust,ignore
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 2), 4);
    }

    #[test]
    fn adds_negative_numbers() {
        assert_eq!(add(-1, -1), -2);
    }
}
```

运行 `cargo test`：

```console
$ cargo test
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.84s
     Running unittests src/lib.rs (target/debug/deps/demo-8f3c2b1a)

running 2 tests
test tests::adds_negative_numbers ... ok
test tests::adds_two_numbers ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

测试函数有三条硬性规定：必须标 `#[test]`；不能有参数；返回值是 `()`（或者 26.4 讲的 `Result`）。名字随意，但输出里显示的是模块路径加函数名——`tests::adds_two_numbers` 里的 `tests` 来自 `mod tests`，与"测试"这个词无关，你把它改名成 `mod checks`，输出就变成 `checks::adds_two_numbers`。

`#[cfg(test)]` 和 `use super::*` 这两行先照抄，26.3 会分别解释它们解决什么问题。

`cargo test` 把编译、发现、运行、报告四件事串在一起：先按测试模式编译一遍，从二进制里收集所有标了 `#[test]` 的函数，再逐个执行，最后按名字排序打印结果。这条流水线在 26.8 展开。

只想跑一条时，把名字当参数传进去。匹配规则是**子串**，不是全等：

```console
$ cargo test adds_negative
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running unittests src/lib.rs (target/debug/deps/demo-8f3c2b1a)

running 1 test
test tests::adds_negative_numbers ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
```

注意最后那句 `1 filtered out`：过滤掉的测试不计入通过数，CI 里也不会因为它们没跑而报警，所以"本地只跑了一条，CI 却挂了"是常见事故。要精确匹配加 `--exact`；要只看名字不执行，用 `cargo test -- --list`，它逐行打印 `tests::adds_two_numbers: test` 这样的条目，最后一行是 `2 tests, 0 benchmarks`。

`--` 是分隔符：它前面是 cargo 自己的参数（`--lib`、`--test`、`--doc`），后面是交给测试二进制的参数（`--list`、`--test-threads`、`--ignored`）。写成 `cargo test --test-threads=1` 会被 cargo 拒绝，因为 cargo 没有这个选项——这个位置错误几乎每个人都犯过一次。

测试失败时，`cargo test` 的退出码是 101，通过时是 0。CI 只认退出码，所以不要在脚本里用 `grep` 判断测试是否通过。

## 26.2 断言：assert!、assert_eq! 与失败信息

测试的输出手段就三个宏：

- `assert!(条件)`：条件为假时 panic
- `assert_eq!(左, 右)`：两边不等时 panic，并打印两边的值
- `assert_ne!(左, 右)`：两边相等时 panic

三个都接受额外的格式化参数：`assert!(x > 0, "x 必须为正，实际是 {x}")`。消息按 `format!` 的规则拼（第 30 章）。

能写 `assert_eq!(a, b)` 就不要写 `assert!(a == b)`。前者借用了两边再比较，所以比较 `String` 不会把它移动走，也不必 `clone`；失败时要打印两边的值，所以它要求类型实现 `Debug` 和 `PartialEq`——给自定义类型写 `#[derive(Debug, PartialEq)]` 就是为了这个（第 8 章）。而 `assert!(a == b)` 只告诉你"条件为假"，不告诉你两边的值。

下面这个校验函数把契约写进断言，消息里带上实际值：

```rust
#[derive(Debug)]
struct Config {
    retries: u32,
    timeout_ms: u64,
}

fn validate(c: &Config) {
    assert!(
        c.retries <= 10,
        "retries 不能超过 10，实际是 {}",
        c.retries
    );
    assert!((100..=30_000).contains(&c.timeout_ms), "timeout_ms 超出范围：{}", c.timeout_ms);
}

fn main() {
    validate(&Config { retries: 3, timeout_ms: 5_000 });
    println!("配置合法");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_config() {
        validate(&Config { retries: 3, timeout_ms: 5_000 });
    }

    #[test]
    fn rejects_too_many_retries() {
        let c = Config { retries: 99, timeout_ms: 5_000 };
        validate(&c);
    }
}
```

失败时输出是这样：

```text
running 2 tests
test tests::accepts_valid_config ... ok
test tests::rejects_too_many_retries ... FAILED

failures:

---- tests::rejects_too_many_retries stdout ----

thread 'tests::rejects_too_many_retries' panicked at src/main.rs:8:5:
retries 不能超过 10，实际是 99
```

顺着这几行就能定位：哪个测试、哪一行、哪条断言、实际值是多少。`assert!` 只打印你给的字符串，`assert_eq!` 会额外打印两个值——所以自定义消息写得再详细，也替代不了 `assert_eq!` 的差值信息。消息的职责是补上"你没在值里看到的东西"，比如输入参数。

浮点是例外。`0.1 + 0.2` 在二进制浮点里不等于 `0.3`，`assert_eq!` 会如实报出：

```text
assertion `left == right` failed
  left: 0.30000000000000004
 right: 0.3
```

浮点比较要写误差范围：`assert!((a - b).abs() < 1e-9, "{a} 与 {b} 相差过大")`。

条件比较绕时用 `matches!`，比一长串 `if let` 清楚：`assert!(matches!(parse("42"), Ok(42)), "期望 Ok(42)，实际是 {:?}", parse("42"));`。

把常用检查提炼成断言函数时，一定要把参数写进消息。panic 位置指向函数内部的 `assert!` 那一行，如果消息里没有值，失败时你只知道"某个断言没过"，不知道是哪个调用点、传了什么。

## 26.3 测试模块与 cfg(test)

测试和被测代码放在同一个文件里，外面套一层 `#[cfg(test)] mod tests`。这个写法里三样东西各管一件事。

`#[cfg(test)]` 是条件编译（第 18 章）。`cargo build` 不会定义 `test` 这个配置，整个模块在编译前就被删掉：不参与类型检查，不进二进制，也没有运行时开销。`cargo test` 给 rustc 传 `--test`，同时打开这个配置，模块才存在。

所以测试代码可以随便一点：`unwrap()`、写死的常量、`println!` 调试输出，都只活在测试构建里。反过来，`#[cfg(test)]` 里的错误也只在 `cargo test` 时才暴露——`cargo build` 通过，不代表测试代码能编译，案例 3 就是这个坑。

`mod tests` 是普通模块，所以它能看见父模块的私有项：子模块可以访问祖先模块里的所有东西（第 11 章）。`use super::*;` 把这层名字一次导入，省掉逐个写 `use crate::xxx`。这就是单元测试能直接测私有函数的原因——不必为了测试把 API 全部改成 `pub`。

`#[test]` 只认函数，不认模块。要分组，就在 `mod tests` 里继续套子模块，失败信息里的名字会带上完整路径，比如 `tests::parsing::parses_prefixed_hex`。

被测函数是私有的也没关系：

```rust
fn parse_hex_byte(s: &str) -> Option<u8> {
    let digits = s.trim_start_matches("0x");
    u8::from_str_radix(digits, 16).ok()
}

fn main() {
    println!("{:?}", parse_hex_byte("0xff"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixed_hex() {
        assert_eq!(parse_hex_byte("0xff"), Some(255));
    }

    #[test]
    fn rejects_overflow() {
        assert_eq!(parse_hex_byte("0x100"), None);
    }
}
```

`parse_hex_byte` 没有 `pub`，发布时只有 `main` 用它；测试模块通过 `use super::*` 拿到它。真实项目里这类工具函数往往就该是私有的，测试放在同一个文件里，改实现时两边一起改，比把内部细节暴露成公有 API 更省事。

一个 crate 里的单元测试只能看见本 crate 的东西：私有项能测，公有项也能测。要验证外部用户能不能按文档使用这个库，那是集成测试的活，26.5 讲。

## 26.4 失败路径：should_panic 与返回 Result

有些函数的契约本身就是 panic：`Vec` 越界（第 9 章）、`Option::unwrap`、入口处的参数校验。测这类行为要 `#[should_panic]`。裸的写法太宽松：函数里任何一处 panic 都算通过，包括你写错下标导致的越界。要限定原因，就加 `expected`：

```rust,ignore
#[test]
#[should_panic(expected = "端口必须是数字")]
fn rejects_non_numeric() {
    parse_port("http");
}
```

`expected` 是子串匹配，区分大小写，不支持正则。panic 载荷是 `&str` 或 `String` 时都能匹配。期望写错和根本没 panic 是两种失败：后者报 `test did not panic as expected`，前者报：

```text
note: panic did not contain expected string
      panic message: "端口必须是数字: ParseIntError { kind: InvalidDigit }"
 expected substring: "端口不合法"
```

顺带能看到 panic 位置指向 `parse_port` 内部的 `expect`，不是测试函数——排查时先看这一行。完整例子：

```rust
fn parse_port(s: &str) -> u16 {
    s.parse().expect("端口必须是数字")
}

fn main() {
    println!("{}", parse_port("8080"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_port() {
        assert_eq!(parse_port("8080"), 8080);
    }

    #[test]
    #[should_panic(expected = "端口必须是数字")]
    fn rejects_non_numeric() {
        parse_port("http");
    }
}
```

测试名后面的 `- should panic` 是 libtest 加的标记，提醒你这条通过的条件是确实 panic 了：`test tests::rejects_non_numeric - should panic ... ok`。

另一种写法是让测试返回 `Result`，用 `?` 处理可恢复错误：

```rust,ignore
#[test]
fn parses_port() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = "8080".parse()?;
    assert_eq!(port, 8080);
    Ok(())
}
```

返回 `Err` 时测试失败，harness 用 Debug 打印错误：

```text
---- tests::returns_err stdout ----
Error: "invalid digit found in string"
```

能这样写是因为测试函数的返回值要实现 `Termination`：`()` 实现了，`Result<(), E: Debug>` 也实现了，`main` 能返回 `Result` 靠的是同一个 trait（第 10 章）。

`#[should_panic]` 和返回 `Result` 不能同时用，编译器会拒绝整条测试函数，理由是 `functions using #[should_panic] must return ()`：用返回 `Err` 表示失败、用 panic 表示"预期中的失败"，两种语义没法共存。

选择上，被测函数返回 `Result`，测试就返回 `Result` 并用 `?` 串起来，失败信息比 `unwrap` 清楚；`should_panic` 只留给契约确实是 panic 的函数。

## 26.5 测试的组织：单元、集成与文档

一个 crate 的测试分三处放，各有分工。

单元测试写在源文件里，和被测代码同在一个模块树。它编译进库的测试版本，能访问私有项，跑得最快，适合测单个函数、一个模块的内部不变量。

集成测试放在项目根目录的 `tests/` 下，每个 `.rs` 文件是一个独立的 crate。它只能 `use 你的crate名::...` 使用公有 API，看不到私有项——这恰好是用户的视角。`cargo test` 会为 `tests/` 里每个文件单独编译、单独运行：

```rust,ignore
// tests/public_api.rs
use demo::add;

#[test]
fn adds_through_public_api() {
    assert_eq!(add(2, 3), 5);
}
```

```console
$ cargo test --test public_api
     Running tests/public_api.rs (target/debug/deps/public_api-db3b0f31)

running 1 test
test adds_through_public_api ... ok
```

注意测试名没有模块前缀：每个集成测试文件本身就是 crate 根。多个文件共享辅助代码时，放到 `tests/common/mod.rs`，用 `mod common;` 引入；不要放 `tests/common.rs`，那会被当成一个测试 crate 去跑，输出里会多一行 `running 0 tests`。

目录长这样：

```text
demo/
├── src/lib.rs          # 单元测试写在文件末尾的 mod tests 里
├── src/bin/cli.rs
└── tests/
    ├── public_api.rs   # 一个文件 = 一个测试 crate
    └── common/mod.rs   # 共享辅助代码，不是测试目标
```

文档测试是文档注释里的代码块。`cargo test` 会把每个代码块编译成独立的 crate、链接本库、再运行。给 `add` 补上文档：

```rust,ignore
/// 把两个整数相加。
///
/// # 示例
///
/// ```
/// use demo::add;
/// assert_eq!(add(2, 2), 4);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

```console
$ cargo test --doc
   Doc-tests demo

running 1 test
test src/lib.rs - add (line 5) ... ok
```

输出里的 `(line 5)` 是代码块起始行，改文档时会跟着变。文档测试最大的价值是防过期：示例与真实 API 一起编译，签名一改，测试立刻失败，不会留下一堆跑不通的 README。

代码块可以加属性：`no_run` 只编译不运行（适合要联网或读写文件的例子）、`ignore` 跳过、`should_panic` 要求 panic、`compile_fail` 要求编译失败。行首加 `# ` 能把辅助行藏起来不显示，但依然参与编译。

三者的差别可以这样记：单元测试能测私有项、和代码同文件、跑得最快、数量最多；集成测试只能测公有 API、是独立 crate、编译更慢、验证对外契约；文档测试测的是文档示例，防止示例过期，每个代码块单独编译。

`cargo test` 默认按单元、集成、文档的顺序跑；`--lib`、`--test 名字`、`--doc` 分别只跑一类。二进制 crate 没有 lib 目标，`tests/` 里的集成测试无法 `use` 它的内部，只能运行命令比对输出——这也是把逻辑放进 `src/lib.rs`、`main` 只留一层壳的理由之一。

## 26.6 控制运行：ignore、并行与 --test-threads

有些测试不能每次都跑：要连数据库、要几分钟、只在某个平台成立。用 `#[ignore]` 把它们挪出默认流程：

```rust,ignore
#[test]
#[ignore = "需要本地 Redis"]
fn talks_to_redis() {
    let client = connect("redis://127.0.0.1:6379").unwrap();
    assert_eq!(client.ping(), "PONG");
}
```

默认跑的时候它们不计入失败，只在输出里留一行：

```text
test tests::talks_to_redis ... ignored, 需要本地 Redis
```

只跑被忽略的用 `cargo test -- --ignored`；连忽略的一起跑用 `cargo test -- --include-ignored`。CI 里常见两个 job：快的默认集合，和需要外部依赖的 ignored 集合。

并行是默认行为：`cargo test` 按 CPU 可用并行度开线程，每个测试占一个线程。所以测试之间不能有顺序依赖，也不能共享可变的全局状态。

反例，两个测试抢同一个文件：

```rust,noplayground
use std::fs;

fn main() {
    fs::write("shared.txt", "hello").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_creates_file() {
        fs::write("shared.txt", "hello").unwrap();
    }

    #[test]
    fn reader_sees_file() {
        assert_eq!(fs::read_to_string("shared.txt").unwrap(), "hello");
    }
}
```

`reader_sees_file` 可能先跑，`unwrap` 直接 panic；也可能后跑，于是通过。这种时过时不过的测试叫 flaky，比没有测试更糟：你会去怀疑随机数、怀疑编译器，就是不怀疑自己的测试。

判断方法很简单：串行再跑一遍（`cargo test -- --test-threads=1`）。串行通过、并行失败，基本就是隐藏的共享状态。

`--test-threads` 是测试二进制的参数，必须放在 `--` 后面。修法不是长期串行——那会掩盖问题，也变慢——而是让每个测试自带资源：用互不相同的临时路径，每个测试自己建目录、自己清理，不依赖别人留下的状态。

确实需要串行的场合不多：测试要写环境变量、要占固定端口、要访问同一块硬件。这时可以在测试模块里放一个 `static LOCK: Mutex<()> = Mutex::new(());`，每条测试先拿锁再干活。锁只能约束同一进程内的并发，而 cargo 默认就是把一个测试二进制的所有测试放在一个进程里跑，所以够用。

输出捕获也值得知道：测试里的 `println!` 默认被 harness 收走，只有失败时才在 `---- 名字 stdout ----` 下面回放，通过的测试不打印。调试时加 `--nocapture`（新版本写作 `--no-capture`，两个都认）直接输出，但并行下多个测试的输出会交错，最好配合 `--test-threads=1`。

`--show-output` 则是把通过测试的输出也打出来，适合排查"到底哪个测试打乱了输出"。cargo 层还有 `--no-fail-fast`：默认一个测试目标失败后 cargo 就停下，加上它可以把 lib、集成测试、文档测试全跑完，一次看到所有失败。

## 26.7 为什么 Rust 没有内置 mock：trait 与手写替身

mock 要解决的问题是"验证协作者有没有被按约定调用"：注册成功时发一封欢迎邮件、失败时不要发。多数语言的 mock 框架靠运行时替换方法或动态代理实现，而 Rust 没有反射，也没有运行时的类加载，标准库因此不提供 mock。社区方案 `mockall` 用过程宏在编译期生成替身，但它不在标准库里。

更根本的原因是：Rust 里不需要运行时替换。把协作者写成 trait，业务代码依赖 trait 而不是具体类型，测试时传另一个实现就够了——依赖注入在编译期完成。

```rust
trait Notifier {
    fn send(&self, to: &str, text: &str);
}

struct Email;

impl Notifier for Email {
    fn send(&self, to: &str, text: &str) {
        println!("发送邮件到 {to}：{text}");
    }
}

struct Signup<N: Notifier> {
    notifier: N,
}

impl<N: Notifier> Signup<N> {
    fn register(&self, email: &str) -> Result<u64, String> {
        if !email.contains('@') {
            return Err(format!("邮箱格式不对：{email}"));
        }
        self.notifier.send(email, "欢迎注册");
        Ok(42)
    }
}

fn main() {
    let s = Signup { notifier: Email };
    println!("{:?}", s.register("ana@example.com"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct Fake {
        sent: Rc<RefCell<Vec<String>>>,
    }

    impl Notifier for Fake {
        fn send(&self, to: &str, text: &str) {
            self.sent.borrow_mut().push(format!("{to}: {text}"));
        }
    }

    #[test]
    fn sends_welcome_mail_on_success() {
        let fake = Fake::default();
        let s = Signup { notifier: fake.clone() };

        assert_eq!(s.register("ana@example.com"), Ok(42));
        assert_eq!(fake.sent.borrow().as_slice(), ["ana@example.com: 欢迎注册"]);
    }

}
```

`Signup` 只认识 `Notifier`，测试给它一个 `Fake`。`Fake` 做两件事：记录每次调用，返回固定值。`Rc<RefCell<Vec<String>>>` 是为了让测试既能把它交给 `Signup`，又能在事后查看记录——`clone` 复制的是指针，数据只有一份。

三个容易踩的点。替身要"笨"：只记录、只返回固定值，不要在替身里写判断逻辑，否则测试失败时你分不清是业务代码错了还是替身错了。用泛型 `Signup<N: Notifier>` 时，每个 `N` 都实例化一份代码，编译期分发、没有虚表开销；需要运行时换实现（比如从配置决定发邮件还是发短信）就改成 `Box<dyn Notifier>`，代价是一次动态分发。最后，能靠返回值判断的不要验证交互：`register` 返回了 `Ok(42)` 就说明成功了，只有"发邮件"这种副作用无返回值可断言时才需要检查调用记录。

过度 mock 是个信号：一个函数要传五个替身才能测，通常是它同时做了太多事。先拆分，再谈测试。

## 26.8 深入：cargo test 背后发生了什么

`cargo test` 并不是一个魔法命令，它只是把 rustc 和测试二进制串起来。

编译阶段，cargo 给每个测试目标加上 `--test` 调用 rustc。这个标志做三件事：打开 `cfg(test)`、链接 libtest、注入一个 `main`——所以只有 `#[test]` 没有 `main` 的文件也能跑测试（案例 2）。`#[test]` 本身在编译期被展开成一个静态描述符，里面存着测试名（模块路径加函数名）、函数指针、是否 ignore、should_panic 的期望、源码位置。这些描述符被收集进一个数组，交给 libtest。

运行阶段，libtest 解析命令行参数，起一个线程池（默认线程数等于 CPU 可用并行度），每个线程从队列里取一个测试执行。执行时用 `catch_unwind` 包住测试函数：返回 `Ok` 就是通过，返回 `Err` 就是失败；标了 `should_panic` 的测试反过来判断，还要比对 panic 消息里是否包含 `expected`。panic 被接住，所以一条测试失败不会让整个进程退出——这也是为什么一个 panic 的测试之后，其余测试还能继续跑。

输出捕获也是在这一层做的：测试线程的 stdout 被临时替换，失败的测试才把缓冲区回放到 `---- 名字 stdout ----` 下面。全部跑完后，libtest 按结果决定退出码：全过是 0，有失败是 101。

基准测试是另一条路。`#[bench]` 至今仍在 nightly 上，要写 `#![feature(test)]` 才能用，稳定版编译器会直接拒绝。稳定版有两个选择：手写计时循环，或者用 criterion。手写版本要记住三件事：用 `black_box` 阻止优化器把计算删掉；先跑几轮热身；测 release 构建。

```rust
use std::hint::black_box;
use std::time::Instant;

fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    let start = Instant::now();
    for _ in 0..100 {
        black_box(fib(black_box(20)));
    }
    let per_call = start.elapsed().as_secs_f64() * 1e6 / 100.0;
    println!("fib(20) 平均 {per_call:.1} 微秒");
}
```

criterion 做的是同一件事，但带统计分析：多次采样、给出置信区间、和上次的 baseline 比较、报告是否"性能回归"。它要作为开发依赖引入，并且关闭默认的测试 harness：

```toml
[dev-dependencies]
criterion = "0.5"

[[bench]]
name = "fib"
harness = false
```

```rust,noplayground
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn bench_fib(c: &mut Criterion) {
    c.bench_function("fib 20", |b| b.iter(|| fib(black_box(20))));
}

criterion_group!(benches, bench_fib);
criterion_main!(benches);
```

基准测试不要写成 `#[test]`：它慢、噪声大，而且 debug 构建的数字没有意义。放 `benches/` 下，用 `cargo bench --bench fib` 跑，和单元测试分开。

## 典型错误案例

下面五条覆盖测试从编译到运行的主要坑：三条编译期错误、两条测试运行期失败。每条的错误输出都是真实运行得到的，路径与行号可以和代码对上。

### 案例 1：断言两边类型不同——E0308

```rust,editable
fn main() {
    let values = vec![1, 2, 3];
    let expected: u32 = 3;
    assert_eq!(values.len(), expected);
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:4:30
  |
4 |     assert_eq!(values.len(), expected);
  |                              ^^^^^^^^ expected `usize`, found `u32`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Vec::len` 返回 `usize`，而 `expected` 被标注成 `u32`。`assert_eq!` 要求两边能用同一个 `PartialEq` 实现比较，编译器不会在比较时做整数转换或类型提升。

**修复**：把期望值的类型改成 `usize`。类型对齐之后，这条测试才真正在比较长度：

```rust,editable
fn main() {
    let values = vec![1, 2, 3];
    let expected: usize = 3;
    assert_eq!(values.len(), expected);
}
```

</details>

### 案例 2：把测试写进 bin，却没有 main——E0601

```rust,noplayground
#[test]
fn adds_two_numbers() {
    assert_eq!(2 + 2, 4);
}
```

```text
error[E0601]: `main` function not found in crate `demo`
 --> src/main.rs:4:2
  |
4 | }
  |  ^ consider adding a `main` function to `src/main.rs`
```

（这段代码没有运行按钮：它没有 `main`，只有放进 `cargo new` 的项目里用 `cargo test` 才跑得起来。本地运行器对含 `#[test]` 的代码一律按 `cargo test` 编译，跑出来会直接报「测试通过」，看不到上面这条 E0601。）

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`#[test]` 不生成程序入口。入口是 `cargo test` 用 `--test` 编译时由 libtest 注入的，所以这份文件 `cargo test` 能跑过，`cargo build` 却找不到 `main`。把示例改写成测试时最容易踩到这条。

**修复**：bin 里保留 `main`，测试作为附加内容存在；或者把逻辑移进 `src/lib.rs`，测试也放过去。

```rust,editable
fn main() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn adds_two_numbers() {
    assert_eq!(2 + 2, 4);
}
```

</details>

### 案例 3：测试模块里找不到父模块的函数——E0425

```rust,editable
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 2), 4);
    }
}
```

```text
error[E0425]: cannot find function `add` in this scope
 --> src/main.rs:9:20
  |
9 |         assert_eq!(add(2, 2), 4);
  |                    ^^^ not found in this scope
  |
help: consider importing this function
  |
7 +     use crate::add;
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`mod tests` 是子模块，名字不会自动从父模块继承；`add` 定义在 crate 根，子模块里要先导入才能用。更隐蔽的是，`cargo build` 并不报这个错——整个模块被 `cfg(test)` 删掉了，只有 `cargo test` 才会编译它。

**修复**：在 `mod tests` 开头写 `use super::*;`，或者按编译器建议写 `use crate::add;`：

```rust,editable
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 2));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 2), 4);
    }
}
```

</details>

### 案例 4：断言失败——left 与 right 对不上

```rust,editable
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 2), 5);
    }
}
```

```text
running 1 test
test tests::adds_two_numbers ... FAILED

failures:

---- tests::adds_two_numbers stdout ----

thread 'tests::adds_two_numbers' panicked at src/main.rs:11:9:
assertion `left == right` failed
  left: 4
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    tests::adds_two_numbers

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`add(2, 2)` 返回 4，断言写的期望值是 5。`assert_eq!` 把实际值放在 `left`、期望值放在 `right`，输出直接给出差值；命令的退出码是 101。

**修复**：期望值改成 4，或者去修 `add` 的实现——先确认哪一边才是对的：

```rust,editable
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    assert_eq!(add(2, 2), 4);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_two_numbers() {
        assert_eq!(add(2, 2), 4);
    }
}
```

</details>

### 案例 5：should_panic 却没 panic

```rust,editable
pub fn parse_port(s: &str) -> u16 {
    s.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn rejects_non_numeric() {
        parse_port("8080");
    }
}
```

```text
running 1 test
test tests::rejects_non_numeric --- FAILED

failures:

---- tests::rejects_non_numeric stdout ----
note: test did not panic as expected at src/main.rs:11:8

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`unwrap_or(0)` 把解析失败吞成了 0，函数根本不会 panic；测试却声明"它应当 panic"。`should_panic` 的通过条件是测试函数内确实发生了 panic，返回正常值同样算失败。

**修复**：先明确契约。端口解析函数遇到非法输入应当 panic，就用 `expect` 并给出消息；测试同时补上 `expected`，让这个 panic 有明确原因：

```rust,editable
pub fn parse_port(s: &str) -> u16 {
    s.parse().expect("端口必须是数字")
}

fn main() {
    println!("{}", parse_port("8080"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "端口必须是数字")]
    fn rejects_non_numeric() {
        parse_port("http");
    }
}
```

</details>

## 动手练习

1. 给 `fn clamp(x: i32, lo: i32, hi: i32) -> i32` 写四条测试：落在区间内、低于下界、高于上界、上下界相等，全部用 `assert_eq!`，并故意写错一条，观察输出的 left 与 right。
2. 把 26.4 的 `parses_port` 改成返回 `Result` 的测试，再给它加 `#[should_panic]`，确认编译器拒绝这种组合，并读一遍报错里给出的理由。
3. 为 26.7 的 `Signup` 补一条失败路径测试：非法邮箱时不发邮件，用 `fake.sent.borrow().is_empty()` 断言。
4. 建一个 `tests/` 集成测试文件，只调用 `demo::add`，用 `cargo test --test 文件名` 单独跑它，再用 `--doc` 只跑文档测试，比较两次输出里 `Running` 那一行。
5. 给一条需要外部依赖的测试加 `#[ignore = "需要本地 Redis"]`，分别用默认、`--ignored`、`--include-ignored` 跑三次，比较输出里的计数。

## 小结

- `#[test]` 标记测试函数，`cargo test` 把编译、发现、运行、报告串成一条流水线；失败时退出码是 101。
- 断言优先用 `assert_eq!` / `assert_ne!`，它们借用两边并打印值；自定义消息补上"值里看不到的输入"。
- 测试模块写成 `#[cfg(test)] mod tests`，`cargo build` 根本不编译它，所以测试代码的错误只在 `cargo test` 时暴露。
- `#[should_panic(expected = "...")]` 限定 panic 的原因；被测函数返回 `Result` 时，测试也返回 `Result` 并用 `?`。
- 单元测试能测私有项，`tests/` 下的集成测试只测公有 API，文档注释里的代码块也会被编译运行。
- 测试默认并行，不能依赖执行顺序或共享状态；`#[ignore]` 隔离慢测试，测试二进制的参数要放在 `--` 后面。
- Rust 没有内置 mock：把协作者抽成 trait，测试传手写替身，让它只记录调用、返回固定值。
- 基准测试不属于 `cargo test`：`#[bench]` 只在 nightly，稳定版用手写 `Instant` + `black_box`，或用 criterion。
