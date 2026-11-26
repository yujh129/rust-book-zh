# 第 14 章 泛型：一份代码，多种类型

> **本章目标**
>
> - 会用泛型函数、泛型结构体、泛型枚举消除重复代码，而不是复制粘贴
> - 掌握 trait bound 的三种写法（`<T: Trait>`、`where` 从句、返回位置 `impl Trait`）
> - 理解"泛型函数不是万能的"——只声明 `<T>` 时仍可移动和返回值，调用 trait 方法则需要相应约束
> - 知道单态化（monomorphization）如何把泛型变成具体代码，以及它的代价
> - 能区分 `impl Trait` 与 `dyn Trait` 的静态分派/动态分派

第 8 章我们用结构体把数据组织起来，但那时每个结构体的字段类型都是写死的：`Rectangle { width: u32 }`。如果明天要一个 `f64` 版本的矩形，你得再定义一遍，或者把 `u32` 改成 `f64` 全量改一遍。泛型就是解决"类型不同、逻辑相同"这个问题的工具——它把**类型本身**变成参数。

## 14.1 从两段几乎一样的代码说起

先看一个真实场景：找出一组数里的最大值。

```rust
fn largest_i32(list: &[i32]) -> &i32 {
    let mut largest = &list[0];
    for item in list {
        if item > largest {
            largest = item;
        }
    }
    largest
}

fn largest_char(list: &[char]) -> &char {
    let mut largest = &list[0];
    for item in list {
        if item > largest {
            largest = item;
        }
    }
    largest
}

fn main() {
    let numbers = vec![34, 50, 25, 100, 65];
    println!("最大数：{}", largest_i32(&numbers));

    let chars = vec!['y', 'm', 'a', 'q'];
    println!("最大字符：{}", largest_char(&chars));
}
```

这两个函数**逐字相同**，只有类型标注不一样。复制粘贴的后果是：逻辑改一处要改两处，第三处忘了改就埋 bug。泛型把"类型"抽出来当参数：

```rust
fn largest<T: PartialOrd>(list: &[T]) -> &T {
    let mut largest = &list[0];
    for item in list {
        if item > largest {
            largest = item;
        }
    }
    largest
}

fn main() {
    let numbers = vec![34, 50, 25, 100, 65];
    println!("最大数：{}", largest(&numbers));

    let chars = vec!['y', 'm', 'a', 'q'];
    println!("最大字符：{}", largest(&chars));
}
```

注意那个 `T: PartialOrd`。它不是我随手加的装饰，而是**必须的**：函数体里写了 `item > largest`，这个 `>` 需要类型支持比较。如果只写 `fn largest<T>(...)`，编译器会直接拒绝，理由见本章错误案例 1。

## 14.2 trait bound：告诉编译器"T 能做什么"

Rust 的泛型是"白盒"的：**在函数体内，你只能使用 bound 里声明的能力**。这一点和 C++ 模板完全不同——C++ 是"用到才检查"，Rust 是"定义时就检查"。

```rust
use std::fmt::Display;

// 只能调用 bound 里有的方法
fn print_twice<T: Display>(x: T) {
    println!("{}", x);
    println!("{}", x);
}

// 多个 bound 用 + 连接
fn compare_and_print<T: Display + PartialOrd>(a: T, b: T) {
    if a > b {
        println!("{} 更大", a);
    } else {
        println!("{} 更大", b);
    }
}

fn main() {
    print_twice(42);
    print_twice("文本");
    compare_and_print(3, 7);
}
```

bound 的三种写法，效果等价，选可读性最好的：

```rust
use std::fmt::Debug;

// 写法一：尖括号内联
fn dump1<T: Debug>(x: T) {
    println!("{:?}", x);
}

// 写法二：where 从句（bound 多、签名长时更清晰）
fn dump2<T>(x: T)
where
    T: Debug + Clone,
{
    let copy = x.clone();
    println!("{:?} {:?}", x, copy);
}

// 写法三：返回位置 impl Trait（只在"返回"时可用）
fn make_vec() -> impl Iterator<Item = i32> {
    (1..=3).map(|x| x * 2)
}

fn main() {
    dump1(1);
    dump2(String::from("hi"));
    for v in make_vec() {
        println!("{}", v);
    }
}
```

`where` 从句还有一个隐性好处：**它让错误信息更短**。当 bound 不满足时，rustc 会把整个签名连同 bound 一起打印出来，签名越长越难读。

## 14.3 泛型结构体与泛型枚举

类型参数不仅能用 在函数上，也能用在结构体和枚举上。标准库里最熟悉的就是 `Option<T>` 和 `Result<T, E>`：

```rust
#[derive(Debug)]
struct Point<T> {
    x: T,
    y: T,
}

#[derive(Debug)]
struct Pair<A, B> {
    first: A,
    second: B,
}

fn main() {
    let ints = Point { x: 1, y: 2 };
    let floats = Point { x: 1.5, y: 2.5 };
    let mixed = Pair { first: 1, second: "one" };

    println!("{:?} {:?} {:?}", ints, floats, mixed);
}
```

`Point<T>` 的两个字段**必须是同一个 T**。所以 `Point { x: 1, y: 1.5 }` 编译不过——想混搭就用 `Pair<A, B>`，或者定义 `Point<T, U>`。

给泛型结构体写 `impl` 时要带上类型参数：

```rust
#[derive(Debug)]
struct Point<T> {
    x: T,
    y: T,
}

impl<T> Point<T> {
    fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

// 只为"支持相加"的类型实现方法：bound 写在 impl 上
impl<T: std::ops::Add<Output = T> + Copy> Point<T> {
    fn sum(&self) -> T {
        self.x + self.y
    }
}

fn main() {
    let p = Point::new(1, 2);
    println!("{:?} sum={}", p, p.sum());

    let f = Point::new(1.5, 2.5);
    println!("{:?} sum={}", f, f.sum());
}
```

这种"给部分 `T` 单独加方法"的能力非常有用：`sum` 只对实现了 `Add` 的 `T` 存在，你写 `Point::new(String::from("a"), String::from("b")).sum()` 会在**编译期**报错，而不是运行期才炸。

还有一个常见技巧：只在某个具体类型上实现方法，实现"特化"效果（注意不是 Rust 的 `specialization` 特性，那个还不稳定）：

```rust
#[derive(Debug)]
struct Wrapper<T> {
    value: T,
}

impl<T> Wrapper<T> {
    fn value(&self) -> &T {
        &self.value
    }
}

impl Wrapper<f64> {
    fn is_positive(&self) -> bool {
        self.value > 0.0
    }
}

fn main() {
    let w = Wrapper { value: 1.5 };
    println!("{}", w.is_positive());

    let i = Wrapper { value: 1 };
    // i.is_positive();   // 编译错误：i32 版本的 Wrapper 没有这个方法
    println!("{}", i.value());
}
```

`impl Wrapper<f64>` 只给 `Wrapper<f64>` 增加方法，其他实例化看不到它。这是用类型系统做"条件 API"的标准手法。

## 14.4 impl Trait 与 dyn Trait：两条分派路线

泛型参数、`impl Trait`、`dyn Trait` 三者经常被混着用，但它们的行为差别很大。先看同一个需求的三种写法：

```rust
trait Shape {
    fn area(&self) -> f64;
}

struct Circle { r: f64 }
struct Square { side: f64 }

impl Shape for Circle {
    fn area(&self) -> f64 { 3.14159 * self.r * self.r }
}

impl Shape for Square {
    fn area(&self) -> f64 { self.side * self.side }
}

// 写法一：泛型参数（静态分派）
fn total_generic<T: Shape>(shapes: &[T]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

// 写法二：impl Trait 参数（语法糖，等价于写法一）
fn total_impl(shapes: &[impl Shape]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

// 写法三：trait object（动态分派）
fn total_dyn(shapes: &[Box<dyn Shape>]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

fn main() {
    let circles = vec![Circle { r: 1.0 }, Circle { r: 2.0 }];
    println!("{}", total_generic(&circles));
    println!("{}", total_impl(&circles));

    let mixed: Vec<Box<dyn Shape>> = vec![
        Box::new(Circle { r: 1.0 }),
        Box::new(Square { side: 3.0 }),
    ];
    println!("{}", total_dyn(&mixed));
}
```

区别在**什么时候决定调用哪个 `area`**：

| 写法 | 决定时机 | 能否混放不同类型 | 运行期开销 |
|---|---|---|---|
| `<T: Shape>` | 编译期（单态化） | 不能 | 零（可能内联） |
| `impl Shape` | 编译期（单态化） | 不能 | 零 |
| `dyn Shape` | 运行期（查 vtable） | 能 | 通常需要一次间接调用，可能妨碍内联 |

`total_generic` 和 `total_impl` 是同一份代码的两种写法，编译器会为 `Circle` 和 `Square` 各生成一份机器码。`total_dyn` 只有一份机器码，但每次调用 `area` 都要从 vtable 里取函数指针。

**选择原则**：类型集合在编译期已知（绝大多数情况）就用泛型/`impl Trait`；需要运行期决定（插件系统、异构容器、配置驱动的处理器）才用 `dyn`。

## 14.5 深入：单态化——泛型不是"运行时多态"

Rust 的泛型实现机制叫**单态化**（monomorphization）：编译器在编译期为每一个实际使用的类型参数生成一份专门的代码。换句话说，你写的泛型函数更像"代码模板"，到了机器码层面**根本不存在泛型**。

### 一个例子看清发生什么

```rust
fn identity<T>(x: T) -> T {
    x
}

fn main() {
    let a = identity(1i32);
    let b = identity(2.5f64);
    let c = identity("hello");
    println!("{} {} {}", a, b, c);
}
```

编译器展开后，机器码里等价于：

```text
fn identity_i32(x: i32) -> i32 { x }
fn identity_f64(x: f64) -> f64 { x }
fn identity_str(x: &str) -> &str { x }
```

这就是为什么泛型**零运行期开销**——没有装箱、没有类型判断、没有虚表，`identity(1)` 直接就是一次寄存器移动。

### 代价一：编译时间与二进制体积

每个不同的类型参数都会生成一份代码。`Vec<i32>`、`Vec<u64>`、`Vec<String>` 用的是三份 `Vec` 的实现。项目里类型组合多的时候，编译时间和产物体积都会明显上升。

这也是为什么标准库宁可牺牲一点可读性，也要把与类型无关的逻辑从泛型代码里拆出去：分配、扩容、指针运算这些工作与 T 的具体类型无关，只是按字节数搬运内存，所以被下沉到非泛型的内部结构里，泛型部分只负责算出"要多少字节"。

```rust
use std::mem::size_of;

fn main() {
    // 每个 T 的实例化只保留自己的 size_of 计算
    println!("u8  = {}", size_of::<u8>());
    println!("u64 = {}", size_of::<u64>());
    println!("String = {}", size_of::<String>());
}
```

经验法则：**能下沉到非泛型代码的逻辑就下沉**，这是标准库控制代码膨胀的主要手段，也是你写大型泛型库时该学的第一招。

### 代价二：与 dyn 的取舍表

| 维度 | 泛型 / impl Trait | dyn Trait |
|---|---|---|
| 分派 | 静态（可能内联） | 动态（vtable） |
| 代码体积 | 每个类型一份 | 一份 |
| 编译时间 | 随实例数增长 | 固定 |
| 异构集合 | 不支持 | 支持 |
| 对象安全要求 | 无 | 必须 object-safe |
| 典型场景 | 库的核心 API、热路径 | 插件、异构容器、依赖注入 |

### 与 Java、C++ 的对比

- **Java** 用类型擦除：`List<T>` 在运行期就是 `List`，元素都是 `Object`，所以不能 `new T[]`、不能对 `T` 做基本类型运算（要装箱）。Rust 的单态化没有这些限制，代价是代码体积。
- **C++** 模板也是编译期展开，但约束是"用到才检查"，错误信息极长；Rust 的 trait bound 在定义时就检查，报错更早、更准。
- **Go** 1.18 之后的泛型用 GC shape stenciling，介于两者之间。

## 14.6 常见的标准库泛型长什么样

读标准库源码时，你会频繁遇到下面这些形态：

```rust
use std::collections::HashMap;
use std::fmt::Debug;

fn describe<K, V>(map: &HashMap<K, V>) -> String
where
    K: Debug,
    V: Debug,
{
    map.iter()
        .map(|(k, v)| format!("{:?} => {:?}", k, v))
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() {
    let mut m = HashMap::new();
    m.insert("a", 1);
    m.insert("b", 2);
    println!("{}", describe(&m));
}
```

`HashMap<K, V, S = RandomState>` 的第三个参数是**默认类型参数**，平时不写它就用 `RandomState`（防哈希洪水攻击）。想换成确定性的哈希器（测试、可复现）时可以显式指定：

```rust
use std::collections::HashMap;
use std::collections::hash_map::RandomState;

fn main() {
    let m: HashMap<&str, i32, RandomState> = HashMap::default();
    println!("{}", m.len());
}
```

另一个常见形态是 `PhantomData<T>`：结构体里没有任何 `T` 类型的字段，却要对 `T` 做泛型。

```rust
use std::marker::PhantomData;

struct Meter;

struct Reading<T> {
    value: f64,
    _unit: PhantomData<T>,
}

fn main() {
    let r: Reading<Meter> = Reading { value: 1.5, _unit: PhantomData };
    println!("{}", r.value);
}
```

`PhantomData<T>` 占 0 字节，只影响类型检查（泛型参数、变型、auto trait 推导）。第 36 章讲类型状态模式时会用它做"编译期单位"。

## 典型错误案例

### 案例 1：泛型函数里直接比较——E0369

```rust,editable
fn largest<T>(list: &[T]) -> &T {
    let mut max = &list[0];
    for item in list {
        if item > max {
            max = item;
        }
    }
    max
}

fn main() {
    let v = vec![1, 2, 3];
    println!("{}", largest(&v));
}
```

```text
error[E0369]: binary operation `>` cannot be applied to type `&T`
 --> src/main.rs:4:17
  |
4 |         if item > max {
  |            ---- ^ --- &T
  |            |
  |            &T
  |
help: consider restricting type parameter `T` with trait `PartialOrd`
  |
1 | fn largest<T: std::cmp::PartialOrd>(list: &[T]) -> &T {
  |             ++++++++++++++++++++++

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：函数签名只声明了 `<T>`，意味着 `T` 可以是任何类型，包括不支持比较的类型。Rust 在**定义处**就检查函数体，不会等你调用时才报错。rustc 的 help 已经给出了答案。

**修复**：加 `PartialOrd` bound。注意 `&` 的问题——`item` 是 `&T`，`max` 也是 `&T`，比较的是两个引用，`T: PartialOrd` 之后 `&T` 也能比较（标准库为 `&T` 实现了 `PartialOrd`）。

```rust,editable
fn largest<T: PartialOrd>(list: &[T]) -> &T {
    let mut max = &list[0];
    for item in list {
        if item > max {
            max = item;
        }
    }
    max
}

fn main() {
    let v = vec![3, 9, 2];
    println!("{}", largest(&v));
}
```

</details>

### 案例 2：泛型值无法推断——E0282

```rust,editable
fn make<T>() -> T {
    unimplemented!()
}

fn main() {
    let x = make();
    println!("{}", x);
}
```

```text
error[E0282]: type annotations needed
 --> src/main.rs:6:9
  |
6 |     let x = make();
  |         ^
  |
help: consider giving `x` an explicit type
  |
6 |     let x: /* Type */ = make();
  |          ++++++++++++

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`make::<T>` 的返回类型由调用方决定，而 `let x = ...` 没有给出任何线索，`println!` 也不需要特定类型（任何 `Display` 都行）。编译器无法从上下文推出 `T`。

**修复**：标注类型或用 turbofish：

```rust,editable
fn make<T: Default>() -> T {
    T::default()
}

fn main() {
    let x: i32 = make();
    let y = make::<u8>();
    println!("{} {}", x, y);
}
```

</details>

### 案例 3：对泛型 T 调用不存在的方法——E0599

```rust,editable
fn get_len<T>(v: &T) -> usize {
    v.len()
}

fn main() {
    println!("{}", get_len(&"abc"));
}
```

```text
error[E0599]: no method named `len` found for reference `&T` in the current scope
 --> src/main.rs:2:7
  |
2 |     v.len()
  |       ^^^ method not found in `&T`
  |
  = help: items from traits can only be used if the type parameter is bounded by the trait
help: the following trait defines an item `len`, perhaps you need to restrict type parameter `T` with it:
  |
1 | fn get_len<T: ExactSizeIterator>(v: &T) -> usize {
  |             ++++++++++++++++++++

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`T` 没有 bound，编译器不知道它有什么方法。这条规则看起来苛刻，其实是在保护你：如果没有 bound 也能调方法，那就回到了动态类型的运行期报错。

**修复**：想接受"任何有 `len` 的东西"，最通用的是 `T: AsRef<[U]> + ?Sized`，但更简单的做法是**直接接受切片**：

```rust,editable
fn get_len<T>(v: &[T]) -> usize {
    v.len()
}

fn main() {
    let s = "abc";
    println!("{}", get_len(s.as_bytes()));
    println!("{}", get_len(&[1, 2, 3]));
}
```

注意 rustc 建议的 `ExactSizeIterator` 不是最佳答案——它只是"恰好也定义了 `len`"的 trait 之一。这类 help 有时会给出误导性的候选，判断依据还是要回到"我真正想约束什么"。

</details>

### 案例 4：`impl` 上漏了类型参数——E0425 + E0107

```rust,editable
struct Wrapper<T> {
    value: T,
}

impl Wrapper {
    fn get(&self) -> &T {
        &self.value
    }
}

fn main() {
    let w = Wrapper { value: 1 };
    println!("{}", w.get());
}
```

```text
error[E0425]: cannot find type `T` in this scope
 --> src/main.rs:6:23
  |
6 |     fn get(&self) -> &T {
  |                       ^ not found in this scope

error[E0107]: missing generics for struct `Wrapper`
 --> src/main.rs:5:6
  |
5 | impl Wrapper {
  |      ^^^^^^^ expected 1 generic argument
  |
note: struct defined here, with 1 generic parameter: `T`
 --> src/main.rs:1:8
  |
1 | struct Wrapper<T> {
  |        ^^^^^^^^^
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`impl` 块有自己的类型参数列表，它**不会**从结构体定义里自动继承。所以 `impl Wrapper` 里的 `T` 未定义，同时 `Wrapper` 缺少参数。

**修复**：写成 `impl<T> Wrapper<T>`：

```rust,editable
struct Wrapper<T> {
    value: T,
}

impl<T> Wrapper<T> {
    fn get(&self) -> &T {
        &self.value
    }
}

fn main() {
    let w = Wrapper { value: 1 };
    println!("{}", w.get());
}
```

顺带一提：如果只想给 `Wrapper<i32>` 加方法，写 `impl Wrapper<i32>`（不带 `<T>`），这时不需要类型参数，因为没有泛型变量要声明。

</details>

### 案例 5：bound 不满足——E0277

```rust,editable
fn print_it<T: std::fmt::Display>(x: T) {
    println!("{}", x);
}

fn main() {
    print_it(vec![1, 2, 3]);
}
```

```text
error[E0277]: `Vec<{integer}>` doesn't implement `std::fmt::Display`
 --> src/main.rs:6:14
  |
6 |     print_it(vec![1, 2, 3]);
  |     -------- ^^^^^^^^^^^^^ the trait `std::fmt::Display` is not implemented for `Vec<{integer}>`
  |     |
  |     required by a bound introduced by this call
  |
note: required by a bound in `print_it`
 --> src/main.rs:1:16
  |
1 | fn print_it<T: std::fmt::Display>(x: T) {
  |                ^^^^^^^^^^^^^^^^^ required by this bound in `print_it`

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Vec<T>` 没有实现 `Display`（因为"怎么显示一个向量"没有唯一答案：`[1, 2, 3]`？`1 2 3`？）。它实现的是 `Debug`。

**修复**：换成 `Debug` bound，或者传一个已经 `Display` 的值：

```rust,editable
fn print_it<T: std::fmt::Debug>(x: T) {
    println!("{:?}", x);
}

fn main() {
    print_it(vec![1, 2, 3]);
    print_it("普通字符串也实现了 Debug");
}
```

</details>

## 动手练习

**练习 1**：写一个泛型函数 `fn largest_by<T, K, F>(items: &[T], key: F) -> Option<&T>`：`F: Fn(&T) -> K`，`K: Ord`，返回按 key 最大的元素。用它对 `&[("a", 3), ("b", 9)]` 和 `&[1.5, 2.5]` 各调一次。体会为什么返回 `Option<&T>` 而不是 `&T`（空切片怎么办）。

**练习 2**：定义一个 `struct Stack<T>`，实现 `push`、`pop`、`peek`、`len`。然后只给 `Stack<i32>` 加一个 `sum(&self) -> i32` 方法，验证 `Stack<String>` 调用 `sum` 会编译失败，并读懂报错。

**练习 3**：下面的函数为什么编译不过？改对之后，再解释 `?Sized` 在这里起了什么作用。

```rust,noplayground
fn first_char<T: AsRef<str>>(s: T) -> char {
    s.as_ref().chars().next().unwrap_or(' ')
}
```

**练习 4**：把第 13 章的迭代器适配器思想用起来，写一个泛型 `fn chunk_by<T, F>(items: &[T], same: F) -> Vec<Vec<&T>>`，其中 `F: Fn(&T, &T) -> bool`，把相邻且 `same` 返回 true 的元素分到一组。

**练习 5**：用 `Instant` 测一下泛型与 `dyn` 的性能差：写 `fn sum_generic<T: Iterator<Item = i64>>(it: T) -> i64` 和 `fn sum_dyn(it: Box<dyn Iterator<Item = i64>>) -> i64`，各跑一千万次累加，记录耗时。注意用 `std::hint::black_box` 防止被优化掉。

## 小结

- 泛型把"类型"变成参数，消除"同逻辑、不同类型"的复制粘贴
- 泛型是白盒的：函数体内只能使用 bound 声明过的能力，即使没有显式 bound，也可以移动、借用或直接返回 `T`
- bound 三种写法：`<T: Trait>`、`where T: Trait`、返回位置 `impl Trait`；`where` 在 bound 多时更清晰
- `impl<T> Wrapper<T>` 里的 `<T>` 不能省，`impl` 块不继承结构体的类型参数
- `impl Wrapper<f64>` 只给特定实例化加方法，是"条件 API"的标准手法
- 泛型用单态化实现：每个具体类型一份机器码，因此零运行期开销，但会增大二进制体积和编译时间
- `dyn Trait` 用 vtable 动态分派，一份代码支持异构集合，通常需要间接调用；优化器若能确定目标，有机会消去这次分派
- 需要 `new T[]` 式的"运行时才知道类型"时用 `dyn`；其余情况优先泛型
- `PhantomData<T>` 让结构体在没有 `T` 字段时也能泛型，且不占空间
