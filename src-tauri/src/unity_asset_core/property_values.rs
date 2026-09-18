//! Version-gated adapters for Inspector value payloads. Unknown layouts fail
//! closed; callers can still edit proven serialized leaves with the raw API.
use serde_json::{json, Value};
fn number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or("property.finite_number_required".into())
}
fn mode(value: &Value, names: &[(&str, i64)], default: i64) -> Result<i64, String> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(n) = value.as_i64().filter(|n| names.iter().any(|(_, v)| v == n)) {
        return Ok(n);
    }
    names
        .iter()
        .find(|(name, _)| value.as_str().is_some_and(|value|value.eq_ignore_ascii_case(name)))
        .map(|(_, n)| *n)
        .ok_or("property.invalid_enum".into())
}
pub fn lower(current: &Value, value: &Value) -> Result<Value, String> {
    if current.get("m_Curve").is_some() && value.get("keys").is_some() {
        if current["serializedVersion"] != 2 {
            return Err("property.unsupported_curve_version".into());
        }
        let keys = value["keys"]
            .as_array()
            .filter(|k| k.len() <= 10000)
            .ok_or("property.invalid_curve")?;
        let mut previous = f64::NEG_INFINITY;
        let mut output = vec![];
        for key in keys {
            let time = number(&key["time"])?;
            if time < previous {
                return Err("property.curve_keys_must_be_sorted".into());
            }
            previous = time;
            let mut result = json!({"serializedVersion":3,"time":time,"value":number(&key["value"] )?,"tangentMode":0});
            for (input, output, default) in [
                ("inTangent", "inSlope", 0.0),
                ("outTangent", "outSlope", 0.0),
                ("inWeight", "inWeight", 1.0 / 3.0),
                ("outWeight", "outWeight", 1.0 / 3.0),
            ] {
                result[output] = json!(if key[input].is_null() {
                    default
                } else {
                    number(&key[input])?
                });
            }
            result["weightedMode"] = json!(mode(
                &key["weightedMode"],
                &[("None", 0), ("In", 1), ("Out", 2), ("Both", 3)],
                0
            )?);
            output.push(result);
        }
        let mut result = current.clone();
        result["m_Curve"] = json!(output);
        for (input, output) in [
            ("preWrapMode", "m_PreInfinity"),
            ("postWrapMode", "m_PostInfinity"),
        ] {
            if value[input].is_number(){return Err("property.curve_wrap_name_required".into());}
            result[output] = json!(mode(
                &value[input],
                &[
                    ("Default", 2),
                    ("Once", 2),
                    ("Clamp", 2),
                    ("Loop", 1),
                    ("PingPong", 0),
                    ("ClampForever", 2)
                ],
                2
            )?);
        }
        return Ok(result);
    }
    if current.get("m_NumColorKeys").is_some() && value.get("colorKeys").is_some() {
        if current["serializedVersion"] != 2 {
            return Err("property.unsupported_gradient_version".into());
        }
        let colors = value["colorKeys"]
            .as_array()
            .filter(|v| (2..=8).contains(&v.len()))
            .ok_or("property.gradient_color_keys_2_to_8")?;
        let alphas = value["alphaKeys"]
            .as_array()
            .filter(|v| (2..=8).contains(&v.len()))
            .ok_or("property.gradient_alpha_keys_2_to_8")?;
        let mut result = current.clone();
        for i in 0..8 {
            result[format!("key{i}")] = json!({"r":0,"g":0,"b":0,"a":0});
            result[format!("ctime{i}")] = json!(0);
            result[format!("atime{i}")] = json!(0);
        }
        for (keys, color) in [(colors, true), (alphas, false)] {
            let mut previous = -1.0;
            for (i, key) in keys.iter().enumerate() {
                let time = number(&key["time"])?;
                if !(0.0..=1.0).contains(&time) || time < previous {
                    return Err("property.gradient_times_must_be_sorted_in_unit_interval".into());
                }
                previous = time;
                result[format!("{}time{i}", if color { "c" } else { "a" })] =
                    json!((time * 65535.0).round() as u32);
                if color {
                    let hex = key["color"]
                        .as_str()
                        .and_then(|s| s.strip_prefix('#'))
                        .filter(|s| (s.len() == 6 || s.len() == 8) && s.bytes().all(|b|b.is_ascii_hexdigit()))
                        .ok_or("property.gradient_hex_color_required")?;
                    for (channel, start) in [("r", 0), ("g", 2), ("b", 4)] {
                        result[format!("key{i}")][channel] = json!(
                            u8::from_str_radix(&hex[start..start + 2], 16)
                                .map_err(|_| "property.invalid_color")?
                                as f64
                                / 255.0
                        );
                    }
                } else {
                    let alpha = number(&key["alpha"])?;
                    if !(0.0..=1.0).contains(&alpha) {
                        return Err("property.invalid_alpha".into());
                    }
                    result[format!("key{i}")]["a"] = json!(alpha);
                }
            }
        }
        result["m_NumColorKeys"] = json!(colors.len());
        result["m_NumAlphaKeys"] = json!(alphas.len());
        result["m_Mode"] = json!(mode(&value["mode"], &[("Blend", 0), ("Fixed", 1)], 0)?);
        return Ok(result);
    }
    if current.get("m_Center").is_some()
        && current.get("m_Extent").is_some()
        && value.get("center").is_some()
    {
        let mut result = current.clone();
        result["m_Center"] = value["center"].clone();
        result["m_Extent"] = value["extents"].clone();
        return Ok(result);
    }
    if current.get("Hash").is_some() && value.is_string() {
        let hash = value
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("property.invalid_hash128")?;
        let mut result = current.clone();
        result["Hash"] = json!(hash.to_ascii_lowercase());
        return Ok(result);
    }
    Ok(value.clone())
}

pub fn project(value: &Value) -> Option<(&'static str, Value)> {
    if value["serializedVersion"] != 2 {
        return None;
    }
    if let Some(keys) = value["m_Curve"].as_array() {
        let keys=keys.iter().map(|key|json!({"time":key["time"],"value":key["value"],"inTangent":key["inSlope"],"outTangent":key["outSlope"],"inWeight":key["inWeight"],"outWeight":key["outWeight"],"weightedMode":match key["weightedMode"].as_i64(){Some(1)=>"In",Some(2)=>"Out",Some(3)=>"Both",_=>"None"}})).collect::<Vec<_>>();
        let wrap = |key: &str| match value[key].as_i64() {
            Some(0) => "PingPong",
            Some(1) => "Loop",
            _ => "ClampForever",
        };
        return Some((
            "AnimationCurve",
            json!({"keys":keys,"preWrapMode":wrap("m_PreInfinity"),"postWrapMode":wrap("m_PostInfinity")}),
        ));
    }
    if let (Some(colors), Some(alphas)) = (
        value["m_NumColorKeys"].as_u64(),
        value["m_NumAlphaKeys"].as_u64(),
    ) {
        if colors > 8 || alphas > 8 {
            return None;
        }
        let mut color_keys = vec![];
        let mut alpha_keys = vec![];
        for i in 0..colors {
            let key = &value[format!("key{i}")];
            let mut color = String::from("#");
            for channel in ["r", "g", "b"] {
                color.push_str(&format!(
                    "{:02X}",
                    (key[channel].as_f64()?.clamp(0.0, 1.0) * 255.0).round() as u8
                ));
            }
            color.push_str("FF");
            color_keys.push(
                json!({"time":value[format!("ctime{i}")].as_f64()? as f32/65535.0,"color":color}),
            );
        }
        for i in 0..alphas {
            alpha_keys.push(json!({"time":value[format!("atime{i}")].as_f64()? as f32/65535.0,"alpha":value[format!("key{i}")]["a"]}));
        }
        return Some((
            "Gradient",
            json!({"mode":if value["m_Mode"]==1{"Fixed"}else{"Blend"},"colorKeys":color_keys,"alphaKeys":alpha_keys}),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_infinity_uses_serialized_codes_not_wrapmode_enum_values() {
        let current = json!({"serializedVersion":2,"m_Curve":[],"m_PreInfinity":2,"m_PostInfinity":2,"m_RotationOrder":4});
        let value = json!({"keys":[{"time":0,"value":1,"weightedMode":"Both"}],"preWrapMode":"Loop","postWrapMode":"PingPong"});
        let raw = lower(&current, &value).unwrap();
        assert_eq!(raw["m_PreInfinity"], 1);
        assert_eq!(raw["m_PostInfinity"], 0);
        let (_, roundtrip) = project(&raw).unwrap();
        assert_eq!(roundtrip["preWrapMode"], "Loop");
        assert_eq!(roundtrip["keys"][0]["weightedMode"], "Both");
    }
    #[test]
    fn curve_rejects_nonfinite_unordered_and_unknown_layout() {
        let current = json!({"serializedVersion":2,"m_Curve":[]});
        for value in [
            json!({"keys":[{"time":0,"value":1,"inTangent":"Infinity"}]}),
            json!({"keys":[{"time":2,"value":1},{"time":1,"value":0}]}),
        ] {
            assert!(lower(&current, &value).is_err());
        }
        assert!(lower(
            &json!({"serializedVersion":99,"m_Curve":[]}),
            &json!({"keys":[]})
        )
        .is_err());
    }
}
