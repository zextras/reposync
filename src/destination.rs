use aws_sdk_cloudfront::types::{InvalidationBatch, Paths};
use aws_sdk_s3::primitives::ByteStream;
use std::fs::File;
use std::io::{Error, ErrorKind, Read};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::{DestinationConfig, GeneralConfig};

pub trait Destination {
    fn upload(&mut self, path: &str, file: File) -> Result<(), std::io::Error>;
    fn delete(&mut self, path: &str) -> Result<(), std::io::Error>;
    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), std::io::Error>;
    fn name(&self) -> String;
}

pub fn create_destination(
    general: &GeneralConfig,
    destination: &DestinationConfig,
) -> Result<Box<dyn Destination>, std::io::Error> {
    if destination.s3.is_some() {
        let s3 = destination.s3.clone().unwrap();

        let (access_key, access_key_secret) = s3
            .get_aws_credentials()
            .expect("cannot read aws cred, should be already validated");

        Ok(Box::new(S3Destination::new(
            &s3.path,
            &s3.s3_endpoint,
            &s3.s3_bucket,
            s3.cloudfront_endpoint.clone(),
            s3.cloudfront_distribution_id.clone(),
            &s3.region_name,
            &access_key,
            &access_key_secret,
            general.max_retries,
            Duration::from_secs(general.retry_sleep),
        )))
    } else {
        Ok(Box::new(LocalDestination::new(
            &destination.local.clone().unwrap().path,
        )?))
    }
}

pub struct LocalDestination {
    pub path: String,
}

impl LocalDestination {
    pub fn new(path: &str) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(path)?;
        Ok(LocalDestination { path: path.into() })
    }
}

impl Destination for LocalDestination {
    fn upload(&mut self, path: &str, mut file: File) -> Result<(), Error> {
        let s_path = format!("{}/{}", self.path, path);
        let path = Path::new(&s_path);
        println!("writing {}", &s_path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut writer = File::create(path)?;
        std::io::copy(&mut file, &mut writer)?;
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        let path = format!("{}/{}", self.path, path);
        println!("deleting {}", &path);
        std::fs::remove_file(&path)
    }

    fn invalidate(&mut self, _paths: Vec<String>) -> Result<(), Error> {
        Ok(())
    }

    fn name(&self) -> String {
        "local".into()
    }
}

pub struct S3Destination {
    pub path: String,
    pub s3_bucket: String,
    pub s3_endpoint: String,
    pub cloudfront_arn: Option<String>,
    pub max_retries: u32,
    pub retry_sleep: Duration,
    s3_client: aws_sdk_s3::Client,
    cloudfront_client: Option<aws_sdk_cloudfront::Client>,
}

impl S3Destination {
    pub fn new(
        path: &str,
        s3_endpoint: &str,
        s3_bucket: &str,
        cloudfront_endpoint: Option<String>,
        cloudfront_arn: Option<String>,
        region_name: &str,
        access_key_id: &str,
        access_key_secret: &str,
        max_retries: u32,
        retry_sleep: Duration,
    ) -> S3Destination {
        let s3_creds = aws_sdk_s3::config::Credentials::new(
            access_key_id,
            access_key_secret,
            None,
            None,
            "reposync-static",
        );
        let s3_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .credentials_provider(s3_creds.clone())
            .region(aws_sdk_s3::config::Region::new(region_name.to_string()))
            .endpoint_url(s3_endpoint)
            .force_path_style(true)
            .build();
        let s3_client = aws_sdk_s3::Client::from_conf(s3_config);

        let cloudfront_client = if cloudfront_arn.is_some() {
            let cf_endpoint = cloudfront_endpoint
                .as_deref()
                .unwrap_or("https://cloudfront.amazonaws.com");
            let cf_creds = aws_sdk_cloudfront::config::Credentials::new(
                access_key_id,
                access_key_secret,
                None,
                None,
                "reposync-static",
            );
            let cf_config = aws_sdk_cloudfront::config::Builder::new()
                .behavior_version(aws_sdk_cloudfront::config::BehaviorVersion::latest())
                .credentials_provider(cf_creds)
                .region(aws_sdk_cloudfront::config::Region::new(
                    "us-east-1".to_string(),
                ))
                .endpoint_url(cf_endpoint)
                .build();
            Some(aws_sdk_cloudfront::Client::from_conf(cf_config))
        } else {
            None
        };

        Self {
            path: path.into(),
            s3_bucket: s3_bucket.into(),
            s3_endpoint: s3_endpoint.into(),
            cloudfront_arn,
            max_retries,
            retry_sleep,
            s3_client,
            cloudfront_client,
        }
    }

    fn s3_path(&self, path: &str) -> String {
        if self.path.is_empty() {
            path.into()
        } else {
            format!("{}/{}", &self.path, path)
        }
    }
}

/// Helper: run an async future on the current thread using a one-shot tokio
/// runtime.  The callers (upload / delete / invalidate) are invoked from
/// synchronous code so we cannot simply `.await`.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => h.block_on(future),
        Err(_) => tokio::runtime::Runtime::new()
            .expect("failed to create tokio runtime")
            .block_on(future),
    }
}

impl Destination for S3Destination {
    fn upload(&mut self, path: &str, mut file: File) -> Result<(), Error> {
        let mut err: Option<Error> = None;

        for n in 0..self.max_retries {
            if n > 0 {
                sleep(self.retry_sleep);
                println!("Failed, retrying in {}s...", self.retry_sleep.as_secs());
            }

            // Read file into memory for the upload body
            let mut buf = Vec::new();
            file.seek_to_start()?;
            file.read_to_end(&mut buf)?;
            let body = ByteStream::from(buf);

            println!(
                "uploading {}/{}/{}",
                &self.s3_endpoint,
                self.s3_bucket,
                &self.s3_path(path)
            );

            let result = block_on(
                self.s3_client
                    .put_object()
                    .bucket(&self.s3_bucket)
                    .key(self.s3_path(path))
                    .body(body)
                    .send(),
            );

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    err = Some(std::io::Error::new(
                        ErrorKind::Other,
                        format!("upload failed: {}", e),
                    ));
                }
            }
        }

        Err(err.unwrap())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        let mut err: Option<Error> = None;

        for n in 0..self.max_retries {
            if n > 0 {
                sleep(self.retry_sleep);
                println!("Failed, retrying in {}s...", self.retry_sleep.as_secs());
            }
            println!(
                "deleting {}/{}/{}",
                &self.s3_endpoint,
                self.s3_bucket,
                self.s3_path(path)
            );

            let result = block_on(
                self.s3_client
                    .delete_object()
                    .bucket(&self.s3_bucket)
                    .key(self.s3_path(path))
                    .send(),
            );

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    err = Some(std::io::Error::new(
                        ErrorKind::Other,
                        format!("delete failed: {}", e),
                    ));
                }
            }
        }

        Err(err.unwrap())
    }

    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), Error> {
        let client = match &self.cloudfront_client {
            Some(c) => c,
            None => {
                for path in paths {
                    println!("skipping cloudfront invalidation for {}", path);
                }
                return Ok(());
            }
        };

        if paths.is_empty() {
            return Ok(());
        }

        let mut err: Option<Error> = None;

        for n in 0..self.max_retries {
            if n > 0 {
                sleep(self.retry_sleep);
                println!("Failed, retrying in {}s...", self.retry_sleep.as_secs());
            }
            for path in &paths {
                println!("invalidating {}", path);
            }

            let items: Vec<String> = paths
                .iter()
                .map(|p| format!("/{}", self.s3_path(p)))
                .collect();

            let invalidation_paths = Paths::builder()
                .quantity(paths.len() as i32)
                .set_items(Some(items))
                .build()
                .map_err(|e| {
                    std::io::Error::new(ErrorKind::Other, format!("failed to build Paths: {}", e))
                })?;

            let batch = InvalidationBatch::builder()
                .caller_reference(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis()
                        .to_string(),
                )
                .paths(invalidation_paths)
                .build()
                .map_err(|e| {
                    std::io::Error::new(
                        ErrorKind::Other,
                        format!("failed to build InvalidationBatch: {}", e),
                    )
                })?;

            let result = block_on(
                client
                    .create_invalidation()
                    .distribution_id(self.cloudfront_arn.clone().unwrap())
                    .invalidation_batch(batch)
                    .send(),
            );

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    err = Some(std::io::Error::new(
                        ErrorKind::Other,
                        format!("cloudfront invalidation failed: {}", e),
                    ));
                }
            }
        }

        Err(err.unwrap())
    }

    fn name(&self) -> String {
        format!("{}/{}", self.s3_endpoint, self.s3_bucket)
    }
}

/// Extension trait to seek a File to the beginning.
trait SeekToStart {
    fn seek_to_start(&mut self) -> Result<(), Error>;
}

impl SeekToStart for File {
    fn seek_to_start(&mut self) -> Result<(), Error> {
        use std::io::{Seek, SeekFrom};
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }
}

#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
pub struct MemoryDestination {
    path: String,
    map: BTreeMap<String, Vec<u8>>,
    delete_set: BTreeSet<String>,
    invalidation_set: BTreeSet<String>,
}
#[cfg(test)]
impl MemoryDestination {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.into(),
            map: BTreeMap::new(),
            delete_set: BTreeSet::new(),
            invalidation_set: BTreeSet::new(),
        }
    }

    pub fn explode(
        &self,
    ) -> (
        BTreeMap<String, Vec<u8>>,
        BTreeSet<String>,
        BTreeSet<String>,
    ) {
        (
            self.map.clone(),
            self.delete_set.clone(),
            self.invalidation_set.clone(),
        )
    }

    pub fn print(&self) {
        self.map.iter().for_each(|(k, v)| {
            println!("file[{:04}]: {}", v.len(), k);
        });

        self.delete_set.iter().for_each(|k| {
            println!("deletion: {}", k);
        });

        self.invalidation_set.iter().for_each(|k| {
            println!("invalidation: {}", k);
        });
    }
}

#[cfg(test)]
impl Destination for MemoryDestination {
    fn upload(&mut self, path: &str, mut file: File) -> Result<(), Error> {
        let mut vec = Vec::new();
        file.read_to_end(&mut vec)?;
        self.map.insert(format!("{}/{}", &self.path, path), vec);
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        self.delete_set.insert(format!("{}/{}", &self.path, path));
        Ok(())
    }

    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), Error> {
        paths.iter().for_each(|path| {
            self.invalidation_set
                .insert(format!("{}/{}", &self.path, path));
        });
        Ok(())
    }

    fn name(&self) -> String {
        "memory".into()
    }
}
