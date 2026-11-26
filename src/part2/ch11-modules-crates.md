# 第 11 章 模块、包与工作空间

> **本章目标**
>
> - 会用 `mod` 把代码组织成一棵树，并说清"模块"和"文件"不是一回事
> - 能给每个项选对可见性：私有、`pub`、`pub(crate)`、`pub(super)`、`pub(in path)`
> - 会用 `use`、`as` 和 `crate`/`self`/`super` 写出既不含糊也不啰嗦的路径
> - 能分清包（package）、crate、模块三者的边界，会写 `Cargo.toml` 的依赖、feature 与 workspace
> - 知道 crate 边界上编译器做了什么：为什么没有头文件、泛型为什么要单态化、拆 crate 的收益和代价

第 8 章到第 10 章的代码都写在同一个 `main.rs` 里。真实项目里，一个文件很快会涨到几千行：矩形和圆都想叫 `area`，工具函数散落在各处，改一个函数要在文件里翻半天。更麻烦的是"谁能用谁"没有任何约束——任何函数都能调到任何别的函数，几个月后没人敢删代码。

C 用 `#include` 解决复用，但它是文本替换：头文件里的声明和 `.c` 文件里的定义可能对不上，编译器要到链接期才发现；两边结构体布局不一致时甚至不报错，直接读出垃圾数据。Java 把包名和目录绑死，挪一个文件就得改 `package` 声明。Rust 走的是第三条路：**模块树由源码里的 `mod` 声明决定，文件只是装载模块的一种方式**。

这一章讲三件事：模块树怎么长（`mod`、`pub`、`use`）、模块怎么落到文件（`mod name;` 的查找规则）、crate 与包怎么划边界（`Cargo.toml`、feature、workspace）。最后一段往编译器里走一步：crate 边界上到底发生了什么。

## 11.1 模块是一棵树

`mod` 后面跟一个名字和一对花括号，就在模块树里挂一个节点。节点里的项用 `::` 逐层访问：

```rust
mod geometry {
    pub mod rect {
        pub struct Rectangle {
            pub width: u32,
            pub height: u32,
        }

        pub fn area(r: &Rectangle) -> u32 {
            r.width * r.height
        }
    }

    pub mod circle {
        pub struct Circle {
            pub radius: u32,
        }

        pub fn area(c: &Circle) -> u32 {
            3 * c.radius * c.radius
        }
    }
}

fn main() {
    let r = geometry::rect::Rectangle { width: 3, height: 4 };
    let c = geometry::circle::Circle { radius: 2 };
    println!("{} {}", geometry::rect::area(&r), geometry::circle::area(&c));
}
```

输出 `12 12`。注意两个 `area` 同名却不冲突：它们的**完整路径**不同，一个是 `geometry::rect::area`，一个是 `geometry::circle::area`。模块的第一职责就是这个——给名字划出互不干扰的命名空间（namespace）。

第二职责是把"谁能看见谁"变成结构。上面 `Rectangle` 和 `Circle` 各自待在自己的模块里，谁想用都得写出完整路径，读者一眼就知道这个类型属于哪一块。

从 crate 根往下画，模块是一棵树：根是 crate，`geometry` 是它的子节点，`rect` 和 `circle` 又是 `geometry` 的子节点。写路径时有三个固定起点：

- `crate::geometry::rect::area`：从 crate 根出发的绝对路径
- `self::area`：从当前模块出发
- `super::area`：从父模块出发

模块也是项（item），所以它自己也能被 `pub` 修饰，也能被 `use` 起别名。这一点在 11.3 会展开。

## 11.2 可见性：谁能看见谁

模块树的默认规则只有一条：**项对它所在的模块及其后代可见，对外一律不可见**。想放宽，就加 `pub` 一类的修饰。

```rust
mod geometry {
    pub(crate) fn unit() -> &'static str {
        "cm"
    }

    pub mod rect {
        use super::unit;

        pub struct Rectangle {
            width: u32,
            height: u32,
        }

        impl Rectangle {
            pub fn new(width: u32, height: u32) -> Self {
                Self { width, height }
            }

            pub fn area(&self) -> u32 {
                self.width * self.height
            }

            pub fn describe(&self) -> String {
                format!("{} x {} {}", self.width, self.height, unit())
            }
        }
    }
}

fn main() {
    let r = geometry::rect::Rectangle::new(3, 4);
    println!("{} / {}", r.area(), r.describe());
}
```

输出 `12 / 3 x 4 cm`。这段代码里有四处可见性决策，每一处都在表达一个意图：

1. `unit` 标成 `pub(crate)`：整个 crate 都能用，但 crate 的使用者看不到它。它是个实现细节。
2. `Rectangle` 的字段没写 `pub`：外部**不能**写 `Rectangle { width: 3, height: 4 }`，必须走 `new`。将来想在 `new` 里加"宽高不能为 0"的校验，所有调用方自动受约束。
3. `new`、`area`、`describe` 是 `pub`：这是这个类型对外的接口。
4. `rect` 模块本身是 `pub`：否则 `geometry::rect::Rectangle` 这条路走不通。

第 4 条是最容易忽略的一条：**可见性要逐层检查**。哪怕 `Rectangle` 是 `pub`，只要它所在的 `rect` 是私有的，crate 外面就依然拿不到它。想暴露里面的东西，要么把外层也 `pub`，要么用 `pub use` 把它重导出到一条公开路径上（11.7 会用到）。

五种写法对应五个范围：

| 写法 | 可见范围 |
|---|---|
| 什么都不写 | 当前模块及其后代 |
| `pub` | 任何能走到这个模块的路径 |
| `pub(crate)` | 当前 crate 内，不出 crate |
| `pub(super)` | 父模块及其后代 |
| `pub(in crate::geometry)` | 指定的祖先模块内 |

`pub(in path)` 里的 `path` 必须是当前模块的祖先，写 `pub(in crate::geometry)` 合法，写 `pub(in crate::geometry::circle)` 就会报 `error[E0742]: visibilities can only be restricted to ancestor modules`——可见范围只能往祖先方向收窄，不能指向兄弟模块。

`pub(super)` 的典型用途是"兄弟模块之间共享，但不给外面看"：

```rust
mod geometry {
    pub mod rect {
        pub fn area(width: u32, height: u32) -> u32 {
            width * height
        }
    }

    mod helpers {
        pub(super) fn label() -> &'static str {
            "rect"
        }
    }

    pub fn describe() -> String {
        format!("{}: {}", helpers::label(), rect::area(3, 4))
    }
}

fn main() {
    println!("{}", geometry::describe());
}
```

`helpers` 是私有模块，外面连名字都写不出来；但它里面的 `label` 对父模块 `geometry` 可见，所以 `geometry::describe` 能调它。这是"内部结构藏起来、对外只留一个入口"的最小形态。

## 11.3 use：给路径起个短名字

每次都写全路径太啰嗦。`use` 把某个路径绑到一个短名字上：

```rust
use std::collections::HashMap;
use std::fmt::{self, Display};

struct Point {
    x: i32,
    y: i32,
}

impl Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

fn main() {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    counts.insert("a", 1);
    println!("{} {:?}", Point { x: 1, y: 2 }, counts);
}
```

`use std::fmt::{self, Display};` 里的 `self` 指 `std::fmt` 本身，所以一行同时引入了模块名 `fmt` 和里面的 `Display`。花括号里可以并列多个名字，也可以嵌套。

`use` 是**别名**，不是文本粘贴。它不复制任何代码，也不会像 `#include` 那样把同一个定义引入两次。编译器只是记住"这个模块里 `Display` 指向 `std::fmt::Display`"，之后你做名字解析时去查这张表。

这一点带来一个和 C 很不一样的后果：**`use` 只在它所在的模块（或块）里生效**。父模块写了 `use std::collections::HashMap;`，子模块里并不能直接用。这条规则初看别扭，但它保证了你读任何一个模块时，名字的来源都写在这个模块自己的开头。

两个名字撞车时，用 `as` 给其中一个改名：

```rust
use std::collections::HashMap as StdMap;

struct Map {
    name: &'static str,
}

fn main() {
    let m: StdMap<&str, u32> = StdMap::new();
    let tile = Map { name: "森林" };
    println!("{} {}", m.len(), tile.name);
}
```

还有一种更隐蔽的用法——`as _`。它把 trait 引入作用域，让方法可以被调用，但不占用一个名字，因此不会和别处的同名 trait 撞车。标准库里的 `use std::io::Read as _;` 就是这个套路：只要方法能用，名字不必存在。

通配导入 `use geometry::*;` 能把一个模块的公开项全部拉进来，但它把"名字从哪来"这件事从代码里抹掉了。除了测试模块和刻意模仿 prelude 的场景，不要用。

最后把三个路径起点放在一起看：

```rust
mod outer {
    pub const NAME: &str = "outer";

    pub mod inner {
        pub const NAME: &str = "inner";

        pub fn describe() -> String {
            format!(
                "crate={} super={} self={}",
                crate::outer::NAME,
                super::NAME,
                self::NAME
            )
        }
    }
}

fn main() {
    println!("{}", outer::inner::describe());
}
```

输出 `crate=outer super=outer self=inner`。`super::NAME` 和 `crate::outer::NAME` 在这里指的是同一个常量，因为 `inner` 只比根深一层；层数一多，两者的区别就出来了。

`use` 前面加 `pub` 就是**重导出**（re-export）：别名本身成为公开接口的一部分。11.7 会用它把内部结构压平成一个干净的门面。

## 11.4 模块怎么落到文件

`mod name;`（没有花括号）是一条指令：这个模块的内容不在本文件里，去磁盘上找。找哪个文件，取决于这条 `mod` 写在哪：

| `mod` 写在哪 | 去哪里找子模块 |
|---|---|
| `src/main.rs` 或 `src/lib.rs`（crate 根） | `src/name.rs` 或 `src/name/mod.rs` |
| `src/name/mod.rs` | `src/name/child.rs` |
| `src/name.rs` | `src/name/child.rs` |

规律可以压成一句话：**子模块的文件，放在"父模块同名目录"里；crate 根的文件，放在 `src/` 下**。

一个两层的例子：

```text
src/
├── main.rs
└── geometry/
    ├── mod.rs
    ├── shape.rs
    └── rect.rs
```

```rust,ignore
// src/main.rs
mod geometry;

use crate::geometry::Rectangle;
use crate::geometry::shape::Shape;

fn main() {
    let r = Rectangle::new(3, 4);
    println!("{}", r.area());
}
```

```rust,ignore
// src/geometry/mod.rs
pub mod shape;

mod rect;

pub use rect::Rectangle;
```

`mod rect;` 写在 `src/geometry/mod.rs` 里，所以编译器去找 `src/geometry/rect.rs`。`rect` 本身是私有的，但 `pub use rect::Rectangle;` 把类型搬到了 `geometry` 这一层，于是 `use crate::geometry::Rectangle;` 能走通。这就是"内部私有、外部可见"的标准写法。

同样的结构也可以用新风格表达，不再需要 `mod.rs`：

```text
src/
├── main.rs
├── geometry.rs
└── geometry/
    ├── shape.rs
    └── rect.rs
```

`src/geometry.rs` 代替了 `src/geometry/mod.rs`，内容一模一样，子模块依旧放在 `src/geometry/` 目录里。两种风格都合法，2018 版之后新代码一般用第二种——目录里不会每层都躺一个 `mod.rs`。

**同一个模块不能两种风格同时存在**。如果 `src/geometry.rs` 和 `src/geometry/mod.rs` 都在，编译器会报 `error[E0761]`，说明同一个模块在两处都找到了文件，让你删掉一个。

把上面的文件系统版本改写成单文件时，只需把每一条 `mod x;` 换成用花括号包住原文件内容的内联模块，其余一字不改——模块树没变，只是从文件装载换成了内联装载。

反过来说，一个文件里可以写任意多个内联模块；但一条 `mod name;` 只对应一个文件，它的子模块再放进同名目录。模块树和文件树是两棵树，编译器用 `mod` 声明把后者映射到前者。

## 11.5 包、crate 与工作空间

三个词经常被混着用，先把它们分开：

- **包（package）**：一个 `Cargo.toml` 描述的工程单元。它最多有一个库 crate，可以有任意多个二进制 crate。
- **crate**：一次编译的单元，有一个 crate 根（`src/lib.rs`、`src/main.rs` 或 `src/bin/foo.rs`）。`cargo build` 实际调用 rustc 的次数，大致就是 crate 的个数。
- **模块**：crate 内部的树，就是前四节讲的东西。

所以"一个包里有几个 crate"是有确定答案的：看 `src/lib.rs` 和 `src/bin/`。而"一个 crate 里有几个模块"由 `mod` 决定。

`Cargo.toml` 里需要看懂的部分：

```toml
[package]
name = "shapes"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"], optional = true }

[features]
default = []
json = ["dep:serde"]

[dev-dependencies]
approx = "0.5"
```

- `version = "1"` 是 `^1` 的简写，含义是"至少 1.0.0，但不要 2.0.0"。真正的版本由 `Cargo.lock` 钉死。写 `=1.4.2` 是精确锁定，写 `~1.4` 只允许补丁级升级。
- `features` 是编译期开关，原则是**只做加法**：打开一个 feature 只能增加能力，不能改变已有 API 的语义，否则下游任意组合都会炸。
- 可选依赖默认会自动生成同名 feature，`dep:serde` 语法表示"不要生成那个同名 feature，只让 `json` 控制它"。
- `dev-dependencies` 只服务于 `cargo test`、`cargo bench` 和 `examples/`，不会进入最终产物。

多个相关的包可以放进一个工作空间（workspace），共享一个 `Cargo.lock` 和一个 `target/` 目录：

```toml
[workspace]
members = ["shapes", "cli"]
resolver = "2"

[workspace.dependencies]
serde = "1"

[profile.release]
lto = "thin"
codegen-units = 1
```

成员包的 `Cargo.toml` 用 `workspace = true` 继承版本，例如 `serde.workspace = true`，同一个表里再写 `shapes = { path = "../shapes" }` 依赖同 workspace 的另一个包。

这样做的直接好处：依赖版本只在一处声明，不会出现 A 包要 serde 1.0、B 包要 serde 1.2 导致依赖图里出现两份 serde；`cargo test --workspace` 一条命令跑全部测试；`cargo build` 能并行编译多个 crate。

代价也要说清楚：拆成十个 crate 不会自动变快。每个 crate 都要单独写 `Cargo.toml`，跨 crate 的函数默认不能内联（见 11.6），小 crate 多了反而增加链接与元数据开销。拆分的判据是"这块代码是否独立演化、是否需要单独发布或被别的项目复用"，不是"目录看起来整齐"。

## 11.6 深入：crate 边界上发生了什么

**没有头文件，因为接口是编译器写的。** rustc 编完一个 crate，会把它的公开接口序列化进 metadata，随 `.rlib` 一起产出（`cargo check` 时只产出 `.rmeta`）。下游 crate 编译时直接读这份二进制接口，不需要你重复声明一遍。C 的 `#include` 是把头文件文本复制进每个 `.c`，声明和定义是两份独立的事实，可能漂移；Rust 只有一份事实，就在源码里。所以 Rust 里不存在"头文件和实现不一致"这个 bug 类别。

**`pub` 不控制链接。** 可见性只在名字解析和类型检查阶段生效。一个 `pub(crate)` 函数照样会被编成符号放进 `.rlib`；反过来，`pub` 也不等于"C 语言能调用"，要导出 C ABI 得加 `extern "C"` 和 `no_mangle` 属性。真正让一段代码从二进制里消失的是死代码消除和 LTO，而链接器看的是符号，不是可见性。所以"把函数改成私有能减小二进制"是错觉——只有没人调用才行。

**泛型在使用的 crate 里生成代码。** `fn max<T: Ord>(a: T, b: T) -> T` 编进库时并不会生成机器码，它要等下游写出 `max(1u8, 2u8)` 才生成一份 `u8` 版本。这就是单态化（monomorphization）。代价是泛型函数体必须放进 metadata（编译器保留它的 MIR），下游才能生成代码；于是泛型代码天然"源码可见"，泛型用得多，二进制就大。普通非泛型函数没有这个问题：下游只拿到一个符号引用，默认不会被内联，除非标了 `#[inline]` 或者开了 LTO。

**crate 是增量的边界。** 一个 crate 内部会再切成若干代码生成单元（codegen unit，CGU），开发配置默认 256 个，release 默认 16 个。改一个函数，只有包含它的 CGU 需要重编；但**所有下游 crate 都要重编**，因为 crate 是编译单元，上游接口可能变了。这解释了拆 workspace 的真实收益：多个 crate 能并行编译，改动只波及依赖它的那一条链。代价是跨 crate 优化变少，需要 `lto = "thin"` 补回来一部分。

**同一个 crate 的两个版本可以同时存在。** 符号名里编码了 crate 的消歧哈希，所以 `foo 1.x` 和 `foo 2.x` 能出现在同一张依赖图里。代价是两边的 `Foo` 是不同的类型，不能互相赋值，trait 实现也不通用。`cargo tree` 里看到重复版本时，要么统一升级，要么用 `[patch]` 指到同一个源。

## 11.7 综合：把 Rectangle 拆成一个库 crate

第 8 章的 `Rectangle` 现在只有一个文件。把它拆成一个可以被别的项目依赖的库，同时保留一个命令行入口：

```text
shapes/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── main.rs
    └── geometry/
        ├── mod.rs
        ├── shape.rs
        └── rect.rs
```

```toml
[package]
name = "shapes"
version = "0.1.0"
edition = "2021"
```

```rust,ignore
// src/lib.rs
pub mod geometry;

pub use geometry::{Rectangle, Shape};
```

```rust,ignore
// src/geometry/mod.rs
mod shape;
mod rect;

pub use rect::Rectangle;
pub use shape::Shape;
```

```rust,ignore
// src/geometry/shape.rs
pub trait Shape {
    fn area(&self) -> u32;
    fn name(&self) -> &'static str;
}
```

```rust,ignore
// src/geometry/rect.rs
use crate::geometry::shape::Shape;

pub struct Rectangle {
    width: u32,
    height: u32,
}

impl Rectangle {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl Shape for Rectangle {
    fn area(&self) -> u32 {
        self.width * self.height
    }

    fn name(&self) -> &'static str {
        "rectangle"
    }
}
```

```rust,ignore
// src/main.rs
use shapes::{Rectangle, Shape};

fn main() {
    let r = Rectangle::new(3, 4);
    println!("{} 的面积是 {}", r.name(), r.area());
}
```

```console
$ cargo run
rectangle 的面积是 12
```

`main.rs` 是同一包里另一个 crate。对它来说 `shapes` 是**外部 crate**，所以路径从 crate 名开头，写 `use shapes::{Rectangle, Shape};`，而不是从 `crate::` 走；Cargo 自动让二进制依赖同包的库，不用在 `Cargo.toml` 里声明。

整个结构里有三处可见性设计，缺一不可：

1. `mod rect;` 是私有的。外部拿不到 `shapes::geometry::rect::Rectangle` 这条路径。
2. `pub use rect::Rectangle;` 把类型搬到 `geometry` 层，`pub use geometry::{Rectangle, Shape};` 再搬到 crate 根。
3. 字段私有、只留 `new`，外部无法绕过校验直接拼一个 `Rectangle`。

对外只剩两个名字：`shapes::Rectangle` 和 `shapes::Shape`。将来把 `rect` 拆成目录、或者换一个内部实现，只要 `lib.rs` 的重导出不变，调用方一行都不用改。这就是门面（facade）模式在 crate 层面的用法。

在练习或书里的 playground 上，把每一条 `mod x;` 换成内联模块就能塞进一个文件，模块路径与 `pub use` 的写法都不用改。

## 典型错误案例

下面五个错误覆盖了模块这一章最常见的坑。报错文本来自 rustc 1.100.0，代码块行号与报错里的行号一一对应。

### 案例 1：漏写中间那层模块——E0432

```rust,editable
mod geometry {
    pub mod rect {
        pub struct Rectangle {
            pub width: u32,
            pub height: u32,
        }
    }
}

use crate::geometry::Rectangle;

fn main() {
    let r = Rectangle { width: 1, height: 2 };
    println!("{}", r.width);
}
```

```text
error[E0432]: unresolved import `crate::geometry::Rectangle`
  --> src/main.rs:10:5
   |
10 | use crate::geometry::Rectangle;
   |     ^^^^^^^^^^^^^^^^^---------
   |                      |
   |                      no `Rectangle` in `geometry`
   |
help: consider importing this struct instead
   |
10 | use crate::geometry::rect::Rectangle;
   |                      ++++++

error: aborting due to 1 previous error
```

**原因**：`use` 后面的路径必须**完整**，每一层模块都要写出来。`Rectangle` 定义在 `geometry` 的**子模块** `rect` 里，它的真实路径是 `crate::geometry::rect::Rectangle`；`crate::geometry::Rectangle` 这个名字在 `geometry` 这一层根本不存在。

**修复**：补上 `rect`。如果不想让调用方关心内部层级，就在 `geometry` 里重导出（11.7 的做法）：

```rust,editable
mod geometry {
    pub mod rect {
        pub struct Rectangle {
            pub width: u32,
            pub height: u32,
        }
    }

    pub use rect::Rectangle;
}

use crate::geometry::Rectangle;

fn main() {
    let r = Rectangle { width: 1, height: 2 };
    println!("{}", r.width);
}
```

### 案例 2：忘了给中间模块加 pub——E0603

```rust,editable
mod geometry {
    mod rect {
        pub struct Rectangle {
            pub width: u32,
        }
    }
}

fn main() {
    let r = crate::geometry::rect::Rectangle { width: 1 };
    println!("{}", r.width);
}
```

```text
error[E0603]: module `rect` is private
  --> src/main.rs:10:30
   |
10 |     let r = crate::geometry::rect::Rectangle { width: 1 };
   |                              ^^^^  --------- struct `Rectangle` is not publicly re-exported
   |                              |
   |                              private module
   |
note: the module `rect` is defined here
  --> src/main.rs:2:5
   |
 2 |     mod rect {
   |     ^^^^^^^^

error: aborting due to 1 previous error
```

**原因**：`Rectangle` 本身是 `pub`，但它所在的 `rect` 模块是私有的。可见性要逐层检查：一条路径上的任何一层不可见，整条路径就不可达。报错指向的是 `rect`，不是 `Rectangle`，这一点很容易看错。

**修复**：把 `rect` 也标成 `pub`，或者像案例 1 那样用 `pub use` 把类型提到一个公开的层级：

```rust,editable
mod geometry {
    pub mod rect {
        pub struct Rectangle {
            pub width: u32,
        }
    }
}

use crate::geometry::rect::Rectangle;

fn main() {
    let r = Rectangle { width: 1 };
    println!("{}", r.width);
}
```

### 案例 3：模块文件没建——E0583

```rust,editable
mod shapes;

fn main() {
    println!("{}", shapes::area(2, 3));
}
```

```text
error[E0583]: file not found for module `shapes`
 --> src/main.rs:1:1
  |
1 | mod shapes;
  | ^^^^^^^^^^^
  |
  = help: to create the module `shapes`, create file "shapes.rs" or "shapes\mod.rs"
  = note: if there is a `mod shapes` elsewhere in the crate already, import it with `use crate::...` instead

error: aborting due to 1 previous error
```

**原因**：`mod shapes;` 是一条"去磁盘找文件"的指令，编译器按 11.4 的规则去找 `src/shapes.rs` 或 `src/shapes/mod.rs`，两个都没有。注意它不会退化成"内容为空的模块"，而是直接编译失败——Rust 宁可报错也不猜你的意图。提示里的路径分隔符随平台变化，Windows 上是反斜杠。

**修复**：建出文件，并把它放进模块树。单文件等价写法如下，真实项目里对应新建 `src/shapes.rs`：

```rust,editable
mod shapes {
    pub fn area(w: u32, h: u32) -> u32 {
        w * h
    }
}

fn main() {
    println!("{}", shapes::area(2, 3));
}
```

### 案例 4：子模块里直接叫兄弟模块——E0433

```rust,editable
mod geometry {
    pub mod shape {
        pub fn name() -> &'static str {
            "shape"
        }
    }

    pub mod rect {
        pub struct Rectangle {
            pub width: u32,
        }

        impl Rectangle {
            pub fn describe(&self) -> String {
                let kind = shape::name();
                format!("{} {}", kind, self.width)
            }
        }
    }
}

fn main() {
    let r = geometry::rect::Rectangle { width: 1 };
    println!("{}", r.describe());
}
```

```text
error[E0433]: cannot find module or crate `shape` in this scope
  --> src/main.rs:15:28
   |
15 |                 let kind = shape::name();
   |                            ^^^^^ use of unresolved module or unlinked crate `shape`
   |
   = help: you might be missing a crate named `shape`
help: consider importing this module
   |
 9 +         use crate::geometry::shape;
   |

error: aborting due to 1 previous error
```

**原因**：模块不会把兄弟模块自动带进作用域。`rect` 里能直接写 `shape`，前提是 `shape` 在这个模块里有一条名字记录——要么它定义在 `rect` 内部，要么被 `use` 进来。编译器甚至先怀疑你少装了一个 crate，因为裸名字 `shape` 也可能是外部 crate 名。

**修复**：在 `rect` 模块内加一条 `use`。用 `crate::` 开头的绝对路径，比 `super::` 更不容易在挪动模块时失效：

```rust,editable
mod geometry {
    pub mod shape {
        pub fn name() -> &'static str {
            "shape"
        }
    }

    pub mod rect {
        use crate::geometry::shape;

        pub struct Rectangle {
            pub width: u32,
        }

        impl Rectangle {
            pub fn describe(&self) -> String {
                format!("{} {}", shape::name(), self.width)
            }
        }
    }
}

fn main() {
    let r = geometry::rect::Rectangle { width: 1 };
    println!("{}", r.describe());
}
```

### 案例 5：导入的名字和本地定义撞车——E0255

```rust,editable
mod geometry {
    pub fn area(w: u32, h: u32) -> u32 {
        w * h
    }
}

use crate::geometry::area;

fn area(w: u32, h: u32) -> u32 {
    w * h
}

fn main() {
    println!("{}", area(2, 3));
}
```

```text
error[E0255]: the name `area` is defined multiple times
 --> src/main.rs:9:1
  |
7 | use crate::geometry::area;
  |     --------------------- previous import of the value `area` here
8 |
9 | fn area(w: u32, h: u32) -> u32 {
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `area` redefined here
  |
  = note: `area` must be defined only once in the value namespace of this module
help: you can use `as` to change the binding name of the import
  |
7 | use crate::geometry::area as other_area;
  |                           +++++++++++++

error: aborting due to 1 previous error
```

**原因**：`use` 引入的是一个真实的名字，它和本地定义共享同一个命名空间。函数、常量、单元结构体都住在"值命名空间"里，所以两处同名就是重复定义。注意这和"同名方法与自由函数"不同——方法在类型的 impl 里，不占模块的值命名空间。

**修复**：给导入改名，或者删掉其中一个。编译器的建议是前者：

```rust,editable
mod geometry {
    pub fn area(w: u32, h: u32) -> u32 {
        w * h
    }
}

use crate::geometry::area as geometry_area;

fn area(w: u32, h: u32) -> u32 {
    w * h
}

fn main() {
    println!("{} {}", area(2, 3), geometry_area(2, 3));
}
```

## 动手练习

**练习 1**：把 11.1 的示例拆成三个文件：`src/main.rs`、`src/geometry/rect.rs`、`src/geometry/circle.rs`，加一个 `src/geometry/mod.rs` 把两者串起来。跑通后，把 `mod rect;` 改成私有再把 `Rectangle` 用 `pub use` 提到 `geometry` 层，确认 `main.rs` 一行都不用改。

**练习 2**：给 11.7 的 `shapes` 加一个方法 `fn scale(&mut self, factor: u32)`，先标成 `pub(crate)`，从 `src/main.rs` 调用它并记录错误码；再改成 `pub` 重试。解释为什么 `main.rs` 明明在同一个包里，却拿不到 `pub(crate)` 的东西。

**练习 3**：写一个 `mod prelude`，里面只有 `pub use` 语句，把 `Shape` 和 `Rectangle` 重新导出。让 `main.rs` 只写 `use crate::prelude::*;` 就能用这两个名字。然后回答：这样做省了什么，又让读者失去了什么信息？

**练习 4**：把 `shapes` 和一个新的 `cli` 包放进同一个 workspace，`cli` 用 `shapes = { path = "../shapes" }` 依赖它。分别执行 `cargo build` 与 `cargo build -p cli`，观察 `target/` 和 `Cargo.lock` 是不是只有一份。再故意把两个成员的 `serde` 版本要求写成冲突的，看 Cargo 怎么报错。

**练习 5**：给 `shapes` 加一个可选依赖和一个 feature：`serde = { version = "1", features = ["derive"], optional = true }`、`json = ["dep:serde"]`。分别用默认配置和 `cargo build --features json` 构建，用 `cargo tree -e features` 对比两次的依赖图。说明为什么 feature 只能做加法，不能改变已有函数的行为。

## 小结

- `mod` 在源码里长出一棵模块树，模块提供命名空间与可见性边界；文件只是装载模块的方式，不是模块本身
- 默认可见性是"当前模块及其后代"，`pub(super)` 限于父模块范围，`pub(crate)` 限于整个 crate，`pub(in path)` 指定祖先模块，`pub` 允许外部访问；可见性沿路径逐层检查，缺一层就整条不可达
- 结构体字段单独控制可见性；字段私有加一个 `pub fn new` 是强制校验的标准手法
- `use` 只建立别名，不复制代码，且只在所在模块（或块）生效；`as` 用于消歧，`as _` 只引入 trait 方法不引入名字
- 路径的三个起点是 `crate`、`self`、`super`；`pub use` 是重导出，能把内部层级压平成对外的门面
- `mod name;` 按声明位置去 `src/name.rs` 或 `src/name/mod.rs` 找文件；子模块文件放在父模块同名目录里；两种风格不能混用
- 包（package）由 `Cargo.toml` 定义，含至多一个库 crate 和任意多个二进制 crate；crate 是编译单元；模块是 crate 内的树
- `Cargo.toml` 的 `version = "1"` 是 `^1`；feature 必须只做加法；workspace 共享锁文件与 `target/`，换来并行编译和更细的增量粒度
- crate 边界上没有头文件：接口以 metadata 形式随 `.rlib` 产出；`pub` 只管可见性不管链接；泛型在使用方 crate 单态化，所以泛型体必须进 metadata
- 改一个 crate 会重编所有下游；crate 是增量编译的边界，CGU（开发 256 / release 16）是 crate 内部的增量粒度