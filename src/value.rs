// All arithmetic / comparison methods on Value fail with TypeError when
// operands have incompatible types. Suppressing per-method # Errors docs
// until the API surface stabilizes.
#![allow(clippy::missing_errors_doc)]

use core::fmt;

use crate::heap::{Gc, ListObj, StringObj};

/// Runtime value in the Pausible VM.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    /// Heap-allocated UTF-8 string (identity semantics for equality).
    String(Gc<StringObj>),
    /// Heap-allocated dynamic list (identity semantics for equality).
    List(Gc<ListObj>),
}

// Manual PartialEq: String/List compare by Gc handle identity.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(l), Self::Int(r)) => l == r,
            (Self::Float(l), Self::Float(r)) => l == r,
            (Self::Bool(l), Self::Bool(r)) => l == r,
            (Self::Null, Self::Null) => true,
            (Self::String(l), Self::String(r)) => l == r,
            (Self::List(l), Self::List(r)) => l == r,
            _ => false,
        }
    }
}

impl Value {
    #[must_use]
    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }
    #[must_use]
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }
    #[must_use]
    pub fn is_bool(&self) -> bool {
        matches!(self, Self::Bool(_))
    }
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    #[must_use]
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }
    #[must_use]
    pub fn is_list(&self) -> bool {
        matches!(self, Self::List(_))
    }

    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_string(&self) -> Option<Gc<StringObj>> {
        match self {
            Self::String(v) => Some(*v),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_list(&self) -> Option<Gc<ListObj>> {
        match self {
            Self::List(v) => Some(*v),
            _ => None,
        }
    }

    pub fn add(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Int(l + r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Float(l + r)),
            _ => Err(TypeError {
                op: "add",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn sub(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Int(l - r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Float(l - r)),
            _ => Err(TypeError {
                op: "sub",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn mul(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Int(l * r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Float(l * r)),
            _ => Err(TypeError {
                op: "mul",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn div(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Int(l / r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Float(l / r)),
            _ => Err(TypeError {
                op: "div",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn modulo(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Int(l % r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Float(l % r)),
            _ => Err(TypeError {
                op: "mod",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn neg(&self) -> Result<Self, TypeError> {
        match self {
            Self::Int(v) => Ok(Self::Int(-v)),
            Self::Float(v) => Ok(Self::Float(-v)),
            other => Err(TypeError {
                op: "neg",
                lhs: other.clone(),
                rhs: Value::Null,
            }),
        }
    }

    pub fn eq(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l == r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l == r)),
            (Self::Bool(l), Self::Bool(r)) => Ok(Self::Bool(l == r)),
            (Self::Null, Self::Null) => Ok(Self::Bool(true)),
            (Self::String(l), Self::String(r)) => Ok(Self::Bool(l == r)),
            (Self::List(l), Self::List(r)) => Ok(Self::Bool(l == r)),
            _ => Err(TypeError {
                op: "eq",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn neq(&self, rhs: &Self) -> Result<Self, TypeError> {
        self.eq(rhs)
            .map(|v| Self::Bool(!v.as_bool().unwrap_or(true)))
    }
    pub fn lt(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l < r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l < r)),
            _ => Err(TypeError {
                op: "lt",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn lte(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l <= r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l <= r)),
            _ => Err(TypeError {
                op: "lte",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn gt(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l > r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l > r)),
            _ => Err(TypeError {
                op: "gt",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn gte(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l >= r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l >= r)),
            _ => Err(TypeError {
                op: "gte",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }

    pub fn and(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Bool(l), Self::Bool(r)) => Ok(Self::Bool(*l && *r)),
            _ => Err(TypeError {
                op: "and",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn or(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Bool(l), Self::Bool(r)) => Ok(Self::Bool(*l || *r)),
            _ => Err(TypeError {
                op: "or",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }
    pub fn not(&self) -> Result<Self, TypeError> {
        match self {
            Self::Bool(v) => Ok(Self::Bool(!v)),
            other => Err(TypeError {
                op: "not",
                lhs: other.clone(),
                rhs: Value::Null,
            }),
        }
    }

    /// Returns truthy (non-zero, non-null, true); heap refs are always truthy.
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Bool(v) => *v,
            Self::String(_) | Self::List(_) => true,
            Self::Null => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(v) => write!(f, "{v}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Null => write!(f, "null"),
            Self::String(gc) => write!(f, "<string@{}>", gc.index),
            Self::List(gc) => write!(f, "<list@{}>", gc.index),
        }
    }
}

/// Error returned when a binary operation receives incompatible types.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeError {
    pub op: &'static str,
    pub lhs: Value,
    pub rhs: Value,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "type error: cannot {} {} and {}",
            self.op, self.lhs, self.rhs
        )
    }
}

impl core::error::Error for TypeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::Heap;

    fn heap_with_str(s: &str) -> (Heap, Gc<StringObj>) {
        let mut heap = Heap::new();
        let gc = heap.alloc_string(s.into());
        (heap, gc)
    }

    fn heap_with_list(vals: Vec<Value>) -> (Heap, Gc<ListObj>) {
        let mut heap = Heap::new();
        let gc = heap.alloc_list(vals);
        (heap, gc)
    }

    #[test]
    fn string_is_string() {
        let (_heap, gc) = heap_with_str("hello");
        assert!(Value::String(gc).is_string());
        assert!(!Value::String(gc).is_int());
    }

    #[test]
    fn list_is_list() {
        let (_heap, gc) = heap_with_list(vec![]);
        assert!(Value::List(gc).is_list());
        assert!(!Value::List(gc).is_float());
    }

    #[test]
    fn as_string_conversion() {
        let (_heap, gc) = heap_with_str("hi");
        let val = Value::String(gc);
        assert_eq!(val.as_string(), Some(gc));
        assert_eq!(Value::Int(1).as_string(), None);
    }

    #[test]
    fn as_list_conversion() {
        let (_heap, gc) = heap_with_list(vec![]);
        let val = Value::List(gc);
        assert_eq!(val.as_list(), Some(gc));
        assert_eq!(Value::Null.as_list(), None);
    }

    #[test]
    fn string_eq_by_identity() {
        let mut heap = Heap::new();
        let a = heap.alloc_string("abc".into());
        let b = heap.alloc_string("abc".into());
        assert_eq!(
            Value::String(a).eq(&Value::String(a)),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            Value::String(a).eq(&Value::String(b)),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn list_eq_by_identity() {
        let mut heap = Heap::new();
        let a = heap.alloc_list(vec![Value::Int(1)]);
        let b = heap.alloc_list(vec![Value::Int(1)]);
        assert_eq!(Value::List(a).eq(&Value::List(a)), Ok(Value::Bool(true)));
        assert_eq!(Value::List(a).eq(&Value::List(b)), Ok(Value::Bool(false)));
    }

    #[test]
    fn heap_types_always_truthy() {
        let (_heap, str_gc) = heap_with_str("");
        let (_heap, list_gc) = heap_with_list(vec![]);
        assert!(Value::String(str_gc).is_truthy());
        assert!(Value::List(list_gc).is_truthy());
    }

    #[test]
    fn display_heap_types() {
        let mut heap = Heap::new();
        let str_gc = heap.alloc_string("hello".into());
        let list_gc = heap.alloc_list(vec![]);
        assert_eq!(format!("{}", Value::String(str_gc)), "<string@0>");
        assert_eq!(format!("{}", Value::List(list_gc)), "<list@1>");
    }

    // -- original value tests (Int, Float, Bool, Null) --

    #[test]
    fn int_is_int() {
        assert!(Value::Int(42).is_int());
        assert!(!Value::Int(42).is_float());
        assert!(!Value::Int(42).is_bool());
        assert!(!Value::Int(42).is_null());
    }

    #[test]
    fn float_is_float() {
        assert!(Value::Float(1.5).is_float());
        assert!(!Value::Float(1.5).is_int());
    }

    #[test]
    fn bool_is_bool() {
        assert!(Value::Bool(true).is_bool());
        assert!(!Value::Bool(true).is_null());
    }

    #[test]
    fn null_is_null() {
        assert!(Value::Null.is_null());
        assert!(!Value::Null.is_int());
    }

    #[test]
    fn as_int_conversion() {
        assert_eq!(Value::Int(7).as_int(), Some(7));
        assert_eq!(Value::Float(1.0).as_int(), None);
        assert_eq!(Value::Null.as_int(), None);
    }

    #[test]
    fn as_float_conversion() {
        assert_eq!(Value::Float(2.5).as_float(), Some(2.5));
        assert_eq!(Value::Int(3).as_float(), None);
    }

    #[test]
    fn as_bool_conversion() {
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Int(1).as_bool(), None);
    }

    #[test]
    fn int_add() {
        assert_eq!(Value::Int(2).add(&Value::Int(3)), Ok(Value::Int(5)));
    }

    #[test]
    fn float_add() {
        assert_eq!(
            Value::Float(1.5).add(&Value::Float(2.5)),
            Ok(Value::Float(4.0))
        );
    }

    #[test]
    fn add_type_mismatch_fails() {
        assert!(Value::Int(1).add(&Value::Float(1.0)).is_err());
        assert!(Value::Bool(true).add(&Value::Bool(false)).is_err());
    }

    #[test]
    fn int_sub_mul_div() {
        assert_eq!(Value::Int(10).sub(&Value::Int(3)), Ok(Value::Int(7)));
        assert_eq!(Value::Int(4).mul(&Value::Int(5)), Ok(Value::Int(20)));
        assert_eq!(Value::Int(20).div(&Value::Int(4)), Ok(Value::Int(5)));
    }

    #[test]
    fn float_sub_mul_div() {
        assert_eq!(
            Value::Float(5.0).sub(&Value::Float(2.0)),
            Ok(Value::Float(3.0))
        );
        assert_eq!(
            Value::Float(3.0).mul(&Value::Float(4.0)),
            Ok(Value::Float(12.0))
        );
        assert_eq!(
            Value::Float(10.0).div(&Value::Float(2.0)),
            Ok(Value::Float(5.0))
        );
    }

    #[test]
    fn modulo() {
        assert_eq!(Value::Int(10).modulo(&Value::Int(3)), Ok(Value::Int(1)));
        assert_eq!(
            Value::Float(5.5).modulo(&Value::Float(2.0)),
            Ok(Value::Float(1.5))
        );
    }

    #[test]
    fn negate() {
        assert_eq!(Value::Int(5).neg(), Ok(Value::Int(-5)));
        assert_eq!(Value::Float(1.5).neg(), Ok(Value::Float(-1.5)));
        assert!(Value::Bool(true).neg().is_err());
    }

    #[test]
    fn eq_comparison() {
        assert_eq!(Value::Int(1).eq(&Value::Int(1)), Ok(Value::Bool(true)));
        assert_eq!(Value::Int(1).eq(&Value::Int(2)), Ok(Value::Bool(false)));
        assert_eq!(
            Value::Float(1.0).eq(&Value::Float(1.0)),
            Ok(Value::Bool(true))
        );
        assert_eq!(Value::Null.eq(&Value::Null), Ok(Value::Bool(true)));
        assert!(Value::Int(1).eq(&Value::Null).is_err());
    }

    #[test]
    fn comparison_lt_lte_gt_gte() {
        assert_eq!(Value::Int(1).lt(&Value::Int(2)), Ok(Value::Bool(true)));
        assert_eq!(Value::Int(2).lt(&Value::Int(1)), Ok(Value::Bool(false)));
        assert_eq!(Value::Int(2).lte(&Value::Int(2)), Ok(Value::Bool(true)));
        assert_eq!(Value::Int(3).gt(&Value::Int(2)), Ok(Value::Bool(true)));
        assert_eq!(Value::Int(3).gte(&Value::Int(3)), Ok(Value::Bool(true)));
    }

    #[test]
    fn comparison_type_mismatch_fails() {
        assert!(Value::Int(1).lt(&Value::Float(1.0)).is_err());
        assert!(Value::Bool(true).gt(&Value::Bool(false)).is_err());
    }

    #[test]
    fn logical_and_or_not() {
        assert_eq!(
            Value::Bool(true).and(&Value::Bool(true)),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            Value::Bool(true).and(&Value::Bool(false)),
            Ok(Value::Bool(false))
        );
        assert_eq!(
            Value::Bool(false).or(&Value::Bool(true)),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            Value::Bool(false).or(&Value::Bool(false)),
            Ok(Value::Bool(false))
        );
        assert_eq!(Value::Bool(true).not(), Ok(Value::Bool(false)));
        assert_eq!(Value::Bool(false).not(), Ok(Value::Bool(true)));
    }

    #[test]
    fn truthy_basic() {
        assert!(Value::Int(1).is_truthy());
        assert!(!Value::Int(0).is_truthy());
        assert!(Value::Float(0.1).is_truthy());
        assert!(!Value::Float(0.0).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(!Value::Null.is_truthy());
    }

    #[test]
    fn display_basic() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Float(1.5)), "1.5");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Bool(false)), "false");
        assert_eq!(format!("{}", Value::Null), "null");
    }
}
