use crate::ast::binary_op::BinaryOp;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::value::{Builtin, SymbolicExpr};
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::number::{ComplexNumber, Number};
use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;

#[derive(Clone, Copy, Debug)]
struct Complex64 {
    real: f64,
    imaginary: f64,
}

impl Complex64 {
    fn add(self, other: Self) -> Self {
        Self {
            real: self.real + other.real,
            imaginary: self.imaginary + other.imaginary,
        }
    }

    fn sub(self, other: Self) -> Self {
        Self {
            real: self.real - other.real,
            imaginary: self.imaginary - other.imaginary,
        }
    }

    fn mul(self, other: Self) -> Self {
        Self {
            real: self.real * other.real - self.imaginary * other.imaginary,
            imaginary: self.real * other.imaginary + self.imaginary * other.real,
        }
    }

    fn div(self, other: Self) -> Self {
        let denominator = other.real * other.real + other.imaginary * other.imaginary;
        Self {
            real: (self.real * other.real + self.imaginary * other.imaginary) / denominator,
            imaginary: (self.imaginary * other.real - self.real * other.imaginary) / denominator,
        }
    }

    fn magnitude(self) -> f64 {
        self.real.hypot(self.imaginary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Variable {
    name: String,
    symbol: Symbol,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Monomial(Vec<(Variable, u32)>);

#[derive(Debug, Clone)]
pub struct Polynomial {
    terms: BTreeMap<Monomial, BigRational>,
}

impl Polynomial {
    const MAX_RESULTANT_SIZE: usize = 128;

    pub fn solve_expression(
        expression: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Option<Vec<Rc<SymbolicExpr>>> {
        let polynomial = Self::from_expression(expression, symbols)?;
        let (found_variable, coefficients) = polynomial.univariate_coefficients()?;
        if found_variable.symbol != variable || coefficients.len() <= 1 {
            return None;
        }
        Some(Self::solve_coefficients(coefficients))
    }

    pub fn degree_expression(
        expression: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<usize, String> {
        let (_, coefficients) = Self::coefficients_in(expression, variable, symbols)?;
        coefficients
            .len()
            .checked_sub(1)
            .ok_or_else(|| "degree is undefined for the zero polynomial".to_owned())
    }

    pub fn coefficient_expressions(
        expression: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<Vec<Rc<SymbolicExpr>>, String> {
        let (_, coefficients) = Self::coefficients_in(expression, variable, symbols)?;
        if coefficients.is_empty() {
            return Ok(vec![Self::integer_expression(0)]);
        }
        Ok(coefficients
            .into_iter()
            .map(|coefficient| Rc::new(SymbolicExpr::Number(Number::from_rational(coefficient))))
            .collect())
    }

    pub fn divrem_expressions(
        dividend: &SymbolicExpr,
        divisor: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<(Rc<SymbolicExpr>, Rc<SymbolicExpr>), String> {
        let (variable, dividend) = Self::coefficients_in(dividend, variable, symbols)?;
        let (_, divisor) = Self::coefficients_in(divisor, variable.symbol, symbols)?;
        if divisor.is_empty() {
            return Err("polynomial_divrem divisor must be nonzero".to_owned());
        }
        let (quotient, remainder) = Self::divide_coefficients(dividend, &divisor)
            .ok_or_else(|| "polynomial division failed".to_owned())?;
        Ok((
            Self::from_univariate(variable.clone(), quotient).to_expression(),
            Self::from_univariate(variable, remainder).to_expression(),
        ))
    }

    pub fn gcd_expression(
        left: &SymbolicExpr,
        right: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<Rc<SymbolicExpr>, String> {
        let (variable, left) = Self::coefficients_in(left, variable, symbols)?;
        let (_, right) = Self::coefficients_in(right, variable.symbol, symbols)?;
        if left.is_empty() && right.is_empty() {
            return Err("polynomial_gcd is undefined for two zero polynomials".to_owned());
        }
        let gcd = Self::polynomial_gcd(left, right);
        Ok(Self::from_univariate(variable, gcd).to_expression())
    }

    pub fn resultant_expression(
        left: &SymbolicExpr,
        right: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<BigRational, String> {
        let (_, left) = Self::coefficients_in(left, variable, symbols)?;
        let (_, right) = Self::coefficients_in(right, variable, symbols)?;
        Self::resultant_coefficients(&left, &right)
    }

    pub fn discriminant_expression(
        expression: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<BigRational, String> {
        let (_, coefficients) = Self::coefficients_in(expression, variable, symbols)?;
        let Some(degree) = coefficients.len().checked_sub(1) else {
            return Err("discriminant is undefined for the zero polynomial".to_owned());
        };
        if degree <= 1 {
            return Ok(BigRational::one());
        }
        let derivative = coefficients
            .iter()
            .enumerate()
            .skip(1)
            .map(|(exponent, coefficient)| coefficient * BigInt::from(exponent))
            .collect::<Vec<_>>();
        let resultant = Self::resultant_coefficients(&coefficients, &derivative)?;
        let leading = coefficients
            .last()
            .expect("a nonzero polynomial has a leading coefficient");
        let sign = if degree * (degree - 1) / 2 % 2 == 0 {
            BigRational::one()
        } else {
            -BigRational::one()
        };
        Ok(sign * resultant / leading)
    }

    fn coefficients_in(
        expression: &SymbolicExpr,
        variable: Symbol,
        symbols: &SymbolTable,
    ) -> Result<(Variable, Vec<BigRational>), String> {
        let variable = Variable {
            name: symbols.resolve(variable).to_owned(),
            symbol: variable,
        };
        let polynomial = Self::from_expression(expression, symbols)
            .ok_or_else(|| "expected a polynomial with rational coefficients".to_owned())?;
        let mut coefficients = polynomial
            .coefficients_for(&variable)
            .ok_or_else(|| "expression is not univariate in the requested variable".to_owned())?;
        Self::trim_coefficients(&mut coefficients);
        Ok((variable, coefficients))
    }

    fn resultant_coefficients(
        left: &[BigRational],
        right: &[BigRational],
    ) -> Result<BigRational, String> {
        if left.is_empty() || right.is_empty() {
            return Ok(BigRational::zero());
        }
        let left_degree = left.len() - 1;
        let right_degree = right.len() - 1;
        let size = left_degree + right_degree;
        if size > Self::MAX_RESULTANT_SIZE {
            return Err(format!(
                "resultant matrix dimension {size} exceeds the limit of {}",
                Self::MAX_RESULTANT_SIZE
            ));
        }
        if size == 0 {
            return Ok(BigRational::one());
        }
        let mut matrix = vec![vec![BigRational::zero(); size]; size];
        for (row, matrix_row) in matrix.iter_mut().enumerate().take(right_degree) {
            for exponent in 0..=left_degree {
                matrix_row[row + exponent] = left[left_degree - exponent].clone();
            }
        }
        for (row, matrix_row) in matrix.iter_mut().skip(right_degree).enumerate() {
            for exponent in 0..=right_degree {
                matrix_row[row + exponent] = right[right_degree - exponent].clone();
            }
        }
        Ok(Self::rational_determinant(matrix))
    }

    fn rational_determinant(mut matrix: Vec<Vec<BigRational>>) -> BigRational {
        let mut determinant = BigRational::one();
        for column in 0..matrix.len() {
            let Some(pivot) = (column..matrix.len()).find(|&row| !matrix[row][column].is_zero())
            else {
                return BigRational::zero();
            };
            if pivot != column {
                matrix.swap(pivot, column);
                determinant = -determinant;
            }
            let pivot_value = matrix[column][column].clone();
            determinant *= &pivot_value;
            for row in column + 1..matrix.len() {
                if matrix[row][column].is_zero() {
                    continue;
                }
                let factor = &matrix[row][column] / &pivot_value;
                for entry in column + 1..matrix.len() {
                    let amount = &factor * &matrix[column][entry];
                    matrix[row][entry] -= amount;
                }
            }
        }
        determinant
    }

    fn solve_coefficients(mut coefficients: Vec<BigRational>) -> Vec<Rc<SymbolicExpr>> {
        let mut roots = Vec::new();

        // Pull out every rational root exactly, including roots of a polynomial
        // whose remaining factor has irrational or complex roots.
        for candidate in Self::root_candidates(&coefficients) {
            let mut found = false;
            while coefficients.len() > 1
                && Self::evaluate_coefficients(&coefficients, &candidate).is_zero()
            {
                let Some(quotient) = Self::synthetic_divide(&coefficients, &candidate) else {
                    break;
                };
                coefficients = quotient;
                found = true;
            }
            if found {
                roots.push(Rc::new(SymbolicExpr::Number(Number::from_rational(
                    candidate,
                ))));
            }
        }

        match coefficients.len().saturating_sub(1) {
            0 => {}
            1 => {
                let root = -&coefficients[0] / &coefficients[1];
                roots.push(Rc::new(SymbolicExpr::Number(Number::from_rational(root))));
            }
            2 => roots.extend(Self::quadratic_roots(&coefficients)),
            _ => roots.extend(Self::numeric_roots(&coefficients)),
        }
        roots
    }

    fn quadratic_roots(coefficients: &[BigRational]) -> Vec<Rc<SymbolicExpr>> {
        let (constant, linear, quadratic) = (&coefficients[0], &coefficients[1], &coefficients[2]);
        let discriminant =
            linear * linear - BigRational::from_integer(BigInt::from(4)) * quadratic * constant;
        let denominator = BigRational::from_integer(BigInt::from(2)) * quadratic;
        let discriminant_number = Number::from_rational(discriminant.clone());
        if let Some(root) = discriminant_number.exact_root(2) {
            let root = root
                .try_to_rational()
                .expect("an exact rational root stays rational");
            let mut result = vec![
                Rc::new(SymbolicExpr::Number(Number::from_rational(
                    (-linear - &root) / &denominator,
                ))),
                Rc::new(SymbolicExpr::Number(Number::from_rational(
                    (-linear + root) / denominator,
                ))),
            ];
            if result[0].equals(&result[1]) {
                result.pop();
            }
            return result;
        }

        if discriminant.is_negative() {
            let magnitude = Number::from_rational(discriminant.abs());
            if let Some(root) = magnitude
                .exact_root(2)
                .and_then(|value| value.try_to_rational())
            {
                let real = -linear / &denominator;
                let imaginary_coefficient = (root / &denominator).abs();
                let imaginary_unit = Rc::new(SymbolicExpr::Number(Number::Complex(
                    ComplexNumber::new(BigDecimal::from(0), BigDecimal::from(1)),
                )));
                let imaginary = if imaginary_coefficient.is_one() {
                    imaginary_unit
                } else {
                    Self::binary(
                        Rc::new(SymbolicExpr::Number(Number::from_rational(
                            imaginary_coefficient,
                        ))),
                        BinaryOp::Mul,
                        imaginary_unit,
                    )
                };
                let real = Rc::new(SymbolicExpr::Number(Number::from_rational(real)));
                let negative = if matches!(real.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
                {
                    Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Rc::clone(&imaginary),
                    })
                } else {
                    Self::binary(Rc::clone(&real), BinaryOp::Sub, Rc::clone(&imaginary))
                };
                let positive = if matches!(real.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
                {
                    imaginary
                } else {
                    Self::binary(real, BinaryOp::Add, imaginary)
                };
                return vec![negative, positive];
            }
        }

        let negative_discriminant = discriminant.is_negative();
        let positive_discriminant = Number::from_rational(discriminant.abs());
        let radical = positive_discriminant.exact_root(2).map_or_else(
            || {
                Rc::new(SymbolicExpr::Call {
                    function: Builtin::Sqrt,
                    args: vec![Rc::new(SymbolicExpr::Number(positive_discriminant))],
                })
            },
            |root| Rc::new(SymbolicExpr::Number(root)),
        );
        let radical = if negative_discriminant {
            Self::binary(
                radical,
                BinaryOp::Mul,
                Rc::new(SymbolicExpr::Number(Number::Complex(ComplexNumber::new(
                    BigDecimal::from(0),
                    BigDecimal::from(1),
                )))),
            )
        } else {
            radical
        };
        let negative_linear = Rc::new(SymbolicExpr::Number(Number::from_rational(-linear)));
        let denominator = Rc::new(SymbolicExpr::Number(Number::from_rational(denominator)));
        [BinaryOp::Sub, BinaryOp::Add]
            .into_iter()
            .map(|op| {
                Self::binary(
                    Self::binary(Rc::clone(&negative_linear), op, Rc::clone(&radical)),
                    BinaryOp::Div,
                    Rc::clone(&denominator),
                )
            })
            .collect()
    }

    /// Durand-Kerner finds every complex root at once. Exact/rational factors
    /// have already been removed, so this is only the fallback for irreducible
    /// degree-three-and-higher factors.
    fn numeric_roots(coefficients: &[BigRational]) -> Vec<Rc<SymbolicExpr>> {
        let Some(values) = coefficients
            .iter()
            .map(ToPrimitive::to_f64)
            .collect::<Option<Vec<_>>>()
        else {
            return Vec::new();
        };
        let degree = values.len() - 1;
        let leading = values[degree];
        if degree > 64 || !leading.is_finite() || leading == 0.0 {
            return Vec::new();
        }
        let radius = 1.0
            + values[..degree]
                .iter()
                .map(|value| (value / leading).abs())
                .fold(0.0_f64, f64::max);
        let mut roots = (0..degree)
            .map(|index| {
                let angle = std::f64::consts::TAU * (index as f64 + 0.271_828) / degree as f64;
                Complex64 {
                    real: radius * angle.cos(),
                    imaginary: radius * angle.sin(),
                }
            })
            .collect::<Vec<_>>();
        for _ in 0..2_000 {
            let previous = roots.clone();
            let mut largest_delta = 0.0_f64;
            for index in 0..degree {
                let value = values.iter().rev().fold(
                    Complex64 {
                        real: 0.0,
                        imaginary: 0.0,
                    },
                    |accumulator, coefficient| {
                        accumulator.mul(previous[index]).add(Complex64 {
                            real: *coefficient,
                            imaginary: 0.0,
                        })
                    },
                );
                let denominator = previous
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .fold(
                        Complex64 {
                            real: 1.0,
                            imaginary: 0.0,
                        },
                        |product, (_, root)| product.mul(previous[index].sub(*root)),
                    );
                if denominator.magnitude() <= f64::EPSILON {
                    continue;
                }
                let delta = value.div(denominator);
                roots[index] = previous[index].sub(delta);
                largest_delta = largest_delta.max(delta.magnitude());
            }
            if largest_delta < 1e-13 {
                break;
            }
        }
        for root in &mut roots {
            if root.real.abs() < 1e-12 {
                root.real = 0.0;
            }
            if root.imaginary.abs() < 1e-12 {
                root.imaginary = 0.0;
            }
        }
        roots.sort_by(|left, right| {
            if (left.real - right.real).abs() < 1e-10 {
                left.imaginary.total_cmp(&right.imaginary)
            } else {
                left.real.total_cmp(&right.real)
            }
        });
        roots.dedup_by(|left, right| left.sub(*right).magnitude() < 1e-10);
        roots
            .into_iter()
            .filter_map(|root| {
                let real = BigDecimal::from_str(&format!("{:.14}", root.real)).ok()?;
                let imaginary = BigDecimal::from_str(&format!("{:.14}", root.imaginary)).ok()?;
                let number = if imaginary.is_zero() {
                    Number::Decimal(real)
                } else {
                    Number::Complex(ComplexNumber::new(real, imaginary))
                };
                Some(Rc::new(SymbolicExpr::Number(number)))
            })
            .collect()
    }

    pub fn from_expression(expression: &SymbolicExpr, symbols: &SymbolTable) -> Option<Self> {
        match expression {
            SymbolicExpr::Number(number) => Some(Self::constant(number.try_to_rational()?)),
            SymbolicExpr::Variable(symbol) => Some(Self::variable(Variable {
                name: symbols.resolve(*symbol).to_owned(),
                symbol: *symbol,
            })),
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(Self::from_expression(expr, symbols)?.scaled(-BigRational::one())),
            SymbolicExpr::Binary { left, op, right } => {
                let left = Self::from_expression(left, symbols)?;
                match op {
                    BinaryOp::Add => Some(left.add(&Self::from_expression(right, symbols)?)),
                    BinaryOp::Sub => Some(left.sub(&Self::from_expression(right, symbols)?)),
                    BinaryOp::Mul => Some(left.mul(&Self::from_expression(right, symbols)?)),
                    BinaryOp::Div => {
                        let divisor = Self::from_expression(right, symbols)?.constant_value()?;
                        (!divisor.is_zero()).then(|| left.scaled(divisor.recip()))
                    }
                    BinaryOp::Pow => {
                        let SymbolicExpr::Number(Number::Integer(exponent)) = right.as_ref() else {
                            return None;
                        };
                        let exponent = exponent.to_u32()?;
                        Some(left.pow(exponent))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn partial_fraction_expression(
        expression: &SymbolicExpr,
        symbols: &SymbolTable,
    ) -> Option<Rc<SymbolicExpr>> {
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Div,
            right,
        } = expression
        else {
            return None;
        };
        let numerator = Self::from_expression(left, symbols)?;
        let denominator = Self::from_expression(right, symbols)?;
        let (variable, denominator_coefficients) = denominator.univariate_coefficients()?;
        let mut numerator_coefficients = numerator.coefficients_for(&variable)?;
        // Improper fractions divide first: the quotient becomes a
        // polynomial summand and only the remainder is decomposed.
        let mut quotient = None;
        if numerator_coefficients.len() >= denominator_coefficients.len() {
            let (divided, remainder) =
                Self::divide_coefficients(numerator_coefficients, &denominator_coefficients)?;
            numerator_coefficients = remainder;
            quotient = Some(divided);
        }

        let mut roots = Vec::new();
        for root in Self::root_candidates(&denominator_coefficients) {
            if Self::evaluate_coefficients(&denominator_coefficients, &root).is_zero() {
                roots.push(root);
            }
        }
        roots.sort();
        roots.dedup();
        if roots.len() + 1 != denominator_coefficients.len() {
            return None;
        }

        let derivative: Vec<_> = denominator_coefficients
            .iter()
            .enumerate()
            .skip(1)
            .map(|(exponent, coefficient)| coefficient * BigInt::from(exponent))
            .collect();
        let mut result = None;
        for root in roots {
            let derivative_value = Self::evaluate_coefficients(&derivative, &root);
            if derivative_value.is_zero() {
                return None;
            }
            let coefficient =
                Self::evaluate_coefficients(&numerator_coefficients, &root) / derivative_value;
            if coefficient.is_zero() {
                continue;
            }
            let variable_expression = Rc::new(SymbolicExpr::Variable(variable.symbol));
            let root_expression = Rc::new(SymbolicExpr::Number(Number::from_rational(
                root.clone().abs(),
            )));
            let linear = if root.is_zero() {
                variable_expression
            } else {
                Self::binary(
                    variable_expression,
                    if root.is_negative() {
                        BinaryOp::Add
                    } else {
                        BinaryOp::Sub
                    },
                    root_expression,
                )
            };
            let numerator = Rc::new(SymbolicExpr::Number(Number::from_rational(
                coefficient.abs(),
            )));
            let fraction = Self::binary(numerator, BinaryOp::Div, linear);
            result = Some(match result {
                None if coefficient.is_negative() => Rc::new(SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr: fraction,
                }),
                None => fraction,
                Some(previous) => Self::binary(
                    previous,
                    if coefficient.is_negative() {
                        BinaryOp::Sub
                    } else {
                        BinaryOp::Add
                    },
                    fraction,
                ),
            });
        }
        match (quotient, result) {
            (Some(quotient), rest) => {
                let quotient = Self::from_univariate(variable, quotient).to_expression();
                Some(match rest {
                    None => quotient,
                    Some(rest) => Self::binary(quotient, BinaryOp::Add, rest),
                })
            }
            (None, result) => result,
        }
    }

    /// Combines a sum of fractions into a single fraction (`together`),
    /// the inverse of `apart`. Shared univariate content is cancelled.
    pub fn together_expression(
        expression: &SymbolicExpr,
        symbols: &SymbolTable,
    ) -> Option<Rc<SymbolicExpr>> {
        let mut terms: Vec<(Polynomial, Polynomial)> = Vec::new();
        Self::collect_fraction_terms(expression, symbols, true, &mut terms)?;
        let one = || Self::constant(BigRational::one());
        let mut denominator = one();
        for (_, term_denominator) in &terms {
            denominator = denominator.mul(term_denominator);
        }
        let mut numerator: Option<Polynomial> = None;
        for (index, (term_numerator, _)) in terms.iter().enumerate() {
            let mut cofactor = one();
            for (other_index, (_, other_denominator)) in terms.iter().enumerate() {
                if other_index != index {
                    cofactor = cofactor.mul(other_denominator);
                }
            }
            let contribution = term_numerator.mul(&cofactor);
            numerator = Some(match numerator {
                None => contribution,
                Some(previous) => previous.add(&contribution),
            });
        }
        let numerator = numerator?;
        let mut numerator = numerator.expanded_expression();
        let mut denominator = denominator.expanded_expression();
        if let Some((reduced_numerator, reduced_denominator)) =
            Self::reduce_fraction(&numerator, &denominator, symbols)
        {
            numerator = reduced_numerator;
            denominator = reduced_denominator;
        }
        Some(Self::binary(numerator, BinaryOp::Div, denominator))
    }

    /// Collects `left ± right` recursively into (numerator, denominator)
    /// polynomial pairs, negating the numerator of subtracted terms.
    fn collect_fraction_terms(
        expression: &SymbolicExpr,
        symbols: &SymbolTable,
        positive: bool,
        out: &mut Vec<(Polynomial, Polynomial)>,
    ) -> Option<()> {
        let sign = |polynomial: Polynomial| {
            if positive {
                polynomial
            } else {
                polynomial.scaled(-BigRational::one())
            }
        };
        match expression {
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Add,
                right,
            } => {
                Self::collect_fraction_terms(left, symbols, positive, out)?;
                Self::collect_fraction_terms(right, symbols, positive, out)
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Sub,
                right,
            } => {
                Self::collect_fraction_terms(left, symbols, positive, out)?;
                Self::collect_fraction_terms(right, symbols, !positive, out)
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Div,
                right,
            } => {
                let numerator = sign(Self::from_expression(left, symbols)?);
                let denominator = Self::from_expression(right, symbols)?;
                out.push((numerator, denominator));
                Some(())
            }
            _ => {
                let numerator = sign(Self::from_expression(expression, symbols)?);
                out.push((numerator, Self::constant(BigRational::one())));
                Some(())
            }
        }
    }

    pub fn reduce_fraction(
        numerator: &SymbolicExpr,
        denominator: &SymbolicExpr,
        symbols: &SymbolTable,
    ) -> Option<(Rc<SymbolicExpr>, Rc<SymbolicExpr>)> {
        let numerator = Self::from_expression(numerator, symbols)?;
        let denominator = Self::from_expression(denominator, symbols)?;
        let variable = numerator
            .univariate_coefficients()
            .map(|(variable, _)| variable)
            .or_else(|| {
                denominator
                    .univariate_coefficients()
                    .map(|(variable, _)| variable)
            })?;
        let numerator_coefficients = numerator.coefficients_for(&variable)?;
        let denominator_coefficients = denominator.coefficients_for(&variable)?;
        if denominator_coefficients.iter().all(Zero::is_zero) {
            return None;
        }
        let gcd = Self::polynomial_gcd(
            numerator_coefficients.clone(),
            denominator_coefficients.clone(),
        );
        if gcd.len() <= 1 {
            return None;
        }
        let (numerator, numerator_remainder) =
            Self::divide_coefficients(numerator_coefficients, &gcd)?;
        let (denominator, denominator_remainder) =
            Self::divide_coefficients(denominator_coefficients, &gcd)?;
        if numerator_remainder.iter().any(|value| !value.is_zero())
            || denominator_remainder.iter().any(|value| !value.is_zero())
        {
            return None;
        }
        Some((
            Self::from_univariate(variable.clone(), numerator).to_expression(),
            Self::from_univariate(variable, denominator).to_expression(),
        ))
    }

    fn constant(value: BigRational) -> Self {
        let mut terms = BTreeMap::new();
        if !value.is_zero() {
            terms.insert(Monomial::default(), value);
        }
        Self { terms }
    }

    fn variable(variable: Variable) -> Self {
        let mut terms = BTreeMap::new();
        terms.insert(Monomial(vec![(variable, 1)]), BigRational::one());
        Self { terms }
    }

    fn constant_value(&self) -> Option<BigRational> {
        if self.terms.is_empty() {
            return Some(BigRational::zero());
        }
        (self.terms.len() == 1)
            .then(|| self.terms.get(&Monomial::default()).cloned())
            .flatten()
    }

    fn add(&self, other: &Self) -> Self {
        let mut result = self.clone();
        for (monomial, coefficient) in &other.terms {
            *result
                .terms
                .entry(monomial.clone())
                .or_insert_with(BigRational::zero) += coefficient;
        }
        result.remove_zeros();
        result
    }

    fn sub(&self, other: &Self) -> Self {
        self.add(&other.clone().scaled(-BigRational::one()))
    }

    fn mul(&self, other: &Self) -> Self {
        let mut result = Self::constant(BigRational::zero());
        for (left_monomial, left_coefficient) in &self.terms {
            for (right_monomial, right_coefficient) in &other.terms {
                let monomial = left_monomial.multiply(right_monomial);
                *result
                    .terms
                    .entry(monomial)
                    .or_insert_with(BigRational::zero) += left_coefficient * right_coefficient;
            }
        }
        result.remove_zeros();
        result
    }

    fn pow(&self, mut exponent: u32) -> Self {
        let mut base = self.clone();
        let mut result = Self::constant(BigRational::one());
        while exponent > 0 {
            if exponent & 1 == 1 {
                result = result.mul(&base);
            }
            exponent >>= 1;
            if exponent > 0 {
                base = base.mul(&base);
            }
        }
        result
    }

    fn scaled(mut self, factor: BigRational) -> Self {
        for coefficient in self.terms.values_mut() {
            *coefficient *= &factor;
        }
        self.remove_zeros();
        self
    }

    fn remove_zeros(&mut self) {
        self.terms.retain(|_, coefficient| !coefficient.is_zero());
    }

    pub fn expanded_expression(&self) -> Rc<SymbolicExpr> {
        self.to_expression()
    }

    pub fn term_count(&self) -> usize {
        self.terms.len()
    }

    pub fn horner_expression(&self) -> Rc<SymbolicExpr> {
        let Some((variable, coefficients)) = self.univariate_coefficients() else {
            return self.to_expression();
        };
        let Some(leading) = coefficients.last() else {
            return Self::integer_expression(0);
        };
        let variable = Rc::new(SymbolicExpr::Variable(variable.symbol));
        let mut result = Rc::new(SymbolicExpr::Number(Number::from_rational(leading.clone())));
        for coefficient in coefficients[..coefficients.len() - 1].iter().rev() {
            result = Self::binary(result, BinaryOp::Mul, Rc::clone(&variable));
            if !coefficient.is_zero() {
                result = Self::binary(
                    result,
                    if coefficient.is_negative() {
                        BinaryOp::Sub
                    } else {
                        BinaryOp::Add
                    },
                    Rc::new(SymbolicExpr::Number(Number::from_rational(
                        coefficient.abs(),
                    ))),
                );
            }
        }
        result
    }

    pub fn factored_expression(&self) -> Rc<SymbolicExpr> {
        if let Some(identity) = self.factor_power_identity() {
            return identity;
        }
        let (content, common) = self.content_and_common_monomial();
        if content != BigRational::one() || !common.0.is_empty() {
            let mut primitive = BTreeMap::new();
            for (monomial, coefficient) in &self.terms {
                primitive.insert(monomial.divide(&common), coefficient / &content);
            }
            let outside = Self::term_expression(content, &common);
            let inside = Self { terms: primitive }.factored_expression();
            return Self::binary(outside, BinaryOp::Mul, inside);
        }
        if let Some(factored) = self.factor_square_free_parts() {
            return factored;
        }
        if let Some(factored) = self.factor_univariate_roots() {
            return factored;
        }
        self.to_expression()
    }

    fn content_and_common_monomial(&self) -> (BigRational, Monomial) {
        let mut coefficients = self.terms.values();
        let Some(first) = coefficients.next() else {
            return (BigRational::one(), Monomial::default());
        };
        let mut numerator_gcd = first.numer().abs();
        let mut denominator_lcm = first.denom().clone();
        for coefficient in coefficients {
            numerator_gcd = numerator_gcd.gcd(&coefficient.numer().abs());
            denominator_lcm = denominator_lcm.lcm(coefficient.denom());
        }
        let content = BigRational::new(numerator_gcd, denominator_lcm);

        let mut common = self.terms.keys().next().cloned().unwrap_or_default();
        for monomial in self.terms.keys().skip(1) {
            common = common.intersection(monomial);
        }
        (content, common)
    }

    fn factor_power_identity(&self) -> Option<Rc<SymbolicExpr>> {
        if self.terms.len() != 2 {
            return None;
        }
        let mut positive = None;
        let mut second_positive = None;
        let mut negative = None;
        for (monomial, coefficient) in &self.terms {
            if coefficient == &BigRational::one() {
                if positive.is_none() {
                    positive = Some(monomial);
                } else {
                    second_positive = Some(monomial);
                }
            } else if coefficient == &-BigRational::one() {
                negative = Some(monomial);
            } else {
                return None;
            }
        }
        if let (Some(left), Some(right)) = (positive, second_positive) {
            if left.divisible_exponents(3) && right.divisible_exponents(3) {
                return Some(Self::cube_factor(left.root(3), right.root(3), true));
            }
            return None;
        }
        let (positive, negative) = (positive?, negative?);

        if positive.divisible_exponents(2) && negative.divisible_exponents(2) {
            let left = Self::term_expression(BigRational::one(), &positive.root(2));
            let right = Self::term_expression(BigRational::one(), &negative.root(2));
            let difference = Self::binary(Rc::clone(&left), BinaryOp::Sub, Rc::clone(&right));
            let sum = Self::binary(left, BinaryOp::Add, right);
            return Some(Self::binary(difference, BinaryOp::Mul, sum));
        }

        if positive.divisible_exponents(3) && negative.divisible_exponents(3) {
            return Some(Self::cube_factor(positive.root(3), negative.root(3), false));
        }
        None
    }

    fn factor_univariate_roots(&self) -> Option<Rc<SymbolicExpr>> {
        let (variable, mut coefficients) = self.univariate_coefficients()?;
        if coefficients.len() <= 2 {
            return None;
        }
        let mut factors: Vec<(BigRational, u32)> = Vec::new();
        for root in Self::root_candidates(&coefficients) {
            let mut multiplicity = 0;
            while coefficients.len() > 1
                && Self::evaluate_coefficients(&coefficients, &root).is_zero()
            {
                coefficients = Self::synthetic_divide(&coefficients, &root)?;
                multiplicity += 1;
            }
            if multiplicity > 0 {
                factors.push((root, multiplicity));
            }
        }
        if factors.is_empty() {
            return None;
        }

        let mut expression = None;
        for (root, multiplicity) in factors {
            let variable_expression = Rc::new(SymbolicExpr::Variable(variable.symbol));
            let root_expression = Rc::new(SymbolicExpr::Number(Number::from_rational(
                root.clone().abs(),
            )));
            let factor = if root.is_zero() {
                variable_expression
            } else {
                Self::binary(
                    variable_expression,
                    if root.is_negative() {
                        BinaryOp::Add
                    } else {
                        BinaryOp::Sub
                    },
                    root_expression,
                )
            };
            let factor = if multiplicity == 1 {
                factor
            } else {
                Self::binary(
                    factor,
                    BinaryOp::Pow,
                    Self::integer_expression(multiplicity),
                )
            };
            expression = Some(match expression {
                None => factor,
                Some(previous) => Self::binary(previous, BinaryOp::Mul, factor),
            });
        }

        let remainder = Self::from_univariate(variable, coefficients).to_expression();
        if !matches!(remainder.as_ref(), SymbolicExpr::Number(number) if number.is_one()) {
            expression = Some(match expression {
                None => remainder,
                Some(previous) => Self::binary(previous, BinaryOp::Mul, remainder),
            });
        }
        expression
    }

    fn factor_square_free_parts(&self) -> Option<Rc<SymbolicExpr>> {
        let (variable, coefficients) = self.univariate_coefficients()?;
        if coefficients.len() <= 2 {
            return None;
        }
        let derivative: Vec<_> = coefficients
            .iter()
            .enumerate()
            .skip(1)
            .map(|(exponent, coefficient)| coefficient * BigInt::from(exponent))
            .collect();
        let gcd = Self::polynomial_gcd(coefficients.clone(), derivative);
        if gcd.len() <= 1 {
            return None;
        }
        let (quotient, remainder) = Self::divide_coefficients(coefficients, &gcd)?;
        if remainder.iter().any(|coefficient| !coefficient.is_zero()) {
            return None;
        }
        let left = Self::from_univariate(variable.clone(), gcd).factored_expression();
        let right = Self::from_univariate(variable, quotient).factored_expression();
        if left.as_ref().equals(right.as_ref()) {
            Some(Self::binary(
                left,
                BinaryOp::Pow,
                Self::integer_expression(2),
            ))
        } else {
            Some(Self::binary(left, BinaryOp::Mul, right))
        }
    }

    fn univariate_coefficients(&self) -> Option<(Variable, Vec<BigRational>)> {
        let mut variable = None;
        let mut degree = 0usize;
        for monomial in self.terms.keys() {
            if monomial.0.len() > 1 {
                return None;
            }
            if let Some((current, exponent)) = monomial.0.first() {
                if variable.as_ref().is_some_and(|known| known != current) {
                    return None;
                }
                variable = Some(current.clone());
                degree = degree.max(*exponent as usize);
            }
        }
        let variable = variable?;
        let mut coefficients = vec![BigRational::zero(); degree + 1];
        for (monomial, coefficient) in &self.terms {
            let exponent = monomial
                .0
                .first()
                .map_or(0, |(_, exponent)| *exponent as usize);
            coefficients[exponent] = coefficient.clone();
        }
        while coefficients.last().is_some_and(Zero::is_zero) {
            coefficients.pop();
        }
        Some((variable, coefficients))
    }

    fn coefficients_for(&self, variable: &Variable) -> Option<Vec<BigRational>> {
        let mut degree = 0usize;
        for monomial in self.terms.keys() {
            if monomial.0.len() > 1 {
                return None;
            }
            if let Some((current, exponent)) = monomial.0.first() {
                if current != variable {
                    return None;
                }
                degree = degree.max(*exponent as usize);
            }
        }
        let mut coefficients = vec![BigRational::zero(); degree + 1];
        for (monomial, coefficient) in &self.terms {
            let exponent = monomial
                .0
                .first()
                .map_or(0, |(_, exponent)| *exponent as usize);
            coefficients[exponent] = coefficient.clone();
        }
        Some(coefficients)
    }

    fn root_candidates(coefficients: &[BigRational]) -> Vec<BigRational> {
        let mut candidates = Vec::new();
        let denominator_lcm = coefficients.iter().fold(BigInt::one(), |lcm, coefficient| {
            lcm.lcm(coefficient.denom())
        });
        let integer_coefficients: Vec<_> = coefficients
            .iter()
            .map(|coefficient| coefficient.numer() * (&denominator_lcm / coefficient.denom()))
            .collect();
        let constant = integer_coefficients
            .first()
            .and_then(|value| value.abs().to_u64());
        let leading = integer_coefficients
            .last()
            .and_then(|value| value.abs().to_u64());

        let bounded = match (constant, leading) {
            (Some(constant), Some(leading))
                if constant <= 1_000_000_000_000 && leading <= 1_000_000_000_000 =>
            {
                Some((constant, leading))
            }
            _ => None,
        };
        if let Some((constant, leading)) = bounded {
            if constant == 0 {
                // Zero is a root; the rational-root enumeration must then run
                // on the polynomial with the zero roots stripped off, since
                // the theorem needs a nonzero constant term.
                candidates.push(BigRational::zero());
                let reduced = &integer_coefficients[integer_coefficients
                    .iter()
                    .position(|coefficient| !coefficient.is_zero())
                    .unwrap_or(integer_coefficients.len())..];
                let bounded = match (
                    reduced.first().and_then(|value| value.abs().to_u64()),
                    reduced.last().and_then(|value| value.abs().to_u64()),
                ) {
                    (Some(constant), Some(leading))
                        if constant <= 1_000_000_000_000 && leading <= 1_000_000_000_000 =>
                    {
                        Some((constant, leading))
                    }
                    _ => None,
                };
                if let Some((constant, leading)) = bounded {
                    for numerator in Self::positive_divisors(constant) {
                        for denominator in &Self::positive_divisors(leading) {
                            let candidate = BigRational::new(
                                BigInt::from(numerator),
                                BigInt::from(*denominator),
                            );
                            candidates.push(candidate.clone());
                            candidates.push(-candidate);
                        }
                    }
                }
            } else {
                let numerators = Self::positive_divisors(constant);
                let denominators = Self::positive_divisors(leading);
                for numerator in numerators {
                    for denominator in &denominators {
                        let candidate =
                            BigRational::new(BigInt::from(numerator), BigInt::from(*denominator));
                        candidates.push(candidate.clone());
                        candidates.push(-candidate);
                    }
                }
            }
        } else {
            for denominator in 1..=32 {
                for numerator in -256..=256 {
                    candidates.push(BigRational::new(
                        BigInt::from(numerator),
                        BigInt::from(denominator),
                    ));
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn positive_divisors(value: u64) -> Vec<u64> {
        let mut divisors = Vec::new();
        let mut candidate = 1;
        while candidate <= value / candidate {
            if value % candidate == 0 {
                divisors.push(candidate);
                let paired = value / candidate;
                if paired != candidate {
                    divisors.push(paired);
                }
            }
            candidate += 1;
        }
        divisors.sort_unstable();
        divisors
    }

    fn from_univariate(variable: Variable, coefficients: Vec<BigRational>) -> Self {
        let terms = coefficients
            .into_iter()
            .enumerate()
            .filter(|(_, coefficient)| !coefficient.is_zero())
            .map(|(exponent, coefficient)| {
                let monomial = if exponent == 0 {
                    Monomial::default()
                } else {
                    Monomial(vec![(variable.clone(), exponent as u32)])
                };
                (monomial, coefficient)
            })
            .collect();
        Self { terms }
    }

    fn evaluate_coefficients(coefficients: &[BigRational], value: &BigRational) -> BigRational {
        coefficients
            .iter()
            .rev()
            .fold(BigRational::zero(), |result, coefficient| {
                result * value + coefficient
            })
    }

    fn synthetic_divide(
        coefficients: &[BigRational],
        root: &BigRational,
    ) -> Option<Vec<BigRational>> {
        let degree = coefficients.len().checked_sub(1)?;
        if degree == 0 {
            return None;
        }
        let mut quotient = vec![BigRational::zero(); degree];
        quotient[degree - 1] = coefficients[degree].clone();
        for index in (1..degree).rev() {
            quotient[index - 1] = coefficients[index].clone() + root * &quotient[index];
        }
        let remainder = coefficients[0].clone() + root * &quotient[0];
        remainder.is_zero().then_some(quotient)
    }

    fn polynomial_gcd(mut left: Vec<BigRational>, mut right: Vec<BigRational>) -> Vec<BigRational> {
        Self::trim_coefficients(&mut left);
        Self::trim_coefficients(&mut right);
        while !right.is_empty() {
            let remainder = Self::divide_coefficients(left, &right)
                .map(|(_, remainder)| remainder)
                .unwrap_or_default();
            left = right;
            right = remainder;
            Self::trim_coefficients(&mut right);
        }
        let Some(leading) = left.last().cloned() else {
            return left;
        };
        for coefficient in &mut left {
            *coefficient /= &leading;
        }
        left
    }

    fn divide_coefficients(
        mut dividend: Vec<BigRational>,
        divisor: &[BigRational],
    ) -> Option<(Vec<BigRational>, Vec<BigRational>)> {
        Self::trim_coefficients(&mut dividend);
        let mut divisor = divisor.to_vec();
        Self::trim_coefficients(&mut divisor);
        if divisor.is_empty() {
            return None;
        }
        if dividend.len() < divisor.len() {
            return Some((vec![BigRational::zero()], dividend));
        }
        let mut quotient = vec![BigRational::zero(); dividend.len() - divisor.len() + 1];
        while dividend.len() >= divisor.len() && !dividend.is_empty() {
            let exponent = dividend.len() - divisor.len();
            let coefficient = dividend.last()? / divisor.last()?;
            quotient[exponent] = coefficient.clone();
            for (index, divisor_coefficient) in divisor.iter().enumerate() {
                dividend[index + exponent] -= &coefficient * divisor_coefficient;
            }
            Self::trim_coefficients(&mut dividend);
        }
        Some((quotient, dividend))
    }

    fn trim_coefficients(coefficients: &mut Vec<BigRational>) {
        while coefficients.last().is_some_and(Zero::is_zero) {
            coefficients.pop();
        }
    }

    fn cube_factor(left: Monomial, right: Monomial, sum: bool) -> Rc<SymbolicExpr> {
        let a = Self::term_expression(BigRational::one(), &left);
        let b = Self::term_expression(BigRational::one(), &right);
        let first = Self::binary(
            Rc::clone(&a),
            if sum { BinaryOp::Add } else { BinaryOp::Sub },
            Rc::clone(&b),
        );
        let a_squared = Self::binary(Rc::clone(&a), BinaryOp::Pow, Self::integer_expression(2));
        let ab = Self::binary(Rc::clone(&a), BinaryOp::Mul, Rc::clone(&b));
        let b_squared = Self::binary(b, BinaryOp::Pow, Self::integer_expression(2));
        let middle = Self::binary(
            a_squared,
            if sum { BinaryOp::Sub } else { BinaryOp::Add },
            ab,
        );
        let second = Self::binary(middle, BinaryOp::Add, b_squared);
        Self::binary(first, BinaryOp::Mul, second)
    }

    fn to_expression(&self) -> Rc<SymbolicExpr> {
        if self.terms.is_empty() {
            return Self::integer_expression(0);
        }
        let mut terms: Vec<_> = self.terms.iter().collect();
        terms.sort_by(|(left, _), (right, _)| {
            right
                .degree()
                .cmp(&left.degree())
                .then_with(|| left.term_order(right))
        });

        let mut expression = None;
        for (monomial, coefficient) in terms {
            let negative = coefficient.is_negative();
            let term = Self::term_expression(coefficient.abs(), monomial);
            expression = Some(match expression {
                None if negative => Rc::new(SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr: term,
                }),
                None => term,
                Some(previous) => Self::binary(
                    previous,
                    if negative {
                        BinaryOp::Sub
                    } else {
                        BinaryOp::Add
                    },
                    term,
                ),
            });
        }
        expression.expect("non-empty polynomial has a term")
    }

    fn term_expression(coefficient: BigRational, monomial: &Monomial) -> Rc<SymbolicExpr> {
        let mut factors = Vec::new();
        if coefficient != BigRational::one() || monomial.0.is_empty() {
            factors.push(Rc::new(SymbolicExpr::Number(Number::from_rational(
                coefficient,
            ))));
        }
        for (variable, exponent) in &monomial.0 {
            let variable = Rc::new(SymbolicExpr::Variable(variable.symbol));
            factors.push(if *exponent == 1 {
                variable
            } else {
                Self::binary(variable, BinaryOp::Pow, Self::integer_expression(*exponent))
            });
        }
        factors
            .into_iter()
            .reduce(|left, right| Self::binary(left, BinaryOp::Mul, right))
            .unwrap_or_else(|| Self::integer_expression(1))
    }

    fn integer_expression(value: impl Into<BigInt>) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Number(Number::Integer(value.into())))
    }

    fn binary(left: Rc<SymbolicExpr>, op: BinaryOp, right: Rc<SymbolicExpr>) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Binary { left, op, right })
    }
}

impl Monomial {
    fn multiply(&self, other: &Self) -> Self {
        let mut powers: BTreeMap<Variable, u32> = self.0.iter().cloned().collect();
        for (variable, exponent) in &other.0 {
            *powers.entry(variable.clone()).or_default() += exponent;
        }
        Self(
            powers
                .into_iter()
                .filter(|(_, exponent)| *exponent != 0)
                .collect(),
        )
    }

    fn intersection(&self, other: &Self) -> Self {
        let other: BTreeMap<_, _> = other.0.iter().cloned().collect();
        Self(
            self.0
                .iter()
                .filter_map(|(variable, exponent)| {
                    other
                        .get(variable)
                        .map(|other_exponent| (variable.clone(), (*exponent).min(*other_exponent)))
                })
                .filter(|(_, exponent)| *exponent != 0)
                .collect(),
        )
    }

    fn divide(&self, divisor: &Self) -> Self {
        let divisor: BTreeMap<_, _> = divisor.0.iter().cloned().collect();
        Self(
            self.0
                .iter()
                .filter_map(|(variable, exponent)| {
                    let remaining = exponent - divisor.get(variable).copied().unwrap_or(0);
                    (remaining != 0).then(|| (variable.clone(), remaining))
                })
                .collect(),
        )
    }

    fn degree(&self) -> u32 {
        self.0.iter().map(|(_, exponent)| exponent).sum()
    }

    fn divisible_exponents(&self, divisor: u32) -> bool {
        self.0.iter().all(|(_, exponent)| exponent % divisor == 0)
    }

    fn root(&self, divisor: u32) -> Self {
        Self(
            self.0
                .iter()
                .map(|(variable, exponent)| (variable.clone(), exponent / divisor))
                .collect(),
        )
    }

    fn term_order(&self, other: &Self) -> Ordering {
        let left: BTreeMap<_, _> = self.0.iter().cloned().collect();
        let right: BTreeMap<_, _> = other.0.iter().cloned().collect();
        let mut variables: Vec<_> = left.keys().chain(right.keys()).cloned().collect();
        variables.sort();
        variables.dedup();
        for variable in variables {
            let left_exponent = left.get(&variable).copied().unwrap_or(0);
            let right_exponent = right.get(&variable).copied().unwrap_or(0);
            match right_exponent.cmp(&left_exponent) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}
