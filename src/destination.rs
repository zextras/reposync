use bytes::Bytes;
use futures::future::Future;
use futures::stream::Stream;
use rusoto_cloudfront::{
    CloudFront, CloudFrontClient, CreateInvalidationRequest, InvalidationBatch, Paths,
};
use rusoto_core::credential::StaticProvider;
use rusoto_core::{region, HttpClient, Region};
use rusoto_s3::{DeleteObjectRequest, PutObjectRequest, S3Client, StreamingBody, S3};
use std::fs::File;
use std::io::{Error, ErrorKind, Read};
use std::pin::Pin;
use std::task::{Context, Poll};

pub trait Destination {
    fn upload(&mut self, path: &str, file: File) -> Result<(), std::io::Error>;
    fn delete(&mut self, path: &str) -> Result<(), std::io::Error>;
    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), std::io::Error>;
    fn name(&self) -> String;
}

pub struct S3Destination {
    pub path: String,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub cloudfront_endpoint: Option<String>,
    pub cloudfront_arn: Option<String>,
    pub region_name: String,
    pub access_key_id: String,
    pub access_key_secret: String,
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
    ) -> S3Destination {
        Self {
            path: path.into(),
            s3_endpoint: s3_endpoint.into(),
            s3_bucket: s3_bucket.into(),
            cloudfront_endpoint: cloudfront_endpoint.clone(),
            cloudfront_arn: cloudfront_arn.clone(),
            region_name: region_name.into(),
            access_key_id: access_key_id.into(),
            access_key_secret: access_key_secret.into(),
        }
    }

    fn s3_client(&mut self) -> S3Client {
        let request_dispatcher = HttpClient::new().expect("failed to create request dispatcher");
        let credential_provider = StaticProvider::new(
            self.access_key_id.clone(),
            self.access_key_secret.clone(),
            None,
            None,
        );
        rusoto_s3::S3Client::new_with(
            request_dispatcher,
            credential_provider,
            self.region(&self.region_name, &self.s3_endpoint),
        )
    }

    fn cloudfront_client(&mut self) -> Option<CloudFrontClient> {
        if self.cloudfront_arn.is_none() {
            return None;
        }

        let request_dispatcher = HttpClient::new().expect("failed to create request dispatcher");
        let credential_provider = StaticProvider::new(
            self.access_key_id.clone(),
            self.access_key_secret.clone(),
            None,
            None,
        );
        Some(rusoto_cloudfront::CloudFrontClient::new_with(
            request_dispatcher,
            credential_provider,
            self.region("us-east-1", &self.cloudfront_endpoint.clone().unwrap()),
        ))
    }

    fn region(&self, name: &str, endpoint: &str) -> Region {
        region::Region::Custom {
            name: name.into(),
            endpoint: endpoint.into(),
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

#[tokio::main]
async fn await_for<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    future.await
}

impl Destination for S3Destination {
    fn upload(&mut self, path: &str, file: File) -> Result<(), Error> {
        let client = self.s3_client();

        let len = Some(file.metadata()?.len() as i64);
        let body = StreamingBody::new(FileAdapter { file });

        println!(
            "uploading {}/{}/{}",
            &self.s3_endpoint,
            self.s3_bucket,
            &self.s3_path(path)
        );
        let result = await_for(client.put_object(PutObjectRequest {
            bucket: self.s3_bucket.clone(),
            key: self.s3_path(path),
            body: Some(body),
            content_length: len,
            ..Default::default()
        }));

        if result.is_err() {
            return Err(std::io::Error::new(
                ErrorKind::Other,
                format!("upload failed: {}", result.err().unwrap().to_string()),
            ));
        }

        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        let client = self.s3_client();

        println!(
            "deleting {}/{}/{}",
            &self.s3_endpoint,
            self.s3_bucket,
            self.s3_path(path)
        );
        let future = client.delete_object(DeleteObjectRequest {
            bucket: self.s3_bucket.clone(),
            key: self.s3_path(path),
            ..Default::default()
        });

        let result = await_for(future);
        if result.is_err() {
            return Err(std::io::Error::new(
                ErrorKind::Other,
                format!("delete failed: {}", result.err().unwrap().to_string()),
            ));
        }

        Ok(())
    }

    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), Error> {
        if let Some(client) = self.cloudfront_client() {
            if !paths.is_empty() {
                for path in &paths {
                    println!("invalidating {}", path);
                }
                let future = client.create_invalidation(CreateInvalidationRequest {
                    distribution_id: self.cloudfront_arn.clone().unwrap(),
                    invalidation_batch: InvalidationBatch {
                        caller_reference: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis()
                            .to_string(),
                        paths: Paths {
                            quantity: paths.len() as i64,
                            items: Some(
                                paths
                                    .iter()
                                    .map(|path| format!("/{}", self.s3_path(path)))
                                    .collect::<Vec<String>>(),
                            ),
                        },
                    },
                });

                let result = await_for(future);
                if result.is_err() {
                    return Err(std::io::Error::new(
                        ErrorKind::Other,
                        format!(
                            "cloudfront invalidation failed: {}",
                            result.err().unwrap().to_string()
                        ),
                    ));
                }
            }
        } else {
            for path in paths {
                println!("skipping cloudfront invalidation for {}", path);
            }
        }

        Ok(())
    }

    fn name(&self) -> String {
        format!("{}/{}", self.s3_endpoint, self.s3_bucket)
    }
}

struct FileAdapter {
    file: File,
}

impl Stream for FileAdapter {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut buffer: [u8; 4096] = [0; 4096];
        let result = self.get_mut().file.read(&mut buffer);
        if result.is_err() {
            return Poll::Ready(Some(Err(result.err().unwrap())));
        }
        let size = result.unwrap();
        if size == 0 {
            return Poll::Ready(None);
        }
        let bytes = Bytes::from(buffer[0..size].to_vec());
        Poll::Ready(Some(Ok(bytes)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if let Ok(metadata) = self.file.metadata() {
            (metadata.len() as usize, Some(metadata.len() as usize))
        } else {
            (0, None)
        }
    }
}

#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

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
        self,
    ) -> (
        BTreeMap<String, Vec<u8>>,
        BTreeSet<String>,
        BTreeSet<String>,
    ) {
        (self.map, self.delete_set, self.invalidation_set)
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
