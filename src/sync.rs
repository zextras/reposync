use crate::config::{Config, GeneralConfig, RepositoryConfig};
use crate::fetcher::Fetcher;
use crate::packages::{Collection, Hash, Package, PackageKey, Repository};
use crate::state::SavedRepoMetadataStore;
use crate::{debian, fetcher};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::stream::StreamExt;

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

/*
/data/ubuntu_rc/bionic/...sasdsasdasd
/data/${repo_name}/XXXXXXXXXXX
*/

trait Uploader {
    fn upload(path: &str, reader: &dyn Read) -> Result<(), std::io::Error>;
}

struct CopyOperation {
    is_replace: bool,
    path: String,
    hash: Hash,
}

struct DeleteOperation {
    path: String,
    hash: Hash,
}

struct SyncManager {}

impl SyncManager {
    pub fn new() -> Self {
        SyncManager {}
    }

    fn lock_repo(&mut self, repo_name: &str) -> () {}

    pub fn load_current(
        &mut self,
        config: &Config,
        repo_config: &RepositoryConfig,
    ) -> Result<(Repository, SavedRepoMetadataStore), std::io::Error> {
        let data_path = format!("{}/{}", config.general.data_path, repo_config.name);

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
        let (repo, store) = debian::load_repository(&data_path, &repo_config)?;
        Ok((repo, store))
    }

    pub fn sync_repo(&mut self, config: &Config, repo_name: &str) -> Result<(), std::io::Error> {
        let fetcher = fetcher::create_chain(&config.general.tmp_path);
        self.sync_repo_internal(fetcher, config, repo_name)
    }

    fn sync_repo_internal(
        &mut self,
        fetcher: Box<dyn Fetcher>,
        config: &Config,
        repo_name: &str,
    ) -> Result<(), std::io::Error> {
        let fetcher: Rc<dyn Fetcher> = Rc::from(fetcher);

        let repo_config = config.repo.iter().find(|x| x.name == repo_name);
        if repo_config.is_none() {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("repository {} not found", repo_name),
            ));
        }
        let repo_config = repo_config.unwrap();

        let lock = self.lock_repo(repo_name);

        let (repo, metadata_store) = debian::fetch_repository(
            fetcher.clone(),
            &format!("{}/tmp_{}/", &config.general.data_path, repo_name),
            &repo_config,
        )?;

        let (current_repo, _) = self.load_current(config, repo_config)?;

        let (packages_copy_list, packages_delete_list, index_copy_list, index_delete_list) =
            SyncManager::repo_diff(&repo, current_repo);

        if packages_copy_list.is_empty() && index_copy_list.is_empty() {
            return Ok(());
        }

        let mut invalidation_paths: Vec<String> = Vec::new();
        invalidation_paths.append(&mut SyncManager::copy(
            &config.general.tmp_path,
            &repo_config.source.endpoint,
            fetcher.borrow(),
            packages_copy_list,
        )?);

        invalidation_paths.append(&mut SyncManager::copy(
            &config.general.tmp_path,
            &repo_config.source.endpoint,
            fetcher.borrow(),
            index_copy_list,
        )?);

        //invalidate cdn: &invalidation_paths

        for operation in packages_delete_list {
            //delete(operation.path)?
        }

        for operation in index_delete_list {
            //delete(operation.path)?
        }

        metadata_store.replace(&format!(
            "{}/{}",
            config.general.data_path, repo_config.name
        ))?;

        Ok(())
    }

    fn copy(
        tmp_path: &str,
        source_endpoint: &str,
        fetcher: &dyn Fetcher,
        copy_list: Vec<CopyOperation>,
    ) -> Result<Vec<String>, std::io::Error> {
        let mut invalidation_paths: Vec<String> = Vec::new();

        for operation in copy_list {
            let mut reader = fetcher
                .fetch(&format!("{}/{}", source_endpoint, operation.path))
                .unwrap();

            let mut tmp_file = tempfile::tempfile_in(tmp_path).unwrap();
            let _ = std::io::copy(&mut reader, &mut tmp_file)?;
            tmp_file.flush()?;
            tmp_file.seek(SeekFrom::Start(0))?;

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

            //uploader.upload(operation.path,&tmp_file);
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
                            }
                        } else {
                            CopyOperation {
                                path: new_package.path.clone(),
                                hash: new_package.hash.clone(),
                                is_replace: false,
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

            index_copy_list.append(
                &mut collection
                    .indexes
                    .iter()
                    //skip unchanged indexes
                    .filter(|x| !current_collection.indexes.contains(x))
                    .map(|new_index| {
                        if let Some(current_index) = current_collection
                            .indexes
                            .iter()
                            .find(|x| new_index.path == x.path)
                        {
                            CopyOperation {
                                path: new_index.path.clone(),
                                hash: new_index.hash.clone(),
                                is_replace: true,
                            }
                        } else {
                            CopyOperation {
                                path: new_index.path.clone(),
                                hash: new_index.hash.clone(),
                                is_replace: false,
                            }
                        }
                    })
                    .collect(),
            );

            index_delete_list.append(
                &mut current_collection
                    .indexes
                    .iter()
                    //skip still valid indexes
                    .filter(|x| !collection.indexes.contains(x))
                    //skip still used paths
                    .filter(|index| {
                        collection
                            .indexes
                            .iter()
                            .find(|x| x.path == index.path)
                            .is_none()
                    })
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
    use crate::fetcher::MockFetcher;
    use crate::packages::Repository;
    use crate::state::{LiveRepoMetadataStore, RepoMetadataStore, SavedRepoMetadataStore};
    use crate::sync::SyncManager;
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
                    username: "".to_string(),
                    password: "".to_string(),
                },
                destination: DestinationConfig {
                    s3: "".to_string(),
                    cdn_arn: "".to_string(),
                    access_key: "".to_string(),
                    secret: "".to_string(),
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

        let mut sync_manager = SyncManager {};
        let (repository, saved_metadata_store) = sync_manager
            .load_current(&config, &config.repo.get(0).unwrap())
            .unwrap();

        assert_eq!("test-ubuntu", repository.name);
        assert_eq!(0, repository.collections.len());
    }

    #[test]
    fn sync_debian_repo() {
        let mut mock_fetcher = MockFetcher::new();

        mock_fetcher.expect_fetch().returning(|url: &str| {
            Result::Ok(Box::new(match url {
                "http://fake-url/rc/dists/focal/Release" => {
                    File::open("samples/debian/Release").unwrap()
                }
                "http://fake-url/rc/main/binary-amd64/Packages" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/main/binary-amd64/Packages.bz2" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/main/binary-amd64/Packages.gz" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/main/binary-i386/Packages" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/main/binary-i386/Packages.bz2" => {
                    File::open("samples/debian/Packages").unwrap()
                }
                "http://fake-url/rc/main/binary-i386/Packages.gz" => {
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

        let mut sync_manager = SyncManager {};
        sync_manager
            .sync_repo_internal(Box::new(mock_fetcher), &config, "test-ubuntu")
            .unwrap();
    }
}
