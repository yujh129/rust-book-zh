# 第 15 章 trait（一）：定义、实现与约束

> **本章目标**
>
> - 会定义一个 trait，并用 `impl Trait for Type` 为任意多个类型实现它
> - 会给 trait 写默认方法，并说清默认方法为什么不能访问实现类型的字段
> - 会用 `impl Trait`、泛型参数和 `dyn Trait` 三种方式接收“实现了某 trait 的值”，并按场景选型
> - 能画出 `&dyn Trait` 的胖指针与 vtable 布局，说清一次动态调用的实际开销
> - 能判断一个 trait 能不能变成 trait 对象，并解释孤儿规则挡住了哪类 `impl`

第 14 章的泛型函数签名里已经出现过 `T: PartialOrd` 这样的约束。那时你是 trait 的使用者：标准库定义好 `PartialOrd`，你拿它来限制 `T`。本章回答另一半问题——trait 由谁定义、怎么定义、定义完之后编译器拿它做什么。

假设要写一个绘图程序，圆形和矩形都要算面积。不用 trait 的写法是给每种类型配一个函数；调用方必须知道具体类型，也没法写“对一批图形求和”。用枚举可以解决一部分，但它把类型集合钉死在定义处：库作者没法预知用户会定义什么图形。

trait 是 Rust 对“共享行为”的建模方式。它只描述一个类型能做什么，不描述它由什么组成。这条界线让 Rust 在没有继承的前提下表达多态：数据靠组合，行为靠 trait。本章先把定义、默认方法和两种使用方式讲透，再往底层看一次 vtable，最后解释孤儿规则这条看似武断的限制。

## 15.1 为什么需要 trait：一种能力一个函数走不通

```rust
struct Circle {
    r: f64,
}

struct Square {
    side: f64,
}

fn circle_area(c: &Circle) -> f64 {
    3.14159 * c.r * c.r
}

fn square_area(s: &Square) -> f64 {
    s.side * s.side
}

fn main() {
    let c = Circle { r: 1.0 };
    let s = Square { side: 2.0 };
    println!("{:.2} {:.2}", circle_area(&c), square_area(&s));
}
```

这段代码能跑，但每加一种图形就要加一个函数；每加一种操作（周长、绘制、缩放），函数数量按“类型 × 操作”增长。更麻烦的是调用方：想写一个对一批图形求总面积的函数，签名写不出来——`&[Circle]` 和 `&[Square]` 之间没有共同类型。

问题可以归成三条：

1. **没有共同类型**。`circle_area` 与 `square_area` 之间没有编译器认可的共性，没法给 `fn total` 写出一个既能收 `Circle` 又能收 `Square` 的参数类型。
2. **分派逻辑散落在调用方**。一旦有了混合容器，调用方得写 `match` 判断“这是圆还是方”，每加一种类型就要改所有 `match`。
3. **命名膨胀**。`circle_area`、`square_area`、`triangle_area`，每加一种图形就多一个名字，语义却只是“求面积”。

枚举能消掉第三条。把两种图形合成一个 `enum Shape { Circle { r: f64 }, Square { side: f64 } }`，`area` 只留一份 `match self` 实现，`Vec<Shape>` 可以混放；编译器还会做穷尽性检查：将来加一个 `Triangle` 变体，所有漏掉它的 `match` 都编译失败，逼你处理。

但枚举有两个硬限制。**第一，它是封闭的。** 变体列表在定义 `enum` 的那个 crate 里就固定了，用户想给你的图形库加一种多边形，只能改你的源码。**第二，类型和行为被绑死。** 每加一个操作（`perimeter`、`draw`、`scale`），都要回到同一个 `impl Shape` 里加一个 `match self` 分支，几何类型各自的实现代码被摊平在一处。

trait 把这两件事拆开：类型定义数据，trait 定义契约，`impl` 把两者接起来。类型集合因此是开放的——任何 crate 都能为自己的类型实现你的 trait。

## 15.2 定义 trait 与实现 trait

trait 的定义只有签名：

```rust,noplayground
trait Shape {
    fn area(&self) -> f64;
    fn perimeter(&self) -> f64;
}
```

几点语法事实：

- 方法只有签名、没有函数体，末尾是**分号**而不是花括号。这种叫必需方法（required method）。
- `&self` 的类型是实现 trait 的那个类型。trait 不知道谁会实现它，所以签名里不能出现具体类型名。
- trait 名用驼峰，通常按能力或角色命名：`Display`、`Iterator`、`From`、`Read`。
- trait 里也可以有关联函数（没有 `self` 参数），比如 `fn new() -> Self;`，调用时写 `Type::new()`。

实现写在 `impl` 块里，顺序是 `impl Trait for Type`：

```rust
trait Shape {
    fn area(&self) -> f64;
    fn perimeter(&self) -> f64;
}

struct Circle {
    r: f64,
}

struct Square {
    side: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }

    fn perimeter(&self) -> f64 {
        2.0 * std::f64::consts::PI * self.r
    }
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }

    fn perimeter(&self) -> f64 {
        4.0 * self.side
    }
}

fn main() {
    let c = Circle { r: 1.0 };
    let s = Square { side: 2.0 };
    println!("{:.2} {:.2}", c.area(), s.area());
    println!("{:.2}", s.perimeter());
}
```

四个容易踩的点：

1. **必需方法一个都不能少**。少写一个，编译器报 E0046；签名写错报 E0053。第 16 章讲关联类型时会再见到它们。
2. **签名必须逐字匹配**。参数名可以不同，类型不行。
3. **trait 方法天生公开**，不能在 `impl` 里给它加 `pub`；能加 `pub` 的是 trait 本身。
4. **孤儿规则**：`impl` 里的 trait 和类型至少有一个定义在当前 crate。15.7 节解释原因。

`Self` 在 `impl` 块里指实现类型。写关联函数时它比重复类型名更常用：`fn new(r: f64) -> Self { Self { r } }`——无论类型以后改名还是变成泛型，这行都不用动。

一个类型可以实现多个 trait，一个 trait 也可以被多个类型实现。这种多对多关系是 trait 比继承灵活的地方：`Circle` 可以同时实现 `Shape`、`Display`、`Clone`，三者互不知情，按需拼装，不必先设计一棵类型树。

还有一个选择值得早点记住：trait 方法的接收者默认写成 `&self`。写成 `self` 的方法会消费实例，调用方之后就不能再用这个值，而且它无法出现在 trait 对象里（15.6 会看到原因）。第 8 章关于三种接收者的经验在这里同样适用：只读用 `&self`，要改用 `&mut self`，只有“这个值的使命就是变成别的东西”才用 `self`。

## 15.3 默认方法：把共享实现放进契约

15.2 的两个 `impl` 里，`perimeter` 的写法完全不同，但如果每个实现都有相同的部分（比如都返回一个人类可读的名字），逐个复制就成了负担。trait 允许给方法提供默认实现：

```rust
trait Shape {
    fn area(&self) -> f64;

    fn name(&self) -> String {
        String::from("未命名图形")
    }

    fn describe(&self) -> String {
        format!("{}：面积 {:.2}", self.name(), self.area())
    }
}

struct Circle {
    r: f64,
}

struct Square {
    side: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }

    fn name(&self) -> String {
        String::from("圆")
    }
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn main() {
    let c = Circle { r: 1.0 };
    let s = Square { side: 2.0 };
    println!("{}", c.describe());
    println!("{}", s.describe());
}
```

输出是两行：`圆：面积 3.14` 和 `未命名图形：面积 4.00`。`Square` 只实现了 `area`，却直接获得了 `name` 和 `describe` 两个方法。

这里有三个机制值得说清。

**默认方法可以调用必需方法。** `describe` 的实现依赖 `area` 和 `name`——一个是必需方法，一个是另一个默认方法。实现者只要填上必需方法，就能拿到整套行为。这种“父类调用子类实现”的结构在面向对象里叫模板方法（template method），Rust 用 trait 默认方法表达同样的东西，但没有任何继承层次。

**默认方法不能访问字段。** 你没法在 `describe` 里写 `self.r`，因为 trait 不知道实现类型有哪些字段，甚至不知道实现类型有没有字段。默认方法能用的只有 trait 自己声明的方法、`Self` 上其他 trait bound 提供的方法，以及标准库对 `&Self` 的通用操作。这条限制不是缺陷：它正是“接口”与“数据”分离的代价，也保证了任何类型都能实现这个 trait，哪怕它是元组结构体或者单元结构体。

**覆盖会改变默认方法的实际行为。** `Circle` 覆盖了 `name`，于是 `describe` 里那句 `self.name()` 调到的是 `Circle` 的版本。原因和分派方式有关：静态分派时，默认方法体针对具体类型展开，`self.name()` 在展开后的代码里是一次普通调用；动态分派时，`name` 在 vtable 里指向的也是覆盖后的函数。两条路径的结果一致，所以你不需要关心“默认方法里调的是哪个版本”。

默认方法还有一条工程上的含义：给一个已经发布的公开 trait **加必需方法**是破坏性变更，所有外部实现都会编译失败；加带默认实现的方法则是兼容的。这和给结构体加字段、给枚举加变体属于同一类兼容性判断，第 32 章讲发布时会再展开。

## 15.4 trait 作为参数：把约束写进签名

第 14 章已经把 bound 的三种写法摆过一遍，这里只补几个语法细节，然后讲第 14 章没展开的位置：返回值。

```rust
trait Shape {
    fn area(&self) -> f64;
}

#[derive(Debug)]
struct Circle {
    r: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }
}

fn print_area(shape: &impl Shape) {
    println!("{:.2}", shape.area());
}

fn print_debug_area<T>(shape: &T)
where
    T: Shape + std::fmt::Debug,
{
    println!("{:?} -> {:.2}", shape, shape.area());
}

fn unit_circle() -> impl Shape {
    Circle { r: 1.0 }
}

fn main() {
    let c = Circle { r: 1.0 };
    print_area(&c);
    print_debug_area(&c);
    println!("{:.2}", unit_circle().area());
}
```

`&impl Shape` 是**每个参数各自独立**的匿名类型参数：`fn draw(a: &impl Shape, b: &impl Shape)` 允许 a、b 是不同类型，要求两者同类型时必须写 `fn draw<T: Shape>(a: &T, b: &T)`。它也不能 turbofish，`print_area::<Circle>(&c)` 编译不过；需要显式指定类型参数就换 `<T: Shape>`。bound 超过三个、或涉及关联类型约束时，`where` 比尖括号好读，两者语义和生成的代码完全一样。

`-> impl Shape` 的意思是“返回某个实现了 `Shape` 的类型，但我不告诉你是哪个”。调用方能调用 `Shape` 的方法，却写不出类型名，也不能把它赋给 `let c: Circle`。它仍是**静态分派**：调用点知道具体类型是 `Circle`，可以内联 `area`，性能与直接返回 `Circle` 相同。代价是所有 `return` 分支必须返回同一个具体类型——按条件返回 `Circle` 或 `Square` 会得到 `expected Circle, found Square`（E0308），这时只能改成 `Box<dyn Shape>`。

换来的自由是隐藏实现：`fn evens(&self) -> impl Iterator<Item = &u32>` 让调用方拿到可迭代对象，却不必知道你内部是 filter 加 map，将来换实现签名不用动。四种写法怎么选只有两条判据：类型集合在编译期是否已知，以及是否需要隐藏类型。已知且不必隐藏，用泛型或 `impl Trait`；要在运行期容纳不同类型，才付动态分派的代价。

## 15.5 trait 对象：把不同类型装进同一个容器

`impl Trait` 和泛型参数都要求类型在编译期确定，所以 `Vec<Circle>` 和 `Vec<Square>` 永远没法放进同一个 `Vec`。需要“一个容器里装多种类型”时，要用 trait 对象：

```rust
trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

struct Square {
    side: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn total_area(shapes: &[Box<dyn Shape>]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

fn main() {
    let mut shapes: Vec<Box<dyn Shape>> = Vec::new();
    shapes.push(Box::new(Circle { r: 1.0 }));
    shapes.push(Box::new(Square { side: 3.0 }));

    for s in &shapes {
        println!("{:.2}", s.area());
    }
    println!("total = {:.2}", total_area(&shapes));
}
```

关键在 `Box<dyn Shape>`。`dyn Shape` 不是具体类型，而是一个**类型擦除后的占位**：它只知道“某个实现了 `Shape` 的类型”，具体是谁要等运行期查表。`dyn Shape` 本身是 unsized 类型，不能直接当局部变量、不能按值传参，必须套一层指针才能使用。

`Box::new(Circle { r: 1.0 })` 的类型是 `Box<Circle>`，赋给 `Box<dyn Shape>` 时发生一次 unsized 强制转换（unsized coercion）：堆上的数据还是那个 `Circle`，没有额外分配，只是指针从单指针变成胖指针。`&Circle` 到 `&dyn Shape` 也会自动强转，所以 `fn print_area(shape: &dyn Shape)` 能直接收 `&c`：借用不拥有数据，生命周期受 `c` 限制。

三种所有权形态对应三种需求：

- `&dyn Shape`：只借用，调用期间数据有效，传参最省。
- `Box<dyn Shape>`：独占拥有，可以放进 `Vec`、结构体字段，离开作用域自动释放。
- `Rc<dyn Shape>`：共享拥有，多个持有者，第 18 章讲智能指针时展开。

代价是**具体类型信息丢了**：一旦变成 `dyn Shape`，你不能访问 `r` 字段，不能把它转回 `Circle`，只能调用 trait 里声明过的方法。trait 对象是单向的，数据能进，类型出不来。这也解释了它的用武之地：类型集合在运行期才确定——插件系统从配置读出一批处理器、GUI 控件树里每个节点是不同控件、命令表里每条命令是不同结构体，这些都无法在编译期列全类型，只能用 `Box<dyn Trait>`。反过来，类型在编译期已知就用泛型，因为动态分派有实打实的开销，下一节算给你看。

## 15.6 深入：vtable 里有什么，什么样的 trait 才能变成对象

先把大小量出来：

```rust
use std::mem::size_of;

trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }
}

fn main() {
    println!("Circle             = {}", size_of::<Circle>());
    println!("&Circle            = {}", size_of::<&Circle>());
    println!("&dyn Shape         = {}", size_of::<&dyn Shape>());
    println!("Box<dyn Shape>     = {}", size_of::<Box<dyn Shape>>());
    println!("Option<&dyn Shape> = {}", size_of::<Option<&dyn Shape>>());
}
```

```text
Circle             = 8
&Circle            = 8
&dyn Shape         = 16
Box<dyn Shape>     = 16
Option<&dyn Shape> = 16
```

`Circle` 本身 8 字节（一个 `f64`），`&Circle` 8 字节（一个指针）。`&dyn Shape` 变成 16 字节：它是**胖指针**（fat pointer），由两个指针组成——一个指向数据，一个指向 vtable。`Box<dyn Shape>` 也是 16 字节，因为 `Box` 内部就是一个胖指针，不是“指针 + 另一层指针”。

`Option<&dyn Shape>` 仍是 16 字节，靠的是第 8 章提过的 niche 优化：vtable 指针永远不为空，编译器拿全零位模式表示 `None`，不需要额外标记。

内存布局可以画成这样：

```text
&dyn Shape（16 字节）
┌──────────────┬──────────────┐
│ data 指针    │ vtable 指针  │
└──────┬───────┴──────┬───────┘
       │              │
       v              v
   [r: f64]      ┌────────────────────┐
   堆上的        │ drop_in_place      │
   Circle        │ size               │
                 │ align              │
                 │ area               │
                 └────────────────────┘
```

vtable 是编译器为每个“类型 + trait”组合生成的一张**静态表**，放在只读数据段。所有 `Circle` 的 `&dyn Shape` 共享同一张表，不会每个对象复制一份。表里的条目按固定顺序排列：

- `drop_in_place`：`Box<dyn Shape>` 释放时调用，按真实类型执行析构。`Circle` 没有 `Drop`，但换成含 `String` 的类型，这个槽位就负责释放堆内存。
- `size` 与 `align`：`size_of_val(&*boxed)`、`Box::from_raw`、把 `Box<dyn T>` 回收成具体类型时都要用。类型擦除之后，这是唯一还能拿到大小和对齐的途径。
- 方法指针：按 trait 里声明的顺序排列，每个方法一个地址。

所以 `shape.area()` 在机器码层面近似于“从 vtable 的固定偏移取出函数指针，把 data 指针作为第一个参数传进去，然后 call”。这一次调用是**间接**的：编译器不知道目标地址，没法内联，也没法把 `area` 的结果常量折叠。对单次调用来说开销很小（一次额外的内存读取加一次间接跳转），但在百万次的紧循环里，它和内联后的静态调用能差出几倍。

“能不能变成 trait 对象”有一套明确规则。旧文档把这套规则叫对象安全（object safety），新版 rustc 的提示改叫 dyn 兼容（dyn compatible），错误码都是 E0038。规则有五条：

1. **方法不能以 `Self` 作为参数或返回值**（接收者除外）。`fn duplicate(&self) -> Self` 的返回类型在编译期未知，vtable 不知道该准备多大的返回空间——这正是案例 2 的报错。
2. **方法不能有泛型参数**。vtable 每个槽位只有一个地址，`fn cmp<T>(&self, other: &T)` 需要为每个 `T` 生成不同代码，塞不进同一张表。
3. **不能有关联常量**。trait 对象只有“数据指针 + 函数表”，没有地方存每个实现各自的常量值；写成 `Box<dyn Versioned>` 会得到 E0038，提示里明说 “contains the associated constant”。关联类型不阻止 dyn 兼容，但使用时必须写死，例如 `dyn Iterator<Item = u8>`，第 16 章展开。
4. **接收者必须是 vtable 能表达的形式**：`&self`、`&mut self`，或 `Box<Self>`、`Rc<Self>`、`Arc<Self>` 这类自带间接层的接收者。按值 `self` 的方法在 dyn 上没法调用（调用方不知道大小），通常要配 `where Self: Sized`。
5. **`where Self: Sized` 的方法被排除在 vtable 之外**，不参与上面的判定。这条规则是逃生门：trait 里可以既有给 dyn 用的通用方法，也有只给具体类型用的便利方法。标准库的 `Iterator` 就靠它保住 `dyn Iterator`：`map`、`filter` 这些适配器方法都带 `Self: Sized`，所以 `dyn Iterator` 还能用，只是不能链式调用适配器。

和别的语言对比一下这份开销花在哪。C++ 把 vptr 存进对象里，每个含虚函数的对象至少多一个指针，对象布局被虚函数表污染；Rust 把表指针放在引用或 `Box` 上，`Circle` 本身仍是 8 字节，只有当你选择 `&dyn` 时才多付那 8 字节。Java 和 Go 的接口值同样是两个字（对象指针 + 方法表），但它们的对象头自带类型信息，依赖 GC 管理生命周期。Rust 的取舍是：类型擦除的代价由使用者按需支付，不用 `dyn` 就一分不付——这是“零成本抽象”在运行期的具体含义，也是为什么 `dyn` 必须套一层指针：没有对象头，就只能把表指针放在指针里。

## 15.7 孤儿规则：为什么不能给别人类型实现别人 trait

设想两个 crate 各自写了 `impl Display for Vec<i32>`。单独编译都通过，一起链接时同一个类型出现两份 `Display` 实现，调用点该用哪一份没有答案。更糟的是这种冲突可能由一次无关的依赖升级引入：今天能编译的项目，明天 `cargo update` 之后就炸了。

孤儿规则（orphan rule）把这种可能性在编译期排除掉：**`impl Trait for Type` 里，trait 和 Type 至少有一个定义在你自己的 crate 里**。两个都是外部的，就不许写。这条规则还有一条针对泛型的加强版：`impl<T> ForeignTrait for Vec<T>` 同样不行，因为下游可以把 `T` 填成本地类型，从而间接实现外部 trait。

允许的两种方向：

```rust
use std::fmt;

struct Meters(f64);

impl fmt::Display for Meters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} m", self.0)
    }
}

trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Vec<i32> {
    fn describe(&self) -> String {
        format!("{} 个整数", self.len())
    }
}

fn main() {
    println!("{}", Meters(1.5));
    println!("{}", vec![1, 2, 3].describe());
}
```

第一种是“本地类型 + 外部 trait”：给自己的 `Meters` 实现标准库的 `Display`，这是日常写得最多的一类 impl。第二种是“本地 trait + 外部类型”：给你自己的 `Describe` 实现到 `Vec<i32>` 上，标准库不会因此改变，别的 crate 也看不到这个实现，除非它们依赖你。

被禁止的是第三种组合，也就是案例 3。想给外部类型加外部 trait 的行为，标准做法是 newtype：`struct Wrapper(Vec<i32>)`，然后为 `Wrapper` 实现 `Display`。包装类型是本地类型，规则满足；运行期零开销，解包只要 `.0`。第 16 章会讲怎么用 `Deref` 让包装类型用起来像原类型。

孤儿规则还有一个不太直观的后果：它保证了**每个 impl 都属于某个确定的 crate**，因此依赖图可以独立编译，不会出现“下游悄悄加了一个 impl，改变上游代码的行为”。代价是你不能像在 Ruby 或 Python 里那样给任何类打猴子补丁（monkey patch），这是一次有意的取舍。

## 典型错误案例

下面五个错误是写 trait 时最常撞上的，报错文本来自 rustc 1.100.0 的实际输出。

### 案例 1：忘了给类型实现 trait——E0277

```rust,editable
trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

fn report<T: Shape>(item: &T) {
    println!("{:.2}", item.area());
}

fn main() {
    let c = Circle { r: 1.0 };
    report(&c);
}
```

```text
error[E0277]: the trait bound `Circle: Shape` is not satisfied
  --> src/main.rs:15:12
   |
15 |     report(&c);
   |     ------ ^^ unsatisfied trait bound
   |     |
   |     required by a bound introduced by this call
   |
help: the trait `Shape` is not implemented for `Circle`
  --> src/main.rs:5:1
   |
 5 | struct Circle {
   | ^^^^^^^^^^^^^
help: this trait has no implementations, consider adding one
  --> src/main.rs:1:1
   |
 1 | trait Shape {
   | ^^^^^^^^^^^
note: required by a bound in `report`
  --> src/main.rs:9:14
   |
 9 | fn report<T: Shape>(item: &T) {
   |              ^^^^^ required by this bound in `report`

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`<T: Shape>` 是调用点必须满足的约束，编译器先查 bound 再谈单态化，所以错误落在 `report(&c)` 而不是函数体里。help 第二句 “this trait has no implementations” 说明整个程序里 `Shape` 一个实现都没有；如果你写了 impl 但类型名写错，这句不会出现，可以用它区分“忘了实现”和“实现到了别的类型上”。

**修复**：补上 `impl Shape for Circle`：

```rust,editable
trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.r * self.r
    }
}

fn report<T: Shape>(item: &T) {
    println!("{:.2}", item.area());
}

fn main() {
    let c = Circle { r: 1.0 };
    report(&c);
}
```

</details>

### 案例 2：trait 方法返回 Self——E0038

```rust,editable
trait Cloneable {
    fn duplicate(&self) -> Self;
}

struct Token {
    id: u32,
}

fn main() {
    let items: Vec<Box<dyn Cloneable>> = Vec::new();
    println!("{}", items.len());
}
```

```text
error[E0038]: the trait `Cloneable` is not dyn compatible
  --> src/main.rs:10:28
   |
10 |     let items: Vec<Box<dyn Cloneable>> = Vec::new();
   |                            ^^^^^^^^^ `Cloneable` is not dyn compatible
   |
note: for a trait to be dyn compatible it needs to allow building a vtable
  --> src/main.rs:2:28
   |
 1 | trait Cloneable {
   |       --------- this trait is not dyn compatible...
 2 |     fn duplicate(&self) -> Self;
   |                            ^^^^ ...because method `duplicate` references the `Self` type in its return type
   = help: consider moving `duplicate` to another trait

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：vtable 的前提是所有实现共用一套调用约定。`duplicate` 的返回类型随实现类型变化、大小未知，编译器没法为 `dyn Cloneable` 生成固定布局的返回槽，于是整个 trait 失去 dyn 兼容性。help 建议把方法移到另一个 trait，就是让主 trait 的每个方法都能进表。

**修复**：只让具体类型使用这个方法，给它加 `where Self: Sized`，它就退出了 vtable：

```rust,editable
trait Cloneable {
    fn duplicate(&self) -> Self
    where
        Self: Sized;

    fn tag(&self) -> &'static str;
}

struct Token {
    id: u32,
}

impl Cloneable for Token {
    fn duplicate(&self) -> Self {
        Token { id: self.id }
    }

    fn tag(&self) -> &'static str {
        "token"
    }
}

fn main() {
    let items: Vec<Box<dyn Cloneable>> = vec![Box::new(Token { id: 1 })];
    let t = Token { id: 2 };
    println!("{} {} {}", items[0].tag(), t.duplicate().id, t.id);
}
```

</details>

### 案例 3：给外部类型实现外部 trait——E0117

```rust,editable
use std::fmt;

impl fmt::Display for Vec<i32> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

fn main() {
    println!("{}", vec![1, 2, 3]);
}
```

```text
error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate
 --> src/main.rs:3:1
  |
3 | impl fmt::Display for Vec<i32> {
  | ^^^^^^^^^^^^^^^^^^^^^^--------
  |                       |
  |                       `Vec` is not defined in the current crate
  |
  = note: impl doesn't have any local type before any uncovered type parameters
  = note: for more information see https://doc.rust-lang.org/reference/items/implementations.html#orphan-rules
  = note: define and implement a trait or new type instead

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Display` 和 `Vec` 都来自标准库，孤儿规则不允许。判定方式是“在遇到未覆盖的类型参数之前，必须先出现一个本地类型”：`Vec<i32>` 的最外层是外部的 `Vec`，`i32` 也是外部的，找不到本地类型。写成 `impl<T> Display for Vec<T>` 同样不行——`T` 未覆盖，下游可以把它填成本地类型，等于让别的 crate 间接实现了外部 trait。

**修复**：用 newtype 把外部类型包一层，包装类型是本地类型，规则就满足了：

```rust,editable
use std::fmt;

struct Numbers(Vec<i32>);

impl fmt::Display for Numbers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|n| n.to_string()).collect();
        write!(f, "[{}]", parts.join(", "))
    }
}

fn main() {
    println!("{}", Numbers(vec![1, 2, 3]));
}
```

</details>

### 案例 4：trait 没导入，方法找不到——E0599

```rust,editable
mod shapes {
    pub trait Shape {
        fn area(&self) -> f64;
    }

    pub struct Circle {
        pub r: f64,
    }

    impl Shape for Circle {
        fn area(&self) -> f64 {
            3.14159 * self.r * self.r
        }
    }
}

fn main() {
    let c = shapes::Circle { r: 1.0 };
    println!("{:.2}", c.area());
}
```

```text
error[E0599]: no method named `area` found for struct `Circle` in the current scope
  --> src/main.rs:19:25
   |
 3 |         fn area(&self) -> f64;
   |            ---- the method is available for `Circle` here
...
 6 |     pub struct Circle {
   |     ----------------- method `area` not found for this struct
...
19 |     println!("{:.2}", c.area());
   |                         ^^^^ method not found in `Circle`
   |
   = help: items from traits can only be used if the trait is in scope
help: trait `Shape` which provides `area` is implemented but not in scope; perhaps you want to import it
   |
 1 + use crate::shapes::Shape;
   |

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：方法解析分两步——先找类型上的固有方法，再在“当前作用域内的 trait”里找。`Shape` 定义在 `shapes` 模块里，`main` 没有把它引入作用域，候选集里就没有 `area`。`= help: items from traits can only be used if the trait is in scope` 是这条规则的原话。

**修复**：把 trait 引入作用域。`use shapes::Shape as _;` 也可以，它只引入方法、不引入名字，适合两个 trait 有同名方法时避免冲突：

```rust,editable
mod shapes {
    pub trait Shape {
        fn area(&self) -> f64;
    }

    pub struct Circle {
        pub r: f64,
    }

    impl Shape for Circle {
        fn area(&self) -> f64 {
            3.14159 * self.r * self.r
        }
    }
}

use shapes::Shape;

fn main() {
    let c = shapes::Circle { r: 1.0 };
    println!("{:.2}", c.area());
}
```

</details>

### 案例 5：关联类型没写清属于哪个 trait——E0223

```rust,editable
trait Container {
    type Item;
}

struct Numbers;

impl Container for Numbers {
    type Item = i32;
}

fn main() {
    let x: Numbers::Item = 1;
    println!("{}", x);
}
```

```text
error[E0223]: ambiguous associated type
  --> src/main.rs:12:12
   |
12 |     let x: Numbers::Item = 1;
   |            ^^^^^^^^^^^^^
   |
help: use fully-qualified syntax
   |
12 -     let x: Numbers::Item = 1;
12 +     let x: <Numbers as Container>::Item = 1;
   |
12 -     let x: Numbers::Item = 1;
12 +     let x: <Numbers as IntoIterator>::Item = 1;
   |

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Numbers::Item` 只说了“某个 `Item`”，没说属于哪个 trait。一个类型可以实现多个带同名关联类型的 trait（标准库的 `IntoIterator` 就是一个），编译器拒绝猜；即使实际上只有一个实现，判定也发生在类型检查阶段，不做全局唯一性推理。

**修复**：用全限定语法写死 trait：

```rust,editable
trait Container {
    type Item;
}

struct Numbers;

impl Container for Numbers {
    type Item = i32;
}

fn main() {
    let x: <Numbers as Container>::Item = 1;
    println!("{}", x);
}
```

</details>

## 动手练习

**练习 1**：定义 `trait Summary { fn summarize(&self) -> String; fn author(&self) -> String; }`，为 `Article` 和 `Video` 各写一个实现。给 `summarize` 写默认实现（调用 `author`），再让 `Video` 覆盖它，输出“视频：作者名”。最后写一个接收 `&dyn Summary` 的函数，把两种值依次传进去。

**练习 2**：在 15.5 的 `Shape` 上新增 `Triangle`，要求 `main` 不改动已有的 `Circle`、`Square` 实现，只新增一个 `impl` 和一行 `push`。再写 `fn largest(shapes: &[Box<dyn Shape>]) -> Option<&str>`，返回面积最大的图形名字。体会“开放集合”具体省掉了哪些改动。

**练习 3**：设计 `trait Cache { fn get(&self, key: &str) -> Option<&str>; fn put(&mut self, key: &str, value: &str); fn contains(&self, key: &str) -> bool { self.get(key).is_some() } }`，用 `HashMap<String, String>` 实现它（提示：`get` 的返回类型要先 `map(|s| s.as_str())`）。写测试验证默认方法 `contains` 在两个分支上的行为。

**练习 4**：实现一个插件注册表：`trait Plugin { fn name(&self) -> &'static str; fn run(&self, input: &str) -> String; }`，用 `HashMap<&'static str, Box<dyn Plugin>>` 保存三个插件，按名字从命令行参数选择并执行。要求 `main` 里不出现任何具体插件类型名，只有构造时的 `Box::new` 这一处例外。

**练习 5**：写一个带 `fn clone_me(&self) -> Self` 的 trait，试着声明 `Box<dyn ThatTrait>`，把 E0038 的完整报错抄下来，逐句对上 15.6 的规则。然后加 `where Self: Sized` 修好，并解释为什么修好之后 `dyn` 能用了、但 `clone_me` 不能在 `dyn` 上调用。

## 小结

- trait 描述“类型能做什么”，不描述“类型由什么组成”；类型集合因此是开放的，这是它相对枚举的核心优势
- `impl Trait for Type` 必须实现全部必需方法，签名逐字匹配；trait 方法天生公开
- 默认方法可以调用必需方法，但不能访问字段；覆盖后，默认方法体内部的调用会走到覆盖版本
- `impl Trait` 参数是匿名泛型参数，每个位置各自独立；需要“同一类型”时改用 `<T: Trait>`
- 返回位置的 `impl Trait` 是静态分派，能隐藏类型名，但所有返回分支必须是同一个具体类型
- `dyn Trait` 是类型擦除，必须套指针使用：`&dyn` 借用、`Box<dyn>` 拥有、`Rc<dyn>` 共享；代价是丢失具体类型和一次间接调用
- `&dyn Trait` 是 16 字节的胖指针：数据指针加 vtable 指针；vtable 里是析构函数、大小、对齐和方法指针
- trait 要能变成对象，方法就不能返回 `Self`、不能带泛型参数、不能有关联常量；`where Self: Sized` 的方法会被排除在 vtable 之外
- 孤儿规则要求 impl 里的 trait 或类型至少有一个是本地定义的，它保证了每个 impl 归属确定、依赖图可独立编译




