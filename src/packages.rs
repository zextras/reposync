use data_encoding::HEXLOWER_PERMISSIVE;
use pgp::armor::Dearmor;
use pgp::crypto::HashAlgorithm;
use pgp::de::Deserialize;
use pgp::packet::{Packet, PacketParser, Subpacket};
use pgp::types::Version::{New, Old};
use pgp::types::{Mpi, PublicKeyTrait};
use pgp::{Deserializable, SignedPublicKey, StandaloneSignature};
use sha1::digest::{FixedOutput, Update};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::borrow::BorrowMut;
use std::collections::BTreeMap;
use std::io::{Cursor, Error, ErrorKind, Read, Seek};

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
pub enum Signature {
    PGPEmbedded,
    PGPExternal { signature: String },
    None,
}

impl Default for Signature {
    fn default() -> Self {
        Signature::None
    }
}

impl Signature {
    fn extract_body_and_signature(text: &str) -> Option<(String, String)> {
        let mut split = text.splitn(2, "\n\n");
        split.next();

        let second_token = split.next();
        if second_token.is_none() {
            return None;
        }

        let mut split = second_token
            .unwrap()
            .splitn(2, "\n-----BEGIN PGP SIGNATURE-----");

        let third_token = split.next();
        if third_token.is_none() {
            return None;
        }

        let forth_token = split.next();
        if forth_token.is_none() {
            return None;
        }
        let signature = format!("-----BEGIN PGP SIGNATURE-----{}", forth_token.unwrap());
        let body = third_token.unwrap().to_string();

        Some((body, signature))
    }

    pub fn matches<T>(
        &self,
        public_key: &SignedPublicKey,
        mut reader: &mut T,
    ) -> Result<(), std::io::Error>
    where
        T: Read + Seek,
    {
        match self {
            Signature::PGPEmbedded => {
                let mut text = String::new();
                reader.read_to_string(&mut text)?;
                let result = Signature::extract_body_and_signature(&text);
                if let Some((data, signature)) = result {
                    Signature::match_internal(public_key, &signature, data.as_bytes())
                } else {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        format!("cannot find pgp message & signature in file"),
                    ));
                }
            }
            Signature::PGPExternal { signature } => {
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;
                Signature::match_internal(public_key, signature, data.as_slice())
            }
            Signature::None => Ok(()),
        }
    }

    fn match_internal(
        public_key: &SignedPublicKey,
        signature: &String,
        data: &[u8],
    ) -> Result<(), Error> {
        let result = StandaloneSignature::from_armor_single(Cursor::new(signature.as_bytes()));

        if let Ok((signature, _)) = result {
            let result = signature.verify(&public_key, data);
            if let Err(err) = result {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidData,
                    format!("validation failed: {}", err.to_string()),
                ));
            }
        } else {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "cannot parse signature".to_string(),
            ));
        }
        Ok(())
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
    pub signature: Signature,
}

impl IndexFile {
    pub fn same_content(&self, other: &IndexFile) -> bool {
        self.path == other.path && self.size == other.size && self.hash == other.hash
    }
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

#[cfg(test)]
pub mod tests {
    use crate::packages::Signature;

    #[test]
    fn pgp_signature() {
        let text = "-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

Origin: Artifactory
Label: Artifactory
Suite: bionic
Codename: bionic
Date: Wed, 26 May 2021 14:35:57 UTC
Acquire-By-Hash: yes
Components: main test
Architectures: amd64 i386
MD5Sum:
 4130e8e033fb8de63e69228c4e7ac10a           205209 main/binary-amd64/Packages
 477db54328b917bc5d167c6fc6268dc2            24599 main/binary-amd64/Packages.bz2
 8ec5a802ba9e17574eac655844412176            29191 main/binary-amd64/Packages.gz
 d41d8cd98f00b204e9800998ecf8427e                0 main/binary-i386/Packages
 4059d198768f9f8dc9372dc1c54bc3c3               14 main/binary-i386/Packages.bz2
 3970e82605c7d109bb348fc94e9eecc0               20 main/binary-i386/Packages.gz
 f7fa524f13930bf01b98eaa6cf26a4bf            38446 test/binary-amd64/Packages
 44804d7800dc265fb088f84a4df89742             5549 test/binary-amd64/Packages.bz2
 fcb7408f6fbb71e4e2629e6ff8b92b24             5688 test/binary-amd64/Packages.gz
 d41d8cd98f00b204e9800998ecf8427e                0 test/binary-i386/Packages
 4059d198768f9f8dc9372dc1c54bc3c3               14 test/binary-i386/Packages.bz2
 3970e82605c7d109bb348fc94e9eecc0               20 test/binary-i386/Packages.gz
SHA1:
 009fcf60814654dc200649278ec05913dfdcb68e           205209 main/binary-amd64/Packages
 3ceafcb0c0b9d3b2e2ce705060dfd3d3dec0f00f            24599 main/binary-amd64/Packages.bz2
 d88610656ac72c9d632566b83f076815740640ee            29191 main/binary-amd64/Packages.gz
 da39a3ee5e6b4b0d3255bfef95601890afd80709                0 main/binary-i386/Packages
 64a543afbb5f4bf728636bdcbbe7a2ed0804adc2               14 main/binary-i386/Packages.bz2
 e03849ea786b9f7b28a35c17949e85a93eb1cff1               20 main/binary-i386/Packages.gz
 fc2371a7f0308851a02ef784ef07b24961313e0e            38446 test/binary-amd64/Packages
 9ab3438b22031dab13d23f6737ac27d8367aac35             5549 test/binary-amd64/Packages.bz2
 dd1c9bdec36f33022bbc339197072c020193c7be             5688 test/binary-amd64/Packages.gz
 da39a3ee5e6b4b0d3255bfef95601890afd80709                0 test/binary-i386/Packages
 64a543afbb5f4bf728636bdcbbe7a2ed0804adc2               14 test/binary-i386/Packages.bz2
 e03849ea786b9f7b28a35c17949e85a93eb1cff1               20 test/binary-i386/Packages.gz
SHA256:
 cf7a3976cf54fb3918c64410b84ca5de8f126eaac0935053369b3eeb2773cd5f           205209 main/binary-amd64/Packages
 bd3fa0815743f2f002c4a6a5c92bc91a054eb4ff26d507b6da907c892cba169e            24599 main/binary-amd64/Packages.bz2
 f92913b768c353e6a273d45ed05a78cbd51951daf19b0af6ca764e58355eef90            29191 main/binary-amd64/Packages.gz
 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                0 main/binary-i386/Packages
 d3dda84eb03b9738d118eb2be78e246106900493c0ae07819ad60815134a8058               14 main/binary-i386/Packages.bz2
 f5d031af01f137ae07fa71720fab94d16cc8a2a59868766002918b7c240f3967               20 main/binary-i386/Packages.gz
 0250197b4050b53dc392546c00fbdec3a5e835a68e432d58147c0256ea277daf            38446 test/binary-amd64/Packages
 089e684efc34b5292b130ed5c77dfc4cdf72f501011158c3622ccf55a84867c5             5549 test/binary-amd64/Packages.bz2
 3e32cb5d4c9dd7e8c9346012784f9817112af7f94d16720c2466c251ffa2e21b             5688 test/binary-amd64/Packages.gz
 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                0 test/binary-i386/Packages
 d3dda84eb03b9738d118eb2be78e246106900493c0ae07819ad60815134a8058               14 test/binary-i386/Packages.bz2
 f5d031af01f137ae07fa71720fab94d16cc8a2a59868766002918b7c240f3967               20 test/binary-i386/Packages.gz

-----BEGIN PGP SIGNATURE-----
Version: BCPG v1.64

iQIcBAABCAAGBQJgrlzNAAoJENpBjIijIZ97mj8QALEqd+xMXVPwFchkokVZxu8T
mPRue2G0YUkPPxmx1bZsbl4A3kJTc7G6mrk+e85rl0yXBhF8mU7jCKAp956KIp0I
8Bsg7XJDUyo+xL4zbYu2oR9ETR1f+5IPz/YzilzlaDPScrIWwHpCBmAGTpg01TKf
noHKHvV0ZopJTq3/fJmhx8c7TvAsuQxIhzi1TTV+TM1ir5SfLgSi46rREtrgkwcB
jgNXHLpBJ+4J5Y5Hq+M7vA0RIIULZI01pREVO0+1x67NQpm4A11GgJ1xi9nupsRO
CuupCTty5HJXUuKMNVvFW2QNN+qV+aN4kcOU0K/hnSKlxG3dPNc9vjCOj5D9TOte
/DhWCTbqY3lkqtG+aih5pU+qdkmyQXc1TZ/juJR3vPti/eL9xCu2sMU1ckOJVuyx
F6aX2dxtvAgWknwGAvkBnIoOs+LGx6MugNPEmbdKRQFrmXPyFYutojZIApUa+2Rr
YdwAd1lAL5RCp71uqPIz2tzC0ZfEMV4RbXbVoLzRhOHGleasMdMfJnhzbq/C10do
l8rZPuCOEEOBh/P40OkFxbFjzG7imQtZqD+XipufB4JOhBIKZydnCMqtz2nlrfCb
IKNuRAjB6Wzg+9PIh9cciYQxqzBLWa++33vnJ85CMa39dsB8r3mdCT2mBU4thAIG
lEq9/6sr7HPHcpDquH2n
=yLRq
-----END PGP SIGNATURE-----";

        let expected_body = "Origin: Artifactory
Label: Artifactory
Suite: bionic
Codename: bionic
Date: Wed, 26 May 2021 14:35:57 UTC
Acquire-By-Hash: yes
Components: main test
Architectures: amd64 i386
MD5Sum:
 4130e8e033fb8de63e69228c4e7ac10a           205209 main/binary-amd64/Packages
 477db54328b917bc5d167c6fc6268dc2            24599 main/binary-amd64/Packages.bz2
 8ec5a802ba9e17574eac655844412176            29191 main/binary-amd64/Packages.gz
 d41d8cd98f00b204e9800998ecf8427e                0 main/binary-i386/Packages
 4059d198768f9f8dc9372dc1c54bc3c3               14 main/binary-i386/Packages.bz2
 3970e82605c7d109bb348fc94e9eecc0               20 main/binary-i386/Packages.gz
 f7fa524f13930bf01b98eaa6cf26a4bf            38446 test/binary-amd64/Packages
 44804d7800dc265fb088f84a4df89742             5549 test/binary-amd64/Packages.bz2
 fcb7408f6fbb71e4e2629e6ff8b92b24             5688 test/binary-amd64/Packages.gz
 d41d8cd98f00b204e9800998ecf8427e                0 test/binary-i386/Packages
 4059d198768f9f8dc9372dc1c54bc3c3               14 test/binary-i386/Packages.bz2
 3970e82605c7d109bb348fc94e9eecc0               20 test/binary-i386/Packages.gz
SHA1:
 009fcf60814654dc200649278ec05913dfdcb68e           205209 main/binary-amd64/Packages
 3ceafcb0c0b9d3b2e2ce705060dfd3d3dec0f00f            24599 main/binary-amd64/Packages.bz2
 d88610656ac72c9d632566b83f076815740640ee            29191 main/binary-amd64/Packages.gz
 da39a3ee5e6b4b0d3255bfef95601890afd80709                0 main/binary-i386/Packages
 64a543afbb5f4bf728636bdcbbe7a2ed0804adc2               14 main/binary-i386/Packages.bz2
 e03849ea786b9f7b28a35c17949e85a93eb1cff1               20 main/binary-i386/Packages.gz
 fc2371a7f0308851a02ef784ef07b24961313e0e            38446 test/binary-amd64/Packages
 9ab3438b22031dab13d23f6737ac27d8367aac35             5549 test/binary-amd64/Packages.bz2
 dd1c9bdec36f33022bbc339197072c020193c7be             5688 test/binary-amd64/Packages.gz
 da39a3ee5e6b4b0d3255bfef95601890afd80709                0 test/binary-i386/Packages
 64a543afbb5f4bf728636bdcbbe7a2ed0804adc2               14 test/binary-i386/Packages.bz2
 e03849ea786b9f7b28a35c17949e85a93eb1cff1               20 test/binary-i386/Packages.gz
SHA256:
 cf7a3976cf54fb3918c64410b84ca5de8f126eaac0935053369b3eeb2773cd5f           205209 main/binary-amd64/Packages
 bd3fa0815743f2f002c4a6a5c92bc91a054eb4ff26d507b6da907c892cba169e            24599 main/binary-amd64/Packages.bz2
 f92913b768c353e6a273d45ed05a78cbd51951daf19b0af6ca764e58355eef90            29191 main/binary-amd64/Packages.gz
 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                0 main/binary-i386/Packages
 d3dda84eb03b9738d118eb2be78e246106900493c0ae07819ad60815134a8058               14 main/binary-i386/Packages.bz2
 f5d031af01f137ae07fa71720fab94d16cc8a2a59868766002918b7c240f3967               20 main/binary-i386/Packages.gz
 0250197b4050b53dc392546c00fbdec3a5e835a68e432d58147c0256ea277daf            38446 test/binary-amd64/Packages
 089e684efc34b5292b130ed5c77dfc4cdf72f501011158c3622ccf55a84867c5             5549 test/binary-amd64/Packages.bz2
 3e32cb5d4c9dd7e8c9346012784f9817112af7f94d16720c2466c251ffa2e21b             5688 test/binary-amd64/Packages.gz
 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855                0 test/binary-i386/Packages
 d3dda84eb03b9738d118eb2be78e246106900493c0ae07819ad60815134a8058               14 test/binary-i386/Packages.bz2
 f5d031af01f137ae07fa71720fab94d16cc8a2a59868766002918b7c240f3967               20 test/binary-i386/Packages.gz
";
        let expected_signature = "-----BEGIN PGP SIGNATURE-----
Version: BCPG v1.64

iQIcBAABCAAGBQJgrlzNAAoJENpBjIijIZ97mj8QALEqd+xMXVPwFchkokVZxu8T
mPRue2G0YUkPPxmx1bZsbl4A3kJTc7G6mrk+e85rl0yXBhF8mU7jCKAp956KIp0I
8Bsg7XJDUyo+xL4zbYu2oR9ETR1f+5IPz/YzilzlaDPScrIWwHpCBmAGTpg01TKf
noHKHvV0ZopJTq3/fJmhx8c7TvAsuQxIhzi1TTV+TM1ir5SfLgSi46rREtrgkwcB
jgNXHLpBJ+4J5Y5Hq+M7vA0RIIULZI01pREVO0+1x67NQpm4A11GgJ1xi9nupsRO
CuupCTty5HJXUuKMNVvFW2QNN+qV+aN4kcOU0K/hnSKlxG3dPNc9vjCOj5D9TOte
/DhWCTbqY3lkqtG+aih5pU+qdkmyQXc1TZ/juJR3vPti/eL9xCu2sMU1ckOJVuyx
F6aX2dxtvAgWknwGAvkBnIoOs+LGx6MugNPEmbdKRQFrmXPyFYutojZIApUa+2Rr
YdwAd1lAL5RCp71uqPIz2tzC0ZfEMV4RbXbVoLzRhOHGleasMdMfJnhzbq/C10do
l8rZPuCOEEOBh/P40OkFxbFjzG7imQtZqD+XipufB4JOhBIKZydnCMqtz2nlrfCb
IKNuRAjB6Wzg+9PIh9cciYQxqzBLWa++33vnJ85CMa39dsB8r3mdCT2mBU4thAIG
lEq9/6sr7HPHcpDquH2n
=yLRq
-----END PGP SIGNATURE-----";

        let (body, signature) = Signature::extract_body_and_signature(text).unwrap();

        assert_eq!(body, expected_body);
        assert_eq!(signature, expected_signature);
    }
}
