use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};
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

    pub fn is_negative(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_negative(),
            Number::Rational(value) => value.is_negative(),
            Number::Decimal(value) => value.is_negative(),
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

    pub fn abs(&self) -> Number {
        match self {
            Number::Integer(value) => Number::Integer(value.abs()),
            Number::Rational(value) => Number::Rational(value.abs()),
            Number::Decimal(value) => Number::Decimal(value.abs()),
        }
    }

    pub fn to_f64(&self) -> Option<f64> {
        match self {
            Number::Integer(value) => value.to_f64(),
            Number::Rational(value) => value.to_f64(),
            Number::Decimal(value) => value.to_f64(),
        }
    }

    pub fn from_f64(value: f64) -> Option<Number> {
        if !value.is_finite() {
            return None;
        }
        if value.fract() == 0.0 {
            if let Some(integer) = BigInt::from_f64(value) {
                return Some(Number::Integer(integer));
            }
        }
        format!("{value:.15e}")
            .parse::<BigDecimal>()
            .ok()
            .map(Number::Decimal)
    }

    pub fn exact_root(&self, degree: u32) -> Option<Number> {
        fn integer_root(value: &BigInt, degree: u32) -> Option<BigInt> {
            if degree % 2 == 0 && value.is_negative() {
                return None;
            }
            let root = value.nth_root(degree);
            (root.pow(degree) == *value).then_some(root)
        }

        let rational = self.try_to_rational()?;
        let numerator = integer_root(rational.numer(), degree)?;
        let denominator = integer_root(rational.denom(), degree)?;
        Some(Number::from_rational(BigRational::new(
            numerator,
            denominator,
        )))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_rational_arithmetic_obeys_randomized_inverse_invariants() {
        // A deterministic generator keeps this property test reproducible and
        // dependency-free while covering zero and negative boundaries.
        let mut state = 0x5eed_u64;
        for _ in 0..2_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let a = (state as i64 % 2_001) - 1_000;
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let mut b = (state as i64 % 2_001) - 1_000;
            if b == 0 {
                b = 1;
            }
            let value = BigRational::new(BigInt::from(a), BigInt::from(b));
            assert_eq!((&value + &value) - &value, value);
            if !value.is_zero() {
                assert_eq!(&value * value.recip(), BigRational::one());
            }
        }
    }

    #[test]
    fn equality_and_formatting_cross_numeric_representations() {
        let half = Number::from_rational(BigRational::new(1.into(), 2.into()));
        let decimal = Number::Decimal("0.5".parse().unwrap());
        assert_eq!(half, decimal);
        assert_eq!(half.to_string(), "1/2");
        assert_eq!(
            Number::from_rational(BigRational::new(6.into(), 3.into())).to_string(),
            "2"
        );
    }
}
