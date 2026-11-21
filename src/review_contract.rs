//! Shared review wire types and provider-schema constraints; semantic evidence stays local.
use crate::{
    limits::MAX_SUMMARY_CHARS,
    report::{Finding, Status},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The response document of an adapter: the exit status carries the verdict and the
/// request carries the review, so neither status nor ID is part of it (DEC-0025).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterVerdict {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub findings: Vec<Finding>,
}
/// The verdict of one review once the exit status and the review ID are attached.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResponse {
    pub reviews: Vec<ReviewVerdict>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewVerdict {
    pub id: String,
    pub status: Status,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub findings: Vec<Finding>,
}
pub const VERDICTS: [Status; 3] = [Status::Pass, Status::Fail, Status::Error];
pub fn valid_summary(reason: &str) -> bool {
    !reason.trim().is_empty()
        && reason.chars().count() <= MAX_SUMMARY_CHARS
        && !reason
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// Describe the response document of an adapter. Byte bounds and evidence existence/line
/// ranges still require local validation.
pub fn schema() -> Value {
    let mut schema: Value = serde_json::from_str(include_str!(
        "../why/contracts/adapter/response.schema.json"
    ))
    .expect("embedded adapter response schema is valid JSON");
    let object = schema
        .as_object_mut()
        .expect("response schema is an object");
    object.remove("$schema");
    object.remove("title");
    let properties = object["properties"].as_object_mut().unwrap();
    properties.remove("protocol_version");
    properties.remove("usage");
    object["required"]
        .as_array_mut()
        .unwrap()
        .retain(|name| name != "protocol_version");
    schema
}
