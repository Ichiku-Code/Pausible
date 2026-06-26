// All arithmetic / comparison methods on Value fail with TypeError when
// operands have incompatible types. Suppressing per-method # Errors docs
// until the API surface stabilizes.
#![allow(clippy::missing_errors_doc)]

use core::fmt;

/// Runtime value in the Pausible VM.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

impl Value {
    // -- type predicates --

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

    // -- conversions --

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

    // -- arithmetic --

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

    // -- comparison --

    pub fn eq(&self, rhs: &Self) -> Result<Self, TypeError> {
        match (self, rhs) {
            (Self::Int(l), Self::Int(r)) => Ok(Self::Bool(l == r)),
            (Self::Float(l), Self::Float(r)) => Ok(Self::Bool(l == r)),
            (Self::Bool(l), Self::Bool(r)) => Ok(Self::Bool(l == r)),
            (Self::Null, Self::Null) => Ok(Self::Bool(true)),
            _ => Err(TypeError {
                op: "eq",
                lhs: self.clone(),
                rhs: rhs.clone(),
            }),
        }
    }

    pub fn neq(&self, rhs: &Self) -> Result<Self, TypeError> {
        self.eq(rhs).map(|v| Self::Bool(!v.as_bool().unwrap_or(true)))
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

    // -- logical --

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

    /// Returns truthy (non-zero, non-null, true).
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Bool(v) => *v,
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

    // -- type predicates --

    #[test]
    fn int_is_int() {
        assert!(Value::Int(42).is_int());
        assert!(!Value::Int(42).is_float());
        assert!(!Value::Int(42).is_bool());
        assert!(!Value::Int(42).is_null());
    }

    #[test]
    fn float_is_float() {
        assert!(Value::Float(3.14).is_float());
        assert!(!Value::Float(3.14).is_int());
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

    // -- conversions --

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

    // -- arithmetic --

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
        assert_eq!(Value::Float(3.14).neg(), Ok(Value::Float(-3.14)));
        assert!(Value::Bool(true).neg().is_err());
    }

    // -- comparison --

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

    // -- logical --

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

    // -- truthy --

    #[test]
    fn truthy() {
        assert!(Value::Int(1).is_truthy());
        assert!(!Value::Int(0).is_truthy());
        assert!(Value::Float(0.1).is_truthy());
        assert!(!Value::Float(0.0).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(!Value::Null.is_truthy());
    }

    // -- display --

    #[test]
    fn display() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Float(3.14)), "3.14");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Bool(false)), "false");
        assert_eq!(format!("{}", Value::Null), "null");
    }
}
