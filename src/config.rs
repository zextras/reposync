use pgp::{Deserializable, SignedPublicKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::{Error, ErrorKind, Read};

#[derive(Serialize, Deserialize, Clone)]
pub struct SourceConfig {
    pub endpoint: String,
    pub kind: String,
    pub public_pgp_key: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub authorization_file: Option<String>,
}

impl SourceConfig {
    pub fn get_authorization_secret(&self) -> Result<Option<String>, std::io::Error> {
        if self.username.is_some() && self.password.is_some() {
            return Ok(Some(format!(
                "{}:{}",
                self.username.clone().unwrap(),
                self.password.clone().unwrap()
            )));
        }

        if self.authorization_file.is_some() {
            let mut text = String::new();
            File::open(&self.authorization_file.clone().unwrap())?.read_to_string(&mut text)?;
            return Ok(Some(text.replace('\n', "").replace('\r', "")));
        }

        Ok(None)
    }

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
pub struct S3Destination {
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub path: String,
    pub cloudfront_endpoint: Option<String>,
    pub cloudfront_distribution_id: Option<String>,
    pub region_name: String,
    pub access_key_id: Option<String>,
    pub access_key_secret: Option<String>,
    pub aws_credential_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalDestination {
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DestinationConfig {
    pub s3: Option<S3Destination>,
    pub local: Option<LocalDestination>,
}

impl S3Destination {
    ///returns (access_key_id,access_key_secret)
    pub fn get_aws_credentials(&self) -> Result<(String, String), std::io::Error> {
        if self.access_key_id.is_some() && self.access_key_secret.is_some() {
            Ok((
                self.access_key_id.clone().unwrap(),
                self.access_key_secret.clone().unwrap(),
            ))
        } else {
            if self.aws_credential_file.is_some() {
                let mut text = String::new();
                File::open(&self.aws_credential_file.clone().unwrap())?
                    .read_to_string(&mut text)?;
                let vec: Vec<&str> = text.splitn(2, "\n").collect();
                if vec.len() == 2 {
                    Ok((
                        vec.get(0)
                            .unwrap()
                            .to_string()
                            .replace('\n', "")
                            .replace('\r', ""),
                        vec.get(1)
                            .unwrap()
                            .to_string()
                            .replace('\n', "")
                            .replace('\r', ""),
                    ))
                } else {
                    Err(Error::new(
                        ErrorKind::InvalidInput,
                        "invalid aws credential file, expected: \"access_key_id\naccess_key_secret\"",
                    ))
                }
            } else {
                Err(Error::new(
                    ErrorKind::InvalidInput,
                    "missing aws credential",
                ))
            }
        }
    }
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
    pub bind_address: String,
    pub timeout: u32,
    pub max_retries: u32,
    pub retry_sleep: u64,
    pub min_sync_delay: u32,
    pub max_sync_delay: u32,
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

    //normalize slashes
    let mut config: Config = config_result.unwrap();
    for repo in &mut config.repo {
        repo.source.endpoint = remove_trailing_slash(&repo.source.endpoint);
        if repo.destination.s3.is_some() {
            let mut s3 = repo.destination.s3.clone().unwrap();
            s3.s3_endpoint = remove_trailing_slash(&s3.s3_endpoint);
            s3.path = remove_initial_slash(&remove_trailing_slash(&s3.path));
            if s3.cloudfront_endpoint.is_some() {
                s3.cloudfront_endpoint = Some(remove_trailing_slash(
                    &s3.cloudfront_endpoint.clone().unwrap(),
                ));
            }
            repo.destination.s3 = Some(s3);
        }

        if repo.destination.local.is_some() {
            let mut local = repo.destination.local.clone().unwrap();
            local.path = remove_trailing_slash(&local.path);
            repo.destination.local = Some(local);
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

        if !["debian", "redhat"].contains(&repo.source.kind.as_str()) {
            return Result::Err(format!(
                "unknown repository type '{}', only 'debian' and 'redhat' are supported",
                &repo.source.kind
            ));
        }

        if let Err(err) = repo.source.get_authorization_secret() {
            return Result::Err(format!("cannot parse authorization: {}", err.to_string()));
        }

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

        if repo.destination.s3.is_some() && repo.destination.local.is_some() {
            return Result::Err(format!("cannot have both s3 and local destination"));
        }

        if repo.destination.s3.is_none() && repo.destination.local.is_none() {
            return Result::Err(format!(
                "you must define at least one destination, either local or s3"
            ));
        }

        if repo.destination.s3.is_some() {
            if let Err(err) = repo.destination.s3.clone().unwrap().get_aws_credentials() {
                return Err(format!("cannot read aws credential: {}", err.to_string()));
            }
        }

        if repo.destination.local.is_some() {
            if !repo
                .destination
                .local
                .clone()
                .unwrap()
                .path
                .starts_with("/")
            {
                return Err(format!("local destination path must be absolute"));
            }
        }
    }

    Result::Ok(config)
}

fn remove_initial_slash(s: &str) -> String {
    if s.starts_with("/") {
        let len = s.len();
        s[1..len].into()
    } else {
        s.into()
    }
}
fn remove_trailing_slash(s: &str) -> String {
    if s.ends_with("/") {
        let len = s.len();
        s[0..len - 1].into()
    } else {
        s.into()
    }
}

#[cfg(test)]
pub mod tests {
    use crate::config::{load_config, SourceConfig};
    use std::fs;

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
            authorization_file: None,
        };

        source_config.parse_public_key().unwrap().unwrap();
    }
}
