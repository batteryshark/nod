//! Closed, single-value answer contract. Decimal and calendar values stay strings.
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

/// With serde(default), absence is None but explicit null is rejected.
pub fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnswerValue {
    Choice { choice_id: String },
    Number { decimal: String },
    Date { date: String },
    DateTime { instant: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[typeshare]
#[serde(deny_unknown_fields)]
pub struct AnswerChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnswerField {
    Choice {
        label: String,
        choices: Vec<AnswerChoice>,
        default: AnswerValue,
    },
    Number {
        label: String,
        default: AnswerValue,
    },
    Date {
        label: String,
        default: AnswerValue,
    },
    DateTime {
        label: String,
        default: AnswerValue,
    },
}

impl AnswerField {
    pub fn label(&self) -> &str {
        match self {
            Self::Choice { label, .. }
            | Self::Number { label, .. }
            | Self::Date { label, .. }
            | Self::DateTime { label, .. } => label,
        }
    }

    pub fn default_value(&self) -> &AnswerValue {
        match self {
            Self::Choice { default, .. }
            | Self::Number { default, .. }
            | Self::Date { default, .. }
            | Self::DateTime { default, .. } => default,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_label(self.label(), 64) {
            return Err("invalid answer label");
        }
        if serde_json::to_vec(self)
            .map_err(|_| "invalid answer definition")?
            .len()
            > 4096
        {
            return Err("answer definition exceeds 4096 bytes");
        }
        if let Self::Choice { choices, .. } = self {
            if !(2..=12).contains(&choices.len()) {
                return Err("choice count must be 2–12");
            }
            for (i, choice) in choices.iter().enumerate() {
                if choice.id.is_empty()
                    || choice.id.len() > 40
                    || !choice
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    || !valid_label(&choice.label, 48)
                    || choices[..i]
                        .iter()
                        .any(|old| old.id == choice.id || old.label == choice.label)
                {
                    return Err("invalid or duplicate choice");
                }
            }
        }
        self.validate_value(self.default_value())
    }

    pub fn validate_value(&self, value: &AnswerValue) -> Result<(), &'static str> {
        let valid = match (self, value) {
            (Self::Choice { choices, .. }, AnswerValue::Choice { choice_id }) => {
                choices.iter().any(|c| &c.id == choice_id)
            }
            (Self::Number { .. }, AnswerValue::Number { decimal }) => valid_decimal(decimal),
            (Self::Date { .. }, AnswerValue::Date { date }) => valid_date(date),
            (Self::DateTime { .. }, AnswerValue::DateTime { instant }) => valid_instant(instant),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("answer value does not match definition")
        }
    }
}

fn valid_label(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= max
        && !value.chars().any(char::is_control)
}

fn valid_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or("");
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || integer.len() > 12
        || !integer.bytes().all(|c| c.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
    {
        return false;
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 6
            || fraction.ends_with('0')
            || !fraction.bytes().all(|c| c.is_ascii_digit())
        {
            return false;
        }
    }
    !(value.starts_with('-') && integer == "0" && fraction.is_none())
}

fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[..4].parse().unwrap_or(0);
    let month: u32 = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..].parse().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    };
    year != 0 && day != 0 && day <= days
}

fn valid_instant(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 20
        && b.is_ascii()
        && valid_date(&value[..10])
        && b[10] == b'T'
        && b[13] == b':'
        && &b[16..] == b":00Z"
        && b[11..13].iter().chain(&b[14..16]).all(u8::is_ascii_digit)
        && value[11..13].parse::<u8>().is_ok_and(|h| h < 24)
        && value[14..16].parse::<u8>().is_ok_and(|m| m < 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_and_calendar_validation() {
        for v in ["4", "1.5", "-0.25", "999999999999.123456", "0"] {
            assert!(valid_decimal(v), "{v}");
        }
        for v in [
            "1.50",
            "-0",
            "-0.0",
            "01",
            "+1",
            "1e2",
            " 4",
            "NaN",
            "1000000000000",
            "0.1234567",
        ] {
            assert!(!valid_decimal(v), "{v}");
        }
        assert!(valid_date("2028-02-29"));
        assert!(!valid_date("2027-02-29"));
        assert!(!valid_date("0000-01-01"));
        assert!(valid_instant("2026-10-25T09:30:00Z"));
        assert!(!valid_instant("2026-10-25T09:30:01Z"));
        assert!(!valid_instant("2026-10-25T24:30:00Z"));
    }

    #[test]
    fn closed_definition_and_matching_default() {
        let raw = r#"{"kind":"choice","label":"Batch","choices":[{"id":"small","label":"Small"},{"id":"large","label":"Large"}],"default":{"kind":"choice","choice_id":"small"}}"#;
        let field: AnswerField = serde_json::from_str(raw).unwrap();
        assert!(field.validate().is_ok());
        assert!(field
            .validate_value(&AnswerValue::Choice {
                choice_id: "large".into()
            })
            .is_ok());
        assert!(field
            .validate_value(&AnswerValue::Number {
                decimal: "4".into()
            })
            .is_err());
        assert!(serde_json::from_str::<AnswerField>(
            &raw.replace("\"Batch\"", "\"Batch\",\"scale\":0")
        )
        .is_err());
        assert!(serde_json::from_str::<AnswerField>(&raw.replace(
            "\"default\":{\"kind\":\"choice\",\"choice_id\":\"small\"}",
            "\"default\":null"
        ))
        .is_err());
        let ordinary: crate::CreateDecisionRequest =
            serde_json::from_str(r#"{"title":"Ordinary"}"#).unwrap();
        assert!(!serde_json::to_value(ordinary)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("answer_field"));
        for invalid in [
            r#"{"title":"x","answer_field":null}"#,
            r#"{"title":"x","answer_field":[]}"#,
            r#"{"title":"x","answer_fields":[]}"#,
        ] {
            assert!(serde_json::from_str::<crate::CreateDecisionRequest>(invalid).is_err());
        }
        assert!(serde_json::from_str::<AnswerField>(r#"{"kind":"number","label":"Count","scale":0,"default":{"kind":"number","decimal":"4"}}"#).is_err());
    }
}
