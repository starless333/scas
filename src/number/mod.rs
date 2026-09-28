use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Zero};
use std::cmp::Ordering;
use std::fmt;

/// Runtime numeric values. Integers remain exact and use an unbounded
/// magnitude. Decimals store an unbounded integer coefficient and a decimal
/// scale, so literals and finite arithmetic never pass through a float.
#[derive(Debug, Clone)]
pub enum Number {
    Integer(BigInt),
    Rational(BigRational),
    Decimal(BigDecimal),
}

impl Number {
    pub fn from_rational(value: BigRational) -> Number {
        if value.denom().is_one() {
            Number::Integer(value.to_integer())
        } else {
            Number::Rational(value)
        }
    }

    pub fn to_rational(&self) -> BigRational {
        self.try_to_rational()
            .expect("decimal scale is too large to materialize as a rational")
    }

    fn try_to_rational(&self) -> Option<BigRational> {
        match self {
            Number::Integer(value) => Some(BigRational::from_integer(value.clone())),
            Number::Rational(value) => Some(value.clone()),
            Number::Decimal(value) => {
                let (coefficient, scale) = value.as_bigint_and_exponent();
                let exponent = u32::try_from(scale.unsigned_abs()).ok()?;
                let power = BigInt::from(10u8).pow(exponent);
                if scale >= 0 {
                    Some(BigRational::new(coefficient, power))
                } else {
                    Some(BigRational::from_integer(coefficient * power))
                }
            }
        }
    }

    pub fn to_decimal(&self, context: &Context) -> BigDecimal {
        match self {
            Number::Integer(value) => BigDecimal::from(value.clone()),
            Number::Rational(value) => {
                let numerator = BigDecimal::from(value.numer().clone());
                let denominator = BigDecimal::from(value.denom().clone());
                let reciprocal = context.invert(&denominator);
                context.multiply(&numerator, &reciprocal)
            }
            Number::Decimal(value) => value.clone(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_zero(),
            Number::Rational(value) => value.is_zero(),
            Number::Decimal(value) => value.is_zero(),
        }
    }

    pub fn is_one(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_one(),
            Number::Rational(value) => value.is_one(),
            Number::Decimal(value) => value.is_one(),
        }
    }

    pub fn compare(&self, other: &Number) -> Ordering {
        match (self, other) {
            (Number::Integer(a), Number::Integer(b)) => a.cmp(b),
            (Number::Decimal(a), Number::Decimal(b)) => a.cmp(b),
            _ => match (self.try_to_rational(), other.try_to_rational()) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => {
                    let context = Context::default();
                    self.to_decimal(&context).cmp(&other.to_decimal(&context))
                }
            },
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::Integer(value) => value.fmt(formatter),
            Number::Rational(value) => write!(formatter, "{}/{}", value.numer(), value.denom()),
            Number::Decimal(value) => value.normalized().fmt(formatter),
        }
    }
}
