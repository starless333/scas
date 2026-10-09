use scas::intelligence::SourceIndex;
use scas::interpreter::value::Builtin;

pub const MAX_MATCHES: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insertion {
    Template(&'static str),
    Builtin(&'static str),
}

impl Insertion {
    pub fn text(self) -> String {
        match self {
            Self::Template(template) => template.to_owned(),
            Self::Builtin(name) => format!("{name}($0)"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assist {
    pub label: &'static str,
    pub detail: &'static str,
    pub insertion: Insertion,
}

#[derive(Clone, Copy)]
pub struct Matches {
    pub start_char: usize,
    pub items: [Option<Assist>; MAX_MATCHES],
    pub len: usize,
}

impl Matches {
    pub fn empty(cursor_char: usize) -> Self {
        Self {
            start_char: cursor_char,
            items: [None; MAX_MATCHES],
            len: 0,
        }
    }

    fn push(&mut self, assist: Assist) {
        if self.len < MAX_MATCHES {
            self.items[self.len] = Some(assist);
            self.len += 1;
        }
    }

    pub fn first(self) -> Option<Assist> {
        self.items[0]
    }

    pub fn iter(&self) -> impl Iterator<Item = Assist> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }
}

pub const SNIPPETS: &[Assist] = &[
    Assist {
        label: "Document block",
        detail: "multi-line documented calculation",
        insertion: Insertion::Template("\\{\n    $0\n\\}\n"),
    },
    Assist {
        label: "Display result",
        detail: "render one dynamic result",
        insertion: Insertion::Template("\\[ $0 => \\]"),
    },
    Assist {
        label: "Function",
        detail: "block function",
        insertion: Insertion::Template("fn name(args) {\n    $0\n}"),
    },
    Assist {
        label: "If / else",
        detail: "conditional branches",
        insertion: Insertion::Template("if condition {\n    $0\n} else {\n    \n}"),
    },
    Assist {
        label: "For loop",
        detail: "counted loop",
        insertion: Insertion::Template("for let i = 0; i < n; i += 1 {\n    $0\n}"),
    },
    Assist {
        label: "Matrix",
        detail: "2 × 2 matrix",
        insertion: Insertion::Template("[$0, 0; 0, 1]"),
    },
    Assist {
        label: "Document RREF",
        detail: "show every row operation",
        insertion: Insertion::Template("rref($0) =>;"),
    },
    Assist {
        label: "Solve system",
        detail: "step-by-step linear solve",
        insertion: Insertion::Template("solve_linear_steps($0, b)"),
    },
];

const CURATED: &[Assist] = &[
    Assist {
        label: "let",
        detail: "variable declaration",
        insertion: Insertion::Template("let name = $0;"),
    },
    Assist {
        label: "fn",
        detail: "function declaration",
        insertion: Insertion::Template("fn name(args) = $0;"),
    },
    Assist {
        label: "print",
        detail: "print a value",
        insertion: Insertion::Template("print($0);"),
    },
    Assist {
        label: "solve",
        detail: "solve(expression, variable)",
        insertion: Insertion::Template("solve($0, x)"),
    },
    Assist {
        label: "derivative",
        detail: "derivative(expression, variable)",
        insertion: Insertion::Template("derivative($0, x)"),
    },
    Assist {
        label: "integral",
        detail: "integral(expression, variable)",
        insertion: Insertion::Template("integral($0, x)"),
    },
    Assist {
        label: "definite_integral",
        detail: "definite_integral(expression, variable, lower, upper)",
        insertion: Insertion::Template("definite_integral($0, x, 0, 1)"),
    },
    Assist {
        label: "rref",
        detail: "reduced row echelon form",
        insertion: Insertion::Template("rref($0)"),
    },
    Assist {
        label: "inverse",
        detail: "matrix inverse",
        insertion: Insertion::Template("inverse($0)"),
    },
    Assist {
        label: "gradient",
        detail: "gradient(expression, variables)",
        insertion: Insertion::Template("gradient($0, [x, y])"),
    },
    Assist {
        label: "jacobian",
        detail: "Jacobian matrix",
        insertion: Insertion::Template("jacobian($0, [x, y])"),
    },
    Assist {
        label: "hessian",
        detail: "Hessian matrix",
        insertion: Insertion::Template("hessian($0, [x, y])"),
    },
    Assist {
        label: "resultant",
        detail: "resultant(left, right, variable)",
        insertion: Insertion::Template("resultant($0, g, x)"),
    },
    Assist {
        label: "discriminant",
        detail: "polynomial discriminant",
        insertion: Insertion::Template("discriminant($0, x)"),
    },
    Assist {
        label: "coefficients",
        detail: "coefficients in ascending degree",
        insertion: Insertion::Template("coefficients($0, x)"),
    },
    Assist {
        label: "degree",
        detail: "polynomial degree",
        insertion: Insertion::Template("degree($0, x)"),
    },
    Assist {
        label: "polynomial_gcd",
        detail: "monic polynomial GCD",
        insertion: Insertion::Template("polynomial_gcd($0, g, x)"),
    },
    Assist {
        label: "polynomial_divrem",
        detail: "[quotient, remainder]",
        insertion: Insertion::Template("polynomial_divrem($0, divisor, x)"),
    },
];

pub fn completions(source: &str, cursor_char: usize) -> Matches {
    let (cursor_byte, cursor_char) = char_position(source, cursor_char);
    let mut start_byte = cursor_byte;
    while start_byte > 0 {
        let previous = source[..start_byte].char_indices().next_back();
        let Some((index, character)) = previous else {
            break;
        };
        if character.is_alphanumeric() || character == '_' {
            start_byte = index;
        } else {
            break;
        }
    }
    let prefix = &source[start_byte..cursor_byte];
    if prefix.chars().count() < 2 {
        return Matches::empty(cursor_char);
    }
    let start_char = cursor_char - prefix.chars().count();
    let mut matches = Matches::empty(start_char);
    for assist in CURATED {
        if assist.label.starts_with(prefix) {
            matches.push(*assist);
        }
    }
    for &(name, _) in Builtin::ALL {
        if matches.len == MAX_MATCHES {
            break;
        }
        if name.starts_with(prefix) && !CURATED.iter().any(|assist| assist.label == name) {
            matches.push(Assist {
                label: name,
                detail: "built-in function",
                insertion: Insertion::Builtin(name),
            });
        }
    }
    matches
}

pub fn user_function_completions(
    source: &str,
    cursor_char: usize,
) -> (usize, Vec<(String, String)>) {
    let (cursor_byte, cursor_char) = char_position(source, cursor_char);
    let mut start_byte = cursor_byte;
    while let Some((index, character)) = source[..start_byte].char_indices().next_back() {
        if character.is_alphanumeric() || character == '_' {
            start_byte = index;
        } else {
            break;
        }
    }
    let prefix = &source[start_byte..cursor_byte];
    let start_char = cursor_char.saturating_sub(prefix.chars().count());
    if prefix.chars().count() < 2 {
        return (start_char, Vec::new());
    }
    let mut functions = Vec::new();
    for function in SourceIndex::new(source).user_functions() {
        if function.name.starts_with(prefix)
            && !functions
                .iter()
                .any(|(name, _): &(String, String)| name == &function.name)
        {
            functions.push((
                function.name.clone(),
                format!("fn {}({})", function.name, function.parameters.join(", ")),
            ));
        }
    }
    functions.truncate(MAX_MATCHES);
    (start_char, functions)
}

fn char_position(source: &str, requested: usize) -> (usize, usize) {
    let mut count = 0;
    for (byte, _) in source.char_indices() {
        if count == requested {
            return (byte, count);
        }
        count += 1;
    }
    (source.len(), count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_is_bounded_and_finds_curated_and_builtin_entries() {
        let solve = completions("sol", 3);
        assert_eq!(solve.first().unwrap().label, "solve");
        assert!(solve.len <= MAX_MATCHES);

        let trig = completions("asi", 3);
        assert_eq!(trig.first().unwrap().label, "asin");
    }

    #[test]
    fn completion_replaces_only_the_identifier_at_the_cursor() {
        let matches = completions("let y = sol", 11);
        assert_eq!(matches.start_char, 8);
        assert_eq!(matches.first().unwrap().label, "solve");
    }

    #[test]
    fn user_function_completion_includes_signature() {
        let source = "fn blend(left, right) = left + right; ble";
        let (start, matches) = user_function_completions(source, source.chars().count());
        assert_eq!(&source[char_position(source, start).0..], "ble");
        assert_eq!(
            matches,
            [("blend".to_owned(), "fn blend(left, right)".to_owned())]
        );
    }
}
