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
    T: RepoMetadataStore,
{
    let result = state.fetch(path);
    if result.is_err() {
        let err = result.err().unwrap();
        if err.kind() == ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(err)
        }
    } else {
        let (disk_path, reader, size) = result.unwrap();
        indexes.insert(
            0,
            IndexFile {
                file_path: disk_path,
                path: path.into(),
                size,
                hash: Hash::None,
                signature,
            },
        );
        Ok(Some(reader))
    }
}
