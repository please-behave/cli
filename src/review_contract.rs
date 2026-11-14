//! Shared review wire types and provider-schema constraints; semantic evidence stays local.
use crate::{
    limits::*,
    report::{Finding, Status},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    json!({"type":"object", "additionalProperties":false, "required":["reason","findings"], "properties":{
        "reason":{"type":"string", "minLength":1, "maxLength":MAX_SUMMARY_CHARS,
            "pattern":r"^[^\u0000-\u001F\u007F-\u009F\u2028\u2029]*[^\s\u0000-\u001F\u007F-\u009F\u2028\u2029][^\u0000-\u001F\u007F-\u009F\u2028\u2029]*$",
            "description":"One short summary line; detailed evidence belongs in findings"},
        "explanation":{"type":["string","null"], "maxLength":MAX_EXPLANATION_BYTES, "pattern":r"\S",
            "description":format!("Optional longer explanation of at most {MAX_EXPLANATION_BYTES} UTF-8 bytes")},
        "findings":{"type":"array", "maxItems":MAX_FINDINGS, "items":{
            "type":"object", "additionalProperties":false, "required":["message","path","line"], "properties":{
                "message":{"type":"string", "minLength":1, "maxLength":MAX_FINDING_MESSAGE_BYTES, "pattern":r"\S",
                    "description":format!("At most {MAX_FINDING_MESSAGE_BYTES} UTF-8 bytes; keep evidence concise")},
                "path":{"type":["string","null"]},
                "line":{"type":["integer","null"], "minimum":1, "maximum":u32::MAX}
            }
        }}
    }})
}
