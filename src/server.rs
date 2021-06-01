use async_trait::async_trait;
use reposync_lib::server::MakeService;
use reposync_lib::{
    Api, HealthGetResponse, RepositoryRepoGetResponse, RepositoryRepoSyncPostResponse,
};
use std::marker::PhantomData;
use std::sync::Arc;
use swagger::auth::MakeAllowAllAuthenticator;
use swagger::ApiError;
use swagger::EmptyContext;
use swagger::{Has, XSpanIdString};

use crate::sync::SyncManager;
use reposync_lib::models::Status;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn create(sync_manager: SyncManager, addr: &str) -> hyper::Result<()> {
    let addr = addr.parse().expect("Failed to parse bind address");
    let server = Server::new(sync_manager);
    server.start_scheduler();

    let service = MakeService::new(server);
    let service = MakeAllowAllAuthenticator::new(service, "cosmo");
    let service = reposync_lib::server::context::MakeAddContext::<_, EmptyContext>::new(service);

    hyper::server::Server::bind(&addr).serve(service).await
}

#[derive(Clone)]
pub struct Server<C> {
    marker: PhantomData<C>,
    sync_manager: Arc<SyncManager>,
}

fn normalize(system_time: &SystemTime) -> i64 {
    system_time.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

impl<C> Server<C> {
    pub fn new(sync_manager: SyncManager) -> Self {
        Server {
            marker: PhantomData,
            sync_manager: Arc::new(sync_manager),
        }
    }

    pub fn start_scheduler(&self) {
        SyncManager::start_scheduler(self.sync_manager.clone());
    }

    ///return None when repo is not found
    fn get_repo_status(&self, repo_name: &str) -> Option<Status> {
        let result = self.sync_manager.load_current_by_name(&repo_name);
        if let Ok(Some(result)) = result {
            let (repo, _metadata) = result;
            if let Some(sync_state) = self.sync_manager.get_status(&repo.name) {
                Some(Status {
                    status: sync_state.current.to_string(),
                    next_sync: normalize(&sync_state.next_sync),
                    last_sync: normalize(&sync_state.last_sync),
                    last_result: sync_state.last_result.unwrap_or("".into()),
                    name: repo.name.clone(),
                    size: repo.size() as i64,
                    packages: repo.count_packages() as isize,
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[async_trait]
impl<C> Api<C> for Server<C>
where
    C: Has<XSpanIdString> + Send + Sync,
{
    /// Simple health-check
    async fn health_get(&self, _context: &C) -> Result<HealthGetResponse, ApiError> {
        if let Err(err) = self.sync_manager.check_permissions() {
            println!("health-check failed: {}", err.to_string());
            Ok(HealthGetResponse::ServiceUnavailableWhenServiceHasKnownIssues)
        } else {
            Ok(HealthGetResponse::EmptyResponseWhenEverythingIsOk)
        }
    }

    /// status of repository
    async fn repository_repo_get(
        &self,
        repo: String,
        _context: &C,
    ) -> Result<RepositoryRepoGetResponse, ApiError> {
        if let Some(status) = self.get_repo_status(&repo) {
            Ok(reposync_lib::RepositoryRepoGetResponse::TheStatusOfTheRepository { 0: status })
        } else {
            Ok(reposync_lib::RepositoryRepoGetResponse::RepositoryNotFound {})
        }
    }

    /// Perform a synchronization
    async fn repository_repo_sync_post(
        &self,
        repo: String,
        _context: &C,
    ) -> Result<RepositoryRepoSyncPostResponse, ApiError> {
        self.sync_manager.queue_sync(&repo);
        if let Some(status) = self.get_repo_status(&repo) {
            Ok(
                reposync_lib::RepositoryRepoSyncPostResponse::TheSynchronizationHasBeenQueuedCorrectly {
                    0: status,
                },
            )
        } else {
            Ok(reposync_lib::RepositoryRepoSyncPostResponse::RepositoryNotFound {})
        }
    }
}
