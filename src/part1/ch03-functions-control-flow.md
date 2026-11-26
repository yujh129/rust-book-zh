# 第 3 章 函数与控制流

> **本章目标**
> - 写出参数、返回值齐全的函数，并说清签名每一部分的含义
> - 分清表达式与语句，解释块的值从哪来、尾表达式为什么不能带分号
> - 用 if 表达式、loop 的 break 值、带标签的 break 组织控制流
> - 认识发散函数与 never type，知道它和单元类型 () 的区别
> - 从栈帧与调用约定的角度解释一次调用的开销，判断 #[inline] 什么时候有用
> - 躲开 5 个函数与控制流的经典编译错误

前两章你一直在 main 里写代码：变量怎么声明、类型怎么标注。但真正的程序不会是一条从头走到尾的直线——同一段逻辑要用很多次，某些步骤只在条件下执行，另一些要重复到满足条件为止。函数、条件、循环就是应付这三种需求的工具，任何语言都有。

Rust 在这件事上做了一个不太一样的决定：**几乎一切都是表达式**。C、Java、Python 把语言切成"语句"和"表达式"两半，if 和 for 是语句，不能出现在赋值号右边；Rust 里 if、match、块、循环都是表达式，都能产生值。这个决定来自 ML 家族（OCaml、Haskell），它让"把结果算出来交给下一步"成为默认写法，也让一堆临时变量消失。

代价是你得重新校准直觉。在 C 里写 `if (x) return 1;` 没人觉得奇怪，在 Rust 里函数体最后那句不带分号的表达式才是返回值，多一个分号就变成 ()。这一章先把这套规则讲透，再往下看编译器为一次调用究竟做了什么。

## 3.1 函数：签名即契约

最短的完整例子：

```rust
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn main() {
    println!("{}", gcd(1071, 462));
}
```

签名由四段组成：`fn` 关键字、函数名、参数列表（每个参数写成 `名字: 类型`）、返回类型（`-> T`）。函数体是一个块。

- 参数类型不能省。写 `fn f(a, b)` 直接报错，没有隐式推断。签名是对调用方的承诺，编译器不会替你猜；C++ 模板和 Python 鸭子类型是另一种取舍，代价转移到调用点和运行期。
- 返回类型可以省，但只在返回 () 的时候。`fn log(msg: &str)` 与 `fn log(msg: &str) -> ()` 完全等价。
- 定义顺序无关。main 写在 gcd 之前或之后都行，Rust 没有 C 那种"先声明后使用"的限制，也没有头文件。
- 命名用 snake_case，编译器会用 non_snake_case 警告提醒你。

### 没有重载，也没有默认参数

C++ 和 Java 允许 `area(int)` 与 `area(double)` 同名，Rust 不允许：一个名字只绑定一个函数（trait 方法另说，第 12 章）。默认参数同样没有。替代做法是把函数名写清楚：

```rust
fn area_rect(w: f64, h: f64) -> f64 {
    w * h
}

fn area_circle(r: f64) -> f64 {
    std::f64::consts::PI * r * r
}

fn main() {
    println!("{:.2} {:.2}", area_rect(2.0, 3.0), area_circle(1.0));
}
```

看起来是退步，实际是把"调用点长什么样"固定下来：读代码时不必在十几个重载里找匹配的那个。参数确实可选时，标准库的惯例是用 Option 参数，或者干脆提供两个函数（`Vec::get` 返回 Option，索引语法 `v[i]` 直接 panic，两种语义分开）。

## 3.2 表达式与语句：块的值

判据只有一条：**能不能产生一个值**。

| 形式 | 类别 | 值 |
| --- | --- | --- |
| let x = 1; | 语句 | 无 |
| x + 1 | 表达式 | 计算结果 |
| if c { a } else { b } | 表达式 | 被选中分支的值 |
| 块 { } | 表达式 | 块尾表达式的值 |
| x = 1 | 表达式 | 单元值 () |
| println!("hi") | 表达式 | 单元值 () |

块本身是表达式，它的值是最后一条**不带分号**的表达式，叫尾表达式（tail expression）。尾表达式一旦加上分号，就从"值"降级成"语句"，块的值随之变成 ()：

```rust
fn main() {
    let x = {
        let a = 2;
        let b = 3;
        a * b
    };
    println!("{x}");

    let y = {
        let a = 2;
        a * 3;
    };
    println!("{y:?}");
}
```

输出是 `6` 和 `()`。第二个块里 `a * 3;` 的乘积被丢掉，块的值是 ()，y 的类型也就是 ()。rustc 还会顺带给一条 `unused arithmetic operation that must be used` 警告——它在提醒你"算完就扔"通常是笔误。

函数体就是一个块，所以"函数返回什么"和"块的值是什么"是同一条规则。本章后面所有报错都从这条规则长出来。

赋值也是表达式，值是 ()。于是 C 的链式赋值 `a = b = c` 不成立（`b = c` 是 ()），`let x = (let y = 2);` 也不成立——let 是语句，没有值可给。

## 3.3 提前返回与发散函数

### return 与卫语句

return 立刻结束函数，可以带值（`return 1;`）也可以不带（`return;`，等价于 `return ();`）。函数体最后一句再 return 是多余的，习惯上省略。

真正有用的是**卫语句**（guard clause）：把不合法的情况在函数开头赶走，主逻辑留在最外层，不用套一层 if。

```rust
fn parse_positive(s: &str) -> Option<u32> {
    let n: i64 = match s.trim().parse() {
        Ok(n) => n,
        Err(_) => return None,
    };
    if n <= 0 {
        return None;
    }
    Some(n as u32)
}

fn main() {
    for s in ["42", "-7", "abc", " 9 "] {
        println!("{s:?} -> {:?}", parse_positive(s));
    }
}
```

注意 match 的 Err 分支：`return None` 的类型是 !（下一小节），可以出现在任何需要值的位置，所以整个 match 依然能给 n 一个 i64。卫语句能写得这么平，靠的就是这一点。

### 发散函数与 never type

返回类型写成 ! 的函数叫发散函数（diverging function）：它不返回，调用点之后的代码永远不会执行。! 读作 never type。

标准库里产生 ! 的入口有这些：

- panic! 宏
- std::process::exit(code) 与 std::process::abort()
- unreachable!()、todo!()、unimplemented!()
- 没有 break 的 loop { }，它的类型就是 !

! 是类型系统里的底部类型（bottom type），可以强制转换成任何类型。所以 match 的某个分支 panic! 时，不需要凑一个同类型的值：

```rust
fn first_or_die(v: &[i32]) -> i32 {
    match v.first() {
        Some(x) => *x,
        None => panic!("空切片"),
    }
}

fn main() {
    println!("{}", first_or_die(&[7, 8]));
}
```

None 分支的类型是 !，被转换成 i32，整个 match 的类型由 Some 分支决定。把 panic! 换成 `0` 也能编译，但那样就吞掉了错误。

! 与 () 的区别值得记牢：() 是"只有一个值的真实类型"，函数正常返回了，只是没带东西；! 表示"没有返回值，因为根本回不来"。`let x: () = ();` 合法，而 `fn f() -> !` 的函数体不能以普通表达式结尾。

main 的返回类型是 ()，你也可以用 std::process::exit 提前结束，但要清楚它**跳过所有析构**（Drop）：文件缓冲可能没刷、临时文件可能没删。return 会老老实实走完析构。需要"立刻退出"的时候才用 exit。

## 3.4 if 是表达式

条件必须是 bool。C 的 `if (x)`（非零为真）和 Python 的真值判断在这里都不成立：`x` 是 i32 时写 `if x`，rustc 报 E0308，expected bool, found integer。这条规则顺手消灭了一整类 bug——C 里把 `==` 敲成 `=` 的 `if (x = 1)` 恒为真，在 Rust 里是明确的类型错误。

分支的值就是 if 的值，所以可以直接赋给变量：

```rust
fn main() {
    let n = 9;
    let parity = if n % 2 == 0 { "偶" } else { "奇" };
    println!("{n} 是{parity}数");

    let label = if n > 100 {
        "大"
    } else if n > 10 {
        "中"
    } else {
        "小"
    };
    println!("{label}");
}
```

规则只有两条：

1. 所有分支的类型必须一致，rustc 会拿它们互相比较（E0308）。
2. 当表达式用时必须有 else。没有 else 时条件不成立就没有值可给，类型只能退化成 ()。

当语句用就不需要 else，此时整条 if 的值被丢弃：

```rust
fn main() {
    let n = 100;
    if n > 50 {
        println!("大于 50");
    }
}
```

Rust 没有三目运算符 `?:`，if 表达式就是它。区别在于分支是完整的块：能写多行、能加注释、能放 let，嵌套三层也不会变成一行密码。

### let 与 if 的类型一致性

if 一旦出现在等号右边，两个分支就必须是同一个类型。一半分支有值、另一半只是 println! 的写法编不过：else 分支的值是 ()，和 `&str` 冲突（完整报错见案例 1）。两种修法：把两个分支改成同类型，或者承认这段代码只是"做事"不产生值，丢掉 let，把这个分支改成一条语句形式的 if。

### if let：先看一眼

if let 是"只关心一种模式"的 if，日常主要用在 Option 和 Result 上：

```rust
fn main() {
    let v = vec![10, 20, 30];
    let hit = v.iter().position(|x| *x == 20);
    if let Some(i) = hit {
        println!("20 在下标 {i}");
    }
}
```

`if let Some(i) = hit` 读作"如果 hit 是 Some，就把里面的值绑到 i"。它等价于只有一个分支的 match：匹配失败时什么都不做，没有 else 也不报错。第 4 章讲 match 的穷尽性检查时会把这块补完——到那时你会看到 if let 省掉的正是"另一个分支必须写"这条义务。

## 3.5 三种循环

loop、while、for 的分工很清楚：不知道要转几次用 loop，有明确条件用 while，有集合用 for。

```rust
fn main() {
    let mut n = 1;
    let first_big = loop {
        n *= 2;
        if n > 100 {
            break n;
        }
    };
    println!("{first_big}");

    let mut countdown = 3;
    while countdown > 0 {
        println!("{countdown}");
        countdown -= 1;
    }

    for x in [10, 20, 30] {
        println!("{x}");
    }
}
```

loop 是唯一能带值退出的循环：`break expr` 的值成为整个 loop 表达式的值。这也是它相对于 `while true` 的实质优势——不用在循环外声明一个可变的 Option，更不用担心"某条路径忘了赋值"。编译器会检查所有 break 的值类型是否一致。

while 和 for 的值恒为 ()，`let x = while countdown > 0 { countdown -= 1; };` 拿不到任何东西。

continue 跳过本次迭代。for 和 while 里的 continue 会先推进到下一项或重新判断条件，loop 里则是直接回到开头。

### 标签：一次跳出多层

嵌套循环里 break 只跳出最内层。要一次跳出两层，给外层循环加标签，标签以单引号开头，写在循环关键字前面：

```rust
fn main() {
    let grid = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
    let mut found = None;

    'rows: for (r, row) in grid.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            if v == 6 {
                found = Some((r, c));
                break 'rows;
            }
        }
    }

    println!("{found:?}");
}
```

`continue 'rows` 同理，表示跳过外层这一轮。Rust 1.65 起，带标签的 loop 也能带值 break：`break 'outer i * 10` 把值送到标签处，外层循环因此得到一个 i32。

别把标签和生命周期标注搞混：两者都写成 `'a` 的样子，但一个是控制流名字，一个是借用区间，编译器按出现位置区分（第 16 章讲生命周期）。

`while let` 是同一族的写法：`while let Some(x) = stack.pop()` 表示"取到空就停"，第 4 章细讲，先记住这个形状。

## 3.6 迭代 range 的几种写法

`a..b` 是左闭右开区间（不含 b），`a..=b` 是闭区间。两者都实现了 IntoIterator，能直接放进 for，也能 collect 成 Vec 看结果：

```rust
fn main() {
    let a: Vec<i32> = (0..5).collect();
    let b: Vec<i32> = (0..=5).collect();
    let c: Vec<i32> = (0..5).rev().collect();
    let d: Vec<i32> = (0..10).step_by(3).collect();
    println!("{a:?} {b:?} {c:?} {d:?}");
    assert_eq!((0..8).step_by(3).rev().collect::<Vec<_>>(), [6, 3, 0]);

    let sum: i32 = (1..=100).filter(|n| n % 7 == 0).sum();
    println!("{sum}");
}
```

几个容易踩的点：

- `rev` 要求 `DoubleEndedIterator`。底层迭代器还实现 `ExactSizeIterator` 时，`StepBy` 也可以反向遍历。试试 `0..8`：先 `step_by(3)` 再 `rev()` 得到 `6, 3, 0`，先 `rev()` 再 `step_by(3)` 得到 `7, 4, 1`。
- range 的元素类型由端点决定。`for i in 0..v.len()` 里端点是 usize，i 就是 usize，不用转换；写成 `0..v.len() as i32` 再去索引反而要转回来。
- range 是惰性的：`0..1_000_000` 只保存端点，也不会预先展开成列表。链式调用（filter、map、sum）每取一个元素走一遍流水线，中间不产生临时 Vec。

同一个求和既可以写 `for i in 0..v.len() { sum += v[i]; }`，也可以写 `v.iter().sum()`。前者每次下标都带一次边界检查，越界就 panic；后者把长度和游标绑在一起，编译器常常能把检查消掉。默认选迭代器，需要下标参与计算（同时读相邻元素、或往另一个数组里写）时才用 enumerate() 或显式下标。

## 3.7 深入：一次调用在机器上发生什么

### 栈帧与调用约定

以 x86-64 上一次未内联的普通调用为例，CPU 先把返回地址压栈，被调函数再为局部变量和要保存的寄存器预留一块空间，这块空间就是栈帧（stack frame）。函数返回时整块弹掉——这就是局部变量不能"活过"函数的原因，也是第 6 章悬垂引用在机器层面的来源。

参数怎么传由调用约定（calling convention）决定：

```text
System V（Linux/macOS）x86-64
  参数 1  参数 2  参数 3  参数 4  参数 5  参数 6   更多
  rdi     rsi     rdx     rcx     r8      r9       压栈
Windows x64
  rcx     rdx     r8      r9      （第 5 个起压栈）
整型返回值：rax
```

只有 `extern "C"` 才承诺 C 的 ABI。Rust 默认的 `extern "Rust"` ABI 没有稳定承诺，传参方式、字段顺序都可能随版本变化，所以 FFI（外部函数接口）必须显式写 `extern "C"`，不能指望"反正都在 x86-64 上"。

结构体按值传递时，小到能塞进寄存器就直接塞；超过两个机器字通常由调用方分配空间、传一个隐藏指针（sret），函数往那块内存里写。这解释了为什么"返回一个大结构体"未必像看起来那么贵，也解释了为什么结构体大小会实实在在影响性能。

### 内联与 #[inline]

`fn inc(x: u64) -> u64 { x + 1 }` 在 `-O` 构建里通常不会留下一次真正的调用：LLVM 把函数体复制到调用点，一条加法就完事。这个决定由优化器做，不用你操心。

`#[inline]` 是给优化器的**建议**，不是命令。它真正的用途是跨 crate：每个 crate 独立编译，A 里的小函数如果没标 `#[inline]`，B 看不到它的函数体，也就无从内联。标准库的 Iterator 方法几乎全带 `#[inline]`，正是为了让 `v.iter().map(f).sum()` 在你这边被完整展开成循环。

代价也要说清：`#[inline(always)]` 会让每个调用点复制一份函数体，代码体积膨胀、指令缓存压力上升，大函数用了通常更慢。经验法则是只在"函数很小且确实在热路径上"时标，先测量再标。

调试构建里还有另一类开销。同样是 `x + 1`，debug 版会多一条进位判断，溢出就 panic；`-O` 构建默认关掉溢出检查（overflow-checks = false），溢出变成回绕。同一份代码两种行为，这是"release 里结果不对劲"的常见来源之一。

### 普通递归调用没有尾调用优化保证

函数式语言（Scheme、OCaml）保证尾调用优化（tail call optimization）：如果函数的最后一步是调用另一个函数，就复用当前栈帧，递归一百万层也只占一个帧。

**Rust 不保证这件事。** 普通尾递归可能一层层长栈，够深就 `thread 'main' has overflowed its stack`。原因在于 Rust 有析构：返回前要先 drop 栈上的值，还要维护借用、调试信息和"当前在第几层"这些状态，让"直接跳过去"不总是合法。

实用结论：递归深度和输入规模挂钩时，改写成循环。主线程栈的大小由平台和启动环境决定；`std::thread::Builder::stack_size` 设置新建线程的栈大小，但调栈往往是在掩盖"本该写成循环"这件事。

## 典型错误案例

### 案例 1：if 两个分支类型不一致

```rust,editable
fn main() {
    let n = 7;
    let doubled = if n % 2 == 0 {
        n * 2
    } else {
        println!("奇数，跳过");
    };
    println!("{doubled}");
}
```

```text
error[E0308]: `if` and `else` have incompatible types
 --> src/main.rs:6:9
  |
3 |       let doubled = if n % 2 == 0 {
  |  ___________________-
4 | |         n * 2
  | |         ----- expected because of this
5 | |     } else {
6 | |         println!("奇数，跳过");
  | |         ^^^^^^^^^^^^^^^^^^^^^^ expected integer, found `()`
7 | |     };
  | |_____- `if` and `else` have incompatible types
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：if 是表达式，两个分支必须给出同一个类型的值。then 分支的尾表达式是 `n * 2`（i32），else 分支的尾表达式是一次 println! 调用（()），编译器拿它们互相比较，只能报类型不符。注意报错用的是"expected integer, found ()"而不是"分支不一致"，因为它先固定了第一个分支的类型。

**修复**：先想清楚这段是"算一个值"还是"做一件事"。要算值，两个分支都得给 i32，打印挪到 if 外面：

```rust,editable
fn main() {
    let n = 7;
    let doubled = if n % 2 == 0 { n * 2 } else { n };
    println!("{doubled}");
}
```

</details>

### 案例 2：尾表达式多了分号

```rust,editable
fn double(x: i32) -> i32 {
    x * 2;
}

fn main() {
    println!("{}", double(3));
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:1:22
  |
1 | fn double(x: i32) -> i32 {
  |    ------            ^^^ expected `i32`, found `()`
  |    |
  |    implicitly returns `()` as its body has no tail or `return` expression
2 |     x * 2;
  |          - help: remove this semicolon to return this value
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：分号把表达式变成语句，函数体的值就退化成 ()。这是全章最高频的一个错误，几乎每个从 C/Java 转过来的人都踩过——在那些语言里 `return` 是必需的，末尾加不加分号无所谓；Rust 把"最后一句"本身当成返回值。

**修复**：删掉分号。rustc 的 help 行已经直接说了。如果函数体不止一行，也可以写 `return x * 2;`，语义相同，但多行分支里用尾表达式更主流。

```rust,editable
fn double(x: i32) -> i32 {
    x * 2
}

fn main() {
    println!("{}", double(3));
}
```

</details>

### 案例 3：调用了一个不存在的函数

```rust,editable
fn main() {
    let total = add(3, 4);
    println!("{total}");
}
```

```text
error[E0425]: cannot find function `add` in this scope
 --> src/main.rs:2:17
  |
2 |     let total = add(3, 4);
  |                 ^^^ not found in this scope
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：名字没绑到任何函数上。常见来源有三个：忘了定义；定义在别的模块里但没 use；把方法当自由函数调用（`3.add(4)` 是 trait 方法，不能写成 `add(3, 4)`）。注意 Rust 不要求函数定义在使用之前，所以"顺序问题"不在其中。

**修复**：补上定义，或把路径写全。用了标准库函数忘了 use 时，编译器通常会给"consider importing"的建议。

```rust,editable
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let total = add(3, 4);
    println!("{total}");
}
```

</details>

### 案例 4：参数个数不对

```rust,editable
fn area(w: u32, h: u32) -> u32 {
    w * h
}

fn main() {
    println!("{}", area(3));
}
```

```text
error[E0061]: this function takes 2 arguments but 1 argument was supplied
 --> src/main.rs:6:20
  |
6 |     println!("{}", area(3));
  |                    ^^^^--- argument #2 of type `u32` is missing
  |
note: function defined here
 --> src/main.rs:1:4
  |
1 | fn area(w: u32, h: u32) -> u32 {
  |    ^^^^         ------
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：Rust 没有默认参数，也没有重载，所以参数个数必须精确对上。note 行指回定义处，^- 标记直接标出缺的是第几个参数、什么类型。

**修复**：把参数补全。确实需要"可省略"的参数时，用 Option 显式表达，或者再写一个包装函数（`fn square(s: u32) -> u32 { area(s, s) }`）：

```rust,editable
fn area(w: u32, h: u32) -> u32 {
    w * h
}

fn main() {
    println!("{}", area(3, 4));
}
```

</details>

### 案例 5：丢弃 must_use 的返回值，后面还写了不可达代码

```rust,editable
#![deny(unused_must_use, unreachable_code)]

#[must_use]
fn save(data: &[u8]) -> usize {
    data.len()
}

fn main() {
    let total = save(b"hello");
    if total == 0 {
        return;
    }
    save(b"world");
    return;
    println!("总计 {total} 字节");
}
```

```text
error: unreachable statement
  --> src/main.rs:15:5
   |
14 |     return;
   |     ------ any code following this expression is unreachable
15 |     println!("总计 {total} 字节");
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ unreachable statement
   |
note: the lint level is defined here
  --> src/main.rs:1:26
   |
 1 | #![deny(unused_must_use, unreachable_code)]
   |                          ^^^^^^^^^^^^^^^^

error: unused return value of `save` that must be used
  --> src/main.rs:13:5
   |
13 |     save(b"world");
   |     ^^^^^^^^^^^^^^
   |
note: the lint level is defined here
  --> src/main.rs:1:9
   |
 1 | #![deny(unused_must_use, unreachable_code)]
   |         ^^^^^^^^^^^^^^^
help: use `let _ = ...` to ignore the resulting value
   |
13 |     let _ = save(b"world");
   |     +++++++

error: aborting due to 2 previous errors
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`#[must_use]` 告诉编译器"这个返回值不能默默丢掉"，因为丢掉通常意味着逻辑漏了一步——`Result` 没处理、锁没释放、字节数没累加。默认级别下它只是警告，编译能过、程序照跑，这正是危险之处。第二条是另一个问题：第 14 行的 return 让后面的 println! 永远执行不到，rustc 的 unreachable_code 检查会指出来。示例里用 `#![deny(...)]` 把两条都升级成错误，方便一次看全。这两条都属于 lint（代码检查），不是类型错误，所以 rustc 不给 E 编号——只有类型、借用、名字解析这类硬错误才有 E 码。

**修复**：要么用掉返回值，要么明确表示"我知道我在丢弃"。顺手删掉不可达的语句。

```rust,editable
#![deny(unused_must_use, unreachable_code)]

#[must_use]
fn save(data: &[u8]) -> usize {
    data.len()
}

fn main() {
    let total = save(b"hello") + save(b"world");
    println!("总计 {total} 字节");
}
```

CI 里常见的做法是加 `RUSTFLAGS="-D warnings"`，把全项目的警告一律升级为错误；本地开发时保留警告即可，但不要养成无视它的习惯。

</details>

## 动手练习

1. 写 `fn collatz_steps(mut n: u64) -> u32`：偶数除以 2，奇数乘 3 加 1，从 n 走到 1 一共几步。要求用 loop 的 break 带值返回，全程不写 return。
2. 把 3.1 的 gcd 改写成 while 版本，签名不变。再回答：递归版在输入很大时为什么不安全？本章哪一小节解释了原因？
3. 写 `fn first_index_gt(v: &[i32], limit: i32) -> Option<usize>`，返回第一个大于 limit 的元素下标。要求用 for + enumerate，不用下标循环，也不要调用 position。
4. 用 `'outer` 标签在二维数组里找第一个能被 7 整除的数，返回它本身，找不到返回 0。写完后只删掉标签、保留 break，看编译器报什么。
5. 写两版 `fn sum_evens(v: &[i32]) -> i32`：一版用 `0..v.len()` 下标，一版用 `v.iter()`。给其中一版加 `#[must_use]`，然后故意丢掉返回值，分别在默认级别和 `#![deny(unused_must_use)]` 下编译，对比结果。

## 小结

- **函数是签名加一个块。** 参数类型必须写，返回类型只在返回 () 时可省；名字和类型构成调用方唯一能依赖的东西，没有重载、没有默认参数。
- **分号决定值。** 表达式加分号变语句，块的值退化成 ()；函数体的最后一句就是返回值，这也是 E0308 最高频的来源。
- **if 是表达式，不是语句。** 两边分支类型一致才能当值用，缺 `else` 时整个表达式的类型是 `()`；条件必须是真的 bool。
- **! 与 () 相反。** () 是唯一值也是"没有东西"，! 是"永不返回"，可以转换到任何类型，所以 panic! 能填在任何需要值的位置。
- **loop 能带值退出，while 和 for 不能。** 需要"算一个值"用 loop + break，需要"遍历"用 for，标签让 break 一次跳出多层。
- **range 决定类型，也决定能力。** 端点是 usize 就得到 usize；双向、长度已知的范围可以先 `step_by` 再 `rev`，交换顺序可能改变结果。
- **内联是优化器的决定，不是语法。** #[inline] 的真正用途是跨 crate 暴露函数体；#[inline(always)] 有代码膨胀的代价，先测量再标。
- **普通函数调用不保证尾调用优化。** 递归深度跟着输入走时，改写成循环比调大栈更可靠。
