use data_encoding::HEXLOWER_PERMISSIVE;
use sha1::digest::{FixedOutput, Update};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::borrow::BorrowMut;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Hash {
    Sha1 { hex: String },
    Sha256 { hex: String },
    None,
}

impl Hash {
    /**
        returns an error when hash doesn't match
    */
    pub fn matches<T>(&self, mut reader: &mut T) -> Result<bool, std::io::Error>
    where
        T: Read,
    {
        match self {
            Hash::Sha1 { hex } => Hash::verify(reader, Sha1::new(), hex),
            Hash::Sha256 { hex } => Hash::verify(reader, Sha256::new(), hex),
            Hash::None => Ok(true),
        }
    }

    fn verify<T, D>(
        reader: &mut T,
        mut hasher: D,
        expected_hash: &str,
    ) -> Result<bool, std::io::Error>
    where
        T: Read,
        D: Update + FixedOutput,
    {
        let mut buffer: [u8; 4096] = [0u8; 4096];
        loop {
            let size = reader.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hasher.update(&buffer[0..size]);
        }

        let hash = HEXLOWER_PERMISSIVE.encode(hasher.finalize_fixed().as_slice());
        Ok(hash == expected_hash)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub path: String,
    pub hash: Hash,
    pub size: usize,
}

#[derive(Debug, Eq, PartialEq, Clone, PartialOrd, Ord)]
pub struct PackageKey {
    pub name: String,
    pub version: String,
    pub architecture: String,
}

impl Package {
    pub fn empty() -> Self {
        Self {
            name: "".to_string(),
            version: "".to_string(),
            architecture: "".to_string(),
            path: "".to_string(),
            hash: Hash::None,
            size: 0,
        }
    }

    pub fn is_same_version(&self, other: &Package) -> bool {
        self.name == other.name
            && self.version == other.version
            && self.architecture == other.architecture
    }

    pub fn key(&self) -> PackageKey {
        PackageKey {
            name: self.name.clone(),
            version: self.version.clone(),
            architecture: self.architecture.clone(),
        }
    }
}

/*
redhat:
 - repomod.xml
    - other.xml
    - primary.xml
 - [packages.rpm]

debian:
 - {xenial,bionic,focal}
    - Release
        - Packages{amd64,i386}.{,bz2,gz}

 */

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Target {
    //xenial bionic focal
    //empty for redhat?
    pub release_name: String,
    //amd64 x86_64 i386 arm64
    pub architectures: Vec<String>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct IndexFile {
    pub file_path: String,
    pub path: String,
    pub size: u64,
    pub hash: Hash,
}

pub struct Collection {
    pub target: Target,
    // pub relative_path: String,
    pub indexes: Vec<IndexFile>,
    pub packages: Vec<Package>,
}

impl Collection {
    pub fn empty(target: &Target) -> Self {
        Collection {
            target: target.clone(),
            indexes: Vec::new(),
            packages: Vec::new(),
        }
    }
}

pub struct Repository {
    pub name: String,
    pub collections: Vec<Collection>,
}
