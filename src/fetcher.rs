use data_encoding::BASE32_NOPAD;
#[cfg(test)]
use mockall::automock;
use reqwest::StatusCode;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::sync::Arc;

#[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Hash, Clone)]
pub struct FetchError {
    pub code: u16,
    pub error: String,
}

#[cfg_attr(test, automock)]
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError>;
}

struct DirectFetcher {}
impl Fetcher for DirectFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        println!("Requesting: {}", url);
        let result = reqwest::blocking::get(url);
        if result.is_ok() {
            let response = result.unwrap();
            if response.status().is_success() {
                Result::Ok(Box::new(response))
            } else {
                Result::Err(FetchError {
                    code: response.status().as_u16(),
                    error: format!("Request failed: {}", response.status().to_string()),
                })
            }
        } else {
            let err = result.err().unwrap();
            Result::Err(FetchError {
                code: err
                    .status()
                    .unwrap_or(StatusCode::SERVICE_UNAVAILABLE)
                    .as_u16(),
                error: format!("Request failed: {}", err.to_string()),
            })
        }
    }
}

pub struct DiskCacheFetcher {
    fetcher: Arc<dyn Fetcher>,
    cache_path: String,
}
impl Fetcher for DiskCacheFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        let base32 = BASE32_NOPAD.encode(url.as_bytes());
        let path = format!("{}/{}", self.cache_path, base32);

        let read_result = File::open(&path);
        let mut fetch_result: Result<Box<dyn Read>, FetchError>;
        if read_result.is_ok() {
            println!("Cache hit!");
            println!("{}", base32);
            fetch_result = Ok(Box::new(read_result.unwrap()));
        } else {
            println!("Cache miss!");
            fetch_result = self.fetcher.fetch(url);
            if fetch_result.is_ok() {
                let mut reader = fetch_result.unwrap();
                let mut writer = File::create(&path).unwrap();

                let write_result = std::io::copy(&mut reader, &mut writer);
                if write_result.is_err() {
                    return Err(FetchError {
                        code: reqwest::StatusCode::SERVICE_UNAVAILABLE.as_u16(),
                        error: format!("Write Failed: {}", write_result.err().unwrap().to_string()),
                    });
                }

                fetch_result = Ok(Box::new(File::open(path).unwrap()));
            }
        }

        fetch_result
    }
}

pub fn create_chain(cache_path: &str) -> Box<dyn Fetcher> {
    Box::new(DiskCacheFetcher {
        fetcher: Arc::new(DirectFetcher {}),
        cache_path: cache_path.to_string(),
    })
}

#[cfg(test)]
pub mod test {
    use crate::fetcher::{DirectFetcher, DiskCacheFetcher, Fetcher, MockFetcher};
    use mockall::predicate;
    use mockall::predicate::*;
    use mockall::*;
    use std::borrow::BorrowMut;
    use std::cell::RefCell;
    use std::fs;
    use std::io::{Cursor, Read};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    #[test]
    fn disk_read() {
        let mut mock = MockFetcher::new();

        mock.expect_fetch()
            .with(predicate::eq("https://url"))
            .times(1)
            .returning(|_| {
                let read: Box<dyn Read> = Box::new("ciao".as_bytes());
                Result::Ok(read)
            });

        let tmp_dir = tempfile::tempdir().unwrap();

        let mut fetcher = DiskCacheFetcher {
            fetcher: Arc::new(mock),
            cache_path: tmp_dir.path().to_str().unwrap().into(),
        };

        let mut text = String::new();
        fetcher
            .fetch("https://url")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();

        assert_eq!("ciao", text);
        assert_eq!(
            "ciao",
            fs::read_to_string(format!(
                "{}/NB2HI4DTHIXS65LSNQ",
                tmp_dir.path().to_str().unwrap()
            ))
            .unwrap()
        );
        let mut text = String::new();
        fetcher
            .fetch("https://url")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();

        assert_eq!("ciao", text, "mock can only be called once");
    }
}
