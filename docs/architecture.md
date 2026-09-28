# Architecture

The scanner interns lexemes and emits spanned tokens. The recursive-descent
parser gives every statement and expression node a source span with a
Unicode-aware line/column and a half-open UTF-8 byte range. The interpreter
walks that AST against linked lexical environments and writes through an
injected `Write` target.
Runtime flow (`return`, loop control, and errors) uses one internal propagation
type.

Numeric values are exact integers/rationals or arbitrary-precision decimals.
Symbolic expressions are immutable `Rc` trees. Local canonicalization handles
cheap identities; the sparse rational `Polynomial` layer implements expansion,
collection, GCD cancellation, factoring, roots, partial fractions, and Horner
form. `simplify`, `expand`, and `factor` are deliberately distinct operations.

`execute_with_options` is the reusable front door. The binary supplies file,
stdin, and terminal behavior; the REPL keeps a stable symbol table and
interpreter between submissions.
