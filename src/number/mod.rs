use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::cmp::Ordering;
use std::fmt;

mod transcendental;

pub use transcendental::{
    BinaryMathFunction, MathConstant, MathEvaluationError, UnaryMathFunction,
};

/// An approximate complex number. Both components are arbitrary-precision
/// decimals rounded under the interpreter's active decimal context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexNumber {
    pub real: BigDecimal,
    pub imaginary: BigDecimal,
}

impl ComplexNumber {
    pub fn new(real: BigDecimal, imaginary: BigDecimal) -> Self {
        Self { real, imaginary }
    }

    pub fn is_zero(&self) -> bool {
        self.real.is_zero() && self.imaginary.is_zero()
    }

    pub fn is_one(&self) -> bool {
        self.real.is_one() && self.imaginary.is_zero()
    }
}

/// Runtime numeric values. Integers remain exact and use an unbounded
/// magnitude. Decimals store an unbounded integer coefficient and a decimal
/// scale, so literals and finite arithmetic never pass through a float.
#[derive(Debug, Clone)]
pub enum Number {
    Integer(BigInt),
    Rational(BigRational),
    Decimal(BigDecimal),
    Complex(ComplexNumber),
}

impl Number {
    pub fn is_exact(&self) -> bool {
        matches!(self, Number::Integer(_) | Number::Rational(_))
    }
    pub fn constant(constant: MathConstant, precision: u64) -> Result<Number, MathEvaluationError> {
        transcendental::evaluate_constant(constant, precision)
    }
    pub fn from_rational(value: BigRational) -> Number {
        if value.denom().is_one() {
            Number::Integer(value.to_integer())
        } else {
            Number::Rational(value)
        }
    }

    /// Returns the exact rational represented by a real value. Complex values
    /// return `None`, as do decimals whose scale cannot fit the conversion.
    pub fn as_rational(&self) -> Option<BigRational> {
        self.try_to_rational()
    }

    pub(crate) fn to_rational(&self) -> BigRational {
        self.try_to_rational()
            .expect("internal invariant: value must be an exactly representable real number")
    }

    pub(crate) fn try_to_rational(&self) -> Option<BigRational> {
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
            Number::Complex(_) => None,
        }
    }

    /// Converts a real value to a decimal using `context`. Complex values
    /// return `None` instead of discarding their imaginary component.
    pub fn as_decimal(&self, context: &Context) -> Option<BigDecimal> {
        match self {
            Number::Complex(_) => None,
            _ => Some(self.to_decimal(context)),
        }
    }

    pub(crate) fn to_decimal(&self, context: &Context) -> BigDecimal {
        match self {
            Number::Integer(value) => BigDecimal::from(value.clone()),
            Number::Rational(value) => {
                let numerator = BigDecimal::from(value.numer().clone());
                let denominator = BigDecimal::from(value.denom().clone());
                let reciprocal = context.invert(&denominator);
                context.multiply(&numerator, &reciprocal)
            }
            Number::Decimal(value) => value.clone(),
            Number::Complex(_) => {
                unreachable!("internal invariant: complex value reached real conversion")
            }
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_zero(),
            Number::Rational(value) => value.is_zero(),
            Number::Decimal(value) => value.is_zero(),
            Number::Complex(value) => value.is_zero(),
        }
    }

    pub fn is_one(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_one(),
            Number::Rational(value) => value.is_one(),
            Number::Decimal(value) => value.is_one(),
            Number::Complex(value) => value.is_one(),
        }
    }

    pub(crate) fn is_negative(&self) -> bool {
        match self {
            Number::Integer(value) => value.is_negative(),
            Number::Rational(value) => value.is_negative(),
            Number::Decimal(value) => value.is_negative(),
            Number::Complex(_) => false,
        }
    }
    pub fn compare(&self, other: &Number) -> Option<Ordering> {
        if matches!(self, Number::Complex(_)) || matches!(other, Number::Complex(_)) {
            return None;
        }
        match (self, other) {
            (Number::Integer(a), Number::Integer(b)) => Some(a.cmp(b)),
            (Number::Decimal(a), Number::Decimal(b)) => Some(a.cmp(b)),
            _ => match (self.try_to_rational(), other.try_to_rational()) {
                (Some(a), Some(b)) => Some(a.cmp(&b)),
                _ if !matches!(self, Number::Complex(_))
                    && !matches!(other, Number::Complex(_)) =>
                {
                    let context = Context::default();
                    Some(self.to_decimal(&context).cmp(&other.to_decimal(&context)))
                }
                _ => None,
            },
        }
    }

    pub(crate) fn abs(&self) -> Number {
        match self {
            Number::Integer(value) => Number::Integer(value.abs()),
            Number::Rational(value) => Number::Rational(value.abs()),
            Number::Decimal(value) => Number::Decimal(value.abs()),
            Number::Complex(value) => Number::Complex(value.clone()),
        }
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

    /// Evaluate an elementary function at the requested number of decimal
    /// significant digits. Real inputs remain real whenever the principal
    /// result is real; otherwise the result is promoted to a complex value.
    pub fn evaluate_unary(
        &self,
        function: UnaryMathFunction,
        precision: u64,
    ) -> Result<Number, MathEvaluationError> {
        transcendental::evaluate_unary(self, function, precision)
    }

    pub fn evaluate_binary(
        &self,
        other: &Number,
        function: BinaryMathFunction,
        precision: u64,
    ) -> Result<Number, MathEvaluationError> {
        transcendental::evaluate_binary(self, other, function, precision)
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Number::Complex(a), Number::Complex(b)) => {
                a.real == b.real && a.imaginary == b.imaginary
            }
            (Number::Complex(a), value) | (value, Number::Complex(a)) => {
                a.imaginary.is_zero()
                    && Number::Decimal(a.real.clone()).compare(value) == Some(Ordering::Equal)
            }
            _ => self.compare(other) == Some(Ordering::Equal),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::Integer(value) => value.fmt(formatter),
            Number::Rational(value) => write!(formatter, "{}/{}", value.numer(), value.denom()),
            Number::Decimal(value) => value.normalized().fmt(formatter),
            Number::Complex(value) => {
                let real = value.real.normalized();
                let imaginary = value.imaginary.normalized();
                if imaginary.is_zero() {
                    return real.fmt(formatter);
                }
                if real.is_zero() {
                    if imaginary.is_one() {
                        return formatter.write_str("i");
                    }
                    if imaginary == -1 {
                        return formatter.write_str("-i");
                    }
                    return write!(formatter, "{imaginary}i");
                }
                if imaginary.is_negative() {
                    write!(formatter, "{real} - {}i", imaginary.abs())
                } else {
                    write!(formatter, "{real} + {imaginary}i")
                }
            }
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
