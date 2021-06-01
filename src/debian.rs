use crate::config::RepositoryConfig;
use crate::fetcher::Fetcher;
use crate::packages::{Collection, Hash, IndexFile, Package, Repository, Signature, Target};
use crate::state::{LiveRepoMetadataStore, RepoMetadataStore, SavedRepoMetadataStore};
use crate::utils::add_optional_index;
use regex::Regex;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::rc::Rc;
use std::str::FromStr;

#[derive(Debug, Eq, PartialEq, Clone)]
struct PackagesReference {
    path: String,
    size: usize,
    sha256: String,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Release {
    pub codename: String,
    pub components: Vec<String>,
    pub architectures: Vec<String>,
    pub indexes: Vec<IndexFile>,
}

pub fn fetch_repository(
    fetcher: Rc<dyn Fetcher>,
    tmp_path: &str,
    config: &RepositoryConfig,
) -> Result<(Repository, LiveRepoMetadataStore), std::io::Error> {
    let repo_metadata = LiveRepoMetadataStore::new(&config.source.endpoint, tmp_path, fetcher);
    let result = fetch_repository_internal(&repo_metadata, config, false);
    if let Err(err) = result {
        return Err(std::io::Error::new(
            err.kind(),
            format!("cannot fetch repo state: {}", &err.to_string()),
        ));
    }
    Ok((result.unwrap(), repo_metadata))
}

pub fn load_repository(
    data_path: &str,
    config: &RepositoryConfig,
) -> Result<(Repository, SavedRepoMetadataStore), std::io::Error> {
    let repo_metadata = SavedRepoMetadataStore::new(data_path);
    let result = fetch_repository_internal(&repo_metadata, config, true);
    if let Err(err) = result {
        return Err(std::io::Error::new(
            err.kind(),
            format!("cannot load current repo state: {}", &err.to_string()),
        ));
    }

    Ok((result.unwrap(), repo_metadata))
}

//internal function for dependency injection
fn fetch_repository_internal<T>(
    state: &T,
    config: &RepositoryConfig,
    allow_empty: bool,
) -> Result<Repository, std::io::Error>
where
    T: RepoMetadataStore,
{
    let mut repo = Repository {
        name: config.name.clone(),
        collections: vec![],
    };

    for version_codename in &config.versions {
        let version_path = format!("dists/{}", version_codename);
        let path = format!("{}/Release", &version_path);
        let result = state.fetch(&path);
        if allow_empty {
            if let Err(err) = result {
                //mostly useful when adding a new distribution
                if err.kind() == ErrorKind::NotFound {
                    continue;
                } else {
                    return Err(err);
                }
            }
        } else {
            if result.is_err() {
                return Err(result.err().unwrap());
            }
        }
        let (disk_path, reader, size) = result.unwrap();
        let mut release = parse_release(reader, &version_path)?;

        let mut indexes: Vec<IndexFile> = vec![];

        //this index file is optional
        add_optional_index(
            state,
            &format!("{}/InRelease", version_path),
            &mut indexes,
            Signature::PGPEmbedded,
        )?;
        let signature = add_optional_index(
            state,
            &format!("{}/Release.gpg", version_path),
            &mut indexes,
            Signature::None,
        )?;

        if signature.is_some() {
            let mut text_signature = String::new();
            signature.unwrap().read_to_string(&mut text_signature)?;
            indexes.insert(
                0,
                IndexFile {
                    file_path: disk_path,
                    path,
                    size,
                    hash: Hash::None,
                    signature: Signature::PGPExternal {
                        signature: text_signature,
                    },
                },
            );
        } else {
            indexes.insert(
                0,
                IndexFile {
                    file_path: disk_path,
                    path,
                    size,
                    hash: Hash::None,
                    signature: Signature::None,
                },
            );
        }

        let mut packages: Vec<Package> = Vec::new();

        for index in &mut release.indexes {
            let (disk_path, reader, size) = state.fetch(&index.path)?;
            index.file_path = disk_path;
            if index.size != size {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "wrong file size for '{}', expected: {} found {}",
                        index.path, index.size, size
                    ),
                ));
            }
            if index.path.ends_with("Packages") {
                packages.append(&mut parse_packages(reader)?);
            }
        }

        indexes.append(&mut release.indexes);

        repo.collections.push(Collection {
            target: Target {
                release_name: version_codename.clone(),
                architectures: release.architectures.clone(),
            },
            indexes,
            packages,
        });
    }

    Ok(repo)
}

pub fn parse_release<R>(input_read: R, base_path: &str) -> Result<Release, std::io::Error>
where
    R: Read,
{
    let mut input = BufReader::new(input_read);

    let mut release = Release {
        codename: "".to_string(),
        components: Vec::new(),
        architectures: Vec::new(),
        indexes: Vec::new(),
    };

    let mut parsing_sha256 = false;

    loop {
        let mut buffer: Vec<u8> = Vec::new();
        if input.read_until(b'\n', &mut buffer)? == 0 {
            break;
        }

        let line = String::from_utf8(buffer).unwrap().replace("\n", "");
        if line.starts_with(" ") {
            if parsing_sha256 {
                let re = Regex::new(" *([a-z0-9]+) *([0-9]+) *(.*)").unwrap();
                if let Some(group) = re.captures(&line) {
                    if group.len() == 4 {
                        let size = u64::from_str(group.get(2).unwrap().as_str());
                        if size.is_err() {
                            return Result::Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                format!(
                                    "cannot parse release file, invalid number in line: {}",
                                    line
                                ),
                            ));
                        }
                        release.indexes.push(IndexFile {
                            file_path: "".into(),
                            path: format!("{}/{}", base_path, group.get(3).unwrap().as_str()),
                            size: size.unwrap(),
                            hash: Hash::Sha256 {
                                hex: group.get(1).unwrap().as_str().to_string(),
                            },
                            signature: Signature::None,
                        })
                    } else {
                        return Result::Err(std::io::Error::new(
                            ErrorKind::InvalidData,
                            format!("cannot parse release file, invalid line: {}", line),
                        ));
                    }
                } else {
                    return Result::Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        format!("cannot parse release file, invalid line: {}", line),
                    ));
                }
            }
            continue;
        }
        parsing_sha256 = false;

        let tokens: Vec<&str> = line.splitn(2, ":").collect();
        if tokens.len() != 2 {
            return Result::Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("cannot parse release file, invalid line: {}", line),
            ));
        }

        let key = *tokens.get(0).unwrap();
        let value = tokens.get(1).unwrap().trim();

        match key {
            "Codename" => release.codename = value.into(),
            "Components" => release.components = value.split(" ").map(|x| x.into()).collect(),
            "Architectures" => release.architectures = value.split(" ").map(|x| x.into()).collect(),
            "SHA256" => parsing_sha256 = true,
            _ => {}
        }
    }

    Result::Ok(release)
}

/**
    Parse packages filename streaming line per line, line which starts with an empty space(' ')
    are a continuation of the previous line.
*/
pub fn parse_packages<R>(input_read: R) -> Result<Vec<Package>, std::io::Error>
where
    R: Read,
{
    let mut input = BufReader::new(input_read);

    let mut key = String::new();
    let mut value = String::new();
    let mut packages: Vec<Package> = Vec::new();
    let mut current = Package::empty();

    let check_and_add = |key: &mut String,
                         value: &mut String,
                         current: &mut Package|
     -> Result<(), std::io::Error> {
        if key.len() > 0 {
            match key.as_str() {
                "Package" => current.name = value.clone(),
                "Version" => current.version = value.clone(),
                "Architecture" => current.architecture = value.clone(),
                "Filename" => current.path = value.clone(),
                "SHA256" => current.hash = Hash::Sha256 { hex: value.clone() },
                "Size" => {
                    let clean_value = value.trim();
                    let result = u64::from_str(clean_value);
                    if result.is_err() {
                        return Result::Err(std::io::Error::new(
                            ErrorKind::InvalidData,
                            format!("invalid number {}", clean_value),
                        ));
                    }
                    current.size = result.unwrap();
                }
                _ => {}
            }
            key.clear();
            value.clear();
        }

        Result::Ok(())
    };

    loop {
        let mut buffer: Vec<u8> = Vec::new();
        if input.read_until(b'\n', &mut buffer)? == 0 {
            check_and_add(&mut key, &mut value, &mut current)?;
            if current.name.len() > 0 {
                packages.push(current);
            }
            break;
        }

        let line = String::from_utf8(buffer).unwrap().replace("\n", "");

        if line.is_empty() {
            check_and_add(&mut key, &mut value, &mut current)?;
            if current.name.len() > 0 {
                packages.push(current);
                current = Package::empty();
            }
            continue;
        }

        if line.starts_with(" ") {
            //skip the first space
            value += &line[1..];
        } else {
            check_and_add(&mut key, &mut value, &mut current)?;
            let tokens: Vec<&str> = line.splitn(2, ":").collect();
            if tokens.len() != 2 || tokens.get(1).unwrap().is_empty() {
                return Result::Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!("invalid line {}", line),
                ));
            }

            key = (*tokens.get(0).unwrap()).into();
            //skip first space
            value = (*tokens.get(1).unwrap())[1..].into();
        }
    }

    Result::Ok(packages)
}

#[cfg(test)]
pub mod tests {
    use crate::config::{DestinationConfig, RepositoryConfig, SourceConfig};
    use crate::debian::{
        fetch_repository_internal, parse_packages, parse_release, LiveRepoMetadataStore, Package,
    };
    use crate::fetcher::MockFetcher;
    use crate::packages::{Hash, IndexFile, Signature};
    use crate::state::RepoMetadataStore;
    use std::fs::File;
    use std::io::Read;
    use std::rc::Rc;

    #[test]
    fn fetch_repository_state() {
        let mut mock_fetcher = MockFetcher::new();

        mock_fetcher.expect_fetch().times(9).returning(|url: &str| {
            Result::Ok(Box::new(match url {
                "http://fake-url/rc/dists/focal/Release" => {
                    File::open("samples/debian/Release").unwrap()
                }
                "http://fake-url/rc/dists/focal/Release.gpg" => {
                    File::open("samples/fake-signature").unwrap()
                }
                "http://fake-url/rc/dists/focal/InRelease" => {
                    File::open("samples/debian/Release").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages.bz2" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages.gz" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages.bz2" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages.gz" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                _ => panic!("unexpected url: {}", url),
            }))
        });

        let tmp_dir = tempfile::tempdir().unwrap();

        let state = LiveRepoMetadataStore::new(
            "http://fake-url/rc",
            tmp_dir.path().to_str().unwrap(),
            Rc::new(mock_fetcher),
        );

        let repository = fetch_repository_internal(
            &state,
            &RepositoryConfig {
                name: "test-repo".to_string(),
                source: SourceConfig {
                    endpoint: "http://fake-url".to_string(),
                    kind: "".to_string(),
                    public_pgp_key: None,
                    username: None,
                    password: None,
                },
                destination: DestinationConfig {
                    s3_endpoint: "".to_string(),
                    cloudfront_endpoint: None,
                    s3_bucket: "".to_string(),
                    cloudfront_arn: None,
                    region_name: "".to_string(),
                    access_key_id: "".to_string(),
                    access_key_secret: "".to_string(),
                    path: "".to_string(),
                },
                versions: vec!["focal".into()],
            },
            false,
        )
        .unwrap();

        assert_eq!("test-repo", repository.name);
        assert_eq!(1, repository.collections.len());

        let collection0 = repository.collections.get(0).unwrap();
        assert_eq!(vec!["amd64", "i386"], collection0.target.architectures);
        assert_eq!("focal", collection0.target.release_name);
        assert_eq!(4, collection0.packages.len());
        assert_eq!(9, collection0.indexes.len());

        let mut text = String::new();
        state
            .read("dists/focal/Release")
            .unwrap()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.starts_with("Origin: Artifactory"));

        assert!(state.read("un-existing-file").unwrap().is_none())
    }

    #[test]
    fn load_sample_release() {
        let reader = File::open("samples/debian/Release").unwrap();
        let release = parse_release(&reader, "dists/fake-distro").unwrap();
        assert_eq!("bionic", release.codename);
        assert_eq!(vec!["main"], release.components);
        assert_eq!(vec!["amd64", "i386"], release.architectures);
        assert_eq!(
            vec![
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-amd64/Packages".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                },
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-amd64/Packages.bz2".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                },
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-amd64/Packages.gz".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                },
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-i386/Packages".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                },
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-i386/Packages.bz2".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                },
                IndexFile {
                    file_path: String::new(),
                    path: "dists/fake-distro/main/binary-i386/Packages.gz".to_string(),
                    size: 1085,
                    hash: Hash::Sha256 {
                        hex: "6db5a7a47b02f04f3bbaf39fbdc8e5599c55a082f55270a45ff1a57a43a398a5"
                            .into()
                    },
                    signature: Signature::None,
                }
            ],
            release.indexes
        );
    }

    #[test]
    fn load_sample_packages() {
        let packages = parse_packages(&File::open("samples/debian/Packages").unwrap()).unwrap();
        assert_eq!(2, packages.len());
        assert_eq!(
            vec![
                Package {
                    name: "service-discover-daemon".to_string(),
                    version: "0.1.0-0ubuntu1~".to_string(),
                    architecture: "amd64".to_string(),
                    path: "pool/service-discover-daemon_0.1.0_amd64.deb".to_string(),
                    hash: Hash::Sha256 {
                        hex: "9ed5e5312df1aa047aa64799960b281e56b724bbbb457b5114bde9a829f17af2"
                            .into()
                    },
                    size: 2702470,
                },
                Package {
                    name: "service-discover-agent".to_string(),
                    version: "0.1.0-0ubuntu1~".to_string(),
                    architecture: "amd64".to_string(),
                    path: "pool/service-discover-agent_0.1.0_amd64.deb".to_string(),
                    hash: Hash::Sha256 {
                        hex: "9ed5e5312df1aa047aa64799960b281e56b724bbbb457b5114bde9a829f17af2"
                            .into()
                    },
                    size: 1918012
                }
            ],
            packages,
        );
    }
}
