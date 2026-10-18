//! Import syntax only: dependency-cruiser remains responsible for file resolution.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};

use crate::model::EdgeKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedName {
    pub imported: String,
    pub local: String,
    pub type_only: bool,
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
    pub require: bool,
}

#[derive(Debug)]
struct Token {
    text: String,
    literal: Option<String>,
    line: usize,
}

/// What the passes over one source need, from a single lex.
#[derive(Debug)]
pub struct Analysis {
    pub statements: Vec<ImportStatement>,
    pub exports: ModuleExports,
    pub type_exports: BTreeSet<String>,
    type_only_names: BTreeSet<String>,
}

pub fn analyze(source: &str) -> Analysis {
    let tokens = lex(source);
    let statements = statements(&tokens);
    Analysis {
        exports: module_exports(&tokens, &statements),
        type_exports: type_only_exports(&tokens, &statements),
        type_only_names: type_only_names(&tokens),
        statements,
    }
}

impl Analysis {
    /// The `names` used only in erased type syntax outside import statements.
    pub fn type_only_uses(&self, names: &[String]) -> BTreeSet<String> {
        names
            .iter()
            .filter(|name| self.type_only_names.contains(*name))
            .cloned()
            .collect()
    }
}

/// Sources inside one root, each read and analyzed once, by root-relative path.
pub struct Sources {
    pub(super) root: PathBuf,
    analyzed: BTreeMap<String, Option<Rc<Analysis>>>,
}

impl Sources {
    pub fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.canonicalize().context("resolving TypeScript root")?,
            analyzed: BTreeMap::new(),
        })
    }

    /// `None` for a file outside the root or unreadable.
    pub fn get(&mut self, file: &str) -> Option<Rc<Analysis>> {
        if let Some(analysis) = self.analyzed.get(file) {
            return analysis.clone();
        }
        let analysis = self
            .root
            .join(file)
            .canonicalize()
            .ok()
            .filter(|path| path.starts_with(&self.root))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|source| Rc::new(analyze(&source)));
        self.analyzed.insert(file.to_owned(), analysis.clone());
        analysis
    }
}

/// Collects literal and computed imports without interpreting JavaScript expressions.
/// shortcut: require shadowing needs semantic metadata; validate these candidates with dependency-cruiser.
fn statements(tokens: &[Token]) -> Vec<ImportStatement> {
    let mut result = Vec::new();
    let mut functions = Vec::new();
    let mut parens = Vec::new();
    let mut closed_paren = None;
    let mut expression_arrows = Vec::new();
    let mut brackets = 0usize;
    let parameter_opens = function_parameters(tokens);
    let shadowed_requires = parameter_require_scopes(tokens, &parameter_opens);
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
            if text == "require"
                && shadowed_requires
                    .iter()
                    .any(|&(open, close)| i > open && i < close)
            {
                continue;
            }
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
                require: text == "require",
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
                require: false,
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
                type_only: type_prefix,
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
                    type_only: type_prefix || per_type,
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
            require: false,
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
                    expressions.push(Token {
                        text: "(".into(),
                        literal: None,
                        line,
                    });
                    expressions.extend(nested);
                    expressions.push(Token {
                        text: ")".into(),
                        literal: None,
                        line: end_line,
                    });
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

#[derive(Debug, Default)]
pub struct ModuleExports {
    pub pure: bool,
    pub declared: Vec<String>,
    pub local: Vec<ImportedName>,
}

/// Recognizes forwarding-only modules; any executable declaration keeps load-time edges intact.
fn module_exports(tokens: &[Token], statements: &[ImportStatement]) -> ModuleExports {
    let mut result = ModuleExports {
        pure: true,
        ..ModuleExports::default()
    };
    let mut forwarding = false;
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].text == ";" {
            i += 1;
            continue;
        }
        if tokens[i].text == "import" && tokens.get(i + 1).is_some_and(|t| t.text != "(") {
            if tokens.get(i + 1).is_some_and(|t| t.literal.is_some()) {
                result.pure = false;
            }
            let mut end = i + 1;
            while end < tokens.len()
                && tokens[end].literal.is_none()
                && !matches!(tokens[end].text.as_str(), ";" | "import" | "export")
            {
                end += 1;
            }
            if end < tokens.len() && tokens[end].literal.is_some() {
                i = end + 1;
                continue;
            }
        }
        if tokens[i].text == "export" {
            let start = i
                + 1
                + usize::from(
                    tokens.get(i + 1).is_some_and(|t| t.text == "type")
                        && tokens
                            .get(i + 2)
                            .is_some_and(|t| matches!(t.text.as_str(), "{" | "*")),
                );
            if tokens
                .get(start)
                .is_some_and(|t| matches!(t.text.as_str(), "{" | "*"))
            {
                let mut end = start;
                while end < tokens.len()
                    && !matches!(tokens[end].text.as_str(), ";" | "import" | "export")
                {
                    if tokens[end].text == "from"
                        && tokens.get(end + 1).is_some_and(|t| t.literal.is_some())
                    {
                        forwarding = true;
                        i = end + 2;
                        break;
                    }
                    if tokens[end].text == "}" {
                        if tokens.get(end + 1).is_some_and(|t| t.text == "from") {
                            end += 1;
                            continue;
                        }
                        let mut j = start + 1;
                        while j < end {
                            if tokens[j].text == "," {
                                j += 1;
                                continue;
                            }
                            if tokens[j].text == "type"
                                && tokens
                                    .get(j + 1)
                                    .is_some_and(|t| !matches!(t.text.as_str(), "as" | "," | "}"))
                            {
                                j += 1;
                            }
                            let imported = tokens[j].text.clone();
                            let mut local = imported.clone();
                            j += 1;
                            if tokens.get(j).is_some_and(|t| t.text == "as") && j + 1 < end {
                                local = tokens[j + 1].text.clone();
                                j += 2;
                            }
                            result.local.push(ImportedName {
                                imported,
                                local,
                                type_only: false,
                            });
                        }
                        forwarding = true;
                        i = end + 1;
                        break;
                    }
                    end += 1;
                }
                if i > start {
                    continue;
                }
            } else {
                let mut declaration_index = start;
                while tokens
                    .get(declaration_index)
                    .is_some_and(|t| matches!(t.text.as_str(), "async" | "abstract" | "declare"))
                {
                    declaration_index += 1;
                }
                let declaration = tokens.get(declaration_index).map(|t| t.text.as_str());
                if declaration == Some("default") {
                    result.declared.push("default".into());
                } else if matches!(declaration, Some("const" | "let" | "var")) {
                    result
                        .declared
                        .extend(variable_bindings(tokens, declaration_index + 1));
                } else if matches!(
                    declaration,
                    Some("function" | "class" | "interface" | "type" | "enum")
                ) && let Some(name) = tokens.get(declaration_index + 1)
                {
                    result.declared.push(name.text.clone());
                }
            }
        }
        result.pure = false;
        i += 1;
    }
    result.pure &= forwarding
        && statements.iter().filter(|s| !s.reexport).all(|statement| {
            !statement.side_effect
                && !statement.names.is_empty()
                && statement.names.iter().all(|name| {
                    result
                        .local
                        .iter()
                        .any(|export| export.imported == name.local)
                })
        });
    result.declared.sort();
    result.declared.dedup();
    result
}

fn identifier(token: &Token) -> bool {
    token.literal.is_none()
        && token
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || matches!(c, '_' | '$'))
}

fn variable_bindings(tokens: &[Token], mut i: usize) -> Vec<String> {
    let mut names = Vec::new();
    while i < tokens.len() {
        let mut declarator = Vec::new();
        if !binding_pattern(tokens, &mut i, &mut declarator, 0) {
            break;
        }
        if tokens.get(i).is_some_and(|t| t.text == ":") {
            i += 1;
            skip_initializer(tokens, &mut i, &["=", ",", ";"], true);
        }
        if tokens.get(i).is_some_and(|t| t.text == "=") {
            i += 1;
            skip_initializer(tokens, &mut i, &[",", ";"], false);
        } else if tokens.get(i).is_some_and(|t| {
            !matches!(t.text.as_str(), "," | ";") && !matches!(t.text.as_str(), "export" | "import")
        }) {
            break;
        }
        names.extend(declarator);
        if tokens.get(i).is_none_or(|t| t.text != ",") {
            break;
        }
        i += 1;
    }
    names
}

fn binding_pattern(tokens: &[Token], i: &mut usize, names: &mut Vec<String>, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    let Some(token) = tokens.get(*i) else {
        return false;
    };
    if identifier(token) {
        names.push(token.text.clone());
        *i += 1;
        return true;
    }
    let object = token.text == "{";
    if !object && token.text != "[" {
        return false;
    }
    let close = if object { "}" } else { "]" };
    *i += 1;
    while let Some(token) = tokens.get(*i) {
        if token.text == close {
            *i += 1;
            return true;
        }
        if token.text == "," {
            *i += 1;
            continue;
        }
        if token.text == "." {
            while tokens.get(*i).is_some_and(|t| t.text == ".") {
                *i += 1;
            }
        }
        if object && tokens.get(*i).is_some_and(|t| t.text == "[") {
            *i += 1;
            skip_initializer(tokens, i, &["]"], false);
            if tokens.get(*i).is_none_or(|t| t.text != "]") {
                return false;
            }
            *i += 1;
            if tokens.get(*i).is_none_or(|t| t.text != ":") {
                return false;
            }
            *i += 1;
            if !binding_pattern(tokens, i, names, depth + 1) {
                return false;
            }
        } else if object {
            let Some(key) = tokens.get(*i) else {
                return false;
            };
            let shorthand = identifier(key).then(|| key.text.clone());
            *i += 1;
            if tokens.get(*i).is_some_and(|t| t.text == ":") {
                *i += 1;
                if !binding_pattern(tokens, i, names, depth + 1) {
                    return false;
                }
            } else if let Some(name) = shorthand {
                names.push(name);
            } else {
                return false;
            }
        } else if !binding_pattern(tokens, i, names, depth + 1) {
            return false;
        }
        if tokens.get(*i).is_some_and(|t| t.text == "=") {
            *i += 1;
            skip_initializer(tokens, i, &[",", close], false);
        }
        if tokens
            .get(*i)
            .is_none_or(|t| t.text != "," && t.text != close)
        {
            return false;
        }
    }
    false
}

fn skip_initializer(tokens: &[Token], i: &mut usize, stops: &[&str], type_annotation: bool) {
    let mut stack = Vec::new();
    while let Some(token) = tokens.get(*i) {
        let text = token.text.as_str();
        if stack.is_empty()
            && (stops.contains(&text)
                || text == "export"
                || text == "import"
                    && tokens
                        .get(*i + 1)
                        .is_none_or(|t| !matches!(t.text.as_str(), "(" | ".")))
        {
            return;
        }
        match text {
            "(" => stack.push(")"),
            "[" => stack.push("]"),
            "{" => stack.push("}"),
            "<" if type_annotation || generic_arguments(tokens, *i) => stack.push(">"),
            ")" | "]" | "}" | ">" if stack.last() == Some(&text) => {
                stack.pop();
            }
            ")" | "]" | "}" if stack.is_empty() => return,
            _ => {}
        }
        *i += 1;
    }
}

fn generic_arguments(tokens: &[Token], start: usize) -> bool {
    let mut depth = 1;
    for i in start + 1..tokens.len() {
        match tokens[i].text.as_str() {
            "<" => depth += 1,
            ">" => {
                depth -= 1;
                if depth == 0 {
                    return tokens.get(i + 1).is_some_and(|t| t.text == "(");
                }
            }
            ";" | "=" => return false,
            _ => {}
        }
    }
    false
}

/// Only names with an erased declaration and no same-name value declaration are proven types.
fn type_only_exports(tokens: &[Token], statements: &[ImportStatement]) -> BTreeSet<String> {
    let mut erased = BTreeSet::new();
    let mut values = BTreeSet::new();
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 && tokens[i - 1].text == "." {
            continue;
        }
        if matches!(token.text.as_str(), "type" | "interface")
            && let Some(name) = tokens.get(i + 1).filter(|name| identifier(name))
        {
            let modifiers: Vec<_> = tokens[..i]
                .iter()
                .rev()
                .take_while(|token| matches!(token.text.as_str(), "default" | "declare" | "export"))
                .collect();
            if modifiers.iter().any(|token| token.text == "export") {
                erased.insert(if modifiers.iter().any(|token| token.text == "default") {
                    "default".into()
                } else {
                    name.text.clone()
                });
            }
        }
        if matches!(
            token.text.as_str(),
            "class" | "function" | "enum" | "namespace" | "module"
        ) && let Some(name) = tokens.get(i + 1).filter(|name| identifier(name))
        {
            values.insert(name.text.clone());
        }
        if matches!(token.text.as_str(), "const" | "let" | "var") {
            values.extend(variable_bindings(tokens, i + 1));
        }
    }
    let forwarded = statements
        .iter()
        .filter(|statement| statement.reexport)
        .flat_map(|statement| statement.names.iter().map(|name| name.local.as_str()))
        .collect::<BTreeSet<_>>();
    erased.retain(|name| {
        !values.contains(name) && !forwarded.contains(name.as_str()) && !forwarded.contains("*")
    });
    erased
}

/// The names whose every occurrence outside import statements is erased type syntax. Unknown
/// expression contexts stay runtime; only recognized erased syntax proves a type use.
fn type_only_names(tokens: &[Token]) -> BTreeSet<String> {
    let mut erased = vec![false; tokens.len()];
    let mut imported = vec![false; tokens.len()];
    for (i, token) in tokens.iter().enumerate() {
        if token.text == "import"
            && tokens
                .get(i + 1)
                .is_some_and(|t| !matches!(t.text.as_str(), "(" | "."))
        {
            let mut end = i + 1;
            while end < tokens.len()
                && tokens[end].literal.is_none()
                && !matches!(tokens[end].text.as_str(), ";" | "import" | "export")
            {
                end += 1;
            }
            imported[i..=end.min(tokens.len().saturating_sub(1))].fill(true);
        }
        if matches!(token.text.as_str(), "type" | "interface")
            && tokens.get(i + 1).is_some_and(identifier)
        {
            if token.text == "interface" {
                if let Some(open) =
                    (i + 2..tokens.len()).find(|&j| matches!(tokens[j].text.as_str(), "{" | ";"))
                    && tokens[open].text == "{"
                    && let Some(close) = closing(tokens, open, "{", "}")
                {
                    erased[i..=close].fill(true);
                }
            } else {
                let mut end = i + 2;
                skip_initializer(tokens, &mut end, &[";"], true);
                erased[i..end].fill(true);
            }
        }
        if matches!(token.text.as_str(), "const" | "let" | "var") {
            let mut start = i + 1;
            loop {
                if !binding_pattern(tokens, &mut start, &mut Vec::new(), 0) {
                    break;
                }
                if tokens.get(start).is_some_and(|t| t.text == ":") {
                    let begin = start;
                    start += 1;
                    skip_initializer(tokens, &mut start, &["=", ",", ";"], true);
                    erased[begin..start].fill(true);
                }
                if tokens.get(start).is_some_and(|t| t.text == "=") {
                    start += 1;
                    skip_initializer(tokens, &mut start, &[",", ";"], false);
                }
                if tokens.get(start).is_none_or(|t| t.text != ",") {
                    break;
                }
                start += 1;
            }
        }
        if matches!(token.text.as_str(), "as" | "satisfies")
            && let Some(next) = tokens.get(i + 1).filter(|t| identifier(t))
        {
            let _ = next;
            erased[i + 1] = true;
        }
    }
    for open in function_parameters(tokens) {
        let Some(close) = closing(tokens, open, "(", ")") else {
            continue;
        };
        let mut nesting = Vec::new();
        let mut annotation = false;
        for j in open + 1..close {
            let text = tokens[j].text.as_str();
            if nesting.is_empty() {
                match text {
                    ":" => annotation = true,
                    "=" | "," => annotation = false,
                    _ => {}
                }
            }
            if annotation {
                erased[j] = true;
            }
            match text {
                "(" => nesting.push(")"),
                "[" => nesting.push("]"),
                "{" => nesting.push("}"),
                _ if nesting.last() == Some(&text) => {
                    nesting.pop();
                }
                _ => {}
            }
        }
        if tokens.get(close + 1).is_some_and(|t| t.text == ":") {
            let mut end = close + 2;
            while end < tokens.len() && !matches!(tokens[end].text.as_str(), "{" | "=>" | ";" | "=")
            {
                erased[end] = true;
                end += 1;
            }
        }
    }
    let mut all_erased: BTreeMap<&str, bool> = BTreeMap::new();
    for (i, token) in tokens.iter().enumerate() {
        if !imported[i] {
            *all_erased.entry(&token.text).or_insert(true) &= erased[i];
        }
    }
    all_erased
        .into_iter()
        .filter(|&(_, erased)| erased)
        .map(|(name, _)| name.to_owned())
        .collect()
}

fn closing(tokens: &[Token], open: usize, begin: &str, end: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        if token.text == begin {
            depth += 1;
        } else if token.text == end {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn parameter_require_scopes(
    tokens: &[Token],
    parameters: &std::collections::BTreeSet<usize>,
) -> Vec<(usize, usize)> {
    let mut scopes = Vec::new();
    for &open in parameters {
        let Some(close) = closing(tokens, open, "(", ")") else {
            continue;
        };
        let mut i = open + 1;
        let mut shadowed = false;
        while i < close {
            let mut names = Vec::new();
            if !binding_pattern(tokens, &mut i, &mut names, 0) {
                break;
            }
            shadowed |= names.iter().any(|name| name == "require");
            if tokens.get(i).is_some_and(|token| token.text == ":") {
                i += 1;
                skip_initializer(tokens, &mut i, &["=", ",", ")"], true);
            }
            if tokens.get(i).is_some_and(|token| token.text == "=") {
                i += 1;
                skip_initializer(tokens, &mut i, &[",", ")"], false);
            }
            if tokens.get(i).is_none_or(|token| token.text != ",") {
                break;
            }
            i += 1;
        }
        if !shadowed {
            continue;
        }
        let mut body = close + 1;
        if tokens.get(body).is_some_and(|token| token.text == "=>") {
            body += 1;
        }
        if tokens.get(body).is_some_and(|token| token.text == "{")
            && let Some(end) = closing(tokens, body, "{", "}")
        {
            scopes.push((open, end));
        }
    }
    scopes
}
