use bytes::Bytes;
use futures::future::Future;
use futures::io::AllowStdIo;
use futures::stream::Stream;
use rusoto_cloudfront::{
    CloudFront, CloudFrontClient, CreateInvalidationRequest, InvalidationBatch, Paths,
};
use rusoto_core::credential::StaticProvider;
use rusoto_core::{region, HttpClient, Region};
use rusoto_s3::{DeleteObjectRequest, PutObjectRequest, S3Client, StreamingBody, S3};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Error, ErrorKind, Read};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::runtime::Runtime;

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
        rusoto_s3::S3Client::new_with(request_dispatcher, credential_provider, self.region())
    }

    fn cloudfront_client(&mut self) -> CloudFrontClient {
        let request_dispatcher = HttpClient::new().expect("failed to create request dispatcher");
        let credential_provider = StaticProvider::new(
            self.access_key_id.clone(),
            self.access_key_secret.clone(),
            None,
            None,
        );
        rusoto_cloudfront::CloudFrontClient::new_with(
            request_dispatcher,
            credential_provider,
            self.region(),
        )
    }

    fn region(&self) -> Region {
        region::Region::Custom {
            name: self.region_name.clone(),
            endpoint: self.s3_endpoint.clone(),
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
            &self.s3_endpoint, self.s3_bucket, path
        );
        let result = await_for(client.put_object(PutObjectRequest {
            bucket: self.s3_bucket.clone(),
            key: format!("{}/{}", &self.path, path),
            body: Some(body),
            content_length: len,
            ..Default::default()
        }));

        if result.is_err() {
            return Err(std::io::Error::new(
                ErrorKind::Other,
                result.err().unwrap().to_string(),
            ));
        }

        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        let client = self.s3_client();

        println!("deleting {}/{}/{}", &self.s3_endpoint, self.s3_bucket, path);
        let future = client.delete_object(DeleteObjectRequest {
            bucket: self.s3_bucket.clone(),
            key: path.into(),
            ..Default::default()
        });

        let result = await_for(future);
        if result.is_err() {
            return Err(std::io::Error::new(
                ErrorKind::Other,
                result.err().unwrap().to_string(),
            ));
        }

        Ok(())
    }

    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), Error> {
        if self.cloudfront_arn.is_some() {
            let client = self.cloudfront_client();
            for path in &paths {
                println!("invalidating {}", path);
            }
            let future = client.create_invalidation(CreateInvalidationRequest {
                distribution_id: self.cloudfront_arn.clone().unwrap(),
                invalidation_batch: InvalidationBatch {
                    //either empty or random
                    caller_reference: "".to_string(),
                    paths: Paths {
                        quantity: paths.len() as i64,
                        items: Some(paths),
                    },
                },
            });

            let result = await_for(future);
            if result.is_err() {
                return Err(std::io::Error::new(
                    ErrorKind::Other,
                    result.err().unwrap().to_string(),
                ));
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

pub struct MemoryDestination {
    path: String,
    map: BTreeMap<String, Vec<u8>>,
    delete_set: BTreeSet<String>,
    invalidation_set: BTreeSet<String>,
}

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

impl Destination for MemoryDestination {
    fn upload(&mut self, path: &str, mut file: File) -> Result<(), Error> {
        let mut vec = Vec::new();
        file.read_to_end(&mut vec)?;
        self.map.insert(format!("{}/{}", &self.path, path), vec);
        Ok(())
    }

    fn delete(&mut self, path: &str) -> Result<(), Error> {
        self.delete_set.insert(path.into());
        Ok((()))
    }

    fn invalidate(&mut self, paths: Vec<String>) -> Result<(), Error> {
        paths.iter().for_each(|x| {
            self.invalidation_set.insert(x.clone());
        });
        Ok((()))
    }

    fn name(&self) -> String {
        "memory".into()
    }
}
