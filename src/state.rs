use crate::config::S3Destination;
use crate::fetcher::Fetcher;
use aws_smithy_types::byte_stream::ByteStream;
use data_encoding::BASE32_NOPAD;
use log::warn;
use std::fs;
use std::fs::File;
use std::io::{Error, ErrorKind, Read, Write};
use std::rc::Rc;
use std::time::Duration;

pub trait RepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), std::io::Error>;
    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, std::io::Error>;
}

// ---------------------------------------------------------------------------
// SavedRepoMetadataStore — reads from local disk (used for local destinations)
// ---------------------------------------------------------------------------

pub struct SavedRepoMetadataStore {
    directory: String,
}

impl SavedRepoMetadataStore {
    pub fn new(directory: &str) -> Self {
        SavedRepoMetadataStore {
            directory: directory.into(),
        }
    }
}

impl RepoMetadataStore for SavedRepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), Error> {
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file_path = format!("{}/{}", self.directory, base32);
        let file = File::open(&file_path)?;
        let size = file.metadata()?.len();
        Ok((file_path, Box::new(Box::new(file)), size))
    }

    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, std::io::Error> {
        let (_, reader, _) = self.fetch(path)?;
        Ok(Some(reader))
    }
}

// ---------------------------------------------------------------------------
// LiveRepoMetadataStore — fetches from a remote URL into a tmp directory
// ---------------------------------------------------------------------------

pub struct LiveRepoMetadataStore {
    repo_base_url: String,
    tmp_directory: String,
    fetcher: Rc<dyn Fetcher>,
}

impl LiveRepoMetadataStore {
    pub fn new(
        repo_base_url: &str,
        tmp_directory: &str,
        fetcher: Rc<dyn Fetcher>,
    ) -> Result<Self, std::io::Error> {
        //just a safeguard in case something goes wrong
        //the directory should be {repo_name}_tmp
        if !tmp_directory.contains("tmp") {
            panic!("bug: abort before removing directory '{}'", tmp_directory);
        }
        if File::open(tmp_directory).is_ok() {
            fs::remove_dir_all(tmp_directory)?;
        }

        Ok(LiveRepoMetadataStore {
            repo_base_url: repo_base_url.into(),
            tmp_directory: tmp_directory.into(),
            fetcher,
        })
    }

    pub fn replace(&self, path: &str) -> Result<(), std::io::Error> {
        let tmp_dir = &format!("{}__", path);
        let existed = File::open(path).is_ok();
        if existed {
            std::fs::rename(&path, tmp_dir)?;
        }
        std::fs::rename(&self.tmp_directory, path)?;
        if existed {
            std::fs::remove_dir_all(tmp_dir)?;
        }

        Ok(())
    }
}

impl RepoMetadataStore for LiveRepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), std::io::Error> {
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file_path = format!("{}/{}", self.tmp_directory, base32);

        std::fs::create_dir_all(&self.tmp_directory)?;

        let fetch_result = self
            .fetcher
            .fetch(&format!("{}/{}", &self.repo_base_url, path));

        if fetch_result.is_err() {
            let err = fetch_result.err().unwrap();
            if err.code == 404 {
                return Err(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("file not found '{}'", path),
                ));
            }
            return Err(std::io::Error::new(
                ErrorKind::Other,
                format!("cannot fetch file '{}': {}", path, err.error),
            ));
        }

        let mut reader = fetch_result.unwrap();
        let mut output = File::create(&file_path)?;
        let size = std::io::copy(&mut reader, &mut output)?;
        let file_reader =
            Box::new(File::open(&file_path).expect("cannot open a just created file"));

        Ok((file_path, file_reader, size))
    }

    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, std::io::Error> {
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file = File::open(&format!("{}/{}", self.tmp_directory, base32));
        if let Ok(file) = file {
            Ok(Some(Box::new(file)))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// S3RepoMetadataStore — reads current state directly from the S3 destination.
//
// This eliminates the need for a local `data_path` persistence directory: the
// S3 bucket already holds the last-uploaded indexes, so we download those tiny
// metadata files at the start of each sync cycle to rebuild the "current" state
// and compute the diff.  First-run (empty bucket) returns NotFound which the
// callers already handle gracefully (full sync from scratch).
// ---------------------------------------------------------------------------

pub struct S3RepoMetadataStore {
    s3_client: aws_sdk_s3::Client,
    bucket: String,
    /// S3 key prefix, e.g. "my-repo" (no leading/trailing slash)
    s3_prefix: String,
    tmp_directory: String,
    /// Dedicated runtime so the S3 client's connection pool and body
    /// streaming always run on the same executor, avoiding deadlocks
    /// when the caller is already inside another tokio runtime.
    runtime: tokio::runtime::Runtime,
}

impl S3RepoMetadataStore {
    pub fn new(s3: &S3Destination, tmp_directory: &str) -> Self {
        let (access_key_id, access_key_secret) = s3
            .get_aws_credentials()
            .expect("cannot read aws credentials for S3RepoMetadataStore");

        let creds = aws_sdk_s3::config::Credentials::new(
            &access_key_id,
            &access_key_secret,
            None,
            None,
            "reposync-static",
        );
        let timeout_config = aws_sdk_s3::config::timeout::TimeoutConfig::builder()
            .read_timeout(Duration::from_secs(30))
            .build();
        let config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .credentials_provider(creds)
            .region(aws_sdk_s3::config::Region::new(s3.region_name.clone()))
            .endpoint_url(&s3.s3_endpoint)
            .force_path_style(true)
            .timeout_config(timeout_config)
            .build();

        S3RepoMetadataStore {
            s3_client: aws_sdk_s3::Client::from_conf(config),
            bucket: s3.s3_bucket.clone(),
            s3_prefix: s3.path.clone(),
            tmp_directory: tmp_directory.into(),
            runtime: tokio::runtime::Runtime::new().expect("failed to create tokio runtime for S3"),
        }
    }

    fn s3_key(&self, path: &str) -> String {
        if self.s3_prefix.is_empty() {
            path.into()
        } else {
            format!("{}/{}", self.s3_prefix, path)
        }
    }
}

/// Stream an S3 body to a local file chunk-by-chunk instead of buffering the
/// entire body in memory.  This avoids "streaming error" failures on large
/// objects (e.g. the Packages index) that can occur with `body.collect()`.
fn stream_body_to_file(
    rt: &tokio::runtime::Runtime,
    body: ByteStream,
    file_path: &str,
) -> Result<u64, Error> {
    let mut file = File::create(file_path)?;
    let mut size: u64 = 0;

    rt.block_on(async {
        let mut body = body;
        loop {
            match body.try_next().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk)?;
                    size += chunk.len() as u64;
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(Error::new(
                        ErrorKind::Other,
                        format!("streaming error after {} bytes: {:?}", size, e),
                    ));
                }
            }
        }
        Ok::<(), Error>(())
    })?;

    Ok(size)
}

impl RepoMetadataStore for S3RepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), Error> {
        const MAX_RETRIES: u32 = 3;
        let key = self.s3_key(path);

        std::fs::create_dir_all(&self.tmp_directory)?;
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file_path = format!("{}/{}", self.tmp_directory, base32);

        let mut last_err = None;

        for attempt in 1..=MAX_RETRIES {
            let result = self.runtime.block_on(
                self.s3_client
                    .get_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .send(),
            );

            match result {
                Ok(output) => match stream_body_to_file(&self.runtime, output.body, &file_path) {
                    Ok(size) => {
                        return Ok((file_path.clone(), Box::new(File::open(&file_path)?), size));
                    }
                    Err(e) => {
                        let msg = format!("S3 read body error for '{}': {}", path, e);
                        if attempt < MAX_RETRIES {
                            warn!("attempt {}/{}: {}, retrying...", attempt, MAX_RETRIES, msg);
                            std::thread::sleep(Duration::from_millis(500 * attempt as u64));
                        }
                        last_err = Some(Error::new(ErrorKind::Other, msg));
                    }
                },
                Err(e) => {
                    let is_404 = matches!(
                        &e,
                        aws_sdk_s3::error::SdkError::ServiceError(se)
                            if se.raw().status().as_u16() == 404
                    );
                    if is_404 {
                        return Err(Error::new(
                            ErrorKind::NotFound,
                            format!("not found in S3: {}", path),
                        ));
                    }
                    let msg = format!("S3 fetch error for '{}': {}", path, e);
                    if attempt < MAX_RETRIES {
                        warn!(
                            "attempt {}/{} for '{}': {}, retrying...",
                            attempt, MAX_RETRIES, path, msg
                        );
                        std::thread::sleep(Duration::from_millis(500 * attempt as u64));
                    }
                    last_err = Some(Error::new(ErrorKind::Other, msg));
                }
            }
        }

        Err(last_err.unwrap())
    }

    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, Error> {
        match self.fetch(path) {
            Ok((_, reader, _)) => Ok(Some(reader)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use tempfile::TempDir;

    struct NoOpFetcher;
    impl Fetcher for NoOpFetcher {
        fn fetch(&self, _url: &str) -> Result<Box<dyn std::io::Read>, crate::fetcher::FetchError> {
            Err(crate::fetcher::FetchError {
                code: 404,
                error: "not found".into(),
            })
        }
    }

    #[test]
    fn saved_store_read_returns_err_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let store = SavedRepoMetadataStore::new(dir.path().to_str().unwrap());
        let result = store.read("nonexistent/path");
        // SavedRepoMetadataStore propagates file-not-found as Err (no NotFound→None mapping)
        assert!(result.is_err());
        assert_eq!(result.err().unwrap().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn live_store_new_clears_tmp_directory() {
        let base = TempDir::new().unwrap();
        let tmp_dir = base.path().join("repo_tmp");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        // create a sentinel file inside
        std::fs::write(tmp_dir.join("sentinel"), b"hello").unwrap();

        let store = LiveRepoMetadataStore::new(
            "https://example.com",
            tmp_dir.to_str().unwrap(),
            Rc::new(NoOpFetcher),
        );
        assert!(store.is_ok(), "LiveRepoMetadataStore::new should succeed");
        // sentinel file should have been removed along with the directory
        assert!(!tmp_dir.join("sentinel").exists());
    }

    #[test]
    fn live_store_read_returns_none_for_missing_file() {
        let base = TempDir::new().unwrap();
        let tmp_dir = base.path().join("repo2_tmp");

        let store = LiveRepoMetadataStore::new(
            "https://example.com",
            tmp_dir.to_str().unwrap(),
            Rc::new(NoOpFetcher),
        )
        .unwrap();

        let result = store.read("nonexistent/path");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
