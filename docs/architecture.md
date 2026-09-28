# Architecture

The scanner interns lexemes and emits spanned tokens. The recursive-descent
parser gives every statement and expression node a source span with a
Unicode-aware line/column and a half-open UTF-8 byte range. The interpreter
walks that AST against linked lexical environments and writes through an
injected `Write` target.
Runtime flow (`return`, loop control, and errors) uses one internal propagation
type.

Numeric values are exact integers/rationals, arbitrary-precision decimals, or
arbitrary-precision decimal complex pairs. Exact constants remain symbolic;
`evalf` crosses explicitly into numerical evaluation. Elementary real and
complex functions use the pinned `dashu-float`/`dashu-cmplx` backend. Real
special functions use precision-scaled series, continued fractions,
recurrence/reflection, and asymptotic expansions with guard digits. A separate
transcendental-precision limit bounds their cost.

Symbolic expressions are immutable `Rc` trees. Local canonicalization handles
cheap identities; the sparse rational `Polynomial` layer implements expansion,
collection, assumption-qualified GCD cancellation, factoring, roots, partial
fractions, and Horner form. `simplify`, `expand`, and `factor` are deliberately
distinct operations. Domain predicates are conservative: an unbound variable
is not presumed real or nonzero, so branch-sensitive identities remain
unevaluated until justified.

`execute_with_options` is the reusable front door. The binary supplies file,
stdin, and terminal behavior; the REPL keeps a stable symbol table and
interpreter between submissions.
