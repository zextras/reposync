use crate::fetcher::Fetcher;
use data_encoding::BASE32_NOPAD;
use std::fs;
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use tempfile::TempDir;

    struct NoOpFetcher;
    impl Fetcher for NoOpFetcher {
        fn fetch(&self, _url: &str) -> Result<Box<dyn std::io::Read>, crate::fetcher::FetchError> {
            Err(crate::fetcher::FetchError { code: 404, error: "not found".into() })
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