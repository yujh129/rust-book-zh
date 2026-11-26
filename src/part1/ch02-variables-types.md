# 第 2 章 变量、类型与可变性

> **本章目标**
> - 分清绑定、赋值、遮蔽三件不同的事，知道什么时候该用哪个
> - 掌握整数溢出的四种处理策略，理解 debug 与 release 的行为差异及其代价
> - 搞懂 `as` 转换和 `From/Into` 的区别，知道哪个是安全的
> - 从内存布局角度理解 char 为什么是 4 字节、`&str` 和 `String` 差在哪
> - 认识类型推断的边界：什么时候必须写类型标注
> - 躲开 5 个最常见的类型错误

## 2.1 绑定、赋值与遮蔽

Rust 里 `let` 是**绑定**（binding）：把名字和值关联起来。之后有两种改法，语义完全不同。

```rust
fn main() {
    // 1. 赋值：改的是同一个变量，要求 mut
    let mut a = 1;
    a = 2;
    println!("a = {a}"); // 2

    // 2. 遮蔽：创建一个新变量，可以换类型
    let b = "42";
    let b: i32 = b.parse().unwrap();
    let b = b * 2;
    println!("b = {b}"); // 84
}
```

对比一下这两种"改"：

| | 赋值 `a = 2` | 遮蔽 `let a = 2` |
| --- | --- | --- |
| 需要 `mut` 吗 | 需要 | 不需要 |
| 能换类型吗 | 不能 | 能 |
| 原来的值 | 被覆盖 | 还在内存里，但名字不可达（会被释放或继续借用） |
| 编译期可追踪吗 | 是同一个变量 | 是两个不同的变量 |
| 典型用途 | 累加器、状态机 | 解析后的类型转换、逐步收窄 |

**遮蔽的经典用法是"解析后收窄类型"**：

```rust
fn main() {
    let input = "  1.75 ";
    let height = input.trim();            // &str
    let height: f64 = height.parse().unwrap(); // 同一个名字，换成了 f64
    println!("{height}");
}
```

如果硬要用 `mut`，就得写成 `let mut height = input.trim();` 再设法转成 `f64`——做不到，类型不能变。所以这里遮蔽是唯一自然写法。

**什么时候不该用遮蔽**：依赖关系没断开时容易埋坑。

```rust
fn main() {
    let value = 10;
    let value = value + 1;   // 合理：基于旧值算新值
    println!("{value}");

    let total = 100;
    {
        let total = total + 1;   // 内层遮蔽，离开这个块就没了
        println!("内层 {total}");
    }
    println!("外层 {total}");    // 还是 100 —— 别误以为改掉了
}
```

块作用域内的遮蔽只在该块内生效。如果你想要"改掉外层变量"，那必须用 `mut`。

### const 与 static

除了 `let`，还有两种声明常量的方式：

```rust
// 编译期常量，可以定义在函数内或模块级，每次使用处会被直接内联替换
const MAX_RETRY: u32 = 5;
const TIMEOUT_MS: u64 = 30 * 1000;   // 编译期算好

// 静态变量，有固定内存地址，程序整个生命周期都存在
static VERSION: &str = "1.0.0";

fn main() {
    println!("{MAX_RETRY} {TIMEOUT_MS} {VERSION}");
}
```

三个要点：

- `const` 必须写类型，且值必须在编译期能算出来。不能写 `const X = compute();`（除非是 `const fn`）。
- `const` 没有固定地址，编译器可以在每个使用处复制一份，所以不占静态内存。`static` 有唯一地址。
- `static` 默认不可变；可变的 `static mut` 在 2024 edition 里被视为不安全操作，几乎总该用 `OnceLock`、`AtomicUsize` 这类同步工具代替。

## 2.2 标量类型

Rust 的标量有四类：整数、浮点数、布尔、字符。

### 整数

| 类型 | 位数 | 取值范围 |
| --- | --- | --- |
| `i8` / `u8` | 8 | −128..=127 / 0..=255 |
| `i16` / `u16` | 16 | −32768..=32767 / 0..=65535 |
| `i32` / `u32` | 32 | ±约 21 亿 / 0..=约 42 亿 |
| `i64` / `u64` | 64 | ±约 9.2×10^18 |
| `i128` / `u128` | 128 | 很大 |
| `isize` / `usize` | 指针宽度 | 64 位平台上等于 i64/u64 |

`usize` 是"用于索引和长度的整数"，`vec.len()` 返回它，数组下标也要求它。**不要为了省内存而到处都是 u8**，类型转换的噪音往往比内存收益更贵；默认用 `i32`（Rust 的默认整数类型），长度和索引用 `usize`。

字面量写法：

```rust
fn main() {
    let dec = 98_222;          // 十进制，下划线随便加，只为好看
    let hex = 0xff;            // 十六进制
    let oct = 0o77;            // 八进制
    let bin = 0b1111_0000;     // 二进制
    let byte = b'A';           // 字节字面量，类型是 u8
    let big: u64 = 1_000_000_000_000;
    println!("{dec} {hex} {oct} {bin} {byte} {big}");
}
```

整数字面量默认是 `i32`。如果放不下，编译器会推断成更大的类型；一旦出现歧义就会要求你标注。

### 整数溢出：四种策略

这是 Rust 新手最容易踩的语义坑，因为**同一段代码在 debug 和 release 下行为不同**。

```rust
fn main() {
    let x = std::hint::black_box(u8::MAX); // 让加法在运行时求值
    let y = x + 1;   // debug 下 panic，release 下得到 0
    println!("{y}");
}
```

| 策略 | 方法 | 255 + 1 的结果 | 什么时候用 |
| --- | --- | --- | --- |
| 检查 | `checked_add` | `None` | 溢出是**业务错误**，要显式处理 |
| 回绕 | `wrapping_add` | `0` | 哈希、校验和、加密算法 |
| 饱和 | `saturating_add` | `255` | 计数、进度、音量这类"到顶就停" |
| 报告 | `overflowing_add` | `(0, true)` | 需要同时拿到结果和溢出标志 |

```rust
fn main() {
    let x: u8 = 255;
    println!("{:?}", x.checked_add(1));     // None
    println!("{}", x.wrapping_add(1));      // 0
    println!("{}", x.saturating_add(1));    // 255
    println!("{:?}", x.overflowing_add(1)); // (0, true)

    // 典型用法：检查后走不同分支
    match x.checked_add(1) {
        Some(v) => println!("结果是 {v}"),
        None => println!("溢出了，拒绝计算"),
    }
}
```

为什么默认不 panic 到 release？因为检查每个算术操作会拖慢运行速度，而 release 的目标是性能。Rust 的选择是：**开发时帮你抓 bug（debug panic），交付时按硬件语义走（release 回绕）**。

不想接受这个差异，就在 `Cargo.toml` 里固定：

```toml
[profile.release]
overflow-checks = true   # 代价：部分算术变慢
```

**判断标准**：如果溢出意味着"用户输入或程序逻辑有问题"，用 `checked_*`；如果溢出是算法本身的一部分（哈希），用 `wrapping_*`；如果只是显示层的上限，用 `saturating_*`。

### 浮点数

`f32` 和 `f64`（默认）。用 IEEE-754 表示，因此有三个必须记住的性质：

```rust
fn main() {
    let a = 0.1_f64 + 0.2_f64;
    println!("{a}");                 // 0.30000000000000004
    println!("{}", a == 0.3);        // false

    let nan = 0.0_f64 / 0.0;
    println!("{nan}");               // NaN
    println!("{}", nan == nan);      // false！NaN 不等于自己

    // 比较浮点数要用误差范围，不能用 ==
    let ok = (a - 0.3_f64).abs() < 1e-10;
    println!("{ok}");
}
```

- `f64` 没有实现 `Eq` 和 `Ord`（只实现 `PartialEq`/`PartialOrd`），因为 NaN 破坏了全序。所以浮点数不能做 `HashMap` 的键，也不能直接 `sort`。
- 需要给浮点数排序时用 `sort_by(|a, b| a.partial_cmp(b).unwrap())`，但要先确保没有 NaN；更稳妥的是 `f64::total_cmp`。
- 涉及钱和精确小数，不要用浮点，用定点数（整数存"分"）或 `rust_decimal`。

### char

`char` 是**一个 Unicode 标量值**，占 4 字节：

```rust
fn main() {
    let c = '中';
    let e = '😀';
    println!("{c} {e}");
    println!("{} {}", c.len_utf8(), e.len_utf8()); // 3 4 —— UTF-8 编码后的字节数
    println!("{}", std::mem::size_of::<char>());   // 4
}
```

字符用**单引号**，字符串用双引号，这是两个不同的类型：

```rust
fn main() {
    let c: char = 'a';      // 4 字节
    let s: &str = "a";      // 指向 1 字节的 UTF-8 数据
    println!("{c} {s}");
}
```

"😀 是一个 char，但它的 UTF-8 编码是 4 字节；'中' 是一个 char，UTF-8 编码 3 字节"——这句话解释了 Rust 里为什么**不能用下标取字符串里的第 n 个字符**（第 7 章讲字符串时展开）。

## 2.3 深入：类型大小与内存布局

`size_of` 是最能暴露语言底层设计的工具：

```rust
use std::mem::size_of;

fn main() {
    println!("u8   {}", size_of::<u8>());      // 1
    println!("i32  {}", size_of::<i32>());     // 4
    println!("usize {}", size_of::<usize>());  // 8（64 位平台）
    println!("char {}", size_of::<char>());    // 4
    println!("bool {}", size_of::<bool>());    // 1
    println!("(u8, u32) {}", size_of::<(u8, u32)>());  // 8，不是 5
    println!("&u8  {}", size_of::<&u8>());     // 8，指针
    println!("String {}", size_of::<String>()); // 24
    println!("&str  {}", size_of::<&str>());    // 16，指针 + 长度
}
```

三个值得停下来想的数字：

1. **`(u8, u32)` 占 8 字节而不是 5**。因为对齐（alignment）要求 `u32` 的地址是 4 的倍数，编译器在 `u8` 后插了 3 字节填充。字段顺序会影响结构体大小，第 8 章会讲怎么用手工排列省内存。
2. **`&str` 是 16 字节**：8 字节指针 + 8 字节长度。它叫"胖指针"。数组切片 `&[T]` 同理。对比 C 的 `char*`（只有指针，长度靠 `\0` 或额外参数），切片引用携带运行时长度，代价就是两倍体积。
3. **`String` 是 24 字节**：指针 + 长度 + 容量，这三个字段在 `String` 值本身，堆缓冲区存 UTF-8 字节。

还有个更妙的优化——**空指针优化（niche optimization）**：

```rust
use std::mem::size_of;
fn main() {
    println!("{}", size_of::<Option<&u8>>()); // 8，和 &u8 一样！
    println!("{}", size_of::<Option<Box<u8>>>()); // 8
    println!("{}", size_of::<Option<u8>>());  // 2，多了一个字节做标签
}
```

引用永远不为空，所以 `Option<&T>` 直接用"空指针"这个不可能出现的值表示 `None`，不额外占空间。这解释了为什么 Rust 里 `Option<&T>` 可以零成本替代 C 的可空指针。

## 2.4 as 转换 vs From/Into

`as` 是**数值转换**，规则是"尽力而为"，可能悄悄丢数据：

```rust
fn main() {
    let big: i32 = 300;
    let small = big as u8;          // 44 —— 300 截断成低 8 位（300 % 256）
    println!("{small}");

    let neg: i32 = -1;
    let u = neg as u32;              // 4294967295 —— 按位重新解释
    println!("{u}");

    let pi = 3.99_f64;
    println!("{}", pi as i32);       // 3，截断不是四舍五入

    let huge = 1e30_f64;
    println!("{}", huge as i32);     // 2147483647，饱和到上限（Rust 1.45+）
}
```

`as` 的规则总结：整数→整数按位截断/扩展；浮点→整数向零截断，超范围饱和；整数→浮点可能丢精度。**`as` 不会失败、不会 panic，也就不会保护你。**

`From`/`Into` 是**无损转换**，只在不会丢信息的方向上实现：

```rust
fn main() {
    let a: i32 = 100;
    let b: i64 = i64::from(a);      // 安全：i32 一定能放进 i64
    let c: i64 = a.into();          // 同上，反过来写
    println!("{b} {c}");

    // let d: u8 = u8::from(a);     // 编译错误：u8 可能装不下 i32
}
```

需要可能失败的转换，用 `TryFrom`：

```rust
use std::convert::TryFrom;

fn main() {
    match u8::try_from(300_i32) {
        Ok(v) => println!("转换成功 {v}"),
        Err(e) => println!("转换失败：{e}"),
    }
    // 或者用 ? 在返回 Result 的函数里传播
}
```

| | `as` | `From`/`Into` | `TryFrom`/`TryInto` |
| --- | --- | --- | --- |
| 会失败吗 | 不会，静默截断 | 不会，且保证无损 | 会，返回 `Result` |
| 能用于类型推断吗 | 否 | 是，配合泛型很自然 | 是 |
| 什么时候用 | 位运算、底层序列化 | 明确的放大转换 | 用户输入、外部数据 |

**实践建议**：业务代码里优先 `From/Into`/`TryFrom`，把 `as` 留给确实需要按位语义的场景。如果必须用 `as` 做收窄，前面加一个范围检查。

## 2.5 类型推断的边界

Rust 的类型推断是局部且单向的，比 Haskell 保守得多。理解这一点能省下大量"为什么这里非要我写字"的困惑。

```rust
fn main() {
    let x = 5;              // 推断为 i32（默认整数类型）
    let y = 2.0;            // 推断为 f64
    let v = vec![1, 2, 3];  // Vec<i32>
    println!("{x} {y} {:?}", v);

    // 有歧义时必须标注
    let n: u32 = "42".parse().unwrap();
    println!("{n}");
}
```

```rust,noplayground
fn main() {
    let n = "42".parse().unwrap();  // 猜不出目标类型
    println!("{n}");
}
```

```text
error[E0284]: type annotations needed
 --> src/main.rs:2:22
  |
2 |     let n = "42".parse().unwrap();
  |                      ^^^^^ cannot infer type of the type parameter `F` declared on the enum `Result`
  |
  = note: cannot satisfy `<_ as FromStr>`
help: consider specifying the generic argument
  |
2 |     let n = "42".parse::<f32>().unwrap();  // 或写 let n: f64 = ...
```

`parse` 是泛型的，返回 `Result<F, F::Err>`，而 `F` 可能有一百多种实现（`i32`、`f64`、`IpAddr`……）。**如果上下文里没有足够线索，编译器就推断不出 `F` 是哪个类型**。

实际操作中有两种写法，都明确告诉编译器 `F` 是什么：

```rust
fn main() {
    // 写法一：turbofish 直接指定
    let n = "42".parse::<i64>().unwrap();
    // 写法二：给变量标注类型
    let m: i64 = "42".parse().unwrap();
    let total: i64 = n + m;
    println!("{total}");
}
```

这种"从后面倒推"的能力是局部的：**函数签名不受函数体影响**。所以写函数时参数和返回值类型必须写全：

```rust
fn double(x: i32) -> i32 { x * 2 }   // 参数与返回值类型必须写

fn main() {
    println!("{}", double(21));
}
```

## 典型错误案例

### 案例 1：改了不可变变量

```rust,editable
fn main() {
    let count = 0;
    count += 1;
    println!("{count}");
}
```

```text
error[E0384]: cannot assign twice to immutable variable `count`
 --> src/main.rs:3:5
  |
2 |     let count = 0;
  |         ----- first assignment to `count`
  |         |
  |         help: consider making this binding mutable: `mut count`
3 |     count += 1;
  |     ^^^^^^^^^^ cannot assign twice to immutable variable
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`let` 默认不可变，`+=` 是赋值操作。
**修复**：`let mut count = 0;`。如果你其实想要"基于旧值算新值且不关心旧名字"，用遮蔽 `let count = count + 1;` 也行——但循环里这样写没有意义。


修复后的完整代码：

```rust,editable
fn main() {
    let mut count = 0;
    count += 1;
    println!("{count}");
}
```

</details>

### 案例 2：类型不匹配（整数 vs 浮点）

```rust,editable
fn main() {
    let x: i32 = 3.14;
    println!("{x}");
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:2:18
  |
2 |     let x: i32 = 3.14;
  |            ---   ^^^^ expected `i32`, found floating-point number
  |            |
  |            expected due to this
  |
help: if you intended to convert the value, use `as`
  |
2 |     let x: i32 = 3.14 as i32;
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：Rust 不做隐式数值转换，连 `3.14` 到 `i32` 这种"显然"的也不行。
**修复**：显式转换。注意 `as i32` 是截断（得 3），要四舍五入得写 `3.14_f64.round() as i32`。


修复后的完整代码：

```rust,editable
fn main() {
    let x: i32 = 3.14_f64.round() as i32;
    println!("{x}");
}
```

</details>

### 案例 3：整数溢出（debug 下 panic）

```rust,editable
fn main() {
    let mut total: u8 = 250;
    for _ in 0..10 {
        total += 10;
    }
    println!("{total}");
}
```

```text
thread 'main' panicked at src/main.rs:4:15:
attempt to add with overflow
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`u8` 最大 255，加到 260 溢出。debug build 会 panic。
**修复**：按语义选一个——`total = total.saturating_add(10)`（封顶 255）、换成 `u32`（放宽范围）、或用 `checked_add` 显式处理错误。**千万不要只在 release 测**，默认 release 配置下第一次加法回绕成 4，循环十次最终得到 94。


修复后的完整代码：

```rust,editable
fn main() {
    let mut total: u8 = 250;
    for _ in 0..10 {
        total = total.saturating_add(10);
    }
    println!("{total}");
}
```

</details>

### 案例 4：用 `partial_cmp(...).unwrap()` 排序时遇到 NaN

```rust,editable
fn main() {
    let mut list = vec![3.0_f64, f64::NAN, 1.0];
    list.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("{list:?}");
}
```

```text
thread 'main' panicked at src/main.rs:3:38:
called `Option::unwrap()` on a `None` value
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`partial_cmp` 比较涉及 NaN 的两个值时返回 `None`，`unwrap` panic。
**修复**：用 `f64::total_cmp`（把所有浮点值当成可比较的位模式）或者提前过滤：`list.retain(|x| !x.is_nan())`。


修复后的完整代码：

```rust,editable
fn main() {
    let mut list = vec![3.0_f64, f64::NAN, 1.0];
    list.sort_by(|a, b| a.total_cmp(b));
    println!("{list:?}");
}
```

</details>

### 案例 5：as 静默截断

```rust,editable
fn main() {
    let user_input: i32 = 300;
    let level = user_input as u8;   // 44，编译通过，逻辑全错
    println!("等级 {level}");
}
```

```text
（无编译错误 —— 这正是最危险的地方）
等级 44
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`as` 按位截断，不检查范围。
**修复**：用 `u8::try_from(user_input)` 或先判断 `if (0..=255).contains(&user_input)`。这类 bug 编译器帮不了你，只能靠"凡是收窄转换都检查"的纪律。


修复后的完整代码：

```rust,editable
fn main() {
    let user_input: i32 = 300;
    match u8::try_from(user_input) {
        Ok(level) => println!("等级 {level}"),
        Err(_) => println!("等级超范围：{user_input}（应在 0..=255）"),
    }
}
```

</details>

## 动手练习

1. 用四种溢出策略分别实现一个"余额扣款"函数，说明哪种最符合业务语义。
2. 打印以下类型的大小，并解释每个数字：`(u8, u8)`、`(u8, u16)`、`(u16, u8)`、`Option<u32>`、`Result<u8, u8>`。
3. 写一个函数接收 `&str`，把它解析成 `u8`（超过 255 返回错误而不是截断），用 `TryFrom` 或范围检查实现。
4. 用遮蔽把 `"1,2,3"` 依次变成 `&str`、`Vec<&str>`、`Vec<i32>`、所有元素之和，每个中间步骤用不同变量名和同名各写一遍，比较可读性。
5. 找出下面代码的 bug 并说明为什么编译器不报错：
   ```rust,noplayground
   fn main() {
       let x: i8 = 100;
       let y: i8 = 100;
       let z = x + y;   // debug 构建下运行到这里 panic
       println!("{z}");
   }
   ```

## 小结

- `let` 是绑定，默认不可变；`mut` 允许赋值；遮蔽创建新变量并允许换类型。
- `const` 是编译期常量（会内联），`static` 有固定地址。
- 整数溢出的四种策略对应四种业务语义；debug panic / release 回绕是刻意的性能取舍，可用 `overflow-checks` 固定。
- 浮点数有 NaN、精度和 `Eq` 缺失三个坑；比较用误差，排序用 `total_cmp`。
- `char` 是 4 字节的 Unicode 标量；本章的 64 位目标上，`&str` 占 16 字节，`String` 占 24 字节。
- 对齐让 `(u8, u32)` 占 8 字节；空指针优化让 `Option<&T>` 零额外开销。
- `as` 静默截断，`From/Into` 保证无损，`TryFrom` 返回 `Result`；收窄转换优先后者。
- 类型推断是局部的：函数签名必须写全，`parse::<T>()` 这类泛型调用常常需要标注或后续用法给线索。
