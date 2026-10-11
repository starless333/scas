use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct C64 {
    pub re: f64,
    pub im: f64,
}

impl C64 {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };

    pub fn conj(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

impl From<f64> for C64 {
    fn from(re: f64) -> Self {
        Self { re, im: 0.0 }
    }
}
impl Add for C64 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}
impl Sub for C64 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }
}
impl Neg for C64 {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            re: -self.re,
            im: -self.im,
        }
    }
}
impl Mul for C64 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}
impl Div for C64 {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        let denominator = rhs.re * rhs.re + rhs.im * rhs.im;
        Self {
            re: (self.re * rhs.re + self.im * rhs.im) / denominator,
            im: (self.im * rhs.re - self.re * rhs.im) / denominator,
        }
    }
}

fn adjoint(matrix: &[Vec<C64>]) -> Vec<Vec<C64>> {
    (0..matrix[0].len())
        .map(|column| matrix.iter().map(|row| row[column].conj()).collect())
        .collect()
}

fn gram(matrix: &[Vec<C64>]) -> Vec<Vec<C64>> {
    let columns = matrix[0].len();
    (0..columns)
        .map(|i| {
            (0..columns)
                .map(|j| {
                    matrix
                        .iter()
                        .map(|row| row[i].conj() * row[j])
                        .fold(C64::ZERO, |sum, value| sum + value)
                })
                .collect()
        })
        .collect()
}

/// Hermitian Jacobi diagonalization; columns of the second result are eigenvectors.
pub(super) fn hermitian_eigen(mut matrix: Vec<Vec<C64>>) -> Option<(Vec<f64>, Vec<Vec<C64>>)> {
    let n = matrix.len();
    if n == 0 || matrix.iter().any(|row| row.len() != n) {
        return None;
    }
    let mut vectors = vec![vec![C64::ZERO; n]; n];
    for (index, row) in vectors.iter_mut().enumerate() {
        row[index] = C64::ONE;
    }
    let scale = matrix
        .iter()
        .flatten()
        .map(|entry| entry.abs())
        .fold(1.0_f64, f64::max);
    let tolerance = scale * 1e-14;
    let mut converged = false;
    for _ in 0..(100 * n * n) {
        let mut largest = 0.0;
        let mut pivot = (0, 0);
        for p in 0..n {
            for q in p + 1..n {
                if matrix[p][q].abs() > largest {
                    largest = matrix[p][q].abs();
                    pivot = (p, q);
                }
            }
        }
        if largest <= tolerance {
            converged = true;
            break;
        }
        let (p, q) = pivot;
        let phase = matrix[p][q] / C64::from(largest);
        let tau = (matrix[q][q].re - matrix[p][p].re) / (2.0 * largest);
        let t = if tau >= 0.0 {
            1.0 / (tau + (1.0 + tau * tau).sqrt())
        } else {
            -1.0 / (-tau + (1.0 + tau * tau).sqrt())
        };
        let c = 1.0 / (1.0 + t * t).sqrt();
        let s = t * c;
        let a = matrix[p][p].re;
        let d = matrix[q][q].re;
        matrix[p][p] = C64::from(a - t * largest);
        matrix[q][q] = C64::from(d + t * largest);
        matrix[p][q] = C64::ZERO;
        matrix[q][p] = C64::ZERO;
        for k in 0..n {
            if k != p && k != q {
                let kp = matrix[k][p];
                let kq = matrix[k][q];
                matrix[k][p] = C64::from(c) * kp - C64::from(s) * phase.conj() * kq;
                matrix[k][q] = C64::from(s) * kp + C64::from(c) * phase.conj() * kq;
                matrix[p][k] = matrix[k][p].conj();
                matrix[q][k] = matrix[k][q].conj();
            }
            let vp = vectors[k][p];
            let vq = vectors[k][q];
            vectors[k][p] = C64::from(c) * vp - C64::from(s) * phase.conj() * vq;
            vectors[k][q] = C64::from(s) * vp + C64::from(c) * phase.conj() * vq;
        }
    }
    if !converged || matrix.iter().flatten().any(|entry| !entry.finite()) {
        return None;
    }
    Some((
        (0..n).map(|index| matrix[index][index].re).collect(),
        vectors,
    ))
}

/// Full numerical SVD, including rank-deficient and rectangular matrices.
pub(super) fn svd(matrix: Vec<Vec<C64>>) -> Option<(Vec<Vec<C64>>, Vec<Vec<f64>>, Vec<Vec<C64>>)> {
    let rows = matrix.len();
    let columns = matrix.first()?.len();
    if rows == 0
        || columns == 0
        || matrix
            .iter()
            .any(|row| row.len() != columns || row.iter().any(|entry| !entry.finite()))
    {
        return None;
    }
    if rows < columns {
        let (u_t, sigma_t, vt_t) = svd(adjoint(&matrix))?;
        let u = adjoint(&vt_t);
        let sigma = (0..rows)
            .map(|row| (0..columns).map(|column| sigma_t[column][row]).collect())
            .collect();
        return Some((u, sigma, adjoint(&u_t)));
    }
    let (eigenvalues, eigenvectors) = hermitian_eigen(gram(&matrix))?;
    let mut order = (0..columns).collect::<Vec<_>>();
    order.sort_by(|left, right| eigenvalues[*right].total_cmp(&eigenvalues[*left]));
    let sigma_values = order
        .iter()
        .map(|index| eigenvalues[*index].max(0.0).sqrt())
        .collect::<Vec<_>>();
    let v = (0..columns)
        .map(|row| {
            order
                .iter()
                .map(|index| eigenvectors[row][*index])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut u = vec![vec![C64::ZERO; rows]; rows];
    let scale = sigma_values.first().copied().unwrap_or(0.0).max(1.0);
    for column in 0..rows {
        let mut candidate = if column < columns && sigma_values[column] > scale * 1e-12 {
            (0..rows)
                .map(|row| {
                    (0..columns)
                        .map(|k| matrix[row][k] * v[k][column])
                        .fold(C64::ZERO, |sum, value| sum + value)
                        / C64::from(sigma_values[column])
                })
                .collect::<Vec<_>>()
        } else {
            vec![C64::ZERO; rows]
        };
        if candidate
            .iter()
            .map(|entry| entry.abs() * entry.abs())
            .sum::<f64>()
            .sqrt()
            < 1e-10
        {
            let mut found = false;
            for seed in 0..rows {
                let mut trial = vec![C64::ZERO; rows];
                trial[seed] = C64::ONE;
                for prior in 0..column {
                    let dot = (0..rows)
                        .map(|row| u[row][prior].conj() * trial[row])
                        .fold(C64::ZERO, |sum, value| sum + value);
                    for row in 0..rows {
                        trial[row] = trial[row] - u[row][prior] * dot;
                    }
                }
                if trial
                    .iter()
                    .map(|entry| entry.abs() * entry.abs())
                    .sum::<f64>()
                    .sqrt()
                    > 1e-10
                {
                    candidate = trial;
                    found = true;
                    break;
                }
            }
            if !found {
                return None;
            }
        }
        for prior in 0..column {
            let dot = (0..rows)
                .map(|row| u[row][prior].conj() * candidate[row])
                .fold(C64::ZERO, |sum, value| sum + value);
            for row in 0..rows {
                candidate[row] = candidate[row] - u[row][prior] * dot;
            }
        }
        let norm = candidate
            .iter()
            .map(|entry| entry.abs() * entry.abs())
            .sum::<f64>()
            .sqrt();
        if norm < 1e-10 {
            return None;
        }
        for row in 0..rows {
            u[row][column] = candidate[row] / C64::from(norm);
        }
    }
    let mut sigma = vec![vec![0.0; columns]; rows];
    for index in 0..columns {
        sigma[index][index] = sigma_values[index];
    }
    Some((u, sigma, adjoint(&v)))
}

fn matmul(left: &[Vec<C64>], right: &[Vec<C64>]) -> Vec<Vec<C64>> {
    let n = left.len();
    (0..n)
        .map(|row| {
            (0..n)
                .map(|column| {
                    (0..n)
                        .map(|index| left[row][index] * right[index][column])
                        .fold(C64::ZERO, |sum, value| sum + value)
                })
                .collect()
        })
        .collect()
}

/// Numeric eigenpairs of a small general complex matrix. Polynomial roots
/// supply the eigenvalues; the smallest right singular vector of A-lambda I
/// supplies each eigenvector. Residual checks reject unstable results.
pub(super) fn general_eigenpairs(matrix: Vec<Vec<C64>>) -> Option<Vec<(C64, Vec<C64>)>> {
    let n = matrix.len();
    if !(3..=6).contains(&n)
        || matrix
            .iter()
            .any(|row| row.len() != n || row.iter().any(|entry| !entry.finite()))
    {
        return None;
    }
    let scale = matrix
        .iter()
        .flatten()
        .map(|value| value.abs())
        .fold(1.0_f64, f64::max);
    let a = matrix
        .iter()
        .map(|row| {
            row.iter()
                .map(|entry| *entry / C64::from(scale))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut b = vec![vec![C64::ZERO; n]; n];
    for index in 0..n {
        b[index][index] = C64::ONE;
    }
    let mut coefficients = vec![C64::ONE];
    for degree in 1..=n {
        let mut product = matmul(&a, &b);
        let trace = (0..n)
            .map(|index| product[index][index])
            .fold(C64::ZERO, |sum, value| sum + value);
        let coefficient = -trace / C64::from(degree as f64);
        coefficients.push(coefficient);
        for index in 0..n {
            product[index][index] = product[index][index] + coefficient;
        }
        b = product;
    }
    let radius = 1.0
        + coefficients
            .iter()
            .skip(1)
            .map(|coefficient| coefficient.abs())
            .fold(0.0_f64, f64::max);
    let mut roots = (0..n)
        .map(|index| {
            let angle = std::f64::consts::TAU * (index as f64 + 0.271_828) / n as f64;
            C64 {
                re: radius * angle.cos(),
                im: radius * angle.sin(),
            }
        })
        .collect::<Vec<_>>();
    let mut converged = false;
    for _ in 0..4_000 {
        let previous = roots.clone();
        let mut largest_delta = 0.0_f64;
        for index in 0..n {
            let value = coefficients
                .iter()
                .skip(1)
                .fold(coefficients[0], |sum, coefficient| {
                    sum * previous[index] + *coefficient
                });
            let denominator = previous
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, root)| previous[index] - *root)
                .fold(C64::ONE, |product, value| product * value);
            if denominator.abs() < 1e-20 {
                return None;
            }
            let delta = value / denominator;
            roots[index] = previous[index] - delta;
            largest_delta = largest_delta.max(delta.abs());
        }
        if largest_delta < 1e-12 {
            converged = true;
            break;
        }
    }
    if !converged {
        return None;
    }
    let mut pairs = Vec::with_capacity(n);
    for root in roots {
        if !root.finite() {
            return None;
        }
        let mut shifted = a.clone();
        for index in 0..n {
            shifted[index][index] = shifted[index][index] - root;
        }
        let (singular_squared, vectors) = hermitian_eigen(gram(&shifted))?;
        let smallest = (0..n)
            .min_by(|left, right| singular_squared[*left].total_cmp(&singular_squared[*right]))?;
        let vector = (0..n).map(|row| vectors[row][smallest]).collect::<Vec<_>>();
        let residual = (0..n)
            .map(|row| {
                (0..n)
                    .map(|column| shifted[row][column] * vector[column])
                    .fold(C64::ZERO, |sum, value| sum + value)
                    .abs()
                    .powi(2)
            })
            .sum::<f64>()
            .sqrt();
        if residual > 1e-7 {
            return None;
        }
        pairs.push((root * C64::from(scale), vector));
    }
    Some(pairs)
}
