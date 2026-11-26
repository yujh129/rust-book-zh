# 第 34 章 过程宏

> **本章目标**
>
> - 分清 derive、attribute、function-like 三种过程宏的输入输出与调用形态
> - 会用 proc-macro = true 的独立 crate 组织代码，说清 syn、quote、proc-macro2 各自的职责
> - 手写一个 derive 宏：解析 DeriveInput、遍历字段、用 quote! 生成 impl
> - 写一个 attribute 宏给函数套上计时外壳，知道它会怎样改变控制流
> - 用 syn::Error 与 span 把错误指回用户写错的那一行
> - 理解过程宏看不到类型信息、要付出编译期成本，以及不卫生会带来什么后果

第 33 章的 macro_rules! 只能按 token 的形状做匹配。它能拼出 vec!、json! 这样的东西，但有三件事做不到：读不懂一段完整语法的结构，没法按字段名生成方法，更不能读取写在自己身上的参数。一个像 serde 那样要遍历每个字段、处理每一种类型的宏，用声明宏写出来会变成几百行递归规则。

过程宏（procedural macro）把宏的作者从这个牢笼里放了出来：它就是一个普通的 Rust 函数，输入一串 token，输出一串 token，中间随便你写什么代码。代价也很实在——它必须单独住在一个 crate 里，依赖 syn 这类重量级库，编译期会变慢，而且因为展开发生在类型检查之前，它只能看见语法、看不见类型。

这一章从工程结构讲到三种过程宏的写法，再到错误报告与调试，最后拆开 TokenStream 看它到底长什么样。

## 34.1 三种过程宏与它们的输入输出

过程宏分三种，区别在「谁调用它、拿到什么、输出贴回哪里」。

| 种类 | 声明属性 | 输入 | 输出位置 | 调用形式 |
| --- | --- | --- | --- | --- |
| derive | proc_macro_derive | 被 derive 的整个项 | 追加在原项之后 | #[derive(Name)] |
| attribute | proc_macro_attribute | 属性参数 + 被标注的项 | 替换被标注的项 | #[the_attr] |
| function-like | proc_macro | 括号里的 token | 替换这次调用 | the_macro!(参数) |

三种宏的入口函数签名由编译器强制规定，都收 proc_macro::TokenStream，也都返回它：

```rust,noplayground
use proc_macro::TokenStream;

#[proc_macro_derive(HelloName)]
pub fn derive_hello_name(input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_attribute]
pub fn timed(attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro]
pub fn hashmap(input: TokenStream) -> TokenStream {
    input
}
```

attribute 宏多一个参数：attr 是属性名后面那对括号里的 token。写 #[timed] 时它是空的；写 #[timed(label = "db")] 时它是 label = "db"。item 则是被标注的那个项的全部 token。

三个函数的签名一旦写错，编译器直接拒绝，而且报的不是普通的类型错误。把返回类型换成 proc_macro2::TokenStream 会得到：

```text
error: derive proc macro has incorrect signature
 --> src/lib.rs:4:20
  |
4 | pub fn nope(input: TokenStream) -> TokenStream {
  |                    ^^^^^^^^^^^ expected `proc_macro::TokenStream`, found `proc_macro2::TokenStream`
  |
  = note: expected signature `fn(proc_macro::TokenStream) -> proc_macro::TokenStream`
             found signature `fn(proc_macro2::TokenStream) -> proc_macro2::TokenStream`
```

derive 与另外两种有一个关键差别：它不能修改原来的项，只能往后面追加代码。你没法用 derive 删掉一个字段或改写一个方法，只能加 impl、加类型、加函数。attribute 宏则是整项替换，原样不动地吐回去等于什么也没干，这是它和 derive 最大的区别。

## 34.2 工程结构：为什么必须单独一个 crate

过程宏不能和普通库住在同一个 crate 里。Cargo 里要显式声明 crate 类型：

```toml
[package]
name = "hello-macros"
version = "0.1.0"
edition = "2021"

[lib]
proc-macro = true

[dependencies]
syn = { version = "3", features = ["full"] }
quote = "1"
proc-macro2 = "1"
```

典型布局是一个 workspace，一个宏 crate，一个使用它的普通 crate：

```text
ch34-workspace/
├── Cargo.toml          # [workspace] members = ["app", "hello-macros"]
├── hello-macros/       # proc-macro = true
│   ├── Cargo.toml
│   └── src/lib.rs
└── app/
    ├── Cargo.toml      # hello-macros = { path = "../hello-macros" }
    └── src/main.rs
```

**为什么非要拆开。** 过程宏 crate 会被编译成动态库，在**编译你的程序时**由 rustc 加载执行。它不是一个运行期依赖，最终二进制里没有它的代码。这带来三条硬约束：

1. proc-macro crate 只能导出被 #[proc_macro]、#[proc_macro_derive]、#[proc_macro_attribute] 标注的函数。你写一个 pub struct 或 pub fn，编译器会报错。
2. 一个 crate 只能是一种 crate type。普通库要参与链接、能被别人依赖，过程宏要走动态库加载，两者不能兼得。
3. 你不能在自己的 crate 里用自己定义的过程宏。宏要先编译成动态库，才能用来编译代码，自举没有意义。

由此诞生了配套 crate 的模式：serde 提供运行期的 trait 与类型，serde_derive 只放宏，用户同时依赖两者。本书第 29 章已经用过这对组合，这里只是把幕后的分工点破。

### syn、quote、proc-macro2 各管什么

| crate | 职责 | 在本章里的角色 |
| --- | --- | --- |
| proc-macro2 | proc_macro 的可移植封装，可在测试和工具里使用 | 统一代码内部流通的 TokenStream |
| quote | 准引用，把 Rust 源码写成模板并插值 | 用 quote! 拼出要生成的代码 |
| syn | 把 TokenStream 解析成语法树 | 把输入变成 DeriveInput、ItemFn 等结构 |

三者分工清晰：syn 负责**读懂输入**，quote 负责**写出输出**，proc-macro2 是它们之间、以及与你代码之间流通的 token 类型。编译器只认 proc_macro::TokenStream，所以在入口处解析、在返回处用 .into() 转回去，中间全用 proc_macro2::TokenStream。

syn 的默认 feature 已经包含 derive、parsing、printing、clone-impls、proc-macro。要用 attribute 宏解析函数体，需要打开 full。本章的 Cargo.toml 里写的是 features = ["full"]，因为 #[timed] 要解析 ItemFn。

版本提醒：syn 3 的结构和网上大量 syn 2 教程基本一致，但并非完全一样。例如 syn 3 的 ItemFn 多了一个 modifiers 字段，照着 2.x 的写法把四个字段全部解构出来会编译失败，改成用引用逐个访问字段即可：

```rust,noplayground
let attrs = &item.attrs;
let vis = &item.vis;
let sig = &item.sig;
let block = &item.block;
```

## 34.3 手写一个 derive：HelloName

现在把三件工具拼起来，写一个最小的 derive。它的目标很朴素：给结构体加一个方法 hello_name，返回 "Hello, 名字!"。这个例子小到可以一眼看完，但包含 derive 宏的全部骨架——解析输入、检查合法性、用 quote 生成 impl、把结果交回编译器。

### 宏 crate 里的完整实现

在 hello-macros/src/lib.rs 里写入：

```rust,noplayground
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

#[proc_macro_derive(HelloName)]
pub fn derive_hello_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let has_name = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .any(|f| f.ident.as_ref().is_some_and(|i| i == "name")),
            _ => false,
        },
        _ => false,
    };

    if !has_name {
        let err = syn::Error::new_spanned(name, "HelloName 需要一个名为 name 的字段");
        return err.to_compile_error().into();
    }

    quote! {
        impl #name {
            pub fn hello_name(&self) -> String {
                format!("Hello, {}!", self.name)
            }
        }
    }
    .into()
}
```

逐段拆开看。

parse_macro_input!(input as DeriveInput) 做了两件事：调用 syn 的解析器把 TokenStream 变成 DeriveInput，失败时立刻 return 一段 compile_error! 代码。它不是普通的函数，而是一个宏，因为要在出错时从你的函数里直接 return。

DeriveInput 是 derive 宏能看到的一切，字段有五个：attrs、vis、ident、generics、data。data 区分三种被 derive 的目标：

- Data::Struct，里面是 DataStruct，字段在 data.fields
- Data::Enum，里面是 DataEnum，变体在 data.variants
- Data::Union，字段在 data.fields

Fields 又分三种：Named(FieldsNamed) 对应带字段名的结构体，Unnamed(FieldsUnnamed) 对应元组结构体，Unit 对应空结构体。FieldsNamed 里真正装字段的容器是 named: `Punctuated<Field, Token![,]>`，可以当作迭代器用。上面的代码只在 Data::Struct 且 Fields::Named 的情况下遍历字段，找 ident 等于 name 的那一个；其他情况一律判为不合法。

quote! 负责生成代码。模板里用 # 做插值：写 #name 就会把 name 这个变量的 ToTokens 结果填进去。这里 name 是 &Ident，插进去就是类型名。最后生成的 TokenStream 是 proc_macro2::TokenStream，而函数签名要求 proc_macro::TokenStream，所以末尾加上 .into() 转换。

### 宏展开后等价于什么

deriving 的效果，用手写代码表达就是下面这样。这也是理解 derive 的最好方式：它没有魔法，只是替你把一段 impl 抄进去。

```rust
struct User {
    name: String,
}

impl User {
    pub fn hello_name(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}

fn main() {
    let u = User {
        name: String::from("ana"),
    };
    println!("{}", u.hello_name());
}
```

输出：

```text
Hello, ana!
```

### 使用者这边怎么写

app 依赖 hello-macros，代码里只多一行 derive：

```rust,noplayground
use hello_macros::HelloName;

#[derive(HelloName)]
struct User {
    name: String,
}

fn main() {
    let u = User {
        name: String::from("ana"),
    };
    println!("{}", u.hello_name());
}
```

注意两个名字：#[proc_macro_derive(HelloName)] 里是暴露给使用者的宏名，函数名 derive_hello_name 只影响宏 crate 内部。宏名写成 HelloName 就要求使用者写 #[derive(HelloName)]，大小写敏感。

### 两个必须补上的细节

第一，derive 只能在原 item 后面追加，不能改写或删除原 item。编译器会保留使用者写的结构体，再把你返回的 impl、函数、常量拼上去。所以 derive 宏拿到的 DeriveInput 虽然类型是"可变"的 AST，但改动它没有意义——你返回的内容才是全部输出（除原 item 之外）。

第二，泛型必须自己转交。上面的模板写死了 impl #name，遇到 `struct Wrapper<T>` 会生成不合法代码。正确做法是用 syn 提供的 split_for_impl：

```rust,noplayground
let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

quote! {
    impl #impl_generics #name #ty_generics #where_clause {
        pub fn hello_name(&self) -> String {
            format!("Hello, {}!", self.name)
        }
    }
}
```

拆出来的三段分别放在 impl 后、类型名后、大括号前。它的细节可以放到练习里验证。

还有一个常被忽略的能力：derive 宏可以声明辅助属性。写成 #[proc_macro_derive(HelloName, attributes(hello))] 之后，使用者就能在字段上写 #[hello(参数)]，你在解析时从 Field::attrs 里读。serde 的 #[serde(rename = "新名字")] 就是这条路。本章不展开，但你应该知道 derive 宏的输入只包含语法，所有额外信息都得靠这种辅助属性传进来。

最后说明设计取舍：本章的 HelloName 生成的是固有方法 impl User。真实的 derive 通常生成 trait impl，而 trait 必须定义在使用者也可见的 crate 里，于是就有了 34.2 说的配套 crate 模式。为了不让第一个例子牵扯两个 crate，这里先用固有方法。

## 34.4 attribute 宏：用 #[timed] 包一层计时

attribute 宏的入口比 derive 多一个参数：

```rust,noplayground
#[proc_macro_attribute]
pub fn timed(attr: TokenStream, item: TokenStream) -> TokenStream
```

item 是被标注的那段代码的 token，attr 是宏名字后面的那截 token。写 #[timed] 时 attr 是空的；写 #[timed(label = "db")] 时 attr 里是 label = "db"。返回的 TokenStream 会整体替换掉 item，所以你既可以把原代码原样吐回去，也可以像下面这样在外面包一层。

```rust,noplayground
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn timed(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let attrs = &item.attrs;
    let vis = &item.vis;
    let sig = &item.sig;
    let block = &item.block;
    let label = sig.ident.to_string();

    quote! {
        #(#attrs)*
        #vis #sig {
            let __start = ::std::time::Instant::now();
            let __result = #block;
            ::std::eprintln!("[timed] {} 耗时 {:?}", #label, __start.elapsed());
            __result
        }
    }
    .into()
}
```

关键点是把 ItemFn 拆成四块再拼回去。签名里已经带了 async、unsafe、泛型和返回类型，所以整段 #sig 照抄即可，不需要逐个处理。attrs 用 #(#attrs)* 这种重复插值展开——quote 对迭代器的重复写法与 macro_rules! 里的星号重复神似。block 是一个块表达式，赋值给 __result 之后既能拿到返回值，又能在后面打印。

使用者这边：

```rust,noplayground
use hello_macros::timed;

#[timed]
fn work() -> u64 {
    (1..=1_000_000u64).sum()
}

fn main() {
    let total = work();
    println!("total = {}", total);
}
```

本机运行的真实输出（耗时会浮动）：

```text
[timed] work 耗时 8.9778ms
total = 500000500000
```

### 这个实现会在哪里失效

计时包装看起来天衣无缝，实际有几个明确的边界，写库时必须知道：

- 函数体里提前 return 或用 ? 提前退出，打印语句在 return 之后，不会执行。想统计所有出口，得把原函数体挪进一个闭包，再在闭包外统一收尾。
- async fn 的体会被编译成状态机，__start 在第一次 poll 时才创建，所以计时从第一次被 poll 开始，且包含所有 await 等待的时间，而不是 CPU 时间。
- const fn 无法使用，Instant::now 不是 const 函数。
- 属性宏按从外到内的顺序展开：写在 #[timed] 前面的属性宏先运行，它看到的 item 里还带着 #[timed]；而内置属性（如 #[inline]）会被原样保留在 attrs 里。这也意味着 #[timed] 与 #[inline] 的先后顺序会影响最终代码形状。
- 如果标注到 struct 或 mod 上，parse_macro_input!(item as ItemFn) 会失败，报错见 34.6 与错误案例 4。

真实世界的 attribute 宏大多会解析 attr。比如 #[timed(label = "db")] 想自定义标签，就要为 attr 写一个 Parse 实现，或者用 syn::meta 系列的解析器。本章的 hashmap! 会展示同款 Parse 手法。

## 34.5 function-like 宏：hashmap!

第三种过程宏用起来像函数调用：hashmap! { "a": 1, "b": 2 }。它的入口只有一个 TokenStream，就是括号里的全部内容。

```rust,noplayground
use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Expr, Token};

struct Pair {
    key: Expr,
    value: Expr,
}

impl Parse for Pair {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Expr = input.parse()?;
        input.parse::<Token![:]>()?;
        let value: Expr = input.parse()?;
        Ok(Pair { key, value })
    }
}

#[proc_macro]
pub fn hashmap(input: TokenStream) -> TokenStream {
    let pairs = parse_macro_input!(
        input with syn::punctuated::Punctuated::<Pair, Token![,]>::parse_terminated
    );
    let keys = pairs.iter().map(|p| &p.key);
    let values = pairs.iter().map(|p| &p.value);

    quote! {{
        let mut map = ::std::collections::HashMap::new();
        #( map.insert(#keys, #values); )*
        map
    }}
    .into()
}
```

ParseStream 是"剩下的 token"的游标。`input.parse::<Expr>()` 吃掉一个表达式，`input.parse::<Token![:]>()` 吃掉一个冒号，失败会返回带 span 的 syn::Error。`Punctuated::<Pair, Token![,]>::parse_terminated` 是一个现成的 Parser，它会反复调用 Pair::parse，直到输入耗尽，并自动处理末尾多余的逗号。注意 parse_macro_input! 的第二种形式：with 后面接一个 Parser，而不是类型。

quote! 里的双重大括号不是笔误。外层是 quote 的分隔符，内层才是要生成的块表达式，最终产出一段以 let mut map 开头、以 map 结尾的块表达式。重复片段 #( map.insert(#keys, #values); )* 会按迭代器长度展开，keys 和 values 必须长度相同——它们来自同一个 pairs，天然对齐。

使用与输出：

```rust,noplayground
use hello_macros::hashmap;

fn main() {
    let counts = hashmap! {
        "a": 1,
        "b": 2,
    };
    println!("{:?}", counts);
}
```

```text
{"b": 2, "a": 1}
```

花括号的顺序每次运行可能不同，这是 HashMap 的迭代顺序，不是宏的问题。

### 和 macro_rules! 版本比，多了什么

第 33 章的 hashmap! 用 macro_rules! 也能写，靠 $( $k:expr => $v:expr ),* 匹配。过程宏版本换来的是：输入可以按任意语法解析（这里的 Pair 就是自定义语法），生成逻辑可以用普通 Rust 写（循环、条件、查表），报错能精确到某个 token 的 span。代价是多一个 crate、一套 syn/quote 依赖，以及 34.8 要谈的编译时间。

## 34.6 错误报告：syn::Error 与 span

过程宏报错的正道只有一条：返回一段包含 compile_error! 的 token，并让它落在使用者代码的正确位置上。syn::Error 就是为此设计的。

两种构造方式：

- syn::Error::new(span, "消息")——消息挂在某个 Span 上
- syn::Error::new_spanned(tokens, "消息")——tokens 是任何实现了 ToTokens 的东西（Ident、Field、Type、整个 DeriveInput），underline 覆盖它的全部 token

调用 err.to_compile_error() 得到一个 proc_macro2::TokenStream，内容形如 compile_error! { "消息" }，span 就是构造时给的那个。把它交给编译器，就变成使用者看得见的红字。

34.3 里的检查用的就是 new_spanned(name, "消息")。这里 name 是结构体标识符，所以报错落在结构体名上。本机实测：

```text
error: HelloName 需要一个名为 name 的字段
 --> app/src/bin/bad.rs:4:8
  |
4 | struct Config {
  |        ^^^^^^
```

如果换成 Error::new(Span::call_site(), "消息")，消息会指向宏调用附近而不是结构体名，定位体验差很多。凡是手上还有使用者的 token，就优先 new_spanned 它们；实在没有，再退回到具体节点的 span。

选 span 的粒度也有讲究。字段类型不对，就把错误挂在字段的类型上；字段名不允许，就挂在字段标识符上；只有整个 item 都不合法时，才挂在 DeriveInput 上。

一个容易踩的坑是级联报错。compile_error! 只让编译器记住"这里有个错误"，并不会中止后面所有检查。上面那份代码在报完自定义错误后，main 里调用 c.hello_name() 还会再报一次：

```text
error[E0599]: no method named `hello_name` found for struct `Config` in the current scope
  --> app/src/bin/bad.rs:10:22
   |
 4 | struct Config {
   | ------------- method `hello_name` not found for this struct
```

所以自定义错误之后应该立刻返回，不要再输出半成品代码。parse_macro_input! 出错时就是这么做的：返回 compile_error!，而不是继续。

需要一次报多个问题时，用 Error::combine：先从一个错误开始，遇到下一个就 combine 进去，最后统一 to_compile_error()。顺序遍历字段、逐条检查的 derive 宏常用这个模式。

还有两点现实约束。第一，稳定版过程宏只能报错，不能报警告：proc_macro::Diagnostic 仍是 nightly 特性。想提示但不拦编译，只能把提醒写进生成代码（例如生成一个 #[deprecated] 项）。第二，syn::Error 只在解析相关的代码里自然流通（ParseStream 上的 ? 会把它传上来）；一旦越过 proc_macro 边界，它就变成 token 了。

## 34.7 调试过程宏

过程宏的调试难点在于：它没有运行时，出错发生在编译期，而且中间的 token 看不见。按可获得的工具排序，有三条路。

第一条是 cargo expand。它把宏展开后的代码打印出来，是最直观的工具。用法是 cargo install cargo-expand 之后在项目里跑 cargo expand 或 cargo expand --bin hello-app。它依赖 rustc 的 -Zunpretty=expanded，所以需要 nightly 工具链。本书的离线环境里没有安装它，也不建议读者在离线时尝试安装。等价的 nightly 命令是 cargo rustc -- -Zunpretty=expanded。

第二条是拆分逻辑加单元测试，离线可用，也是本章推荐的做法。要点是让真正的生成逻辑不碰 proc_macro::TokenStream，只吃 proc_macro2::TokenStream，宏入口退化成一层薄壳：

```rust,noplayground
fn expand_hello_name(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let has_name = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .any(|f| f.ident.as_ref().is_some_and(|i| i == "name")),
            _ => false,
        },
        _ => false,
    };
    if !has_name {
        return syn::Error::new_spanned(name, "HelloName 需要一个名为 name 的字段")
            .to_compile_error();
    }
    quote! {
        impl #name {
            pub fn hello_name(&self) -> String {
                format!("Hello, {}!", self.name)
            }
        }
    }
}

#[proc_macro_derive(HelloName)]
pub fn derive_hello_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_hello_name(input).into()
}
```

这样 expand_hello_name 就是一个普通函数，能在 proc-macro crate 自己的测试里跑。用 syn::parse_quote! 造一个 DeriveInput，再打印或断言结果：

```rust,noplayground
#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn expands_hello_name() {
        let input: DeriveInput = parse_quote! {
            struct User {
                name: String,
            }
        };
        let code = expand_hello_name(input).to_string();
        assert!(code.contains("impl User"));
        println!("{code}");
    }
}
```

本机执行 cargo test -p ch34-macros -- --nocapture，真实输出：

```text
running 1 test
impl User { pub fn hello_name (& self) -> String { format ! ("Hello, {}!" , self . name) } }
```

to_string() 的输出不还原缩进，token 之间留空格，适合断言、不适合阅读。但正是这种字符串断言，能在仓库里把"展开结果不能退化"固化成测试。

第三条是在宏体里直接用 eprintln!。过程宏运行在 rustc 进程里，它的 stderr 会出现在 cargo 的输出中，本机实测可见。它适合临时救急，不适合留下：每次展开都会打印一次，构建输出会被刷屏，而且不改变任何编译结果。要长期观察，还是回到第二条的测试。

最后提一句测试编译失败的方式：错误信息本身也可以被测试。trybuild 这个 crate 提供 ui 测试框架，把期望的报错写成 *.stderr 文件，与真实输出逐字比对。它是校验 syn::Error 消息和 span 是否稳定的标准手段。

## 34.8 编译期成本与卫生性

过程宏不产生运行期开销，它的账单全部记在编译期。

本机实测：干净构建一个只有 syn、quote、proc-macro2 三个依赖的小 workspace，cargo build --offline 用时 10.01 秒，绝大部分花在编译 syn 上；改动宏 crate 的源码后重新构建，用时 2.82 秒，因为三个依赖已经被 Cargo 缓存，只有宏 crate 和它下游的 app 需要重编。这两个数字说明了成本的分布：一次性成本是解析库本身，持续成本是"改宏就要重编所有使用者"。

由此有几条实用规则：

- syn 的 feature 要按需开。默认的 derive、parsing、printing、clone-impls、proc-macro 足够写 derive；解析函数体才需要 full。full 会显著增加编译时间。
- 能用 macro_rules! 解决的就别上过程宏。语法层面的重复在第 33 章的工具箱里通常有解；过程宏的价值在于需要真正的 Rust 逻辑、需要读属性、需要精确报错。
- 宏 crate 的依赖会被所有使用者间接编译一次。给宏 crate 引入一个大依赖，等于给整个下游加税。

运行期成本是零，但生成代码的体积不是零。derive 为每个类型生成一份 impl，泛型还会随单态化再复制。一个给上百个类型用的 derive，展开结果可能比手写的多不少，只是这些代码通常都能被内联和优化掉。

卫生性是过程宏最容易被低估的部分。macro_rules! 有"混合卫生"：宏里写的局部变量不会和调用处的同名变量冲突。过程宏没有这层保护——默认所有 token 的 span 都是调用处（call_site），生成的标识符按调用处的名字解析。三个直接后果：

第一，生成的代码里不要写 std:: 这种相对路径。使用者可能定义了同名模块，或者整个 crate 是 no_std。一律写 ::std:: 或 ::core:: 开头的绝对路径。本章的 hashmap! 就用了 ::std::collections::HashMap。

第二，不要在生成代码里调用宏 crate 自己的函数。那些函数在宏 crate 里，用户 crate 既看不见也链接不到。错误案例 5 就是这件事，报错是 E0425。

第三，自己造的辅助标识符要足够不可能撞车，并且尽量用 Span::mixed_site()。quote 的 format_ident! 支持指定 span：

```rust,noplayground
let tmp = format_ident!("__{}_start", name, span = proc_macro2::Span::mixed_site());
```

mixed_site 让这个名字获得接近 macro_rules! 的局部卫生性，不会与使用者手写的 __User_start 意外互踩。直接用 quote! 模板里写死的 let __start，则仍然是调用处卫生。

最后一条与构建系统有关：过程宏必须是确定性的。不要在宏里读时间、随机数、环境变量或文件内容来决定输出。这些值不会被 rustc 记录进依赖图，增量编译会给你一个与干净构建不一致的结果，可复现构建也会失败。

## 34.9 深入：TokenStream 里到底装了什么

### TokenTree：四类节点

proc_macro::TokenStream 不是字符串，也不是字节流，而是一棵由 TokenTree 组成的树。TokenTree 只有四种：

- Ident：标识符和关键字，例如 User、fn、self
- Punct：标点，带一个 Spacing，取值 Joint 或 Alone
- Literal：字面量，例如 42、"ana"、'x'
- Group：带分隔符的一组 token，Delimiter 取 Parenthesis、Brace、Bracket 或 None

Group 里又是 TokenStream，所以整体是递归的树。这也解释了为什么过程宏能"按括号切分"而不必做字符扫描。

Punct 的 Joint/Alone 用来区分多字符运算符。=> 在 token 层面是两个 Punct：= 是 Joint，> 是 Alone；写成 = > 中间有空格时，= 就是 Alone。syn 解析组合运算符时依赖这个标记。

一个把输入原样打印出来的 function-like 宏，能让你直观看到这棵树：

```rust,noplayground
use proc_macro::TokenStream;

#[proc_macro]
pub fn dump(input: TokenStream) -> TokenStream {
    for tt in input {
        eprintln!("{tt:?}");
    }
    TokenStream::new()
}
```

对 dump!(a + b) 调用，stderr 里会依次出现 Ident(a)、Punct('+', Alone)、Ident(b)。真实项目里很少直接操作 TokenTree，syn 已经把常用语法解析好了；但遇到 syn 不认识的语法，手写 TokenTree 遍历是最后的退路。

### 为什么过程宏看不到类型信息

编译器处理一个 crate 的顺序大致是：解析源文件 → 展开宏（含过程宏）→ 名称解析 → 类型检查 → 借用检查 → 代码生成。过程宏在第二步运行，此时类型检查还没开始，名称解析也没做完。它手里只有语法树，没有"这个类型实现了哪些 trait""这个字段的类型是什么"这类语义信息。

这不是实现偷懒，而是架构决定的：过程宏是编译器的外部程序，运行在宿主机上，通过 token 与编译器通信。它拿不到编译器的内部数据结构，也无法查询其他 item。于是：

- derive 想知道字段类型是否实现 Display，只能生成代码让编译器去检查，或者要求使用者用辅助属性显式声明。
- serde 必须发明 #[serde(rename = "新名字")]，因为宏看不见别的 crate 里的实现。
- 一个宏无法"扫描整个 crate"找某个标记，它只收到被标注的那一段 token。

理解这一点，就不会写出"宏能不能自动判断某个字段该不该序列化"这类无法实现的需求。

### 展开顺序

同一项上的多个属性，按从外到内的顺序展开；写在最前面的属性宏先运行，它收到的 item 里还带着后面的属性。derive 在其它属性宏之后处理，所以它看到的是被前面属性宏改写过的 item。项内部的宏调用（包括 derive 生成的宏调用）要等外层展开完，再逐层展开。

另一条硬性限制：过程宏不能在自己的 crate 里使用。定义宏的 crate 还没编译完，编译器无法加载它的动态库。所以测试宏只能走 34.7 的拆分函数路线，不能写一个 #[test] 去调用自己的 derive。

### 增量编译与缓存

过程宏 crate 首先被编译成一个动态库，然后 rustc 在处理使用者 crate 时加载它、调用入口函数、拿回 token。这个调用发生在每次需要展开的时候，展开结果进入 HIR，后续的类型检查和代码生成都在展开后的代码上做。

rustc 的增量缓存以查询为单位，展开结果会连同宏的输入一起被缓存；因此同一个 crate 内不改动相关代码时，重复构建不会重新执行你的宏。但缓存键里包含宏 crate 的元数据：宏 crate 一改，所有依赖它的 crate 都会失效并重新展开。Cargo 的 fingerprint 也是这个粒度，所以"只改了一行宏代码"会引起下游全量重编，这在大型项目里是真实的等待成本。

这也解释了 34.8 的最后一条：如果宏的输出依赖时间或随机数，缓存与新鲜构建会得到不同结果，问题极难排查。过程宏应当被当作纯函数——同样的输入 token，永远给同样的输出 token。

## 典型错误案例

下面五个错误都能在本机用 cargo build --offline 复现，报错原文来自 rustc 1.100.0。前三个发生在宏 crate 自己编译时，后两个发生在使用者 crate 编译时——这个区别本身就是排查过程宏问题的第一把尺子：先看报错文件路径指向谁的 src。

### 案例 1：宏写进了普通 lib，缺 proc-macro = true

现象：crate type 报错，附带 E0432。

```rust,noplayground
extern crate proc_macro;

use proc_macro::TokenStream;

#[proc_macro_derive(Nope)]
pub fn nope(input: TokenStream) -> TokenStream {
    input
}
```

真实的编译器输出：

```text
error: the `proc_macro_derive` attribute is only usable with crates of the `proc-macro` crate type
 --> e1-noproc/src/lib.rs:5:3
  |
5 | #[proc_macro_derive(Nope)]
  |   ^^^^^^^^^^^^^^^^^
```

如果把 extern crate proc_macro; 也删掉，第一条错误之前还会多一条：

```text
error[E0432]: unresolved import `proc_macro`
 --> e1-noproc/src/lib.rs:1:5
  |
1 | use proc_macro::TokenStream;
  |     ^^^^^^^^^^ use of unresolved module or unlinked crate `proc_macro`
  |
  = help: if you wanted to use a crate named `proc_macro`, use `cargo add proc_macro` to add it to your `Cargo.toml`
```

**原因**：Cargo 默认把 [lib] 当成普通库（crate type 为 rlib）编译。普通库既不能导入 proc_macro 这个编译器内部 crate，也不允许出现 #[proc_macro_derive] 等入口属性。编译器校验 crate type 时先把属性驳回，名称解析再报 proc_macro 找不到。这两条错误指向同一个根因，别被第二条的 help 误导去 cargo add proc_macro——那不解决问题。

**修复**：在宏 crate 的 Cargo.toml 里显式声明 crate 类型。注意 proc-macro crate 只能导出宏，不能同时当普通库用，所以这份配置必须放在 34.2 说的独立 crate 里：

```toml
[lib]
proc-macro = true
```

### 案例 2：忘了把 proc_macro2 的 token 转回 proc_macro

现象：E0308，expected TokenStream, found proc_macro2::TokenStream。

```rust,noplayground
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Nope)]
pub fn nope(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    quote! {
        impl #name {}
    }
}
```

真实的编译器输出：

```text
error[E0308]: mismatched types
   --> e2-ts/src/lib.rs:9:5
    |
  6 |   pub fn nope(input: TokenStream) -> TokenStream {
    |                                      ----------- expected `TokenStream` because of return type
  9 | /     quote! {
 10 | |         impl #name {}
 11 | |     }
    | |_____^ expected `TokenStream`, found `proc_macro2::TokenStream`
    |
    = note: `proc_macro2::TokenStream` and `TokenStream` have similar names, but are actually distinct types
help: call `Into::into` on this expression to convert `proc_macro2::TokenStream` into `TokenStream`
    |
 11 |     }.into()
    |      +++++++
```

换个位置犯同一个错，报错会变样：如果入口函数签名直接写成 proc_macro2::TokenStream，编译器报的是 derive proc macro has incorrect signature，并列出它期望的完整签名 fn(proc_macro::TokenStream) -> proc_macro::TokenStream。过程宏入口的类型是被编译器写死的，不能换。

**原因**：quote! 返回的是 proc_macro2::TokenStream，它是 proc_macro::TokenStream 的跨平台封装，名字像但是两个独立类型，没有任何隐式转换。编译器只接受 proc_macro::TokenStream 作为过程宏的输入输出，代码内部用 proc_macro2 是为了让逻辑可测试、可脱离编译器运行。

**修复**：在返回值处补一次 .into()。proc_macro2::TokenStream 和 proc_macro::TokenStream 互相都实现了 From，两个方向都能转：

```rust,noplayground
#[proc_macro_derive(Nope)]
pub fn nope(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    quote! {
        impl #name {}
    }
    .into()
}
```

### 案例 3：调用 to_token_stream 却没引入 trait

现象：E0599，no method named to_token_stream。

```rust,noplayground
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Nope)]
pub fn nope(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let expanded = name.to_token_stream();
    expanded.into()
}
```

真实的编译器输出：

```text
error[E0599]: no method named `to_token_stream` found for struct `syn::Ident` in the current scope
  --> e3-totokens/src/lib.rs:8:25
   |
 8 |     let expanded = name.to_token_stream();
   |                         ^^^^^^^^^^^^^^^
   |
   = help: items from traits can only be used if the trait is in scope
help: trait `ToTokens` which provides `to_token_stream` is implemented but not in scope; perhaps you want to import it
   |
 1 + use quote::to_tokens::ToTokens;
   |
help: there is a method `into_token_stream` with a similar name
   |
 8 |     let expanded = name.into_token_stream();
   |                         ++
```

**原因**：to_token_stream 不是 Ident 自带的方法，而是 quote::ToTokens trait 提供的方法。Rust 要求 trait 必须在作用域里，方法才会出现在方法解析结果中。这个文件只写了 use quote::quote;，引的是宏而不是 trait，所以编译器说"trait 已实现但不在作用域"。quote! 模板里写 #name 之所以没事，是因为宏展开成的是完全限定的方法调用，不依赖引入。

**修复**：导入 trait。编译器提示的 quote::to_tokens::ToTokens 能用，但惯用写法是直接使用重导出路径：

```rust,noplayground
use quote::ToTokens;

let expanded = name.to_token_stream();
```

### 案例 4：把函数当 DeriveInput 解析

现象：expected one of: struct, enum, union。

宏 crate 里把 attribute 宏的输入当作 derive 输入来解析：

```rust,noplayground
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_attribute]
pub fn as_derive(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = input.ident;
    quote::quote! { impl #name {} }.into()
}
```

使用者把它标注在函数上：

```rust,noplayground
use e4_macros::as_derive;

#[as_derive]
fn compute() -> u32 {
    1
}
```

真实的编译器输出：

```text
error: expected one of: `struct`, `enum`, `union`
 --> e4-app/src/main.rs:4:1
  |
4 | fn compute() -> u32 {
  | ^^
```

**原因**：DeriveInput 的解析器只认 struct、enum、union 三种开头。它看到 fn 这个关键字时，lookahead 直接把候选集列出来。注意报错位置在**使用者**的 main.rs，而不是宏 crate——因为解析发生在展开期，span 指向被展开的 token。这类"报错文件不在宏 crate"的现象，正是判断问题出在输入形状而不是宏实现的重要线索。

**修复**：按被标注的目标选类型。标注函数就用 ItemFn，标注结构体就用 ItemStruct；如果一个宏要同时接受多种目标，解析成 syn::Item 再 match：

```rust,noplayground
use proc_macro::TokenStream;
use syn::{parse_macro_input, Item, ItemFn};

#[proc_macro_attribute]
pub fn as_derive(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let name = match &item {
        Item::Fn(ItemFn { sig, .. }) => sig.ident.clone(),
        Item::Struct(s) => s.ident.clone(),
        other => {
            let err = syn::Error::new_spanned(other, "只支持函数和结构体");
            return err.to_compile_error().into();
        }
    };
    quote::quote! { impl #name {} }.into()
}
```

### 案例 5：生成的代码调用了宏 crate 里的私有函数

现象：E0425，cannot find function。

宏 crate 里先写了一个辅助函数，再在 quote! 里直接调用它：

```rust,noplayground
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

fn hello_name_impl(name: &str) -> String {
    format!("Hello, {}!", name)
}

#[proc_macro_derive(HelloName)]
pub fn derive_hello_name(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    quote! {
        impl #name {
            pub fn hello_name(&self) -> String {
                hello_name_impl(&self.name)
            }
        }
    }
    .into()
}
```

使用者这边只写正常的 derive，编译却失败：

```text
error[E0425]: cannot find function `hello_name_impl` in this scope
 --> e5-app/src/main.rs:3:10
  |
3 | #[derive(HelloName)]
  |          ^^^^^^^^^ not found in this scope
  |
  = note: this error originates in the derive macro `HelloName` (in Nightly builds, run with -Z macro-backtrace for more info)
```

宏 crate 自己还会顺带报一条 warning: function hello_name_impl is never used——因为它在宏 crate 里确实没人调用。

**原因**：这是卫生性与可见性的双重问题。过程宏生成的 token 默认带调用处的 span，hello_name_impl 这个名字会在**使用者 crate** 里解析；而那个函数定义在宏 crate 里，是私有项，也不在用户 crate 的依赖图中（proc-macro crate 只能导出宏，运行期不可链接）。编译器找不到它，只能按"未定义的函数"报 E0425。注意报错指向 derive 那一行，而不是宏 crate，这是过程宏问题的典型特征。

**修复**：要么把逻辑内联进生成代码，要么把共享代码放进一个普通运行时 crate，让使用者也依赖它，再用绝对路径调用：

```rust,noplayground
// 宏 crate 里：只生成内联表达式，不引用任何外部名字
quote! {
    impl #name {
        pub fn hello_name(&self) -> String {
            format!("Hello, {}!", self.name)
        }
    }
}

// 如果逻辑确实要共享，就建一个普通 crate（例如 hello-runtime），
// 宏生成 ::hello_runtime::hello_name(&self.name)，并由使用者依赖 hello-runtime。
```

## 动手练习

1. 给 34.3 的 HelloName 补上泛型支持。用 split_for_impl 拆出三段并填进模板，然后构造 `struct Wrapper<T> { name: String, inner: T }`，确认 #[derive(HelloName)] 后 hello_name 仍能调用。再把它搬进 expand 函数，用 parse_quote! 写一个断言生成代码里有 `impl<T>` 的单元测试。

2. 让 #[timed] 接受参数。要求 #[timed] 用函数名当标签，#[timed(label = "db")] 用给定字符串。提示：attr 是一个 TokenStream，若为空就退回 sig.ident，否则用 `syn::parse::<syn::MetaNameValue>` 解析它，从 LitStr 里取值。改完后给两种写法各加一个调用，确认输出里的标签不同。

3. 扩展 hashmap!。把键值分隔符从冒号改成 =>，即 hashmap! { "a" => 1, "b" => 2 }。需要把 Pair::parse 里的 Token![:] 换成两个 Punct 组成的 =>，或者直接用 `input.parse::<Token![=>]>()`。再进一步：允许重复键，后写的覆盖先写的，并用 HashMap 的 len 验证。

4. 写一个 CountFields derive，为结构体生成 pub const FIELD_COUNT: usize。要求：元组结构体和具名结构体都支持；遇到 enum 或 union 时用 syn::Error::new_spanned 返回“只支持结构体”，并把 span 落在类型名上。最后用 34.7 的 expand 函数断言错误分支真的产出了 compile_error! 文本。

5. 用 34.9 的 dump! 观察 token 树。分别调用 dump!(a => b) 和 dump!(a = > b)，对比 Punct 的 Spacing 是 Joint 还是 Alone；再调用 dump!(f(x, [y]))，观察 Group 的 Delimiter 如何嵌套。把 eprintln! 的输出整理成一张表，说明为什么 syn 能在不扫描字符的情况下知道括号是否配对。

## 小结

- 过程宏有三种入口：derive 追加代码、attribute 替换被标注的项、function-like 替换整个调用，三者边界类型都是 proc_macro::TokenStream。
- 过程宏必须住在独立的 proc-macro = true crate 里。这个 crate 只能导出宏，运行期不可链接，共享逻辑要放到配套的普通 crate。
- syn 负责把 token 解析成语法树，quote 负责用模板生成 token，proc-macro2 是两者之间以及与测试代码之间流通的 token 类型。
- derive 的核心是 DeriveInput：看 ident、generics、data，按 Data::Struct、Data::Enum、Data::Union 分支处理；只能在原 item 后追加，不能改写它。
- attribute 宏要把 ItemFn 的 attrs、vis、sig、block 拆开再拼回，签名整段复用；提前 return、? 和 async 都会让朴素的计时包装失真。
- function-like 宏用自定义 Parse 加 Punctuated::parse_terminated 处理重复和尾逗号，比 macro_rules! 更灵活，代价是多一个 crate 和一套依赖。
- 报错走 syn::Error 加 to_compile_error，span 尽量落在使用者的 token 上；compile_error! 不会中止后续检查，所以报错后要立即返回，避免级联噪音。
- 调试的首选是把生成逻辑拆成吃 DeriveInput、吐 proc_macro2::TokenStream 的普通函数，用 parse_quote! 加单元测试断言展开结果；cargo expand 适合有 nightly 的机器。
- 过程宏的成本全在编译期：syn 的 feature 要克制，改宏会重编所有下游；生成的代码必须用绝对路径、不引用宏 crate 内部名字，并且保持确定性，否则增量编译和可复现构建都会出问题。
