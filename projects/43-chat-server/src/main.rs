//! 第 43 章 项目五：异步聊天服务器 —— 进程入口（main.rs）

use std::env;

use tokio::sync::watch;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| String::from(chat_server::DEFAULT_ADDR));
    let (stop_tx, stop_rx) = watch::channel(false);
    println!("聊天服务器监听 {}", addr);

    let mut server = tokio::spawn(async move { chat_server::serve(&addr, stop_rx).await });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("收到 Ctrl-C，正在关闭");
            let _ = stop_tx.send(true);
        }
        res = &mut server => {
            res.unwrap()?;
            return Ok(());
        }
    }

    server.await.unwrap()?;
    println!("已退出");
    Ok(())
}
