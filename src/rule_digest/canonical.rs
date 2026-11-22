use std::collections::BTreeMap;

use anyhow::{Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Calculate the version 1 rule digest. Reject unsupported knowledge values
/// and object keys that collide after line ending normalization.
pub(super) fn digest(input: Value) -> Result<String> {
    let canonical = canonicalize(input)?;
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(format!("v1:sha256:{:x}", Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Result<Value> {
    match value {
        Value::String(string) => Ok(Value::String(normalize_line_endings(&string))),
        Value::Bool(_) => Ok(value),
        Value::Array(items) => Ok(Value::Array(
            items.into_iter().map(canonicalize).collect::<Result<_>>()?,
        )),
        Value::Object(members) => {
            // Order normalized keys even if serde_json's preserve_order feature is enabled.
            let mut sorted = BTreeMap::new();
            for (key, value) in members {
                let key = normalize_line_endings(&key);
                let previous = sorted.insert(key, canonicalize(value)?);
                ensure!(
                    previous.is_none(),
                    "duplicate rule digest key after line ending normalization"
                );
            }
            Ok(Value::Object(sorted.into_iter().collect()))
        }
        Value::Null | Value::Number(_) => bail!("rule digest input cannot contain null or numbers"),
    }
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}
