# 第 16 章 trait（二）：关联类型、dyn 与对象安全

> **本章目标**
>
> - 会用关联类型（associated type）表达"一个实现只产出一种类型"，并说清它和泛型参数的分工
> - 会写关联常量与 supertrait（`trait A: B`），能用 `where Self: Sized` 把方法从虚表里排除
> - 能判断一个 trait 是否支持 `dyn`（对象安全），会用 `Box<dyn Trait>` 和 `&dyn Trait` 做动态分发
> - 知道单态化与虚表调用的代价，能按场景在泛型和 `dyn` 之间选
> - 会用 newtype 绕开孤儿规则，理解 `Deref` 强制转换的解析链与滥用的后果

第 15 章把 trait 讲成了"一组方法签名的契约"：定义、实现、当约束用，这三件事撑起了大部分日常代码。但真实项目里很快就会撞上三类问题。

第一类关于**类型从哪来**。你想写一个"能产出元素"的抽象，元素类型该写成 trait 的类型参数 `Iterator<T>`，还是写成 trait 内部的占位 `Iterator` 加 `type Item`？标准库选了后者，这不是风格偏好，而是会影响调用点、类型推断和能否放进 `Box` 的设计决策。

第二类关于**运行时多态**。第 15 章末尾的 `Vec<Box<dyn Shape>>` 能跑，但你想给 trait 加一个泛型方法时，编译器会突然拒绝把它变成 `dyn`。拒绝的理由写在 vtable 的构造方式里，不理解这一点，你只能靠试错。

第三类关于**谁能实现谁**。给 `Vec<String>` 实现 `Display` 看起来天经地义，编译器却报 E0117；而两个 crate 各自写一个 `impl<T> MyTrait for T`，链接时也不会冲突——这背后是同一条规则的两面。

这一章逐个拆开。它们共同的主题是：trait 不只是"方法清单"，还是一份**在编译期被固定下来的类型级契约**。

## 16.1 关联类型：把输出类型钉在实现上

先从一个具体的需求出发。第 15 章的 `Summary` trait 返回 `String`，现在要让不同类型的摘要返回不同形态：新闻返回一段文字，天气返回一个温度数值。

最直接的想法是给 trait 加类型参数：

```rust,noplayground
trait Summary<T> {
    fn summarize(&self) -> T;
}
```

这个写法能编译，但用起来有三个问题：

1. **每个使用点都要携带 `T`**。写 `fn print_all<T, S: Summary<T>>(items: &[S])`，`T` 只是个过路参数，函数体根本不用它。
2. **同一个类型可以实现多次**。`impl Summary<String> for Report` 和 `impl Summary<f64> for Report` 可以共存，于是 `report.summarize()` 无法确定调用哪个，编译器只能报类型推断失败（E0283）。
3. **约束表达不了"由实现决定"**。你想要的是"每个实现自己挑一种输出类型"，泛型参数表达的是"调用方挑一种输出类型"，方向正好相反。

关联类型（associated type）把 `T` 从 trait 的输入挪到实现的输出：

```rust
trait Summary {
    type Output;
    fn summarize(&self) -> Self::Output;
}

struct NewsArticle {
    title: String,
    body: String,
}

impl Summary for NewsArticle {
    type Output = String;
    fn summarize(&self) -> String {
        let excerpt: String = self.body.chars().take(20).collect();
        format!("{}: {}", self.title, excerpt)
    }
}

struct WeatherReport {
    temperature: f64,
}

impl Summary for WeatherReport {
    type Output = f64;
    fn summarize(&self) -> f64 {
        self.temperature
    }
}

fn main() {
    let article = NewsArticle {
        title: String::from("Rust 1.0"),
        body: String::from("Rust 发布了 1.0 版本，这是系统编程的一件大事。"),
    };
    let report = WeatherReport { temperature: 23.5 };
    println!("{}", article.summarize());
    println!("{:.1}", report.summarize());
}
```

`type Output = String;` 这行只在 `impl` 里出现一次，之后所有 `Self::Output` 在编译器眼里都等价于 `String`。语法上它长得像类型别名，语义上它是**实现的一部分**：换个实现，这个"字段"的值就变了。

`Self::Output` 这种写法叫关联类型投影（associated type projection）。在泛型代码里你也可以写 `S::Output`；当编译器分不清是哪个 trait 的关联类型时，用完整形式 `<S as Summary>::Output` 消歧。**一个类型对同一个 trait 只能有一个 impl**，所以关联类型对每个实现只能取一个值。这条限制看似不方便，其实正是"每个迭代器只产出一种元素"的语义保证。

### 为什么 `Iterator` 用关联类型而不是泛型

标准库的定义是：

```rust,noplayground
pub trait Iterator {
    type Item;

    fn next(&mut self) -> Option<Self::Item>;
}
```

如果改成 `Iterator<T>`，会连锁地出问题。看一个适配器的签名：

```rust,noplayground
fn map<B, F>(self, f: F) -> Map<Self, F>
where
    F: FnMut(Self::Item) -> B;
```

`Self::Item` 在这里是一个确定的位置。换成泛型参数，每个用到迭代器的地方都得把元素类型再传播一遍，`for` 循环、`.collect()`、`zip` 的组合会变成类型参数的迷宫。

更根本的是**唯一性**。用关联类型时，`Counter` 是"产出 `u32` 的迭代器"这件事写死在 impl 里；用泛型时，同一个 `Counter` 可以同时实现 `Iterator<u8>` 和 `Iterator<String>`，那么 `counter.collect()` 要收集成 `Vec<u8>` 还是 `Vec<String>` 就取决于上下文推断——推断失败时要手写 turbofish，代码会变得很难读。

| 维度 | 关联类型 `trait Trait { type Out; }` | 泛型参数 `trait Trait<T>` |
| --- | --- | --- |
| 谁决定类型 | 实现方，每个类型只能选一个 | 调用方，可以选多个 |
| 调用点写法 | `x.method()`，无需标注 | 常要标注 `x.method::<T>()` 或靠推断 |
| 同一类型多份实现 | 不允许 | 允许，但可能推断歧义 |
| 能否直接做 trait 对象 | `dyn Trait<Out = u8>` | `dyn Trait<u8>` |
| 适合表达 | "这个类型的产物/错误/键是什么" | "这个类型能处理哪些输入" |

选择判据很直接：**如果每个实现只应该有一种答案，用关联类型；如果调用方需要为同一类型选不同参数，用泛型参数**。`Iterator`、`Add`（`type Output`）、`Deref`（`type Target`）、`From<T>` 的 `T` 都是这个判据的产物：`From` 允许一个类型从多种类型转换而来，所以 `T` 必须是泛型参数。

还要注意一个组合限制：**关联类型必须在 trait 对象类型里被指定**。写 `Box<dyn Iterator>` 编译不过，必须写 `Box<dyn Iterator<Item = u8>>`。原因同样在 vtable 里——vtable 的函数指针签名要按 `Item` 的具体类型生成，不指定就没法生成。

## 16.2 关联常量与 supertrait

关联类型之外，trait 里还能放常量：

```rust
trait Shape {
    const SIDES: u32;
    fn area(&self) -> f64;
}

struct Circle {
    radius: f64,
}

impl Shape for Circle {
    const SIDES: u32 = 1;
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.radius * self.radius
    }
}

fn sides_of<T: Shape>(_: &T) -> u32 {
    T::SIDES
}

fn main() {
    println!("{:.2}", sides_of(&Circle { radius: 2.0 }));
}
```

关联常量的价值和关联类型同源：它让"每个实现自带的固定参数"跟着类型走，而不是跟着实例走。在泛型函数里写 `T::SIDES`，单态化之后就是一个字面量，不产生函数调用，也不占结构体空间。

它和普通 `const` 的区别在**访问路径**。`Circle::SIDES` 这种写法用普通 `impl` 块也能做到，但泛型代码里的 `T::SIDES` 必须由 trait 声明，否则编译器不知道 `T` 有没有这个常量。协议版本号、缓冲区大小、默认端口这类"类型自带的魔数"适合放这里。

### supertrait：给实现加前置条件

`trait Shape: std::fmt::Debug` 里的 `: std::fmt::Debug` 叫 supertrait。它的含义是：**任何实现了 `Shape` 的类型，必须先实现 `Debug`**。

```rust
trait Shape: std::fmt::Debug {
    fn area(&self) -> f64;
    fn describe(&self) -> String {
        format!("{:?} -> {:.2}", self, self.area())
    }
}

#[derive(Debug)]
struct Square {
    side: f64,
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn main() {
    println!("{}", Square { side: 3.0 }.describe());
}
```

三件事值得注意：

- 默认方法 `describe` 里能直接对 `self` 用 `{:?}`，因为 `Self: Debug` 是 `Shape` 的前置条件。
- 如果 `Square` 没写 `#[derive(Debug)]`，报错会落在 `impl Shape for Square` 这一行，而不是 `describe` 里——编译器检查的是"实现 `Shape` 的前置条件是否满足"。
- supertrait **不是继承**。它不传递字段，也不传递实现，只是把额外约束写进 trait 的签名；实现方仍要自己写 `impl Shape`。

多个 supertrait 用 `+` 连接，最常见的组合是 `trait Repo: Send + Sync`：它保证任何 `Repo` 实现都能跨线程共享，于是持有 `Box<dyn Repo>` 的代码不用逐处标注 `Send + Sync`。

### `where Self: Sized`：把方法从虚表里排除

默认方法如果用了泛型参数，或者返回 `Box<Self>` 这类"尺寸依赖具体类型"的东西，就会让 trait 无法变成 `dyn`。给这个方法加 `where Self: Sized`，编译器就把它从 vtable 中排除：具体类型照样能调用，`dyn Trait` 上调不到。

```rust
trait Shape {
    fn area(&self) -> f64;
    fn clone_boxed(&self) -> Box<dyn Shape>
    where
        Self: Sized + Clone + 'static,
    {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
struct Square {
    side: f64,
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn main() {
    println!("{}", Square { side: 4.0 }.clone_boxed().area());
}
```

代价是：**这个方法的调用点必须坐落在具体类型上**。`shapes[0].clone_boxed()`（`shapes: Vec<Box<dyn Shape>>`）编译不过，报错原文是 `the method cannot be invoked on a trait object`。标准库的 `Iterator::count`、`Iterator::last`、`Iterator::collect` 都带 `Self: Sized`，因为它们要把迭代器按值消费掉，而 `dyn Iterator` 是无尺寸类型，没法按值传递。

## 16.3 trait 对象：`dyn`、胖指针与对象安全

`&dyn Shape` 和 `&Shape` 的区别，本质是"编译期确定"和"运行期查表"的区别。先看内存布局：

```text
        &dyn Shape（16 字节）
        +----------------+----------------+
        | data 指针       | vtable 指针     |
        +-------+--------+--------+-------+
                |                 |
                v                 v
          Square 实例        +------------------+
          [side: f64]        | drop_in_place    |
                             | size             |
                             | align            |
                             | area  ------------> Square::area
                             +------------------+
```

普通引用是一个机器字，trait 对象的引用是**两个**：一个指向数据，一个指向虚表（vtable）。虚表由编译器为"每个具体类型 × 每个 trait"生成一份，里面按固定顺序放着析构函数、尺寸、对齐和各个方法的函数指针。调用 `shape.area()` 时，机器指令是"从 vtable 的第 N 个槽取出地址，间接跳转"——这就是动态分发（dynamic dispatch）。

因为多了一个指针，`size_of::<&dyn Shape>()` 是 `16`（64 位平台），而 `size_of::<&Square>()` 是 `8`。这个开销在单个对象上微不足道，在百万级元素的集合里值得注意，第 20 章会算这笔账。

### 什么样的 trait 能做 `dyn`

不是所有 trait 都能变成 trait 对象。规则叫**对象安全**（object safety），新版文档改称 **dyn 兼容**（dyn compatibility），四条主要限制：

1. 方法不能带泛型参数（`fn f<T>(&self)` 不行）。
2. 方法不能返回 `Self`，也不能把 `Self` 放在参数位置按值传递（`fn clone(&self) -> Self` 不行）。
3. 不能有 `where Self: Sized` 之外依赖具体尺寸的用法；关联常量在 trait 对象上不可访问。
4. 接收者必须是 `&self`、`&mut self`、`Box<Self>`、`Rc<Self>` 等可分发形式，不能是裸 `self`。

原因都指向同一件事：**vtable 必须是一张大小固定的函数指针表**。泛型方法意味着对每组类型参数都要生成新函数，表里放不下无限多种；返回 `Self` 则要求调用方知道具体类型有多大，而 `dyn` 的整个意义就是"不知道"。

违反规则的报错长这样：

```rust,noplayground
trait Store {
    fn get(&self, key: &str) -> Option<String>;
    fn get_or<T: Default>(&self, key: &str) -> T;
}

fn use_store(s: &dyn Store) {
    println!("{:?}", s.get("k"));
}

fn main() {
    let _ = use_store;
}
```

```text
error[E0038]: the trait `Store` is not dyn compatible
 --> src/main.rs:6:18
  |
6 | fn use_store(s: &dyn Store) {
  |                  ^^^^^^^^^ `Store` is not dyn compatible
  |
note: for a trait to be dyn compatible it needs to allow building a vtable
 --> src/main.rs:3:8
  |
3 |     fn get_or<T: Default>(&self, key: &str) -> T;
  |        ^^^^^^ ...because method `get_or` has generic type parameters
  = help: consider moving `get_or` to another trait
```

编译器甚至给了修法：把泛型方法挪到另一个 trait，或者给它加 `where Self: Sized`。这两种改法对应两种设计——要么承认"这个方法只能在具体类型上用"，要么承认"这个 trait 分成两部分"。

### 用 `Box<dyn Trait>` 装异构集合

把第 15 章的 `Repo` 升级成带关联类型的版本，就能同时演示关联类型与 trait 对象的配合：

```rust
struct Book {
    title: String,
    pages: u32,
}

trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
    fn insert(&mut self, id: Self::Id, item: T);
}

struct MemoryRepo {
    books: Vec<(u64, Book)>,
}

impl Repo<Book> for MemoryRepo {
    type Id = u64;
    fn get(&self, id: u64) -> Option<&Book> {
        self.books.iter().find(|(k, _)| *k == id).map(|(_, b)| b)
    }
    fn insert(&mut self, id: u64, item: Book) {
        self.books.push((id, item));
    }
}

fn main() {
    let mut repo = MemoryRepo { books: Vec::new() };
    repo.insert(1, Book { title: String::from("Rust"), pages: 500 });

    let mut dynamic: Box<dyn Repo<Book, Id = u64>> = Box::new(repo);
    dynamic.insert(2, Book { title: String::from("Cargo"), pages: 200 });

    for id in [1, 2, 3] {
        match dynamic.get(id) {
            Some(book) => println!("{}: {} 页", book.title, book.pages),
            None => println!("{} 号不存在", id),
        }
    }
}
```

两个细节值得记：

- 类型写法是 `dyn Repo<Book, Id = u64>`，泛型参数用位置、关联类型用 `名字 = 类型`。关联类型不写就报 E0191（下一节第一个案例）。
- `Box<dyn Repo<...>>` 拥有数据，所以能调用 `&mut self` 的 `insert`；如果是 `&dyn Repo<...>`，只能调用 `&self` 的方法。`Box` 在这里不只是"装箱"，它同时解决了 trait 对象无尺寸、不能按值传递的问题。

### 泛型还是 `dyn`

泛型在编译期单态化，为每个具体类型生成一份代码，调用是直接的、可内联的，代价是代码体积和编译时间；`dyn` 只有一份代码，每次调用多一次查表和间接跳转。判据：类型集合在编译期固定、调用在热路径上，用泛型；类型要到运行期才能确定（插件、异构集合、trait 对象字段），或者二进制体积敏感，用 `dyn`。

## 16.4 为新类型实现外部 trait

你想给 `Vec<String>` 实现 `Display`，好让它按行输出：

```rust,noplayground
use std::fmt;

struct Lines(Vec<String>);

impl fmt::Display for Vec<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.join(","))
    }
}

```

```text
error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate
 --> src/main.rs:5:1
  |
5 | impl fmt::Display for Vec<String> {
  | ^^^^^^^^^^^^^^^^^^^^^^-----------
  |                       |
  |                       `Vec` is not defined in the current crate
  |
  = note: define and implement a trait or new type instead
```

这条规则叫**孤儿规则**（orphan rule）：一个 `impl` 要么 trait 属于当前 crate，要么类型属于当前 crate，两者都来自外部就不允许。它是一致性（coherence）在跨 crate 场景下的投影——如果两个 crate 都能给 `Vec<String>` 实现 `Display`，把这两个 crate 同时引入的程序就会出现两份实现，编译器无法选择；更隐蔽的是，上游 crate 某天新增一个 impl，会静默地破坏下游已经能编译的代码。

标准库给出的出路就是最后那句提示：**定义一个新类型**（newtype）。

```rust
use std::fmt;

struct Lines(Vec<String>);

impl fmt::Display for Lines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.0 {
            writeln!(f, "{}", line)?;
        }
        Ok(())
    }
}

fn main() {
    let lines = Lines(vec![String::from("alpha"), String::from("beta")]);
    print!("{}", lines);
}
```

`Lines` 是本地类型，所以 `impl fmt::Display for Lines` 合法。这就是第 8 章提过的 newtype 手法在 trait 层面的用途：**包装一层，把"外部的类型"变成"你的类型"**，顺便获得一个可以挂语义的名字。代价是访问内部值要多写一层 `.0`，以及跨 crate 传递时对方得认识这个包装类型。如果你嫌 `.0` 麻烦，可以给 `Lines` 实现 `Deref<Target = Vec<String>>`，让它的方法"看起来"和 `Vec` 一样。这招能用，但下一节会说明它的边界。

## 16.5 深入：`Deref` 与自动解引用链

`Deref` 的定义里用的正是本章的关联类型：

```rust,noplayground
pub trait Deref {
    type Target: ?Sized;

    fn deref(&self) -> &Self::Target;
}
```

注意返回的是**引用**，不是值。这决定了 `Deref` 只能"换一个角度看同一块数据"，不能搬走数据。

实现 `Deref` 之后，编译器在三个地方自动插入 `deref()`：

1. **强制转换（coercion）**：`&T` 能当 `&U` 用，只要 `T: Deref<Target = U>`。所以 `&String` 能传给要 `&str` 的函数。
2. **方法调用**：`s.len()` 在 `String` 上找不到 `len` 时，会自动尝试 `str`。
3. **索引与字段访问**：`s[0]` 会先解引用到目标类型再索引。

方法解析的完整顺序是：对接收者类型 `T` 反复解引用，得到 `T, *T, **T, ...` 一串候选；对每个候选依次尝试按值、`&T`、`&mut T` 三种接收者。找到第一个匹配就停。所以 `&&String` 上调 `len` 也能成功，编译器会一路剥到 `str`。

```rust
use std::ops::{Deref, DerefMut};

struct Stack(Vec<i32>);

impl Deref for Stack {
    type Target = [i32];

    fn deref(&self) -> &[i32] {
        &self.0
    }
}

impl DerefMut for Stack {
    fn deref_mut(&mut self) -> &mut [i32] {
        &mut self.0
    }
}

fn sum(xs: &[i32]) -> i32 {
    xs.iter().sum()
}

fn main() {
    let mut s = Stack(vec![3, 1, 2]);
    println!("{}", sum(&s));
    println!("{}", s.len());
    s.sort();
    println!("{:?}", &s[..]);
}
```

`sum(&s)` 能通过，是因为 `&Stack` 被强制转换成 `&[i32]`；`s.len()` 走的是方法解析；`s.sort()` 需要 `DerefMut`，因为它要拿 `&mut [i32]`。

### 可变的那一半：`DerefMut`

`Deref` 只提供 `&Target`。想通过包装类型修改内部值，必须实现 `DerefMut`，它提供 `&mut Target`。这条区分很重要：**没有 `DerefMut` 时，`&mut T` 不会自动变成 `&mut U`**，即使 `T: Deref<Target = U>`。反过来，`DerefMut` 是把双刃剑。实现了它，任何持有 `&mut Lines` 的代码都能拿到 `&mut Vec<String>`，于是能 `push`、`clear`、换成任意内容——你在 `Lines` 上建立的"每行一个元素"的约束全部失效。

### 什么时候不该实现 `Deref`

`Deref` 的语义是"解引用后得到我代表的那个东西"，它是给智能指针和透明包装准备的（`Box<T>`、`Rc<T>`、`String` 到 `str`），不是给"继承"准备的。想让 `Lines` 拥有 `Vec` 的全部方法，正确的做法是显式转发你需要的那几个方法，而不是把整个内部 API 暴露出去。判断标准很简单：**如果包装类型加了不变量（比如"每个元素不含换行"），就不要实现 `DerefMut`**。

## 16.6 深入：一致性、blanket impl 与 `#[fundamental]`

16.4 的孤儿规则只是一致性（coherence）的一个特例。完整规则是：**对任意一对 (trait, 类型)，整个程序里最多只能存在一个 impl**。编译器在编译每个 crate 时都要能独立判断这条规则，不能等到链接期——否则增量编译、动态库、泛型单态化都没法工作。

这条规则解释了一类让人意外的报错。标准库里有 `impl<T> From<T> for T`（任何类型都能从自己转换而来），于是你写 `impl From<MyType> for MyType` 会直接冲突，尽管那个 impl 看起来什么都没做。

更大范围的问题是 **blanket impl**（毯式实现）：

```rust,noplayground
impl<T: Display> Label for T {
    fn label(&self) -> String {
        format!("{}", self)
    }
}
```

这一行把"所有实现了 `Display` 的类型"一次性占满。之后无论你还是下游用户，想给 `String`、`i32` 或任何实现了 `Display` 的类型单独写一个更合适的 `Label`，都会得到 E0119（本节的最后一个案例）。blanket impl 的诱惑很大——一行代码换来全类型支持——但它等于宣布"这个 trait 对这些类型的行为从此固定"，没有例外、无法特化（specialization 至今未稳定）。

最后一个细节是 `#[fundamental]`。孤儿检查默认要求"本地类型出现在 impl 的类型头里"，但有些类型被编译器标记为**基本类型**，会让内部的本地类型"穿透"出来：`&T`、`&mut T`、`Box<T>`、`Pin<P>`。所以 `impl ForeignTrait for &LocalType` 是合法的，即使 `&` 来自标准库；同理，本地 trait 也能为本地的引用类型实现，比如 `impl Shape for &Square` 一样合法。这个属性只对标准库开放，你无法给自己的类型加——否则一致性检查就可以被用户绕过。

## 典型错误案例

### 案例 1：`dyn` 里漏写关联类型——E0191

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
}

fn total(r: &dyn Repo<String>) -> usize {
    let _ = r;
    0
}

fn main() {}
```

```text
error[E0191]: the value of the associated type `Id` in `Repo<String>` must be specified
 --> src/main.rs:6:18
  |
2 |     type Id;
  |     ------- `Id` defined here
...
6 | fn total(r: &dyn Repo<String>) -> usize {
  |                  ^^^^^^^^^^^^
  |
help: specify the associated type
  |
6 | fn total(r: &dyn Repo<String, Id = /* Type */>) -> usize {
  |                             +++++++++++++++++
```

**原因**：trait 对象要为每个具体类型生成 vtable，而 vtable 里函数指针的签名依赖关联类型的具体取值。`dyn Repo<String>` 没说 `Id` 是什么，编译器就不知道 `get` 的参数该按 `u64` 还是 `String` 生成，于是拒绝构造这张表。注意报错落在**类型标注**上，而不是调用点。

**修复**：在 `dyn` 的类型参数里用 `关联类型名 = 具体类型` 补齐：

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
}

struct Memory;

impl Repo<String> for Memory {
    type Id = u64;
    fn get(&self, _id: u64) -> Option<&String> {
        None
    }
}

fn total(_r: &dyn Repo<String, Id = u64>) -> usize {
    0
}

fn main() {
    let m = Memory;
    println!("{}", total(&m));
}
```

### 案例 2：实现里用错关联类型的名字——E0220

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
}

struct Memory;

impl Repo<String> for Memory {
    type Id = u64;

    fn get(&self, id: Self::Key) -> Option<&String> {
        let _ = id;
        None
    }
}

fn main() {}
```

```text
error[E0220]: associated type `Key` not found for `Self`
  --> src/main.rs:11:29
   |
11 |     fn get(&self, id: Self::Key) -> Option<&String> {
   |                             ^^^ help: `Self` has the following associated type: `Id`
```

**原因**：`Self::Key` 是一次关联类型投影，编译器要在 `impl` 里找名为 `Key` 的关联类型。这个 impl 只定义了 `type Id`，而 `Repo` 也只声明了 `Id`，所以 `Key` 既不在 trait 里也不在实现里。这种情况常出现在把 `Id`、`Key`、`Item` 几个名字混用的重构之后。

**修复**：把 `Self::Key` 改成 `Self::Id`。编译器在 `help` 里已经列出了当前可用的关联类型，照着改即可：

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
}

struct Memory;

impl Repo<String> for Memory {
    type Id = u64;

    fn get(&self, id: Self::Id) -> Option<&String> {
        let _ = id;
        None
    }
}

fn main() {
    println!("{:?}", Memory.get(1));
}
```

### 案例 3：只实现了关联类型，方法没写——E0046

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
    fn len(&self) -> usize;
}

struct Memory;

impl Repo<String> for Memory {
    type Id = u64;
}

fn main() {}
```

```text
error[E0046]: not all trait items implemented, missing: `get`, `len`
 --> src/main.rs:9:1
  |
3 |     fn get(&self, id: Self::Id) -> Option<&T>;
  |     ------------------------------------------ `get` from trait
4 |     fn len(&self) -> usize;
  |     ----------------------- `len` from trait
...
9 | impl Repo<String> for Memory {
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ missing `get`, `len` in implementation
```

**原因**：`impl` 块只填了 `type Id`。关联类型不是"可选的补充"，它和方法一样是 trait 的成员；只要 trait 里没有默认实现，impl 就必须全部给出。编译器会把缺的成员列在第一个 `error` 行里，这个列表通常就是你要补的清单。

**修复**：补上 `get` 和 `len`：

```rust,editable
trait Repo<T> {
    type Id;
    fn get(&self, id: Self::Id) -> Option<&T>;
    fn len(&self) -> usize;
}

struct Memory;

impl Repo<String> for Memory {
    type Id = u64;
    fn get(&self, _id: u64) -> Option<&String> {
        None
    }
    fn len(&self) -> usize {
        0
    }
}

fn main() {
    println!("{}", Memory.len());
}
```

### 案例 4：实现的方法签名和 trait 不一致——E0053

```rust,editable
trait Shape {
    fn area(&self) -> f64;
}

struct Square {
    side: f64,
}

impl Shape for Square {
    fn area(&self) -> u32 {
        (self.side * self.side) as u32
    }
}

fn main() {}
```

```text
error[E0053]: method `area` has an incompatible type for trait
  --> src/main.rs:10:23
   |
10 |     fn area(&self) -> u32 {
   |                       ^^^ expected `f64`, found `u32`
   |
note: type in trait
  --> src/main.rs:2:23
   |
 2 |     fn area(&self) -> f64;
   |                       ^^^
```

**原因**：impl 里的方法必须和 trait 声明**逐字匹配**：参数类型、返回类型、接收者形式、泛型参数数量都要一致。这里 `u32` 和 `f64` 在 Rust 里没有隐式转换，编译器不会替你插入 `as f64`，于是直接判定签名不符。

**修复**：把返回类型改成 `f64`，转换放在函数体里：

```rust,editable
trait Shape {
    fn area(&self) -> f64;
}

struct Square {
    side: f64,
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn main() {
    println!("{}", Square { side: 2.0 }.area());
}
```

### 案例 5：blanket impl 和具体实现打架——E0119

```rust,editable
use std::fmt::Display;

trait Label {
    fn label(&self) -> String;
}

impl<T: Display> Label for T {
    fn label(&self) -> String {
        format!("{}", self)
    }
}

impl Label for String {
    fn label(&self) -> String {
        format!("str: {}", self)
    }
}

fn main() {}
```

```text
error[E0119]: conflicting implementations of trait `Label` for type `String`
  --> src/main.rs:13:1
   |
 7 | impl<T: Display> Label for T {
   | ---------------------------- first implementation here
...
13 | impl Label for String {
   | ^^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `String`
```

**原因**：`String` 实现了 `Display`，所以它已经落进第一个 blanket impl 的覆盖范围。再写一个专门给 `String` 的 impl，就出现同一个 (trait, 类型) 的两份实现，违反一致性。编译器只能拒绝，而且拒绝得对：它无法判断你想用哪一份。

**修复**：两者只能留一个。放弃定制、保留 blanket：

```rust,editable
trait Label {
    fn label(&self) -> String;
}

impl<T: std::fmt::Display> Label for T {
    fn label(&self) -> String {
        format!("{}", self)
    }
}

fn main() {
    println!("{}", 42.label());
    println!("{}", "hi".label());
}
```

如果 `String` 的行为必须不同于其它类型，就删掉 blanket impl，给每种类型写具体实现，或者给 `String` 套一个 newtype 再为 newtype 实现 `Label`。

## 动手练习

**练习 1**（基础）：给 16.2 的 `Shape` 增加关联常量 `SIDES`，写 `fn total_sides<T: Shape>(items: &[T]) -> u32` 求和。然后试着把这个函数改成接收 `&[Box<dyn Shape>]`，解释为什么不能通过编译。

**练习 2**（基础）：实现 `Iterator for Fibonacci`，`type Item = u64`，让 `Fibonacci::new().take(10).collect::<Vec<_>>()` 输出前十个斐波那契数。注意 `next` 返回 `Option<Self::Item>`，溢出用 `checked_add` 处理。

**练习 3**（进阶）：给 `Repo<Book>` 再写一个按标题排序的 `SortedRepo`，然后用 `Vec<Box<dyn Repo<Book, Id = u64>>>` 同时装 `MemoryRepo` 和 `SortedRepo`，遍历调用 `get`。思考：为什么 `insert` 必须通过 `&mut dyn` 调用。

**练习 4**（进阶）：用 newtype 包装 `HashMap<String, u32>`，为它实现 `Display`，按 key 字典序输出 `key=value`。先直接给 `HashMap` 实现一次，确认报 E0117，再改成 newtype 版本。

**练习 5**（挑战）：给 16.5 的 `Stack` 实现 `DerefMut`，然后写代码用 `clear()` 把内部 `Vec` 清空，绕过"只能通过 push 添加"的约定。再写一个不实现 `DerefMut` 的版本，对比两者在 API 约束上的差别，并说明 `Deref` 适合什么样的包装类型。

## 小结

- 关联类型表达"每个实现只产出一种类型"，泛型参数表达"调用方可以选多种输入"；`Iterator` 选前者是为了唯一性和调用点简洁
- `dyn Trait` 里必须写全关联类型（`dyn Iterator<Item = u8>`），否则报 E0191，因为 vtable 要按具体类型生成
- 关联常量随类型走，在泛型代码里写 `T::SIDES`，单态化后是字面量
- supertrait 是前置条件而非继承：`trait A: B` 要求实现方先实现 `B`
- `where Self: Sized` 把方法从 vtable 排除，保住 trait 的对象安全，代价是该方法在 `dyn` 上调不到
- trait 对象是胖指针（数据 + vtable），对象安全要求方法无泛型参数、不返回 `Self`、接收者可分发
- 孤儿规则：trait 和类型至少一个属于当前 crate；给外部类型实现外部 trait 要用 newtype 包装
- `Deref` 只提供 `&Target`，`DerefMut` 才允许可变穿透；包装类型带不变量时不要实现 `DerefMut`
- 一致性保证一个 (trait, 类型) 只有一份 impl；blanket impl 会占满所有匹配类型，让下游无法再定制
- `&T`、`Box<T>` 等 `#[fundamental]` 类型会让内部的本地类型穿透孤儿检查
