use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

#[derive(Serialize, Deserialize)]
pub struct SourceConfig {
    pub endpoint: String,
    pub kind: String,
    pub username: String,
    pub password: String,
}

#[derive(Serialize, Deserialize)]
pub struct DestinationConfig {
    pub s3: String,
    pub cdn_arn: String,
    pub access_key: String,
    pub secret: String,
}

#[derive(Serialize, Deserialize)]
pub struct RepositoryConfig {
    pub name: String,
    pub source: SourceConfig,
    pub destination: DestinationConfig,
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GeneralConfig {
    pub data_path: String,
    pub tmp_path: String,
    pub timeout: u32,
    pub debounce: u32,
    pub auto_align: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub repo: Vec<RepositoryConfig>,
}

pub fn load_config(path: &str) -> Result<Config, String> {
    let text = fs::read_to_string(path);
    if text.is_err() {
        return Result::Err(format!("Cannot read file {}", path));
    }

    let config = serde_yaml::from_str(&text.unwrap());
    if config.is_err() {
        return Result::Err(format!("Cannot parse config file"));
    }

    Result::Ok(config.unwrap())
}

#[cfg(test)]
pub mod tests {
    use crate::config::{load_config, Config};
    use std::fs;

    #[test]
    fn load_sample_config() {
        let config = load_config("samples/config.yaml").expect("cannot config sample");
        assert_eq!("/data/repo/", config.general.data_path);
        assert_eq!(2, config.repo.len());
        let repo0 = config.repo.get(0).unwrap();
        assert_eq!("centos8", repo0.name);
        let repo1 = config.repo.get(1).unwrap();
        assert_eq!("ubuntu", repo1.name);
        serde_yaml::to_string(&config).expect("cannot convert back to toml");
    }
}
