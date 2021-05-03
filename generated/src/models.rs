#![allow(unused_qualifications)]

use crate::models;
#[cfg(any(feature = "client", feature = "server"))]
use crate::header;

/// Status of a repository
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "conversion", derive(frunk::LabelledGeneric))]
pub struct Status {
    /// Name of the repository, also work as UID
    #[serde(rename = "name")]
    pub name: String,

    /// The current status of the repository. Syncing means it's currently syncing. Waiting means it's waiting for the sync.
    // Note: inline enums are not fully supported by openapi-generator
    #[serde(rename = "status")]
    pub status: String,

    /// UTC timestamp for the next sync
    #[serde(rename = "next_sync")]
    pub next_sync: i64,

    /// UTC timestamp of last sync, 0 if never performed
    #[serde(rename = "last_sync")]
    pub last_sync: i64,

    /// Result of last sync, either \"ok\" or \"failure: reason\". When a sync is never performed \"ok\" is returned.
    #[serde(rename = "last_result")]
    pub last_result: String,

    /// Current size of the repository, in bytes.
    #[serde(rename = "size")]
    pub size: i64,

}

impl Status {
    pub fn new(name: String, status: String, next_sync: i64, last_sync: i64, last_result: String, size: i64, ) -> Status {
        Status {
            name: name,
            status: status,
            next_sync: next_sync,
            last_sync: last_sync,
            last_result: last_result,
            size: size,
        }
    }
}

/// Converts the Status value to the Query Parameters representation (style=form, explode=false)
/// specified in https://swagger.io/docs/specification/serialization/
/// Should be implemented in a serde serializer
impl std::string::ToString for Status {
    fn to_string(&self) -> String {
        let mut params: Vec<String> = vec![];

        params.push("name".to_string());
        params.push(self.name.to_string());


        params.push("status".to_string());
        params.push(self.status.to_string());


        params.push("next_sync".to_string());
        params.push(self.next_sync.to_string());


        params.push("last_sync".to_string());
        params.push(self.last_sync.to_string());


        params.push("last_result".to_string());
        params.push(self.last_result.to_string());


        params.push("size".to_string());
        params.push(self.size.to_string());

        params.join(",").to_string()
    }
}

/// Converts Query Parameters representation (style=form, explode=false) to a Status value
/// as specified in https://swagger.io/docs/specification/serialization/
/// Should be implemented in a serde deserializer
impl std::str::FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        #[derive(Default)]
        // An intermediate representation of the struct to use for parsing.
        struct IntermediateRep {
            pub name: Vec<String>,
            pub status: Vec<String>,
            pub next_sync: Vec<i64>,
            pub last_sync: Vec<i64>,
            pub last_result: Vec<String>,
            pub size: Vec<i64>,
        }

        let mut intermediate_rep = IntermediateRep::default();

        // Parse into intermediate representation
        let mut string_iter = s.split(',').into_iter();
        let mut key_result = string_iter.next();

        while key_result.is_some() {
            let val = match string_iter.next() {
                Some(x) => x,
                None => return std::result::Result::Err("Missing value while parsing Status".to_string())
            };

            if let Some(key) = key_result {
                match key {
                    "name" => intermediate_rep.name.push(<String as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    "status" => intermediate_rep.status.push(<String as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    "next_sync" => intermediate_rep.next_sync.push(<i64 as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    "last_sync" => intermediate_rep.last_sync.push(<i64 as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    "last_result" => intermediate_rep.last_result.push(<String as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    "size" => intermediate_rep.size.push(<i64 as std::str::FromStr>::from_str(val).map_err(|x| format!("{}", x))?),
                    _ => return std::result::Result::Err("Unexpected key while parsing Status".to_string())
                }
            }

            // Get the next key
            key_result = string_iter.next();
        }

        // Use the intermediate representation to return the struct
        std::result::Result::Ok(Status {
            name: intermediate_rep.name.into_iter().next().ok_or("name missing in Status".to_string())?,
            status: intermediate_rep.status.into_iter().next().ok_or("status missing in Status".to_string())?,
            next_sync: intermediate_rep.next_sync.into_iter().next().ok_or("next_sync missing in Status".to_string())?,
            last_sync: intermediate_rep.last_sync.into_iter().next().ok_or("last_sync missing in Status".to_string())?,
            last_result: intermediate_rep.last_result.into_iter().next().ok_or("last_result missing in Status".to_string())?,
            size: intermediate_rep.size.into_iter().next().ok_or("size missing in Status".to_string())?,
        })
    }
}

// Methods for converting between header::IntoHeaderValue<Status> and hyper::header::HeaderValue

#[cfg(any(feature = "client", feature = "server"))]
impl std::convert::TryFrom<header::IntoHeaderValue<Status>> for hyper::header::HeaderValue {
    type Error = String;

    fn try_from(hdr_value: header::IntoHeaderValue<Status>) -> std::result::Result<Self, Self::Error> {
        let hdr_value = hdr_value.to_string();
        match hyper::header::HeaderValue::from_str(&hdr_value) {
             std::result::Result::Ok(value) => std::result::Result::Ok(value),
             std::result::Result::Err(e) => std::result::Result::Err(
                 format!("Invalid header value for Status - value: {} is invalid {}",
                     hdr_value, e))
        }
    }
}

#[cfg(any(feature = "client", feature = "server"))]
impl std::convert::TryFrom<hyper::header::HeaderValue> for header::IntoHeaderValue<Status> {
    type Error = String;

    fn try_from(hdr_value: hyper::header::HeaderValue) -> std::result::Result<Self, Self::Error> {
        match hdr_value.to_str() {
             std::result::Result::Ok(value) => {
                    match <Status as std::str::FromStr>::from_str(value) {
                        std::result::Result::Ok(value) => std::result::Result::Ok(header::IntoHeaderValue(value)),
                        std::result::Result::Err(err) => std::result::Result::Err(
                            format!("Unable to convert header value '{}' into Status - {}",
                                value, err))
                    }
             },
             std::result::Result::Err(e) => std::result::Result::Err(
                 format!("Unable to convert header: {:?} to string: {}",
                     hdr_value, e))
        }
    }
}

