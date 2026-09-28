# Error guide

SCAS diagnostics are categorized as `lexical`, `syntax`, `runtime`, or `limit`
errors and include a source location and snippet. Lexing reports every invalid
character found in one scan; parsing and execution stop at the first error.

Common syntax failures are a missing statement semicolon, an unclosed delimiter,
or comma-style `for` separators. Runtime failures include division by zero,
invalid operand types, bad function arity, and array bounds errors. Limit errors
mean a configured resource budget was reached; adjust the matching CLI flag or
the library `Limits` value only for trusted programs.
