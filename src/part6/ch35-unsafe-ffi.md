# 第 35 章 unsafe 与 FFI

> **本章目标**
>
> - 说清 unsafe 打开的五个口子，以及它仍然关着的借用、类型、初始化三道检查
> - 会创建、判空、做算术、在引用与裸指针之间转换，并知道每一步谁需要 unsafe
> - 会写带 SAFETY 注释的 unsafe fn，把「调用方义务」写成可核查的清单
> - 能用裸指针实现 split_at_mut 这样的安全抽象，并说清它的不变量
> - 会写 extern "C" 的导出与导入，用 CString/CStr 与 Box::into_raw/from_raw 管好跨语言所有权
> - 记住 FFI 的三条铁律，知道 Miri、repr(C)、bindgen 各自解决什么问题

Rust 的安全保证有一道明确的边界：编译器能看见的代码，它替你证明内存安全；编译器看不见的东西——另一个语言写的函数、操作系统给的地址、硬件寄存器、你自己知道的布局事实——它无法证明，只能让你签字担保。unsafe 就是这张签字单。

很多人对 unsafe 有两种误解。一种是「unsafe 很危险，能不用就不用」，于是写出又慢又绕的代码；另一种是「unsafe 就是关掉安全检查的性能开关」，于是随手解引用空指针。两种都错。unsafe 不是模式开关，它是一组**恰好五个**的额外能力，除此之外的所有检查照常执行；写出正确的 unsafe 代码靠的不是勇气，而是把「为什么这里安全」写成一行行可以被别人核对的前提。

本章前半讲这五个能力分别是什么、裸指针的操作边界在哪、如何用 unsafe 包出一个对外完全安全的抽象。后半讲 FFI（Foreign Function Interface，外部函数接口）：把 Rust 函数导出给 C 调用，把 C 函数导入给 Rust 调用，以及跨语言时谁负责释放内存、panic 能不能穿过边界这些必须提前定好的规矩。最后一节回到未定义行为（Undefined Behavior，UB）本身，看清它为什么不是「未定义但实际能用」。

## 35.1 unsafe 关掉什么，没关掉什么

先给结论。unsafe 只允许你做下面五件事，它们互不重叠：

1. **解引用裸指针**：读或写 `*const T`、`*mut T` 指向的内存。
2. **调用 unsafe 函数**，包括所有用 `extern` 声明进来的外部函数。
3. **访问或修改可变静态量** `static mut`，以及取得它的引用。
4. **实现 unsafe trait**，典型的是 `Send`、`Sync` 和 `GlobalAlloc`。
5. **访问 union 的字段**，因为编译器无法知道当前活跃的是哪个变体。

这五件事之外的一切都没有变。**创建**一个裸指针不需要 unsafe，给裸指针变量赋值、比较两个指针、调用 `is_null`、做 `p as *const u8` 这样的类型转换，全都能在安全代码里完成。真正需要签字的只有「通过指针碰内存」那一步。

要特别记牢的是，unsafe 块**不会**打开下面三道闸门：

```rust,noplayground
fn main() {
    let mut x = 1;
    let a = &mut x;
    let b = &mut x;
    unsafe {
        *a += 1;
        *b += 1;
    }
    println!("{}", x);
}
```

```text
error[E0499]: cannot borrow `x` as mutable more than once at a time
 --> src/main.rs:4:13
  |
3 |     let a = &mut x;
  |             ------ first mutable borrow occurs here
4 |     let b = &mut x;
  |             ^^^^^^ second mutable borrow occurs here
5 |     unsafe {
6 |         *a += 1;
  |         ------- first borrow later used here

error: aborting due to 1 previous error
```

- **借用检查照旧**。上面这段在 `&mut x` 第二次出现时就报 E0499，跟有没有 unsafe 块无关。想真正造出两个别名，得先用裸指针把引用「洗掉」——那才是 UB 的来源。
- **类型检查照旧**。`*p` 的类型由 `p` 的类型决定，不存在「unsafe 里可以乱转」这回事；类型不匹配仍然报 E0308。
- **初始化与移动检查照旧**。在 unsafe 块里读一个未初始化的变量、把已经移动的值再用一次，一样报错。

所以 unsafe 的正确读法是：**它把「编译器证明不了」的部分交给你，同时把「编译器能证明」的部分原样保留。** 你签的字只覆盖那五个能力；覆盖不到的地方报错，说明你根本不需要 unsafe，删掉它反而更好。

## 35.2 裸指针：*const T 与 *mut T

裸指针是 unsafe 最常打交道的东西。它和引用长得像，但语义完全不同：

- 引用保证不为空、指向有效且对齐的数据、遵守借用规则；裸指针**什么也不保证**：可能为空、悬垂、未对齐，也可以同时存在多个指向同一地址的可变指针。
- 引用带生命周期，编译器据此安排使用顺序；裸指针不带生命周期，使用期限由你自己保证。
- 因此引用可以随手转成裸指针（这一步安全），裸指针转回引用则要 unsafe，并且由你负责上面列出的全部条件。

创建裸指针的三种常见方式：

```rust
fn main() {
    let value = 42;
    let p1 = &value as *const i32;

    let mut v = vec![1, 2, 3];
    let p2 = v.as_mut_ptr();

    let p3 = std::ptr::null::<i32>();

    println!("{} {} {}", p1.is_null(), p2.is_null(), p3.is_null());
    unsafe { *p2 = 9 };
    println!("{:?}", v);
}
```

解引用才开始要 unsafe。下面这段用指针遍历数组：

```rust
fn main() {
    let data = [10, 20, 30, 40];
    let q = data.as_ptr();
    unsafe {
        for i in 0..data.len() {
            println!("第 {} 个：{}", i, *q.add(i));
        }
    }
}
```

`q.add(i)` 表示「向后移动 i 个 `i32`」，单位是元素而不是字节，这正是指针有类型的原因。它和 C 的 `p + i` 一样，只允许落在同一个分配对象（这里是数组）的内部或末尾后一个位置，再往外就违规。想按字节移动用 `q.byte_add(n)`；想明确表达「可能溢出、采用环绕语义」用 `q.wrapping_add(n)`（这个是安全函数）。

判空用 `is_null`。如果某个指针从设计上就不允许为空，可以用 `std::ptr::NonNull<T>` 表示：它保证非空，而「非空」这个不变量让编译器能把 `Option<NonNull<T>>` 压成一个指针大小（niche 优化，第 8 章讲对齐时提过）。

裸指针和引用互转的完整路径：

```rust
fn main() {
    let mut value = 7;

    let p = std::ptr::addr_of_mut!(value);
    unsafe {
        *p = 9;
        let r: &i32 = &*p;
        println!("r = {}", r);
    }

    let big = &value as *const i32;
    let small = big as *const u8;
    println!("首个字节 = {}", unsafe { *small });
}
```

`addr_of_mut!(value)` 比 `&mut value as *mut i32` 更值得推荐：后者会先创建一个临时的 `&mut`，如果这块内存因为别的原因已经处于「可写但不能借」的状态，创建引用这一动作本身就违规了；`addr_of_mut!` 不创建引用，直接把地址取出来。这是很多人第一次写 unsafe 时会踩的坑。

## 35.3 unsafe fn 与 SAFETY 注释

`unsafe fn` 的意思不是「这个函数内部有 unsafe 代码」，而是「**调用这个函数的人必须满足某些前提，编译器查不出来**」。所以 `unsafe` 修饰符是函数签名的一部分，和参数类型一样对外公开；文档里的 `# Safety` 一节是它的使用说明。

Rust 标准库在这件事上非常严格：`Vec::set_len`、`slice::from_raw_parts`、`ptr::read` 的文档第一段永远是 `# Safety`，逐条列出调用方要保证什么。你写自己的 unsafe fn 时也应该这样，因为下一个读代码的人（可能是三个月后的你）只能靠这段话判断能不能调用。

一条可执行的 SAFETY 注释包含三样东西：

1. **不变量**：什么样的指针、长度、对齐、别名关系是允许的。
2. **谁保证**：调用方保证，还是函数内部自己检查。
3. **违反后果**：越界、悬垂、数据竞争，还是仅仅结果错误。

```rust
use std::ptr;

/// 从 `base` 偏移 `offset` 字节处读一个按小端存放的 u32。
///
/// # Safety
///
/// - `base` 必须非空，且从 `base` 起至少有 `offset + 4` 个字节可读；
/// - 这段内存在调用期间不能被其它线程写入；
/// - `offset` 不能超过 `isize::MAX`。
unsafe fn read_u32_at(base: *const u8, offset: usize) -> u32 {
    debug_assert!(!base.is_null(), "base 不能为空");
    // SAFETY：调用方保证偏移和四个已初始化字节均位于同一分配内。
    let p = unsafe { base.add(offset) }.cast::<u32>();
    u32::from_le(unsafe { ptr::read_unaligned(p) })
}

fn main() {
    let bytes: [u8; 8] = [1, 0, 0, 0, 2, 0, 0, 0];
    let a = unsafe { read_u32_at(bytes.as_ptr(), 0) };
    let b = unsafe { read_u32_at(bytes.as_ptr(), 4) };
    println!("{} {}", a, b);
}
```

这段代码有三个细节值得停一下。

**`read_unaligned` 而不是 `read`。** `bytes.as_ptr().add(4)` 的地址按类型只保证 `u8` 对齐，不保证满足 `u32` 的 4 字节要求（运行时它碰巧可能是对齐的，但不能依赖）。`ptr::read` 要求指针对齐，否则就是 UB；`read_unaligned` 明确表示「我知道可能不对齐，请按字节拼」。选哪个不是风格问题，是正确性问题。

**`debug_assert!` 的位置。** 它检查的是「调用方违反了契约」这种程序员错误，只在 debug 构建里生效，release 下整条语句消失，所以不能拿它防外部输入。来自文件、网络、C 库的输入要用 `assert!` 或者返回 `Result`，那是程序逻辑的一部分，不能省。

**内层还写了一个 `unsafe` 块。** 在 edition 2021 里，`unsafe fn` 的函数体本身就是一个 unsafe 上下文，这个内层块是多余的；但 edition 2024 起不再如此，`unsafe_op_in_unsafe_fn` 会要求你把每个危险操作单独圈出来。提前这么写，迁到 2024 时一行都不用改，而且读者能一眼看到危险发生在哪一行。

调用点的注释同样重要，而且比函数文档更短：

```rust,noplayground
// SAFETY: bytes 是长度 8 的数组，offset 为 4，满足「至少有 8 个字节可读」；
// 数组在 main 的栈帧里，调用期间没有其它线程访问。
let b = unsafe { read_u32_at(bytes.as_ptr(), 4) };
```

「函数内部有 unsafe 块」和「调用方要写 SAFETY」是两件事。前者说明实现里用了裸指针，后者说明调用有前提。一个设计良好的库，对外暴露的接口应该尽量是安全函数，把 unsafe 全部关在里面——这就是下一节要做的事。

## 35.4 用 unsafe 实现一个安全抽象

标准库有 `slice::split_at_mut`：把一个 `&mut [T]` 按索引切成两半，两半可以同时修改。

```rust
fn main() {
    let mut data = [1, 2, 3, 4];
    let (left, right) = data.split_at_mut(2);
    left[0] = 10;
    right[0] = 40;
    println!("{:?} {:?}", left, right);
}
```

如果不用标准库，只用安全代码，这件事写不出来。直觉写法是这样：

```rust,noplayground
fn split_at_mut<T>(values: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {
    (&mut values[..mid], &mut values[mid..])
}
```

编译器会拒绝，理由是「不能把 `values` 同时借两次」，即使两次借用的下标区间根本不重叠。借用检查器的工作单位是**整个引用**，它不做区间算术——这是它为了保证可靠性付出的代价，不是缺陷。

正确的实现要把「我知道这两个区间不重叠」这件事翻译成裸指针操作，再用类型签名把这个保证重新封起来：

```rust
use std::slice;

fn split_at_mut<T>(values: &mut [T], mid: usize) -> (&mut [T], &mut [T]) {
    let len = values.len();
    assert!(mid <= len, "切分点 {} 超过长度 {}", mid, len);

    let ptr = values.as_mut_ptr();
    unsafe {
        (
            slice::from_raw_parts_mut(ptr, mid),
            slice::from_raw_parts_mut(ptr.add(mid), len - mid),
        )
    }
}

fn main() {
    let mut data = [1, 2, 3, 4, 5, 6];
    let (left, right) = split_at_mut(&mut data, 3);
    left[0] = 10;
    right[0] = 40;
    println!("{:?} + {:?}", left, right);
    println!("原数组：{:?}", data);
}
```

输出的两半确实是同一块内存的两个不相交部分，改写会反映到原数组上。

这段 15 行的代码之所以是安全的，靠四条不变量，缺一条就是 UB：

1. **`mid <= len`**：由 `assert!` 保证。没有它，`from_raw_parts_mut(ptr, mid)` 会造出一个超出分配范围的切片。
2. **两段不重叠**：第一段是 `[0, mid)`，第二段是 `[mid, len)`，由 `ptr.add(mid)` 保证起点相接。
3. **长度之和不超过原长**：`mid + (len - mid) == len`，由减法本身保证。
4. **生命周期绑定回输入**：签名写的是 `(&mut [T], &mut [T])`，省略的生命周期让两个返回值都借用 `values`，所以调用方不可能在持有切片时又去动原数组——这一条是**类型系统**帮你保证的，不需要 unsafe。

第 4 条最容易被忽略。很多人以为「只要指针算对了就安全」，其实 `from_raw_parts_mut` 返回的切片生命周期是推断出来的，如果函数签名写成返回 `&'static mut [T]`，指针再正确也是悬垂引用。

审查这类代码有一个固定流程：先把不变量逐条列出来，再对每条问「谁保证的」，最后为每条写一个测试。上面四条里，第 1 条用 `#[should_panic]` 测，第 2、3 条用「改左半看右半」测，第 4 条交给编译器——如果你试着在持有 `left` 时再调用 `data.len()`，借用检查器会拦住你。

标准库的实现思路与此相同：内部同样用 `ptr::slice_from_raw_parts_mut` 加偏移构造两个切片，只是越界检查的位置与 panic 信息更讲究。`get_unchecked` 是同一类抽象的另一个例子：它把「索引一定在范围内」的证明责任交给调用方，换来去掉边界检查的读。

## 35.5 extern "C"：把边界画出来

本书按 2021 edition 编译。若把下面的示例移到 2024 edition，导入声明块要写 `unsafe extern "C" { ... }`，导出符号的属性要写 `#[unsafe(no_mangle)]`。前者要求声明者核对外部函数签名，后者要求导出者确保符号名不会冲突。这两种新写法从 Rust 1.82 起也适用于 2021 edition。

FFI 有两个方向：把 Rust 函数导出给别的语言调用，和把别的语言的函数导入给 Rust 调用。两个方向都靠 `extern "C"` 描述 ABI（Application Binary Interface，应用二进制接口）。

### 导出：让 C 调用 Rust

```rust
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(2, 3));
}
```

两个属性各管一件事。`extern "C"` 规定调用约定按 C 的来——参数怎么进寄存器、栈由谁清理、返回值放哪，都按平台约定执行。`#[no_mangle]` 阻止 Rust 把函数名改写成带哈希的符号（mangling），这样链接器里就真的有一个叫 `add` 的符号，C 才能按名字找到它。

C 那边的头文件写：

```c
#include <stdint.h>

int32_t add(int32_t a, int32_t b);
```

把 crate 编成动态库要设置 `crate-type`：

```toml
[lib]
crate-type = ["cdylib"]
```

然后 `cargo build --release` 会在 `target/release/` 下产出 `.dll`（Windows）、`.so`（Linux）或 `.dylib`（macOS），C 程序链接它即可。这里用 `no_mangle` 公开名为 `add` 的符号，也可以用 `export_name` 指定导出名。

### 导入：让 Rust 调用 C

```rust
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

extern "C" {
    fn strlen(s: *const c_char) -> usize;
}

fn c_len(s: &CStr) -> usize {
    unsafe { strlen(s.as_ptr()) }
}

fn main() {
    let owned = CString::new("hello ffi").unwrap();
    println!("长度：{}", c_len(&owned));

    let raw = owned.as_ptr();
    let view = unsafe { CStr::from_ptr(raw) };
    println!("再读一次：{}", view.to_str().unwrap());
}
```

`extern "C"` 块只是**声明**，不产生代码；链接阶段由链接器去 C 运行库里找名叫 `strlen` 的符号。所以调用它必须放在 unsafe 块里：编译器不知道那个函数会不会写坏你传进去的指针。

类型上有个细节：`c_char` 在 x86_64 Linux 和 Windows 上是 `i8`，在部分 ARM 平台上是 `u8`。别直接写 `i8`，用 `std::os::raw::c_char`，让平台自己决定。同理，`c_int`、`c_uint`、`size_t` 这些别名就是为跨平台准备的。

Linux 和 macOS 上 C 标准库默认参与链接，所以 `strlen` 直接可用；调用其它库要在 `build.rs` 里写 `println!("cargo:rustc-link-lib=m")` 这类指令，或者用现成的 `-sys` crate。

### C 字符串：没有长度字段的字节串

C 不认识 Rust 的 `String`：它不是以 0 结尾的，长度存在结构体里，而且内部可能包含 0。C 世界的字符串约定是「从指针开始，直到遇到第一个 0 字节」，于是标准库给了两个类型：

- `CString`：**拥有**一块以 0 结尾的内存。`CString::new("abc")` 会检查内容里有没有 0，有就返回 `Err`——因为一旦传给 C，0 后面的内容会被当成不存在。
- `CStr`：**借用**一段以 0 结尾的字节序列，是 C 字符串的 Rust 视图。`CStr::from_ptr` 是 unsafe 的，调用方要保证指针指向一段可读、以 0 结尾的内存；`to_str()` 再检查 UTF-8 并返回 `Result`。

记住 `as_ptr` 的生命周期：它借的是 `CString` 自己。`CString` 一旦被 drop，缓冲被释放，那个指针立刻悬垂。危险的地方在于，**裸指针不带生命周期，编译器不会拦你**：

```rust,noplayground
use std::ffi::CString;
use std::os::raw::c_char;

fn bad() -> *const c_char {
    let s = CString::new("boom").unwrap();
    s.as_ptr()   // 编译通过：返回的指针指向已经释放的缓冲
}
```

这段代码能编译、能运行，把返回的地址打印出来也像模像样。换成返回引用，借用检查器立刻就发现了：

```rust,noplayground
use std::ffi::CString;

fn bad_str() -> &'static str {
    let s = CString::new("boom").unwrap();
    s.to_str().unwrap()
}
```

```text
error[E0515]: cannot return value referencing local variable `s`
 --> src/main.rs:5:5
  |
5 |     s.to_str().unwrap()
  |     -^^^^^^^^^^^^^^^^^^
  |     |
  |     returns a value referencing data owned by the current function
  |     `s` is borrowed here
```

同一个错误，引用版本被拦住，裸指针版本放行——差别不在安全性，而在**编译器能不能把这个约束表达出来**。修法是把 `CString` 的所有权交给调用方（`Box::into_raw`），或者要求调用方自己持有它，这正是下一小节的模式。

### 不透明指针与所有权移交

C 侧只想拿一个句柄（handle）来调用你的 Rust 对象，不想知道它内部长什么样。做法是：Rust 用 `Box::into_raw` 把对象的所有权交出去，只返回裸指针；C 拿着指针调用配套函数；最后调用配套的释放函数，用 `Box::from_raw` 把所有权收回来并 drop。

```rust
#[repr(C)]
pub struct Counter {
    value: i64,
}

#[no_mangle]
pub extern "C" fn counter_new(init: i64) -> *mut Counter {
    Box::into_raw(Box::new(Counter { value: init }))
}

#[no_mangle]
pub unsafe extern "C" fn counter_add(handle: *mut Counter, delta: i64) {
    if handle.is_null() {
        return;
    }
    unsafe { (*handle).value += delta };
}

#[no_mangle]
pub unsafe extern "C" fn counter_free(handle: *mut Counter) {
    if handle.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(handle)) };
}

fn main() {
    let h = counter_new(10);
    unsafe { counter_add(h, 5) };
    println!("{}", unsafe { (*h).value });
    unsafe { counter_free(h) };
    println!("已释放");
}
```

C 那头只需要一行前置声明，结构体定义完全不导出：

```c
struct Counter;

struct Counter *counter_new(int64_t init);
void counter_add(struct Counter *handle, int64_t delta);
void counter_free(struct Counter *handle);
```

三条约束必须同时满足。第一，`Box::from_raw` 只能对 `Box::into_raw` 产生的指针调用，而且**恰好一次**；忘了调用会泄漏，调用两次会 double free。第二，`handle` 可能为 NULL，进函数前先检查；Rust 的 `&mut T` 引用不允许为空，所以不能直接写 `&mut *handle` 而不检查。第三，`counter_add` 和 `counter_free` 内部解引用了裸指针，因此它们本身是 `unsafe extern "C" fn`——ABI 变成 C 的，但 Rust 调用方仍要写 unsafe。

## 35.6 FFI 的三条铁律

前两节把语法摆完了，这一节只讲规矩。跨语言边界的 bug 几乎都出在下面三件事上。

**第一，谁分配，谁释放。** 分配和释放必须用同一个分配器。Rust 的 `String`、`Vec`、`Box` 用的是 Rust 全局分配器（默认是平台 malloc 的封装，但不保证与 C 的 `malloc` 是同一个实现），C 的 `free` 只认识 `malloc`/`calloc`/`realloc` 返回的指针。把 `Box::into_raw` 的指针交给 C 的 `free`，或者把 `malloc` 的内存交给 `Box::from_raw`，都是 UB。

正确做法是配成一对导出：你提供 `counter_new` 就必须提供 `counter_free`，并在头文件注释里写清楚「只能用 counter_free 释放」。如果确实需要 C 分配、Rust 释放，就在两侧统一走 libc 的 `malloc`/`free`，别混用。

**第二，panic 不能穿过 FFI 边界。** Rust 的 panic 默认是 unwind（逐层退栈），但 C 代码没有对应的清理机制。从 rustc 1.71 起，如果 panic 试图离开一个 `extern "C"` 函数，运行时会直接 abort 并打印 `panic in a function that cannot unwind`；更早的版本这是 UB。无论哪种，都不是你想要的结果。

处理方式是**在边界内解决**：用 `std::panic::catch_unwind` 包住可能 panic 的代码，把 panic 转成一个错误码或负返回值。

```rust,noplayground
#[no_mangle]
pub extern "C" fn safe_div(a: i32, b: i32) -> i32 {
    let result = std::panic::catch_unwind(|| a / b);
    match result {
        Ok(v) => v,
        Err(_) => -1,
    }
}
```

如果确实需要让 unwind 穿过边界（比如 C++ 调用方有异常处理），把那侧的 ABI 写成 `extern "C-unwind"`，两侧都同意才能用。另外，把 `panic = "abort"` 写进 `[profile.release]` 后 panic 直接终止进程，catch_unwind 也拦不住，这是发布版的常见选择。

**第三，ABI 与类型大小要对齐。** 两边必须对同一块内存有完全一致的解释。常见对照如下：

| Rust 类型 | C 类型 | 注意 |
| --- | --- | --- |
| i32 / u32 | int32_t / uint32_t | 宽度一致，符号必须对上 |
| i64 / u64 | int64_t / uint64_t | 不要用 long，它在 Windows 上是 32 位 |
| usize / isize | size_t / ptrdiff_t | 宽度随目标平台变化 |
| std::os::raw::c_char | char | 有无符号随平台变化 |
| bool | _Bool | 只有 0 和 1 合法，传 2 进来是 UB |
| *const T / *mut T | const T* / T* | 可空；Rust 引用不可为空 |
| #[repr(C)] struct | struct | 默认布局是与 C 不兼容的 repr(Rust) |
| String / Vec / &str | 无 | 不能直接传，改用指针加长度或 C 字符串 |

三点补充。调用约定也必须一致：`extern "C"` 与 `extern "system"` 在 64 位 Windows 上恰好等价，但在 32 位 Windows 上不同，跨平台代码里要明确写。枚举要加 `#[repr(C)]` 或者用显式的整数类型，否则 Rust 可能选一个更窄的表示。struct 必须 `#[repr(C)]`，因为默认的 `repr(Rust)` 允许编译器重排字段以压缩体积，五次编译可能得到五种布局。

## 35.7 工具链：Miri、repr(C)、bindgen

unsafe 代码不能只靠眼睛审。这一节介绍三件工具，它们分别负责「动态检查」「布局约定」「绑定生成」。

### Miri：把未定义行为变成一条报错

Miri 是 rustc 的 MIR 解释器。它不把程序编译成机器码，而是逐条解释中间表示，并在每一步检查那些正常运行时没人检查的规则：指针是否越界、内存是否已释放、值是否未初始化、对齐是否满足、两个线程是否在无同步的情况下访问同一块内存、`&mut` 的别名关系是否被破坏。

用法只有三行：

```console
$ rustup +nightly component add miri
$ cargo +nightly miri test
$ cargo +nightly miri run
```

它需要 nightly 工具链，因为 Miri 依赖编译器内部接口，稳定版不提供。离线环境装不了组件，这一点要提前确认；但只要你手里有网，`cargo miri test` 应该是每个 unsafe 模块的常规检查项。

Miri 有三个边界，知道了才不会误用：

- **它只检查执行到的路径**。测试没覆盖的分支，UB 照样溜过去。所以 Miri 要和覆盖率一起看。
- **它不懂外部世界**。调用真正的 C 函数时 Miri 会报 unsupported，需要把 FFI 调用隔离出去，或者用 `#[cfg(miri)]` 换成纯 Rust 的桩。
- **它慢**。解释执行通常比原生慢两个数量级，适合跑单元测试，不适合跑集成压测。

### repr(C) 家族

`#[repr(C)]` 这类属性决定类型的布局。默认的 `repr(Rust)` 允许编译器重排字段、插入填充、选择更窄的判别值，只要语义不变；这对性能有利，对 FFI 是灾难。

- `#[repr(C)]`：字段按声明顺序排列，对齐规则与 C 一致。结构体、枚举、union 跨边界时都必须加。
- `#[repr(transparent)]`：只有一个非零大小字段的结构体，布局与那个字段完全相同。用来做 newtype 包装而不引入 ABI 差异。
- `#[repr(packed)]`：去掉所有填充，对齐为 1。省内存，但取字段引用会产生未对齐引用，编译器会直接报错；读字段要用 `read_unaligned` 或者先复制到局部变量。
- `#[repr(align(N))]`：把对齐提高到 N 的倍数，常用于避免伪共享或满足 SIMD 要求。
- `#[repr(u8)]` 等：给枚举指定判别值的整数类型，跨语言传枚举时必备。

### bindgen 与 cbindgen

手写 `extern` 声明在头文件只有十几个函数时没问题；对接 OpenSSL、SQLite 这种几百个函数的库，手写既不现实也容易错。两个工具各管一个方向：

- **bindgen** 读 C 头文件，生成 Rust 的 `extern` 块、`#[repr(C)]` 结构体、常量与类型别名。通常放在 `build.rs` 里跑，产物写进 `OUT_DIR` 再 `include!` 进来。它依赖 libclang，所以构建机要装 LLVM。
- **cbindgen** 读 Rust 源码，生成 C 头文件。适合「Rust 写库、C 写调用方」的方向，能自动把 `#[no_mangle] extern "C"` 函数和 `#[repr(C)]` 类型翻译成声明。

生态里更常见的组织方式是 `-sys` crate：`foo-sys` 只放 bindgen 生成的原始绑定，全是 unsafe；`foo` 在它上面包一层安全 API，把指针、长度、生命周期重新管起来。这就是本章反复讲的模式——**unsafe 集中在最薄的一层，对外只暴露安全函数**。

## 35.8 深入：未定义行为到底是什么

前面所有规则最后都指向一个概念：UB。它的定义不是「程序会崩溃」，而是「编译器不再对这段程序的行为作任何承诺」。这个区别决定了你该怎么看待那些「跑起来看着正常」的 unsafe 代码。

### 三个真实的 UB

**悬垂指针。** 下面这段先释放堆内存，再从旧指针读：

```rust,noplayground
fn main() {
    let p = Box::into_raw(Box::new(42i32));
    unsafe { drop(Box::from_raw(p)) };
    let v = unsafe { *p };
    println!("读到的值 = {}", v);
}
```

在本机 debug 构建下连跑三次，输出是：

```text
读到的值 = 54275728
读到的值 = 722252992
读到的值 = 1569961152
```

同一个可执行文件，三次三个值。换成 `--release` 编译，读到的是 `0`。这就是 UB 最典型的外观：**不是崩溃，而是没有承诺**。释放后的内存可能被分配器复用、可能被归还给操作系统、可能还留着旧值，读到什么取决于当时堆的状态。

**越界读。** 只读不写，看起来「无害」：

```rust,noplayground
fn main() {
    let v = vec![1u8, 2, 3, 4];
    let x = unsafe { *v.as_ptr().add(8) };
    println!("越界读到 = {}", x);
}
```

本机 debug 构建输出 `越界读到 = 80`，release 构建输出 `0`。`add(8)` 已经超出了 4 字节的分配范围，指针算术本身就违规；即便读到的地址恰好可读，也不代表这次访问合法。

**数据竞争。** 两个线程不加同步地写同一个 `static mut`：

```rust,noplayground
static mut TOTAL: u64 = 0;

fn main() {
    let mut handles = Vec::new();
    for _ in 0..2 {
        handles.push(std::thread::spawn(|| unsafe {
            for _ in 0..100_000 {
                TOTAL += 1;
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    println!("TOTAL = {}", unsafe { TOTAL });
}
```

本机 debug 构建连跑三次：`121288`、`160223`、`120359`。两个线程各加十万次，结果却远小于二十万，因为 `+=` 是「读—加—写」三步，中间被另一个线程插了队。release 构建下这三次都输出 `200000`——但**结果正确不等于程序正确**：数据竞争的定义是「两个线程并发访问同一位置且至少一个是写、又没有同步」，它是否表现出错乱取决于调度，而不是取决于结果好不好看。

### 为什么 UB 不是「未定义但实际可用」

关键在于优化的方向。编译器做优化时，前提是「程序里不会发生 UB」。它不需要证明这个前提，而是**假设**它成立，然后据此删改代码。一旦你真的触发了 UB，之前所有的推理都失去依据，编译器可以做任何事——包括你认为绝不可能发生的事。

最具体的机制是 `noalias`。Rust 的 `&mut T` 参数在 LLVM 中间表示里会被标上 `noalias`，含义是「这块内存在这个引用的生命周期内不会被其它指针访问」。优化器据此可以：

- 把「先通过 `a` 写，再通过 `b` 读」重排成「先读 `b`，再写 `a`」；
- 认为两次读之间没有写入，于是把第二次读消掉，直接复用第一次的值；
- 认为某个分支不可达，把它整段删除。

当你用裸指针造出两个指向同一对象的 `&mut` 时，这些变换全部失去合法性，但编译器不知道，它仍然照做。于是可能出现的现象是：你明明写了 `*a = 1`，随后读 `*a` 却得到旧值；或者某个循环被优化成死循环。这类问题不会在 debug 构建里出现，只在 `--release` 且某个优化 pass 恰好生效时出现，换个编译器版本又消失。

另一类典型是「无效值」。Rust 规定 `bool` 只能是 0 或 1，`char` 必须是合法 Unicode 标量，引用必须非空且对齐。这些约束不是建议，是类型的一部分。用 `transmute` 把 `2u8` 变成 `bool`，或者把未初始化内存读成 `bool`，即使那个字节"看起来像真的"，也是 UB；优化器可以把 `if b { A } else { B }` 直接编译成两段代码而不做比较。

判断一段 unsafe 代码是否可靠的依据，不是「我跑过，没问题」，而是「我能列出它的全部前提，并且每个前提都能被代码或类型系统保证」。前者是观察，后者是证明。

### MaybeUninit 与未初始化内存

Rust 不允许读未初始化的变量，但真实场景里经常需要「先划一块地，再慢慢填」：读文件到缓冲区、给数组批量赋值、实现 `Vec` 的扩容。标准库给的答案是 `std::mem::MaybeUninit<T>`。

它的大小和对齐与 `T` 完全相同，语义是「这里有一块能放下 T 的内存，但里面可能是任意字节」。你可以安全地创建它、把它传给函数、逐字段写入；只有在**读**的时候才需要 `assume_init()`，而那是 unsafe 的，因为编译器无法确认你真的填满了。

```rust
use std::mem::MaybeUninit;

fn main() {
    let mut buf: [MaybeUninit<u8>; 4] = [MaybeUninit::uninit(); 4];

    for (i, slot) in buf.iter_mut().enumerate() {
        slot.write(i as u8 * 10);
    }

    let ready = buf.map(|slot| unsafe { slot.assume_init() });
    println!("{:?}", ready);
}
```

这段代码是安全的：每个槽位都在 `assume_init` 之前写过一次，且 `u8` 没有无效值，任何字节都是合法的 `u8`。两个条件缺一不可。

反例只需要一行：

```rust,noplayground
let b: bool = unsafe { MaybeUninit::uninit().assume_init() };
```

`bool` 有无效值（2 到 255 都不合法），未初始化内存里恰好是什么字节完全随机，所以这一行是 UB。同样的写法如果换成 `u8`，就只是「读到垃圾值」，不构成 UB——这正是「有效值」和「未初始化」两个概念的区别。

对数组这类批量初始化，标准库还提供了 `MaybeUninit::zeroed()`，它保证全零，对整数、浮点、数组都合法，但不能用于 `&T`、`Box<T>` 这类零不是有效值的类型。

## 典型错误案例

### 案例 1：在 unsafe 块外解引用裸指针——E0133

```rust,editable
fn main() {
    let value = 42;
    let p = &value as *const i32;
    println!("{}", *p);
}
```

```text
error[E0133]: dereference of raw pointer is unsafe and requires unsafe function or block
 --> src/main.rs:4:20
  |
4 |     println!("{}", *p);
  |                    ^^ dereference of raw pointer
  |
  = note: raw pointers may be null, dangling or unaligned; they can violate aliasing rules and cause data races: all of these are undefined behavior

error: aborting due to 1 previous error
```

**原因**：解引用裸指针是那五个能力之一，编译器无法验证它指向的内存是否有效、是否对齐、是否已经被释放。注意报错只落在 `*p` 上：上一行 `&value as *const i32` 是安全的，创建指针不需要 unsafe。很多人看到 E0133 就把整个函数包进 unsafe 块，那只是把警告压下去，没有解决任何问题。

**修复**：把解引用圈进 unsafe，并在上方写清这次为什么安全。范围要尽可能小：

```rust,editable
fn main() {
    let value = 42;
    let p = &value as *const i32;
    // SAFETY: p 指向栈上的 value，value 在读取时仍然存活且已初始化。
    let v = unsafe { *p };
    println!("{}", v);
}
```

### 案例 2：把 String 直接传给 C 函数——E0308

```rust,editable
extern "C" {
    fn strlen(s: *const i8) -> usize;
}

fn main() {
    let name = String::from("rust");
    let len = unsafe { strlen(name) };
    println!("len = {}", len);
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:7:31
  |
7 |     let len = unsafe { strlen(name) };
  |                        ------ ^^^^ expected `*const i8`, found `String`
  |                        |
  |                        arguments to this function are incorrect
  |
  = note: expected raw pointer `*const i8`
                  found struct `String`
note: function defined here
 --> src/main.rs:2:8
  |
2 |     fn strlen(s: *const i8) -> usize;
  |        ^^^^^^ -
```

**原因**：`String` 是「指针 + 长度 + 容量」的三字段结构体，C 要的是一个指向以 0 结尾字节序列的裸指针。两者的内存表示完全不同，编译器不会替你转换——这是好事，因为自动转换会悄悄丢掉长度或忘记补 0。顺带一提，这里写 `i8` 也不严谨，应该用 `std::os::raw::c_char`，它在不同平台上可能是 `u8`。

**修复**：用 `CString` 造一块以 0 结尾的缓冲，再用 `as_ptr()` 取出指针。`CString` 必须在调用期间存活：

```rust,editable
use std::ffi::CString;
use std::os::raw::c_char;

extern "C" {
    fn strlen(s: *const c_char) -> usize;
}

fn main() {
    let name = CString::new("rust").unwrap();
    // SAFETY: name 在调用期间存活，其内部缓冲以 0 结尾。
    let len = unsafe { strlen(name.as_ptr()) };
    println!("len = {}", len);
}
```

### 案例 3：把裸指针送进线程——E0277

```rust,editable
fn main() {
    let value = 7;
    let p: *const i32 = &value;
    std::thread::spawn(move || unsafe {
        println!("{}", *p);
    }).join().unwrap();
}
```

```text
error[E0277]: `*const i32` cannot be sent between threads safely
   --> src/main.rs:4:24
    |
  4 |       std::thread::spawn(move || unsafe {
    |       ------------------ ^------
    |       |                  |
    |  _____|__________________within this `{closure@src/main.rs:4:24: 4:31}`
    | |     |
    | |     required by a bound introduced by this call
  5 | |         println!("{}", *p);
  6 | |     }).join().unwrap();
    | |_____^ `*const i32` cannot be sent between threads safely
    |
    = help: within `{closure@src/main.rs:4:24: 4:31}`, the trait `Send` is not implemented for `*const i32`
note: required because `p` is used within this closure
   --> src/main.rs:5:24
    |
  5 |         println!("{}", *p);
    |                        ^^
```

**原因**：`Send` 表示「这个值可以安全地转移到另一个线程」。裸指针没有实现 `Send`，因为它不携带任何所有权或同步信息：编译器不知道这个地址会不会在另一个线程里被同时写，也不知道它指向的对象还活着没有。`&i32` 能跨线程是因为 `i32: Sync` 且借用检查器能证明它活着；裸指针把这两条保证都丢了。

**修复**：优先换成共享所有权加锁的类型，让编译器重新拿到保证：

```rust,editable
use std::sync::{Arc, Mutex};

fn main() {
    let value = Arc::new(Mutex::new(7));
    let cloned = Arc::clone(&value);
    std::thread::spawn(move || {
        *cloned.lock().unwrap() += 1;
    }).join().unwrap();
    println!("{}", value.lock().unwrap());
}
```

如果确实必须传裸指针（比如把地址交给一个自己保证线程安全的 C 库），只能定义一个 newtype 并手写 `unsafe impl Send`。那一刻起，「这个指针跨线程安全」的责任就正式从编译器转移到了你身上。

### 案例 4：transmute 两侧大小不一致——E0512

```rust,editable
fn main() {
    let x: u32 = 7;
    let bytes: [u8; 8] = unsafe { std::mem::transmute(x) };
    println!("{:?}", bytes);
}
```

```text
error[E0512]: cannot transmute between types of different sizes, or dependently-sized types
 --> src/main.rs:3:35
  |
3 |     let bytes: [u8; 8] = unsafe { std::mem::transmute(x) };
  |                                   ^^^^^^^^^^^^^^^^^^^
  |
  = note: source type: `u32` (32 bits)
  = note: target type: `[u8; 8]` (64 bits)
```

**原因**：`transmute` 不做任何数值转换，它只是「把同一段比特重新解释成另一个类型」，所以要求源和目标的字节数完全相同。`u32` 是 4 字节，`[u8; 8]` 是 8 字节，编译器在编译期就能发现不匹配，于是用 E0512 拦下来——这比运行期读到半截数据好得多。

**修复**：想拆字节就用专门的转换函数，它们不仅大小对，还把端序写进了名字：

```rust,editable
fn main() {
    let x: u32 = 7;
    let bytes: [u8; 4] = x.to_le_bytes();
    println!("{:?}", bytes);
}
```

### 案例 5：释放后继续读指针——没有编译错误，但它是 UB

这个案例特别之处在于：**它编译通过，也没有任何警告**。错误要等运行期才出现，而且每次出现的值都可能不同。

```rust,editable
fn main() {
    let p = Box::into_raw(Box::new(42i32));
    unsafe { drop(Box::from_raw(p)) };
    let v = unsafe { *p };
    println!("读到的值 = {}", v);
}
```

```text
# debug 构建，连跑三次
读到的值 = 54275728
读到的值 = 722252992
读到的值 = 1569961152

# --release 构建
读到的值 = 0
```

**原因**：`Box::from_raw(p)` 把所有权收回并 drop，堆内存被分配器回收，`p` 从那一刻起就是悬垂指针。解引用它的前提「指针指向一块存活的分配」不再成立。编译器检查不了这一点，因为要判断必须跨语句跟踪所有权的运行时流向——这恰恰是安全 Rust 用所有权模型替你避免的东西，而裸指针把这个模型绕过去了。

注意三次运行得到三个不同的值，release 下又是另一个值。这就是 UB 的典型表现：它不是「保证读到 42」，也不是「保证崩溃」，而是「行为没有承诺」。

**修复**：读要在释放之前，并且保证同一个指针只 `from_raw` 一次：

```rust,editable
fn main() {
    let p = Box::into_raw(Box::new(42i32));
    // SAFETY: p 来自 Box::into_raw，此刻仍然有效且已初始化。
    let v = unsafe { *p };
    println!("读到的值 = {}", v);
    // SAFETY: 同一个指针只 from_raw 一次，且此后不再使用 p。
    unsafe { drop(Box::from_raw(p)) };
}
```

## 动手练习

**练习 1**：把 35.2 里用指针遍历数组的代码改写成函数 `unsafe fn sum_raw(ptr: *const i32, len: usize) -> i32`，写全 `# Safety` 文档，并在调用点写 SAFETY 注释。然后用安全的切片版本 `fn sum_slice(values: &[i32]) -> i32` 实现同样的功能，对比两者：安全版本的边界检查在什么位置，代价是多少。

**练习 2**：为 35.4 的 `split_at_mut` 写三个测试：`mid = 0`、`mid = len`、`mid > len`（用 `#[should_panic]`）。然后回答：如果把开头的 `assert!` 删掉，`mid = len + 1` 时 `from_raw_parts_mut(ptr, mid)` 会造出一个什么样的切片，接下来哪一步会触发 UB。

**练习 3**：实现一个安全函数 `fn nth<T>(values: &[T], index: usize) -> Option<&T>`，要求内部先做边界检查、再用 `get_unchecked` 取元素。写完后解释：这个包装有没有消除边界检查？它把检查移动到了哪里？为什么对调用方来说它仍然是 100% 安全的？

**练习 4**：给 35.5 的 `Counter` 补一个 `counter_get(handle: *mut Counter) -> i64`，并为三个函数写一份 C 风格的头文件。再写一个测试验证「new 一次、add 两次、get 一次、free 一次」的完整流程；然后在代码里故意漏掉 `counter_free`，说明泄漏发生在哪一段、为什么 `Box` 的所有权机制在这里帮不上忙。

**练习 5**：判断下面哪些是未定义行为，哪些是定义良好的行为，逐条写出理由：

1. 解引用空指针。
2. 用 `transmute` 把 `2u8` 变成 `bool`。
3. 用 `get_unchecked` 读取越界元素。
4. 两个线程并发写同一个 `static mut`。
5. 计算 `i32::MAX + 1`（debug 下 panic，release 下环绕）。
6. 对未初始化的 `MaybeUninit<u8>` 调用 `assume_init`。

提示：判断标准是「Rust 的抽象机器对这次操作有没有给出确定语义」，而不是「我这台机器上跑出来是什么结果」。

## 小结

- unsafe 只打开五个能力：解引用裸指针、调用 unsafe 函数、访问修改 `static mut`、实现 unsafe trait、访问 union 字段，别的检查照旧
- 借用检查、类型检查、初始化与移动检查在 unsafe 块里完全生效；遇到类型或借用错误仍需修正代码
- 创建裸指针是安全的，解引用、指针算术、转成引用才是 unsafe；`add` 以元素为单位，跨分配对象的算术本身就违规
- unsafe fn 的 `# Safety` 文档是签名的一部分；每个 unsafe 块上方都要有一行 SAFETY 注释，写清不变量、谁保证、违反后果
- `debug_assert!` 只在 debug 构建检查程序员错误，来自外部的输入要用 `assert!` 或返回 `Result`
- 安全抽象 = 内部 unsafe 加对外安全签名，`split_at_mut` 靠四条不变量成立，其中「生命周期绑回输入」是类型系统保证的
- extern "C" 配 `#[no_mangle]` 负责导出，`extern` 块负责导入；`CStr`/`CString` 管 C 字符串，`Box::into_raw`/`from_raw` 管所有权移交
- FFI 三条铁律：谁分配谁释放、panic 不能穿过边界、ABI 与类型大小必须匹配，尤其是 `#[repr(C)]` 和 `bool` 这类有无效值的类型
- Miri 能把执行到的路径上的 UB 变成报错，`#[repr(C)]` 固定布局，bindgen 与 cbindgen 生成两个方向的绑定，`-sys` crate 加安全包装是标准分层
- UB 不是「未定义但实际可用」：优化器基于「不会发生 UB」做推理，`noalias` 等假设会让错误代码得到任意结果；正确性的依据是可列出的前提，不是「我跑过没问题」