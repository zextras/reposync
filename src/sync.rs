use crate::config::{Config, RepositoryConfig};
use crate::destination::{create_destination, Destination};
use crate::fetcher::Fetcher;
use crate::locks::Lock;
use crate::packages::{Collection, Hash, IndexFile, Package, Repository};
use crate::state::SavedRepoMetadataStore;
use crate::{debian, fetcher, redhat};
use core::fmt;
#[cfg(test)]
use mockall::automock;
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt::Formatter;
use std::fs::File;
use std::io::{Error, ErrorKind, Seek, SeekFrom, Write};
use std::ops::{Add, Sub};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

/*
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

#[derive(Clone, Eq, PartialEq)]
struct CopyOperation {
    is_replace: bool,
    path: String,
    hash: Hash,
    size: u64,
    local_file: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
struct DeleteOperation {
    path: String,
}

#[derive(Clone)]
pub enum RepoStatus {
    Syncing,
    Waiting,
}

impl fmt::Display for RepoStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            RepoStatus::Syncing => write!(f, "syncing"),
            RepoStatus::Waiting => write!(f, "waiting"),
        }
    }
}

#[derive(Clone)]
pub struct SyncStatus {
    pub current: RepoStatus,
    pub next_sync: SystemTime,
    pub last_sync: SystemTime,
    pub last_result: Option<String>,
}

#[cfg_attr(test, automock)]
pub trait TimeProvider: Send + Sync {
    fn now(&self) -> SystemTime;
}

pub struct RealTimeProvider {}
impl TimeProvider for RealTimeProvider {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub struct SyncManager {
    config: Config,
    lock: Lock,
    time_provider: Arc<dyn TimeProvider>,
    sync_map: Arc<Mutex<BTreeMap<String, SyncStatus>>>,
}

impl SyncManager {
    pub fn new(config: Config) -> Self {
        Self::new_internal(config, Lock::new(), Arc::new(RealTimeProvider {}))
    }

    fn new_internal(config: Config, lock: Lock, time_provider: Arc<dyn TimeProvider>) -> Self {
        let mut map = BTreeMap::new();
        config.repo.iter().for_each(|r| {
            map.insert(
                r.name.clone(),
                SyncStatus {
                    current: RepoStatus::Waiting,
                    next_sync: time_provider.now().add(Duration::from_secs(
                        config.general.max_sync_delay as u64 * 60,
                    )),
                    last_sync: SystemTime::UNIX_EPOCH,
                    last_result: None,
                },
            );
        });
        SyncManager {
            config,
            lock,
            time_provider,
            sync_map: Arc::new(Mutex::new(map)),
        }
    }

    pub fn start_scheduler(self: Arc<Self>) {
        thread::spawn(move || loop {
            let now = self.time_provider.now();
            if let Some((name, time)) = self.next_repo_to_sync() {
                if let Ok(sleep_time) = time.duration_since(now) {
                    thread::sleep(sleep_time.min(Duration::from_secs(10)));
                } else {
                    //negative time
                    let result = self.sync_repo(&name);
                    if let Err(err) = result {
                        println!("failed to synchronize {}: {}", &name, &err.to_string());
                        self.sync_completed(&name, &err.to_string());
                    } else {
                        println!("{} fully synchronized", &name);
                        self.sync_completed(&name, "successful");
                    }
                }
            } else {
                thread::sleep(Duration::from_secs(10));
            }
        });
    }

    ///returns true if all paths in the configuration are accessible
    pub fn check_permissions(&self) -> Result<(), std::io::Error> {
        Self::check_writable(&self.config.general.data_path)?;
        Self::check_writable(&self.config.general.tmp_path)?;
        Ok(())
    }

    fn check_writable(path: &str) -> Result<(), Error> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("expected directory, found file instead '{}'", path),
            ));
        }
        if metadata.permissions().readonly() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("expected write access, found read-only for '{}'", path),
            ));
        }
        Ok(())
    }

    pub fn queue_sync(&self, repo_name: &str) {
        let now = self.time_provider.now();
        let mut map = self.sync_map.lock().unwrap();
        //set next_sync
        if let Some(status) = map.get_mut(repo_name) {
            let tmp_next_sync = now
                .add(Duration::from_secs(
                    self.config.general.min_sync_delay as u64 * 60,
                ))
                .sub(now.duration_since(status.last_sync).unwrap());
            status.next_sync = tmp_next_sync.min(status.next_sync);
        }
    }

    fn sync_completed(&self, repo_name: &str, result: &str) {
        let now = self.time_provider.now();
        let mut map = self.sync_map.lock().unwrap();
        //set next_sync
        if let Some(status) = map.get_mut(repo_name) {
            status.last_sync = now;
            status.next_sync = now.add(Duration::from_secs(
                self.config.general.max_sync_delay as u64 * 60,
            ));
            status.last_result = Some(result.into());
        }
    }

    pub fn next_repo_to_sync(&self) -> Option<(String, SystemTime)> {
        let map = self.sync_map.lock().unwrap();
        let mut closer = None;

        for (key, value) in &*map {
            if let Some((name, next_sync)) = closer {
                if value.next_sync < next_sync {
                    closer = Some((key.clone(), value.next_sync.clone()));
                } else {
                    closer = Some((name, next_sync));
                }
            } else {
                closer = Some((key.clone(), value.next_sync.clone()));
            }
        }

        closer
    }

    fn _repo_status(&self, repo_name: &str) -> RepoStatus {
        if self.lock.is_repo_syncing(repo_name) {
            RepoStatus::Syncing
        } else {
            RepoStatus::Waiting
        }
    }

    pub fn get_status(&self, repo_name: &str) -> Option<SyncStatus> {
        let map = self.sync_map.lock().unwrap();
        if let Some(status) = map.get(repo_name) {
            let mut new_status = status.clone();
            new_status.current = self._repo_status(repo_name);
            Some(new_status)
        } else {
            None
        }
    }

    pub fn load_current_by_name(
        &self,
        repo_name: &str,
    ) -> Result<Option<(Repository, SavedRepoMetadataStore)>, std::io::Error> {
        let repo_config = self.get_repo_config(repo_name);
        if let Some(repo_config) = repo_config {
            let result = self.load_current(repo_config);
            if let Ok(result) = result {
                Ok(Some(result))
            } else {
                Err(result.err().unwrap())
            }
        } else {
            Ok(None)
        }
    }

    fn get_repo_config(&self, repo_name: &str) -> Option<&RepositoryConfig> {
        self.config.repo.iter().find(|x| x.name == repo_name)
    }

    pub fn load_current(
        &self,
        repo_config: &RepositoryConfig,
    ) -> Result<(Repository, SavedRepoMetadataStore), std::io::Error> {
        let _write_lock = self.lock.lock_write(&repo_config.name);
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
        println!("starting synchronization of {}", repo_name);
        let repo_config = self.get_repo_config(repo_name);
        if repo_config.is_none() {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                format!("repository {} not found", repo_name),
            ));
        }
        let repo_config = repo_config.unwrap();

        let fetcher = fetcher::create_chain(
            self.config.general.max_retries,
            Duration::from_secs(self.config.general.retry_sleep),
            repo_config
                .source
                .get_authorization_secret()
                .expect("cannot read authorization secret"),
            Duration::from_secs(self.config.general.timeout as u64),
        )?;

        let mut destination = create_destination(&self.config.general, &repo_config.destination)?;

        return if let Some(_lock) = self.lock.lock_sync(&repo_config.name) {
            self.sync_repo_internal(fetcher, destination.as_mut(), repo_config)
        } else {
            Result::Err(std::io::Error::new(
                ErrorKind::WouldBlock,
                "sync already in progress",
            ))
        };
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
            println!("no public pgp key provided, skipping metadata signature validation")
        }

        let (current_repo, _) = self.load_current(repo_config)?;

        let (packages_copy_list, packages_delete_list, index_copy_list, index_delete_list) =
            SyncManager::repo_diff(&repo, current_repo);

        if packages_copy_list.is_empty() && index_copy_list.is_empty() {
            return Ok(());
        }

        println!(
            "{} packages and {} indexes to copy or update for a total of {:.2} MB.",
            packages_copy_list.len(),
            index_copy_list.len(),
            (packages_copy_list.iter().fold(0, |a, p| a + p.size)
                + index_copy_list.iter().fold(0, |a, p| a + p.size)) as f64
                / (1024f64 * 1024f64)
        );

        println!(
            "{} packages and {} indexes to delete.",
            packages_delete_list.len(),
            index_delete_list.len()
        );

        println!("sync operation is atomic, either it's fully completed or will be performed from scratch");

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

        let _write_lock = self.lock.lock_write(&repo_config.name);
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
                    format!("failed hash validation for '{}'", operation.path),
                ));
            }

            let tmp_file_size = tmp_file.metadata()?.len();
            if operation.size != tmp_file_size {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "invalid file size for '{}', expected {} found {}",
                        operation.path, operation.size, tmp_file_size
                    ),
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
                                size: new_package.size,
                            }
                        } else {
                            CopyOperation {
                                path: new_package.path.clone(),
                                hash: new_package.hash.clone(),
                                is_replace: false,
                                local_file: None,
                                size: new_package.size,
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
                    .map(|(_key, current_package)| DeleteOperation {
                        path: current_package.path.clone(),
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
                                size: new_index.size,
                            }
                        } else {
                            CopyOperation {
                                path: new_index.path.clone(),
                                hash: new_index.hash.clone(),
                                is_replace: false,
                                local_file: Some(new_index.file_path.clone()),
                                size: new_index.size,
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
                    })
                    .collect(),
            );
        }

        (
            SyncManager::deduplicate_list(packages_copy_list),
            SyncManager::deduplicate_list(packages_delete_list),
            SyncManager::deduplicate_list(index_copy_list),
            SyncManager::deduplicate_list(index_delete_list),
        )
    }

    fn deduplicate_list<T>(list: Vec<T>) -> Vec<T>
    where
        T: Eq + Clone,
    {
        let mut tmp: Vec<T> = Vec::new();
        list.into_iter().for_each(|x| {
            if !tmp.contains(&x) {
                tmp.push(x);
            }
        });
        tmp
    }
}

#[cfg(test)]
pub mod tests {
    use crate::config::{Config, DestinationConfig, GeneralConfig, RepositoryConfig, SourceConfig};
    use crate::destination::MemoryDestination;
    use crate::fetcher::MockFetcher;
    use crate::sync::{Lock, MockTimeProvider, RealTimeProvider, SyncManager};
    use std::fs::File;
    use std::ops::Add;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    fn create_config(tmp_dir: &TempDir) -> Config {
        let config = Config {
            general: GeneralConfig {
                data_path: format!("{}/data", tmp_dir.path().to_str().unwrap()),
                tmp_path: format!("{}/tmp", tmp_dir.path().to_str().unwrap()),
                bind_address: "".to_string(),
                timeout: 0,
                max_retries: 0,
                retry_sleep: 0,
                min_sync_delay: 10,
                max_sync_delay: 30,
            },
            repo: vec![RepositoryConfig {
                name: "test-ubuntu".to_string(),
                source: SourceConfig {
                    endpoint: "http://fake-url/rc".to_string(),
                    kind: "debian".to_string(),
                    public_pgp_key: None,
                    username: None,
                    password: None,
                    authorization_file: None,
                },
                destination: DestinationConfig {
                    s3: None,
                    local: None,
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

        let sync_manager = SyncManager {
            config: config.clone(),
            lock: Lock::new(),
            time_provider: Arc::new(RealTimeProvider {}),
            sync_map: Arc::new(Mutex::new(Default::default())),
        };
        let (repository, _saved_metadata_store) = sync_manager
            .load_current(&config.repo.get(0).unwrap())
            .unwrap();

        assert_eq!("test-ubuntu", repository.name);
        assert_eq!(0, repository.collections.len());
    }

    #[test]
    fn sync_debian_repo_from_scratch() {
        let mut mock_fetcher = MockFetcher::new();

        setup_fetcher(
            &mut mock_fetcher,
            "samples/debian/Release",
            "samples/debian/Packages",
        );

        let tmp_dir = tempfile::tempdir().unwrap();
        let config = create_config(&tmp_dir);

        let repo_config = config.repo.get(0).unwrap();
        let mut destination: MemoryDestination = MemoryDestination::new("ubuntu");

        let sync_manager = SyncManager {
            config: config.clone(),
            lock: Lock::new(),
            sync_map: Arc::new(Mutex::new(Default::default())),
            time_provider: Arc::new(RealTimeProvider {}),
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
            14,
            contents
                .get("ubuntu/dists/focal/Release.gpg")
                .unwrap()
                .len()
        );
        assert_eq!(
            1868,
            contents.get("ubuntu/dists/focal/InRelease").unwrap().len()
        );
        assert_eq!(
            1075,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages")
                .unwrap()
                .len()
        );
        assert_eq!(
            1075,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages.bz2")
                .unwrap()
                .len()
        );
        assert_eq!(
            1075,
            contents
                .get("ubuntu/dists/focal/main/binary-amd64/Packages.gz")
                .unwrap()
                .len()
        );
        assert_eq!(
            1075,
            contents
                .get("ubuntu/dists/focal/main/binary-i386/Packages")
                .unwrap()
                .len()
        );
        assert_eq!(
            1075,
            contents
                .get("ubuntu/dists/focal/main/binary-i386/Packages.bz2")
                .unwrap()
                .len()
        );
        assert_eq!(
            1075,
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

        let mut mock_fetcher = MockFetcher::new();
        setup_fetcher(
            &mut mock_fetcher,
            "samples/debian/Release.2",
            "samples/debian/Packages.2",
        );
        let mut destination: MemoryDestination = MemoryDestination::new("ubuntu");
        sync_manager
            .sync_repo_internal(Box::new(mock_fetcher), &mut destination, repo_config)
            .unwrap();

        destination.print();

        let (contents, deletions, invalidations) = destination.explode();
        assert_eq!(8, contents.len());
        assert_eq!(1, deletions.len());
        assert!(deletions.contains("ubuntu/pool/service-discover-agent_0.1.0_amd64.deb"));
        assert_eq!(8, invalidations.len());
        assert!(invalidations.contains("ubuntu/dists/focal/Release"));
        assert!(!invalidations.contains("ubuntu/dists/focal/Release.gpg"));
        assert!(invalidations.contains("ubuntu/dists/focal/InRelease"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-amd64/Packages"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-amd64/Packages.gz"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-amd64/Packages.bz2"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-i386/Packages"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-i386/Packages.gz"));
        assert!(invalidations.contains("ubuntu/dists/focal/main/binary-i386/Packages.bz2"));
    }

    fn setup_fetcher(mock_fetcher: &mut MockFetcher, release: &str, packages: &str) {
        let packages: String = packages.into();
        let release: String = release.into();
        mock_fetcher.expect_fetch().returning(move |url: &str| {
            Result::Ok(Box::new(match url {
                "http://fake-url/rc/dists/focal/Release" => File::open(&release).unwrap(),
                "http://fake-url/rc/dists/focal/Release.gpg" => {
                    File::open("samples/fake-signature").unwrap()
                }
                "http://fake-url/rc/dists/focal/InRelease" => File::open(&release).unwrap(),
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages" => {
                    File::open(&packages).unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages.bz2" => {
                    File::open(&packages).unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-amd64/Packages.gz" => {
                    File::open(&packages).unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages" => {
                    File::open(&packages).unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages.bz2" => {
                    File::open(&packages).unwrap()
                }
                "http://fake-url/rc/dists/focal/main/binary-i386/Packages.gz" => {
                    File::open(&packages).unwrap()
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
    }

    #[test]
    fn scheduler() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let config = create_config(&tmp_dir);
        let secs_offset = Arc::new(AtomicU64::new(0));

        let mut mock = MockTimeProvider::new();
        {
            let secs_offset = secs_offset.clone();
            mock.expect_now().returning(move || {
                UNIX_EPOCH.add(Duration::from_secs(secs_offset.load(Ordering::SeqCst)))
            });
        }

        let sync_manager = SyncManager::new_internal(config.clone(), Lock::new(), Arc::new(mock));

        secs_offset.store(0, Ordering::SeqCst);
        {
            let (next_name, next_time) = sync_manager.next_repo_to_sync().unwrap();
            assert_eq!("test-ubuntu", next_name);
            assert_eq!(UNIX_EPOCH.add(Duration::from_secs(30 * 60)), next_time);
        }
        {
            sync_manager.queue_sync("test-ubuntu");
            let (next_name, next_time) = sync_manager.next_repo_to_sync().unwrap();
            assert_eq!("test-ubuntu", next_name);
            assert_eq!(UNIX_EPOCH.add(Duration::from_secs(10 * 60)), next_time);
        }
        secs_offset.store(60, Ordering::SeqCst);
        {
            sync_manager.sync_completed("test-ubuntu", "success");
            let (next_name, next_time) = sync_manager.next_repo_to_sync().unwrap();
            assert_eq!("test-ubuntu", next_name);
            assert_eq!(UNIX_EPOCH.add(Duration::from_secs(31 * 60)), next_time);
        }
        {
            sync_manager.queue_sync("test-ubuntu");
            let (next_name, next_time) = sync_manager.next_repo_to_sync().unwrap();
            assert_eq!("test-ubuntu", next_name);
            assert_eq!(UNIX_EPOCH.add(Duration::from_secs(11 * 60)), next_time);
        }
    }
}
