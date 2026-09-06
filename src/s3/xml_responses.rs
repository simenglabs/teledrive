use serde::Serialize;
use quick_xml::se::to_string;

#[derive(Debug, Serialize)]
#[serde(rename = "ListAllMyBucketsResult")]
pub struct ListAllMyBucketsResult {
    #[serde(rename = "@xmlns")]
    pub xmlns: &'static str,
    #[serde(rename = "Owner")]
    pub owner: Owner,
    #[serde(rename = "Buckets")]
    pub buckets: BucketsWrapper,
}

#[derive(Debug, Serialize)]
pub struct Owner {
    #[serde(rename = "ID")]
    pub id: &'static str,
    #[serde(rename = "DisplayName")]
    pub display_name: &'static str,
}

#[derive(Debug, Serialize)]
pub struct BucketsWrapper {
    #[serde(rename = "Bucket")]
    pub list: Vec<XmlBucket>,
}

#[derive(Debug, Serialize)]
pub struct XmlBucket {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "CreationDate")]
    pub creation_date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename = "ListBucketResult")]
pub struct ListBucketResult {
    #[serde(rename = "@xmlns")]
    pub xmlns: &'static str,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Prefix")]
    pub prefix: String,
    #[serde(rename = "MaxKeys")]
    pub max_keys: usize,
    #[serde(rename = "IsTruncated")]
    pub is_truncated: bool,
    #[serde(rename = "Contents")]
    pub contents: Vec<XmlContent>,
}

#[derive(Debug, Serialize)]
pub struct XmlContent {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "LastModified")]
    pub last_modified: String,
    #[serde(rename = "ETag")]
    pub etag: String,
    #[serde(rename = "Size")]
    pub size: i64,
    #[serde(rename = "StorageClass")]
    pub storage_class: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename = "Error")]
pub struct S3XmlError {
    #[serde(rename = "Code")]
    pub code: String,
    #[serde(rename = "Message")]
    pub message: String,
    #[serde(rename = "Resource")]
    pub resource: String,
    #[serde(rename = "RequestId")]
    pub request_id: &'static str,
}

pub fn render_xml<T: Serialize>(value: &T) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if let Ok(rendered) = to_string(value) {
        xml.push_str(&rendered);
    }
    xml
}
