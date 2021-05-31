use pgp::{Deserializable, SignedPublicKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;

#[derive(Serialize, Deserialize, Clone)]
pub struct SourceConfig {
    pub endpoint: String,
    pub kind: String,
    pub public_pgp_key: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl SourceConfig {
    pub fn parse_public_key(&self) -> Result<Option<SignedPublicKey>, std::io::Error> {
        if self.public_pgp_key.is_some() {
            let result = SignedPublicKey::from_string(&self.public_pgp_key.clone().unwrap());
            if result.is_err() {
                let err = result.err().unwrap();
                return Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!("cannot parse public key: {}", err.as_code()),
                ));
            }

            let (public_key, _) = result.unwrap();
            Ok(Some(public_key))
        } else {
            Ok(None)
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DestinationConfig {
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub path: String,
    pub cloudfront_endpoint: Option<String>,
    pub cloudfront_arn: Option<String>,
    pub region_name: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RepositoryConfig {
    pub name: String,
    pub source: SourceConfig,
    pub destination: DestinationConfig,
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    pub data_path: String,
    pub tmp_path: String,
    pub timeout: u32,
    pub debounce: u32,
    pub auto_align: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub general: GeneralConfig,
    pub repo: Vec<RepositoryConfig>,
}

pub fn load_config(path: &str) -> Result<Config, String> {
    let text = fs::read_to_string(path);
    if text.is_err() {
        return Result::Err(format!(
            "Cannot read file {}: {}",
            path,
            text.err().unwrap().to_string()
        ));
    }

    let config_result = serde_yaml::from_str(&text.unwrap());
    if config_result.is_err() {
        return Result::Err(format!(
            "Cannot parse file {}: {}",
            path,
            config_result.err().unwrap().to_string()
        ));
    }

    //normalize ending slash
    let mut config: Config = config_result.unwrap();
    for repo in &mut config.repo {
        repo.source.endpoint = normalize(&repo.source.endpoint);
        repo.destination.s3_endpoint = normalize(&repo.destination.s3_endpoint);
        repo.destination.path = normalize(&repo.destination.path);
        if repo.destination.cloudfront_endpoint.is_some() {
            repo.destination.cloudfront_endpoint = Some(normalize(
                &repo.destination.cloudfront_endpoint.clone().unwrap(),
            ));
        }
    }

    //verify
    let mut used_names: Vec<&String> = vec![];
    for repo in &config.repo {
        if repo.name == "all" {
            return Result::Err(format!(
                "'all' was used as repository name, but it's a reserved word'",
            ));
        }
        if used_names.contains(&&repo.name) {
            return Result::Err(format!(
                "'{}' was used as repository name twice",
                &repo.name
            ));
        }
        used_names.push(&repo.name);
        let result = repo.source.parse_public_key();
        if result.is_err() {
            return Result::Err(result.err().unwrap().to_string());
        }
        if let Some(public_key) = result.unwrap() {
            let result = public_key.verify();
            if result.is_err() {
                return Result::Err(format!(
                    "cannot verify public key: {}",
                    result.err().unwrap().to_string()
                ));
            }
        }
    }

    Result::Ok(config)
}

fn normalize(s: &str) -> String {
    if s.ends_with("/") {
        let len = s.len();
        s[0..len - 1].into()
    } else {
        s.into()
    }
}

#[cfg(test)]
pub mod tests {
    use crate::config::{load_config, Config, SourceConfig};
    use pgp::armor::BlockType;
    use pgp::packet::{Packet, PacketParser, PublicKey};
    use pgp::types::{PublicKeyTrait, Version};
    use pgp::{Deserializable, PublicKeyParser, SignedPublicKey};
    use std::convert::TryFrom;
    use std::fs;
    use std::io::{BufReader, Cursor, Read};
    use std::str::FromStr;

    #[test]
    fn load_sample_config() {
        let config = load_config("samples/config.yaml").unwrap();
        assert_eq!("/data/repo/", config.general.data_path);
        assert_eq!(2, config.repo.len());
        let repo0 = config.repo.get(0).unwrap();
        assert_eq!("centos8", repo0.name);
        let repo1 = config.repo.get(1).unwrap();
        assert_eq!("ubuntu", repo1.name);
        serde_yaml::to_string(&config).expect("cannot convert back to toml");
    }

    #[test]
    fn parse_public_key() {
        let public_key_text =
            fs::read_to_string("samples/public-key").expect("cannot read public-key file");

        let source_config = SourceConfig {
            endpoint: "".to_string(),
            kind: "".to_string(),
            public_pgp_key: Some(public_key_text),
            username: None,
            password: None,
        };

        source_config.parse_public_key().unwrap().unwrap();
    }
}
