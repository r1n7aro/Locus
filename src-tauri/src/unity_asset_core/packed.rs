//! Unity's compact primitive-array YAML scalar codec. The element type must
//! come from source/compiled schema; hexadecimal appearance alone is not proof.
use super::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackedElement {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedArray {
    element: PackedElement,
    values: Vec<Value>,
}

/// Internal typed Set value used by frozen merge plans. This reserved key
/// cannot be a C# member name; rendering validates every value before encoding.
pub fn packed_array_value(element: PackedElement, values: &[Value]) -> Result<Value, CoreError> {
    encode(values, element)?;
    Ok(serde_json::json!({"$locus_packed_array":{"element":element,"values":values}}))
}

pub(super) fn marker(value: &Value) -> Result<String, CoreError> {
    let array: PersistedArray = serde_json::from_value(value.clone())
        .map_err(|error| CoreError::new("packed_array", error.to_string(), 0))?;
    encode(&array.values, array.element)
}
impl PackedElement {
    fn width(self) -> usize {
        match self {
            Self::Bool | Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}
fn error(message: impl Into<String>) -> CoreError {
    CoreError::new("packed_array", message, 0)
}
fn integer(value: i128) -> Value {
    if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value) {
        serde_json::json!({"kind":if value > i64::MAX as i128 {"uint64"} else {"int64"},"value":value.to_string()})
    } else {
        Value::Number(Number::from(value as i64))
    }
}
fn float(value: f64) -> Value {
    if value.is_nan() {
        serde_json::json!({"kind":"float64","value":".nan"})
    } else if value.is_infinite() {
        serde_json::json!({"kind":"float64","value":if value.is_sign_negative() {"-.inf"} else {".inf"}})
    } else if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        Value::Number(Number::from(value as i64))
    } else {
        Value::Number(Number::from_f64(value).expect("finite"))
    }
}

pub(super) fn decode(raw: &str, element: PackedElement) -> Result<Vec<Value>, CoreError> {
    let raw = raw.trim();
    let width = element.width();
    if raw.len() % (width * 2) != 0 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error(format!(
            "compact {:?} array requires hexadecimal groups of {} characters",
            element,
            width * 2
        )));
    }
    if raw.len() / (width * 2) > super::Limits::default().max_nodes {
        return Err(error("compact array exceeds node limit"));
    }
    let mut values = Vec::with_capacity(raw.len() / (width * 2));
    for chunk in raw.as_bytes().chunks_exact(width * 2) {
        let mut bytes = [0_u8; 8];
        for (index, pair) in chunk.chunks_exact(2).enumerate() {
            let hex = |byte: u8| {
                if byte <= b'9' {
                    byte - b'0'
                } else {
                    byte.to_ascii_lowercase() - b'a' + 10
                }
            };
            bytes[index] = (hex(pair[0]) << 4) | hex(pair[1]);
        }
        let value = match element {
            PackedElement::Bool => match bytes[0] {
                0 | 1 => integer(bytes[0] as i128),
                _ => return Err(error("compact boolean elements must be 0 or 1")),
            },
            PackedElement::I8 => integer(i8::from_le_bytes([bytes[0]]) as i128),
            PackedElement::U8 => integer(bytes[0] as i128),
            PackedElement::I16 => {
                integer(i16::from_le_bytes(bytes[..2].try_into().unwrap()) as i128)
            }
            PackedElement::U16 => {
                integer(u16::from_le_bytes(bytes[..2].try_into().unwrap()) as i128)
            }
            PackedElement::I32 => {
                integer(i32::from_le_bytes(bytes[..4].try_into().unwrap()) as i128)
            }
            PackedElement::U32 => {
                integer(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as i128)
            }
            PackedElement::I64 => integer(i64::from_le_bytes(bytes) as i128),
            PackedElement::U64 => integer(u64::from_le_bytes(bytes) as i128),
            PackedElement::F32 => float(f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64),
            PackedElement::F64 => float(f64::from_le_bytes(bytes)),
        };
        values.push(value);
    }
    Ok(values)
}

pub(super) fn encode(values: &[Value], element: PackedElement) -> Result<String, CoreError> {
    let mut output = String::with_capacity(values.len().saturating_mul(element.width() * 2));
    for value in values {
        let mut bytes = Vec::new();
        if matches!(element, PackedElement::Bool) {
            let boolean = value
                .as_bool()
                .map(u8::from)
                .or_else(|| match value.as_i64() {
                    Some(0) => Some(0),
                    Some(1) => Some(1),
                    _ => None,
                })
                .ok_or_else(|| error("compact boolean writes require boolean or integer 0/1"))?;
            bytes.push(boolean);
        } else if matches!(element, PackedElement::F32 | PackedElement::F64) {
            let number = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| error("compact float writes require finite numeric values"))?;
            if matches!(element, PackedElement::F32) {
                if number.abs() > f32::MAX as f64 {
                    return Err(error("compact float exceeds Float32 range"));
                }
                bytes.extend_from_slice(&(number as f32).to_le_bytes());
            } else {
                bytes.extend_from_slice(&number.to_le_bytes());
            }
        } else {
            let decoded = super::decode_asset_value(value)?;
            let number = decoded
                .as_i64()
                .map(i128::from)
                .or_else(|| decoded.as_u64().map(i128::from))
                .ok_or_else(|| error("compact integer arrays require exact integer values"))?;
            macro_rules! push {
                ($ty:ty) => {{
                    let number: $ty = number
                        .try_into()
                        .map_err(|_| error(format!("value exceeds {:?} element range", element)))?;
                    bytes.extend_from_slice(&number.to_le_bytes());
                }};
            }
            match element {
                PackedElement::I8 => push!(i8),
                PackedElement::U8 => push!(u8),
                PackedElement::I16 => push!(i16),
                PackedElement::U16 => push!(u16),
                PackedElement::I32 => push!(i32),
                PackedElement::U32 => push!(u32),
                PackedElement::I64 => push!(i64),
                PackedElement::U64 => push!(u64),
                _ => unreachable!(),
            }
        }
        const HEX: &[u8] = b"0123456789abcdef";
        for byte in bytes {
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 15) as usize] as char);
        }
    }
    Ok(output)
}

/// Expand into a temporary flow sequence without passing exact integers through
/// floats or leaving transport envelopes in Unity serialization.
pub(super) fn flow(values: &[Value]) -> Result<Vec<u8>, CoreError> {
    let values = values
        .iter()
        .map(|value| {
            if let Some(raw) = value
                .as_object()
                .filter(|map| map.get("kind").and_then(Value::as_str) == Some("float64"))
                .and_then(|map| map.get("value"))
                .and_then(Value::as_str)
            {
                Ok(raw.to_string())
            } else {
                serde_json::to_string(&super::decode_asset_value(value)?)
                    .map_err(|error| CoreError::new("packed_array", error.to_string(), 0))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", values.join(", ")).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unity2022_int32_fixture_and_negative_boundaries_round_trip() {
        assert_eq!(
            decode("010000000200000003000000", PackedElement::I32).unwrap(),
            vec![json!(1), json!(2), json!(3)]
        );
        let values = vec![json!(-1), json!(i32::MIN), json!(i32::MAX)];
        assert_eq!(
            encode(&values, PackedElement::I32).unwrap(),
            "ffffffff00000080ffffff7f"
        );
        assert_eq!(
            decode("ffffffff00000080ffffff7f", PackedElement::I32).unwrap(),
            values
        );
        assert!(decode("0100", PackedElement::I32).is_err());
        assert!(decode("zz000000", PackedElement::I32).is_err());
        assert!(encode(&[json!(2147483648_i64)], PackedElement::I32).is_err());
        assert!(encode(&[json!(1.5)], PackedElement::I32).is_err());
    }
    #[test]
    fn exact_wide_ids_and_ieee_floats_have_no_rounding_via_json() {
        let values = vec![
            json!({"kind":"int64","value":"9007199254740993"}),
            json!({"kind":"int64","value":"9223372036854775807"}),
        ];
        assert_eq!(
            decode(
                &encode(&values, PackedElement::I64).unwrap(),
                PackedElement::I64
            )
            .unwrap(),
            values
        );
        assert_eq!(
            decode("0000803f00002040", PackedElement::F32).unwrap(),
            vec![json!(1), json!(2.5)]
        );
        assert_eq!(
            encode(&[json!(1), json!(2.5)], PackedElement::F32).unwrap(),
            "0000803f00002040"
        );
        assert_eq!(decode("", PackedElement::I32).unwrap(), Vec::<Value>::new());
        let unsigned = vec![json!({"kind":"uint64","value":"18446744073709551615"})];
        assert_eq!(
            decode("ffffffffffffffff", PackedElement::U64).unwrap(),
            unsigned
        );
        assert_eq!(
            encode(&unsigned, PackedElement::U64).unwrap(),
            "ffffffffffffffff"
        );
    }
}
