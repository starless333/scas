#!/usr/bin/env python3
"""Seeded, reproducible differential checks against SymPy.

Run after `cargo build --release --bin scas` with `python3 scripts/compare_sympy.py`.
Requires SymPy; SCAS itself has no Python dependency.
"""

import argparse
import random
import subprocess
import sys
from pathlib import Path

import sympy as sp
from sympy.parsing.sympy_parser import convert_xor, parse_expr, standard_transformations


ROOT = Path(__file__).resolve().parent.parent
X = sp.Symbol("x")
TRANSFORMS = standard_transformations + (convert_xor,)


def polynomial(rng, degree):
    coefficients = [rng.randint(-5, 5) for _ in range(degree)]
    coefficients.append(rng.choice([-3, -2, -1, 1, 2, 3]))
    return sp.Poly.from_list(list(reversed(coefficients)), X).as_expr()


def scas_expression(expr):
    return sp.sstr(expr).replace("**", "^")


def cases(seed, per_family):
    rng = random.Random(seed)
    for _ in range(per_family):
        a, b = polynomial(rng, 2), polynomial(rng, 2)
        yield "expand", f"expand(({scas_expression(a)})*({scas_expression(b)}))", sp.expand(a * b)
        yield "derivative", f"derivative({scas_expression(a*b)}, x)", sp.diff(a * b, X)
        common = X - rng.randint(-3, 3)
        p, q = sp.expand(common * a), sp.expand(common * b)
        yield "polynomial_gcd", f"polynomial_gcd({scas_expression(p)}, {scas_expression(q)}, x)", sp.gcd(p, q)
        yield "resultant", f"resultant({scas_expression(a)}, {scas_expression(b)}, x)", sp.resultant(a, b, X)


def run(binary, selected):
    program = "\n".join(f"print({expression});" for _, expression, _ in selected) + "\n"
    process = subprocess.run([str(binary)], input=program, text=True, capture_output=True, cwd=ROOT)
    if process.returncode:
        raise RuntimeError(f"SCAS exited {process.returncode}: {process.stderr.strip()}")
    output = process.stdout.splitlines()
    if len(output) != len(selected):
        raise RuntimeError(f"Expected {len(selected)} results, received {len(output)}: {process.stdout[:500]}")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/scas")
    parser.add_argument("--seed", type=int, default=20261011)
    parser.add_argument("--per-family", type=int, default=25)
    args = parser.parse_args()
    selected = list(cases(args.seed, args.per_family))
    counts = {family: [0, 0] for family in ("expand", "derivative", "polynomial_gcd", "resultant")}
    failures = []
    for (family, expression, expected), output in zip(selected, run(args.binary, selected)):
        counts[family][1] += 1
        try:
            actual = parse_expr(output, local_dict={"x": X}, transformations=TRANSFORMS)
            correct = sp.simplify(actual - expected) == 0
        except (SyntaxError, TypeError, ValueError) as error:
            correct = False
            output += f" (parse error: {error})"
        if correct:
            counts[family][0] += 1
        else:
            failures.append((family, expression, output, str(expected)))
    print(f"seed={args.seed} sympy={sp.__version__} binary={args.binary}")
    for family, (passed, total) in counts.items():
        print(f"{family}: {passed}/{total} ({passed / total:.1%})")
    print(f"total: {sum(v[0] for v in counts.values())}/{len(selected)}")
    for family, expression, actual, expected in failures[:10]:
        print(f"FAIL {family}: {expression}\n  SCAS: {actual}\n  SymPy: {expected}")
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
