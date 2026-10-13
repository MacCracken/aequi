//! Strict argument extraction for tool handlers.
//!
//! A missing required argument, a value of the wrong type, or a fractional
//! number where whole cents are expected is an error, never a silent default.

use std::str::FromStr;

use aequi_core::Quarter;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
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

/// The exact decimal value of a JSON number, as written (`1.15` stays 1.15,
/// not the nearest binary float).
fn as_decimal(value: &Value) -> Option<Decimal> {
    let Value::Number(n) = value else {
        return None;
    };
    let text = n.to_string();
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .ok()
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

    /// An amount in whole cents for an update that can also clear a value:
    /// absent is `None` (leave unchanged), an explicit `null` is `Some(None)`
    /// (clear), and a number is `Some(Some(cents))`.
    pub fn nullable_cents(&self, name: &str) -> Result<Option<Option<i64>>, ToolError> {
        match self.map.and_then(|m| m.get(name)) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(_) => self.opt_cents(name).map(Some),
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

    pub fn f64(&self, name: &str) -> Result<f64, ToolError> {
        self.opt_f64(name)?.ok_or_else(|| self.missing(name))
    }

    pub fn opt_f64(&self, name: &str) -> Result<Option<f64>, ToolError> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|f| f.is_finite())
                .map(Some)
                .ok_or_else(|| {
                    ToolError(format!(
                        "Argument {} must be a number, got {v}",
                        self.label(name)
                    ))
                }),
        }
    }

    /// A required quantity with at most two decimal places, returned in
    /// hundredths (`1.5` becomes 150).
    pub fn hundredths(&self, name: &str) -> Result<i64, ToolError> {
        let value = self.get(name).ok_or_else(|| self.missing(name))?;
        let err = || {
            ToolError(format!(
                "Argument {} must be a number with at most 2 decimal places, got {value}",
                self.label(name)
            ))
        };
        let scaled = as_decimal(value).ok_or_else(err)? * Decimal::ONE_HUNDRED;
        if !scaled.fract().is_zero() {
            return Err(err());
        }
        scaled.to_i64().ok_or_else(err)
    }

    /// An optional nested object.
    pub fn opt_object(&self, name: &str) -> Result<Option<Args<'a>>, ToolError> {
        self.get(name)
            .map(|v| Args::object(v, self.label(name)))
            .transpose()
    }

    /// An optional array; absent means empty.
    pub fn opt_array(&self, name: &str) -> Result<&'a [Value], ToolError> {
        match self.get(name) {
            None => Ok(&[]),
            Some(_) => self.array(name).map(Vec::as_slice),
        }
    }

    /// Reject keys other than `allowed`, so a misspelled optional field in a
    /// nested object fails loudly instead of being ignored.
    pub fn only(&self, allowed: &[&str]) -> Result<(), ToolError> {
        let unknown: Vec<String> = self
            .map
            .into_iter()
            .flat_map(|m| m.keys())
            .filter(|k| !allowed.contains(&k.as_str()))
            .map(|k| self.label(k))
            .collect();
        if unknown.is_empty() {
            return Ok(());
        }
        Err(ToolError(format!(
            "Unknown argument(s): {}. Expected: {}",
            unknown.join(", "),
            allowed.join(", ")
        )))
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
    fn nullable_cents_distinguishes_absent_and_null() {
        let params = json!({ "a": 1250, "b": null, "c": 12.5, "d": "5" });
        let args = Args::new(&params);
        assert_eq!(args.nullable_cents("a").unwrap(), Some(Some(1250)));
        assert_eq!(args.nullable_cents("b").unwrap(), Some(None));
        assert_eq!(args.nullable_cents("zzz").unwrap(), None);
        assert!(args.nullable_cents("c").is_err());
        assert!(args.nullable_cents("d").is_err());
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
    fn hundredths_are_exact() {
        let params = json!({
            "a": 1.5, "b": 2, "c": 1.15, "d": 0.01, "e": 1.005, "f": "1.5",
            "g": 1e300, "h": 1e-7, "i": 100.10
        });
        let args = Args::new(&params);
        assert_eq!(args.hundredths("a").unwrap(), 150);
        assert_eq!(args.hundredths("b").unwrap(), 200);
        assert_eq!(args.hundredths("c").unwrap(), 115);
        assert_eq!(args.hundredths("d").unwrap(), 1);
        assert_eq!(args.hundredths("i").unwrap(), 10010);
        assert!(args.hundredths("e").is_err());
        assert!(args.hundredths("f").is_err());
        assert!(args.hundredths("g").is_err());
        assert!(args.hundredths("h").is_err());
        assert!(args.hundredths("zzz").unwrap_err().0.contains("Missing"));
    }

    #[test]
    fn floats_must_be_numbers() {
        let params = json!({ "a": 0.5, "b": 1, "c": "0.5" });
        let args = Args::new(&params);
        assert_eq!(args.f64("a").unwrap(), 0.5);
        assert_eq!(args.f64("b").unwrap(), 1.0);
        assert!(args.f64("c").is_err());
        assert_eq!(args.opt_f64("zzz").unwrap(), None);
    }

    #[test]
    fn unknown_nested_keys_rejected() {
        let line = json!({ "description": "x", "is_taxable": true });
        let args = Args::object(&line, "lines[0]".to_string()).unwrap();
        let err = args.only(&["description", "taxable"]).unwrap_err();
        assert!(err.0.contains("lines[0].is_taxable"), "{}", err.0);
        assert!(args.only(&["description", "is_taxable"]).is_ok());
    }

    #[test]
    fn optional_containers() {
        let params = json!({ "list": [1], "obj": { "x": 1 }, "bad": 3 });
        let args = Args::new(&params);
        assert_eq!(args.opt_array("list").unwrap().len(), 1);
        assert!(args.opt_array("none").unwrap().is_empty());
        assert!(args.opt_array("bad").is_err());
        let obj = args.opt_object("obj").unwrap().unwrap();
        assert_eq!(obj.i64("x").unwrap(), 1);
        assert!(args.opt_object("none").unwrap().is_none());
        assert!(args.opt_object("bad").is_err());
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
