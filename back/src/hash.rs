use bytes::Bytes;
use sha1::Digest;
use std::fs::File;
use std::io::Read;

use crate::error::LibResult;

#[derive(Clone, Debug)]
pub struct Hashes {
    pub sha1: String,
    pub sha512: String,
}

impl Hashes {
    pub(crate) fn get_hashes_from_file(file: &mut File) -> LibResult<Self> {
        let metadata = file.metadata()?;
        let mut buf = vec![0; metadata.len() as usize];

        std::fs::File::read(file, &mut buf)?;

        Ok(get_hashes_from_vec(buf))
    }

    #[allow(dead_code)]
    pub(crate) fn get_hashes_from_bytes(bytes: &Bytes) -> Self {
        get_hashes_from_vec(bytes)
    }

    pub(crate) fn dummy() -> Self {
        Self {
            sha1: hex::encode(sha1::Sha1::digest([0])),
            sha512: hex::encode(sha2::Sha512::digest([0])),
        }
    }
}

fn get_hashes_from_vec(vec: impl AsRef<[u8]>) -> Hashes {
    let sha1 = hex::encode(sha1::Sha1::digest(&vec));
    let sha512 = hex::encode(sha2::Sha512::digest(&vec));
    Hashes { sha1, sha512 }
}
