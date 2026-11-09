//! Shared review wire types and provider-schema constraints; semantic evidence stays local.
use crate::{
    limits::*,
    report::{Finding, Status},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

/// Describe the common provider-independent verdict document. Byte bounds, ID uniqueness/coverage and
/// evidence existence/line ranges still require local validation.
pub fn schema(ids: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false, "required":["reviews"], "properties":{
        "reviews":{"type":"array", "minItems":ids.len(), "maxItems":ids.len(), "items":{
            "type":"object", "additionalProperties":false, "required":["id","status","reason","findings"], "properties":{
                "id":{"type":"string", "enum":ids},
                "status":{"type":"string", "enum":VERDICTS.map(Status::label)},
                "reason":{"type":"string", "minLength":1, "maxLength":MAX_SUMMARY_CHARS,
                    "pattern":r"^[^\u0000-\u001F\u007F-\u009F\u2028\u2029]*[^\s\u0000-\u001F\u007F-\u009F\u2028\u2029][^\u0000-\u001F\u007F-\u009F\u2028\u2029]*$",
                    "description":"One short summary line; detailed evidence belongs in findings"},
                "findings":{"type":"array", "maxItems":MAX_FINDINGS, "items":{
                    "type":"object", "additionalProperties":false, "required":["message","path","line"], "properties":{
                        "message":{"type":"string", "minLength":1, "maxLength":MAX_FINDING_MESSAGE_BYTES, "pattern":r"\S",
                            "description":format!("At most {MAX_FINDING_MESSAGE_BYTES} UTF-8 bytes; keep evidence concise")},
                        "path":{"type":["string","null"]},
                        "line":{"type":["integer","null"], "minimum":1, "maximum":u32::MAX}
                    }
                }}
            }
        }}
    }})
}

/// Header evidence is invocation input rather than a repository file.
pub fn header_schema(ids: &[&str]) -> Value {
    let mut value = schema(ids);
    value["properties"]["reviews"]["items"]["properties"]["findings"]["minItems"] = json!(1);
    let finding = &mut value["properties"]["reviews"]["items"]["properties"]["findings"]["items"];
    finding["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("source"));
    finding["properties"]["source"] = json!({"type":"string", "enum":["commit-header"]});
    finding["properties"]["path"] = json!({"type":"null"});
    finding["properties"]["line"] = json!({"type":"integer", "enum":[1]});
    value
}
