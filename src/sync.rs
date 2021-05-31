use crate::config::{Config, GeneralConfig, RepositoryConfig};
use crate::destination::{Destination, S3Destination};
use crate::fetcher::Fetcher;
use crate::packages::{Collection, Hash, IndexFile, Package, PackageKey, Repository};
use crate::state::SavedRepoMetadataStore;
use crate::{debian, fetcher, redhat};
use std::borrow::{Borrow, BorrowMut};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::iter::Chain;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/**
Steps:
 - lock repository
 - create temp dir for metadata
 - fetch & store metadata
 - check metadata against "current", product 2 sets of "copy" and "delete" operation lists.
   the first set is for packages, the second for metadata.
 - execute "copy" operations for packages
 -- BEGIN POSSIBLE INCONSISTENCIES
 - execute "copy" operations for metadata
 - send CDN cache invalidation
 -- END POSSIBLE INCONSISTENCIES
 - execute "delete" operations for packages and metadata of all collection
 - overwrite metadata state to the "current"
 - unlock
*/

trait Uploader {
    fn upload(path: &str, reader: &dyn Read) -> Result<(), std::io::Error>;
}

struct CopyOperation {
    is_replace: bool,
    path: String,
    hash: Hash,
    local_file: Option<String>,
}

struct DeleteOperation {
    path: String,
    hash: Hash,
}

struct Lock {}

impl Lock {
    fn lock_repo(&self, repo_name: &str) -> () {}
}

pub struct SyncManager {
    config: Config,
    lock: Lock,
}

impl SyncManager {
    pub fn new(config: Config) -> Self {
        SyncManager {
            config,
            lock: Lock {},
        }
    }

    pub fn load_current(
        &self,
        repo_config: &RepositoryConfig,
    ) -> Result<(Repository, SavedRepoMetadataStore), std::io::Error> {
        let data_path = format!("{}/{}", self.config.general.data_path, repo_config.name);

        let result = File::open(&data_path);
        if result.is_err() {
            return Ok((
                Repository {
                    name: repo_config.name.clone(),
                    collections: vec![],
                },
                SavedRepoMetadataStore::new(&data_path),
            ));
        }

        match repo_config.source.kind.as_str() {
            "debian" => {
                let (repo, store) = debian::load_repository(&data_path, &repo_config)?;
                Ok((repo, store))
            }

            "redhat" => {
                let (repo, store) = redhat::load_repository(&data_path, &repo_config)?;
                Ok((repo, store))
            }

            _ => panic!("unknown repo of type {}", &repo_config.source.kind),
        }
    }

    pub fn sync_repo(&self, repo_name: &str) -> Result<(), std::io::Error> {
        let repo_config = self.config.repo.iter().find(|x| x.name == repo_name);
        if repo_config.is_none() {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("repository {} not found", repo_name),
            ));
        }
        let repo_config = repo_config.unwrap();

        let fetcher = fetcher::create_chain(
            3,
            2000,
            repo_config.source.username.clone(),
            repo_config.source.password.clone(),
        )?;
        let mut destination = S3Destination::new(
            &repo_config.destination.path,
            &repo_config.destination.s3_endpoint,
            &repo_config.destination.s3_bucket,
            repo_config.destination.cloudfront_endpoint.clone(),
            repo_config.destination.cloudfront_arn.clone(),
            &repo_config.destination.region_name,
            &repo_config.destination.access_key_id,
            &repo_config.destination.access_key_secret,
        );

        let lock = self.lock.lock_repo(&repo_config.name);
        self.sync_repo_internal(fetcher, &mut destination, repo_config)
    }

    fn sync_repo_internal(
        &self,
        fetcher: Box<dyn Fetcher>,
        destination: &mut dyn Destination,
        repo_config: &RepositoryConfig,
    ) -> Result<(), std::io::Error> {
        let fetcher: Rc<dyn Fetcher> = Rc::from(fetcher);

        let (repo, metadata_store) = match repo_config.source.kind.as_str() {
            "debian" => debian::fetch_repository(
                fetcher.clone(),
                &format!(
                    "{}/tmp_{}/",
                    &self.config.general.data_path, &repo_config.name
                ),
                &repo_config,
            )?,

            "redhat" => redhat::fetch_repository(
                fetcher.clone(),
                &format!(
                    "{}/tmp_{}/",
                    &self.config.general.data_path, repo_config.name
                ),
                &repo_config,
            )?,

            _ => panic!("unknown repo of type {}", &repo_config.source.kind),
        };

        let public_key = repo_config.source.parse_public_key()?;
        if let Some(public_key) = public_key {
            for index in repo.collections.iter().map(|c| &c.indexes).flatten() {
                let mut reader = File::open(&index.file_path).expect("cannot open stored index");
                let result = index.signature.matches(&public_key, &mut reader);
                if result.is_err() {
                    let err = result.err().unwrap();
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "cannot validate signature of '{}': {}",
                            &index.path,
                            err.to_string()
                        ),
                    ));
                }
            }
        } else {
            println!("skipping metadata signature validation")
        }

        let (current_repo, _) = self.load_current(repo_config)?;

        let (packages_copy_list, packages_delete_list, index_copy_list, index_delete_list) =
            SyncManager::repo_diff(&repo, current_repo);

        if packages_copy_list.is_empty() && index_copy_list.is_empty() {
            return Ok(());
        }

        let mut invalidation_paths: Vec<String> = Vec::new();
        invalidation_paths.append(&mut SyncManager::copy(
            &self.config.general.tmp_path,
            &repo_config.source.endpoint,
            fetcher.borrow(),
            destination,
            packages_copy_list,
        )?);

        invalidation_paths.append(&mut SyncManager::copy(
            &self.config.general.tmp_path,
            &repo_config.source.endpoint,
            fetcher.borrow(),
            destination,
            index_copy_list,
        )?);

        destination.invalidate(invalidation_paths)?;

        for operation in packages_delete_list {
            destination.delete(&operation.path)?;
        }

        for operation in index_delete_list {
            destination.delete(&operation.path)?;
        }

        metadata_store.replace(&format!(
            "{}/{}",
            self.config.general.data_path, repo_config.name
        ))?;

        Ok(())
    }

    fn copy(
        tmp_path: &str,
        source_endpoint: &str,
        fetcher: &dyn Fetcher,
        destination: &mut dyn Destination,
        copy_list: Vec<CopyOperation>,
    ) -> Result<Vec<String>, std::io::Error> {
        let result =
            SyncManager::copy_internal(tmp_path, source_endpoint, fetcher, destination, copy_list);
        if result.is_err() {
            let err = result.err().unwrap();
            return Err(std::io::Error::new(
                err.kind(),
                format!(
                    "failed to copy {} to {}: {}",
                    source_endpoint,
                    destination.name(),
                    &err.to_string()
                ),
            ));
        }
        result
    }

    fn copy_internal(
        tmp_path: &str,
        source_endpoint: &str,
        fetcher: &dyn Fetcher,
        destination: &mut dyn Destination,
        copy_list: Vec<CopyOperation>,
    ) -> Result<Vec<String>, std::io::Error> {
        let mut invalidation_paths: Vec<String> = Vec::new();
        std::fs::create_dir_all(tmp_path).expect("unable to create tmp_path");

        for operation in copy_list {
            let mut tmp_file;
            if operation.local_file.is_some() {
                let result = File::open(operation.local_file.clone().unwrap());
                if let Err(err) = result {
                    return Err(std::io::Error::new(
                        err.kind(),
                        format!(
                            "cannot copy file '{}': {}",
                            &operation.local_file.clone().unwrap(),
                            err.to_string()
                        ),
                    ));
                }
                tmp_file = result.unwrap();
            } else {
                let fetch_result =
                    fetcher.fetch(&format!("{}/{}", source_endpoint, operation.path));
                if fetch_result.is_err() {
                    return Err(std::io::Error::new(
                        ErrorKind::Other,
                        format!(
                            "cannot copy file '{}': {}",
                            operation.path,
                            fetch_result.err().unwrap().error
                        ),
                    ));
                }
                let mut reader = fetch_result.unwrap();
                tmp_file = tempfile::tempfile_in(tmp_path).expect("cannot create tmp file");
                let _ = std::io::copy(&mut reader, &mut tmp_file)?;
                tmp_file.flush()?;
                tmp_file.seek(SeekFrom::Start(0))?;
            }

            if !operation.hash.matches(&mut tmp_file)? {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!("failed hash validation for {}", operation.path),
                ));
            }

            tmp_file.seek(SeekFrom::Start(0))?;
            if operation.is_replace {
                invalidation_paths.push(operation.path.clone());
            }

            destination.upload(&operation.path, tmp_file)?;
        }

        Ok(invalidation_paths)
    }

    fn repo_diff(
        repo: &Repository,
        current_repo: Repository,
    ) -> (
        Vec<CopyOperation>,
        Vec<DeleteOperation>,
        Vec<CopyOperation>,
        Vec<DeleteOperation>,
    ) {
        let mut packages_copy_list: Vec<CopyOperation> = Vec::new();
        let mut packages_delete_list: Vec<DeleteOperation> = Vec::new();

        let mut index_copy_list: Vec<CopyOperation> = Vec::new();
        let mut index_delete_list: Vec<DeleteOperation> = Vec::new();

        for collection in &repo.collections {
            let empty_collection = Collection::empty(&collection.target);
            let current_collection = current_repo
                .collections
                .iter()
                .find(|x| x.target == collection.target)
                .unwrap_or(&empty_collection);

            let new_packages: BTreeMap<String, &Package> = collection
                .packages
                .iter()
                .map(|x| (x.path.clone(), x))
                .collect();

            let current_packages: BTreeMap<String, &Package> = current_collection
                .packages
                .iter()
                .map(|x| (x.path.clone(), x))
                .collect();

            packages_copy_list.append(
                &mut new_packages
                    .iter()
                    .filter(|&(key, new_package)| {
                        if let Some(current_package) = current_packages.get(key) {
                            //updated package or same old?
                            current_package != new_package
                        } else {
                            //brand new package/version
                            true
                        }
                    })
                    .map(|(key, new_package)| {
                        if current_packages.contains_key(key) {
                            CopyOperation {
                                path: new_package.path.clone(),
                                hash: new_package.hash.clone(),
                                is_replace: true,
                                local_file: None,
                            }
                        } else {
                            CopyOperation {
                                path: new_package.path.clone(),
                                hash: new_package.hash.clone(),
                                is_replace: false,
                                local_file: None,
                            }
                        }
                    })
                    .collect(),
            );

            packages_delete_list.append(
                &mut current_packages
                    .iter()
                    //skip every path still in use
                    .filter(|&(key, _)| !new_packages.contains_key(key))
                    .map(|(key, current_package)| DeleteOperation {
                        path: current_package.path.clone(),
                        hash: current_package.hash.clone(),
                    })
                    .collect(),
            );

            let new_indexes: BTreeMap<String, IndexFile> = collection
                .indexes
                .iter()
                .map(|x| (x.path.clone(), x.clone()))
                .collect();

            let current_indexes: BTreeMap<String, IndexFile> = current_collection
                .indexes
                .iter()
                .map(|x| (x.path.clone(), x.clone()))
                .collect();

            index_copy_list.append(
                &mut collection
                    .indexes
                    .iter()
                    //skip unchanged indexes
                    .filter(|new_index| {
                        if let Some(current_index) = current_indexes.get(&new_index.path) {
                            //updated index
                            !new_index.same_content(current_index)
                        } else {
                            //brand new
                            true
                        }
                    })
                    .map(|new_index| {
                        if current_indexes.contains_key(&new_index.path) {
                            CopyOperation {
                                path: new_index.path.clone(),
                                hash: new_index.hash.clone(),
                                is_replace: true,
                                local_file: Some(new_index.file_path.clone()),
                            }
                        } else {
                            CopyOperation {
                                path: new_index.path.clone(),
                                hash: new_index.hash.clone(),
                                is_replace: false,
                                local_file: Some(new_index.file_path.clone()),
                            }
                        }
                    })
                    .collect(),
            );

            index_delete_list.append(
                &mut current_collection
                    .indexes
                    .iter()
                    //skip still used paths
                    .filter(|x| !new_indexes.contains_key(&x.path))
                    .map(|x| DeleteOperation {
                        path: x.path.clone(),
                        hash: x.hash.clone(),
                    })
                    .collect(),
            );
        }

        (
            packages_copy_list,
            packages_delete_list,
            index_copy_list,
            index_delete_list,
        )
    }
}

#[cfg(test)]
pub mod tests {
    use crate::config::{Config, DestinationConfig, GeneralConfig, RepositoryConfig, SourceConfig};
    use crate::destination::MemoryDestination;
    use crate::fetcher::MockFetcher;
    use crate::packages::Repository;
    use crate::state::{LiveRepoMetadataStore, RepoMetadataStore, SavedRepoMetadataStore};
    use crate::sync::{Lock, SyncManager};
    use mockall::predicate;
    use std::fs::File;
    use std::rc::Rc;
    use tempfile::TempDir;

    fn create_config(tmp_dir: &TempDir) -> Config {
        let config = Config {
            general: GeneralConfig {
                data_path: format!("{}/data", tmp_dir.path().to_str().unwrap()),
                tmp_path: format!("{}/tmp", tmp_dir.path().to_str().unwrap()),
                timeout: 0,
                debounce: 0,
                auto_align: 0,
            },
            repo: vec![RepositoryConfig {
                name: "test-ubuntu".to_string(),
                source: SourceConfig {
                    endpoint: "http://fake-url/rc".to_string(),
                    kind: "debian".to_string(),
                    public_pgp_key: None,
                    username: None,
                    password: None,
                },
                destination: DestinationConfig {
                    s3_endpoint: "".to_string(),
                    s3_bucket: "".to_string(),
                    cloudfront_arn: None,
                    cloudfront_endpoint: None,
                    access_key_id: "".to_string(),
                    access_key_secret: "".to_string(),
                    region_name: "".to_string(),
                    path: "ubuntu/".to_string(),
                },
                versions: vec!["focal".into()],
            }],
        };

        std::fs::create_dir_all(&config.general.data_path).unwrap();
        std::fs::create_dir_all(&config.general.tmp_path).unwrap();

        config
    }

    #[test]
    fn load_current_returns_empty_when_missing() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = create_config(&tmp_dir);

        let mut sync_manager = SyncManager {
            config: config.clone(),
            lock: Lock {},
        };
        let (repository, saved_metadata_store) = sync_manager
            .load_current(&config.repo.get(0).unwrap())
            .unwrap();

        assert_eq!("test-ubuntu", repository.name);
        assert_eq!(0, repository.collections.len());
    }

    #[test]
    fn sync_debian_repo_from_scratch() {
        let mut mock_fetcher = MockFetcher::new();

        mock_fetcher.expect_fetch().returning(|url: &str| {
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
                "http://fake-url/rc/pool/service-discover-agent_0.1.0_amd64.deb" => {
                    File::open("samples/fake-package").unwrap()
                }
                "http://fake-url/rc/pool/service-discover-daemon_0.1.0_amd64.deb" => {
                    File::open("samples/fake-package").unwrap()
                }
                _ => panic!("unexpected url: {}", url),
            }))
        });

        let tmp_dir = tempfile::tempdir().unwrap();
        let config = create_config(&tmp_dir);

        let repo_config = config.repo.get(0).unwrap();
        let mut destination = MemoryDestination::new("ubuntu");

        let mut sync_manager = SyncManager {
            config: config.clone(),
            lock: Lock {},
        };
        sync_manager
            .sync_repo_internal(Box::new(mock_fetcher), &mut destination, repo_config)
            .unwrap();

        destination.print();

        let (contents, deletions, invalidations) = destination.explode();

        assert_eq!(
            1868,
            contents.get("ubuntu/dists/focal/Release").unwrap().len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages")
                .unwrap()
                .len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages.bz2")
                .unwrap()
                .len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages.gz")
                .unwrap()
                .len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-i386/Packages")
                .unwrap()
                .len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-i386/Packages.bz2")
                .unwrap()
                .len()
        );
        assert_eq!(
            1085,
            contents
                .get("ubuntu/dists/focal/main/binary-i386/Packages.gz")
                .unwrap()
                .len()
        );
        assert_eq!(
            20,
            contents
                .get("ubuntu/pool/service-discover-agent_0.1.0_amd64.deb")
                .unwrap()
                .len()
        );
        assert_eq!(
            20,
            contents
                .get("ubuntu/pool/service-discover-daemon_0.1.0_amd64.deb")
                .unwrap()
                .len()
        );

        assert_eq!(0, deletions.len());
        assert_eq!(0, invalidations.len());
    }
}
