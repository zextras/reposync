#![allow(missing_docs)]
mod config;
mod debian;
mod destination;
mod fetcher;
mod locks;
mod packages;
mod redhat;
mod server;
mod state;
mod sync;
mod utils;

use crate::sync::SyncManager;
use clap::{App, Arg};
use std::process::exit;

fn main() {
    env_logger::init();

    let action_validator = |x: String| -> Result<(), String> {
        if vec!["check", "sync", "server"].contains(&x.as_str()) {
            Ok(())
        } else {
            Err("only check, sync, server are valid actions".into())
        }
    };

    let matches = App::new("RepoSync")
        .version("0.10.0")
        .about("Keep a repository synchronized to an S3 bucket")
        .args(&[
            Arg::with_name("config")
                .long("config")
                .value_name("CONFIG_FILE")
                .help("location of config file")
                .takes_value(true)
                .required(true)
                .index(1),
            Arg::with_name("action")
                .long("action")
                .value_name("ACTION")
                .help("action to perform, 'check', 'sync' or 'server'")
                .takes_value(true)
                .required(true)
                .validator(action_validator)
                .index(2),
            Arg::with_name("repository")
                .long("repo")
                .value_name("REPO")
                .help("which repo to synchronize, check, sync, or server")
                .takes_value(true)
                .required(false),
            Arg::with_name("dry-run")
                .long("dry-run")
                .help("show what would be done without making any changes")
                .takes_value(false),
        ])
        .get_matches();

    let config_file = matches.value_of("config").unwrap();

    let result = config::load_config(config_file);
    if result.is_err() {
        println!("{}", result.err().unwrap());
        exit(1);
    }
    let config = result.unwrap();

    let action = matches.value_of("action").unwrap();
    let dry_run = matches.is_present("dry-run");
    match action {
        "check" => {
            println!("config file is correct");
            exit(0);
        }
        "sync" => {
            if let Some(repo_name) = matches.value_of("repository") {
                let repo_names: Vec<String>;
                if repo_name == "all" {
                    repo_names = config.repo.iter().map(|r| r.name.clone()).collect();
                } else {
                    repo_names = vec![repo_name.into()]
                }
                let sync_manager = SyncManager::new(config, dry_run);
                for repo_name in repo_names {
                    let result = sync_manager.sync_repo(&repo_name);
                    if let Err(err) = result {
                        println!("failed to synchronize {}: {}", repo_name, err);
                        exit(1);
                    }
                    if dry_run {
                        println!("{} dry-run complete", repo_name);
                    } else {
                        println!("{} fully synchronized", repo_name);
                    }
                }
                exit(0);
            } else {
                println!("missing argument repo");
                exit(1);
            }
        }
        "server" => {
            start_server(
                &config.general.bind_address.clone(),
                SyncManager::new(config, false),
            );
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
