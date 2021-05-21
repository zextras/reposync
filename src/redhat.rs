/*
https://rpm.releases.hashicorp.com/RHEL/8/x86_64/stable/repodata/repomd.xml

primary.xml.gz Contains an XML file describing the primary metadata of each RPM archive.
filelists.xml.gz Contains an XML file describing all the files contained within each RPM archive.
other.xml.gz Contains an XML file describing miscellaneous information regarding each RPM archive.
repomd.xml Contains information regarding all the other metadata files.

16b72c920dbd5d48e8aceb383b4b74664eb079ba-other.xml.gz:
<?xml version="1.0"?>
<otherdata xmlns="http://linux.duke.edu/metadata/other" packages="1">
  <package pkgid="f4b4281c08a8996ce337f09b1f6bcdb55e3a42ac" name="service-discover-base" arch="x86_64">
    <version epoch="0" ver="1.9.2" rel="1.el7"/>
  </package>
</otherdata>

2e1eb1fb69a2ca7fbd6d8723ce7d3cd91e9a9f13-primary.xml.gz:
*/

use crate::packages::{Hash, Package};
use std::io::{ErrorKind, Read};
use std::str::FromStr;
use xml::reader::{Events, XmlEvent};
use xml::EventReader;

#[derive(Debug, Eq, PartialEq, Clone)]
struct RepomodData {
    type_: String,
    location: String,
    hash: Hash,
    size: usize,
}

fn next_event<R>(iterator: &mut Events<&mut R>) -> Result<Option<XmlEvent>, std::io::Error>
where
    R: Read,
{
    let event = iterator.next();
    if event.is_none() {
        return Ok(None);
    }
    let event = event.unwrap();
    if event.is_err() {
        return Result::Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("invalid xml {}", event.err().unwrap().msg()),
        ));
    }
    Result::Ok(Some(event.unwrap()))
}

pub fn parse_packages<R>(source: &mut R) -> Result<Vec<Package>, std::io::Error>
where
    R: Read,
{
    let mut packages: Vec<Package> = Vec::new();
    let mut iterator = xml::reader::EventReader::new(source).into_iter();
    loop {
        let event = next_event(&mut iterator)?;
        if event.is_none() {
            break;
        }
        match event.unwrap() {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                if name.local_name == "package" {
                    packages.push(parse_package(&mut iterator)?)
                }
            }
            _ => {}
        }
    }
    Result::Ok(packages)
}

fn parse_package<R>(iterator: &mut Events<&mut R>) -> Result<Package, std::io::Error>
where
    R: Read,
{
    let mut data = Package {
        name: "".to_string(),
        version: "".to_string(),
        architecture: "".to_string(),
        path: "".to_string(),
        hash: Hash::None,
        size: 0,
    };

    let mut last_tag = "data".into();
    loop {
        let event = next_event(iterator)?;
        if event.is_none() {
            break;
        }

        match event.unwrap() {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                last_tag = name.local_name.clone();
                match name.local_name.as_str() {
                    "location" => {
                        let location = attributes.iter().find(|x| x.name.local_name == "href");
                        if let Some(location) = location {
                            data.path = location.value.clone();
                        } else {
                            return Result::Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                format!("missing href from location"),
                            ));
                        }
                    }
                    "size" => {
                        let size = attributes
                            .iter()
                            .find(|x| x.name.local_name == "package")
                            .map(|x| x.value.as_str());

                        if size.is_none() {
                            return Result::Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                format!("invalid size tag"),
                            ));
                        }
                        let parsed = usize::from_str(size.unwrap());
                        if parsed.is_err() {
                            return Result::Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                format!("invalid size: {}", &parsed.err().unwrap().to_string()),
                            ));
                        }
                        data.size = parsed.unwrap();
                    }
                    "version" => {
                        let epoch = attributes
                            .iter()
                            .find(|x| x.name.local_name == "epoch")
                            .map(|x| x.value.as_str());
                        let ver = attributes
                            .iter()
                            .find(|x| x.name.local_name == "ver")
                            .map(|x| x.value.as_str());
                        let rel = attributes
                            .iter()
                            .find(|x| x.name.local_name == "rel")
                            .map(|x| x.value.as_str());

                        if epoch.is_none() || ver.is_none() || rel.is_none() {
                            return Result::Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                format!("invalid version tag"),
                            ));
                        }
                        data.version =
                            format!("{}-{}-{}", ver.unwrap(), rel.unwrap(), epoch.unwrap());
                    }
                    _ => {}
                }
            }
            XmlEvent::Characters(text) => match last_tag.as_str() {
                "name" => data.name = text,
                "arch" => data.architecture = text,
                "checksum" => data.hash = Hash::Sha1 { hex: text },
                _ => {}
            },
            XmlEvent::EndElement { name } => {
                if name.local_name == "package" {
                    break;
                }
            }
            _ => {}
        }
    }

    Result::Ok(data)
}

fn parse_repomod<R>(source: &mut R) -> Result<Vec<RepomodData>, std::io::Error>
where
    R: Read,
{
    let mut entries: Vec<RepomodData> = Vec::new();
    let mut iterator = xml::reader::EventReader::new(source).into_iter();
    loop {
        let event = next_event(&mut iterator)?;
        if event.is_none() {
            break;
        }

        match event.unwrap() {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                if name.local_name == "data" {
                    entries.push(parse_repomod_data(
                        &mut iterator,
                        &attributes
                            .iter()
                            .find(|x| x.name.local_name == "type")
                            .map(|x| x.value.as_str())
                            .or(Some("unknown"))
                            .unwrap(),
                    )?)
                }
            }
            _ => {}
        }
    }

    Result::Ok(entries)
}

fn parse_repomod_data<R>(
    iterator: &mut Events<&mut R>,
    type_: &str,
) -> Result<RepomodData, std::io::Error>
where
    R: Read,
{
    let mut data = RepomodData {
        type_: type_.to_string(),
        location: "".to_string(),
        hash: Hash::None,
        size: 0,
    };

    let mut last_tag = "data".into();
    loop {
        let event = next_event(iterator)?;
        if event.is_none() {
            break;
        }

        match event.unwrap() {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                last_tag = name.local_name.clone();
                if name.local_name == "location" {
                    let location = attributes.iter().find(|x| x.name.local_name == "href");
                    if let Some(location) = location {
                        data.location = location.value.clone();
                    } else {
                        return Result::Err(std::io::Error::new(
                            ErrorKind::InvalidData,
                            format!("missing href from location"),
                        ));
                    }
                }
            }
            XmlEvent::Characters(text) => match last_tag.as_str() {
                "checksum" => data.hash = Hash::Sha1 { hex: text },
                "size" => {
                    let parsed = usize::from_str(&text);
                    if parsed.is_err() {
                        return Result::Err(std::io::Error::new(
                            ErrorKind::InvalidData,
                            format!("invalid size: {}", &parsed.err().unwrap().to_string()),
                        ));
                    }
                    data.size = parsed.unwrap()
                }
                _ => {}
            },

            XmlEvent::EndElement { name } => {
                if name.local_name == "data" {
                    break;
                }
            }

            _ => {}
        }
    }

    Result::Ok(data)
}

#[cfg(test)]
pub mod tests {
    use crate::packages::{Hash, Package};
    use crate::redhat::{parse_packages, parse_repomod, RepomodData};
    use std::fs::File;

    #[test]
    fn parse_repomod_successful() {
        let entries =
            parse_repomod(&mut File::open("samples/redhat/repomod.xml").unwrap()).unwrap();
        assert_eq!(
            vec![
                RepomodData {
                    type_: "other".into(),
                    location: "repodata/16b72c920dbd5d48e8aceb383b4b74664eb079ba-other.xml.gz"
                        .into(),
                    hash: Hash::Sha1 {
                        hex: "16b72c920dbd5d48e8aceb383b4b74664eb079ba".into()
                    },
                    size: 212,
                },
                RepomodData {
                    type_: "primary".into(),
                    location: "repodata/2e1eb1fb69a2ca7fbd6d8723ce7d3cd91e9a9f13-primary.xml.gz"
                        .into(),
                    hash: Hash::Sha1 {
                        hex: "2e1eb1fb69a2ca7fbd6d8723ce7d3cd91e9a9f13".into()
                    },
                    size: 784,
                }
            ],
            entries
        );
    }

    #[test]
    fn parse_packages_successful() {
        let entries =
            parse_packages(&mut File::open("samples/redhat/primary.xml").unwrap()).unwrap();
        assert_eq!(
            vec![
                Package {
                    name: "service-discover-server".to_string(),
                    version: "0.1.0-1.el7-0".to_string(),
                    architecture: "x86_64".to_string(),
                    path:
                        "zextras/service-discover-server/service-discover-server-0.1.0.x86_64.rpm"
                            .to_string(),
                    hash: Hash::Sha1 {
                        hex: "d331abce6e2300fc3a6e6d8d04849a7c58d20c00".into()
                    },
                    size: 1089320
                },
                Package {
                    name: "service-discover-daemon".to_string(),
                    version: "0.1.0-1.el7-0".to_string(),
                    architecture: "x86_64".to_string(),
                    path:
                        "zextras/service-discover-daemon/service-discover-daemon-0.1.0.x86_64.rpm"
                            .to_string(),
                    hash: Hash::Sha1 {
                        hex: "46530a9bd48e887301d3de5fbdb7634b9c2ac299".into()
                    },
                    size: 1469912
                }
            ],
            entries
        );
    }
}
