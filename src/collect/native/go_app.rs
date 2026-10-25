//! Services-only Go facts. No package loading or scanned code execution.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Query};

use super::super::{named_children, text};
use crate::infer_services::{AppFacts, Fact, FactKind};

const HTTP: &str = "net/http";
const NATS: &str = "github.com/nats-io/nats.go";

#[derive(Clone)]
enum Receiver {
    Client,
    Mux,
    Nats,
    Request(Option<String>, Option<String>),
}

struct Binding<T> {
    value: T,
    start: usize,
    end: usize,
}

impl<T> Binding<T> {
    fn new(value: T, node: Node<'_>) -> Self {
        let mut scope = node;
        while let Some(parent) = scope.parent() {
            scope = parent;
            if matches!(
                scope.kind(),
                "block"
                    | "if_statement"
                    | "for_statement"
                    | "expression_switch_statement"
                    | "type_switch_statement"
                    | "expression_case"
                    | "type_case"
                    | "communication_case"
                    | "default_case"
            ) {
                break;
            }
        }
        Self {
            value,
            start: if scope.kind() == "source_file" {
                scope.start_byte()
            } else {
                node.end_byte()
            },
            end: scope.end_byte(),
        }
    }

    fn at(&self, node: Node<'_>) -> Option<&T> {
        (self.start <= node.start_byte() && node.end_byte() <= self.end).then_some(&self.value)
    }
}

pub(crate) fn app_facts(_file: &str, source: &str) -> Result<AppFacts> {
    let language = tree_sitter_go::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let query = Query::new(&language, include_str!("queries/go.scm"))?;
    let parsed = super::parse(source, &mut parser, &query)?;
    let tree = parser
        .parse(source, None)
        .context("parsing Go application facts")?;
    let mut nodes = Vec::new();
    walk(tree.root_node(), &mut nodes);
    let mut facts = AppFacts {
        incomplete: parsed.errors,
        imports: parsed.imports.iter().map(|i| i.path.clone()).collect(),
        ..Default::default()
    };
    facts.entrypoint = parsed.package_name == "main"
        && nodes.iter().any(|node| {
            node.kind() == "function_declaration"
                && node
                    .child_by_field_name("name")
                    .is_some_and(|n| text(n, source) == "main")
        });

    let mut counts = BTreeMap::<String, usize>::new();
    let mut assigned = BTreeSet::new();
    for node in &nodes {
        for name in bindings(*node, source) {
            *counts.entry(name).or_default() += 1;
        }
        if node.kind() == "unary_expression"
            && node
                .child_by_field_name("operator")
                .is_some_and(|operator| text(operator, source) == "&")
            && let Some(name) = node
                .child_by_field_name("operand")
                .and_then(|operand| assignment_name(operand, source))
        {
            assigned.insert(name);
        }
        if matches!(
            node.kind(),
            "assignment_statement" | "inc_statement" | "dec_statement"
        ) && let Some(left) = node
            .child_by_field_name("left")
            .or_else(|| named_children(*node).first().copied())
        {
            if left.kind() == "expression_list" {
                assigned.extend(
                    named_children(left)
                        .into_iter()
                        .filter_map(|n| assignment_name(n, source)),
                );
            } else if let Some(name) = assignment_name(left, source) {
                assigned.insert(name);
            }
        }
    }
    let packages: BTreeMap<_, _> = parsed
        .imports
        .iter()
        .filter_map(|import| {
            let default = package_alias(&import.path)?;
            let alias = import.alias.as_deref().unwrap_or(default);
            (!matches!(alias, "." | "_")
                && !parsed.shadowed.contains(alias)
                && !parsed.declarations.contains(alias)
                && !counts.contains_key(alias)
                && !assigned.contains(alias))
            .then(|| (alias.to_owned(), import.path.clone()))
        })
        .collect();

    let mut receivers = BTreeMap::new();
    let mut strings = BTreeMap::new();
    for node in &nodes {
        if !matches!(
            node.kind(),
            "short_var_declaration" | "var_spec" | "const_spec"
        ) {
            continue;
        }
        let names = bindings(*node, source);
        let Some(name) = names.first() else {
            continue;
        };
        if counts.get(name) != Some(&1) || assigned.contains(name) {
            continue;
        }
        let Some(value) = binding_value(*node) else {
            continue;
        };
        if names.len() == 1
            && let Some(value) = literal(value, source)
        {
            let binding = Binding::new(value, *node);
            // Package variables can be changed by another source file.
            if binding.start != tree.root_node().start_byte() || node.kind() == "const_spec" {
                strings.insert(name.clone(), binding);
            }
        }
        if let Some(receiver) = receiver_value(value, source, &packages) {
            receivers.insert(name.clone(), Binding::new(receiver, *node));
        }
    }

    for node in nodes.iter().filter(|n| n.kind() == "call_expression") {
        let Some(function) = node.child_by_field_name("function") else {
            continue;
        };
        let Some((operand, method)) = selector(function, source) else {
            continue;
        };
        let args = node
            .child_by_field_name("arguments")
            .map(named_children)
            .unwrap_or_default();
        let package = packages.get(&operand);
        if let Some(package) = package {
            let kind = match (package.as_str(), method.as_str()) {
                ("database/sql", "Open" | "OpenDB")
                | ("github.com/jackc/pgx/v5", "Connect" | "ConnectConfig")
                | (
                    "github.com/jackc/pgx/v5/pgxpool",
                    "New" | "NewWithConfig" | "Connect" | "ConnectConfig",
                ) => Some(FactKind::Database),
                (
                    "github.com/redis/go-redis/v9" | "github.com/go-redis/redis/v8",
                    "NewClient" | "NewClusterClient" | "NewUniversalClient",
                ) => Some(FactKind::Cache),
                (NATS, "Connect") => Some(FactKind::Queue),
                _ => None,
            };
            if let Some(kind) = kind {
                add(&mut facts, *node, kind, package, None, None);
            }
        }
        let receiver = receivers
            .get(&operand)
            .and_then(|binding| binding.at(*node));
        let http_package = package.is_some_and(|p| p == HTTP);
        let client = matches!(receiver, Some(Receiver::Client))
            || default_client(function, source, &packages);
        if http_package || client {
            let verb = match method.as_str() {
                "Get" => Some("GET"),
                "Head" => Some("HEAD"),
                "Post" | "PostForm" => Some("POST"),
                _ => None,
            };
            if let Some(verb) = verb {
                add(
                    &mut facts,
                    *node,
                    FactKind::Http,
                    HTTP,
                    args.first().and_then(|n| literal(*n, source)),
                    Some(verb.into()),
                );
            } else if client && method == "Do" {
                let request = args.first().and_then(|n| {
                    if n.kind() == "identifier" {
                        receivers
                            .get(text(*n, source))
                            .and_then(|binding| binding.at(*node))
                            .cloned()
                    } else {
                        receiver_value(*n, source, &packages)
                    }
                });
                if let Some(Receiver::Request(url, verb)) = request {
                    add(&mut facts, *node, FactKind::Http, HTTP, url, verb);
                }
            }
        }
        if (http_package || matches!(receiver, Some(Receiver::Mux)))
            && matches!(method.as_str(), "Handle" | "HandleFunc")
            && let Some(pattern) = args.first().and_then(|n| literal(*n, source))
        {
            let (verb, path) = route(&pattern);
            if path.starts_with('/') {
                add(
                    &mut facts,
                    *node,
                    FactKind::Route,
                    HTTP,
                    Some(path.into()),
                    verb.map(str::to_owned),
                );
            }
        }
        if matches!(receiver, Some(Receiver::Nats)) {
            let kind = match method.as_str() {
                "Publish" | "PublishRequest" => Some(FactKind::Publish),
                "Subscribe" | "SubscribeSync" | "QueueSubscribe" | "QueueSubscribeSync" => {
                    Some(FactKind::Subscribe)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                add(
                    &mut facts,
                    *node,
                    kind,
                    NATS,
                    args.first()
                        .and_then(|n| known_string(*n, source, &strings)),
                    None,
                );
            }
        }
    }
    Ok(facts)
}

fn walk<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    out.push(node);
    for child in named_children(node) {
        walk(child, out);
    }
}

fn package_alias(package: &str) -> Option<&'static str> {
    match package {
        HTTP => Some("http"),
        "database/sql" => Some("sql"),
        "github.com/jackc/pgx/v5" => Some("pgx"),
        "github.com/jackc/pgx/v5/pgxpool" => Some("pgxpool"),
        "github.com/redis/go-redis/v9" | "github.com/go-redis/redis/v8" => Some("redis"),
        NATS => Some("nats"),
        _ => None,
    }
}

fn bindings(node: Node<'_>, source: &str) -> Vec<String> {
    let target = match node.kind() {
        "short_var_declaration" | "range_clause" | "receive_statement" => {
            node.child_by_field_name("left")
        }
        "type_switch_statement" => node.child_by_field_name("alias"),
        "var_spec"
        | "const_spec"
        | "parameter_declaration"
        | "type_parameter_declaration"
        | "variadic_parameter_declaration"
        | "type_spec"
        | "type_alias"
        | "function_declaration" => {
            let mut cursor = node.walk();
            return node
                .children(&mut cursor)
                .enumerate()
                .filter(|(index, _)| node.field_name_for_child(*index as u32) == Some("name"))
                .map(|(_, n)| text(n, source).to_owned())
                .collect();
        }
        _ => None,
    };
    target
        .map(|target| {
            named_children(target)
                .into_iter()
                .filter(|n| n.kind() == "identifier")
                .map(|n| text(n, source).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn binding_value(node: Node<'_>) -> Option<Node<'_>> {
    let value = node
        .child_by_field_name("right")
        .or_else(|| node.child_by_field_name("value"))?;
    if value.kind() == "expression_list" {
        named_children(value).first().copied()
    } else {
        Some(value)
    }
}

fn assignment_name(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(node, source).to_owned());
    }
    if node.kind() == "parenthesized_expression" {
        return named_children(node)
            .first()
            .and_then(|child| assignment_name(*child, source));
    }
    node.child_by_field_name("operand")
        .and_then(|operand| assignment_name(operand, source))
}

fn selector(node: Node<'_>, source: &str) -> Option<(String, String)> {
    if node.kind() != "selector_expression" {
        return None;
    }
    let operand = node.child_by_field_name("operand")?;
    let field = node.child_by_field_name("field")?;
    Some((
        text(operand, source).to_owned(),
        text(field, source).to_owned(),
    ))
}

fn receiver_value(
    node: Node<'_>,
    source: &str,
    packages: &BTreeMap<String, String>,
) -> Option<Receiver> {
    if matches!(node.kind(), "unary_expression" | "parenthesized_expression") {
        return named_children(node)
            .last()
            .and_then(|n| receiver_value(*n, source, packages));
    }
    if node.kind() == "composite_literal" {
        let ty = node.child_by_field_name("type")?;
        let package = ty.child_by_field_name("package")?;
        let name = ty.child_by_field_name("name")?;
        if packages
            .get(text(package, source))
            .is_some_and(|p| p == HTTP)
            && text(name, source) == "Client"
        {
            return Some(Receiver::Client);
        }
    }
    if node.kind() == "selector_expression" {
        let (operand, member) = selector(node, source)?;
        return (packages.get(&operand).is_some_and(|p| p == HTTP) && member == "DefaultClient")
            .then_some(Receiver::Client);
    }
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    let (operand, member) = selector(function, source)?;
    let package = packages.get(&operand)?;
    match (package.as_str(), member.as_str()) {
        (NATS, "Connect") => Some(Receiver::Nats),
        (HTTP, "NewServeMux") => Some(Receiver::Mux),
        (HTTP, "NewRequest" | "NewRequestWithContext") => {
            let args = node
                .child_by_field_name("arguments")
                .map(named_children)
                .unwrap_or_default();
            let offset = usize::from(member == "NewRequestWithContext");
            Some(Receiver::Request(
                args.get(offset + 1).and_then(|n| literal(*n, source)),
                args.get(offset)
                    .and_then(|n| http_method(*n, source, packages)),
            ))
        }
        _ => None,
    }
}

fn default_client(function: Node<'_>, source: &str, packages: &BTreeMap<String, String>) -> bool {
    function
        .child_by_field_name("operand")
        .is_some_and(|operand| {
            matches!(
                receiver_value(operand, source, packages),
                Some(Receiver::Client)
            )
        })
}

fn literal(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "interpreted_string_literal" => serde_json::from_str(text(node, source)).ok(),
        "raw_string_literal" => Some(text(node, source).trim_matches('`').replace('\r', "")),
        _ => None,
    }
}

fn known_string(
    node: Node<'_>,
    source: &str,
    strings: &BTreeMap<String, Binding<String>>,
) -> Option<String> {
    literal(node, source).or_else(|| {
        (node.kind() == "identifier").then_some(())?;
        strings.get(text(node, source))?.at(node).cloned()
    })
}

fn http_method(
    node: Node<'_>,
    source: &str,
    packages: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(method) = literal(node, source) {
        return Some(method.to_uppercase());
    }
    let (operand, member) = selector(node, source)?;
    (packages.get(&operand).is_some_and(|p| p == HTTP)).then_some(())?;
    match member.as_str() {
        "MethodGet" => Some("GET"),
        "MethodHead" => Some("HEAD"),
        "MethodPost" => Some("POST"),
        "MethodPut" => Some("PUT"),
        "MethodPatch" => Some("PATCH"),
        "MethodDelete" => Some("DELETE"),
        "MethodConnect" => Some("CONNECT"),
        "MethodOptions" => Some("OPTIONS"),
        "MethodTrace" => Some("TRACE"),
        _ => None,
    }
    .map(str::to_owned)
}

fn route(pattern: &str) -> (Option<&str>, &str) {
    match pattern.split_once(' ') {
        Some((method, path))
            if !method.is_empty() && method.bytes().all(|c| c.is_ascii_uppercase()) =>
        {
            (Some(method), path)
        }
        _ => (None, pattern),
    }
}

fn add(
    facts: &mut AppFacts,
    node: Node<'_>,
    kind: FactKind,
    package: &str,
    operation: Option<String>,
    method: Option<String>,
) {
    facts.facts.push(Fact {
        kind,
        package: package.into(),
        operation,
        method,
        line: node.start_position().row + 1,
        dynamic: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(source: &str) -> AppFacts {
        app_facts("main.go", source).unwrap()
    }

    #[test]
    fn clients_use_explicit_imports_and_versioned_default_names() {
        let result = facts(
            r#"package main
import (
  db "database/sql"
  "github.com/jackc/pgx/v5"
  "github.com/jackc/pgx/v5/pgxpool"
  "github.com/redis/go-redis/v9"
  "github.com/nats-io/nats.go"
)
func main() {
 db.Open("postgres", "url")
 pgx.Connect(ctx, "url")
 pgxpool.New(ctx, "url")
 redis.NewClient(nil)
 nc, err := nats.Connect("url")
 nc.Publish("created", nil)
 nc.Subscribe("created", handler)
}
"#,
        );
        assert!(result.entrypoint);
        assert!(!result.incomplete);
        assert_eq!(
            result.facts.iter().map(|f| f.kind).collect::<Vec<_>>(),
            vec![
                FactKind::Database,
                FactKind::Database,
                FactKind::Database,
                FactKind::Cache,
                FactKind::Queue,
                FactKind::Publish,
                FactKind::Subscribe
            ]
        );
        assert_eq!(result.facts[5].operation.as_deref(), Some("created"));
        assert_eq!(result.facts[5].line, 15);
    }

    #[test]
    fn http_routes_and_calls_require_receiver_and_request_dispatch() {
        let result = facts(
            r#"package api
import h "net/http"
func setup() {
 mux := h.NewServeMux()
 mux.HandleFunc("POST /users/{id}", handler)
 h.HandleFunc("/health", handler)
 h.Get("http://api/health")
 client := &h.Client{}
 req, err := h.NewRequest(h.MethodPost, "http://api/users/42", nil)
 unused, err := h.NewRequest("DELETE", "http://api/unused", nil)
 client.Do(req)
 h.DefaultClient.Post("https://external/x", "text/plain", nil)
 other.Get("/not-a-call")
 other.HandleFunc("/not-a-route", handler)
}
"#,
        );
        assert!(!result.entrypoint);
        assert_eq!(result.facts.len(), 5, "{:?}", result.facts);
        assert_eq!(result.facts[0].method.as_deref(), Some("POST"));
        assert_eq!(result.facts[1].method, None);
        assert_eq!(result.facts[3].method.as_deref(), Some("POST"));
        assert_eq!(
            result.facts[3].operation.as_deref(),
            Some("http://api/users/42")
        );
        assert!(
            !result
                .facts
                .iter()
                .any(|f| f.operation.as_deref() == Some("http://api/unused"))
        );
    }

    #[test]
    fn shadowed_and_reassigned_names_do_not_produce_guessed_facts() {
        let result = facts(
            r#"package main
import "net/http"
import . "database/sql"
import _ "github.com/redis/go-redis/v9"
func main(http Fake) { http.Get("http://api/x") }
func work() {
 nc, err := fake.Connect("url")
 nc.Publish("not-a-topic", nil)
}
"#,
        );
        assert!(result.facts.is_empty());
        let result = facts(
            r#"package api
import "net/http"
func work() {
 client := &http.Client{}
 client = fake
 client.Get("http://api/x")
}
"#,
        );
        assert!(result.facts.is_empty());
    }

    #[test]
    fn dispatched_unknown_methods_remain_unknown_and_nats_receivers_cannot_be_shadowed() {
        let result = facts(
            r#"package api
import "net/http"
import "github.com/nats-io/nats.go"
func work(method string) {
 req, err := http.NewRequest(method, "http://api/x", nil)
 http.DefaultClient.Do(req)
 nc, err := nats.Connect("url")
 nc.Publish("yes", nil)
}
func other(nc Fake) { nc.Publish("no", nil) }
"#,
        );
        assert_eq!(result.facts.len(), 2, "{:?}", result.facts);
        assert_eq!(result.facts[0].kind, FactKind::Http);
        assert_eq!(result.facts[0].method, None);
        assert_eq!(result.facts[1].kind, FactKind::Queue);
    }

    #[test]
    fn local_receiver_provenance_does_not_escape_its_scope() {
        let result = facts(
            r#"package api
import "net/http"
import "github.com/nats-io/nats.go"
func initClient() {
 client := &http.Client{}
 req, err := http.NewRequest("GET", "http://api/x", nil)
 nc, err := nats.Connect("url")
}
func other() {
 client.Get("http://api/not-from-that-client")
 http.DefaultClient.Do(req)
 nc.Publish("not-from-that-client", nil)
}
"#,
        );
        assert_eq!(result.facts.len(), 1, "{:?}", result.facts);
        assert_eq!(result.facts[0].kind, FactKind::Queue);
    }

    #[test]
    fn request_mutation_and_control_statement_scopes_do_not_guess_calls() {
        let result = facts(
            r#"package api
import "net/http"
func work() {
 req, err := http.NewRequest("GET", "http://api/original", nil)
 req.Method = "DELETE"
 http.DefaultClient.Do(req)
 if client := &http.Client{}; ready {
   client.Get("http://api/inside")
 }
 client.Get("http://api/outside")
}
"#,
        );
        assert_eq!(result.facts.len(), 1, "{:?}", result.facts);
        assert_eq!(
            result.facts[0].operation.as_deref(),
            Some("http://api/inside")
        );
    }

    #[test]
    fn nats_operations_resolve_immutable_literal_declarations() {
        let result = facts(
            r#"package main
import "github.com/nats-io/nats.go"
func main() {
 nc, err := nats.Connect(nats.DefaultURL)
 subject := "test"
 const subscription = "test"
 var publication string = "created"
 nc.Publish(subject, nil)
 nc.Subscribe(subscription, handler)
 nc.Publish(publication, nil)
}
"#,
        );
        assert_eq!(
            result
                .facts
                .iter()
                .filter(|fact| matches!(fact.kind, FactKind::Publish | FactKind::Subscribe))
                .map(|fact| fact.operation.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("test"), Some("test"), Some("created")]
        );
    }

    #[test]
    fn nats_literal_aliases_require_safe_scope_order_and_no_mutation() {
        for body in [
            "subject := \"test\"; subject = next; nc.Publish(subject, nil)",
            "subject := \"test\"; mutate(&subject); nc.Publish(subject, nil)",
            "subject := \"test\"; mutate(&(subject)); nc.Publish(subject, nil)",
            "subject := \"test\"; pointer := &subject; *pointer = next; nc.Publish(subject, nil)",
            "{ subject := \"test\" }; nc.Publish(subject, nil)",
            "if subject := \"test\"; ready {}; nc.Publish(subject, nil)",
            "switch value { case 1: subject := \"test\"; case 2: nc.Publish(subject, nil) }",
            "select { case <-ready: subject := \"test\"; case <-done: nc.Publish(subject, nil) }",
            "subject := \"label\"; _ = subject; var data any = \"different\"; switch subject := data.(type) { case string: nc.Publish(subject, nil) }",
            "subject := \"label\"; var ch chan string; select { case subject := <-ch: nc.Publish(subject, nil) }",
            "subject := \"label\"; var ch chan string; select { case subject = <-ch: nc.Publish(subject, nil) }",
            "subject := \"label\"; var ch chan string; select { case subject = <-ch: default: nc.Publish(subject, nil) }",
            "switch value { default: subject := \"label\"; case 1: nc.Publish(subject, nil) }",
        ] {
            let result = facts(&format!(
                "package main\nimport \"github.com/nats-io/nats.go\"\nfunc main() {{ nc, err := nats.Connect(nats.DefaultURL); {body} }}"
            ));
            assert_eq!(
                result
                    .facts
                    .iter()
                    .filter(|fact| fact.kind == FactKind::Publish)
                    .map(|fact| fact.operation.as_deref())
                    .collect::<Vec<_>>(),
                vec![None],
                "{body}: {:?}",
                result.facts
            );
        }
        let result = facts(
            r#"package main
import "github.com/nats-io/nats.go"
func publish(subject string) {
 nc, err := nats.Connect(nats.DefaultURL)
 { subject := "shadow"; nc.Publish(subject, nil) }
 nc.Publish(subject, nil)
}
"#,
        );
        assert_eq!(
            result
                .facts
                .iter()
                .filter(|fact| fact.kind == FactKind::Publish)
                .map(|fact| fact.operation.as_deref())
                .collect::<Vec<_>>(),
            vec![None, None]
        );
        let result = facts(
            r#"package main
import "github.com/nats-io/nats.go"
func main() {
 nc, err := nats.Connect(nats.DefaultURL)
 nc.Publish(subject, nil)
 subject := "test"
 nc.Publish(subject, nil)
}
"#,
        );
        assert_eq!(
            result
                .facts
                .iter()
                .filter(|fact| fact.kind == FactKind::Publish)
                .map(|fact| fact.operation.as_deref())
                .collect::<Vec<_>>(),
            vec![None, Some("test")]
        );
        let result = facts(
            r#"package main
import "github.com/nats-io/nats.go"
var subject = "mutable-in-another-file"
func main() {
 nc, err := nats.Connect(nats.DefaultURL)
 nc.Publish(subject, nil)
}
"#,
        );
        assert_eq!(
            result
                .facts
                .iter()
                .filter(|fact| fact.kind == FactKind::Publish)
                .map(|fact| fact.operation.as_deref())
                .collect::<Vec<_>>(),
            vec![None]
        );
    }

    #[test]
    fn a_type_switch_shadow_cannot_create_a_service_flow() {
        let root = crate::collect::TempDir::new("go-shadowed-topic").unwrap();
        let publisher = r#"package main
import "github.com/nats-io/nats.go"
func main() {
 nc, err := nats.Connect(nats.DefaultURL)
 subject := "label"
 _ = subject
 var data any = "different"
 switch subject := data.(type) { case string: nc.Publish(subject, nil) }
}
"#;
        let subscriber = r#"package main
import "github.com/nats-io/nats.go"
func main() {
 nc, err := nats.Connect(nats.DefaultURL)
 nc.Subscribe("label", handler)
}
"#;
        for (service, source) in [("pub", publisher), ("sub", subscriber)] {
            let dir = root.path().join(service);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("main.go"), source).unwrap();
        }
        std::fs::write(
            root.path().join("go.mod"),
            "module example.com/shadow\n\ngo 1.21\n",
        )
        .unwrap();
        let report = crate::infer_services::infer(root.path(), None).unwrap();
        assert_eq!(report.services, ["pub", "sub"]);
        assert!(report.edges.is_empty(), "{:?}", report.edges);
    }
}
