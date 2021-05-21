use crate::fetcher::Fetcher;
use data_encoding::BASE32_NOPAD;
use std::fs::File;
use std::io::{Error, ErrorKind, Read};
use std::rc::Rc;

pub trait RepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), std::io::Error>;
    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, std::io::Error>;
}

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

pub struct LiveRepoMetadataStore {
    repo_base_url: String,
    directory: String,
    fetcher: Rc<dyn Fetcher>,
}

impl LiveRepoMetadataStore {
    pub fn new(repo_base_url: &str, directory: &str, fetcher: Rc<dyn Fetcher>) -> Self {
        LiveRepoMetadataStore {
            repo_base_url: repo_base_url.into(),
            directory: directory.into(),
            fetcher,
        }
    }

    pub fn replace(&self, path: &str) -> Result<(), std::io::Error> {
        let tmp_dir = &format!("{}_tmp", path);
        let existed = File::open(path).is_ok();
        if existed {
            std::fs::rename(&path, tmp_dir)?;
        }
        std::fs::rename(&self.directory, path)?;
        if existed {
            std::fs::remove_dir_all(tmp_dir)?;
        }

        Ok(())
    }
}

impl RepoMetadataStore for LiveRepoMetadataStore {
    fn fetch(&self, path: &str) -> Result<(String, Box<dyn Read>, u64), std::io::Error> {
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file_path = format!("{}/{}", self.directory, base32);

        std::fs::create_dir_all(&self.directory)?;

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
            return Err(std::io::Error::new(ErrorKind::Other, err.error));
        }

        let mut reader = fetch_result.unwrap();
        let mut output = File::create(&file_path)?;
        let size = std::io::copy(&mut reader, &mut output)?;
        let file_reader = Box::new(File::open(&file_path)?);

        Ok((file_path, file_reader, size))
    }

    fn read(&self, path: &str) -> Result<Option<Box<dyn Read>>, std::io::Error> {
        let base32 = BASE32_NOPAD.encode(path.as_bytes());
        let file = File::open(&format!("{}/{}", self.directory, base32));
        if let Ok(file) = file {
            Ok(Some(Box::new(file)))
        } else {
            Ok(None)
        }
    }
}
