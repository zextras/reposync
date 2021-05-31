use async_trait::async_trait;
use futures::{future, Stream, StreamExt, TryFutureExt, TryStreamExt};
use hyper::server::conn::Http;
use hyper::service::Service;
use log::info;
use reposync_lib::server::MakeService;
use reposync_lib::{Api, HealthGetResponse, RepoRepoGetResponse, RepoRepoSyncPostResponse};
use std::error::Error;
use std::future::Future;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use swagger::auth::MakeAllowAllAuthenticator;
use swagger::ApiError;
use swagger::EmptyContext;
use swagger::{Has, XSpanIdString};

use crate::config::Config;
use crate::sync::SyncManager;
use reposync_lib::models;

pub async fn create(sync_manager: SyncManager, addr: &str) {
    let addr = addr.parse().expect("Failed to parse bind address");
    let server = Server::new(sync_manager);
    let service = MakeService::new(server);
    let service = MakeAllowAllAuthenticator::new(service, "cosmo");
    let service = reposync_lib::server::context::MakeAddContext::<_, EmptyContext>::new(service);

    hyper::server::Server::bind(&addr)
        .serve(service)
        .await
        .unwrap()
}

#[derive(Clone)]
pub struct Server<C> {
    marker: PhantomData<C>,
    sync_manager: Arc<SyncManager>,
}

impl<C> Server<C> {
    pub fn new(sync_manager: SyncManager) -> Self {
        Server {
            marker: PhantomData,
            sync_manager: Arc::new(sync_manager),
        }
    }
}

#[async_trait]
impl<C> Api<C> for Server<C>
where
    C: Has<XSpanIdString> + Send + Sync,
{
    /// simple health-check
    async fn health_get(&self, context: &C) -> Result<HealthGetResponse, ApiError> {
        let context = context.clone();
        info!("health_get() - X-Span-ID: {:?}", context.get().0.clone());
        Err("Generic failuare".into())
    }

    /// status of repository
    async fn repo_repo_get(
        &self,
        repo: String,
        context: &C,
    ) -> Result<RepoRepoGetResponse, ApiError> {
        // self.sync_manager.load_current(&self.config, )
        let context = context.clone();
        info!(
            "repo_repo_get(\"{}\") - X-Span-ID: {:?}",
            repo,
            context.get().0.clone()
        );
        Err("Generic failuare".into())
    }

    /// Perform a synchronization
    async fn repo_repo_sync_post(
        &self,
        repo: String,
        context: &C,
    ) -> Result<RepoRepoSyncPostResponse, ApiError> {
        let context = context.clone();
        info!(
            "repo_repo_sync_post(\"{}\") - X-Span-ID: {:?}",
            repo,
            context.get().0.clone()
        );
        Err("Generic failuare".into())
    }
}
