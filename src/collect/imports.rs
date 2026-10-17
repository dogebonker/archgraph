//! Import syntax only: dependency-cruiser remains responsible for file resolution.
use crate::model::EdgeKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedName {
    pub imported: String,
    pub local: String,
}

#[derive(Debug, Clone)]
pub struct ImportStatement {
    pub specifier: Option<String>,
    pub line: usize,
    pub kind: EdgeKind,
    pub names: Vec<ImportedName>,
    pub reexport: bool,
    pub wildcard: bool,
    pub side_effect: bool,
}

#[derive(Debug)]
struct Token {
    text: String,
    literal: Option<String>,
    line: usize,
}

/// Collects literal and computed imports without interpreting JavaScript expressions.
/// shortcut: require shadowing needs semantic metadata; validate these candidates with dependency-cruiser.
pub fn typescript(source: &str) -> Vec<ImportStatement> {
    let tokens = lex(source);
    let mut result = Vec::new();
    let mut functions = Vec::new();
    let mut parens = Vec::new();
    let mut closed_paren = None;
    let mut expression_arrows = Vec::new();
    let mut brackets = 0usize;
    let parameter_opens = function_parameters(&tokens);
    for (i, token) in tokens.iter().enumerate() {
        let text = token.text.as_str();
        let prev = i.checked_sub(1).map(|j| tokens[j].text.as_str());
        // A newline only ends an arrow expression when the following token cannot continue it.
        if i > 0
            && token.line > tokens[i - 1].line
            && expression_arrows.contains(&(parens.len(), functions.len(), brackets))
            && expression_ends(&tokens[i - 1])
            && statement_starts(token)
        {
            expression_arrows.retain(|&depth| depth != (parens.len(), functions.len(), brackets));
        }
        match text {
            "(" => parens.push(i),
            ")" => {
                closed_paren = parens.pop().map(|open| (open, i));
                expression_arrows.retain(|&(depth, _, _)| parens.len() >= depth);
            }
            "[" => brackets += 1,
            "]" => {
                brackets = brackets.saturating_sub(1);
                expression_arrows.retain(|&(_, _, depth)| brackets >= depth);
            }
            "=>" => expression_arrows.push((parens.len(), functions.len(), brackets)),
            "," => expression_arrows
                .retain(|&depth| depth != (parens.len(), functions.len(), brackets)),
            "{" => {
                let method = closed_paren.is_some_and(|(open, close)| {
                    let return_type = tokens.get(close + 1).is_some_and(|t| t.text == ":")
                        && !tokens[close + 1..i]
                            .iter()
                            .any(|t| matches!(t.text.as_str(), ";" | "=" | "{" | "}"));
                    let before = open.checked_sub(1).map(|j| tokens[j].text.as_str());
                    (prev == Some(")") || return_type)
                        && parameter_opens.contains(&open)
                        && !matches!(
                            before,
                            Some("if" | "for" | "while" | "switch" | "catch" | "with")
                        )
                });
                functions.push(
                    !expression_arrows.is_empty()
                        || method
                        || functions.last().copied().unwrap_or(false),
                );
                expression_arrows.clear();
            }
            "}" => {
                functions.pop();
                expression_arrows.clear();
            }
            ";" => expression_arrows.clear(),
            _ => {}
        }
        if matches!(prev, Some("." | "?.")) {
            continue;
        }
        let lazy = functions.last().copied().unwrap_or(false)
            || !expression_arrows.is_empty()
            || parens.iter().any(|open| parameter_opens.contains(open));
        let next = tokens.get(i + 1).map(|t| t.text.as_str());
        if text == "require" && next == Some("(") || text == "import" && next == Some("(") {
            let argument = tokens.get(i + 2);
            let literal = argument.and_then(|t| t.literal.clone()).filter(|_| {
                tokens
                    .get(i + 3)
                    .is_some_and(|t| t.text == ")" || t.text == ",")
            });
            result.push(ImportStatement {
                specifier: literal,
                line: token.line,
                kind: if text == "import" || lazy {
                    EdgeKind::Dynamic
                } else {
                    EdgeKind::Static
                },
                names: Vec::new(),
                reexport: false,
                wildcard: true,
                side_effect: false,
            });
            continue;
        }
        if !matches!(text, "import" | "export") || next == Some(".") || next == Some("(") {
            continue;
        }
        let reexport = text == "export";
        let start = i + 1;
        if !reexport && tokens.get(start).is_some_and(|t| t.literal.is_some()) {
            result.push(ImportStatement {
                specifier: tokens[start].literal.clone(),
                line: token.line,
                kind: EdgeKind::Static,
                names: Vec::new(),
                reexport: false,
                wildcard: false,
                side_effect: true,
            });
            continue;
        }
        let mut end = start;
        while end < tokens.len()
            && !matches!(
                tokens[end].text.as_str(),
                ";" | "from" | "import" | "export" | "="
            )
        {
            // A declaration is not a re-export.
            if reexport
                && matches!(
                    tokens[end].text.as_str(),
                    "function" | "class" | "const" | "let" | "var" | "interface" | "enum"
                )
            {
                break;
            }
            end += 1;
        }
        if tokens.get(end).is_none_or(|t| t.text != "from") {
            continue;
        }
        let Some(specifier) = tokens.get(end + 1).and_then(|t| t.literal.clone()) else {
            continue;
        };
        let clause = &tokens[start..end];
        let type_prefix = clause.first().is_some_and(|t| t.text == "type")
            && clause
                .get(1)
                .is_some_and(|t| t.text != "," && t.text != "from");
        let clause = if type_prefix { &clause[1..] } else { clause };
        let wildcard = clause.iter().any(|t| t.text == "*");
        let mut names = Vec::new();
        let mut all_type = type_prefix;
        if wildcard {
            let local = clause
                .windows(2)
                .find(|p| p[0].text == "as")
                .map(|p| p[1].text.clone())
                .unwrap_or_else(|| "*".into());
            names.push(ImportedName {
                imported: "*".into(),
                local,
            });
        } else {
            let mut j = 0;
            let mut in_braces = false;
            let mut has_value = false;
            while j < clause.len() {
                match clause[j].text.as_str() {
                    "{" => {
                        in_braces = true;
                        j += 1;
                        continue;
                    }
                    "}" | "," => {
                        j += 1;
                        continue;
                    }
                    _ => {}
                }
                let per_type = in_braces
                    && clause[j].text == "type"
                    && clause
                        .get(j + 1)
                        .is_some_and(|t| !matches!(t.text.as_str(), "as" | "," | "}"));
                if per_type {
                    j += 1;
                }
                let imported = clause[j]
                    .literal
                    .clone()
                    .unwrap_or_else(|| clause[j].text.clone());
                let mut local = imported.clone();
                j += 1;
                if clause.get(j).is_some_and(|t| t.text == "as") {
                    j += 1;
                    if let Some(alias) = clause.get(j) {
                        local = alias.literal.clone().unwrap_or_else(|| alias.text.clone());
                        j += 1;
                    }
                }
                names.push(ImportedName {
                    imported: if !in_braces && !reexport {
                        "default".into()
                    } else {
                        imported
                    },
                    local,
                });
                has_value |= !per_type && !type_prefix;
            }
            all_type |= !has_value && !names.is_empty();
        }
        result.push(ImportStatement {
            specifier: Some(specifier),
            line: token.line,
            kind: match (reexport, all_type) {
                (true, true) => EdgeKind::TypeReexport,
                (true, false) => EdgeKind::Reexport,
                (false, true) => EdgeKind::Type,
                (false, false) => EdgeKind::Static,
            },
            names,
            reexport,
            wildcard,
            side_effect: false,
        });
    }
    result
}

fn expression_ends(token: &Token) -> bool {
    matches!(token.text.as_str(), ")" | "]" | "}" | "string" | "regex")
        || (token
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
            && !matches!(
                token.text.as_str(),
                "return"
                    | "throw"
                    | "yield"
                    | "await"
                    | "typeof"
                    | "void"
                    | "delete"
                    | "new"
                    | "instanceof"
                    | "in"
            ))
}

fn statement_starts(token: &Token) -> bool {
    token
        .text
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        && !matches!(
            token.text.as_str(),
            "in" | "instanceof" | "as" | "satisfies"
        )
        || matches!(token.text.as_str(), "!" | "~")
}

/// Locate function parameter parentheses before scanning their default expressions.
fn function_parameters(tokens: &[Token]) -> std::collections::BTreeSet<usize> {
    let mut stack = Vec::new();
    let mut parameters = std::collections::BTreeSet::new();
    let mut object_or_class = Vec::new();
    let mut parameter_context = std::collections::BTreeSet::new();
    for (close, token) in tokens.iter().enumerate() {
        if token.text == "{" {
            let before = close.checked_sub(1).map(|i| tokens[i].text.as_str());
            let class = tokens[..close]
                .iter()
                .rev()
                .take_while(|t| !matches!(t.text.as_str(), "{" | "}" | ";" | "="))
                .any(|t| t.text == "class");
            object_or_class
                .push(class || matches!(before, Some("=" | "(" | ":" | "[" | "," | "return")));
        } else if token.text == "}" {
            object_or_class.pop();
        }
        if token.text == "(" {
            stack.push(close);
            if object_or_class.last().copied().unwrap_or(false) {
                parameter_context.insert(close);
            }
        }
        if token.text != ")" {
            continue;
        }
        let Some(open) = stack.pop() else {
            continue;
        };
        let before = open.checked_sub(1).map(|i| tokens[i].text.as_str());
        if matches!(
            before,
            Some("if" | "for" | "while" | "switch" | "catch" | "with")
        ) {
            continue;
        }
        let mut after = close + 1;
        if tokens.get(after).is_some_and(|t| t.text == ":") {
            after += 1;
            while tokens
                .get(after)
                .is_some_and(|t| !matches!(t.text.as_str(), "=>" | "{" | ";" | "=" | "}"))
            {
                after += 1;
            }
        }
        let arrow = tokens.get(after).is_some_and(|t| t.text == "=>");
        let function = tokens[..open]
            .iter()
            .rev()
            .take_while(|t| !matches!(t.text.as_str(), "(" | ")" | "{" | "}" | ";" | "=" | "=>"))
            .any(|t| t.text == "function");
        let method = tokens.get(after).is_some_and(|t| t.text == "{")
            && parameter_context.contains(&open)
            && open >= 2
            && matches!(
                tokens[open - 2].text.as_str(),
                "{" | "," | ";" | "static" | "get" | "set" | "*" | "}"
            );
        if arrow || function || method {
            parameters.insert(open);
        }
    }
    parameters
}

fn lex(source: &str) -> Vec<Token> {
    lex_tokens(&source.chars().collect::<Vec<_>>(), 1, false).0
}

fn lex_tokens(
    chars: &[char],
    start_line: usize,
    interpolation: bool,
) -> (Vec<Token>, usize, usize) {
    let mut tokens: Vec<Token> = Vec::new();
    let (mut i, mut line) = (0, start_line);
    let mut brace_depth = 0usize;
    let mut control_parens = Vec::new();
    let mut after_control = false;
    while i < chars.len() {
        let c = chars[i];
        if interpolation && c == '}' && brace_depth == 0 {
            return (tokens, i + 1, line);
        }
        if c.is_whitespace() {
            line += usize::from(c == '\n');
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() {
                line += usize::from(chars[i] == '\n');
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let start_line = line;
        if matches!(c, '\'' | '"' | '`') {
            after_control = false;
            i += 1;
            let mut value = String::new();
            let mut computed = false;
            let mut expressions = Vec::new();
            while i < chars.len() && chars[i] != c {
                let ch = chars[i];
                line += usize::from(ch == '\n');
                if c == '`' && ch == '$' && chars.get(i + 1) == Some(&'{') {
                    computed = true;
                    let (nested, consumed, end_line) = lex_tokens(&chars[i + 2..], line, true);
                    expressions.extend(nested);
                    i += 2 + consumed;
                    line = end_line;
                    continue;
                }
                if ch == '\\' && i + 1 < chars.len() {
                    i += 1;
                    match chars[i] {
                        '\n' => line += 1,
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'x' | 'u' => {
                            let count = if chars[i] == 'x' { 2 } else { 4 };
                            let digits: String = chars.iter().skip(i + 1).take(count).collect();
                            if digits.len() == count
                                && let Ok(code) = u32::from_str_radix(&digits, 16)
                                && let Some(decoded) = char::from_u32(code)
                            {
                                value.push(decoded);
                                i += count;
                            } else {
                                computed = true;
                            }
                        }
                        escaped => value.push(escaped),
                    }
                } else {
                    value.push(ch);
                }
                i += 1;
            }
            let terminated = i < chars.len();
            i += usize::from(terminated);
            tokens.push(Token {
                text: "string".into(),
                literal: (terminated && !computed).then_some(value),
                line: start_line,
            });
            tokens.extend(expressions);
            continue;
        }
        // Regex literals occur where an expression can start; division follows a value.
        if c == '/'
            && (after_control
                || tokens.last().is_none_or(|t| {
                    matches!(
                        t.text.as_str(),
                        "=" | "("
                            | ","
                            | ":"
                            | "["
                            | "return"
                            | "throw"
                            | "=>"
                            | "!"
                            | "&&"
                            | "||"
                            | ";"
                            | "{"
                    )
                }))
        {
            after_control = false;
            i += 1;
            let mut class = false;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i = (i + 2).min(chars.len());
                    continue;
                }
                if chars[i] == '[' {
                    class = true;
                }
                if chars[i] == ']' {
                    class = false;
                }
                if chars[i] == '/' && !class {
                    i += 1;
                    break;
                }
                if chars[i] == '\n' {
                    break;
                }
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            tokens.push(Token {
                text: "regex".into(),
                literal: None,
                line: start_line,
            });
            continue;
        }
        if c == '{' {
            brace_depth += 1;
        } else if c == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        }
        if c == '(' {
            control_parens.push(tokens.last().is_some_and(|t| {
                matches!(
                    t.text.as_str(),
                    "if" | "while" | "for" | "with" | "switch" | "catch"
                )
            }));
        }
        after_control = c == ')' && control_parens.pop().unwrap_or(false);
        let start = i;
        if c.is_alphanumeric() || matches!(c, '_' | '$') {
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$')) {
                i += 1;
            }
        } else if matches!(
            (c, chars.get(i + 1)),
            ('=', Some('>')) | ('?', Some('.')) | ('&', Some('&')) | ('|', Some('|'))
        ) {
            i += 2;
        } else {
            i += 1;
        }
        tokens.push(Token {
            text: chars[start..i].iter().collect(),
            literal: None,
            line: start_line,
        });
    }
    (tokens, i, line)
}
