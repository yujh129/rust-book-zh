# 第 31 章 Web 服务：axum 实战

> **本章目标**
>
> - 说清一次 HTTP 请求在 axum 里经过的四层：Hyper 管字节、Tower Service 管组合、Router 管匹配、handler 管业务
> - 能用 Router、axum::serve 和 TcpListener 起一个服务，处理路径参数与 JSON 请求体
> - 掌握 Path、Query、Json、State、HeaderMap 五类提取器的取值规则，知道消费 body 的提取器为什么必须放最后
> - 能为自定义错误实现 IntoResponse，让 handler 用 Result<T, AppError> 统一表达 4xx 与 5xx
> - 会用 ServiceBuilder 组合 TraceLayer、超时与 CORS，并说清层的包裹顺序
> - 能用 tower::ServiceExt::oneshot 在不开端口的前提下测试路由

写一个能跑的 Web 服务，在 Rust 里曾经是件体力活：手动解析请求行、自己拼响应、自己处理 chunked 编码。
axum 把这件事拆成了四层，每一层只做一件事，而且每一层都对外开放。你既可以只写业务 handler，也可以在任意两层之间插入自己的逻辑。

本章用一个「用户服务」把这些层依次拆开：先看请求怎么进来，再写第一个能响应 curl 的服务，然后逐个介绍提取器、响应类型、错误处理与中间件。
最后一节会停下来回答几个「为什么」：handler 为什么必须 Send，为什么不该用 Mutex 包住 Router，测试为什么不需要真的监听端口。

本章代码基于 axum 0.8（写作时为 0.8.9）与 tokio 1.x。0.8 把路径参数的写法从 `:id` 改成了 `{id}`，旧写法会在启动时 panic，迁移时先改这一处。

## 31.1 一次请求经过哪四层

先看全貌。一个请求从网卡到你的函数，依次经过：

```text
TcpListener          接受 TCP 连接
  -> hyper           解析 HTTP/1.1 或 HTTP/2，产出 http::Request<Body>
    -> Tower Service 路由、中间件、状态，组合成一个 Service
      -> Router      按方法与路径匹配
        -> handler   你的 async fn
          -> Response 反向穿回上面每一层
```

### Hyper：只负责字节与协议

axum 不解析 HTTP。它把这件事交给 hyper：hyper 从 TcpStream 里读出字节，按协议切出请求行、请求头、body 流，拼成标准的 `http::Request<Body>`。
body 是流式的，不会一次性读进内存——这也是后面「body 只能消费一次」的根源。

hyper 还负责把 `http::Response<Body>` 写回连接、管理 keep-alive、HTTP/2 的多路复用。这些都不需要你操心，除非你要自己实现协议层。

### Tower Service：一切皆服务

Tower 定义了一个极小的抽象：能接收请求、返回 future 的东西就是服务。

```rust,noplayground
pub trait Service<Request> {
    type Response;
    type Error;
    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn call(&mut self, req: Request) -> Self::Future;
}
```

`poll_ready` 用来表达背压：服务没准备好时返回 `Poll::Pending`，调用方就该等一等。
`call` 只负责「开始处理」，真正的计算在返回的 future 里进行。
中间件就是「包住另一个 Service 的 Service」：它在 `call` 里做点事，再把请求转交给内层。

真实的 Tower 中间件正是这么写的：它不知道内层是什么，只知道内层能处理请求。所以日志、超时、限流可以叠在任意服务上。

### Router：按方法与路径匹配

`Router` 是一个实现了 `Service<Request<Body>>` 的具体类型。它保存三样东西：路由表、状态、以及叠在上面的层。
请求进来后，它先看路径匹配哪条路由，再看方法是否被允许：路径没命中返回 404，路径命中但方法不对返回 405。
这两类响应在进入你的 handler 之前就已经决定，所以你在 handler 里断点调试是看不到它们的。

### handler：只写业务

handler 是一个普通的 async 函数，但有两个额外要求：

- 每个参数都要实现 `FromRequestParts` 或 `FromRequest`，axum 据此把请求拆成参数；
- 返回值要实现 `IntoResponse`，axum 据此把它变成响应。

axum 为「参数类型组合 + 返回类型」实现了 `Handler` trait，`get(handler)`、`post(handler)` 接受的正是这个东西。
这个 trait 的约束很宽，宽到编译器报错时只会说「Handler 没有实现」，把真正的原因藏在下一层——本章 31.8 的第一个案例就是这种报错。

知道这四层之后，调试就有了顺序：404/405 先看路由表，415/400 先看提取器，500 先看 handler，日志与耗时先看层。

## 31.2 第一个服务

新建一个二进制 crate，依赖如下：

```toml
[package]
name = "axum-demo"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

axum 的默认 feature 支持 Tokio，但应用要直接使用 `tokio::net` 和 `#[tokio::main]`，仍应把 Tokio 写进自己的依赖。这里的 `full` 包含 `macros`、`rt-multi-thread` 和网络等 feature。

```rust,noplayground
use axum::{
    extract::Path,
    routing::get,
    Json, Router,
};
use serde::Serialize;

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn health() -> &'static str {
    "ok"
}

async fn user(Path(id): Path<u64>) -> Json<User> {
    Json(User { id, name: format!("user-{id}") })
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/health", get(health))
        .route("/users/{id}", get(user));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("listening on http://{}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

跑起来：

```text
$ cargo run
listening on http://127.0.0.1:3000
```

另开一个终端，用 curl 请求：

```text
$ curl -i http://127.0.0.1:3000/health
HTTP/1.1 200 OK
content-type: text/plain; charset=utf-8
content-length: 2

ok
$ curl http://127.0.0.1:3000/users/7
{"id":7,"name":"user-7"}
$ curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/users/abc
400
```

`/users/abc` 这一行值得注意：`abc` 不是 `u64`，`Path` 提取失败，axum 直接返回 400，你的 `user` 函数根本没被调用。路由没命中时则是 404 空 body。这两类响应都不是你写的。

逐行拆开看：

- `TcpListener::bind` 只创建监听套接字，不启动任何东西；端口被占用时这里就返回 `Err`，所以 `unwrap` 是合理的——起不来就该立刻退出。
- `axum::serve(listener, app)` 接管监听与连接循环，返回一个 future。它会一直运行，直到遇到致命错误，或者你主动触发优雅关闭。
- 绑定 `127.0.0.1` 只有本机能访问；要让容器外或局域网访问，得绑 `0.0.0.0`。生产环境通常由反向代理转发到本机端口，所以绑回环地址反而更常见。
- `get(health)` 里的 `health` 没有参数、返回 `&'static str`，于是响应是 200 + `text/plain`。
- 路径参数用 `{id}`。在 0.8 里写 `:id` 不会编译错，而是在启动构建路由时 panic，提示你改用花括号。

`Router::new()` 返回的 `Router<()>` 表示「状态已经提供完毕」。带状态的路由是 `Router<AppState>`，只有调用 `with_state` 之后才能交给 `axum::serve`。这一点在下一节会反复出现。

## 31.3 提取器：从请求里取值

handler 的参数不是随便写的，每个类型都要实现提取器 trait。它们分成两类，这条分界线决定了参数的顺序：

| 提取器 | 取什么 | 类别 |
| --- | --- | --- |
| Path | 路径里的占位符 | 只读，可多个 |
| Query | 查询字符串 | 只读，可多个 |
| HeaderMap | 全部请求头 | 只读，可多个 |
| State | 应用状态 | 只读，可多个 |
| Json | 请求体，反序列化 | 消费 body，只能一个且在最后 |
| String | 请求体原文 | 消费 body，只能一个且在最后 |

原因很直接：请求体是一条只能读一次的流。只读的提取器借一下 `Parts` 就够了，谁先谁后无所谓；消费 body 的提取器必须把整个请求拿走，所以只能有一个，且只能排在最后。

下面这个服务把五类提取器都用上了：

```rust,noplayground
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct AppState {
    users: Arc<Mutex<Vec<User>>>,
}

#[derive(Clone, Serialize)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct Page {
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct NewUser {
    name: String,
}

async fn list(State(state): State<AppState>, Query(page): Query<Page>) -> Json<Vec<User>> {
    let users = state.users.lock().unwrap();
    let limit = page.limit.unwrap_or(10);
    Json(users.iter().take(limit).cloned().collect())
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<User>, StatusCode> {
    let users = state.users.lock().unwrap();
    users
        .iter()
        .find(|u| u.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create(
    State(state): State<AppState>,
    Json(new): Json<NewUser>,
) -> (StatusCode, Json<User>) {
    let mut users = state.users.lock().unwrap();
    let user = User { id: users.len() as u64 + 1, name: new.name };
    users.push(user.clone());
    (StatusCode::CREATED, Json(user))
}

fn app() -> Router {
    let state = AppState { users: Arc::new(Mutex::new(Vec::new())) };
    Router::new()
        .route("/users", get(list).post(create))
        .route("/users/{id}", get(get_one))
        .with_state(state)
}
```

### Path 与 Query

`Path<u64>` 声明「这个位置必须能解析成 u64」。解析失败时 axum 返回 400，handler 不会执行。
路径里有多个占位符时，用元组按声明顺序接收：`Path((id, name)): Path<(u64, String)>`。

`Query<Page>` 把查询串按表单规则解析成结构体。字段是 `Option<T>` 就可以缺省；不是 `Option` 又没传，就是 400。
`?limit=abc` 同样是 400，因为 `usize` 解析失败。查询串里的重复键、数组参数需要 `serde` 的辅助类型，本章不展开。

### State

`State<T>` 的 T 必须实现 `Clone`。这不是 axum 偷懒：每次提取都要从 `Router` 里拿出一份状态交给 handler，它拿的方式就是克隆。
所以 `AppState` 要么本身很便宜（几个整数），要么内部用 `Arc` 共享（本章的写法）。

状态用 `with_state` 注入。注意类型要精确对上：handler 写 `State<AppState>`，`with_state` 就传 `AppState`；handler 写 `State<Arc<AppState>>`，就传 `Arc::new(AppState { .. })`。
两者混用会得到 31.8 案例 3 里的 `E0308`。

### Json 与 body

`Json<T>` 只在 `Content-Type: application/json` 时才反序列化。三种失败要分清：

- 头不对（比如 `text/plain`）：415 Unsupported Media Type；
- JSON 语法错：400 Bad Request；
- 语法对但形状不对（缺字段、类型不符）：422 Unprocessable Entity。

axum 默认还给请求体设了 2 MiB 上限，超过就返回 413，这个上限可以用 `DefaultBodyLimit` 调整。

### HeaderMap

`HeaderMap` 是只读提取器，放在哪个位置都行，通常用来取 `Authorization`、`User-Agent` 这类信息：

```rust,noplayground
use axum::http::HeaderMap;

async fn whoami(headers: HeaderMap) -> String {
    headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
```

注意 `to_str` 可能失败（头里允许非 ASCII 字节），所以不能直接 `unwrap`。

## 31.4 IntoResponse：返回值怎样变成响应

handler 的返回值不是「数据」，而是「知道怎么把自己变成响应的东西」。这个 trait 叫 `IntoResponse`，标准库里没有，是 axum 定义的。

常见的返回类型与它们的默认行为：

- `&'static str` 与 `String`：200，`text/plain; charset=utf-8`；
- `StatusCode`：该状态码，空 body；
- `Json<T>`：200，`application/json`，body 是序列化结果；
- `(StatusCode, T)`：状态码加内层响应；
- `([(HeaderName, &str); N], T)`：附加响应头再加内层响应；
- `Result<T, E>`：T 与 E 都实现 `IntoResponse` 时，按 Ok/Err 分别变成响应；
- `Response`：完全手写，用于流式 body 或特殊头。

```rust,noplayground
use axum::{
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

async fn text() -> &'static str {
    "hello"
}

async fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn created() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CREATED, Json(json!({ "ok": true })))
}

```

元组的顺序是「先状态/头，后 body」，因为最后一个元素才是真正的响应体。写反了编译器会告诉你类型对不上。

### 自定义响应类型

当一组响应头反复出现时，可以包一个 newtype，把 `IntoResponse` 实现一次：

```rust,noplayground
use axum::{
    http::header,
    response::{IntoResponse, Response},
};

struct Html(String);

impl IntoResponse for Html {
    fn into_response(self) -> Response {
        (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            self.0,
        )
            .into_response()
    }
}
```

内层那个 `(...).into_response()` 复用了标准实现，所以不需要手动构造 `Response::builder()`。
如果确实需要完全控制（比如设置 HTTP 版本、返回流式 body），再直接返回 `Response`。

`impl IntoResponse` 也可以写在返回位置，用来隐藏具体类型。但它在 trait 方法里不能当返回类型用——那时必须写出具体类型或装箱。

## 31.5 用一个错误类型统一 4xx 与 5xx

上一节的 `get_one` 用 `StatusCode` 当错误类型，能跑，但信息量只有一个数字。真实服务里错误来源很多：找不到、参数非法、磁盘失败。如果每个 handler 各写各的状态码，客户端拿到的格式就会五花八门。

惯常的做法是定义一个错误枚举，为它实现 `IntoResponse`，然后所有 handler 统一返回 `Result<T, AppError>`：

```rust,noplayground
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
enum AppError {
    #[error("user {0} not found")]
    NotFound(u64),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("storage failed")]
    Storage(#[from] std::io::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Invalid(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Storage(_) => {
                tracing::error!(error = ?self, "storage error");
                (StatusCode::INTERNAL_SERVER_ERROR, String::from("internal error"))
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

async fn get_one(id: u64) -> Result<String, AppError> {
    if id == 0 {
        return Err(AppError::NotFound(id));
    }
    Ok(format!("user-{id}"))
}

async fn create(name: String) -> Result<&'static str, AppError> {
    if name.trim().is_empty() {
        return Err(AppError::Invalid(String::from("name is empty")));
    }
    Ok("created")
}

fn app() -> Router {
    Router::new()
        .route("/users/0", get(|| get_one(0)))
        .route("/users", post(create))
}
```

几个设计点：

- `#[from]` 让 `?` 自动把 `std::io::Error` 转成 `AppError::Storage`，handler 里不用写 `map_err`。
- 4xx 的 message 可以直接给客户端，因为它描述的是「你请求错了」；5xx 只回一句 `internal error`，细节通过 `tracing::error!` 进日志。
- 所有错误响应都是同一个 JSON 形状 `{"error": "..."}`，客户端只需要写一个解析器。
- 如果忘了 `impl IntoResponse for AppError`，handler 本身没问题，但 `get(fail)` 会报「Handler 未实现」——原因被 axum 的泛型约束藏起来了，31.8 案例 4 会看到原文。

这套写法与第 27 章的 API 错误设计是同一件事，只是把「库的调用者」换成了「HTTP 客户端」：错误类型是接口的一部分，状态码是它的投影。

## 31.6 中间件：TraceLayer、超时与 CORS

Tower 的 `Layer` 负责「把一个服务变成另一个服务」。`ServiceBuilder` 把多个 Layer 串起来：

```rust,noplayground
use std::time::Duration;

use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode},
    routing::get,
    Router,
};
use tower::{ServiceBuilder, ServiceExt};
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer};

fn app() -> Router {
    Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                "done"
            }),
        )
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    Duration::from_secs(2),
                ))
                .layer(
                    CorsLayer::new()
                        .allow_origin("https://example.com".parse::<HeaderValue>().unwrap())
                        .allow_methods([Method::GET, Method::POST]),
                ),
        )
}

#[tokio::main]
async fn main() {
    let request = Request::builder().uri("/slow").body(Body::empty()).unwrap();
    let response = app().oneshot(request).await.unwrap();
    println!("GET /slow -> {}", response.status());
}
```

输出是：

```text
GET /slow -> 408 Request Timeout
```

handler 明明要睡 10 秒，2 秒后客户端就收到了 408，说明超时层没有等 handler 自己结束，而是直接生成了一个响应。

顺序很重要：`ServiceBuilder` 里先 `layer` 的在最外层。上面的顺序是 Trace 包住 Timeout，Timeout 包住 CORS，CORS 包住 Router。
请求从上往下穿，响应从下往上穿。Trace 在最外层，才能稳定看到每一个响应，包括超时层生成的 408；如果把它放到内层，超时发生时它可能等不到内层返回。

几个容易踩的点：

- `tower-http` 0.7 的 `TimeoutLayer` 不改变服务的错误类型，超时直接变成 408 响应，所以不需要 `HandleErrorLayer`。`TimeoutLayer::new` 已废弃，用 `with_status_code` 指定状态码。
- 如果改用 `tower::timeout::TimeoutLayer`，超时是 `Err(Elapsed)`。这时必须再套一个 `HandleErrorLayer` 把错误翻译成响应，否则 `Router` 的类型对不上。
- `CorsLayer` 会自己处理浏览器的预检 `OPTIONS` 请求，不会进你的路由。`allow_origin` 只写一个来源时不能用 `Any` 混搭凭证；需要带 cookie 时，来源必须逐个列出。
- `TraceLayer` 只产生 span 与事件，要把它们打印出来还得初始化订阅者（第 28 章）。没装订阅者时它不会报错，只是什么都看不见。

## 31.7 深入：Send、提取器与测试

这一节回答五个在写业务时迟早会撞上的问题：handler 的 future 为什么必须 Send，两种提取器 trait 差在哪，为什么不能用 Mutex 包 Router，优雅关闭到底关了什么，以及不监听端口怎么测路由。

### 为什么 handler 的 future 必须是 Send + 'static

axum 在每个连接上 spawn 一个 task。tokio 的多线程运行时会把 task 在 worker 线程之间迁移，所以 task 里的 future 必须能跨线程移动，也就是 `Send`。
`'static` 则禁止 future 借用栈上的局部变量：handler 返回后，future 可能还在别的线程上被 poll，栈早就没了。

最常见的违规是让 `std::sync::MutexGuard` 跨过 `.await`：guard 内部是指向锁的引用，释放时要在原线程解锁，所以它不能跨线程；future 在 await 点挂起时会把局部变量一起存进状态机，于是整个 future 变成 `!Send`。修复办法是把锁的作用域缩到 await 之前，只把需要的数据复制出来；确实要跨 await 持锁就换 `tokio::sync::Mutex`，它的 guard 是 `Send`。这个例子的完整代码与逐字报错见 31.8 案例 4。

### FromRequestParts 与 FromRequest

两个 trait 的分工只有一句话：前者只借请求的「零件」，后者要拿走整个请求。

```rust,noplayground
pub trait FromRequestParts<S>: Sized {
    type Rejection: IntoResponse;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection>;
}

pub trait FromRequest<S>: Sized {
    type Rejection: IntoResponse;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection>;
}
```

`Parts` 里有方法、URI、头、扩展，唯独没有 body。所以 `Path`、`Query`、`HeaderMap`、`State` 都实现前者，可以随便排；`Json`、`String`、`Bytes` 实现后者，只能有一个且必须最后——body 是一条流，读完就没了。

自己写提取器时，`Rejection` 也要实现 `IntoResponse`，这样失败时才能直接变成 4xx：

```rust,noplayground
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

struct ApiKey(String);

impl<S> FromRequestParts<S> for ApiKey
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|v| ApiKey(v.to_string()))
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
```

把它放进 handler 参数表，缺少 `x-api-key` 时 axum 返回 401，handler 不会执行。

### 为什么不要用 Mutex 包 Router

有人想「共享路由」，写出 `Arc<Mutex<Router>>`。这条路在类型层面就不通：`axum::serve` 要求第二个参数实现 `Service`，而 `Mutex` 没有实现 `Service`，编译器会直接拒绝。

即使手动加锁再调用，也有三个问题：`Router` 本身已经实现 `Clone`，克隆只增加内部引用计数，路由表是共享的；全局锁把所有请求串行化，吞吐掉到一个核；锁 guard 一旦跨 `await`，future 立刻变成 `!Send`。

正确的分工是：Router 保持可克隆，共享的东西放进 `AppState`，用 `with_state` 注入。

### with_graceful_shutdown 与 ctrl_c

`axum::serve` 返回的 future 默认永远不结束。要让它体面地停下，用 `with_graceful_shutdown` 给它一个信号 future：

```rust,noplayground
async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}
```

收到 Ctrl+C 后，serve 先停止接受新连接，再等待已经在处理的请求结束，最后返回。容器环境通常发 SIGTERM 而不是 SIGINT，Unix 上要再监听 `tokio::signal::unix::signal(SignalKind::terminate())`，用 `tokio::select!` 合并两个信号。

### 用 oneshot 做路由单测

单元测试不需要真的 bind 端口。`tower::ServiceExt::oneshot` 把一个 Service 变成「只调用一次」的 future：

```rust,noplayground
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use tower::ServiceExt;

fn app() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

#[tokio::test]
async fn health_returns_200() {
    let response = app()
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

测试通过后输出 `test health_returns_200 ... ok`。这种测法没有端口冲突，不依赖网络，跑得比真起服务快一个数量级。
`oneshot` 会消费掉服务，所以每个测试都重新 `app()`；要断言 body，用 `axum::body::to_bytes` 把 `Body` 读成字节。

## 31.8 典型错误案例

下面五段代码都在本机用 rustc 1.100.0 与 axum 0.8.9 编译过，报错只删掉了与本题无关的仓库路径。

### 案例 1：消费 body 的提取器没放最后——E0277

`Json` 会拿走整个请求，却被排在了 `Path` 前面：

```rust,noplayground
use axum::{extract::Path, routing::post, Json, Router};
use serde::Deserialize;

#[derive(Deserialize)]
struct Payload {
    name: String,
}

async fn update(body: Json<Payload>, path: Path<u32>) -> String {
    format!("{} {}", path.0, body.name)
}

fn main() {
    let _app: Router = Router::new().route("/items/{id}", post(update));
}
```

```text
error[E0277]: the trait bound `fn(Json<Payload>, Path<u32>) -> _ {update}: Handler<_, _>` is not satisfied
   --> src/main.rs:14:64
    |
 14 |     let _app: Router = Router::new().route("/items/{id}", post(update));
    |                                                           ---- ^^^^^^ unsatisfied trait bound
    |                                                           |
    |                                                           required by a bound introduced by this call
    |
    = help: the trait `Handler<_, _>` is not implemented for fn item `fn(Json<Payload>, axum::extract::Path<u32>) -> impl Future<Output = String> {update}`
    = note: Consider using `#[axum::debug_handler]` to improve the error message
```

**原因**：`Json` 实现的是 `FromRequest`，它把整个请求拿走；`Path` 实现的是 `FromRequestParts`，需要在 body 被消费前借用 `Parts`。顺序反过来，axum 就没有为这个参数组合实现 `Handler`。

**修复**：把消费 body 的提取器放到最后，其余随意调换：

```rust,noplayground
async fn update(path: Path<u32>, body: Json<Payload>) -> String {
    format!("{} {}", path.0, body.name)
}
```

### 案例 2：状态类型忘了 Clone——E0277

`State<AppState>` 要求 `AppState: Clone`，因为每次提取都要克隆一份交给 handler：

```rust,noplayground
use axum::{extract::State, routing::get, Router};

struct AppState {
    hits: u64,
}

async fn count(State(state): State<AppState>) -> String {
    state.hits.to_string()
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/count", get(count))
        .with_state(AppState { hits: 0 });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

```text
error[E0277]: the trait bound `AppState: Clone` is not satisfied
   --> src/main.rs:13:15
    |
 13 |     let app = Router::new()
    |               ^^^^^^^^^^^^^ the trait `Clone` is not implemented for `AppState`
    |
note: required by a bound in `Router::<S>::new`
    |
140 |     S: Clone + Send + Sync + 'static,
    |        ^^^^^ required by this bound in `Router::<S>::new`
help: consider annotating `AppState` with `#[derive(Clone)]`
    |
  3 + #[derive(Clone)]
  4 | struct AppState {
    |
```

**原因**：`Router<S>` 的每个方法都要求 `S: Clone + Send + Sync + 'static`。上面其实先报了一条「Handler 未实现」，那条是假象；真正卡住的是 `AppState` 没有 `Clone`。

**修复**：给状态加 `#[derive(Clone)]`。若字段不便宜，把内部换成 `Arc`，让克隆只增加引用计数。

```rust,noplayground
#[derive(Clone)]
struct AppState {
    hits: u64,
}
```

### 案例 3：with_state 的类型和 handler 对不上——E0308

handler 声明要 `State<Arc<AppState>>`，注入时却给了 `AppState`：

```rust,noplayground
use std::sync::Arc;

use axum::{extract::State, routing::get, Router};

struct AppState {
    hits: u64,
}

async fn count(State(state): State<Arc<AppState>>) -> String {
    state.hits.to_string()
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/count", get(count))
        .with_state(AppState { hits: 0 });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

```text
error[E0308]: mismatched types
   --> src/main.rs:17:21
    |
 17 |         .with_state(AppState { hits: 0 });
    |          ---------- ^^^^^^^^^^^^^^^^^^^^ expected `Arc<AppState>`, found `AppState`
    |          |
    |          arguments to this method are incorrect
    |
    = note: expected struct `Arc<AppState>`
               found struct `AppState`
note: method defined here
    |
408 |     pub fn with_state<S2>(self, state: S) -> Router<S2> {
help: call `Into::into` on this expression to convert `AppState` into `Arc<AppState>`
    |
 17 |         .with_state(AppState { hits: 0 }.into());
    |                                         +++++++
```

**原因**：`with_state` 的参数类型由 handler 的 `State<T>` 反推出来，两边必须完全一致。`AppState` 和 `Arc<AppState>` 是两个不同的类型。

**修复**：注入时包一层 `Arc::new`，或者按编译器建议调用 `.into()`：

```rust,noplayground
let app = Router::new()
    .route("/count", get(count))
    .with_state(Arc::new(AppState { hits: 0 }));
```

### 案例 4：`MutexGuard` 被带过了 `await`——E0277

先看现象。下面这个 handler 语法上没有任何问题，但它过不了 `get()` 的约束：

```rust,noplayground
use std::sync::Mutex;

use axum::{routing::get, Router};

static LOCK: Mutex<u32> = Mutex::new(0);

async fn handler() -> String {
    let guard = LOCK.lock().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    format!("{}", *guard)
}

fn main() {
    let _app: Router = Router::new().route("/", get(handler));
    println!("ok");
}
```

```text
error[E0277]: the trait bound `fn() -> impl Future<Output = String> {handler}: Handler<_, _>` is not satisfied
   --> src/main.rs:13:53
    |
 13 |     let _app: Router = Router::new().route("/", get(handler));
    |                                                 --- ^^^^^^^ the trait `Handler<_, _>` is not implemented for fn item `fn() -> impl Future<Output = String> {handler}`
    |                                                 |
    |                                                 required by a bound introduced by this call
    |
    = note: Consider using `#[axum::debug_handler]` to improve the error message
note: required by a bound in `axum::routing::get`
   --> .../axum-0.8.9/src/routing/method_routing.rs:167:16
    |
167 |             H: Handler<T, S>,
    |                ^^^^^^^^^^^^^ required by this bound in `get`

For more information about this error, try `rustc --explain E0277`.
```

**原因**：axum 的 `Handler` 要求 handler 返回的 future 实现 `Send`，因为一个连接可能被调度到不同的工作线程。`std::sync::MutexGuard` 不是 `Send`，而它在 `await` 之后还要用，于是整个 future 不是 `Send`，`handler` 也就不满足 `Handler`。axum 的泛型约束只报出「Handler 没实现」，真正的原因被藏在 trait 求解里。把同一个 handler 交给一个显式要求 `Send` 的函数，就能看到底层报错：

```rust,noplayground
use std::sync::Mutex;

static LOCK: Mutex<u32> = Mutex::new(0);

async fn handler() -> String {
    let guard = LOCK.lock().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    format!("{}", *guard)
}

fn require_send<F: Send>(f: F) -> F {
    f
}

fn main() {
    let fut = require_send(handler());
    drop(fut);
}
```

```text
error: future cannot be sent between threads safely
  --> src/main.rs:16:28
   |
16 |     let fut = require_send(handler());
   |                            ^^^^^^^^^ future returned by `handler` is not `Send`
   |
   = help: within `impl Future<Output = String>`, the trait `Send` is not implemented for `std::sync::MutexGuard<'_, u32>`
note: future is not `Send` as this value is used across an await
  --> src/main.rs:7:61
   |
 6 |     let guard = LOCK.lock().unwrap();
   |         ----- has type `std::sync::MutexGuard<'_, u32>` which is not `Send`
 7 |     tokio::time::sleep(std::time::Duration::from_millis(1)).await;
   |                                                             ^^^^^ await occurs here, with `guard` maybe used later
note: required by a bound in `require_send`
  --> src/main.rs:11:20
   |
11 | fn require_send<F: Send>(f: F) -> F {
   |                    ^^^^ required by this bound in `require_send`
```

**修复**：不要跨 `await` 持有锁。需要共享可变状态时用 `tokio::sync::Mutex`（它的 guard 可以跨 `await`），或者更简单地先把数据取出来再放锁：

```rust,noplayground
use std::sync::Mutex;

static LOCK: Mutex<u32> = Mutex::new(0);

async fn handler() -> String {
    let value = {
        let guard = LOCK.lock().unwrap();
        *guard
    }; // guard 在这里就释放了，没有跨过 await
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    format!("{value}")
}
```

`#[axum::debug_handler]` 能把这类错误的报错信息提升到「哪个参数/哪一步导致 future 不是 Send」，需要额外打开 axum 的 `macros` feature；不开也没关系，记住「先看 `-->` 指向哪一行，再把同一段代码交给 `require_send`」这个排查套路就够了。

### 案例 5：自定义错误没实现 `IntoResponse`——E0277

`Result<T, E>` 作为返回值可以让 `?` 顺畅传播，但 axum 只认得实现了 `IntoResponse` 的 `E`：

```rust,noplayground
use axum::response::IntoResponse;

#[derive(Debug)]
struct MyErr;

fn require_into_response<T: IntoResponse>() {}

fn main() {
    require_into_response::<MyErr>();
}
```

```text
error[E0277]: the trait bound `MyErr: IntoResponse` is not satisfied
 --> src/main.rs:9:29
  |
9 |     require_into_response::<MyErr>();
  |                             ^^^^^ unsatisfied trait bound
  |
help: the trait `IntoResponse` is not implemented for `MyErr`
 --> src/main.rs:4:1
  |
4 | struct MyErr;
  | ^^^^^^^^^^^^
  = help: the following other types implement trait `IntoResponse`:
            (StatusCode, R)
            (T1, R)
            (axum::http::response::Parts, R)
            (Response<()>, T1, R)
          and 120 others
note: required by a bound in `require_into_response`
 --> src/main.rs:6:29
  |
6 | fn require_into_response<T: IntoResponse>() {}
  |                             ^^^^^^^^^^^^ required by this bound in `require_into_response`

For more information about this error, try `rustc --explain E0277`.
```

**原因**：`(StatusCode, R)`、`Json<T>`、`String` 这些类型都实现了 `IntoResponse`，而自己定义的 `MyErr` 只是一个普通结构体。错误提示里那句「the following other types implement trait IntoResponse」列出的，正是 axum 内置的转换形式。

**修复**：给错误类型实现 `IntoResponse`，把「内部错误长什么样」的决定权留在类型自己手里：

```rust,noplayground
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
struct MyErr;

impl IntoResponse for MyErr {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}
```

团队里更常见的做法是让库层错误实现 `std::error::Error`（或派生 `thiserror::Error`），在边界层用 `impl From<MyErr> for ApiError` 统一映射成状态码与 JSON；handler 里就可以一路 `?`，由 `ApiError` 的 `IntoResponse` 决定最终响应。

## 动手练习

1. 给 31.2 的 `AppState` 加一个 `todo!`，写 `POST /todos`：用 `Json<NewTodo>` 提取请求体，成功后返回 `(StatusCode::CREATED, Json(todo))`。说明为什么返回元组的两个元素顺序不影响响应格式。
2. 实现一个自定义提取器 `ApiKey`：实现 `FromRequestParts`，从 `X-Api-Key` 请求头取值；缺失或为空时返回 `401` 加 JSON 错误体。写出它与 `HeaderMap` 手写提取相比的好处。
3. 把状态换成 `Arc<Mutex<Vec<Todo>>>`（`std` 与 `tokio` 各写一版），实现 `GET /todos` 与 `POST /todos`。说明哪一版会触发案例 4 的 `Send` 问题，以及怎么改。
4. 用 `tower_http` 的 `TraceLayer`、`TimeoutLayer`、`CorsLayer` 给服务加中间件。写出 `Router::layer` 的代码，并说明「先加的是外层还是内层」。
5. 写两个测试：用 `tower::ServiceExt::oneshot` 断言 `GET /todos` 返回 `200` 且 body 是合法 JSON；再用 `with_graceful_shutdown` 配 `tokio::signal::ctrl_c()` 写一个可以被 Ctrl-C 干净关闭的 `main`。

## 小结

- axum 的分层是 hyper（字节与协议）→ tower（一切皆 service）→ Router（方法与路径匹配）→ handler（业务），route 只负责匹配，handler 只负责业务，中间件插在 service 层。
- `Router::new().route(path, method_router)` 组装应用，`with_state` 注入状态，`serve` 绑定监听地址；`axum::serve` 自带优雅关闭入口。
- 提取器按参数顺序执行，只能有一个实现 `FromRequest`（消费 body，如 `Json`）的提取器，且必须放在参数列表最后。
- 自定义类型要做提取器：只需要请求元信息就实现 `FromRequestParts`，要读 body 就实现 `FromRequest`，失败时给出可 `IntoResponse` 的错误。
- handler 的返回值只要实现 `IntoResponse` 就能当响应：`String`、`StatusCode`、`Json<T>`、元组、`Result<T, E>` 都是现成的组合方式。
- 自定义错误类型要实现 `IntoResponse`（或者统一转换成一个实现了它的 `ApiError`），否则 handler 签名里出现 `Result<T, MyErr>` 就会触发案例 5。
- handler 的 future 必须是 `Send`：不要跨 `await` 持有 `std::sync::MutexGuard`，需要跨就换 `tokio::sync::Mutex`，或者把锁的作用域缩小到 `await` 之前。
- 中间件是 tower 的 `Layer`：`TraceLayer` 记日志、`TimeoutLayer` 限时、`CorsLayer` 处理跨域，用 `Router::layer` 加在匹配之后、`ServiceBuilder` 可以一次串起多层。
- 测试有两种粒度：`oneshot` 直接把请求喂给 `Router`，不需要真实端口；`axum::serve` 起真服务则适合端到端验证。
- 别用 `Mutex<Router>` 来「共享路由」，`Router` 本身是 `Clone` 且内部共享；状态放进 `with_state`，共享用 `Arc`。





