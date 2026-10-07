use serde::{Deserialize, Serialize};

use super::CategoryPolicy;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesCategory {
    #[serde(default)]
    pub policy: CategoryPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ManagedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFile {
    pub path: String,
    pub content: String,

    #[serde(default)]
    pub encoding: FileEncoding,

    #[serde(default = "default_file_mode")]
    pub mode: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum FileEncoding {
    #[serde(rename = "utf-8")]
    #[default]
    Utf8,
    #[serde(rename = "base64")]
    Base64,
}

fn default_file_mode() -> String {
    "100644".to_owned()
}
