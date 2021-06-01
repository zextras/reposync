#![allow(missing_docs, trivial_casts, unused_variables, unused_mut, unused_imports, unused_extern_crates, non_camel_case_types)]

use async_trait::async_trait;
use futures::Stream;
use std::error::Error;
use std::task::{Poll, Context};
use swagger::{ApiError, ContextWrapper};
use serde::{Serialize, Deserialize};

type ServiceError = Box<dyn Error + Send + Sync + 'static>;

pub const BASE_PATH: &'static str = "";
pub const API_VERSION: &'static str = "1.0.0";

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
pub enum HealthGetResponse {
    /// Empty response when everything is ok.
    EmptyResponseWhenEverythingIsOk
    ,
    /// Service unavailable when service has known issues, such as a full disk.
    ServiceUnavailableWhenServiceHasKnownIssues
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
pub enum RepositoryRepoGetResponse {
    /// The status of the repository.
    TheStatusOfTheRepository
    (models::Status)
    ,
    /// Repository not found.
    RepositoryNotFound
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
pub enum RepositoryRepoSyncPostResponse {
    /// The synchronization has been queued correctly.
    TheSynchronizationHasBeenQueuedCorrectly
    (models::Status)
    ,
    /// Repository not found.
    RepositoryNotFound
}

/// API
#[async_trait]
pub trait Api<C: Send + Sync> {
    fn poll_ready(&self, _cx: &mut Context) -> Poll<Result<(), Box<dyn Error + Send + Sync + 'static>>> {
        Poll::Ready(Ok(()))
    }

    /// Simple health-check
    async fn health_get(
        &self,
        context: &C) -> Result<HealthGetResponse, ApiError>;

    /// status of repository
    async fn repository_repo_get(
        &self,
        repo: String,
        context: &C) -> Result<RepositoryRepoGetResponse, ApiError>;

    /// Perform a synchronization
    async fn repository_repo_sync_post(
        &self,
        repo: String,
        context: &C) -> Result<RepositoryRepoSyncPostResponse, ApiError>;

}

/// API where `Context` isn't passed on every API call
#[async_trait]
pub trait ApiNoContext<C: Send + Sync> {

    fn poll_ready(&self, _cx: &mut Context) -> Poll<Result<(), Box<dyn Error + Send + Sync + 'static>>>;

    fn context(&self) -> &C;

    /// Simple health-check
    async fn health_get(
        &self,
        ) -> Result<HealthGetResponse, ApiError>;

    /// status of repository
    async fn repository_repo_get(
        &self,
        repo: String,
        ) -> Result<RepositoryRepoGetResponse, ApiError>;

    /// Perform a synchronization
    async fn repository_repo_sync_post(
        &self,
        repo: String,
        ) -> Result<RepositoryRepoSyncPostResponse, ApiError>;

}

/// Trait to extend an API to make it easy to bind it to a context.
pub trait ContextWrapperExt<C: Send + Sync> where Self: Sized
{
    /// Binds this API to a context.
    fn with_context(self: Self, context: C) -> ContextWrapper<Self, C>;
}

impl<T: Api<C> + Send + Sync, C: Clone + Send + Sync> ContextWrapperExt<C> for T {
    fn with_context(self: T, context: C) -> ContextWrapper<T, C> {
         ContextWrapper::<T, C>::new(self, context)
    }
}

#[async_trait]
impl<T: Api<C> + Send + Sync, C: Clone + Send + Sync> ApiNoContext<C> for ContextWrapper<T, C> {
    fn poll_ready(&self, cx: &mut Context) -> Poll<Result<(), ServiceError>> {
        self.api().poll_ready(cx)
    }

    fn context(&self) -> &C {
        ContextWrapper::context(self)
    }

    /// Simple health-check
    async fn health_get(
        &self,
        ) -> Result<HealthGetResponse, ApiError>
    {
        let context = self.context().clone();
        self.api().health_get(&context).await
    }

    /// status of repository
    async fn repository_repo_get(
        &self,
        repo: String,
        ) -> Result<RepositoryRepoGetResponse, ApiError>
    {
        let context = self.context().clone();
        self.api().repository_repo_get(repo, &context).await
    }

    /// Perform a synchronization
    async fn repository_repo_sync_post(
        &self,
        repo: String,
        ) -> Result<RepositoryRepoSyncPostResponse, ApiError>
    {
        let context = self.context().clone();
        self.api().repository_repo_sync_post(repo, &context).await
    }

}


#[cfg(feature = "client")]
pub mod client;

// Re-export Client as a top-level name
#[cfg(feature = "client")]
pub use client::Client;

#[cfg(feature = "server")]
pub mod server;

// Re-export router() as a top-level name
#[cfg(feature = "server")]
pub use self::server::Service;

#[cfg(feature = "server")]
pub mod context;

pub mod models;

#[cfg(any(feature = "client", feature = "server"))]
pub(crate) mod header;
