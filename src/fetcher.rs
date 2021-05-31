use data_encoding::{BASE32_NOPAD, BASE64_NOPAD};
#[cfg(test)]
use mockall::automock;
use reqwest::blocking::Client;
use reqwest::{header, StatusCode};
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Hash, Clone)]
pub struct FetchError {
    pub code: u16,
    pub error: String,
}

#[cfg_attr(test, automock)]
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError>;
}

struct RetryFetcher {
    fetcher: Box<dyn Fetcher>,
    retries: u8,
    sleep_ms: u64,
}

impl Fetcher for RetryFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        let mut err: Option<FetchError> = None;
        for n in 0..self.retries {
            if n > 0 {
                sleep(Duration::from_millis(self.sleep_ms));
                println!("Failed, retrying in {}ms...", self.sleep_ms);
            }
            let result = self.fetcher.fetch(url);
            if result.is_ok() {
                return result;
            }
            err = Some(result.err().unwrap());
        }
        Err(err.unwrap())
    }
}

struct DirectFetcher {
    username: Option<String>,
    password: Option<String>,
}
impl Fetcher for DirectFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        println!("requesting: {}", url);
        let builder = Client::builder();
        let mut headers = header::HeaderMap::new();
        if self.username.is_some() && self.password.is_some() {
            let mut auth_value = header::HeaderValue::from_str(
                &BASE64_NOPAD.encode(
                    format!(
                        "Basic {}:{}",
                        &self.username.clone().unwrap(),
                        &self.password.clone().unwrap()
                    )
                    .as_bytes(),
                ),
            )
            .expect("cannot crate authorization header");
            auth_value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, auth_value);
        }
        let client = builder
            .default_headers(headers)
            .build()
            .expect("cannot create http client");

        let result = client.get(url).send();
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
                error: format!("request failed: {}", err.to_string()),
            })
        }
    }
}

pub fn create_chain(
    retries: u8,
    sleep_ms: u64,
    username: Option<String>,
    password: Option<String>,
) -> Result<Box<dyn Fetcher>, std::io::Error> {
    Ok(Box::new(RetryFetcher {
        retries,
        sleep_ms,
        fetcher: Box::new(DirectFetcher { username, password }),
    }))
}

#[cfg(test)]
pub mod test {
    use crate::fetcher::{DirectFetcher, FetchError, Fetcher, MockFetcher, RetryFetcher};
    use mockall::predicate;
    use mockall::predicate::*;
    use mockall::*;
    use std::borrow::BorrowMut;
    use std::cell::RefCell;
    use std::fs;
    use std::io::{Cursor, Read};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn retry_fail() {
        let mut mock = MockFetcher::new();

        mock.expect_fetch()
            .with(predicate::eq("https://url"))
            .times(3)
            .returning(|_| {
                Result::Err(FetchError {
                    code: 500,
                    error: "".to_string(),
                })
            });

        let tmp_dir = tempfile::tempdir().unwrap();

        let mut fetcher = RetryFetcher {
            fetcher: Box::new(mock),
            retries: 3,
            sleep_ms: 0,
        };

        let mut text = String::new();
        let result = fetcher.fetch("https://url");
        assert!(result.is_err());
    }

    #[test]
    fn retry_successful() {
        let mut mock = MockFetcher::new();

        mock.expect_fetch()
            .with(predicate::eq("https://url"))
            .times(1)
            .returning(|_| Result::Ok(Box::new("hello".as_bytes())));

        let tmp_dir = tempfile::tempdir().unwrap();

        let mut fetcher = RetryFetcher {
            fetcher: Box::new(mock),
            retries: 3,
            sleep_ms: 1,
        };

        let mut text = String::new();
        let mut reader = fetcher.fetch("https://url").unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content);

        assert_eq!("hello", content);
    }
}
