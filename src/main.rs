#![allow(missing_docs)]
#[allow(clippy::all)]
mod config;
#[allow(clippy::all)]
mod debian;
mod destination;
mod fetcher;
#[allow(clippy::all)]
mod locks;
#[allow(clippy::all)]
mod packages;
#[allow(clippy::all)]
mod redhat;
mod retry;
mod server;
#[allow(clippy::all)]
mod state;
#[allow(clippy::all)]
mod sync;
mod utils;
use crate::sync::SyncManager;
use clap::{Arg, Command};
use std::process::exit;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let matches = Command::new("RepoSync")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Keep a repository synchronized to an S3 bucket")
        .args(&[
            Arg::new("config")
                .long("config")
                .value_name("CONFIG_FILE")
                .help("location of config file")
                .required(true)
                .index(1),
            Arg::new("action")
                .long("action")
                .value_name("ACTION")
                .help("action to perform, 'check', 'sync' or 'server'")
                .required(true)
                .value_parser(["check", "sync", "server"])
                .index(2),
            Arg::new("repository")
                .long("repo")
                .value_name("REPO")
                .help("which repo to synchronize, check, sync, or server")
                .required(false),
            Arg::new("dry-run")
                .long("dry-run")
                .help("show what would be done without making any changes")
                .action(clap::ArgAction::SetTrue),
        ])
        .get_matches();

    let config_file: &String = matches
        .get_one("config")
        .expect("config argument is required");
    let config = config::load_config(config_file).unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    });

    let action: &String = matches
        .get_one("action")
        .expect("action argument is required");
    let dry_run = matches.get_flag("dry-run");
    match action.as_str() {
        "check" => {
            log::info!("config file is correct");
            exit(0);
        }
        "sync" => {
            if let Some(repo_name) = matches.get_one::<String>("repository") {
                let repo_names: Vec<String> = if repo_name == "all" {
                    config.repo.iter().map(|r| r.name.clone()).collect()
                } else {
                    vec![repo_name.clone()]
                };
                let sync_manager = SyncManager::new(config, dry_run);
                for repo_name in repo_names {
                    let result = sync_manager.sync_repo(&repo_name);
                    if let Err(err) = result {
                        log::error!("failed to synchronize {}: {}", repo_name, err);
                        exit(1);
                    }
                    if dry_run {
                        log::info!("{} dry-run complete", repo_name);
                    } else {
                        log::info!("{} fully synchronized", repo_name);
                    }
                }
                exit(0);
            } else {
                log::error!("missing argument repo");
                exit(1);
            }
        }
        "server" => {
            let addr = config.general.bind_address.clone();
            start_server(&addr, SyncManager::new(config, false));
        }
        _ => {
            panic!("unknown action {}", action);
        }
    }
}

#[tokio::main]
async fn start_server(bind_address: &str, sync_manager: SyncManager) {
    server::create(sync_manager, bind_address).await
}
