//! Strict argument extraction for tool handlers.
//!
//! A missing required argument, a value of the wrong type, or a fractional
//! number where whole cents are expected is an error, never a silent default.

use aequi_core::Quarter;
use serde_json::{Map, Value};

use crate::protocol::ToolResult;

/// A tool failure, reported to the client as an `isError` tool result.
#[derive(Debug)]
pub struct ToolError(pub String);

pub type ToolOutcome = Result<ToolResult, ToolError>;

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        ToolError(message.into())
    }
}

impl From<sqlx::Error> for ToolError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::InvalidArgument(msg) => ToolError(msg),
            sqlx::Error::RowNotFound => ToolError("Not found".to_string()),
            sqlx::Error::Database(db) if db.is_unique_violation() => ToolError(format!(
                "A record with the same unique value already exists ({db})"
            )),
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                ToolError("The request references a record that does not exist".to_string())
            }
            other => ToolError(format!("Database error: {other}")),
        }
    }
}

/// Map `RowNotFound` to "`what` not found"; other errors as usual.
pub fn not_found(what: impl Into<String>) -> impl FnOnce(sqlx::Error) -> ToolError {
    let what = what.into();
    move |e| match e {
        sqlx::Error::RowNotFound => ToolError(format!("{what} not found")),
        other => ToolError::from(other),
    }
}

/// Integers are accepted as JSON integers, or as floats with no fractional
/// part that are exactly representable (|x| <= 2^53), e.g. `5000.0`.
fn as_integer(value: &Value) -> Option<i64> {
    if let Some(i) = value.as_i64() {
        return Some(i);
    }
    const MAX_EXACT: f64 = 9_007_199_254_740_992.0;
    let f = value.as_f64()?;
    (f.fract() == 0.0 && f.abs() <= MAX_EXACT).then_some(f as i64)
}

pub struct Args<'a> {
    map: Option<&'a Map<String, Value>>,
    prefix: Option<String>,
}

impl<'a> Args<'a> {
    pub fn new(params: &'a Value) -> Self {
        Args {
            map: params.as_object(),
            prefix: None,
        }
    }

    /// Arguments of a nested object such as one transaction line; `label` is
    /// used in error messages (e.g. `lines[2]`).
    pub fn object(value: &'a Value, label: String) -> Result<Self, ToolError> {
        match value.as_object() {
            Some(map) => Ok(Args {
                map: Some(map),
                prefix: Some(label),
            }),
            None => Err(ToolError(format!("{label} must be an object"))),
        }
    }

    fn label(&self, name: &str) -> String {
        match &self.prefix {
            Some(p) => format!("{p}.{name}"),
            None => name.to_string(),
        }
    }

    /// The argument's value, treating an explicit `null` as absent.
    fn get(&self, name: &str) -> Option<&'a Value> {
        self.map.and_then(|m| m.get(name)).filter(|v| !v.is_null())
    }

    fn missing(&self, name: &str) -> ToolError {
        ToolError(format!("Missing required argument: {}", self.label(name)))
    }

    /// A required, non-blank string.
    pub fn str(&self, name: &str) -> Result<&'a str, ToolError> {
        self.opt_str(name)?.ok_or_else(|| self.missing(name))
    }

    /// An optional string; a blank string counts as absent.
    pub fn opt_str(&self, name: &str) -> Result<Option<&'a str>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(other) => Err(ToolError(format!(
                "Argument {} must be a string, got {other}",
                self.label(name)
            ))),
        }
    }

    pub fn i64(&self, name: &str) -> Result<i64, ToolError> {
        self.opt_i64(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_i64(&self, name: &str) -> Result<Option<i64>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => as_integer(v).map(Some).ok_or_else(|| {
                ToolError(format!(
                    "Argument {} must be an integer, got {v}",
                    self.label(name)
                ))
            }),
        }
    }

    /// A required amount in whole cents.
    pub fn cents(&self, name: &str) -> Result<i64, ToolError> {
        self.opt_cents(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_cents(&self, name: &str) -> Result<Option<i64>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => as_integer(v).map(Some).ok_or_else(|| {
                ToolError(format!(
                    "Argument {} must be a whole number of cents, got {v}",
                    self.label(name)
                ))
            }),
        }
    }

    /// A required database id (a positive integer).
    pub fn id(&self, name: &str) -> Result<i64, ToolError> {
        self.opt_id(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_id(&self, name: &str) -> Result<Option<i64>, ToolError> {
        match self.opt_i64(name)? {
            Some(id) if id <= 0 => Err(ToolError(format!(
                "Argument {} must be a positive id, got {id}",
                self.label(name)
            ))),
            other => Ok(other),
        }
    }

    pub fn opt_bool(&self, name: &str) -> Result<Option<bool>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(other) => Err(ToolError(format!(
                "Argument {} must be a boolean, got {other}",
                self.label(name)
            ))),
        }
    }

    /// A required `YYYY-MM-DD` date.
    pub fn date(&self, name: &str) -> Result<&'a str, ToolError> {
        self.opt_date(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_date(&self, name: &str) -> Result<Option<&'a str>, ToolError> {
        let Some(value) = self.opt_str(name)? else {
            return Ok(None);
        };
        aequi_storage::validate_date(&self.label(name), value)?;
        Ok(Some(value))
    }

    pub fn array(&self, name: &str) -> Result<&'a Vec<Value>, ToolError> {
        match self.get(name) {
            None => Err(self.missing(name)),
            Some(Value::Array(items)) => Ok(items),
            Some(other) => Err(ToolError(format!(
                "Argument {} must be an array, got {other}",
                self.label(name)
            ))),
        }
    }

    /// A required quarter number, 1–4.
    pub fn quarter(&self, name: &str) -> Result<Quarter, ToolError> {
        let n = self.i64(name)?;
        u8::try_from(n).ok().and_then(Quarter::new).ok_or_else(|| {
            ToolError(format!(
                "Argument {} must be a quarter from 1 to 4, got {n}",
                self.label(name)
            ))
        })
    }

    pub fn year(&self, name: &str) -> Result<u16, ToolError> {
        self.opt_year(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_year(&self, name: &str) -> Result<Option<u16>, ToolError> {
        match self.opt_i64(name)? {
            None => Ok(None),
            Some(y) if (2000..=2100).contains(&y) => Ok(Some(y as u16)),
            Some(y) => Err(ToolError(format!(
                "Argument {} must be a year between 2000 and 2100, got {y}",
                self.label(name)
            ))),
        }
    }
}

/// Reject a range whose start is after its end. Both are validated
/// `YYYY-MM-DD` strings, which compare correctly as text.
pub fn check_date_order(start: &str, end: &str) -> Result<(), ToolError> {
    if start > end {
        return Err(ToolError(format!(
            "start_date ({start}) must not be after end_date ({end})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integers_reject_fractions_and_strings() {
        let params = json!({ "a": 5000, "b": 5000.0, "c": 49.99, "d": "12", "e": null });
        let args = Args::new(&params);
        assert_eq!(args.i64("a").unwrap(), 5000);
        assert_eq!(args.i64("b").unwrap(), 5000);
        assert!(args.i64("c").is_err());
        assert!(args.i64("d").is_err());
        assert!(args.i64("e").unwrap_err().0.contains("Missing"));
        assert!(args.i64("zzz").unwrap_err().0.contains("Missing"));
        assert_eq!(args.opt_i64("e").unwrap(), None);
    }

    #[test]
    fn huge_numbers_rejected() {
        let params = json!({ "a": 1e300, "b": u64::MAX });
        let args = Args::new(&params);
        assert!(args.i64("a").is_err());
        assert!(args.i64("b").is_err());
    }

    #[test]
    fn strings_must_be_non_blank() {
        let params = json!({ "a": "x", "b": "  ", "c": 5 });
        let args = Args::new(&params);
        assert_eq!(args.str("a").unwrap(), "x");
        assert!(args.str("b").is_err());
        assert!(args.str("c").is_err());
        assert_eq!(args.opt_str("b").unwrap(), None);
    }

    #[test]
    fn quarter_and_year_ranges() {
        let params = json!({ "q1": 1, "q5": 5, "q0": 0, "qn": -1, "y": 2026, "ybig": 67562 });
        let args = Args::new(&params);
        assert_eq!(args.quarter("q1").unwrap(), Quarter::Q1);
        assert!(args.quarter("q5").is_err());
        assert!(args.quarter("q0").is_err());
        assert!(args.quarter("qn").is_err());
        assert_eq!(args.year("y").unwrap(), 2026);
        assert!(args.year("ybig").is_err());
    }

    #[test]
    fn dates_validated() {
        let params = json!({ "ok": "2026-03-01", "bad": "3/1/2026", "short": "2026-3-1" });
        let args = Args::new(&params);
        assert_eq!(args.date("ok").unwrap(), "2026-03-01");
        assert!(args.date("bad").is_err());
        assert!(args.date("short").is_err());
        assert_eq!(args.opt_date("missing").unwrap(), None);
    }

    #[test]
    fn ids_must_be_positive() {
        let params = json!({ "a": 0, "b": -3, "c": 7 });
        let args = Args::new(&params);
        assert!(args.id("a").is_err());
        assert!(args.id("b").is_err());
        assert_eq!(args.id("c").unwrap(), 7);
    }

    #[test]
    fn nested_labels_in_errors() {
        let line = json!({ "debit_cents": 1.5 });
        let args = Args::object(&line, "lines[0]".to_string()).unwrap();
        let err = args.cents("debit_cents").unwrap_err();
        assert!(err.0.contains("lines[0].debit_cents"), "{}", err.0);
        assert!(Args::object(&json!(3), "lines[1]".to_string()).is_err());
    }
}
