#![allow(missing_docs)]
mod config;
mod server;

#[tokio::main]
async fn main() {
    env_logger::init();
    server::create("127.0.0.1:8080").await;
}
