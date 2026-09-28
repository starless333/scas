use crate::ast::binary_op::BinaryOp;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::value::SymbolicExpr;
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::number::Number;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::rc::Rc;

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
        let mut roots = Self::root_candidates(&coefficients)
            .into_iter()
            .filter(|root| Self::evaluate_coefficients(&coefficients, root).is_zero())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        Some(
            roots
                .into_iter()
                .map(|root| Rc::new(SymbolicExpr::Number(Number::from_rational(root))))
                .collect(),
        )
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
        let numerator_coefficients = numerator.coefficients_for(&variable)?;
        if numerator_coefficients.len() >= denominator_coefficients.len() {
            return None;
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
        result
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
                candidates.push(BigRational::zero());
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
