//! Key kinds, defaults and values.

use serde_json::{Value as Json, json};
use serde_yaml::Value as Yaml;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Text,
    List,
    BoolOrInt,
}

#[derive(Debug, Clone, Copy)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'static str),
    List(&'static [&'static str]),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<String>),
    BoolOrInt(BoolOrInt),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOrInt {
    Bool(bool),
    Int(i64),
}

#[derive(Debug)]
pub struct Key {
    pub full: &'static str,
    pub head: &'static str,
    pub path: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub default: DefaultValue,
    pub comment: &'static str,
    pub queryable: bool,
}

impl DefaultValue {
    pub fn value(self) -> Value {
        match self {
            DefaultValue::Bool(b) => Value::Bool(b),
            DefaultValue::Int(i) => Value::Int(i),
            DefaultValue::Float(f) => Value::Float(f),
            DefaultValue::Text(s) => Value::Text(s.to_string()),
            DefaultValue::List(items) => Value::List(items.iter().map(|s| s.to_string()).collect()),
            DefaultValue::Null => Value::Null,
        }
    }
}

impl Key {
    pub fn default_value(&self) -> Value {
        self.default.value()
    }

    pub fn parse(&self, raw: &Yaml) -> Result<Value, String> {
        if raw.is_null() {
            return match self.default {
                DefaultValue::Null => Ok(Value::Null),
                _ => Err("null is not a value here".into()),
            };
        }
        match self.kind {
            Kind::Bool => raw
                .as_bool()
                .map(Value::Bool)
                .ok_or_else(|| "expected true or false".into()),
            Kind::Int => raw
                .as_i64()
                .map(Value::Int)
                .ok_or_else(|| "expected an integer".into()),
            Kind::Float => raw
                .as_f64()
                .map(Value::Float)
                .ok_or_else(|| "expected a number".into()),
            Kind::Text => raw
                .as_str()
                .map(|s| Value::Text(s.to_string()))
                .ok_or_else(|| "expected text".into()),
            Kind::List => {
                let seq = raw
                    .as_sequence()
                    .ok_or_else(|| "expected a list of text".to_string())?;
                let mut out = Vec::with_capacity(seq.len());
                for item in seq {
                    let s = item
                        .as_str()
                        .ok_or_else(|| "expected a list of text".to_string())?;
                    out.push(s.to_string());
                }
                Ok(Value::List(out))
            }
            Kind::BoolOrInt => {
                if let Some(b) = raw.as_bool() {
                    Ok(Value::BoolOrInt(BoolOrInt::Bool(b)))
                } else if let Some(i) = raw.as_i64() {
                    Ok(Value::BoolOrInt(BoolOrInt::Int(i)))
                } else {
                    Err("expected true/false or an integer".into())
                }
            }
        }
    }
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Value::Bool(b) => json!(b),
            Value::Int(i) => json!(i),
            Value::Float(f) => json!(f),
            Value::Text(s) => json!(s),
            Value::List(items) => json!(items),
            Value::BoolOrInt(BoolOrInt::Bool(b)) => json!(b),
            Value::BoolOrInt(BoolOrInt::Int(i)) => json!(i),
            Value::Null => Json::Null,
        }
    }
}
