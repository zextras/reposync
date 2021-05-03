use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use toml::de::Error;

#[derive(Serialize, Deserialize)]
pub struct Source {
    endpoint: String,
    kind: String,
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
pub struct Destination {
    s3: String,
    cdn_arn: String,
    access_key: String,
    secret: String,
}

#[derive(Serialize, Deserialize)]
pub struct Repository {
    source: Source,
    destination: Destination,
}

#[derive(Serialize, Deserialize)]
struct General {
    data_path: String,
    timeout: u32,
    debounce: u32,
    auto_align: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    general: General,
    repo: BTreeMap<String, Repository>,
}

pub fn load_config(path: &str) -> Result<Config, String> {
    let text = fs::read_to_string(path);
    if text.is_err() {
        return Result::Err(format!("Cannot read file {}", path));
    }

    let config = toml::from_str(&text.unwrap());
    if config.is_err() {
        return Result::Err(format!("Cannot parse toml file"));
    }

    Result::Ok(config.unwrap())
}

#[cfg(test)]
pub mod tests {
    use crate::config::{load_config, Config};
    use std::fs;

    #[test]
    fn load_sample_config() {
        let config = load_config("data/sample-config.toml").expect("cannot config sample");
        assert_eq!("/data/repo/", config.general.data_path);
        assert!(config.repo.contains_key("centos8"));
        assert!(config.repo.contains_key("ubuntu"));
        toml::to_string(&config).expect("cannot convert back to toml");
    }
}
