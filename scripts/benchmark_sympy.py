#!/usr/bin/env python3
"""Reproducible SCAS/SymPy task timings; includes SCAS print cost, excludes process startup."""

import argparse
import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path

import sympy as sp


ROOT = Path(__file__).resolve().parent.parent
X = sp.Symbol("x")


def median_seconds(operation, repeats):
    samples = []
    for _ in range(5):
        start = time.perf_counter()
        for _ in range(repeats):
            operation()
        samples.append((time.perf_counter() - start) / repeats)
    return statistics.median(samples)


def scas_seconds(binary, expression, repeats, precision=None):
    program = (f"print({expression});\n" * repeats)
    command = [str(binary)] + (["--precision", str(precision)] if precision else [])
    samples = []
    for _ in range(5):
        start = time.perf_counter()
        result = subprocess.run(command, input=program, text=True, capture_output=True, cwd=ROOT)
        elapsed = time.perf_counter() - start
        if result.returncode:
            raise RuntimeError(result.stderr.strip())
        if len(result.stdout.splitlines()) != repeats:
            raise RuntimeError("SCAS produced an unexpected number of results")
        samples.append(elapsed / repeats)
    return statistics.median(samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/scas")
    args = parser.parse_args()
    a = X**8 - 1
    b = X**6 - 1
    c = X**5 + 2*X**3 - X + 1
    d = X**4 - 3*X**2 + 2
    entries = [[(i * 7 + j * 11 + i * j * 3) % 17 - 8 for j in range(8)] for i in range(8)]
    matrix = sp.Matrix(entries)
    matrix_source = "[" + ";".join(",".join(str(value) for value in row) for row in entries) + "]"
    workloads = [
        ("Polynomial GCD", "polynomial_gcd(x^8-1, x^6-1, x)", lambda: sp.gcd(a, b), 100, None),
        ("Resultant", "resultant(x^5+2*x^3-x+1, x^4-3*x^2+2, x)", lambda: sp.resultant(c, d, X), 50, None),
        ("8×8 determinant", f"det({matrix_source})", matrix.det, 20, None),
        ("Gamma, 50 digits", "gamma(evalf(0.5))", lambda: sp.gamma(sp.Float("0.5", 50)), 20, 50),
    ]
    print(f"Python {platform.python_version()}, SymPy {sp.__version__}, {platform.platform()}")
    print(f"SCAS binary: {args.binary}")
    print("| Workload | SCAS median, ms | SymPy median, ms |")
    print("| --- | ---: | ---: |")
    for name, source, reference, repeats, precision in workloads:
        scas_time = scas_seconds(args.binary, source, repeats, precision)
        sympy_time = median_seconds(reference, repeats)
        print(f"| {name} | {scas_time * 1000:.3f} | {sympy_time * 1000:.3f} |")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError) as error:
        sys.exit(str(error))
