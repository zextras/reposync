use pgp::composed::{Deserializable, SignedPublicKey};
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
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            return Ok(Some(format!("{}:{}", username, password)));
        }

        if let Some(auth_file) = &self.authorization_file {
            let mut text = String::new();
            File::open(auth_file)?.read_to_string(&mut text)?;
            return Ok(Some(text.replace('\n', "").replace('\r', "")));
        }

        Ok(None)
    }

    pub fn parse_public_key(&self) -> Result<Option<SignedPublicKey>, std::io::Error> {
        if let Some(key_str) = &self.public_pgp_key {
            let (public_key, _) = SignedPublicKey::from_string(key_str).map_err(|err| {
                std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!("cannot parse public key: {}", err),
                )
            })?;
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
        if let (Some(id), Some(secret)) = (&self.access_key_id, &self.access_key_secret) {
            return Ok((id.clone(), secret.clone()));
        }

        if let Some(cred_file) = &self.aws_credential_file {
            let mut text = String::new();
            File::open(cred_file)?.read_to_string(&mut text)?;
            let vec: Vec<&str> = text.splitn(2, '\n').collect();
            if vec.len() == 2 {
                Ok((
                    vec[0].replace('\n', "").replace('\r', ""),
                    vec[1].replace('\n', "").replace('\r', ""),
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
    /// Local path for persisting repo index state between syncs.
    /// Required only when using a `local` destination; S3 destinations read
    /// state directly from the bucket so no local storage is needed.
    #[serde(default)]
    pub data_path: Option<String>,
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
    let text = fs::read_to_string(path).map_err(|e| format!("Cannot read file {}: {}", path, e))?;

    let mut config: Config =
        serde_yaml::from_str(&text).map_err(|e| format!("Cannot parse file {}: {}", path, e))?;

    let has_local_destination = config.repo.iter().any(|r| r.destination.local.is_some());
    if has_local_destination && config.general.data_path.is_none() {
        return Err("general.data_path is required when using a local destination".to_string());
    }

    //normalize slashes
    for repo in &mut config.repo {
        repo.source.endpoint = remove_trailing_slash(&repo.source.endpoint);
        if let Some(s3) = repo.destination.s3.as_mut() {
            s3.s3_endpoint = remove_trailing_slash(&s3.s3_endpoint);
            s3.path = remove_initial_slash(&remove_trailing_slash(&s3.path));
            if let Some(cf_endpoint) = &s3.cloudfront_endpoint {
                s3.cloudfront_endpoint = Some(remove_trailing_slash(cf_endpoint));
            }
        }

        if let Some(local) = repo.destination.local.as_mut() {
            local.path = remove_trailing_slash(&local.path);
        }
    }

    //verify
    let mut used_names: Vec<&String> = vec![];
    for repo in &config.repo {
        if repo.name == "all" {
            return Err("'all' was used as repository name, but it's a reserved word'".to_string());
        }
        if used_names.contains(&&repo.name) {
            return Err(format!(
                "'{}' was used as repository name twice",
                &repo.name
            ));
        }
        used_names.push(&repo.name);

        if !["debian", "redhat"].contains(&repo.source.kind.as_str()) {
            return Err(format!(
                "unknown repository type '{}', only 'debian' and 'redhat' are supported",
                &repo.source.kind
            ));
        }

        if let Err(err) = repo.source.get_authorization_secret() {
            return Err(format!("cannot parse authorization: {}", err));
        }

        if let Some(public_key) = repo.source.parse_public_key().map_err(|e| e.to_string())? {
            public_key
                .verify_bindings()
                .map_err(|e| format!("cannot verify public key: {}", e))?;
        }

        if repo.destination.s3.is_some() && repo.destination.local.is_some() {
            return Err("cannot have both s3 and local destination".to_string());
        }

        if repo.destination.s3.is_none() && repo.destination.local.is_none() {
            return Err("you must define at least one destination, either local or s3".to_string());
        }

        if let Some(s3) = &repo.destination.s3 {
            if let Err(err) = s3.get_aws_credentials() {
                return Err(format!("cannot read aws credential: {}", err));
            }
        }

        if let Some(local) = &repo.destination.local {
            if !local.path.starts_with('/') {
                return Err("local destination path must be absolute".to_string());
            }
        }
    }

    Ok(config)
}

fn remove_initial_slash(s: &str) -> String {
    s.strip_prefix('/')
        .map(|s| s.to_string())
        .unwrap_or_else(|| s.to_string())
}

fn remove_trailing_slash(s: &str) -> String {
    s.strip_suffix('/')
        .map(|s| s.to_string())
        .unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
pub mod tests {
    use crate::config::{load_config, SourceConfig};
    use std::fs;

    #[test]
    fn load_sample_config() {
        let config = load_config("samples/config.yaml").unwrap();
        assert_eq!(Some("/data/repo/".to_string()), config.general.data_path);
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
