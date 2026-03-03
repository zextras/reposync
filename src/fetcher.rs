use data_encoding::BASE64;
#[cfg(test)]
use mockall::automock;
use reqwest::blocking::Client;
use reqwest::{header, StatusCode};
use std::io::Read;

use std::time::Duration;

#[derive(PartialEq, PartialOrd, Eq, Ord, Debug, Hash, Clone)]
pub struct FetchError {
    pub code: u16,
    pub error: String,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.error, self.code)
    }
}

#[cfg_attr(test, automock)]
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError>;
}

struct RetryFetcher {
    fetcher: Box<dyn Fetcher>,
    max_retries: u32,
    retry_sleep: Duration,
}

impl Fetcher for RetryFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        let mut last_err = None;
        let result = crate::retry::retry_with_backoff(
            self.max_retries,
            self.retry_sleep,
            "fetch",
            || {
                let res = self.fetcher.fetch(url);
                match res {
                    Ok(val) => Ok(Ok(val)),
                    Err(e) => {
                        if e.code == 404 {
                            Ok(Err(e))
                        } else {
                            last_err = Some(e.clone());
                            Err(e)
                        }
                    }
                }
            },
        );

        match result {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(last_err.unwrap()),
        }
    }
}

struct DirectFetcher {
    secret: Option<String>,
    timeout: Duration,
}
impl Fetcher for DirectFetcher {
    fn fetch(&self, url: &str) -> Result<Box<dyn Read>, FetchError> {
        log::debug!("requesting: {}", url);
        let builder = Client::builder();
        let mut headers = header::HeaderMap::new();
        if self.secret.is_some() {
            let mut auth_value = header::HeaderValue::from_str(&format!(
                "Basic {}",
                BASE64.encode(self.secret.clone().unwrap().as_bytes(),)
            ))
            .expect("cannot crate authorization header");
            auth_value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, auth_value);
        }
        let client = builder
            .default_headers(headers)
            .timeout(self.timeout)
            .build()
            .expect("cannot create http client");

        let result = client.get(url).send();
        if let Ok(response) = result {
            if response.status().is_success() {
                Result::Ok(Box::new(response))
            } else {
                Result::Err(FetchError {
                    code: response.status().as_u16(),
                    error: format!("request failed: {}", response.status()),
                })
            }
        } else {
            let err = result.err().unwrap();
            Result::Err(FetchError {
                code: err
                    .status()
                    .unwrap_or(StatusCode::SERVICE_UNAVAILABLE)
                    .as_u16(),
                error: format!("request failed: {}", err),
            })
        }
    }
}

pub fn create_chain(
    max_retries: u32,
    retry_sleep: Duration,
    secret: Option<String>,
    timeout: Duration,
) -> Result<Box<dyn Fetcher>, std::io::Error> {
    Ok(Box::new(RetryFetcher {
        max_retries,
        retry_sleep,
        fetcher: Box::new(DirectFetcher { secret, timeout }),
    }))
}

#[cfg(test)]
pub mod test {
    use crate::fetcher::{FetchError, Fetcher, MockFetcher, RetryFetcher};
    use mockall::predicate;
    use std::io::Read;
    use std::time::Duration;

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

        let fetcher = RetryFetcher {
            fetcher: Box::new(mock),
            max_retries: 3,
            retry_sleep: Duration::from_millis(0),
        };

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

        let fetcher = RetryFetcher {
            fetcher: Box::new(mock),
            max_retries: 3,
            retry_sleep: Duration::from_millis(0),
        };

        let mut reader = fetcher.fetch("https://url").unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).unwrap();

        assert_eq!("hello", content);
    }
}
