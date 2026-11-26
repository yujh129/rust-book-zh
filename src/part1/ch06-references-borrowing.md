# 第 6 章 所有权（二）：引用与借用

> **本章目标**
> - 分清 `&T` 与 `&mut T`，理解"借用"为什么比"移动"常用
> - 掌握借用规则，并从"它排除了什么错误"的角度理解它
> - 知道借用检查器大致怎么工作（NLL、活跃区间），能预判报错
> - 分清重借用、切片、`split_at_mut` 这些"绕过冲突"的标准手法
> - 理解为什么 Rust 引用在运行期是零开销的
> - 躲开 5 个借用相关的经典编译错误

## 6.1 借用的动机

上一章结尾留了个问题：函数只要读一下 `String`，为什么非要把所有权交出去？

```rust
fn length(s: &String) -> usize {
    s.len()
}

fn main() {
    let s = String::from("hello");
    let n = length(&s);
    println!("{s} 的长度是 {n}");   // s 仍然可用
}
```

`&s` 创建了一个**引用**（reference）：它指向 `s` 的值，但不拥有它。用术语说，是"把 `s` 借出去了"。函数结束后归还，`s` 从头到尾都是所有者。

借用的价值在参数传递上体现得最明显：

```rust
fn by_value(s: String) -> String {
    s   // 必须还回去，否则调用方就丢了
}

fn by_ref(s: &str) -> usize {
    s.len()
}

fn main() {
    let s = String::from("hello");

    let s = by_value(s);        // 传进去再拿回来，麻烦
    let n = by_ref(&s);         // 只借，干净
    println!("{s} {n}");
}
```

真实代码里，`&T` 出现的频率远高于 `T`。默认写 `&`，只有确实要消费或转移所有权时才传值。

## 6.2 共享引用与独占引用

| | `&T` | `&mut T` |
| --- | --- | --- |
| 别名 | 共享引用、只读引用 | 独占引用、可变引用 |
| 能同时存在几个 | 任意多个 | 同一时刻只能一个 |
| 能读吗 | 能 | 能 |
| 能改吗 | 不能 | 能 |
| 能否与对方共存 | 不能与 `&mut T` 共存 | 不能与任何引用共存 |
| 是否 `Copy` | 是 | 否 |

借用规则一句话：**在任意时刻，要么有任意多个 `&T`，要么有唯一一个 `&mut T`，二者不可兼得。**

```rust
fn main() {
    let mut v = vec![1, 2, 3];

    let a = &v;
    let b = &v;
    println!("{a:?} {b:?}");     // 多个共享引用：可以

    let c = &mut v;
    c.push(4);                    // 独占引用：可以修改
    println!("{c:?}");

    // let d = &v;                // 这里再建共享引用：可以（c 已不再使用）
}
```

注意最后一行注释：`c` 之后不再使用，所以再借 `&v` 是合法的。这就是下一节要讲的 NLL。

### 规则排除了什么

这条规则不是"为了难为你"，它精确地排除了两类真实 bug：

**第一类：迭代中修改集合**

```rust,noplayground
fn main() {
    let mut v = vec![1, 2, 3];
    for x in &v {
        if *x == 2 {
            v.push(4);   // 编译错误：遍历时借用了 v，不能再可变借用
        }
    }
}
```

```text
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:13
  |
3 |     for x in &v {
  |               -- immutable borrow occurs here
4 |         if *x == 2 {
5 |             v.push(4);
  |             ^^^^^^^^^ mutable borrow occurs here
6 |         }
7 |     }
  |     - immutable borrow later used here
```

在 Java 里这段代码会抛 `ConcurrentModificationException`（运行时），在 C++ 里是未定义行为（可能读到被释放的内存），在 Rust 里是编译错误。**同一类 bug，三种语言的应对时机不同。**

正确写法是"先收集再修改"：

```rust
fn main() {
    let mut v = vec![1, 2, 3];
    let mut to_add = Vec::new();
    for x in &v {
        if *x == 2 {
            to_add.push(4);
        }
    }
    v.extend(to_add);
    println!("{v:?}");
}
```

或者用下标（不借用元素）：

```rust
fn main() {
    let mut v = vec![1, 2, 3];
    for i in 0..v.len() {
        if v[i] == 2 {
            v.push(4);
            break;
        }
    }
    println!("{v:?}");
}
```

**第二类：两个指针写同一块内存**

在 C 里，如果你同时持有两个指向同一 `Vec` 的指针，一个 push（可能重新分配并释放旧内存），另一个还在读旧指针——悬垂。Rust 用"独占引用唯一"在编译期禁止了这种组合。

代价是：**你必须重新组织代码来表示"这里确实需要同时读写"**。这通常意味着你的数据结构设计可以更好（分片、索引、内部可变性），而不是 Rust 太严格。

## 6.3 深入：借用检查器怎么工作

理解这一点，能让你从"试错"变成"预判"。

### 生命周期标注不是"活的时长"

先澄清一个常见误解：`'a` 不是"变量活多久"，而是**借用必须有效的区间**。编译器为每个借用计算一个"活跃区间"（region），然后检查：

1. 引用的存活区间不能长于被引用值的存活区间（否则悬垂）。
2. 不得同时存在相互冲突的活跃借用（共享 vs 独占）。

### NLL：非词法生命周期

Rust 2018 起用的是 NLL（Non-Lexical Lifetimes）：借用的结束点是**最后一次使用**，而不是作用域结尾。

```rust
fn main() {
    let mut v = vec![1, 2, 3];

    let first = &v[0];
    println!("{first}");      // first 最后一次被使用 ← 借用在这里结束

    v.push(4);                // 所以这里可以可变借用
    println!("{v:?}");
}
```

如果按旧规则（借用活到作用域结束），`v.push(4)` 会报错。NLL 之后不报了，因为 `first` 在 `println` 之后就没用了。

报错信息的 "immutable borrow later used here" 就是在告诉你活跃区间的另一端在哪：

```rust,noplayground
fn main() {
    let mut v = vec![1, 2, 3];
    let first = &v[0];
    v.push(4);                 // 错误
    println!("{first}");       // ← 因为这里还要用 first
}
```

```text
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
 --> src/main.rs:4:5
  |
3 |     let first = &v[0];
  |                 - immutable borrow occurs here
4 |     v.push(4);
  |     ^^^^^^^^^ mutable borrow occurs here
5 |     println!("{first}");
  |               ----- immutable borrow later used here
```

**读懂这三行标注**（borrow occurs here / mutable borrow occurs here / later used here），就能判断该怎么调整顺序——通常只要把 `push` 挪到 `println` 之后就行。

### 借用是零开销的

`&T` 在机器码层面就是一个指针（`&str` 是"指针 + 长度"）。没有引用计数、没有 GC、没有运行期检查（数组越界检查是另一回事）。**借用检查完全发生在编译期**。

这是 Rust 与 Java/Python 的根本差异：后者靠运行时 GC 保证内存安全，Rust 把这件事提前到了编译期，代价是你要先说服编译器。

## 6.4 悬垂引用：编译期就挡住

```rust,noplayground
fn dangle() -> &String {
    let s = String::from("hello");
    &s
}

fn main() {
    let r = dangle();
    println!("{r}");
}
```

```text
error[E0106]: missing lifetime specifier
 --> src/main.rs:1:16
  |
1 | fn dangle() -> &String {
  |                ^ expected named lifetime parameter
  |
  = help: this function's return type contains a borrowed value, but there is
          no value for it to be borrowed from
help: consider using the `'static` lifetime
  |
1 | fn dangle() -> &'static String {
```

编译器在**函数签名层面**就发现"这个引用没有来源"，根本不让你写出来。补上生命周期标注后，会得到更本质的报错：

```rust,noplayground
fn dangle<'a>() -> &'a String {
    let s = String::from("hello");
    &s
}
```

```text
error[E0515]: cannot return reference to local variable `s`
 --> src/main.rs:3:5
  |
3 |     &s
  |     ^^ returns a reference to data owned by the current function
```

**原因**：`s` 在函数返回时被 drop，返回的引用立刻悬垂。
**修复**：返回所有权 `fn make() -> String`；或者返回 `&'static str`（字面量）；或者让调用方传入 `&mut String` 由它持有。

对比一下 C++ 的等价代码——它能编译、能跑、是未定义行为：

```cpp
std::string& dangle() {
    std::string s = "hello";
    return s;      // 警告都未必有
}
```

这就是"编译期消除一整类 bug"的实际价值。

## 6.5 重借用与切片

### 重借用

从 `&mut T` 出发可以再借出 `&mut T` 或 `&T`，这叫重借用（reborrow）。编译器在参数位置自动做这件事：

```rust
fn read(s: &str) { println!("{s}"); }

fn append(s: &mut String) {
    s.push_str("!");
}

fn main() {
    let mut s = String::from("hi");
    append(&mut s);        // 自动重借用
    read(&s);              // 然后共享借用，没问题
    println!("{s}");
}
```

显式重借用写法是 `&*x`（共享）与 `&mut *x`（独占），在泛型代码和 `match` 里偶尔必须写：

```rust
fn main() {
    let mut v = vec![1, 2, 3];
    let r: &mut Vec<i32> = &mut v;
    let inner: &mut [i32] = &mut *r;   // 从 &mut Vec 借出 &mut [i32]
    inner[0] = 9;
    println!("{v:?}");
}
```

`&mut Vec<i32>` 能自动变成 `&mut [i32]`（Deref 强制转换），这跟 `&String` → `&str` 是同一个机制，第 17 章细讲。

### 切片：只借一部分

```rust
fn main() {
    let v = [10, 20, 30, 40, 50];

    let all: &[i32] = &v;
    let mid: &[i32] = &v[1..4];      // [20, 30, 40]
    let tail: &[i32] = &v[2..];
    let head: &[i32] = &v[..2];

    println!("{} {} {} {}", all.len(), mid.len(), tail.len(), head.len());
    println!("{mid:?}");
}
```

切片是一个"胖指针"：指向起始位置 + 长度。它不拥有数据，所以可以在不拷贝的情况下传给函数：

```rust
fn sum(xs: &[i32]) -> i32 {
    xs.iter().sum()
}

fn main() {
    let v = vec![1, 2, 3];
    let a = [4, 5];
    println!("{} {}", sum(&v), sum(&a));   // Vec 和数组都能传
}
```

**这是 Rust 里最重要的 API 设计实践之一**：参数收 `&[T]` 而不是 `&Vec<T>`，收 `&str` 而不是 `&String`，调用方就不用为了适配而转换或克隆。

### split_at_mut：同时借用两半

想同时可变借用同一个 `Vec` 的两部分，规则上不允许。但切片提供了合法出口：

```rust
fn main() {
    let mut v = vec![1, 2, 3, 4, 5];

    let (left, right) = v.split_at_mut(2);
    // left 和 right 是两个不重叠的可变切片
    left[0] = 10;
    right[0] = 40;

    println!("{v:?}");   // [10, 2, 3, 40, 5]
}
```

标准库还有 `split_first_mut`、`chunks_mut`、`windows_mut`（需第三方或手写）、`iter_mut().enumerate()`。原则是：**把"同时可变访问两处"的需求表达成"两个不重叠的切片"**，借用检查器就能理解。

## 6.6 借用与函数签名

写函数时，参数该用哪种形态，有一个非常实用的判断表：

| 需求 | 参数类型 | 说明 |
| --- | --- | --- |
| 只读字符串 | `&str` | 字面量、`String`、`&String` 都能传 |
| 只读任意序列 | `&[T]` | `Vec<T>`、数组、切片都能传 |
| 只读结构体 | `&Struct` | |
| 要修改 | `&mut T` | 调用方必须有 `mut` 变量 |
| 要消耗/转移 | `T` | 调用方之后不能再用 |
| 可选存在 | `Option<&T>` | 不分配、不转移 |
| 要么拥有要么借用 | `Cow<'_, str>` | 需要返回"有时新造有时借用"时用 |

两个例子：

```rust
use std::borrow::Cow;

// 不好：强制调用方持有 &String
fn bad_len(s: &String) -> usize { s.len() }

// 好：&str 能接受一切字符串形态
fn good_len(s: &str) -> usize { s.len() }

// 返回：可能借用输入，也可能生成新字符串
fn shout(s: &str) -> Cow<'_, str> {
    if s.ends_with('!') {
        Cow::Borrowed(s)          // 不用分配
    } else {
        Cow::Owned(format!("{s}!"))
    }
}

fn main() {
    println!("{}", bad_len(&String::from("a")));
    println!("{}", good_len("a"));
    println!("{} {}", shout("hi"), shout("hi!"));
}
```

## 典型错误案例

### 案例 1：同时存在共享与独占借用

```rust,editable
fn main() {
    let mut data = vec![1, 2, 3];
    let first = &data[0];
    data.push(4);
    println!("{first}");
}
```

```text
error[E0502]: cannot borrow `data` as mutable because it is also borrowed as immutable
 --> src/main.rs:4:5
  |
3 |     let first = &data[0];
  |                 ----- immutable borrow occurs here
4 |     data.push(4);
  |     ^^^^^^^^^^^^^ mutable borrow occurs here
5 |     println!("{first}");
  |               ----- immutable borrow later used here
```

**原因**：`push` 可能导致堆内存重新分配，`first` 会变成悬垂指针。
**修复**：先取出值再改——`let first = data[0];`（`i32` 是 `Copy`，直接复制）。

### 案例 2：两个独占借用

```rust,editable
fn main() {
    let mut s = String::from("hi");
    let a = &mut s;
    let b = &mut s;
    a.push('!');
    b.push('?');
}
```

```text
error[E0499]: cannot borrow `s` as mutable more than once at a time
 --> src/main.rs:4:13
  |
3 |     let a = &mut s;
  |             ------ first mutable borrow occurs here
4 |     let b = &mut s;
  |             ^^^^^^ second mutable borrow occurs here
5 |     a.push('!');
  |     - first borrow later used here
```

**原因**：独占引用必须唯一，否则两个 `&mut` 可能同时修改同一块内存，破坏一致性。
**修复**：让第一个借用结束再借第二个（把 `a.push` 挪到 `let b` 前），或者用 `split_at_mut`。

### 案例 3：对不可变变量取可变借用

```rust,editable
fn main() {
    let s = String::from("hi");
    let r = &mut s;
    r.push('!');
}
```

```text
error[E0596]: cannot borrow `s` as mutable, as it is not declared as mutable
 --> src/main.rs:3:13
  |
2 |     let s = String::from("hi");
  |         - help: consider changing this to be mutable: `mut s`
3 |     let r = &mut s;
  |             ^^^^^^ cannot borrow as mutable
```

**原因**：能不能借用可变，首先取决于所有者自己是否可变。
**修复**：`let mut s = ...`。注意：**`mut` 是变量的性质，不是类型的性质**——`&mut T` 类型本身保证被指向的值可变，但被借的那一方得先允许。

### 案例 4：临时值在语句结束时被释放

```rust,editable
fn longest<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.len() > b.len() { a } else { b }
}

fn main() {
    let s = longest("abc", &String::from("de"));
    println!("{s}");
}
```

```text
error[E0716]: temporary value dropped while borrowed
 --> src/main.rs:6:26
  |
6 |     let s = longest("abc", &String::from("de"));
  |                            ^^^^^^^^^^^^^^^^^^^ creates a temporary which is freed while still in use
7 |     println!("{s}");
  |               - borrow later used here
help: consider using a `let` binding to create a longer lived value
  |
6 |     let t = String::from("de");
7 |     let s = longest("abc", &t);
```

**原因**：`&String::from("de")` 里的 `String` 是临时值，语句结束就被 drop，而 `s` 还想指向它。
**修复**：给临时值一个名字。这类错误在链式调用、`if let` 里也很常见。

### 案例 5：结构体里存引用却没有生命周期

```rust,editable
struct Parser {
    input: &str,
}

fn main() {
    let p = Parser { input: "abc" };
    println!("{}", p.input);
}
```

```text
error[E0106]: missing lifetime specifier
 --> src/main.rs:2:12
  |
2 |     input: &str,
  |            ^ expected named lifetime parameter
  |
help: consider introducing a named lifetime parameter
  |
1 | struct Parser<'a> {
2 |     input: &'a str,
```

**原因**：结构体存引用时，必须说明"这个引用和哪个东西活得一样久"。
**修复**：`struct Parser<'a> { input: &'a str }`，使用时 `Parser { input: "abc" }`。如果觉得生命周期标注太吵，考虑存 `String`（拥有所有权）——第 17 章会讨论这个取舍。

## 动手练习

1. 写一个函数接收 `&mut Vec<i32>`，把里面的偶数都乘以 2。再用 `&[i32]` 版本对比，说明为什么后者不能改。
2. 只写一次遍历，用一个 `&mut` 引用同时修改 `Vec` 的相邻元素（提示：`chunks_mut(2)`）。
3. 预测下面代码能否编译，然后验证并解释：
   ```rust,noplayground
   fn main() {
       let mut v = vec![1, 2, 3];
   let a = &v;
   let b = &v;
   println!("{a:?}");
   v.push(4);
   println!("{b:?}");
   ```
4. 实现 `fn middle(xs: &[i32]) -> Option<&i32>`，返回中间元素（长度为空返回 `None`）。
5. 解释为什么 `fn push_x(s: &String) { ... }` 能通过编译，而 `fn push_x(s: &str) { s.push_str("x") }` 不能。

## 小结

- 借用让函数在不转移所有权的前提下访问数据；默认用 `&`，需要消费才传值。
- 规则：任意多个 `&T` 或唯一一个 `&mut T`，二者不可兼得；它排除了悬垂引用与数据竞争。
- NLL 把借用的结束点定在最后一次使用，报错里的三行标注（occur / mutable occur / later used）是排错地图。
- 借用检查是纯编译期的，对 `T: Sized`，`&T` 与地址同宽；切片或 trait 对象的引用还携带元数据，借用检查本身没有运行时计数。
- 悬垂引用在函数签名层面就被 `E0106`/`E0515` 挡住。
- 重借用（`&*x` / `&mut *x`）与切片（`&[T]`、`split_at_mut`）是化解借用冲突的标准工具。
- API 设计：参数收 `&str`、`&[T]` 而不是 `&String`、`&Vec<T>`。
