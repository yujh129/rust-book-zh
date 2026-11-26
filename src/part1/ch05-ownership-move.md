# 第 5 章 所有权（一）：移动语义

> **本章目标**
> - 用栈与堆的例子解释所有权规则，而不是背三条规则
> - 理解移动在机器层面到底做了什么，为什么它不是"拷贝一份"
> - 分清 `Copy` 和 `Clone`：谁自动、谁显式、谁有代价
> - 掌握部分移动、`mem::take/replace`、函数传参与返回时的所有权流转
> - 建立"什么时候必须转移所有权"的判断标准
> - 躲开 5 个所有权相关的经典编译错误

## 5.1 从一段 C++ 说起

下面的 C++ 片段在对象析构后使用悬垂指针，行为未定义，可能崩溃也可能看似正常：

```cpp
std::vector<int> make() {
    std::vector<int> v = {1, 2, 3};
    return v;                 // 返回时拷贝一次（或移动）
}

int main() {
    std::vector<int>* p = nullptr;
    {
        std::vector<int> v = {1, 2, 3};
        p = &v;               // 拿到栈上对象的地址
    }                          // v 在这里析构，内存被回收
    std::cout << (*p)[0];      // 悬垂指针！未定义行为
}
```

C++ 给出了"零开销抽象"的能力，也把"这块内存归谁管"的问题交给了程序员。Rust 从另一个方向回答同一个问题：**给每个值指定唯一的主人，用编译器检查主人还活着**。

这就是所有权。它不是"Rust 新增的语法糖"，而是一套**静态的、编译期完成的内存管理规则**——没有 GC，也没有手写 `free`。

三条规则：

1. 以 `String` 为例，一个值由一个绑定拥有，借用它不转移所有权。
2. 移动把所有权交给新位置，原位置需要重新初始化后才能再取值。
3. 正常离开作用域时，仍由该位置拥有的值会被析构。`Rc` 和 `Arc` 则可以让多个句柄共同拥有同一份数据（第 18、21 章）。

初读像口号，接下来看它们怎么起作用。

## 5.2 栈与堆：规则从哪来

| | 栈 | 堆 |
| --- | --- | --- |
| 分配速度 | 极快（移动栈指针） | 慢（找空闲块、可能系统调用） |
| 大小 | 编译期已知、固定 | 运行期可变 |
| 释放 | 函数返回自动 | 需要显式或自动管理 |
| 谁在上面 | 局部变量、固定大小数组 | `String`、`Vec`、`Box` 的内容 |

关键在于：**栈上的东西随作用域自动清理，堆上的东西需要一个决定"什么时候清"的机制**。Rust 的答案是"栈上的那个所有者负责它指向的堆内存"。

```text
let s = String::from("hello");

栈 (s)          堆
┌──────────┐    ┌───┬───┬───┬───┬───┐
│ ptr    ──┼───>│ h │ e │ l │ l │ o │
│ len    5 │    └───┴───┴───┴───┴───┘
│ cap    5 │
└──────────┘
```

在本书使用的 64 位目标上，`String` 值的大小是 24 字节，包含指针、长度、容量。图中画的是局部变量；如果它是堆上结构体的字段，这三个字段也随结构体存放。图示不规定字段顺序。

## 5.3 移动：到底发生了什么

```rust
fn main() {
    let s1 = String::from("hello");
    let s2 = s1;                 // s1 被移动（moved）到 s2

    println!("{s2}");
    // println!("{s1}");         // 如果取消注释，编译失败
}
```

如果 Rust 允许在 `let s2 = s1` 之后继续用 `s1`，那么 `s1` 和 `s2` 都认为自己拥有那块堆内存，作用域结束时就会 `free` 两次——double free，这是 C 系语言最经典的安全漏洞来源。

所以 `let s2 = s1` 的行为是：

```text
1. s2 接管原来的 String，包括对同一块堆缓冲区的所有权
2. 把 s1 标记为"已移动"，编译器禁止再使用它
3. 作用域结束时，只有 s2 触发 drop，堆内存只释放一次
```

**移动描述所有权的转移。** 编译器可能复制值的字节，也可能直接复用原位置。Rust 不要求执行一次 `memcpy`，也不要求清零源位置。

移动 `String` 的开销不随字符串长度增长，堆里的字节留在原处。对于内联存放的大数组，如果需要搬运，代价可以随数组大小增长。

### 用编译器确认移动的语义

```rust,noplayground
fn main() {
    let s1 = String::from("hello");
    let s2 = s1;
    println!("{s1}");
}
```

```text
error[E0382]: borrow of moved value: `s1`
 --> src/main.rs:4:15
  |
2 |     let s1 = String::from("hello");
  |         -- move occurs because `s1` has type `String`, which does not implement the `Copy` trait
3 |     let s2 = s1;
  |              -- value moved here
4 |     println!("{s1}");
  |               ^^ value borrowed here after move
  |
  = note: this error originates in the macro `println!`
help: consider cloning the value if the performance cost is acceptable
  |
3 |     let s2 = s1.clone();
  |               ++++++++
```

报错里那句 "`s1` has type `String`, which does not implement the `Copy` trait" 是关键：**移动发生的条件是这个类型不实现 `Copy`**。下一节展开。

## 5.4 Copy 与 Clone

```rust
fn main() {
    // 基本类型实现了 Copy：赋值是复制，原变量仍然可用
    let x = 5;
    let y = x;
    println!("x = {x}, y = {y}");   // 两个都能用

    let a = (1, 2.0, 'c');
    let b = a;                       // 元素全 Copy，元组也 Copy
    println!("{a:?} {b:?}");
}
```

哪些类型是 `Copy`：

| 是 Copy | 不是 Copy |
| --- | --- |
| 所有整数、浮点、`bool`、`char` | `String` |
| `&T`（共享引用） | `Vec<T>`、`Box<T>` |
| 元素全是 Copy 的数组、元组 | `&mut T`（独占引用） |
| `*const T` / `*mut T`（裸指针） | 任何实现了 `Drop` 的类型 |

**判断原则**：`Copy` 意味着"按位复制之后，两个值都有效"。`String` 复制后会出现两个所有者指向同一块堆内存，所以不能 `Copy`。`&mut T` 复制后会出现两个独占引用，违反借用规则，所以也不能 `Copy`（`&T` 可以，多个只读引用无妨）。

`Clone` 是显式的、可以深拷贝的：

```rust
fn main() {
    let s1 = String::from("hello");
    let s2 = s1.clone();       // 堆上的内容也复制一份
    println!("{s1} {s2}");     // 两个都有效，各自拥有一块内存
}
```

| | `Copy` | `Clone` |
| --- | --- | --- |
| 触发方式 | 隐式（赋值、传参） | 显式调用 `.clone()` |
| 语义 | 按位复制 | 可以是任意深拷贝逻辑 |
| 代价 | 与值大小和优化有关，大型 `Copy` 数组也可能昂贵 | 可能很贵 |
| 关系 | `Copy: Clone`（Copy 的类型必然实现 Clone） | 反之不成立 |

**为什么 `Clone` 要显式**：这是 Rust 的一个刻意设计——让"这里有性能开销"在代码里可见。看到 `.clone()` 你就知道可能需要优化；如果像某些语言那样隐式深拷贝，性能问题会藏起来。

### 自己实现 Copy/Clone

```rust
#[derive(Debug, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

fn main() {
    let p1 = Point { x: 1.0, y: 2.0 };
    let p2 = p1;          // Copy，p1 仍可用
    println!("{p1:?} {p2:?}");
}
```

`#[derive(Copy, Clone)]` 要求所有字段都实现对应 trait。如果字段里有 `String`，`Copy` 会编译失败——这是正确的，因为 `Point` 就不该能按位复制。

## 5.5 所有权在函数间的流转

传参和返回值都会转移所有权：

```rust
fn take(s: String) {
    println!("函数内：{s}");
}   // s 在这里被 drop

fn give() -> String {
    String::from("new")
}

fn main() {
    let s = String::from("hello");
    take(s);              // s 的所有权移入函数
    // println!("{s}");   // 编译错误：s 已被移动

    let t = give();       // 所有权移出函数
    println!("{t}");
}
```

这带来一个真实的麻烦：**只想读取，却被迫交出所有权**。于是有了引用：

```rust
fn length(s: &String) -> usize {
    s.len()
}

fn main() {
    let s = String::from("hello");
    let n = length(&s);        // 借用，不转移
    println!("{s} 的长度是 {n}");
}
```

第 6 章整章讲引用。这里先建立判断标准：

| 函数想做什么 | 参数类型 | 调用后原变量 |
| --- | --- | --- |
| 只读 | `&T` | 可用 |
| 修改 | `&mut T` | 可用 |
| 消费/销毁 | `T` | 不可用 |
| 拿走并返回新的 | `T` → `U` | 不可用 |

**API 设计层面**：一个函数如果不需要拥有参数，就不该要所有权。要 `String` 但只用到了只读，改成 `&str` 会让调用方舒服很多（`&String` 能自动转 `&str`，字面量也直接能用）。第 17 章会讲得更好。

## 5.6 部分移动

Rust 允许移走结构体的某些字段，剩下的字段仍可用——这叫部分移动：

```rust
struct Person {
    name: String,
    age: u32,
}

fn main() {
    let p = Person { name: String::from("Alice"), age: 30 };

    let name = p.name;          // 移走 name
    // println!("{:?}", p);     // 编译错误：p 已被部分移动
    println!("{} {}", name, p.age);  // age 是 Copy，仍可读
}
```

规则是：**被移走的字段不能再访问，整个结构体也不能再整体使用，但没被移动的字段仍然可以访问**（如果它们是 `Copy`，或者你只是借用它们）。

```rust,noplayground
fn main() {
    let v = vec![String::from("a"), String::from("b")];
    let first = v[0];      // 想从 Vec 里移出元素
    println!("{first}");
}
```

```text
error[E0507]: cannot move out of index of `Vec<String>`
 --> src/main.rs:3:17
  |
3 |     let first = v[0];
  |                 ^^^^ move occurs because value has type `String`, which does not implement the `Copy` trait
  |
help: consider borrowing here
  |
3 |     let first = &v[0];
  |                 +
help: or use the `Index` trait's `index` method
```

**原因**：`v[0]` 是 `Index` trait 的语法糖，返回的是 `&String`，背后不允许你偷偷拿走元素（否则下标越界、双份所有权都会出问题）。
**修复**：借 `&v[0]`；真要拿走就用 `v.remove(0)`（会移动后续元素）或 `v.into_iter()` 消费整个向量。

### 深入：mem::take 与 mem::replace

"我只有一个 `&mut T`，但我想把里面的值拿出来换个新的"——这是很常见的需求，标准库给了两个零成本的答案：

```rust
use std::mem;

fn main() {
    // take: 拿走值，原地留下 Default::default()
    let mut s = String::from("hello");
    let taken = mem::take(&mut s);     // s 变成空字符串
    println!("taken = {taken}, s = '{s}'");

    // replace: 换成指定值，返回旧值
    let mut n = 10;
    let old = mem::replace(&mut n, 20);
    println!("old = {old}, n = {n}");

    // 经典用法：结构体字段之间转移所有权
    let mut v = vec![1, 2, 3];
    let moved = mem::take(&mut v);   // v 变成空 Vec，moved 拿到 [1,2,3]
    println!("{moved:?} {v:?}");
}
```

`mem::take` 要求类型实现 `Default`；`mem::replace` 不需要。两者都只是"按位搬运 + 写入新值"，没有堆分配。在实现状态机、`Option` 状态迁移时非常有用：

```rust
fn main() {
    let mut slot: Option<String> = Some(String::from("value"));
    if let Some(v) = slot.take() {      // take() 把 Some 换成 None 并拿走内容
        println!("拿到 {v}，slot 现在是 {:?}", slot);
    }
}
```

`Option::take` 就是 `mem::take` 的特例，在链表、树遍历里几乎是标配。

## 5.7 Drop：所有者的告别

值离开作用域时会调用 `Drop::drop`：

```rust
struct Noisy(&'static str);

impl Drop for Noisy {
    fn drop(&mut self) {
        println!("drop {}", self.0);
    }
}

fn main() {
    let _a = Noisy("a");
    let _b = Noisy("b");
    println!("main 结束");
}
```

```console
$ cargo run
main 结束
drop b
drop a
```

**变量按声明的逆序释放**（后进先出，和栈一致）。

三个要点：

- 实现了 `Drop` 的类型**不能实现 `Copy`**。因为 `Copy` 意味着按位复制出两个有效值，而 `Drop` 会在两处都跑一次——双重释放。
- 不能手动调 `drop(x)` 之外的析构；`std::mem::drop(x)` 实际上只是"把它移进一个空函数"，让作用域提前结束。
- `Drop::drop` 里不要 panic（panic 中再 panic 会直接 abort）。

```rust
fn main() {
    let s = String::from("hello");
    drop(s);                 // 显式提前释放
    // println!("{s}");      // 编译错误：已移动
    println!("之后就安全了");
}
```

**使用场景**：需要在函数结束前释放锁、关闭文件句柄、提交事务时，`drop(guard)` 是惯用写法。

## 典型错误案例

### 案例 1：移动之后继续使用

```rust,editable
fn main() {
    let s = String::from("hi");
    let t = s;
    let u = s;
    println!("{t} {u}");
}
```

```text
error[E0382]: use of moved value: `s`
 --> src/main.rs:4:13
  |
2 |     let s = String::from("hi");
  |         - move occurs because `s` has type `String`, which does not implement the `Copy` trait
3 |     let t = s;
  |             - value moved here
4 |     let u = s;
  |             ^ value used here after move
  |
help: consider cloning the value if the performance cost is acceptable
  |
3 |     let t = s.clone();
```

**原因**：同一块堆内存不能有两个所有者。
**修复**：如果你确实需要两份独立数据，`s.clone()`；如果只是读取，改成借用 `&s`。

### 案例 2：在循环里移动集合元素

```rust,editable
fn main() {
    let names = vec![String::from("a"), String::from("b")];
    for name in names {
        println!("{name}");
    }
    println!("{}", names.len());   // 循环已经消费了 names
}
```

```text
error[E0382]: borrow of moved value: `names`
 --> src/main.rs:6:20
  |
2 |     let names = vec![String::from("a"), String::from("b")];
  |         ----- move occurs because `names` has type `Vec<String>`, which does not implement the `Copy` trait
3 |     for name in names {
  |                 ----- `names` moved due to this implicit call to `IntoIterator`
...
6 |     println!("{}", names.len());
  |                    ^^^^^ value borrowed here after move
```

**原因**：`for x in collection` 是 `IntoIterator::into_iter(collection)` 的语法糖，按值迭代会消费集合。
**修复**：`for name in &names`（迭代 `&String`）或 `for name in names.iter()`。要同时拿到索引用 `names.iter().enumerate()`。只有确实要销毁集合、把元素搬走时才用按值循环。

### 案例 3：从不可变的引用里移出

```rust,editable
struct Config {
    name: String,
}

fn describe(c: &Config) -> String {
    let n = c.name;      // 想拿走
    n
}

fn main() {
    let c = Config { name: String::from("x") };
    println!("{}", describe(&c));
}
```

```text
error[E0507]: cannot move out of `c.name` which is behind a shared reference
 --> src/main.rs:6:13
  |
6 |     let n = c.name;
  |             ^^^^^^ move occurs because `c.name` has type `String`, which does not implement the `Copy` trait
  |
help: consider borrowing here
  |
6 |     let n = &c.name;
```

**原因**：你手上只有一份只读借用的 `&Config`，没有权力把里面的字段搬走（否则原值就残缺了）。
**修复**：借用 `&c.name`；确实要一份独立的用 `c.name.clone()`；如果要消耗 `Config`，把参数改成 `c: Config`（`&Config` → `Config` 是调用方的事）。

### 案例 4：把闭包/部分移动后的整体当整体用

```rust,editable
#[derive(Debug)]
struct Job {
    id: u32,
    payload: Vec<u8>,
}

fn main() {
    let job = Job { id: 1, payload: vec![1, 2, 3] };
    let payload = job.payload;
    println!("{job:?}");
    println!("{payload:?}");
}
```

```text
error[E0382]: borrow of partially moved value: `job`
 --> src/main.rs:11:15
  |
9  |     let payload = job.payload;
   |                   ----------- `job.payload` partially moved due to this use
10 |     println!("{job:?}");
   |               ^^^ value borrowed here after partial move
   |
   = note: partial move occurs because `job.payload` has type `Vec<u8>`, which does not implement the `Copy` trait
help: borrow this field to avoid moving the value
   |
9  |     let payload = &job.payload;
```

**原因**：字段被移走之后，`job` 处于"部分移动"状态，不能再整体使用（`{:?}` 需要读全部字段）。
**修复**：只借用字段（`&job.payload`）；或者一开始就整体 `let payload = job.payload.clone()`；或者用 `mem::take(&mut job.payload)` 让原字段变成空 `Vec`，结构体就仍然完整可用。

### 案例 5：需要 Copy 却收到了非 Copy 的类型

```rust,editable
#[derive(Clone, Debug)]
struct Token(String);

fn consume(t: Token) -> Token { t }

fn main() {
    let token = Token(String::from("abc"));
    let a = token;
    let b = token;      // 第二次移动
    println!("{:?} {:?}", consume(a), consume(b));
}
```

```text
error[E0382]: use of moved value: `token`
 --> src/main.rs:8:13
  |
7 |     let a = token;
  |             ----- value moved here
8 |     let b = token;
  |             ^^^^^ value moved here, in previous iteration of loop
  |
  = note: `Token` does not implement `Copy`
```

**原因**：`Token` 内部有 `String`，不可能 `Copy`。
**修复**：`let b = token.clone();`，或者重新设计——比如把 `Token` 设计成 `Token(u64)`（ID 语义，可以 `Copy`），或者让函数收 `&Token` 而不是 `Token`。

## 动手练习

1. 写两个函数，一个收 `String` 返回 `String`，一个收 `&str` 返回 `&str`（或 `String`）。从调用方角度比较哪个更好用。
2. 用 `mem::replace` 实现一个循环队列的 `pop`：从头部取出元素后，用一个默认值填补位置。
3. 给你一个 `struct Request { method: String, path: String, body: Vec<u8> }`，写出四种消费方式：只借用、只消费 body、部分移动、整体消费，并说明各自 `Request` 还剩什么。
4. 用 `impl Drop` 观察下面程序的输出顺序，先猜再跑：
   ```rust
   struct A; struct B;
   impl Drop for A { fn drop(&mut self) { println!("A"); } }
   impl Drop for B { fn drop(&mut self) { println!("B"); } }
   fn main() { let _a = A; { let _b = B; } println!("--"); }
   ```
5. 解释为什么 `&mut T` 不能实现 `Copy`，而 `&T` 可以。

## 小结

- 所有权 = 编译期的、静态的内存管理：唯一所有者 + 作用域结束释放。
- `String` 的指针、长度和容量随值存放；移动 `String` 不会复制堆缓冲区。
- 移动后不能再读原位置；是否生成内存复制指令取决于类型和优化。
- `Copy` 隐式、要求按位复制安全且无 `Drop`；`Clone` 显式、可有深拷贝成本。设计 API 时让开销在调用处可见。
- 非 `Copy` 值按值传参时转移所有权；不需要拥有时用 `&T` / `&mut T`，需要消费才收 `T`。
- 部分移动后整体不可用，但未移动的字段仍可访问；`mem::take`/`replace` 是"从 `&mut` 里拿值"的标准工具。
- 变量按声明逆序 drop；`drop(x)` 可以显式提前释放。
