# 第 33 章 声明宏 macro_rules!

> **本章目标**
>
> - 说清宏与函数的本质区别：宏在语法层展开，函数在值层调用。
> - 用片段说明符（fragment specifier）匹配表达式、类型、标识符与重复片段。
> - 解释卫生性（hygiene）如何隔离宏内外的名字，以及 `$crate` 解决什么问题。
> - 独立写出 vec! 风格宏、简化版 json! 与嵌套括号计数器三个递归宏。
> - 会读展开后的代码，并在宏、泛型与函数之间做出取舍。

函数是值层面的抽象：你把值传进去，它在运行时执行，返回一个值。宏是语法层面的抽象：你把一段 token 交给编译器，它在编译期把这段 token 改写成另一段 token，然后才轮到类型检查。理解这条分界线，本章后面那些看起来古怪的规则就都有了解释。

这条分界线带来两个直接后果。第一，宏能看到源码的形状。println! 能在编译期数清格式串里的占位符，assert_eq! 能在失败时打印出两个表达式的原文；函数做不到，因为传进函数时源码已经变成了值。第二，宏没有类型签名。编译器在展开之前不知道宏会产出什么，所以宏体的错误往往在调用点才暴露，报错也会指回宏内部。

Rust 的宏不是 C 预处理器的文本替换。macro_rules! 对 token 树做模式匹配：每条规则描述一种 token 排列，编译器按顺序尝试，匹配成功后把展开结果贴回原地。这个过程叫按示例宏（macro by example），也叫声明宏（declarative macro）。需要解析完整语法、读取属性时用的是过程宏，详见第 34 章《过程宏》。

## 33.1 宏不是函数：展开发生在类型检查之前

先看一个函数做不到的例子。下面两个版本都想打印「表达式 = 结果」，函数版要调用者手写表达式文本，宏版直接捕获源码。

```rust
fn show_fn(label: &str, value: i32) {
    println!("{} = {}", label, value);
}

macro_rules! show {
    ($e:expr) => {
        println!("{} = {}", stringify!($e), $e);
    };
}

fn main() {
    let a = 2;
    let b = 3;
    show_fn("a + b", a + b);
    show!(a + b);
    show!(a * b + 1);
}
```

输出：

```text
a + b = 5
a + b = 5
a * b + 1 = 7
```

函数版里的 "a + b" 是手写字符串，重构改名后很容易与真实表达式脱节；宏版用 stringify! 把捕获到的 token 原样转成字符串，两者不可能不同步。stringify! 是标准库自带的宏，它接受的正是 token，而不是值。

这也是宏调用要带感叹号的原因。编译器必须在一眼看到名字时就决定「这里要展开」，不能先求值再去找函数。带感叹号的调用永远在语法层，不带感叹号的调用永远在值层，两者不会混淆。

| 维度 | 函数 | 宏 |
| --- | --- | --- |
| 参数 | 值，调用前求值 | token 序列，未求值 |
| 类型检查 | 依据签名，在函数体内完成 | 展开后才知道类型 |
| 参数个数 | 固定 | 可零个、可任意多个 |
| 能否看到源码 | 不能 | 能，配合 stringify! |
| 报错位置 | 函数体或调用点 | 调用点与宏定义处各指一次 |
| 调试手段 | 断点、dbg! | cargo expand 看展开结果 |

最后一行值得强调：宏不是绕过类型系统的后门。展开结果照样参与类型检查，类型不匹配照样报 E0308，只是报错会附带一句 in this macro invocation。后面的错误案例会反复看到这种双层定位。

## 33.2 匹配语法：从 token 排列到片段

一条规则由两部分组成：括号里的模式，花括号里的展开结果。一个宏可以有多条规则，调用时编译器自上而下逐条尝试，第一条整体匹配的规则胜出。

```rust,noplayground
macro_rules! pick {
    ($x:expr) => { $x };
    ($x:expr, $y:expr) => { $x + $y };
}
```

顺序不是装饰。把宽泛的规则写在前面，后面的规则永远轮不到。反过来，把具体规则写在前面、通配规则写在最后，是几乎所有标准库宏的共同布局。另外，圆括号、方括号、花括号在匹配时地位相同，vec! 习惯用方括号只是风格。

模式里的每个变量都要声明片段说明符（fragment specifier），它决定编译器按什么语法范畴去解析接下来的 token。

| 说明符 | 匹配的内容 | 常见用途 |
| --- | --- | --- |
| expr | 任意表达式 | 传值、传条件 |
| ty | 类型 | 生成字段类型、类型别名 |
| ident | 标识符或关键字 | 生成函数名、字段名 |
| tt | 单个 token 树，含成对括号 | 递归吞 token |
| pat | 模式 | 生成 match 分支 |
| path | 路径 | 引用类型、函数、常量 |
| stmt | 语句 | 生成语句序列 |
| block | 花括号块 | 传一段代码 |
| literal | 字面量 | 常量与字符串 |
| item | 项 | 生成函数、结构体、impl |
| vis | 可见性 | 生成 pub 项 |
| meta | 属性内容 | 派生与属性宏参数 |

片段说明符不只是「匹配什么」的过滤器，它还决定捕获后的 token 如何被解析。$e:expr 捕获的是整棵表达式树，替换时作为一个单元插入，所以下面这段不会因为运算符优先级而出错。

```rust
macro_rules! max_of {
    ($a:expr, $b:expr) => {
        if $a > $b { $a } else { $b }
    };
}

fn main() {
    println!("{}", max_of!(3, 7));
    println!("{}", max_of!(1 + 2, 2 * 2));
}
```

但注意 $a 在展开里出现了两次，运行时它也被求值两次。如果实参是 next() 这类每次结果都不同的调用，两次比较与返回的就不一定是同一个值。宏的参数没有「先求值一次」的保证，需要时得自己在宏体开头用 let 绑定，这一点在 33.6 节的变参构造函数里会再遇到。

## 33.3 重复：$(...)*、$(...)+ 与分隔符

不定个数的参数靠重复片段匹配。语法是 $( ... ) 后面跟一个重复记号，记号决定次数下限与分隔符：星号表示零次或多次，加号表示一次或多次，方括号里的分隔符写在重复记号前面。

```rust
macro_rules! sum {
    () => { 0 };
    ($($x:expr),+ $(,)?) => { 0 $(+ $x)+ };
}

fn main() {
    println!("{}", sum!());
    println!("{}", sum!(1, 2, 3));
    println!("{}", sum!(1, 2, 3,));
}
```

第二条规则读作：一个表达式，后面跟着零个或多个「逗号加表达式」，最后允许一个可选的尾逗号。展开侧的 $(+ $x)+ 表示把 + 与每个捕获到的表达式依次拼起来，得到 0 + 1 + 2 + 3。展开侧的重复次数由匹配侧决定，两边的变量必须处在同一层重复里，否则编译器会报没有规则可匹配。

| 写法 | 匹配次数 | 分隔符 | 典型场景 |
| --- | --- | --- | --- |
| $($x:expr)* | 零次或多次 | 无 | 依次拼接 token |
| $($x:expr),* | 零次或多次 | 逗号 | 可空的参数列表 |
| $($x:expr)+ | 一次或多次 | 无 | 至少一个元素 |
| $($x:expr),+ | 一次或多次 | 逗号 | 非空列表 |
| $(,)? | 零次或一次 | 逗号 | 允许尾逗号 |

星号与加号的区别只在空输入上：写成 $($x:expr),* 时 sum!() 能匹配，写成 $($x:expr),+ 时同样的调用会落到别的规则，或者干脆报 no rules expected。标准库的 vec![] 用星号，因为它必须接受空向量；assert! 的格式参数用加号，因为「有消息」与「没消息」是两条不同的路径。

展开侧同样写重复，而且分隔符可以换。下面这个宏把逗号分隔的实参展开成分号分隔的语句。

```rust
macro_rules! build_vec {
    ($($x:expr),* $(,)?) => {{
        let mut v = Vec::new();
        $(v.push($x);)*
        v
    }};
}

fn main() {
    let empty: Vec<i32> = build_vec![];
    let nums = build_vec![1, 2, 3,];
    println!("{:?} {:?}", empty, nums);
}
```

这里的花括号写成 {{ 与 }}，是因为宏展开的结果要作为一整块表达式插回调用点；只写一层花括号会被当成宏规则的边界。

## 33.4 卫生性与 $crate

卫生性（hygiene）管的是名字冲突：宏体里声明的变量不该意外捕获调用点的变量，也不该被调用点捕获。Rust 的 macro_rules! 采用混合卫生（mixed-site hygiene）：局部变量受保护，类型名与项名不受保护。

```rust
macro_rules! with_temp {
    ($e:expr) => {{
        let tmp = 10;
        $e + tmp
    }};
}

fn main() {
    let tmp = 1;
    println!("{}", with_temp!(tmp));
}
```

结果是 11 而不是 2。宏体里的 tmp 与调用点的 tmp 是两个不同的绑定，编译器靠每个 token 携带的语法上下文（syntax context）区分它们，而不是靠改名。没有卫生性的话，宏里随手写的 let tmp 就可能悄悄改掉调用者的变量——这是 C 宏最常见的坑之一。

项名不受保护。下面这种写法展开两次会直接撞车：

```rust,noplayground
macro_rules! define_helper {
    () => {
        fn helper() -> i32 { 1 }
    };
}
```

连写两个 define_helper!(); 会报 E0428（重复定义），因为两个 helper 都落在调用点所在的模块里。写库宏的经验是：辅助函数塞进一个隐藏模块，或者只展开表达式，不在项层留名字。

$crate 解决的是另一个问题：路径在哪解析。宏被 #[macro_export] 导出后，会在别的 crate 里展开，宏体里的 inner::helper() 会按调用者的 crate 去找，找不到就报 E0433。

```rust
mod inner {
    pub fn helper() -> i32 {
        42
    }
}

macro_rules! call_helper {
    () => {
        $crate::inner::helper()
    };
}

fn main() {
    println!("{}", call_helper!());
}
```

$crate 只能出现在宏体里，展开后是指向定义该宏的 crate 根的路径。单 crate 的程序里它指向本 crate，跨 crate 时它指向宏的老家，两种情况写法一致。卫生性管名字，$crate 管路径，两把钥匙不能互相替代。

## 33.5 递归宏：一口一口吃掉 token

递归宏的结构很固定：一条规则吃掉开头的一小段，把剩下的 token 传给自己，直到剩下空、由终止规则收尾。这种写法通常叫 token 吞噬器（token muncher）。内部规则习惯加一个 @ 前缀，因为 @ 不是合法表达式的开头，普通调用几乎不可能误命中，等于给实现细节留了个命名空间。

先写一个 vec! 风格的版本。

```rust
macro_rules! my_vec {
    () => { Vec::new() };
    ($head:expr $(, $tail:expr)* $(,)?) => {{
        let mut v = my_vec![$($tail),*];
        v.insert(0, $head);
        v
    }};
}

fn main() {
    let empty: Vec<i32> = my_vec![];
    let nums = my_vec![1, 2, 3];
    println!("{:?} {:?}", empty, nums);
}
```

输出 [] [1, 2, 3]。每次递归吃掉第一个元素，把尾巴继续交给宏；空输入交给第一条规则。真实的标准库 vec! 还要处理容量预估与 vec![x; n] 的重复形式，骨架就是这两条规则。另外注意 my_vec![$($tail),*] 用了方括号，宏的调用分隔符不影响匹配。

第二个例子是简化版 json!，它靠 $v:tt 把嵌套结构的整组 token 原样转交。

```rust
#[derive(Debug)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

macro_rules! json {
    (null) => { Json::Null };
    (true) => { Json::Bool(true) };
    (false) => { Json::Bool(false) };
    ([$($e:tt),* $(,)?]) => { Json::Arr(vec![$(json!($e)),*]) };
    ({$($k:literal : $v:tt),* $(,)?}) => {
        Json::Obj(vec![$(($k.to_string(), json!($v))),*])
    };
    ($n:literal) => { Json::Num($n as f64) };
}

fn main() {
    let v = json!({
        "ok": true,
        "nums": [1, 2, 3],
        "nested": { "x": null }
    });
    println!("{:?}", v);
}
```

键用 $k:literal，值用 $v:tt，因为一个值可能是一整个数组或对象，而带括号的整组恰好就是一个 token 树。递归到值上时，json!($v) 把整组重新摊开，落到对应的规则上。这个版本没有字符串：宏在匹配阶段分不清字面量的类型，1 与 "x" 对 $n:literal 一视同仁，判断字面量类型要靠过程宏，详见第 34 章。

第三个例子用累加器数出最深的括号层数。

```rust
macro_rules! depth {
    (@scan $d:expr; ) => { $d };
    (@scan $d:expr; ($($inner:tt)*) $($rest:tt)*) => {{
        let here = depth!(@scan $d + 1; $($inner)*);
        let there = depth!(@scan $d; $($rest)*);
        if here > there { here } else { there }
    }};
    (@scan $d:expr; $head:tt $($rest:tt)*) => { depth!(@scan $d; $($rest)*) };
    ($($t:tt)*) => { depth!(@scan 0usize; $($t)*) };
}

fn main() {
    println!("{}", depth!(()));
    println!("{}", depth!(((()) ())));
    println!("{}", depth!(1 + 2));
}
```

第一条规则是终止：没有 token 可吃时返回累积的深度。第二条处理「以括号组开头」：进入组内时深度加一，组内与组后各求一次最大值。第三条处理普通 token，直接跳过。$d:expr 这个累加器在递归里一路传下去，是宏里做带状态遍历的常用手法。输出依次是 1、3、0。

递归宏唯一的硬要求是：每一步都要吃掉至少一个 token，并且最终能命中终止规则。少写一条终止规则，展开就永远停不下来，编译器只能用递归上限把你拦下来，也就是下一个错误案例。

## 33.6 常见用途：测试宏、断言宏、变参构造函数

宏不可替代的场景只有一类：代码的形状必须在编译期变化。日常最常用的是下面三种。

第一种，把重复的测试骨架压成一行。每个 #[test] 都必须挂在一个独立的函数上，函数名还不能重复，这正好是宏擅长的活。

```rust
fn parse(s: &str) -> i32 {
    s.trim().parse().unwrap_or(0)
}

macro_rules! test_case {
    ($name:ident, $input:expr, $want:expr) => {
        #[test]
        fn $name() {
            assert_eq!(parse($input), $want);
        }
    };
}

test_case!(parses_zero, "0", 0);
test_case!(parses_negative, "-7", -7);
test_case!(falls_back_to_zero, "x", 0);

fn main() {
    println!("{}", parse("42"));
}
```

$name:ident 直接把调用时写的名字变成函数名，测试失败时报告的名字就是 case 名。三条声明展开成三个测试函数，这是函数做不到的：函数的参数无法变成函数名。

第二种，带上下文的断言宏。标准库的 assert_eq! 会打印两边的值，但比较浮点数时需要容差，失败时你还想知道差了多少。

```rust
macro_rules! assert_close {
    ($a:expr, $b:expr, $eps:expr) => {{
        let (a, b, eps): (f64, f64, f64) = ($a, $b, $eps);
        assert!(
            (a - b).abs() <= eps,
            "{} 与 {} 的差超过 {}",
            a, b, eps
        );
    }};
}

fn main() {
    assert_close!(0.1 + 0.2, 0.3, 1e-9);
    println!("通过");
}
```

三个参数各自在 let 里求值一次，后面的断言用的是绑定后的变量，所以带副作用的实参也不会被算两遍。类型注解 (f64, f64, f64) 不能省：没有它，编译器不知道 abs 是 f64 还是 f32 的方法，会报 E0689。

第三种，变参构造函数。结构体字段固定，元素个数却不定时，函数签名写不出来。

```rust
#[derive(Debug)]
struct Matrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

macro_rules! matrix {
    ($rows:expr, $cols:expr; $($x:expr),* $(,)?) => {{
        let rows = $rows;
        let cols = $cols;
        let data = vec![$($x as f64),*];
        assert_eq!(data.len(), rows * cols, "元素个数与行列数不符");
        Matrix { rows, cols, data }
    }};
}

fn main() {
    let m = matrix!(2, 2; 1.0, 2.0, 3.0, 4.0);
    println!("{:?}", m);
}
```

$rows 与 $cols 先绑定再使用，避开了 33.2 节提到的重复求值。断言把「形状」这条不变量写进了构造函数：行列数对不上时，错误在构造的那一刻就暴露，而不是等到后续计算出 NaN。三种用法的共同点是，它们都在生成代码的形状，而不是在处理值。

## 33.7 深入：展开后的代码与调试手段

宏难调试，因为源码里看不到它。最直接的办法是把展开结果打印出来。cargo expand 是社区标准工具，它不是标准库的一部分，需要单独安装。

```console
$ cargo install cargo-expand
$ cargo expand --bin demo
```

它会展开每个宏并做基础格式化。另一个入口是编译器的内部选项，只在 nightly 上可用。

```console
$ RUSTFLAGS="-Zunpretty=expanded" cargo +nightly build
```

这两个办法看到的是同一份东西。注意展开结果里含着卫生性信息与内部路径，通常不能原样编译，只适合阅读。

```text
// show!(a + b) 展开后大致相当于
{
    ::std::io::_print(format_args!("{} = {}\n", "a + b", a + b));
};
```

如果手边没有 nightly 也不想装工具，stable 上还有两个土办法。一是故意写错类型，让报错告诉你展开结果的类型，报错里会带 in this macro invocation 指向调用点。二是在宏体里临时插一句 compile_error!(stringify!($x))，编译时就能看到某个片段到底匹配到了什么，比通读展开代码快得多。

调试宏的顺序建议是：先确认命中了哪条规则，再看捕获到的 token 是什么，最后才去读展开后的整体代码。反过来做，往往会在几百行展开结果里迷路。

## 33.8 边界与分工：什么时候不该写宏

第一条边界：宏不做计算。macro_rules! 只重排 token，宏体不会被执行，也没有类型。想按元素个数决定数组长度、按位宽生成实现，要靠 const 泛型与 trait，也就是类型层面的编程，详见第 36 章《类型系统进阶》。

第二条边界：错误信息难懂。展开后的问题常常同时指向调用点与宏定义里的两三行，真正的起因可能只是调用时多写了一个逗号。宏体越长，这层间接越贵。

第三条边界：工具支持弱。rust-analyzer 能展开宏，但补全、跳转、重命名在宏体里都不完整，调试器也无法停在宏里单步。宏用得多，IDE 的体验就下降。

于是有下面这个优先级。

| 需求 | 首选工具 |
| --- | --- |
| 复用一段逻辑 | 函数 |
| 对多种类型复用同一逻辑 | 泛型 |
| 按类型定制行为 | trait |
| 参数个数不定 | 宏 |
| 生成名字、生成代码 | 宏 |
| 自定义语法 | 宏，复杂时用过程宏 |

一句话总结：能用函数就不用宏，能用泛型就不用宏，只有当参数个数、名字或者语法形状必须在编译期变化时，才轮到宏。标准库自己也大量使用宏，但绝大多数只在内部实现里出现，对外暴露的是函数与 trait；少数几个公开的宏，例如 vec! 与 println!，恰恰是因为它们需要语法层的灵活性，函数替代不了。

宏还有一项隐性成本：编译时间。每个调用点都要单独展开并重新类型检查，展开出来的代码也不会被自动合并，一个被调用几百次的宏会让编译明显变慢。

### 一个上限，两处报错

宏展开有递归上限，类型检查也有。两者都受 #![recursion_limit] 控制，撞上时给出的错误却不一样。宏这边是 recursion limit reached，trait 求解这边是 E0275。

```rust,noplayground
trait Marker {}
impl<T: Marker> Marker for T {}

fn need<T: Marker>() {}

fn main() {
    need::<u8>();
}
```

```text
error[E0275]: overflow evaluating the requirement `u8: Marker`
 --> src/main.rs:7:5
  |
7 |     need::<u8>();
  |     ^^^^^^^^^^^^
  |
note: required by a bound in `need`
 --> src/main.rs:4:12
  |
4 | fn need<T: Marker>() {}
  |            ^^^^^^ required by this bound in `need`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0275`.
```

上面这个 impl 对任何满足 Marker 的类型都实现 Marker，于是证明 u8 满足 Marker 又需要先证明 u8 满足 Marker，求解没有尽头。这类错误的根因几乎总是无限递归，调高上限通常只是把报错推迟，正确做法是删掉那条自我引用的规则。

## 典型错误案例

下面五个案例都是真实编译的结果，报错文本来自 rustc 1.100，文件名按项目惯例写成 src/main.rs。宏的错误有个共同特征：报错会同时标出调用点和宏定义里的位置。

### 案例 1：宏展开后的返回值类型不匹配——E0308

```rust,editable
macro_rules! ret {
    ($e:expr) => {
        $e
    };
}

fn answer() -> i32 {
    ret!("42")
}

fn main() {
    println!("{}", answer());
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:8:10
  |
7 | fn answer() -> i32 {
  |                --- expected `i32` because of return type
8 |     ret!("42")
  |          ^^^^ expected `i32`, found `&str`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：宏没有类型签名，$e:expr 接受任何表达式，所以错误不会在宏调用处被拦下，只能等展开结果参与类型检查时才暴露。报错指向第 8 行的调用点，而不是宏定义，因为不匹配的是展开结果与返回类型。

**修复**：问题在调用方，不在宏。要么改返回类型，要么传一个整数。宏本身无法声明「只接受 i32」，想限制类型只能靠宏体里的额外约束，或者干脆换成函数。

```rust,editable
macro_rules! ret {
    ($e:expr) => {
        $e
    };
}

fn answer() -> &'static str {
    ret!("42")
}

fn main() {
    println!("{}", answer());
}
```

</details>

### 案例 2：调用时多给了一个参数——no rules expected

```rust,editable
macro_rules! point {
    ($x:expr) => {
        ($x, 0)
    };
}

fn main() {
    let p = point!(1, 2);
    println!("{:?}", p);
}
```

```text
error: no rules expected `,`
 --> src/main.rs:8:21
  |
1 | macro_rules! point {
  | ------------------ when calling this macro
...
8 |     let p = point!(1, 2);
  |                     ^ no rules expected this token in macro call
  |
note: while trying to match meta-variable `$x:expr`
 --> src/main.rs:2:6
  |
2 |     ($x:expr) => {
  |      ^^^^^^^

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：规则只描述了一个表达式，之后的逗号与 2 没有规则能消费。宏匹配是整体匹配，不会「尽量匹配、丢弃剩余」；多出来的 token 会直接报错。note 一行指出了卡在哪个元变量上，这是排查这类错误最快的线索。

**修复**：补一条双参数规则。规则按书写顺序尝试，短规则写在前面不会抢走双参数的调用，因为 expr 片段遇到逗号就停下，整条规则匹配失败后编译器会继续试下一条。

```rust,editable
macro_rules! point {
    ($x:expr) => {
        ($x, 0)
    };
    ($x:expr, $y:expr) => {
        ($x, $y)
    };
}

fn main() {
    let p = point!(1, 2);
    let q = point!(1);
    println!("{:?} {:?}", p, q);
}
```

</details>

### 案例 3：调用时少给了一个参数——unexpected end of macro invocation

```rust,editable
macro_rules! pair {
    ($a:expr, $b:expr) => {
        ($a, $b)
    };
}

fn main() {
    let p = pair!(1);
    println!("{:?}", p);
}
```

```text
error: unexpected end of macro invocation
 --> src/main.rs:8:20
  |
1 | macro_rules! pair {
  | ----------------- when calling this macro
...
8 |     let p = pair!(1);
  |                    ^ missing tokens in macro arguments
  |
note: while trying to match `,`
 --> src/main.rs:2:13
  |
2 |     ($a:expr, $b:expr) => {
  |             ^

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：模式需要一个逗号再接第二个表达式，但 token 流提前结束，连逗号都没有，所以报的是「宏调用意外结束」，不是「找不到值」。它和案例 2 是一对：一个嫌多，一个嫌少。同样，报错里的 note 精确指出缺的是哪个 token。

**修复**：宏没有默认参数，想允许少给就必须单独写一条规则。下面给单参数版本补上默认值。

```rust,editable
macro_rules! pair {
    ($a:expr) => {
        ($a, 0)
    };
    ($a:expr, $b:expr) => {
        ($a, $b)
    };
}

fn main() {
    let p = pair!(1);
    let q = pair!(1, 2);
    println!("{:?} {:?}", p, q);
}
```

</details>

### 案例 4：宏体里的名字在调用点不存在——E0425

```rust,editable
macro_rules! double_x {
    () => {
        x * 2
    };
}

fn main() {
    let x = 21;
    println!("{}", double_x!());
}
```

```text
error[E0425]: cannot find value `x` in this scope
 --> src/main.rs:3:9
  |
1 | macro_rules! double_x {
2 |     () => {
3 |         x * 2
  |         ^ not found in this scope
...
9 |     println!("{}", double_x!());
  |                    ----------- in this macro invocation
  |
help: an identifier with the same name exists, but is not accessible due to macro hygiene
 --> src/main.rs:8:9
  |
8 |     let x = 21;
  |         ^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：卫生性在起作用。宏体里的 x 属于宏定义处的语法上下文，调用点的 let x 虽然同名，但对它不可见。编译器甚至专门给了一句提示：有一个同名标识符，但被宏卫生性挡住。这不是 bug，而是宏不会意外捕获调用点变量的保证。

**修复**：需要调用点的值时，通过参数传进去，而不是在宏体里直接引用名字。参数是 token，传什么就展开成什么。

```rust,editable
macro_rules! double {
    ($x:expr) => {
        $x * 2
    };
}

fn main() {
    let x = 21;
    println!("{}", double!(x));
}
```

</details>

### 案例 5：递归宏没有终止条件——recursion limit

```rust,editable
macro_rules! countdown {
    ($n:expr) => {
        countdown!($n - 1)
    };
}

fn main() {
    countdown!(3);
}
```

```text
error: recursion limit reached while expanding `countdown!`
 --> src/main.rs:3:9
  |
1 | macro_rules! countdown {
2 |     ($n:expr) => {
3 |         countdown!($n - 1)
  |         ^^^^^^^^^^^^^^^^^^
...
8 |     countdown!(3);
  |     ------------- in this macro invocation
  |
  = help: consider increasing the recursion limit by adding a `#![recursion_limit = "256"]` attribute to your crate (`main`)

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：宏参数是 token，不是值。$n - 1 只是把 3 - 1 这串 token 原样交给下一层，再下一层得到 3 - 1 - 1，永远算不出 2。递归要能停下来，每一层都必须让 token 变少，而不是让数值变小。help 建议调高 recursion_limit，那只会让爆炸来得晚一些。

**修复**：数值递减交给运行时；如果确实要递归展开，就用 tt 逐个吃掉 token，像 33.5 节的 depth! 那样保证每层都在缩短输入。

```rust,editable
macro_rules! countdown {
    ($n:expr) => {{
        let mut n = $n;
        while n > 0 {
            println!("{}", n);
            n -= 1;
        }
        println!("发射");
    }};
}

fn main() {
    countdown!(3);
}
```

</details>

## 动手练习

1. 写一个 hashmap! 宏，支持空调用与 hashmap!{"a" => 1, "b" => 2}，要求允许尾逗号，并解释为什么键值对必须成组重复。
2. 给 33.5 节的 depth! 加上方括号：让 depth!([[[]]]) 也返回 3。写完后回答，为什么不能把圆括号与方括号合成一条规则。
3. 把 assert_close! 改成同时支持 f32 与 f64：宏只做薄壳，比较逻辑放进泛型函数 compare(a: T, b: T, eps: T)。改完后对比两种写法的报错质量。
4. 用 cargo expand 展开一次 println!，数一数它展开成多少行；再展开本章的 my_vec!，观察递归展开后的嵌套结构。
5. 在你自己的项目里找一段重复代码，判断它该用函数、泛型还是宏，并把判断依据写进注释。

## 小结

- 宏在语法层展开，函数在值层调用；宏没有类型签名，类型错误只能等展开之后才发现。
- 规则自上而下匹配，第一条整体匹配的胜出，没有回溯；片段说明符决定 token 按什么语法范畴解析。
- 重复写成 $(...)*（零次或多次）与 $(...)+（一次或多次），$(,)? 专门用来吞掉可选的尾逗号。
- 卫生性保护局部变量，不保护项名；$crate 让宏体里的路径指向定义宏的那个 crate。
- 递归宏每一步都必须吃掉 token，终止规则不能缺；@ 前缀把内部规则与公开入口分开。
- 调试时先确认命中的规则，再用 cargo expand 或 nightly 的 -Zunpretty=expanded 看展开结果。
- 能用函数就不用宏，能用泛型就不用宏；宏只负责语法形状必须在编译期变化的那部分。