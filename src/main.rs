#![allow(missing_docs)]
mod config;
mod debian;
mod fetcher;
mod packages;
mod redhat;
mod server;
mod state;
mod sync;

#[tokio::main]
async fn main() {
    env_logger::init();
    server::create("127.0.0.1:8080").await;
}
