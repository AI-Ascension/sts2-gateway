// SPDX-License-Identifier: MIT

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

pub(super) fn decode(body: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UniqueValue>(body).map(|value| value.0)
}

pub(super) fn canonical(value: &Value) -> Result<Vec<u8>, &'static str> {
    let mut value = value.clone();
    let object = value
        .as_object_mut()
        .ok_or("runtime_v3_gameplay_request_invalid")?;
    object.remove("correlation_id");
    serde_json::to_vec(&value).map_err(|_| "runtime_v3_gameplay_request_invalid")
}

pub(super) fn rebind(body: &[u8], correlation_id: &str) -> Result<Vec<u8>, serde_json::Error> {
    let mut value = decode(body)?;
    value["correlation_id"] = Value::String(correlation_id.to_owned());
    serde_json::to_vec(&value)
}

pub(super) fn terminal(body: &[u8]) -> bool {
    decode(body).ok().is_some_and(|value| {
        matches!(
            value.get("status").and_then(Value::as_str),
            Some("settled" | "rejected" | "cancelled")
        )
    })
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueVisitor)
    }
}

struct UniqueVisitor;

impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON with unique object fields")
    }

    fn visit_bool<E: Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E: Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::from(value)))
    }

    fn visit_u64<E: Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::from(value)))
    }

    fn visit_f64<E: Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(|number| UniqueValue(Value::Number(number)))
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(UniqueValue(value)) = access.next_element()? {
            values.push(value);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(A::Error::custom("duplicate JSON field"));
            }
            let UniqueValue(value) = access.next_value()?;
            object.insert(key, value);
        }
        Ok(UniqueValue(Value::Object(object)))
    }
}
