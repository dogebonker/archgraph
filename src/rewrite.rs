//! Conservative import edits using byte ranges from the collectors' existing grammars.
use std::{ops::Range, path::Path};

use anyhow::{Context, Result, bail, ensure};
use tree_sitter::{Node, Parser};

use crate::model::Language;

#[derive(Debug, Clone)]
pub struct Specifier {
    pub range: Range<usize>,
    pub statement: Range<usize>,
    pub value: String,
    pub line: usize,
    pub form: String,
    pub binding: Option<String>,
    pub safe: bool,
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub range: Range<usize>,
    pub expected: Vec<u8>,
    pub replacement: Vec<u8>,
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).expect("validated UTF-8")
}

fn grammar(language: Language, file: &str) -> tree_sitter::Language {
    match language {
        Language::Typescript => match Path::new(file).extension().and_then(|s| s.to_str()) {
            Some("js" | "jsx" | "mjs" | "cjs") => tree_sitter_javascript::LANGUAGE.into(),
            Some("tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        },
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::Elixir => tree_sitter_elixir::LANGUAGE.into(),
    }
}

fn capture(
    node: Node<'_>,
    statement: Node<'_>,
    source: &[u8],
    form: &str,
    quoted: bool,
    binding: Option<String>,
    safe: bool,
) -> Specifier {
    let mut range = node.byte_range();
    let raw = text(node, source);
    let literal = !quoted
        || (raw.len() >= 2
            && matches!(raw.as_bytes()[0], b'\'' | b'"' | b'`')
            && raw.as_bytes().first() == raw.as_bytes().last());
    if quoted && literal {
        range.start += 1;
        range.end -= 1;
    }
    let value = std::str::from_utf8(&source[range.clone()])
        .expect("validated UTF-8")
        .to_owned();
    Specifier {
        range,
        statement: statement.byte_range(),
        value: value.clone(),
        line: statement.start_position().row + 1,
        form: form.into(),
        binding,
        safe: safe && literal && !value.contains(['\\', '\n', '\r', '\0']),
    }
}

/// Returns only syntactic import references; unsupported forms are marked unsafe.
/// Syntax errors reject the whole file rather than guessing ranges in broken code.
pub fn specifiers(language: Language, file: &str, source: &[u8]) -> Result<Vec<Specifier>> {
    std::str::from_utf8(source).context("source is not UTF-8; edit the import manually")?;
    let mut parser = Parser::new();
    parser.set_language(&grammar(language, file))?;
    let tree = parser
        .parse(source, None)
        .context("import parsing cancelled; retry")?;
    ensure!(
        !tree.root_node().has_error(),
        "{file}: syntax errors; repair syntax before applying a recipe"
    );
    let mut stack = vec![tree.root_node()];
    let mut nodes = Vec::new();
    while let Some(node) = stack.pop() {
        nodes.push(node);
        stack.extend(children(node).into_iter().rev());
    }
    // shortcut: any lexical require binding disables all require edits; use scope analysis if needed.
    let require_shadowed = nodes.iter().any(|node| {
        node.kind() == "identifier"
            && text(*node, source) == "require"
            && !node.parent().is_some_and(|p| {
                p.kind() == "call_expression" && p.child_by_field_name("function") == Some(*node)
            })
    });
    let mut result = Vec::new();
    for node in nodes {
        match language {
            Language::Typescript => match node.kind() {
                "import_statement" | "export_statement" => {
                    let source_node = node.child_by_field_name("source").or_else(|| {
                        children(node)
                            .into_iter()
                            .find(|n| n.kind() == "import_require_clause")
                            .and_then(|n| n.child_by_field_name("source"))
                    });
                    if let Some(n) = source_node {
                        result.push(capture(
                            n,
                            node,
                            source,
                            if node.kind() == "export_statement" {
                                "reexport"
                            } else {
                                "import"
                            },
                            true,
                            None,
                            true,
                        ));
                    }
                }
                "call_expression" => {
                    let Some(function) = node.child_by_field_name("function") else {
                        continue;
                    };
                    let name = text(function, source);
                    if name != "import" && name != "require" {
                        continue;
                    }
                    let Some(arguments) = node.child_by_field_name("arguments") else {
                        continue;
                    };
                    let args = children(arguments);
                    if let Some(n) = args.first().filter(|n| n.kind() == "string") {
                        result.push(capture(
                            *n,
                            node,
                            source,
                            name,
                            true,
                            None,
                            args.len() == 1 && (name != "require" || !require_shadowed),
                        ));
                    }
                }
                _ => {}
            },
            Language::Python => match node.kind() {
                "import_from_statement" => {
                    if let Some(n) = node.child_by_field_name("module_name") {
                        result.push(capture(n, node, source, "from", false, None, true));
                    }
                }
                "import_statement" => {
                    for n in children(node) {
                        if n.kind() == "aliased_import" {
                            if let (Some(name), Some(alias)) = (
                                n.child_by_field_name("name"),
                                n.child_by_field_name("alias"),
                            ) {
                                result.push(capture(
                                    name,
                                    node,
                                    source,
                                    "import",
                                    false,
                                    Some(text(alias, source).into()),
                                    true,
                                ));
                            }
                        } else if n.kind() == "dotted_name" {
                            result.push(capture(n, node, source, "import", false, None, false));
                        }
                    }
                }
                _ => {}
            },
            Language::Go if node.kind() == "import_spec" => {
                if let Some(n) = node.child_by_field_name("path") {
                    let alias = node
                        .child_by_field_name("name")
                        .map(|n| text(n, source).into());
                    result.push(capture(n, node, source, "import", true, alias, true));
                }
            }
            Language::Rust if node.kind() == "use_declaration" => {
                if let Some(mut n) = node.child_by_field_name("argument") {
                    let mut binding = None;
                    if n.kind() == "use_as_clause" {
                        binding = n
                            .child_by_field_name("alias")
                            .map(|n| text(n, source).into());
                        if let Some(path) = n.child_by_field_name("path") {
                            n = path;
                        }
                    }
                    let raw = text(n, source);
                    let mut preceding = node.prev_named_sibling();
                    while preceding
                        .is_some_and(|n| matches!(n.kind(), "line_comment" | "block_comment"))
                    {
                        preceding = preceding.and_then(|n| n.prev_named_sibling());
                    }
                    let attribute = preceding.is_some_and(|n| n.kind() == "attribute_item");
                    let safe = !attribute && raw.split("::").all(identifier);
                    result.push(capture(n, node, source, "use", false, binding, safe));
                }
            }
            Language::Java if node.kind() == "import_declaration" => {
                if let Some(n) = children(node)
                    .into_iter()
                    .find(|n| matches!(n.kind(), "scoped_identifier" | "identifier"))
                {
                    let raw = text(node, source);
                    result.push(capture(
                        n,
                        node,
                        source,
                        "import",
                        false,
                        None,
                        !raw.contains('*') && !raw.split_whitespace().any(|s| s == "static"),
                    ));
                }
            }
            Language::Elixir if node.kind() == "call" => {
                let Some(target) = node.child_by_field_name("target") else {
                    continue;
                };
                let name = text(target, source);
                if !matches!(name, "alias" | "require" | "import" | "use") {
                    continue;
                }
                let args = children(node)
                    .into_iter()
                    .find(|n| n.kind() == "arguments")
                    .map(children)
                    .unwrap_or_default();
                if let Some(n) = args.first().filter(|n| n.kind() == "alias") {
                    let binding = args.iter().find(|n| n.kind() == "keywords").and_then(|n| {
                        children(*n).into_iter().find_map(|pair| {
                            let key = pair.child_by_field_name("key")?;
                            (text(key, source).trim().trim_end_matches(':') == "as")
                                .then(|| {
                                    pair.child_by_field_name("value")
                                        .map(|v| text(v, source).to_owned())
                                })
                                .flatten()
                        })
                    });
                    result.push(capture(*n, node, source, name, false, binding, true));
                }
            }
            _ => {}
        }
    }
    result.sort_by_key(|s| s.range.start);
    result.dedup_by(|a, b| a.range == b.range);
    Ok(result)
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Applies disjoint, fingerprinted byte edits; everything outside their ranges is copied verbatim.
pub fn replace(source: &[u8], edits: &[Edit]) -> Result<Vec<u8>> {
    let mut edits: Vec<_> = edits.iter().collect();
    edits.sort_by_key(|e| e.range.start);
    let mut end = 0;
    let mut previous_start = None;
    let mut output = Vec::new();
    for edit in edits {
        ensure!(
            previous_start != Some(edit.range.start),
            "overlapping import edits; regenerate the recipe"
        );
        ensure!(
            edit.range.start >= end
                && edit.range.start <= edit.range.end
                && edit.range.end <= source.len(),
            "overlapping or invalid import edits; regenerate the recipe"
        );
        ensure!(
            source[edit.range.clone()] == edit.expected,
            "import source changed; rescan and regenerate the recipe"
        );
        output.extend_from_slice(&source[end..edit.range.start]);
        output.extend_from_slice(&edit.replacement);
        end = edit.range.end;
        previous_start = Some(edit.range.start);
    }
    output.extend_from_slice(&source[end..]);
    Ok(output)
}

pub fn rewrite_specifier(
    language: Language,
    file: &str,
    source: &[u8],
    line: Option<usize>,
    from: &str,
    to: &str,
) -> Result<Vec<u8>> {
    let matches: Vec<_> = specifiers(language, file, source)?
        .into_iter()
        .filter(|s| s.value == from && line.is_none_or(|line| s.line == line))
        .collect();
    ensure!(
        matches.len() == 1,
        "{file}: expected one import of {from:?}, found {}; specify its line or edit manually",
        matches.len()
    );
    let s = &matches[0];
    ensure!(
        s.safe,
        "{file}: import needs binding, escaping or conditional analysis; ask the agent to edit it manually"
    );
    ensure!(
        !to.is_empty() && !to.contains(['\'', '"', '`', '\\', '\n', '\r', '\0']),
        "unsafe replacement specifier; use a plain literal and edit manually if escaping is required"
    );
    match language {
        Language::Typescript | Language::Go => {
            if language == Language::Go && s.binding.is_none() {
                ensure!(
                    from.rsplit('/').next() == to.rsplit('/').next(),
                    "Go package binding may change; add an explicit alias manually before applying"
                );
            }
        }
        Language::Python => ensure!(
            to.trim_start_matches('.').split('.').all(identifier),
            "invalid Python module path; edit manually"
        ),
        Language::Rust => {
            ensure!(
                to.split("::").all(identifier),
                "grouped or invalid Rust path; edit manually"
            );
            ensure!(
                s.binding.is_some() || from.rsplit("::").next() == to.rsplit("::").next(),
                "Rust import binding changes; add an explicit alias manually"
            );
        }
        Language::Java => {
            ensure!(
                to.split('.').all(identifier),
                "invalid Java import; edit manually"
            );
            ensure!(
                from.rsplit('.').next() == to.rsplit('.').next(),
                "Java class binding changes; update references manually"
            );
        }
        Language::Elixir => {
            ensure!(
                to.split('.')
                    .all(|s| identifier(s) && s.chars().next().is_some_and(char::is_uppercase)),
                "invalid Elixir module alias; edit manually"
            );
            ensure!(
                s.form != "alias"
                    || s.binding.is_some()
                    || from.rsplit('.').next() == to.rsplit('.').next(),
                "Elixir alias binding changes; add as: explicitly"
            );
        }
    }
    let edited = replace(
        source,
        &[Edit {
            range: s.range.clone(),
            expected: from.as_bytes().to_vec(),
            replacement: to.as_bytes().to_vec(),
        }],
    )?;
    specifiers(language, file, &edited).context("replacement does not parse; edit manually")?;
    Ok(edited)
}

/// Appends an explicit re-export without changing any existing byte.
pub fn append_reexport(
    language: Language,
    file: &str,
    source: &[u8],
    specifier: &str,
    names: &[String],
) -> Result<Vec<u8>> {
    specifiers(language, file, source)?;
    ensure!(
        !names.is_empty() && names.iter().all(|name| identifier(name)),
        "re-export requires explicit identifier names; ask the agent to choose exports"
    );
    ensure!(
        !specifier.is_empty() && !specifier.contains(['\'', '"', '`', '\\', '\n', '\r', '\0']),
        "unsafe re-export specifier; edit manually"
    );
    let mut names = names.to_vec();
    names.sort();
    names.dedup();
    let line = match language {
        Language::Typescript => format!("export {{ {} }} from '{}';", names.join(", "), specifier),
        Language::Python => {
            ensure!(
                specifier.trim_start_matches('.').split('.').all(identifier),
                "invalid Python re-export module"
            );
            // An existing __all__ can hide the new export; the agent must update that contract.
            ensure!(
                !std::str::from_utf8(source)?.contains("__all__"),
                "Python __all__ needs an explicit update; ask the agent to add the re-export"
            );
            format!(
                "from {specifier} import {}",
                names
                    .iter()
                    .map(|name| format!("{name} as {name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        _ => bail!(
            "this language needs package/module declaration changes; ask the agent to add the re-export"
        ),
    };
    let newline = if source.windows(2).any(|s| s == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = source.to_vec();
    if !source.is_empty() && !source.ends_with(b"\n") {
        output.extend_from_slice(newline.as_bytes());
    }
    output.extend_from_slice(line.as_bytes());
    output.extend_from_slice(newline.as_bytes());
    specifiers(language, file, &output).context("re-export does not parse; edit manually")?;
    Ok(output)
}

/// Whether deleting this complete import has no syntactically referenced local bindings.
/// This is a structural check; removing a side-effect import still needs the recipe's risk note.
pub fn can_remove_import(
    language: Language,
    file: &str,
    source: &[u8],
    span: &Specifier,
) -> Result<bool> {
    let captures = specifiers(language, file, source)?;
    if !span.safe
        || !captures.iter().any(|s| {
            s.range == span.range && s.statement == span.statement && s.value == span.value
        })
    {
        return Ok(false);
    }
    if !matches!(
        language,
        Language::Typescript | Language::Python | Language::Go
    ) {
        return Ok(false);
    }
    let mut parser = Parser::new();
    parser.set_language(&grammar(language, file))?;
    let tree = parser
        .parse(source, None)
        .context("import parsing cancelled; retry")?;
    let mut stack = vec![tree.root_node()];
    let mut nodes = Vec::new();
    while let Some(node) = stack.pop() {
        nodes.push(node);
        stack.extend(children(node));
    }
    let Some(statement) = nodes
        .iter()
        .rev()
        .find(|n| n.byte_range() == span.statement)
        .copied()
    else {
        return Ok(false);
    };
    let mut bindings = Vec::new();
    match language {
        Language::Typescript => {
            if statement.kind() != "import_statement"
                || nodes.iter().any(|n| {
                    n.start_byte() >= statement.start_byte()
                        && n.end_byte() <= statement.end_byte()
                        && matches!(n.kind(), "namespace_import" | "import_require_clause")
                })
            {
                return Ok(false);
            }
            bindings.extend(
                nodes
                    .iter()
                    .filter(|n| {
                        n.kind() == "identifier"
                            && n.start_byte() >= statement.start_byte()
                            && n.end_byte() <= statement.end_byte()
                    })
                    .map(|n| text(*n, source).to_owned()),
            );
        }
        Language::Python => {
            if statement.kind() != "import_from_statement" && statement.kind() != "import_statement"
            {
                return Ok(false);
            }
            if nodes.iter().any(|n| {
                n.kind() == "wildcard_import"
                    && n.start_byte() >= statement.start_byte()
                    && n.end_byte() <= statement.end_byte()
            }) {
                return Ok(false);
            }
            if statement.kind() == "import_statement" {
                // Removing one member of a comma-separated import must not remove the others.
                if children(statement)
                    .iter()
                    .filter(|n| matches!(n.kind(), "aliased_import" | "dotted_name"))
                    .count()
                    != 1
                {
                    return Ok(false);
                }
                let Some(binding) = &span.binding else {
                    return Ok(false);
                };
                bindings.push(binding.clone());
            } else {
                let module = statement
                    .child_by_field_name("module_name")
                    .map(|n| n.byte_range());
                bindings.extend(
                    nodes
                        .iter()
                        .filter(|n| {
                            n.kind() == "identifier"
                                && n.start_byte() >= statement.start_byte()
                                && n.end_byte() <= statement.end_byte()
                                && module.as_ref().is_none_or(|m| {
                                    n.start_byte() < m.start || n.end_byte() > m.end
                                })
                        })
                        .map(|n| text(*n, source).to_owned()),
                );
            }
        }
        Language::Go => {
            if statement.kind() != "import_spec" {
                return Ok(false);
            }
            let binding = span
                .binding
                .as_deref()
                .unwrap_or_else(|| span.value.rsplit('/').next().unwrap_or(""));
            if binding == "." || (!identifier(binding) && binding != "_") {
                return Ok(false);
            }
            if binding != "_" {
                bindings.push(binding.into());
            }
        }
        _ => return Ok(false),
    }
    Ok(!nodes.iter().any(|node| {
        matches!(
            node.kind(),
            "identifier" | "package_identifier" | "type_identifier"
        ) && (node.end_byte() <= statement.start_byte()
            || node.start_byte() >= statement.end_byte())
            && bindings
                .iter()
                .any(|binding| text(*node, source) == binding)
    }))
}

/// Complete syntactic range for a safe cut, including Go's enclosing single-import keyword.
pub fn removal_range(
    language: Language,
    file: &str,
    source: &[u8],
    span: &Specifier,
) -> Result<Option<Range<usize>>> {
    if !can_remove_import(language, file, source, span)? {
        return Ok(None);
    }
    if language != Language::Go {
        return Ok(Some(span.statement.clone()));
    }
    let mut parser = Parser::new();
    parser.set_language(&grammar(language, file))?;
    let tree = parser
        .parse(source, None)
        .context("import parsing cancelled; retry")?;
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(span.statement.start, span.statement.end);
    while let Some(current) = node {
        if current.kind() == "import_declaration" {
            let mut stack = vec![current];
            let mut count = 0;
            while let Some(child) = stack.pop() {
                if child.kind() == "import_spec" {
                    count += 1;
                }
                stack.extend(children(child));
            }
            return Ok(Some(if count == 1 {
                current.byte_range()
            } else {
                span.statement.clone()
            }));
        }
        node = current.parent();
    }
    Ok(None)
}
