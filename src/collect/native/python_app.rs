//! Services-only Python evidence. Imports and client receivers stay local and syntactic.
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Query};

use super::{literal, named_children, text};
use crate::infer_services::{AppFacts, Fact, FactKind};

#[derive(Clone, Copy)]
enum Receiver {
    Router,
    Http,
    Celery,
}

#[derive(Clone)]
enum Role {
    Symbol(String),
    Receiver(Receiver),
    Method(Receiver, String),
}

#[derive(Clone)]
struct Binding {
    package: String,
    role: Role,
    start: usize,
    end: usize,
}

pub(crate) fn app_facts(_file: &str, source: &str) -> Result<AppFacts> {
    let language = tree_sitter_python::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("parsing Python application facts")?;
    let root = tree.root_node();
    let query = Query::new(&language, include_str!("queries/python.scm"))?;
    let parsed = super::parse(source, root, &query);
    let mut out = AppFacts {
        incomplete: !parsed.limitations.is_empty(),
        ..Default::default()
    };
    let mut nodes = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        nodes.push(node);
        pending.extend(named_children(node).into_iter().rev());
    }
    let mut declarations = BTreeMap::<String, usize>::new();
    for node in &nodes {
        for name in bound_names(*node, source) {
            *declarations.entry(name).or_default() += 1;
        }
    }
    let mut bindings = BTreeMap::new();
    let mut imported = BTreeMap::<String, usize>::new();
    for import in &parsed.imports {
        for (_, alias) in &import.names {
            *imported.entry(alias.clone()).or_default() += 1;
        }
    }
    for import in &parsed.imports {
        if !import.kind.is_runtime() || import.dynamic {
            continue;
        }
        out.imports.push(import.module.clone());
        if import.from {
            out.imports.extend(import.names.iter().map(|(name, _)| {
                format!(
                    "{}{}{name}",
                    import.module,
                    if import.module.ends_with('.') {
                        ""
                    } else {
                        "."
                    }
                )
            }));
        }
        if !import.at_module || import.wildcard {
            continue;
        }
        let package = import.module.split('.').next().unwrap_or("");
        if !matches!(
            package,
            "fastapi"
                | "flask"
                | "requests"
                | "httpx"
                | "sqlalchemy"
                | "psycopg"
                | "redis"
                | "celery"
        ) {
            continue;
        }
        for (name, alias) in &import.names {
            if declarations.contains_key(alias) {
                continue;
            }
            let symbol = if import.from {
                format!("{}.{name}", import.module)
            } else if alias == import.module.split('.').next().unwrap_or("") {
                alias.clone()
            } else {
                import.module.clone()
            };
            bindings.insert(
                alias.clone(),
                Binding {
                    package: package.into(),
                    role: Role::Symbol(symbol),
                    start: 0,
                    end: source.len(),
                },
            );
        }
    }
    bindings.retain(|name, _| imported.get(name) == Some(&1));
    if parsed
        .imports
        .iter()
        .any(|import| import.wildcard && import.kind.is_runtime())
    {
        bindings.clear();
    }
    out.imports.sort();
    out.imports.dedup();

    // shortcut: competing declarations suppress a name throughout the file; add lexical shadow analysis if this hides needed evidence.
    for node in &nodes {
        let (name, value, start, end) = match node.kind() {
            "assignment" => {
                let Some(left) = node
                    .child_by_field_name("left")
                    .filter(|n| n.kind() == "identifier")
                else {
                    continue;
                };
                let Some(value) = node.child_by_field_name("right") else {
                    continue;
                };
                let Some(scope) = scope(*node) else {
                    continue;
                };
                (text(left, source), value, node.end_byte(), scope.end_byte())
            }
            "as_pattern" if node.parent().is_some_and(|p| p.kind() == "with_item") => {
                let Some(alias) = node.child_by_field_name("alias") else {
                    continue;
                };
                let Some(name) = identifier(alias) else {
                    continue;
                };
                let Some(value) = named_children(*node).into_iter().find(|n| *n != alias) else {
                    continue;
                };
                let Some(with) = ancestors(*node).find(|n| n.kind() == "with_statement") else {
                    continue;
                };
                (text(name, source), value, node.end_byte(), with.end_byte())
            }
            _ => continue,
        };
        if declarations.get(name) != Some(&1) || bindings.contains_key(name) {
            continue;
        }
        if let Some(mut binding) = resolve(value, source, &bindings, 64) {
            binding.start = start;
            binding.end = end;
            bindings.insert(name.into(), binding);
        }
    }
    for node in nodes.iter().filter(|n| n.kind() == "call") {
        let Some(function) = node.child_by_field_name("function") else {
            continue;
        };
        let Some(binding) = resolve(function, source, &bindings, 64) else {
            continue;
        };
        let args = arguments(*node);
        match &binding.role {
            Role::Symbol(symbol) => {
                let kind = match symbol.as_str() {
                    "sqlalchemy.create_engine"
                    | "sqlalchemy.engine.create_engine"
                    | "psycopg.connect" => Some(FactKind::Database),
                    "redis.Redis" => Some(FactKind::Cache),
                    "celery.Celery" => Some(FactKind::Queue),
                    _ => None,
                };
                if let Some(kind) = kind {
                    push(&mut out, *node, &binding.package, kind, None, None, false);
                }
                if let Some((package, verb)) = symbol.split_once('.')
                    && matches!(package, "requests" | "httpx")
                    && http_method(verb).is_some()
                {
                    let (url, dynamic) = string_value(
                        positional(&args, 0).or_else(|| keyword(&args, source, "url")),
                        source,
                    );
                    push(
                        &mut out,
                        *node,
                        &binding.package,
                        FactKind::Http,
                        url,
                        http_method(verb).map(str::to_owned),
                        dynamic,
                    );
                }
            }
            Role::Method(Receiver::Http, verb) => {
                if let Some(method) = http_method(verb) {
                    let (url, dynamic) = string_value(
                        positional(&args, 0).or_else(|| keyword(&args, source, "url")),
                        source,
                    );
                    push(
                        &mut out,
                        *node,
                        &binding.package,
                        FactKind::Http,
                        url,
                        Some(method.into()),
                        dynamic,
                    );
                }
            }
            Role::Method(Receiver::Celery, method) if method == "send_task" => {
                let (topic, dynamic) = string_value(
                    positional(&args, 0).or_else(|| keyword(&args, source, "name")),
                    source,
                );
                push(
                    &mut out,
                    *node,
                    &binding.package,
                    FactKind::Publish,
                    topic,
                    None,
                    dynamic,
                );
            }
            _ => {}
        }
    }
    for decorator in nodes.iter().filter(|n| n.kind() == "decorator") {
        let Some(expression) = named_children(*decorator).first().copied() else {
            continue;
        };
        let (function, args) = if expression.kind() == "call" {
            let Some(function) = expression.child_by_field_name("function") else {
                continue;
            };
            (function, arguments(expression))
        } else {
            (expression, Vec::new())
        };
        let Some(binding) = resolve(function, source, &bindings, 64) else {
            continue;
        };
        match &binding.role {
            Role::Method(Receiver::Router, verb) => {
                let path = positional(&args, 0).or_else(|| {
                    keyword(
                        &args,
                        source,
                        if binding.package == "flask" {
                            "rule"
                        } else {
                            "path"
                        },
                    )
                });
                let (operation, dynamic) = string_value(path, source);
                let operation = operation.filter(|path| path.starts_with('/'));
                let methods = if verb == "route" && binding.package == "flask" {
                    if args.iter().any(|arg| arg.kind() == "dictionary_splat") {
                        out.incomplete = true;
                        continue;
                    }
                    match keyword(&args, source, "methods") {
                        Some(methods) if matches!(methods.kind(), "list" | "tuple" | "set") => {
                            let values: Option<Vec<_>> = named_children(methods)
                                .into_iter()
                                .map(|method| {
                                    literal(method, source).map(|method| method.to_uppercase())
                                })
                                .collect();
                            let Some(values) = values else {
                                out.incomplete = true;
                                continue;
                            };
                            values
                        }
                        Some(_) => {
                            out.incomplete = true;
                            continue;
                        }
                        None => vec!["GET".into()],
                    }
                } else if let Some(method) = http_method(verb) {
                    vec![method.into()]
                } else {
                    continue;
                };
                for method in methods {
                    push(
                        &mut out,
                        *decorator,
                        &binding.package,
                        FactKind::Route,
                        operation.clone(),
                        Some(method),
                        dynamic,
                    );
                }
            }
            Role::Method(Receiver::Celery, method) if method == "task" => {
                let (topic, dynamic) = string_value(keyword(&args, source, "name"), source);
                push(
                    &mut out,
                    *decorator,
                    &binding.package,
                    FactKind::Subscribe,
                    topic,
                    None,
                    dynamic,
                );
            }
            _ => {}
        }
    }
    Ok(out)
}

fn ancestors(mut node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    std::iter::from_fn(move || {
        let parent = node.parent()?;
        node = parent;
        Some(parent)
    })
}
fn scope(node: Node<'_>) -> Option<Node<'_>> {
    ancestors(node)
        .find(|n| {
            matches!(
                n.kind(),
                "module" | "function_definition" | "lambda" | "class_definition"
            )
        })
        .filter(|n| n.kind() != "class_definition")
}
fn identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        Some(node)
    } else {
        named_children(node)
            .into_iter()
            .find(|n| n.kind() == "identifier")
    }
}
fn bound_names(node: Node<'_>, source: &str) -> Vec<String> {
    let bound = match node.kind() {
        "assignment"
        | "augmented_assignment"
        | "for_statement"
        | "for_in_clause"
        | "type_alias_statement" => node.child_by_field_name("left"),
        "function_definition" | "class_definition" | "named_expression" => {
            node.child_by_field_name("name")
        }
        "default_parameter" | "typed_parameter" | "typed_default_parameter" => node
            .child_by_field_name("name")
            .or_else(|| named_children(node).first().copied()),
        "as_pattern" | "except_clause" => node.child_by_field_name("alias"),
        "identifier"
            if node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "parameters" | "lambda_parameters")) =>
        {
            Some(node)
        }
        "tuple_pattern"
            if node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "parameters" | "lambda_parameters")) =>
        {
            Some(node)
        }
        // shortcut: match and type patterns suppress all identifiers; distinguish captures if this hides useful evidence.
        "case_pattern"
        | "type_parameter"
        | "list_splat_pattern"
        | "dictionary_splat_pattern"
        | "delete_statement" => Some(node),
        _ => None,
    };
    let mut pending: Vec<_> = bound.into_iter().collect();
    let mut names = Vec::new();
    while let Some(node) = pending.pop() {
        match node.kind() {
            "identifier" => names.push(text(node, source).into()),
            "attribute" | "subscript" => {
                let mut object = node;
                while let Some(parent) = object
                    .child_by_field_name("object")
                    .or_else(|| object.child_by_field_name("value"))
                {
                    object = parent;
                }
                if object.kind() == "identifier" {
                    names.push(text(object, source).into());
                }
            }
            _ => pending.extend(named_children(node)),
        }
    }
    names
}
fn resolve(
    node: Node<'_>,
    source: &str,
    bindings: &BTreeMap<String, Binding>,
    remaining: usize,
) -> Option<Binding> {
    // shortcut: receiver syntax beyond 64 nodes stays unknown; use iterative resolution if real applications need deeper expressions.
    let remaining = remaining.checked_sub(1)?;
    match node.kind() {
        "identifier" => bindings
            .get(text(node, source))
            .filter(|binding| binding.start <= node.start_byte() && node.end_byte() <= binding.end)
            .cloned(),
        "attribute" => {
            let mut binding = resolve(
                node.child_by_field_name("object")?,
                source,
                bindings,
                remaining,
            )?;
            let member = text(node.child_by_field_name("attribute")?, source);
            binding.role = match binding.role {
                Role::Symbol(symbol) => Role::Symbol(format!("{symbol}.{member}")),
                Role::Receiver(receiver) => Role::Method(receiver, member.into()),
                _ => return None,
            };
            Some(binding)
        }
        "call" => {
            let mut binding = resolve(
                node.child_by_field_name("function")?,
                source,
                bindings,
                remaining,
            )?;
            let Role::Symbol(symbol) = &binding.role else {
                return None;
            };
            let receiver = match symbol.as_str() {
                "fastapi.FastAPI" | "flask.Flask" => Receiver::Router,
                "httpx.Client" | "httpx.AsyncClient" => Receiver::Http,
                "celery.Celery" => Receiver::Celery,
                _ => return None,
            };
            binding.role = Role::Receiver(receiver);
            Some(binding)
        }
        "await" | "parenthesized_expression" => {
            resolve(*named_children(node).first()?, source, bindings, remaining)
        }
        _ => None,
    }
}
fn arguments(node: Node<'_>) -> Vec<Node<'_>> {
    node.child_by_field_name("arguments")
        .map(named_children)
        .unwrap_or_default()
}
fn positional<'a>(args: &[Node<'a>], index: usize) -> Option<Node<'a>> {
    args.iter()
        .filter(|n| !matches!(n.kind(), "keyword_argument" | "dictionary_splat"))
        .nth(index)
        .copied()
}
fn keyword<'a>(args: &[Node<'a>], source: &str, name: &str) -> Option<Node<'a>> {
    args.iter()
        .find(|n| {
            n.kind() == "keyword_argument"
                && n.child_by_field_name("name")
                    .is_some_and(|n| text(n, source) == name)
        })?
        .child_by_field_name("value")
}
fn string_value(node: Option<Node<'_>>, source: &str) -> (Option<String>, bool) {
    let Some(node) = node else {
        return (None, true);
    };
    if let Some(value) = literal(node, source) {
        return (Some(value), false);
    }
    let children = named_children(node);
    if node.kind() == "string"
        && children
            .iter()
            .filter(|n| n.kind() == "interpolation")
            .count()
            == 1
    {
        let meaningful: Vec<_> = children
            .iter()
            .filter(|n| !matches!(n.kind(), "string_start" | "string_end"))
            .collect();
        if meaningful
            .first()
            .is_some_and(|n| n.kind() == "interpolation")
            && meaningful
                .iter()
                .skip(1)
                .all(|n| n.kind() == "string_content")
        {
            return (
                Some(
                    meaningful
                        .iter()
                        .skip(1)
                        .map(|n| text(**n, source))
                        .collect(),
                ),
                true,
            );
        }
    }
    (None, true)
}
fn http_method(method: &str) -> Option<&'static str> {
    match method {
        "get" => Some("GET"),
        "post" => Some("POST"),
        "put" => Some("PUT"),
        "patch" => Some("PATCH"),
        "delete" => Some("DELETE"),
        "head" => Some("HEAD"),
        "options" => Some("OPTIONS"),
        _ => None,
    }
}
fn push(
    out: &mut AppFacts,
    node: Node<'_>,
    package: &str,
    kind: FactKind,
    operation: Option<String>,
    method: Option<String>,
    dynamic: bool,
) {
    out.facts.push(Fact {
        kind,
        package: package.into(),
        operation,
        method,
        line: node.start_position().row + 1,
        dynamic,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(source: &str) -> AppFacts {
        app_facts("service/main.py", source).unwrap()
    }

    #[test]
    fn python_app_facts_clients_aliases_and_http_receivers() {
        let out = facts(
            r#"
from sqlalchemy import create_engine as engine
import psycopg as pg
from redis import Redis as Cache
from celery import Celery as Tasks
import requests as req
from httpx import post as send
import httpx
engine('postgresql://host/db')
pg.connect('dsn')
Cache()
tasks = Tasks('workers')
req.get('https://external/data')
send(url='http://api/predict')
client = httpx.AsyncClient()
client.post('http://api/predict')
"#,
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Database)
                .count(),
            2
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Cache)
                .count(),
            1
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Queue)
                .count(),
            1
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Http)
                .count(),
            3
        );
        assert!(
            out.facts
                .iter()
                .any(|f| f.method.as_deref() == Some("POST") && f.package == "httpx")
        );
    }

    #[test]
    fn python_app_facts_fastapi_flask_routes_and_celery_topics() {
        let out = facts(
            r#"
from fastapi import FastAPI
from flask import Flask
from celery import Celery
app = FastAPI()
web = Flask(__name__)
tasks = Celery('workers')
@app.post('/predict')
def predict(): pass
@web.route('/invoice', methods=['POST', 'PUT'])
def invoice(): pass
@web.route('/health')
def health(): pass
@tasks.task(name='invoice.created')
def process(): pass
@tasks.task
def unnamed(): pass
tasks.send_task('invoice.created')
"#,
        );
        let routes: Vec<_> = out
            .facts
            .iter()
            .filter(|f| f.kind == FactKind::Route)
            .collect();
        assert_eq!(routes.len(), 4);
        assert!(
            routes
                .iter()
                .any(|f| f.operation.as_deref() == Some("/predict")
                    && f.method.as_deref() == Some("POST"))
        );
        assert!(routes.iter().any(
            |f| f.operation.as_deref() == Some("/health") && f.method.as_deref() == Some("GET")
        ));
        assert!(
            out.facts.iter().any(|f| f.kind == FactKind::Subscribe
                && f.operation.as_deref() == Some("invoice.created"))
        );
        assert!(
            out.facts
                .iter()
                .any(|f| f.kind == FactKind::Subscribe && f.operation.is_none())
        );
        assert!(
            out.facts.iter().any(|f| f.kind == FactKind::Publish
                && f.operation.as_deref() == Some("invoice.created"))
        );
    }

    #[test]
    fn python_app_facts_unknown_flask_methods_never_become_any_routes() {
        let out = facts(
            r#"
from flask import Flask
app = Flask(__name__)
@app.route('/unknown', methods=methods)
def unknown(): pass
@app.route('/mixed', methods=['POST', method])
def mixed(): pass
"#,
        );
        assert!(out.incomplete);
        assert!(!out.facts.iter().any(|f| f.kind == FactKind::Route));
    }

    #[test]
    fn python_app_facts_flask_keyword_splats_do_not_invent_get_methods() {
        let out = facts(
            r#"
from flask import Flask
app = Flask(__name__)
options = {'methods': ['POST']}
@app.route('/unknown', **options)
def unknown(): pass
@app.route('/explicit-with-splat', methods=['GET'], **options)
def explicit_with_splat(): pass
@app.route('/known', methods=['POST'])
def known(): pass
@app.route('/default')
def default(): pass
"#,
        );
        assert!(out.incomplete);
        let routes: Vec<_> = out
            .facts
            .iter()
            .filter(|f| f.kind == FactKind::Route)
            .collect();
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().any(
            |f| f.operation.as_deref() == Some("/known") && f.method.as_deref() == Some("POST")
        ));
        assert!(
            routes
                .iter()
                .any(|f| f.operation.as_deref() == Some("/default")
                    && f.method.as_deref() == Some("GET"))
        );
    }

    #[test]
    fn python_app_facts_scopes_mutations_and_unrelated_receivers_stay_unknown() {
        let out = facts(
            r#"
import requests
import httpx
from fastapi import FastAPI
from redis import Redis
def use(requests): requests.post('http://api/shadowed')
def setup():
    app = FastAPI()
    @app.get('/inside')
    def route(): pass
    client = httpx.Client()
    client.get('http://api/inside')
def outside():
    app.get('/outside')
    client.get('http://api/outside')
def unrelated(app):
    @app.get('/unrelated')
    def route(): pass
Redis = custom
Redis()
"#,
        );
        assert_eq!(out.facts.len(), 1);
        assert_eq!(out.facts[0].operation.as_deref(), Some("http://api/inside"));
        let changed = facts(
            "import httpx\nclient = httpx.Client()\nclient.get = fake\nclient.get('http://api/not-real')",
        );
        assert!(changed.facts.is_empty());
    }

    #[test]
    fn python_app_facts_runtime_imports_and_static_template_tails() {
        let out = facts(
            r#"
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    from redis import Redis
    from hidden.db import engine
from shared.db import cache
from . import models
import requests
requests.post(f'{base}/predict')
requests.get(f'https://{host}/private')
Redis()
"#,
        );
        assert!(out.imports.contains(&"shared.db".into()));
        assert!(out.imports.contains(&".models".into()));
        assert!(
            !out.imports
                .iter()
                .any(|m| m.starts_with("hidden") || m == "redis")
        );
        assert_eq!(out.facts.len(), 2);
        assert!(
            out.facts
                .iter()
                .any(|f| f.operation.as_deref() == Some("/predict") && f.dynamic)
        );
        assert!(out.facts.iter().any(|f| f.operation.is_none()));
    }

    #[test]
    fn python_app_facts_context_managed_http_clients_stay_in_their_with_block() {
        let out = facts(
            r#"
import httpx
with httpx.Client() as client:
    client.post('http://api/x')
client.get('http://api/closed')
"#,
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Http)
                .count(),
            1
        );
    }

    #[test]
    fn python_app_facts_varargs_deletion_and_local_imports_hide_global_bindings() {
        for source in [
            "import requests\ndef call(*requests): requests.get('http://api/hidden')",
            "import requests\ncall = lambda requests: requests.get('http://api/hidden')",
            "import requests\ndef call():\n    import fake as requests\n    requests.get('http://api/hidden')",
            "import requests\ndel requests\nrequests.get('http://api/hidden')",
        ] {
            assert!(facts(source).facts.is_empty(), "{source}");
        }
    }

    #[test]
    fn python_app_facts_comprehension_targets_hide_client_and_package_bindings() {
        for expression in [
            "[client.get('http://api/x') for client in [{}]]",
            "{client.get('http://api/x') for client in [{}]}",
            "{'key': client.get('http://api/x') for client in [{}]}",
            "(client.get('http://api/x') for client in [{}])",
            "[client.get('http://api/x') for _, (client, other) in [('tag', ({}, None))]]",
            "[httpx.get('http://api/x') for httpx in [{}]]",
        ] {
            let source = format!("import httpx\nclient = httpx.Client()\nvalues = {expression}\n");
            assert!(facts(&source).facts.is_empty(), "{expression}");
        }
    }

    #[test]
    fn python_app_facts_match_captures_hide_client_and_package_bindings() {
        for pattern in [
            "client",
            "[client]",
            "{'key': client}",
            "{} as client",
            "Holder(client)",
            "Holder(value=client)",
            "{'outer': [Holder(value=client)]}",
            "Holder(client) | Other(client)",
            "httpx",
        ] {
            let source = format!(
                "import httpx\nclient = httpx.Client()\nmatch value:\n    case {pattern}:\n        client.get('http://api/x')\n        httpx.get('http://api/x')\n"
            );
            let out = facts(&source);
            assert!(
                !out.facts.iter().any(|fact| fact.kind == FactKind::Http
                    && if pattern == "httpx" {
                        fact.line == 6
                    } else {
                        fact.line == 5
                    }),
                "{pattern}"
            );
        }
    }

    #[test]
    fn python_app_facts_remaining_binding_shapes_do_not_restore_unknown_receivers() {
        for body in [
            "try:\n    pass\nexcept Exception as client:\n    client.get('http://api/x')",
            "with other() as client:\n    client.get('http://api/x')",
            "for client in [{}]:\n    client.get('http://api/x')",
            "(client := {}).get('http://api/x')",
            "type client = dict\nclient.get('http://api/x')",
            "def call[client]():\n    client.get('http://api/x')",
            "from other import *\nclient.get('http://api/x')",
        ] {
            let source = format!("import httpx\nclient = httpx.Client()\n{body}\n");
            assert!(facts(&source).facts.is_empty(), "{body}");
        }
    }
}
