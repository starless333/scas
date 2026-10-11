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
        UnaryMathFunction::Acosh => {
            // dashu-cmplx's `acosh` hangs outside the real domain, so the
            // principal branch is assembled by hand:
            // acosh(z) = ln(z + sqrt(z - 1) * sqrt(z + 1)).
            let one = DecimalComplex::from(DBig::ONE.with_precision(value.precision()).value());
            let root = (value.clone() - one.clone()).sqrt() * (value.clone() + one).sqrt();
            (value + root).ln()
        }
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

/// The entire series converges everywhere. Extra guard digits account for
/// cancellation in the alternating terms away from the origin.
fn erf_complex(
    number: &Number,
    precision: usize,
    complement: bool,
) -> Result<Number, MathEvaluationError> {
    let work_precision = precision.saturating_mul(3).saturating_add(100);
    let value = complex_float(number, work_precision)?;
    let (real, imaginary) = value.clone().into_parts();
    let radius_squared = &real * &real + &imaginary * &imaginary;
    let asymptotic_threshold = DBig::from(precision.saturating_mul(3).max(100))
        .with_precision(work_precision)
        .value();
    let one = DecimalComplex::from(DBig::ONE.with_precision(work_precision).value());
    let result = if radius_squared > asymptotic_threshold {
        // erfc(z) ~ exp(-z²)/(sqrt(pi) z) Σ (-1)^n (2n-1)!!/(2z²)^n.
        // Reflect the left half-plane first so the asymptotic branch stays
        // within |arg(z)| <= pi/2.
        let reflected = real < DBig::ZERO;
        let argument = if reflected { -value } else { value };
        let square = argument.clone() * argument.clone();
        let mut term = one.clone();
        let mut sum = term.clone();
        let tolerance = decimal_float(
            &format!("1e-{}", precision.saturating_add(8)),
            work_precision,
        )?;
        let mut converged = false;
        for index in 1..=precision.saturating_mul(8).saturating_add(64) {
            term = -term
                * DecimalComplex::from(
                    DBig::from(2 * index - 1)
                        .with_precision(work_precision)
                        .value(),
                )
                / (DecimalComplex::from(DBig::from(2).with_precision(work_precision).value())
                    * square.clone());
            let (term_real, term_imaginary) = term.clone().into_parts();
            sum += term.clone();
            if term_real.abs() < tolerance && term_imaginary.abs() < tolerance {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(MathEvaluationError(
                "complex erf asymptotic expansion did not converge at the requested precision"
                    .to_owned(),
            ));
        }
        let erfc = (-square).exp() * sum
            / (DecimalComplex::from(DBig::pi(work_precision).sqrt()) * argument);
        if reflected {
            if complement {
                DecimalComplex::from(DBig::from(2).with_precision(work_precision).value()) - erfc
            } else {
                erfc - one.clone()
            }
        } else if complement {
            erfc
        } else {
            one.clone() - erfc
        }
    } else {
        let square = value.clone() * value.clone();
        let mut term = value;
        let mut sum = term.clone();
        let iterations = precision.saturating_mul(8).saturating_add(1000).min(16_384);
        for index in 1..iterations {
            let divisor =
                DecimalComplex::from(DBig::from(index).with_precision(work_precision).value());
            term = -(term * square.clone()) / divisor;
            let odd = DecimalComplex::from(
                DBig::from(2 * index + 1)
                    .with_precision(work_precision)
                    .value(),
            );
            sum += term.clone() / odd;
        }
        let scale = DecimalComplex::from(
            DBig::from(2).with_precision(work_precision).value() / DBig::pi(work_precision).sqrt(),
        );
        let erf = scale * sum;
        if complement {
            one - erf
        } else {
            erf
        }
    };
    let (mut result_real, imaginary) = result.into_parts();
    if real == DBig::ZERO {
        result_real = if complement { DBig::ONE } else { DBig::ZERO };
    }
    complex_number(DecimalComplex::from_parts(
        result_real.with_precision(precision).value(),
        imaginary.with_precision(precision).value(),
    ))
}

fn gamma_complex(
    number: &Number,
    precision: usize,
    logarithm: bool,
) -> Result<Number, MathEvaluationError> {
    let work_precision = precision.saturating_add(16);
    let value = complex_float(number, work_precision)?;
    let (real, _) = value.clone().into_parts();
    let one = DecimalComplex::from(DBig::ONE.with_precision(work_precision).value());
    let gamma = if real < decimal_float("0.5", work_precision)? {
        let reflected = one.clone() - value.clone();
        let sine = (DecimalComplex::from(DBig::pi(work_precision)) * value).sin();
        DecimalComplex::from(DBig::pi(work_precision))
            / (sine * gamma_complex_positive(reflected, work_precision)?)
    } else {
        gamma_complex_positive(value, work_precision)?
    };
    let result = if logarithm { gamma.ln() } else { gamma };
    let (real, imaginary) = result.into_parts();
    complex_number(DecimalComplex::from_parts(
        real.with_precision(precision).value(),
        imaginary.with_precision(precision).value(),
    ))
}

fn gamma_complex_positive(
    mut value: DecimalComplex,
    work_precision: usize,
) -> Result<DecimalComplex, MathEvaluationError> {
    let (real, _) = value.clone().into_parts();
    let threshold = DBig::from(work_precision / 2 + 12)
        .with_precision(work_precision)
        .value();
    let one = DecimalComplex::from(DBig::ONE.with_precision(work_precision).value());
    let half = DecimalComplex::from(decimal_float("0.5", work_precision)?);
    let mut correction = one.clone();
    let mut shifted_real = real;
    while shifted_real < threshold {
        correction *= value.clone();
        value += one.clone();
        shifted_real += DBig::ONE;
    }
    let terms = work_precision / 2 + 8;
    let generated = (terms > EVEN_BERNOULLI.len()).then(|| bernoulli_numbers(terms * 2));
    let pi = DBig::pi(work_precision);
    let log_two_pi = (DBig::from(2).with_precision(work_precision).value() * pi).ln();
    let mut result = (value.clone() - half) * value.clone().ln() - value.clone()
        + DecimalComplex::from(decimal_float("0.5", work_precision)? * log_two_pi);
    let squared = value.clone() * value.clone();
    let mut odd_power = value;
    for k in 1..=terms {
        let index = k * 2;
        let coefficient = if let Some((numerator, denominator)) = EVEN_BERNOULLI.get(k - 1) {
            rational_float(numerator, denominator, work_precision)?
        } else {
            let bernoulli = &generated.as_ref().expect("generated above")[index];
            rational_float(
                &bernoulli.numer().to_string(),
                &bernoulli.denom().to_string(),
                work_precision,
            )?
        };
        result += DecimalComplex::from(coefficient)
            / (DecimalComplex::from(
                DBig::from(index * (index - 1))
                    .with_precision(work_precision)
                    .value(),
            ) * odd_power.clone());
        odd_power *= squared.clone();
    }
    Ok(result.exp() / correction)
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
        if matches!(number, Number::Complex(_))
            && matches!(function, UnaryMathFunction::Erf | UnaryMathFunction::Erfc)
        {
            return erf_complex(number, precision, function == UnaryMathFunction::Erfc);
        }
        if matches!(number, Number::Complex(_)) {
            return gamma_complex(number, precision, function == UnaryMathFunction::Lgamma);
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

// Even Bernoulli numbers used by the log-gamma asymptotic expansion. These
// exact coefficients are compile-time data: generating the full Akiyama-
// Tanigawa table for every gamma call used to dominate evaluation time.
const EVEN_BERNOULLI: &[(&str, &str)] = &[
    ("1", "6"),
    ("-1", "30"),
    ("1", "42"),
    ("-1", "30"),
    ("5", "66"),
    ("-691", "2730"),
    ("7", "6"),
    ("-3617", "510"),
    ("43867", "798"),
    ("-174611", "330"),
    ("854513", "138"),
    ("-236364091", "2730"),
    ("8553103", "6"),
    ("-23749461029", "870"),
    ("8615841276005", "14322"),
    ("-7709321041217", "510"),
    ("2577687858367", "6"),
    ("-26315271553053477373", "1919190"),
    ("2929993913841559", "6"),
    ("-261082718496449122051", "13530"),
    ("1520097643918070802691", "1806"),
    ("-27833269579301024235023", "690"),
    ("596451111593912163277961", "282"),
    ("-5609403368997817686249127547", "46410"),
    ("495057205241079648212477525", "66"),
    ("-801165718135489957347924991853", "1590"),
    ("29149963634884862421418123812691", "798"),
    ("-2479392929313226753685415739663229", "870"),
    ("84483613348880041862046775994036021", "354"),
    ("-1215233140483755572040304994079820246041491", "56786730"),
    ("12300585434086858541953039857403386151", "6"),
    ("-106783830147866529886385444979142647942017", "510"),
    ("1472600022126335654051619428551932342241899101", "64722"),
    ("-78773130858718728141909149208474606244347001", "30"),
    ("1505381347333367003803076567377857208511438160235", "4686"),
    ("-5827954961669944110438277244641067365282488301844260429", "140100870"),
    ("34152417289221168014330073731472635186688307783087", "6"),
    ("-24655088825935372707687196040585199904365267828865801", "30"),
    ("414846365575400828295179035549542073492199375372400483487", "3318"),
    ("-4603784299479457646935574969019046849794257872751288919656867", "230010"),
    ("1677014149185145836823154509786269900207736027570253414881613", "498"),
    ("-2024576195935290360231131160111731009989917391198090877281083932477", "3404310"),
    ("660714619417678653573847847426261496277830686653388931761996983", "6"),
    ("-1311426488674017507995511424019311843345750275572028644296919890574047", "61410"),
    ("1179057279021082799884123351249215083775254949669647116231545215727922535", "272118"),
    ("-1295585948207537527989427828538576749659341483719435143023316326829946247", "1410"),
    ("1220813806579744469607301679413201203958508415202696621436215105284649447", "6"),
    ("-211600449597266513097597728109824233673043954389060234150638733420050668349987259", "4501770"),
    ("67908260672905495624051117546403605607342195728504487509073961249992947058239", "6"),
    ("-94598037819122125295227433069493721872702841533066936133385696204311395415197247711", "33330"),
    ("3204019410860907078243020782116241775491817197152717450679002501086861530836678158791", "4326"),
    ("-319533631363830011287103352796174274671189606078272738327103470162849568365549721224053", "1590"),
    ("36373903172617414408151820151593427169231298640581690038930816378281879873386202346572901", "642"),
    ("-3469342247847828789552088659323852541399766785760491146870005891371501266319724897592306597338057", "209191710"),
    ("7645992940484742892248134246724347500528752413412307906683593870759797606269585779977930217515", "1518"),
    ("-2650879602155099713352597214685162014443151499192509896451788427680966756514875515366781203552600109", "1671270"),
    ("21737832319369163333310761086652991475721156679090831360806110114933605484234593650904188618562649", "42"),
    ("-309553916571842976912513458033841416869004128064329844245504045721008957524571968271388199595754752259", "1770"),
    ("366963119969713111534947151585585006684606361080699204301059440676414485045806461889371776354517095799", "6"),
    ("-51507486535079109061843996857849983274095170353262675213092869167199297474922985358811329367077682677803282070131", "2328255930"),
    ("49633666079262581912532637475990757438722790311060139770309311793150683214100431329033113678098037968564431", "6"),
    ("-95876775334247128750774903107542444620578830013297336819553512729358593354435944413631943610268472689094609001", "30"),
    ("5556330281949274850616324408918951380525567307126747246796782304333594286400508981287241419934529638692081513802696639", "4357878"),
    ("-267754707742548082886954405585282394779291459592551740629978686063357792734863530145362663093519862048495908453718017", "510"),
];

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

fn rational_float(
    numerator: &str,
    denominator: &str,
    precision: usize,
) -> Result<DBig, MathEvaluationError> {
    let numerator = decimal_float(numerator, precision)?;
    let denominator = decimal_float(denominator, precision)?;
    Ok(numerator / denominator)
}

fn log_gamma_positive(mut value: DBig, precision: usize) -> Result<DBig, MathEvaluationError> {
    let work_precision = precision;
    value = value.with_precision(work_precision).value();
    if value <= DBig::ZERO {
        return Err(MathEvaluationError(
            "log-gamma positive branch requires a positive argument".to_string(),
        ));
    }
    let terms = precision.saturating_div(2).saturating_add(8).max(12);
    let generated =
        (terms > EVEN_BERNOULLI.len()).then(|| bernoulli_numbers(terms.saturating_mul(2)));
    let threshold_integer = precision.saturating_div(2).saturating_add(10);
    let threshold = DBig::from(threshold_integer)
        .with_precision(work_precision)
        .value();
    let one = DBig::ONE.with_precision(work_precision).value();
    let half = decimal_float("0.5", work_precision)?;
    // Γ(x + n) = (x)(x + 1)…(x + n - 1)Γ(x). Build that product and take
    // one logarithm instead of evaluating a logarithm at every recurrence
    // step; multiplication is dramatically cheaper at high precision.
    let mut correction_product = DBig::ONE.with_precision(work_precision).value();
    while value < threshold {
        correction_product *= &value;
        value += &one;
    }
    let correction = correction_product.ln();

    let pi = DBig::pi(work_precision);
    let mut result = (&value - &half) * value.ln() - &value
        + &half * (DBig::from(2).with_precision(work_precision).value() * pi).ln();
    let value_squared = &value * &value;
    let mut odd_power = value.clone();
    for k in 1..=terms {
        let index = k * 2;
        let coefficient = if let Some((numerator, denominator)) = EVEN_BERNOULLI.get(k - 1) {
            rational_float(numerator, denominator, work_precision)?
        } else {
            let bernoulli = &generated.as_ref().expect("generated above")[index];
            rational_float(
                &bernoulli.numer().to_string(),
                &bernoulli.denom().to_string(),
                work_precision,
            )?
        };
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
    let work_precision = precision.saturating_add(8);
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
