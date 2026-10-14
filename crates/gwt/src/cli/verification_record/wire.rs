//! Preserve versioned verification documents beside their typed projection.
//!
//! Pre-versioned payloads retain their historical struct-order serialization.
//! Opaque-field preservation starts with the versioned protocol; binaries that
//! predate it cannot be made forward-compatible by a newer writer alone.

use std::ops::{Deref, DerefMut};

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Sort every object explicitly, including when serde_json enables preserve_order.
pub fn canonical_hash(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries = object.iter().collect::<Vec<_>>();
                entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect(),
                )
            }
            Value::Array(array) => Value::Array(array.iter().map(sorted).collect()),
            _ => value.clone(),
        }
    }
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&sorted(value)).expect("serializable JSON"))
    )
}

#[derive(Debug, Clone)]
pub struct WireRecord<T> {
    pub data: T,
    original: Option<(Value, Value)>,
}

impl<T> From<T> for WireRecord<T> {
    fn from(data: T) -> Self {
        Self {
            data,
            original: None,
        }
    }
}

impl<T: Serialize> WireRecord<T> {
    fn value(&self) -> Result<Value, serde_json::Error> {
        let current = serde_json::to_value(&self.data)?;
        Ok(match &self.original {
            Some((raw, known)) => patch(raw, known, &current),
            None => current,
        })
    }

    /// Unknown plan inputs must participate in semantic plan comparison too.
    pub fn unknown_fields(&self) -> Value {
        let known = serde_json::to_value(&self.data).expect("serializable verification payload");
        let raw = self.value().expect("serializable verification document");
        extensions(&raw, &known).unwrap_or_else(|| Value::Object(Default::default()))
    }
}

impl<T> Deref for WireRecord<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.data
    }
}

impl<T> DerefMut for WireRecord<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.data
    }
}

impl<T: Serialize> Serialize for WireRecord<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = serde_json::to_value(&self.data).map_err(serde::ser::Error::custom)?;
        if known.get("format_version").is_none_or(Value::is_null) {
            // This also preserves struct order in an embedded legacy plan.
            return self.data.serialize(serializer);
        }
        self.value()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de, T: DeserializeOwned + Serialize> Deserialize<'de> for WireRecord<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let data: T = serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        let known = serde_json::to_value(&data).map_err(serde::de::Error::custom)?;
        Ok(Self {
            data,
            original: Some((raw, known)),
        })
    }
}

impl<T: Serialize> PartialEq for WireRecord<T> {
    fn eq(&self, other: &Self) -> bool {
        matches!((serde_json::to_value(self), serde_json::to_value(other)), (Ok(left), Ok(right)) if left == right)
    }
}

impl<T: Serialize + Eq> Eq for WireRecord<T> {}

fn patch(raw: &Value, before: &Value, after: &Value) -> Value {
    if before == after {
        return raw.clone();
    }
    match (raw, before, after) {
        (Value::Object(raw), Value::Object(before), Value::Object(after)) => {
            let mut result = raw.clone();
            for key in before.keys().filter(|key| !after.contains_key(*key)) {
                result.remove(key);
            }
            for (key, value) in after {
                let value = match (raw.get(key), before.get(key)) {
                    (Some(raw), Some(before)) => patch(raw, before, value),
                    _ => value.clone(),
                };
                result.insert(key.clone(), value);
            }
            Value::Object(result)
        }
        (Value::Array(raw), Value::Array(before), Value::Array(after))
            if after.starts_with(before) =>
        {
            let mut result = raw.clone();
            result.extend_from_slice(&after[before.len()..]);
            Value::Array(result)
        }
        // Changed array elements have no reliable opaque-field identity.
        // Preserve only untouched arrays and their unchanged prefix on append.
        _ => after.clone(),
    }
}

fn extensions(raw: &Value, known: &Value) -> Option<Value> {
    match (raw, known) {
        (Value::Object(raw), Value::Object(known)) => {
            let result = raw
                .iter()
                .filter_map(|(key, value)| {
                    let extension = match known.get(key) {
                        Some(known) => extensions(value, known),
                        None => Some(value.clone()),
                    };
                    extension.map(|value| (key.clone(), value))
                })
                .collect::<serde_json::Map<_, _>>();
            (!result.is_empty()).then_some(Value::Object(result))
        }
        (Value::Array(raw), Value::Array(known)) => {
            let result = raw
                .iter()
                .zip(known)
                .map(|(raw, known)| extensions(raw, known))
                .collect::<Vec<_>>();
            result.iter().any(Option::is_some).then(|| {
                Value::Array(
                    result
                        .into_iter()
                        .map(|value| value.unwrap_or(Value::Null))
                        .collect(),
                )
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct OlderRecord {
        format_version: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        commands: Vec<OlderCommand>,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct OlderCommand {
        command: String,
    }

    fn newer_document() -> Value {
        serde_json::json!({
            "format_version": 1,
            "note": "remove later",
            "commands": [{ "command": "test", "future": { "measurements": [1, 2] } }],
            "continuation": { "previous": { "future": ["opaque"] } }
        })
    }

    #[test]
    fn older_projection_preserves_newer_hash_and_nested_extensions() {
        let newer = newer_document();
        let mut older: WireRecord<OlderRecord> = serde_json::from_value(newer.clone()).unwrap();
        let roundtrip = serde_json::to_value(&older).unwrap();
        assert_eq!(canonical_hash(&roundtrip), canonical_hash(&newer));
        assert_eq!(
            older.unknown_fields()["commands"][0]["future"],
            newer["commands"][0]["future"]
        );

        older.note = None;
        let changed = serde_json::to_value(&older).unwrap();
        assert!(changed.get("note").is_none());
        assert_eq!(changed["commands"], newer["commands"]);
        assert_eq!(changed["continuation"], newer["continuation"]);
        assert_ne!(canonical_hash(&changed), canonical_hash(&newer));
        let reloaded = serde_json::from_value(changed).unwrap();
        assert_eq!(older, reloaded);
    }

    #[test]
    fn append_preserves_opaque_prefix_but_reorder_does_not_reassign_it() {
        let mut record: WireRecord<OlderRecord> = serde_json::from_value(newer_document()).unwrap();
        record.commands.push(OlderCommand {
            command: "next".into(),
        });
        assert!(serde_json::to_value(&record).unwrap()["commands"][0]
            .get("future")
            .is_some());
        record.commands.swap(0, 1);
        let reordered = serde_json::to_value(record).unwrap();
        assert_eq!(reordered["commands"][0]["command"], "next");
        assert!(reordered["commands"][0].get("future").is_none());
        assert!(reordered["commands"][1].get("future").is_none());
    }
}
