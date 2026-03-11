use crate::packages::{Hash, IndexFile, Signature};
use crate::state::RepoMetadataStore;
use std::io::{ErrorKind, Read};

pub fn add_optional_index<T>(
    state: &T,
    path: &str,
    indexes: &mut Vec<IndexFile>,
    signature: Signature,
) -> Result<Option<Box<dyn Read>>, std::io::Error>
where
    T: RepoMetadataStore + ?Sized,
{
    match state.fetch(path) {
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(err)
            }
        }
        Ok((disk_path, mut reader, size)) => {
            indexes.insert(
                0,
                IndexFile {
                    file_path: disk_path,
                    path: path.into(),
                    size,
                    hash: Hash::create_sha256_hash(&mut reader)?,
                    signature,
                },
            );

            Ok(Some(state.read(path).unwrap().unwrap()))
        }
    }
}
