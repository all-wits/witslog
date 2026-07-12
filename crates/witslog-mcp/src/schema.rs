//! Lightweight JSON-Schema validation for MCP tool params (FR-P5-003).
//!
//! Deliberately hand-rolled rather than pulling in a full JSON-Schema crate:
//! the tool schemas here are flat `object` schemas with `properties`,
//! `required`, and per-property `type`/`enum`/`maximum`, which this covers.
//! Anything unrecognized in the schema is ignored (permissive), but every
//! violation we *do* check is reported with a field name and no raw SQL or
//! internal details — safe to surface directly as `-32602 data.detail`.

use serde_json::Value;

/// Validate `params` against `schema` (an "object" JSON Schema as produced
/// by `Tool::input_schema`). Returns `Err(detail)` describing the first
/// violation found.
pub fn validate(schema: &Value, params: &Value) -> Result<(), String> {
    if !params.is_object() && !params.is_null() {
        return Err("params must be a JSON object".to_string());
    }

    let obj = params.as_object().cloned().unwrap_or_default();

    // Required fields.
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        for req in required {
            if let Some(name) = req.as_str() {
                match obj.get(name) {
                    Some(Value::Null) | None => {
                        return Err(format!("missing required field '{}'", name));
                    }
                    _ => {}
                }
            }
        }
    }

    // Per-property type/enum/maximum checks.
    if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in &obj {
            let Some(prop_schema) = properties.get(key) else {
                // Unknown fields are ignored (permissive) — MCP clients may
                // send extra metadata; we don't want to break on that.
                continue;
            };
            validate_value(key, value, prop_schema)?;
        }
    }

    Ok(())
}

fn validate_value(field: &str, value: &Value, prop_schema: &Value) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }

    if let Some(expected_type) = prop_schema.get("type").and_then(|v| v.as_str()) {
        let matches = match expected_type {
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => true,
        };
        if !matches {
            return Err(format!(
                "field '{}' must be of type {}",
                field, expected_type
            ));
        }
    }

    if let Some(allowed) = prop_schema.get("enum").and_then(|v| v.as_array()) {
        if let Some(s) = value.as_str() {
            let ok = allowed.iter().any(|a| a.as_str() == Some(s));
            if !ok {
                return Err(format!("field '{}' has invalid value", field));
            }
        }
    }

    if let Some(max) = prop_schema.get("maximum").and_then(|v| v.as_i64()) {
        if let Some(n) = value.as_i64() {
            if n > max {
                return Err(format!("field '{}' exceeds maximum of {}", field, max));
            }
        }
    }

    if expected_array_of_strings(prop_schema) {
        if let Some(arr) = value.as_array() {
            for item in arr {
                if !item.is_string() {
                    return Err(format!("field '{}' must be an array of strings", field));
                }
            }
        }
    }

    Ok(())
}

fn expected_array_of_strings(prop_schema: &Value) -> bool {
    prop_schema.get("type").and_then(|v| v.as_str()) == Some("array")
        && prop_schema
            .get("items")
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str())
            == Some("string")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_required_field_rejected() {
        let schema = json!({
            "type": "object",
            "properties": {"message": {"type": "string"}},
            "required": ["message"]
        });
        let err = validate(&schema, &json!({})).unwrap_err();
        assert!(err.contains("message"));
    }

    #[test]
    fn wrong_type_rejected() {
        let schema = json!({
            "type": "object",
            "properties": {"limit": {"type": "integer"}}
        });
        let err = validate(&schema, &json!({"limit": "twenty"})).unwrap_err();
        assert!(err.contains("limit"));
    }

    #[test]
    fn valid_params_pass() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "maximum": 200}
            },
            "required": ["query"]
        });
        assert!(validate(&schema, &json!({"query": "boom", "limit": 20})).is_ok());
    }

    #[test]
    fn over_maximum_rejected() {
        let schema = json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "maximum": 200}}
        });
        let err = validate(&schema, &json!({"limit": 500})).unwrap_err();
        assert!(err.contains("limit"));
    }

    #[test]
    fn bad_enum_rejected() {
        let schema = json!({
            "type": "object",
            "properties": {"mode": {"type": "string", "enum": ["a", "b"]}}
        });
        let err = validate(&schema, &json!({"mode": "c"})).unwrap_err();
        assert!(err.contains("mode"));
    }
}
