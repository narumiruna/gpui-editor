use std::ops::Range;

use serde_json::Value;

use super::super::{
    LspError,
    features::{Documentation, documentation},
};

/// A server-supplied overload label and its parameter information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureInformation {
    pub label: String,
    pub documentation: Option<Documentation>,
    pub parameters: Vec<SignatureParameter>,
    active_parameter: Option<usize>,
}

/// Parameter text, its range within the signature label, and its documentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureParameter {
    pub label: String,
    pub byte_range: Option<Range<usize>>,
    pub documentation: Option<Documentation>,
}

#[derive(Debug)]
pub(super) struct SignatureData {
    pub(super) signatures: Vec<SignatureInformation>,
    pub(super) wire: Value,
    active_parameter: Option<usize>,
}

impl SignatureData {
    pub(super) fn parse(value: &Value) -> Result<Option<(Self, usize)>, LspError> {
        if value.is_null() {
            return Ok(None);
        }
        let signatures = value
            .get("signatures")
            .and_then(Value::as_array)
            .ok_or(LspError::InvalidFormat("SignatureHelp.signatures"))?
            .iter()
            .map(SignatureInformation::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if signatures.is_empty() {
            return Ok(None);
        }
        // The LSP fallback is the first signature for absent or out-of-range
        // indices. The upstream view expects a valid index when supplied.
        let active = optional_index(value, "activeSignature")?
            .filter(|&index| index < signatures.len())
            .unwrap_or(0);
        Ok(Some((
            Self {
                signatures,
                wire: value.clone(),
                active_parameter: optional_index(value, "activeParameter")?,
            },
            active,
        )))
    }

    pub(super) fn same_labels(&self, other: &Self) -> bool {
        self.signatures
            .iter()
            .map(|entry| &entry.label)
            .eq(other.signatures.iter().map(|entry| &entry.label))
    }

    pub(super) fn parameter_index(&self, active: usize) -> Option<usize> {
        self.signatures[active]
            .active_parameter
            .or(self.active_parameter)
    }
}

impl SignatureInformation {
    fn parse(value: &Value) -> Result<Self, LspError> {
        let label = value
            .get("label")
            .and_then(Value::as_str)
            .ok_or(LspError::InvalidFormat("SignatureInformation.label"))?;
        let parameters = match optional(value, "parameters") {
            Some(parameters) => parameters
                .as_array()
                .ok_or(LspError::InvalidFormat("SignatureInformation.parameters"))?
                .iter()
                .map(|parameter| SignatureParameter::parse(label, parameter))
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            label: label.into(),
            documentation: parse_documentation(value)?,
            parameters,
            active_parameter: optional_index(value, "activeParameter")?,
        })
    }
}

impl SignatureParameter {
    fn parse(signature: &str, value: &Value) -> Result<Self, LspError> {
        let invalid = LspError::InvalidFormat("ParameterInformation.label");
        let (label, byte_range) = match value.get("label") {
            Some(Value::String(label)) => {
                let byte_range = signature
                    .find(label)
                    .map(|start| start..start + label.len());
                (label.clone(), byte_range)
            }
            Some(Value::Array(offsets)) => {
                let [start, end] = offsets.as_slice() else {
                    return Err(invalid);
                };
                let offset =
                    |value| unsigned_index(value).and_then(|n| utf16_boundary(signature, n));
                let start = offset(start).ok_or_else(|| invalid.clone())?;
                let end = offset(end).ok_or_else(|| invalid.clone())?;
                let range = start..end;
                let text = signature.get(range.clone()).ok_or(invalid)?;
                (text.into(), Some(range))
            }
            _ => return Err(invalid),
        };
        Ok(Self {
            label,
            byte_range,
            documentation: parse_documentation(value)?,
        })
    }
}

fn utf16_boundary(text: &str, mut remaining: usize) -> Option<usize> {
    for (byte, ch) in text.char_indices() {
        if remaining == 0 {
            return Some(byte);
        }
        remaining = remaining.checked_sub(ch.len_utf16())?;
    }
    (remaining == 0).then_some(text.len())
}

fn optional<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|field| !field.is_null())
}

fn optional_index(value: &Value, key: &'static str) -> Result<Option<usize>, LspError> {
    optional(value, key)
        .map(|field| unsigned_index(field).ok_or(LspError::InvalidFormat(key)))
        .transpose()
}

fn unsigned_index(value: &Value) -> Option<usize> {
    let n = i32::try_from(value.as_u64()?).ok()?;
    Some(n as usize)
}

fn parse_documentation(value: &Value) -> Result<Option<Documentation>, LspError> {
    optional(value, "documentation")
        .map(|docs| documentation(docs, false))
        .transpose()
}
