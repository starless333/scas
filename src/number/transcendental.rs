use super::{ComplexNumber, Number};
use bigdecimal::BigDecimal;
use dashu_cmplx::CBig;
use dashu_float::{
    ops::{Abs, CubicRoot},
    round::mode::HalfAway,
    DBig,
};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Zero};
use std::fmt;
use std::str::FromStr;

type DecimalComplex = CBig<HalfAway, 10>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathConstant {
    Pi,
    E,
    Tau,
    Phi,
}

impl MathConstant {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::E => "e",
            Self::Tau => "tau",
            Self::Phi => "phi",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryMathFunction {
    Sqrt,
    Cbrt,
    Exp,
    Ln,
    Log10,
    Log2,
    Sin,
    Cos,
    Tan,
    Sec,
    Csc,
    Cot,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Degrees,
    Radians,
    Erf,
    Erfc,
    Gamma,
    Lgamma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryMathFunction {
    Log,
    Atan2,
    Hypot,
    Pow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathEvaluationError(pub String);

impl fmt::Display for MathEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for MathEvaluationError {}

fn checked_precision(precision: u64) -> Result<usize, MathEvaluationError> {
    usize::try_from(precision)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| MathEvaluationError("invalid decimal precision".to_string()))
}

fn decimal_float(text: &str, precision: usize) -> Result<DBig, MathEvaluationError> {
    DBig::from_str(text)
        .map(|value| value.with_precision(precision).value())
        .map_err(|error| MathEvaluationError(format!("cannot convert number: {error}")))
}

fn real_float(number: &Number, precision: usize) -> Result<DBig, MathEvaluationError> {
    match number {
        Number::Integer(value) => decimal_float(&value.to_string(), precision),
        Number::Rational(value) => {
            let numerator = decimal_float(&value.numer().to_string(), precision)?;
            let denominator = decimal_float(&value.denom().to_string(), precision)?;
            Ok(numerator / denominator)
        }
        Number::Decimal(value) => decimal_float(&value.to_string(), precision),
        Number::Complex(_) => Err(MathEvaluationError("expected a real number".to_string())),
    }
}

fn complex_float(number: &Number, precision: usize) -> Result<DecimalComplex, MathEvaluationError> {
    match number {
        Number::Complex(value) => Ok(DecimalComplex::from_parts(
            decimal_float(&value.real.to_string(), precision)?,
            decimal_float(&value.imaginary.to_string(), precision)?,
        )),
        value => Ok(DecimalComplex::from(real_float(value, precision)?)),
    }
}

fn decimal_number(value: DBig) -> Result<Number, MathEvaluationError> {
    if value.repr().is_infinite() {
        return Err(MathEvaluationError(
            "result is infinite and cannot be represented".to_string(),
        ));
    }
    BigDecimal::from_str(&value.to_string())
        .map(Number::Decimal)
        .map_err(|error| MathEvaluationError(format!("cannot convert result: {error}")))
}

pub(super) fn evaluate_constant(
    constant: MathConstant,
    precision: u64,
) -> Result<Number, MathEvaluationError> {
    let precision = checked_precision(precision)?;
    let value = match constant {
        MathConstant::Pi => DBig::pi(precision),
        MathConstant::E => DBig::ONE.with_precision(precision).value().exp(),
        MathConstant::Tau => DBig::from(2).with_precision(precision).value() * DBig::pi(precision),
        MathConstant::Phi => {
            let one = DBig::ONE.with_precision(precision).value();
            let five = DBig::from(5).with_precision(precision).value();
            (one + five.sqrt()) / DBig::from(2)
        }
    };
    decimal_number(value)
}

fn complex_number(value: DecimalComplex) -> Result<Number, MathEvaluationError> {
    if !value.is_finite() {
        return Err(MathEvaluationError(
            "result is infinite and cannot be represented".to_string(),
        ));
    }
    let (real, imaginary) = value.into_parts();
    let real = BigDecimal::from_str(&real.to_string())
        .map_err(|error| MathEvaluationError(format!("cannot convert real result: {error}")))?;
    let imaginary = BigDecimal::from_str(&imaginary.to_string()).map_err(|error| {
        MathEvaluationError(format!("cannot convert imaginary result: {error}"))
    })?;
    if imaginary == 0 {
        Ok(Number::Decimal(real))
    } else {
        Ok(Number::Complex(ComplexNumber::new(real, imaginary)))
    }
}

fn evaluate_complex(function: UnaryMathFunction, value: DecimalComplex) -> DecimalComplex {
    match function {
        UnaryMathFunction::Sqrt => value.sqrt(),
        UnaryMathFunction::Cbrt => {
            let precision = value.precision();
            let one_third = DBig::ONE.with_precision(precision).value()
                / DBig::from(3).with_precision(precision).value();
            value.powf(&DecimalComplex::from(one_third))
        }
        UnaryMathFunction::Exp => value.exp(),
        UnaryMathFunction::Ln => value.ln(),
        UnaryMathFunction::Log10 => {
            let precision = value.precision();
            let ten = DBig::from(10).with_precision(precision).value();
            value.ln() / DecimalComplex::from(ten.ln())
        }
        UnaryMathFunction::Log2 => {
            let precision = value.precision();
            let two = DBig::from(2).with_precision(precision).value();
            value.ln() / DecimalComplex::from(two.ln())
        }
        UnaryMathFunction::Sin => value.sin(),
        UnaryMathFunction::Cos => value.cos(),
        UnaryMathFunction::Tan => value.tan(),
        UnaryMathFunction::Sec => DecimalComplex::ONE / value.cos(),
        UnaryMathFunction::Csc => DecimalComplex::ONE / value.sin(),
        UnaryMathFunction::Cot => DecimalComplex::ONE / value.tan(),
        UnaryMathFunction::Asin => value.asin(),
        UnaryMathFunction::Acos => value.acos(),
        UnaryMathFunction::Atan => value.atan(),
        UnaryMathFunction::Sinh => value.sinh(),
        UnaryMathFunction::Cosh => value.cosh(),
        UnaryMathFunction::Tanh => value.tanh(),
        UnaryMathFunction::Asinh => value.asinh(),
        UnaryMathFunction::Acosh => value.acosh(),
        UnaryMathFunction::Atanh => value.atanh(),
        UnaryMathFunction::Degrees => {
            let precision = value.precision();
            value * DecimalComplex::from(DBig::from(180).with_precision(precision).value())
                / DecimalComplex::from(DBig::pi(precision))
        }
        UnaryMathFunction::Radians => {
            let precision = value.precision();
            value * DecimalComplex::from(DBig::pi(precision))
                / DecimalComplex::from(DBig::from(180).with_precision(precision).value())
        }
        UnaryMathFunction::Erf
        | UnaryMathFunction::Erfc
        | UnaryMathFunction::Gamma
        | UnaryMathFunction::Lgamma => {
            unreachable!("real-only special function reached complex evaluator")
        }
    }
}

pub(super) fn evaluate_unary(
    number: &Number,
    function: UnaryMathFunction,
    precision: u64,
) -> Result<Number, MathEvaluationError> {
    let precision = checked_precision(precision)?;

    if number.is_zero()
        && matches!(
            function,
            UnaryMathFunction::Ln
                | UnaryMathFunction::Log10
                | UnaryMathFunction::Log2
                | UnaryMathFunction::Csc
                | UnaryMathFunction::Cot
        )
    {
        return Err(MathEvaluationError(format!(
            "{} has a pole at zero",
            match function {
                UnaryMathFunction::Ln => "ln",
                UnaryMathFunction::Log10 => "log10",
                UnaryMathFunction::Log2 => "log2",
                UnaryMathFunction::Csc => "csc",
                UnaryMathFunction::Cot => "cot",
                _ => unreachable!(),
            }
        )));
    }
    if function == UnaryMathFunction::Atanh
        && (number == &Number::Integer(BigInt::one()) || number == &Number::Integer(-BigInt::one()))
    {
        return Err(MathEvaluationError(
            "atanh has poles at -1 and 1".to_string(),
        ));
    }

    if matches!(
        function,
        UnaryMathFunction::Erf
            | UnaryMathFunction::Erfc
            | UnaryMathFunction::Gamma
            | UnaryMathFunction::Lgamma
    ) {
        if matches!(number, Number::Complex(_)) {
            return Err(MathEvaluationError(
                "complex special-function evaluation is not implemented".to_string(),
            ));
        }
        let value = real_float(number, precision.saturating_add(16))?;
        let result = match function {
            UnaryMathFunction::Erf => erf_real(value, precision, false)?,
            UnaryMathFunction::Erfc => erf_real(value, precision, true)?,
            UnaryMathFunction::Gamma => gamma_real(value, precision, false)?,
            UnaryMathFunction::Lgamma => gamma_real(value, precision, true)?,
            _ => unreachable!(),
        };
        return decimal_number(result.with_precision(precision).value());
    }

    if let Number::Complex(_) = number {
        return complex_number(evaluate_complex(
            function,
            complex_float(number, precision)?,
        ));
    }

    let value = real_float(number, precision)?;
    let zero = DBig::ZERO.with_precision(precision).value();
    let one = DBig::ONE.with_precision(precision).value();
    let real_result = match function {
        UnaryMathFunction::Sqrt if value >= zero => Some(value.sqrt()),
        UnaryMathFunction::Cbrt => Some(value.cbrt()),
        UnaryMathFunction::Exp => Some(value.exp()),
        UnaryMathFunction::Ln if value > zero => Some(value.ln()),
        UnaryMathFunction::Log10 if value > zero => Some(value.log10()),
        UnaryMathFunction::Log2 if value > zero => Some(value.log2()),
        UnaryMathFunction::Sin => Some(value.sin()),
        UnaryMathFunction::Cos => Some(value.cos()),
        UnaryMathFunction::Tan => Some(value.tan()),
        UnaryMathFunction::Sec => Some(one.clone() / value.cos()),
        UnaryMathFunction::Csc => Some(one.clone() / value.sin()),
        UnaryMathFunction::Cot => Some(one / value.tan()),
        UnaryMathFunction::Asin if value >= -DBig::ONE && value <= DBig::ONE => Some(value.asin()),
        UnaryMathFunction::Acos if value >= -DBig::ONE && value <= DBig::ONE => Some(value.acos()),
        UnaryMathFunction::Atan => Some(value.atan()),
        UnaryMathFunction::Sinh => Some(value.sinh()),
        UnaryMathFunction::Cosh => Some(value.cosh()),
        UnaryMathFunction::Tanh => Some(value.tanh()),
        UnaryMathFunction::Asinh => Some(value.asinh()),
        UnaryMathFunction::Acosh if value >= DBig::ONE => Some(value.acosh()),
        UnaryMathFunction::Atanh if value > -DBig::ONE && value < DBig::ONE => Some(value.atanh()),
        UnaryMathFunction::Degrees => {
            Some(value * DBig::from(180).with_precision(precision).value() / DBig::pi(precision))
        }
        UnaryMathFunction::Radians => {
            Some(value * DBig::pi(precision) / DBig::from(180).with_precision(precision).value())
        }
        UnaryMathFunction::Erf
        | UnaryMathFunction::Erfc
        | UnaryMathFunction::Gamma
        | UnaryMathFunction::Lgamma => unreachable!(),
        _ => None,
    };
    if let Some(result) = real_result {
        decimal_number(result)
    } else {
        complex_number(evaluate_complex(
            function,
            complex_float(number, precision)?,
        ))
    }
}

fn bernoulli_numbers(maximum: usize) -> Vec<BigRational> {
    let mut work = vec![BigRational::zero(); maximum + 1];
    let mut numbers = vec![BigRational::zero(); maximum + 1];
    for m in 0..=maximum {
        work[m] = BigRational::new(BigInt::one(), BigInt::from(m + 1));
        for j in (1..=m).rev() {
            work[j - 1] = BigRational::from_integer(BigInt::from(j)) * (&work[j - 1] - &work[j]);
        }
        numbers[m] = work[0].clone();
    }
    numbers
}

fn rational_float(value: &BigRational, precision: usize) -> Result<DBig, MathEvaluationError> {
    let numerator = decimal_float(&value.numer().to_string(), precision)?;
    let denominator = decimal_float(&value.denom().to_string(), precision)?;
    Ok(numerator / denominator)
}

fn log_gamma_positive(mut value: DBig, precision: usize) -> Result<DBig, MathEvaluationError> {
    let work_precision = precision.saturating_add(16);
    value = value.with_precision(work_precision).value();
    if value <= DBig::ZERO {
        return Err(MathEvaluationError(
            "log-gamma positive branch requires a positive argument".to_string(),
        ));
    }
    let threshold_integer = precision.saturating_mul(2).saturating_add(32);
    let threshold = DBig::from(threshold_integer)
        .with_precision(work_precision)
        .value();
    let one = DBig::ONE.with_precision(work_precision).value();
    let half = decimal_float("0.5", work_precision)?;
    let mut correction = DBig::ZERO.with_precision(work_precision).value();
    while value < threshold {
        correction += value.ln();
        value += &one;
    }

    let pi = DBig::pi(work_precision);
    let mut result = (&value - &half) * value.ln() - &value
        + &half * (DBig::from(2).with_precision(work_precision).value() * pi).ln();
    let terms = precision.saturating_div(2).saturating_add(12).max(12);
    let bernoulli = bernoulli_numbers(terms.saturating_mul(2));
    let value_squared = &value * &value;
    let mut odd_power = value.clone();
    for k in 1..=terms {
        let index = k * 2;
        let coefficient = rational_float(&bernoulli[index], work_precision)?;
        let divisor = DBig::from(index * (index - 1))
            .with_precision(work_precision)
            .value()
            * &odd_power;
        result += coefficient / divisor;
        odd_power *= &value_squared;
    }
    Ok(result - correction)
}

fn gamma_real(value: DBig, precision: usize, logarithm: bool) -> Result<DBig, MathEvaluationError> {
    let work_precision = precision.saturating_add(16);
    let value = value.with_precision(work_precision).value();
    if value > DBig::ZERO {
        let log_gamma = log_gamma_positive(value, work_precision)?;
        return Ok(if logarithm {
            log_gamma
        } else {
            log_gamma.exp()
        });
    }
    if value.fract() == DBig::ZERO {
        return Err(MathEvaluationError(
            "gamma has a pole at non-positive integers".to_string(),
        ));
    }
    let pi = DBig::pi(work_precision);
    let sine = value.sin_pi();
    let reflected = DBig::ONE.with_precision(work_precision).value() - &value;
    let reflected_log = log_gamma_positive(reflected, work_precision)?;
    let log_absolute = pi.ln() - sine.clone().abs().ln() - reflected_log;
    if logarithm {
        Ok(log_absolute)
    } else {
        let magnitude = log_absolute.exp();
        Ok(if sine < DBig::ZERO {
            -magnitude
        } else {
            magnitude
        })
    }
}

fn erf_real(
    value: DBig,
    precision: usize,
    complementary: bool,
) -> Result<DBig, MathEvaluationError> {
    let work_precision = precision.saturating_add(16);
    let value = value.with_precision(work_precision).value();
    if value == DBig::ZERO {
        return Ok(if complementary {
            DBig::ONE.with_precision(precision).value()
        } else {
            DBig::ZERO.with_precision(precision).value()
        });
    }
    let negative = value < DBig::ZERO;
    let absolute = value.abs();
    let x = &absolute * &absolute;
    let half = decimal_float("0.5", work_precision)?;
    let one = DBig::ONE.with_precision(work_precision).value();
    let log_gamma_half = half.clone() * DBig::pi(work_precision).ln();
    let prefactor = (-&x + &half * x.ln() - log_gamma_half).exp();
    let iterations = precision.saturating_mul(3).saturating_add(80).max(100);
    let series_threshold = DBig::from(16).with_precision(work_precision).value();
    let positive_erf = if x <= series_threshold {
        let mut ap = half.clone();
        let mut delta = one.clone() / &half;
        let mut sum = delta.clone();
        for _ in 1..=iterations {
            ap += &one;
            delta *= &x / &ap;
            sum += &delta;
        }
        prefactor * sum
    } else {
        let tiny = decimal_float(
            &format!("1e-{}", work_precision.saturating_add(20)),
            work_precision,
        )?;
        let mut b = &x + &one - &half;
        let mut c = one.clone() / &tiny;
        let mut d = one.clone() / &b;
        let mut h = d.clone();
        for i in 1..=iterations {
            let i_float = DBig::from(i).with_precision(work_precision).value();
            let an = -(&i_float * (&i_float - &half));
            b += DBig::from(2).with_precision(work_precision).value();
            d = &an * &d + &b;
            if d == DBig::ZERO {
                d.clone_from(&tiny);
            }
            c = &b + &an / &c;
            if c == DBig::ZERO {
                c.clone_from(&tiny);
            }
            d = one.clone() / d;
            h *= &d * &c;
        }
        one.clone() - prefactor * h
    };
    let erf = if negative {
        -positive_erf
    } else {
        positive_erf
    };
    Ok(if complementary { one - erf } else { erf })
}

pub(super) fn evaluate_binary(
    left: &Number,
    right: &Number,
    function: BinaryMathFunction,
    precision: u64,
) -> Result<Number, MathEvaluationError> {
    let precision = checked_precision(precision)?;
    match function {
        BinaryMathFunction::Log => {
            if left.is_zero() {
                return Err(MathEvaluationError("log has a pole at zero".to_string()));
            }
            if right.is_zero() || right.is_one() {
                return Err(MathEvaluationError(
                    "log base must be nonzero and different from 1".to_string(),
                ));
            }
            if matches!(left, Number::Complex(_)) || matches!(right, Number::Complex(_)) {
                let value = complex_float(left, precision)?;
                let base = complex_float(right, precision)?;
                return complex_number(value.ln() / base.ln());
            }
            let value = real_float(left, precision)?;
            let base = real_float(right, precision)?;
            if value > DBig::ZERO && base > DBig::ZERO && base != DBig::ONE {
                decimal_number(value.ln() / base.ln())
            } else {
                let value = complex_float(left, precision)?;
                let base = complex_float(right, precision)?;
                complex_number(value.ln() / base.ln())
            }
        }
        BinaryMathFunction::Atan2 => {
            if left.is_zero() && right.is_zero() {
                return Err(MathEvaluationError(
                    "atan2 is undefined when both arguments are zero".to_string(),
                ));
            }
            let y = real_float(left, precision)?;
            let x = real_float(right, precision)?;
            decimal_number(y.atan2(&x))
        }
        BinaryMathFunction::Hypot => {
            let left = real_float(left, precision)?;
            let right = real_float(right, precision)?;
            decimal_number(left.hypot(&right))
        }
        BinaryMathFunction::Pow => {
            if left.is_zero() {
                if right.is_zero() {
                    return decimal_number(DBig::ONE.with_precision(precision).value());
                }
                let order = match right {
                    Number::Complex(value) if value.imaginary.is_zero() => {
                        Number::Decimal(value.real.clone())
                            .compare(&Number::Integer(BigInt::zero()))
                    }
                    Number::Complex(_) => None,
                    value => value.compare(&Number::Integer(BigInt::zero())),
                };
                if order.is_some_and(std::cmp::Ordering::is_gt) {
                    return decimal_number(DBig::ZERO.with_precision(precision).value());
                }
                return Err(MathEvaluationError(
                    "zero cannot be raised to this power".to_string(),
                ));
            }
            if !matches!(left, Number::Complex(_)) && !matches!(right, Number::Complex(_)) {
                let base = real_float(left, precision)?;
                let exponent = real_float(right, precision)?;
                if base < DBig::ZERO {
                    let magnitude = (-base).powf(&exponent);
                    let real = &magnitude * exponent.cos_pi();
                    let imaginary = magnitude * exponent.sin_pi();
                    return complex_number(DecimalComplex::from_parts(real, imaginary));
                }
                return decimal_number(base.powf(&exponent));
            }
            {
                let base = complex_float(left, precision)?;
                let exponent = complex_float(right, precision)?;
                complex_number((base.ln() * exponent).exp())
            }
        }
    }
}
