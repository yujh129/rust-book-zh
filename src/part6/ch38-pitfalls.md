# 第 38 章 常见陷阱与编译错误速查

> **本章目标**
>
> - 按 error、note、help 的顺序读一条 rustc 报错，判断该改哪一行而不是乱改
> - 认出所有权与借用类错误背后的同一条规则，不必死背十个错误码
> - 说清 debug 与 release 下整数溢出的不同行为，以及 as 截断、负数取模、浮点比较的规则
> - 按字符边界而不是字节下标安全地切分字符串，分清 String、&str、OsStr、PathBuf 的职责
> - 列出并发程序里死锁、锁中毒、跨 await 持锁的成因，并知道对应的替代方案
> - 用一张按症状查的索引表，从现象倒推原因，再跳到对应章节

前面三十多章各自讲清了一类语法。这一章换个角度，从"出问题的地方"往回看：哪些坑几乎每个人都会踩一次，踩的时候 rustc 会打印什么，看到那段输出又该怎么想。

大量初学阶段的挫败感并不来自语法本身，而来自不会读报错。rustc 的报错信息密度很高，第一眼看过去像一堵墙，但它的结构是固定的：一条错误讲一件事，note 补背景，help 给改法。学会按顺序读，你就从"猜"变成"查"。

另一种坑是编译通过但运行结果不对：整数悄悄溢出、字符串切在半个汉字上、浮点比较返回 false、两个线程各持一把锁互等。这些问题编译器帮不了你，只能靠知道规则来避免。本章不重复前面讲过的语法，而是把这些散落各处的边界条件收拢成一张地图：遇到具体问题时，先看 38.9 的索引表定位，再回到对应小节。

## 38.1 读报错：先看哪一行

rustc 打印的一段报错由四部分组成。拿一个真实的例子看：

```text
error[E0502]: cannot borrow `scores` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:13
  |
3 |     for s in &scores {
  |              ------- immutable borrow occurs here
4 |         if *s < 25 {
5 |             scores.push(*s + 100);
  |             ^^^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here
  |
  = help: consider using an index-based loop instead, or collecting modifications into a separate collection
```

四部分分别是：

1. **第一行**：`error[E0502]` 是错误码，后面一句是人话总结。错误码是稳定的，可以直接用 `rustc --explain E0502` 查长解释。
2. **位置行** `--> src/main.rs:5:13`：文件、行、列。编译器认为错误发生在这一处，但真正要改的常常是另一处。
3. **带竖线的代码片段**：记号 `^` 指出主错误位置，记号 `-` 标出相关位置。上面这段里，第 3 行的 `scores` 标了减号，说明它先被不可变借用；第 5 行标了 `^`，说明错误发生在"想可变借用"这一处。两个位置拼起来才是完整故事。
4. **note / help**：note 补规则背景，help 往往直接给出可用的修复方向。E0502 的 help 直接给出两个方向：改用下标循环，或先把修改收集到另一个集合。

读报错有三条顺序规则，比任何技巧都重要：

- **只修第一个 error**。后面的错误经常是第一个的连锁反应。比如一个函数签名写错，调用它的十处全报 E0308，修好签名这十处会一起消失。逐条硬修会做大量无用功。
- **先看两处 span，再看 note/help**。`occurs here` 与 `later used here` 这类句子的先后顺序，就是借用从开始到冲突的时间线。
- **读不懂就 `--explain`**。`rustc --explain E0502` 会打印一段带示例的英文说明，虽然长，但结构比一行报错清楚得多。

```console
$ rustc --explain E0502
A variable already borrowed with a certain mutability (either mutable or
immutable) was borrowed again with a different mutability.
```

报错里的英文句子其实就那么几十个句型，见的次数多了，看关键词就能定位：`moved`、`borrowed`、`does not live long enough`、`cannot infer type`、`trait bound not satisfied`。下面一节把这些关键词和错误码对应起来。

## 38.2 深入：错误码地图与借用检查器在查什么

上一节说"看关键词就能定位"。这张表把最常见的错误码和关键词对应起来，第四列是本章或前文展开的地方：

| 错误码 | 报错里的关键词 | 检查器在问什么 | 详见 |
|---|---|---|---|
| E0382 | use of moved value / borrow of moved value | 这个值已经被移走，你为什么还在用 | 38.3 |
| E0502 | cannot borrow as mutable because it is also borrowed as immutable | 同一时刻既有共享借用又有可变借用 | 38.3、38.4 |
| E0499 | cannot borrow as mutable more than once at a time | 两个可变借用的区间重叠了 | 38.3 |
| E0597 | does not live long enough | 被借用的数据比借用先销毁 | 38.3、第 17 章 |
| E0515 | cannot return value referencing local variable | 函数返回了指向自己栈帧的引用 | 38.3 |
| E0106 | missing lifetime specifier | 引用没有可推导的生命周期 | 第 17 章 |
| E0277 | trait bound is not satisfied / cannot be sent between threads | 类型缺少某个能力（trait） | 38.4、38.7、38.8 |
| E0308 | mismatched types | 两处类型对不上 | 38.4、38.5 |
| E0599 | no method named 方法名 found | 方法不存在，通常是 trait 没导入 | 38.8 |
| E0004 | non-exhaustive patterns | match 没覆盖全部取值 | 38.3 |
| E0282 / E0283 | type annotations needed | 推断不出唯一的类型 | 38.4 |
| E0716 | temporary value dropped while borrowed | 临时值在语句末尾就销毁了 | 38.3 |

这些码看起来分散，其实大半来自同一条规则。借用检查器每次只问三个问题：

1. **这个值的所有者是谁**，它在什么时候 drop。
2. **现在有几个借用**，其中有几个是 `&` 几个是 `&mut`。规则是：任意时刻，要么任意多个 `&`，要么恰好一个 `&mut`，两者不能并存。
3. **每个借用活多久**。借用必须在所有者 drop 之前结束。

E0382 是第 1 条被违反，E0502 和 E0499 是第 2 条，E0597、E0515、E0106、E0716 是第 3 条。所以修一个 E0502 的经验，往往直接能修 E0499：先问"这两个借用能不能错开"，再问"能不能把其中一个改成共享借用"。

"借用活多久"这一条在 2018 年之后由 **NLL**（non-lexical lifetimes）负责。它的判断依据是"最后一次使用"，不是"花括号结束"：

```rust
fn main() {
    let mut data = vec![1, 2, 3];
    let first = &data[0];
    println!("{}", first);   // first 最后一次使用
    data.push(4);            // 借用到这里已经结束，可以可变借用
    println!("{:?}", data);
}
```

把最后一行改成 `println!("{} {:?}", first, data)`，借用就被拖到了 `push` 之后，E0502 立刻回来。这也是为什么"在中间插一行 `println!`"有时会让代码突然编译不过：你无意中延长了一个借用的存活区间。

理解这一点，读报错的方式也会变：不要盯着号子所在的那一行，而要找出**两个 span 之间**的那段区间。编译器画出的区间，就是它认为借用被"用到了"的范围。

## 38.3 所有权与借用陷阱

这一节的五个坑都属于同一族：值去哪了、借用借了多久、闭包替你借了什么。它们对应的错误码不同，但改动思路相通。

### 移动后继续使用

最常见的 E0382：

```rust,noplayground
fn main() {
    let name = String::from("ana");
    let alias = name;
    println!("{}", name);   // 在这里继续用 name
}
```

`String` 没有实现 `Copy`，`let alias = name` 是一次**移动**：堆缓冲的所有权转给了 `alias`，`name` 随即失效。修法有三种，选哪种取决于你到底要不要两份数据：

```rust
fn main() {
    // 方案一：确实需要两份，显式克隆
    let name = String::from("ana");
    let alias = name.clone();
    println!("{} {}", name, alias);

    // 方案二：只需要读，借出去而不是搬走
    let original = String::from("bo");
    let len = original.len();
    println!("{} {}", original, len);
}
```

方案三是不写 `let alias = name`，直接重新组织逻辑。要注意：**编译器只会告诉你"移动后又被用了"，不会告诉你该用哪种修法**。看到 E0382 先问"我需要这个值的所有权，还是只借用一下"，答案会自己浮现。

对实现了 `Copy` 的类型（`i32`、`bool`、`f64`、由它们组成的元组和结构体），赋值是复制而非移动，同样的代码不会报错。这也解释了为什么第 8 章建议：小而纯的数据实现 `Copy`，含 `String`、`Vec` 的类型只实现 `Clone`。

### 临时值的生命周期

第二个高频坑是临时值在语句结束时就被销毁。借用它的引用撑不到下一行：

```rust,noplayground
use std::cell::RefCell;

fn main() {
    let cell = RefCell::new(String::from("  hi  "));
    let trimmed = cell.borrow().trim();   // borrow() 的返回值是临时值
    println!("{}", trimmed);
}
```

`cell.borrow()` 返回一个 `Ref<String>`，它是个临时值，在这条语句结束时就被 drop；而 `trim()` 返回的 `&str` 正指向它。编译器报 E0716。同样的模式也出现在 `match` 的尾表达式上：

```rust,noplayground
use std::cell::RefCell;

fn main() {
    let cell = RefCell::new(vec![1, 2, 3]);
    let last = match cell.borrow().last() {
        Some(v) => v,   // v 指向临时的 Ref，match 一结束就悬空
        None => &0,
    };
    println!("{}", last);
}
```

修复思路有两个：要么把守卫（guard）绑定到变量，让它的生命周期延长到需要的地方；要么当场把值复制出来，别把引用带出去：

```rust
use std::cell::RefCell;

fn main() {
    let cell = RefCell::new(String::from("  hi  "));
    let trimmed = cell.borrow().trim().to_string();   // 复制成 String
    println!("[{}]", trimmed);
}
```

这条规则的通用版本是：**任何返回引用的临时值，都不能让引用活过这条语句**。`&vec![1, 2, 3]`、`&String::from("x")`、`format!("{}", x).as_str()` 都属于同一类问题。

### 闭包捕获了什么

闭包会自动选择"够用的最小捕获方式"：只用只读就捕获 `&`，要修改就捕获 `&mut`，要拿走就按值捕获。问题在于闭包会一直持有这个借用，直到它最后一次被调用；如果后续调用与另一处借用重叠，冲突就出现了：

```rust,noplayground
fn main() {
    let mut list = vec![1, 2, 3];
    let mut add = |x: i32| list.push(x);   // 闭包持有 list 的可变借用
    println!("{:?}", list);               // 这里又想不可变借用 list
    add(4);
}
```

报错是 E0502。修法通常是缩短闭包的存活区间（先调用完再打印），或者去掉闭包、改用普通函数加显式参数。`move` 关键字会把捕获改成按值移动，适用于把数据交给新线程；代价是原变量随之失效，而且当闭包只实现 `FnOnce` 时只能调用一次。

### `&String` 与 `&str`

函数参数写 `&String` 会让调用方受限制：只有 `String` 能传，`&str` 字面量反而不行。写 `&str` 两种都能接：

```rust
fn length_of(text: &str) -> usize {
    text.chars().count()
}

fn main() {
    let owned = String::from("hello");
    println!("{}", length_of(&owned));   // &String 自动转成 &str
    println!("{}", length_of("world"));  // 字面量本来就是 &str
}
```

能自动转换的原因是 **Deref 强制转换**（deref coercion）：`&String` 会先被解引用成 `&str` 再传参。同样的规则适用于 `&Vec<T>` 到 `&[T]`、`&PathBuf` 到 `&Path`。所以经验规则是：**参数用最弱的借用类型**（`&str`、`&[T]`、`&Path`），把调用方的自由度留出来。反向不行：拿到 `&str` 想当 `&String` 用，编译器不会替你造一个 `String`。

### clone 掩盖设计问题

`.clone()` 是万能的止错按钮，但它可能只是把编译器的追问按下去，而没回答问题。每次写下 `clone` 前先问一句：**这两处真的需要各自持有一份数据吗？**

```rust
fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
}

fn main() {
    let line = String::from("hello world");
    let word = first_word(&line);   // 借出去，零分配
    println!("{}", word);
}
```

这个版本没有克隆：`first_word` 返回的 `&str` 指向 `line` 内部的缓冲，只要 `line` 活着就有效。如果改成返回 `String`，每次调用都要分配一次内存，在一万行的循环里代价立刻显现。

什么情况下克隆是对的：数据本身很小（几个字节的 ID）、确实需要两份独立副本、或者把值跨线程/跨异步任务发送需要 `'static` 所有权。什么情况下是错的：在循环里克隆整个集合、为了绕过借用检查而克隆、克隆之后再也没改过原值。

## 38.4 迭代器陷阱

### 三个迭代方法的区别

`Vec<T>` 上同时存在 `iter()`、`iter_mut()`、`into_iter()`，它们的产出和代价完全不同：

| 调用 | 产出元素 | 迭代结束后集合还能用吗 | 底层借用 |
|---|---|---|---|
| iter() | &T | 能 | 共享借用 |
| iter_mut() | &mut T | 能 | 可变借用 |
| into_iter() | T | 不能，已被消费 | 拿走所有权 |

`for x in 集合` 这个语法糖调用的是 `IntoIterator::into_iter`，而 `&Vec<T>` 和 `&mut Vec<T>` 也实现了 `IntoIterator`，所以 `for x in &v`、`for x in &mut v` 分别等价于 `v.iter()`、`v.iter_mut()`。把这三者混用，就会出现"明明只是遍历，变量却没了"：

```rust
fn main() {
    let mut v = vec![1, 2, 3];

    let sum: i32 = v.iter().sum();          // 只读借用，v 仍可用
    for x in v.iter_mut() {
        *x += sum;                           // 可变借用，改的是原位
    }

    for x in v {                             // into_iter：拿走所有权
        print!("{} ", x);
    }
    println!();
}
```

最后那个循环之后 `v` 就已经失效了。如果还想继续用，就写 `for x in &v`。

### 边遍历边修改集合

遍历时修改集合，是初学者最容易撞上的 E0502（完整报错见案例 1）。根源在于两个借用重叠：`&v` 的共享借用还在，`v.push` 又要求可变借用。修法是让"读"和"写"分两步：

```rust
fn main() {
    // 方案一：先把要加的元素收集到新集合，再一次性合并
    let mut v = vec![1, 2, 3];
    let additions: Vec<i32> = v.iter().filter(|x| **x < 3).map(|x| x * 10).collect();
    v.extend(additions);
    println!("{:?}", v);

    // 方案二：用 retain 原地删除，它内部保证遍历与删除不冲突
    let mut nums = vec![1, 2, 3, 4, 5, 6];
    nums.retain(|x| x % 2 == 0);
    println!("{:?}", nums);
}
```

标准库已经为"遍历中删元素"准备了安全的入口：`retain`、`drain`、`extract_if`。原地修改单个元素则用 `iter_mut`。**不要**为了绕过借用检查而给 `len` 做缓存然后按下标加元素，那只是把编译期错误换成了运行期越界。

### collect 的类型歧义

`collect` 的签名是 `fn collect<B: FromIterator<Self::Item>>(self) -> B`：它能变成的目标类型非常多，`Vec`、`HashSet`、`BTreeMap`、`String` 都行。如果上下文不能唯一确定 `B`，编译器就报 E0283：

```rust,noplayground
fn main() {
    let words = vec!["a", "bb", "ccc"];
    let lens = words.iter().map(|w| w.len()).collect();   // 变成什么？
    println!("{:?}", lens);
}
```

`{:?}` 只要求 `Debug`，而 `Vec<usize>`、`HashSet<usize>`、`VecDeque<usize>` 都满足，编译器不猜。修法是明确目标类型，两种写法都可以：

```rust
use std::collections::HashSet;

fn main() {
    let words = vec!["a", "bb", "ccc"];

    let lens: Vec<usize> = words.iter().map(|w| w.len()).collect();
    println!("{:?}", lens);

    let unique = words.iter().copied().collect::<HashSet<&str>>();
    println!("{}", unique.len());
}
```

E0282 是同一类问题的"更早版本"：变量完全没有被约束过。`let v = Vec::new(); println!("{:?}", v);` 报的就是 E0282，因为元素类型从头到尾没出现过。两种错误的修法一样：给出类型标注或者 turbofish。

## 38.5 整数与浮点陷阱

### 溢出：debug 与 release 行为不同

整数溢出有三层行为，必须分清：

- **编译期能算出来的常量溢出**是编译错误。`let a: u8 = 255; let b = a + 1;` 直接报 `this arithmetic operation will overflow`。
- **运行期溢出在 debug 构建里 panic**：
```text
thread 'main' panicked at src/main.rs:3:13:
attempt to add with overflow
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```
- **运行期溢出在 release 构建里按补码回绕**。同一个程序 `cargo run --release` 输出 0，不报错。

这不是 Rust 的 bug，而是有意的取舍：debug 下暴露问题，release 下不插入检查、换取性能。代价是同一份代码在两种构建下结果不同，测试如果在 release 下跑，就可能漏掉溢出。想显式选择行为，用四个明确的方法：

```rust
fn main() {
    let a = std::hint::black_box(255u8);
    println!("{:?}", a.checked_add(1));      // None：溢出返回 None
    println!("{}", a.wrapping_add(1));       // 0：按补码回绕
    println!("{}", a.saturating_add(1));     // 255：钉在边界
    println!("{:?}", a.overflowing_add(1));  // (0, true)：结果加溢出标志
}
```

需要"绝不允许悄悄错"的场合（解析用户输入、金额计算、协议字段），用 `checked_*` 返回 `Option`，把错误显式向上传。需要"确定回绕"的场合（哈希、加密）用 `wrapping_*`。想强制 release 也 panic，可以在 `Cargo.toml` 里开 `overflow-checks = true`。

### as 截断

`as` 是无检查的强制转换，不同位宽、整数与浮点之间都能转，规则是"截断 + 重解释"，不会因为你丢数据而报错：

```rust
fn main() {
    let big: u32 = 300;
    println!("{}", big as u8);        // 44：取低 8 位
    let neg: i8 = -1;
    println!("{}", neg as u8);        // 255：补码按位重新解释
    println!("{}", 3.99f64 as i32);   // 3：向零截断，不是四舍五入
    println!("{}", 1e30f64 as i32);   // 2147483647：超出范围时饱和到边界
}
```

最后一行是需要特别注意的边界：自 Rust 1.45 起，浮点转整数的 `as` 在越界时**饱和**（负数取 `MIN`，正数取 `MAX`），早期版本是未定义行为。整数转整数没有饱和概念，永远是截断。跨语言移植时这一点容易踩坑：C 的越界浮点转整数是未定义行为，Rust 给了确定的答案。

业务代码里更该用 `TryFrom`/`try_into()`：`let small: u8 = big.try_into().unwrap_or(0);` 会在 300 转 `u8` 时明确失败，而不是悄悄变成 44。

### 负数取模与除法

Rust 的 `%` 是"取余"不是数学上的"取模"：结果符号跟被除数走。`/ ` 则向零截断。这两个行为合起来，做环形索引、分页、时间轮换算时很容易算错：

```rust
fn main() {
    println!("{}", -7 / 3);                  // -2：向零截断
    println!("{}", -7 % 3);                  // -1：余数符号跟被除数
    println!("{}", (-7i32).div_euclid(3));   // -3：商的绝对值变小
    println!("{}", (-7i32).rem_euclid(3));   // 2：结果永远非负
}
```

需要"结果落在 0 到除数之间"时用 `rem_euclid`，需要配套的商用 `div_euclid`。另外 `x % 0` 和 `i32::MIN / -1` 都会 panic，除以变量前要确认它不为零。

### 浮点比较

`f64` 不实现 `Eq` 和 `Ord`，因为 `NaN` 不等于任何值，包括它自己；而且十进制小数大多不能用二进制精确表示：

```rust
fn main() {
    let sum = 0.1f64 + 0.2;
    println!("{}", sum == 0.3);                 // false：0.1 与 0.2 本身就不精确
    println!("{}", (sum - 0.3).abs() < 1e-9);   // true：用容差比较

    let nan = sum - sum;
    println!("{}", nan.is_nan());               // true：NaN 只能用 is_nan 判断
}
```

三条经验规则：

1. **不要用 `==` 比较浮点结果**，改用绝对或相对容差；`f64::EPSILON` 是最小精度单位，不是通用容差，别拿它当"差不多"的阈值。
2. **要排序就用 `total_cmp`**。`partial_cmp` 遇到 `NaN` 返回 `None`，直接 `sort_by` 会 panic；`total_cmp` 给出包含 `NaN` 的全序，`[NaN, 1.0, 0.5]` 排序后是 `[0.5, 1.0, NaN]`。
3. **要当 `HashMap` 的键，就别用浮点数**。`f64` 不实现 `Hash`+`Eq`，硬要比较可以用 `to_bits()`，但要先处理 `-0.0` 与 `NaN`。

工程上的替代品是定点数或整数分（把"元"存成"分"、把"秒"存成"毫秒"），从源头绕开浮点误差。

## 38.6 字符串与路径陷阱

### 字节、字符与边界

`String` 内部是 UTF-8 字节数组，`len()` 返回的是**字节数**，不是字符数。一个汉字通常占 3 个字节，带声调的拉丁字母占 2 个。按字节下标切片，只要切点不在字符边界上，运行期就会 panic：

```rust
fn main() {
    let s = String::from("你好，Rust");
    println!("字节数 = {}", s.len());
    println!("字符数 = {}", s.chars().count());
    for (i, ch) in s.char_indices() {
        print!("{}:{}  ", i, ch);
    }
    println!();
    println!("{}", &s[0..3]);   // 0..3 正好是"你"的三个字节
}
```

`char_indices()` 给出的每个下标都是安全切点。反过来，随手写一个下标就会出事：

```rust,noplayground
fn main() {
    let s = String::from("你好");
    println!("{}", &s[0..1]);   // 编译通过，运行期 panic
}
```

```text
thread 'main' panicked at src/main.rs:3:22:
end byte index 1 is not a char boundary; it is inside '你' (bytes 0..3 of string)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

注意这是运行期错误，编译器拦不住——因为切点是不是边界取决于运行时的字符串内容。需要"切不了就返回 None"的语义时用 `get`：

```rust
fn main() {
    let s = "héllo";
    println!("len = {}", s.len());              // 6 字节
    println!("chars = {}", s.chars().count());  // 5 字符
    println!("{:?}", s.get(0..1));              // Some("h")
    println!("{:?}", s.get(1..2));              // None：切在 é 中间
    println!("{:?}", s.get(0..3));              // Some("hé")
}
```

遍历字符用 `chars()`，遍历字节用 `bytes()`，需要"第 n 个字符"用 `chars().nth(n)`（O(n)，不要放在循环里）。要按"用户看到的字"切分（emoji、组合字符）还得用 `unicode-segmentation` 这类 crate，标准库只到 `char` 这一层。

### to_uppercase 与 Unicode

`to_uppercase()`/`to_lowercase()` 是完整的 Unicode 大小写映射，会改变字符数和字节数；`to_ascii_uppercase()` 只处理 A-Z，其余原样保留：

```rust
fn main() {
    println!("{}", "straße".to_uppercase());        // STRASSE：ß 展开成两个字符
    println!("{}", "straße".to_ascii_uppercase());  // STRAßE：ß 没动
    println!("{}", "İ".to_lowercase().chars().count()); // 2：一个字符变两个
}
```

由此得到两条实践规则。第一，**大小写转换后不要再假设长度不变**，拼接、截断、按位置取字符都可能错位。第二，**比较字符串时先想清楚是不是只需要 ASCII 语义**：配置项、协议关键字用 `eq_ignore_ascii_case` 又快又安全；面向用户的搜索、排序则需要 Unicode 的大小写折叠（case folding），而且结果与语言环境有关，不要手写映射表。

### OsStr 与 PathBuf

`String` 是 UTF-8 的，但操作系统里的文件名不保证是合法 UTF-8。Rust 用 `OsStr`/`OsString` 表示"操作系统原生字符串"，用 `Path`/`PathBuf` 表示路径：

```rust
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn main() {
    let name = OsStr::new("报告.txt");

    let mut path = PathBuf::from("data");
    path.push(name);
    println!("{}", path.display());

    println!("{:?}", path.extension());
    println!("{}", path.with_extension("csv").display());

    let joined = Path::new("data").join("2026.csv");
    println!("{}", joined.to_string_lossy());
}
```

三个要点：

- **不要用字符串拼路径**。`format!("{}/{}", dir, file)` 在 Windows 上混出正反斜杠，在 `dir` 以分隔符结尾时多出一层。用 `PathBuf::push` 或 `Path::join`，它们知道平台的规则。
- **`Path::to_str()` 返回 `Option<&str>`**，因为路径可能不是 UTF-8。只是要打印给人看用 `display()`；要参与字符串处理用 `to_string_lossy()`，但要接受非 UTF-8 字节被替换成 `U+FFFD`。
- **`PathBuf` 与 `String` 的转换不是免费的**。从 `String` 到 `PathBuf` 通常不分配（`PathBuf::from` 直接接管），但 `to_str` 会做校验，`to_string_lossy` 在非 UTF-8 时会分配新字符串。

## 38.7 并发陷阱

并发问题的共同点是编译器只能拦住其中一部分，剩下的要靠规则和设计来避免。

### 死锁：加锁顺序

两个线程各持一把锁、再等对方的锁，就会永久卡住。经典场景是两个账户之间转账：

```rust,noplayground
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let a = Arc::new(Mutex::new(1));
    let b = Arc::new(Mutex::new(2));

    let (a1, b1) = (Arc::clone(&a), Arc::clone(&b));
    let t = thread::spawn(move || {
        let _x = a1.lock().unwrap();
        let _y = b1.lock().unwrap();   // 等 main 放掉 b
    });

    let _y = b.lock().unwrap();
    let _x = a.lock().unwrap();        // 同时等子线程放掉 a：死锁
    t.join().unwrap();
}
```

这一版可能挂起，具体表现取决于两个线程取得锁的顺序。修法是给所有锁规定**统一的获取顺序**，任何线程都按同一个顺序加锁：

```rust
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let a = Arc::new(Mutex::new(1));
    let b = Arc::new(Mutex::new(2));

    let (a1, b1) = (Arc::clone(&a), Arc::clone(&b));
    let t = thread::spawn(move || {
        let _x = a1.lock().unwrap();
        let _y = b1.lock().unwrap();
    });

    {
        let _x = a.lock().unwrap(); // 两边都先 a 后 b
        let _y = b.lock().unwrap();
    } // 等待子线程之前先释放它需要的两把锁
    t.join().unwrap();
    println!("done");
}
```

注意变量名前面带下划线的 `_x` 仍然持有守卫；只有单独的 `_` 会立刻把锁释放掉。如果无法规定顺序，可以把多份数据放进同一个 `Mutex`，或者用 `try_lock` 配超时，失败就释放已持有的锁重试。

### 锁中毒

如果一个线程在持锁期间 panic，`Mutex` 会被标记为"中毒"（poisoned），之后所有 `lock()` 都返回 `Err(PoisonError)`：

```rust
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    let shared = Arc::new(Mutex::new(0));
    let worker = Arc::clone(&shared);

    let _ = thread::spawn(move || {
        let mut guard = worker.lock().unwrap();
        *guard += 1;
        panic!("worker failed");
    })
    .join();

    let guard = shared.lock().unwrap_or_else(|e| e.into_inner());
    println!("{}", *guard);
}
```

中毒设计成"报错而不是静默继续"，因为 panic 可能发生在一次没写完的修改中间，数据处于不一致状态。三种处理方式：`unwrap()` 直接把问题传播出去；`.unwrap_or_else(|e| e.into_inner())` 明确表示"我检查过，数据仍然可用"；或者把临界区写成"要么全做完要么不做"，让中毒几乎不可能发生。

### 持锁跨线程或跨 await

`MutexGuard` 不是 `Send`。它的生命周期和锁绑定，必须在同一个线程上释放：

```rust,noplayground
use std::sync::Mutex;
use std::thread;

fn main() {
    let shared = Mutex::new(0);
    let guard = shared.lock().unwrap();
    thread::spawn(move || {
        println!("{}", *guard);   // E0277：MutexGuard 不能被移动到别的线程
    });
}
```

异步里的同一个问题更隐蔽：在 `.await` 期间持有 `std::sync::Mutex` 的守卫，会让整个 `Future` 变成 `!Send`，于是 `tokio::spawn` 报 E0277。修法是用一个花括号把临界区括起来、在 `await` 之前 drop 守卫，或者换用 `tokio::sync::Mutex`（它专为跨 `await` 持有设计，代价是每次加锁更慢、且异步加锁会产生 `Future`）。一个简单的自查规则：**看到 `lock()` 和 `.await` 出现在同一个函数里，就把它们之间隔开**。

### Rc 不能跨线程

`Rc<T>` 的引用计数不是原子的，两个线程同时克隆会让计数错乱，所以 `Rc<T>` 不是 `Send`：

```rust,noplayground
use std::rc::Rc;
use std::thread;

fn main() {
    let shared = Rc::new(1);
    let copy = Rc::clone(&shared);
    thread::spawn(move || {
        println!("{}", copy);   // E0277：Rc 不能在线程间传递
    });
}
```

单线程共享用 `Rc`，跨线程共享用 `Arc`（atomic reference counted）：把上面的 `Rc` 与 `Rc::clone` 依次换成 `Arc` 与 `Arc::clone`，代码立刻通过编译。名字只差一个字母，但 `Arc` 的每次克隆和销毁都带原子操作，比 `Rc` 贵一点，单线程场景不要无脑用 `Arc`。

### static mut

`static mut` 是全局可变量，编译器不给任何同步保证。多个线程同时读写它就是数据竞争，属于未定义行为：

```rust,noplayground
static mut COUNTER: u32 = 0;

fn main() {
    unsafe {
        COUNTER += 1;   // 单线程看似没问题，放到多线程里就是数据竞争
    }
}
```

在 edition 2021 下，对 `static mut` 取引用只会得到一个警告；edition 2024 里已经是硬错误。可变的全局状态应该用 `AtomicU32`、`Mutex` 或 `OnceLock` 表达，让编译器能检查：

```rust
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn main() {
    COUNTER.fetch_add(1, Ordering::Relaxed);
    println!("{}", COUNTER.load(Ordering::Relaxed));
}
```

## 38.8 测试与构建陷阱

### 测试默认并行跑

`cargo test` 由 libtest 驱动，默认在多个线程上并行执行同一个二进制里的测试。两个测试只要共享了 `static` 变量、临时文件路径、环境变量或者同一个数据库连接，就可能互相污染，表现为"单跑通过、一起跑失败"：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ok() {
        assert_eq!(parse("42"), Some(42));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse("nope"), None);
    }
}

fn parse(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

fn main() {
    println!("{:?}", parse("42"));
}
```

这类测试本身没问题，问题出在"共享的可变状态"。处理办法按优先级：把测试写成纯函数（每条数据都当参数传入）；用 `tempfile` 生成互不相同的临时路径；实在绕不开时用 `cargo test -- --test-threads=1` 串行执行，或者用 `serial_test` 的 `#[serial]` 标记必须独占的测试。**不要**用"多睡一会儿"来掩盖竞态，那只是把失败概率降下去。

### 条件编译

`cfg!` 是编译期的布尔值，例如 `cfg!(target_os = "windows")` 可以直接写进 `if` 条件；`#[cfg(条件)]` 则整个删除被标注的条目。两者都在编译期定死，不能在运行期改变。

常见坑有两个。第一，`#[cfg(test)]` 标注的模块只在 `cargo test` 时存在，`cargo run` 时它被整个删掉，所以里面的辅助函数不能给 `main` 用。第二，被 `cfg` 排除的代码**仍然要能通过解析**，只是不做类型检查；如果平台相关代码写错了名字，可能直到在对应平台上编译才发现。跨平台代码最好在 CI 里对多个 target 各编译一次。

### 特性（feature）是累加的

Cargo 的 feature 是**可加的**：只要依赖图里有任何一方打开了某个 feature，它就会对所有人打开。两个 crate 依赖同一个库、其中一个开了 `json`，最终编译出的那份就带 `json`，另一个 crate 无从关闭。在 `Cargo.toml` 里用 `[features]` 声明 `json = ["dep:serde_json"]`，再把依赖标成 `optional = true`，是最常见的写法。

由此得到设计规则：**feature 只能增加能力，不能改变已有 API 的含义**；不要在 feature 里做"省略某个必需函数"这类破坏性开关；测试用的 feature 要放进 `[dev-dependencies]` 或 `required-features`，否则会泄漏给整个依赖图。`default-features = false` 只对**你自己这条依赖边**生效，不能阻止别人打开默认特性。

### 孤儿规则

你只能为"自己 crate 里的类型"实现 trait，或者为"外部类型"实现"自己 crate 里的 trait"。给 `Vec<u8>` 实现标准库的 `Display` 就违反了这条规则：

```rust,noplayground
use std::fmt;

impl fmt::Display for Vec<u8> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes", self.len())
    }
}

fn main() {}
```

报错是 E0117。规则存在的原因是一致性（coherence）：如果两个 crate 都能给同一个外部类型实现同一个外部 trait，链接时就会出现两份实现。标准做法是包一层 newtype：

```rust
use std::fmt;

struct Bytes(Vec<u8>);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes", self.0.len())
    }
}

fn main() {
    println!("{}", Bytes(vec![1, 2, 3]));
}
```

newtype 的另一个好处是零成本：`Bytes` 和它包的 `Vec<u8>` 占用完全相同的内存，运行期没有任何额外间接层（第 20 章会验证这一点）。

## 38.9 按症状查

报错原文是英文的，但你不需要记住每个错误码，只要能把症状归类。下面这张表从现象出发：

| 症状 | 最可能的原因 | 看哪一节 |
|---|---|---|
| value moved / borrow of moved value | 所有权已被移走，之后又使用了原变量 | 38.3 |
| cannot borrow as mutable because it is also borrowed as immutable | 共享借用与可变借用区间重叠 | 38.3、38.4 |
| cannot borrow as mutable more than once | 两个可变借用区间重叠 | 38.3 |
| does not live long enough / cannot return value referencing local | 被借用的数据比引用先销毁 | 38.3、第 17 章 |
| temporary value dropped while borrowed | 临时值在语句结束时被销毁 | 38.3 |
| type annotations needed | collect 或 parse 的目标类型不唯一 | 38.4 |
| no method named 方法名 found | 方法不存在，通常是 trait 没有导入 | 38.8 |
| only traits defined in the current crate | 违反孤儿规则 | 38.8 |
| trait bound not satisfied / cannot be sent between threads | 类型缺少 Send、Sync 等能力 | 38.7 |
| debug 下 panic attempt to add with overflow，release 下正常 | 整数溢出检查只在 debug 打开 | 38.5 |
| 数值结果莫名不对（负数、大数、小数） | 取模符号、as 截断、浮点比较 | 38.5 |
| 字符串切片运行期 panic | 字节下标落在字符中间 | 38.6 |
| 程序卡住不退出 | 死锁，通常由加锁顺序不一致引起 | 38.7 |
| 锁返回 poisoned | 持锁线程 panic | 38.7 |
| 测试单跑通过、一起跑失败 | 测试并行执行且共享可变状态 | 38.8 |

查表之后回到对应小节，按那里的规则改。如果还是拿不准，先别一次改五处——改一处、编一次，让编译器告诉你上一步是否正确。一次改太多，你会分不清剩下的错误是原来的还是新引入的。

## 典型错误案例

下面五个案例的报错都来自 rustc 1.100.0 在 Windows 上的真实输出，文件名换成 `src/main.rs`，行号与上面的代码块一致。

### 案例 1：一边遍历一边 push——E0502

```rust,editable
fn main() {
    let mut scores = vec![10, 20, 30];
    for s in &scores {
        if *s < 25 {
            scores.push(*s + 100);
        }
    }
    println!("{:?}", scores);
}
```

```text
error[E0502]: cannot borrow `scores` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:13
  |
3 |       for s in &scores {
  |       -        -------
  |       |        |
  |       |        immutable borrow occurs here
  |  _____|        immutable borrow later used here
  | |
4 | |         if *s < 25 {
5 | |             scores.push(*s + 100);
  | |             ^^^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here
6 | |         }
7 | |     }
  | |_____- this for loop borrows `scores` immutably, preventing mutation within its body
  |
  = help: consider using an index-based loop instead, or collecting modifications into a separate collection

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`for s in &scores` 创建了一个跨越整个循环体的共享借用，因为迭代器每轮都要从 `scores` 里取下一个元素；循环体里的 `scores.push` 又要求可变借用。两者区间重叠，违反"共享借用与可变借用不能并存"。注意编译器标出的是整段 `for` 区间，而不是单独一行——这就是 NLL 说的"借用一直活到迭代器最后一次使用"。

**修复**：把"读"和"写"拆成两步，先收集要追加的元素，循环结束后再合并：

```rust,editable
fn main() {
    let mut scores = vec![10, 20, 30];
    let mut extra = Vec::new();
    for s in &scores {
        if *s < 25 {
            extra.push(*s + 100);
        }
    }
    scores.extend(extra);
    println!("{:?}", scores);
}
```

</details>

### 案例 2：同时持有两个可变借用——E0499

```rust,editable
fn main() {
    let mut v = vec![1, 2, 3];
    let max = &mut v[0];
    let next = &mut v[1];
    *max = *next;
    println!("{:?}", v);
}
```

```text
error[E0499]: cannot borrow `v` as mutable more than once at a time
 --> src/main.rs:4:21
  |
3 |     let max = &mut v[0];
  |                    - first mutable borrow occurs here
4 |     let next = &mut v[1];
  |                     ^ second mutable borrow occurs here
5 |     *max = *next;
  |     ------------ first borrow later used here
  |
  = help: use `.split_at_mut(position)` to obtain two mutable non-overlapping sub-slices

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`&mut v[0]` 和 `&mut v[1]` 在语法上是两个不相交的元素，但编译器只按"整块 `v`"来统计借用，看不到下标的区别。`max` 一直活到第 5 行被使用，第二个可变借用就与它重叠了。这类错误在"同时改两个字段"的代码里很常见。

**修复**：用 `split_at_mut` 一次性把切片切成两段互不重叠的可变引用，或者改用下标分别赋值：

```rust,editable
fn main() {
    let mut v = vec![1, 2, 3];
    let (left, right) = v.split_at_mut(1);
    left[0] = right[0];
    println!("{:?}", v);
}
```

</details>

### 案例 3：借用活不过作用域——E0597

```rust,editable
fn main() {
    let text;
    {
        let s = String::from("hello");
        text = s.as_str();
    }
    println!("{}", text);
}
```

```text
error[E0597]: `s` does not live long enough
 --> src/main.rs:5:16
  |
4 |         let s = String::from("hello");
  |             - binding `s` declared here
5 |         text = s.as_str();
  |                ^ borrowed value does not live long enough
6 |     }
  |     - `s` dropped here while still borrowed
7 |     println!("{}", text);
  |                    ---- borrow later used here

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`text` 声明在内层块之外，但它借用的 `s` 在内层块结束时就被 drop 了。编译器把三个位置连起来读：第 5 行借出，第 6 行所有者销毁，第 7 行还在用。这就是"借用的存活区间必须被所有者的存活区间包住"。

**修复**：让所有者和借用待在同一个作用域里，或者干脆让 `text` 拥有数据：

```rust,editable
fn main() {
    let s = String::from("hello");
    let text = s.as_str();
    println!("{}", text);
}
```

</details>

### 案例 4：返回指向局部变量的引用——E0515

```rust,editable
fn first_byte(s: &str) -> &str {
    let owned = String::from(s);
    &owned[..1]
}

fn main() {
    println!("{}", first_byte("hello"));
}
```

```text
error[E0515]: cannot return value referencing local variable `owned`
 --> src/main.rs:3:5
  |
3 |     &owned[..1]
  |     ^-----^^^^^
  |     ||
  |     |`owned` is borrowed here
  |     returns a value referencing data owned by the current function

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`owned` 是函数内的局部变量，函数返回时它的堆缓冲会被释放。返回的 `&str` 指向那块已经归还的内存，编译器必须拦住。注意这与 E0597 的区别：E0597 是"调用方拿着引用太久"，E0515 是"函数自己想返回一个指向局部数据的引用"，问题出在签名上。

**修复**：要么直接从入参借用（入参的存活期由调用方保证），要么返回拥有所有权的 `String`：

```rust,editable
fn first_byte(s: &str) -> &str {
    &s[..1]
}

fn first_byte_owned(s: &str) -> String {
    s.chars().next().map(|c| c.to_string()).unwrap_or_default()
}

fn main() {
    println!("{}", first_byte("hello"));
    println!("{}", first_byte_owned("hello"));
}
```

</details>

### 案例 5：collect 不知道变成什么——E0283

```rust,editable
fn main() {
    let words = vec!["a", "b", "c"];
    let lens = words.iter().map(|w| w.len()).collect();
    println!("{:?}", lens);
}
```

```text
error[E0283]: type annotations needed
    --> src/main.rs:3:9
     |
   3 |     let lens = words.iter().map(|w| w.len()).collect();
     |         ^^^^                                 ------- type must be known at this point
     |
     = note: the type must implement `FromIterator<usize>`
help: consider giving `lens` an explicit type
     |
   3 |     let lens: Vec<_> = words.iter().map(|w| w.len()).collect();
     |             ++++++++

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`collect` 的返回类型由泛型参数 `B` 决定，而 `B` 只需要实现 `FromIterator<usize>`。`Vec<usize>`、`HashSet<usize>`、`VecDeque<usize>` 都满足，后面的 `println!("{:?}")` 又只要求 `Debug`，没有任何信息能把这个集合定下来。编译器于是报"需要类型标注"，而不是替你选一个。

**修复**：在绑定上写类型，或者用 turbofish 指定 `collect::<目标类型>()`：

```rust,editable
fn main() {
    let words = vec!["a", "b", "c"];
    let lens: Vec<usize> = words.iter().map(|w| w.len()).collect();
    println!("{:?}", lens);

    let total = words.iter().map(|w| w.len()).sum::<usize>();
    println!("{}", total);
}
```

</details>

## 动手练习

**练习 1**：改正下面代码里的借用错误，并说明它触发了哪条规则：

```rust,noplayground
fn main() {
    let mut names = vec![String::from("ana")];
    let first = &names[0];
    names.push(String::from("bo"));
    println!("{}", first);
}
```

**练习 2**：找一段你写过的编译不过的代码，用 `cargo build 2>&1 | head -30` 只看前 30 行，只修第一个 error，记录第二个 error 是否随之消失。重复三次，体会错误的连锁关系。

**练习 3**：实现 `fn safe_add(a: u8, b: u8) -> Option<u8>`，内部用 `checked_add`；再写一个 `main` 计算 255 加 1，分别在 debug 和 release 下运行，记录两次输出并解释差异。

**练习 4**：实现 `fn truncate_chars(s: &str, n: usize) -> &str`，按字符数截断且绝不 panic。测四种输入：空串、n 为 0、n 大于字符数、包含多字节字符的串。

**练习 5**：写两个线程按相反顺序锁两个 `Mutex`，观察死锁；再统一加锁顺序，确认程序能退出。最后用 `try_lock` 加超时做一个不会永久卡住的版本。

## 小结

- 读报错只修第一个 error，先看两处 span 的时间线，再看 note/help 给的改法
- E0382 是所有权被移走，E0502 与 E0499 是借用区间重叠，E0597 与 E0515 与 E0716 是引用活得比数据久
- 临时值在语句结束时销毁，任何"返回引用的临时值"都不能把引用带过这条语句
- 闭包按需捕获 `&`、`&mut` 或值；冲突时缩短闭包的寿命，或者把它改成显式传参的函数
- `iter`、`iter_mut`、`into_iter` 分别产出 `&T`、`&mut T`、`T`，最后一个会消费集合
- `collect` 的目标类型必须能唯一确定，否则用类型标注或 turbofish
- 整数溢出在 debug 下 panic、release 下回绕，跨边界转换用 `checked_*` 或 `try_into`
- `String::len` 返回字节数，切片必须落在字符边界上，安全版本用 `get`
- 并发三件事：统一加锁顺序、在 await 之前放锁、跨线程共享用 `Arc` 而不是 `Rc`
- 测试默认并行执行，feature 是累加的，孤儿规则要求给外部类型套一层 newtype