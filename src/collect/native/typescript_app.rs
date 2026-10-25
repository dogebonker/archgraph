//! File-local application evidence. Unknown bindings remain unknown.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use super::{bound_names, has_token, inside_function, literal, named_children, names, text};
use crate::infer_services::{AppFacts, Fact, FactKind};

#[derive(Clone, Debug)]
enum Role {
    Import(String),
    Router,
    Kafka,
    Producer,
    Consumer,
    Http,
    NestApp,
}
#[derive(Clone, Debug)]
struct Binding {
    package: String,
    role: Role,
    scope: Option<(usize, usize)>,
}

pub(crate) fn app_facts(
    file: &str,
    source: &str,
    packages: &BTreeMap<String, String>,
) -> Result<AppFacts> {
    if file.ends_with(".d.ts") {
        return Ok(AppFacts::default());
    }
    let language = match file.rsplit('.').next() {
        Some("js" | "jsx" | "mjs" | "cjs") => tree_sitter_javascript::LANGUAGE.into(),
        Some("tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("parsing application facts")?;
    let root = tree.root_node();
    let nodes = descendants(root);
    let declarations = declarations(&nodes, source);
    let mut bindings = BTreeMap::new();
    for node in nodes.iter().filter(|n| n.kind() == "import_statement") {
        let Some(specifier) = node
            .child_by_field_name("source")
            .and_then(|n| literal(n, source))
        else {
            continue;
        };
        let Some(package) = packages.get(&specifier) else {
            continue;
        };
        for name in names(*node, source, has_token(*node, "type"), false) {
            if !name.type_only && !declarations.contains_key(&name.local) {
                bindings.insert(
                    name.local,
                    Binding {
                        package: package.clone(),
                        role: Role::Import(name.imported),
                        scope: None,
                    },
                );
            }
        }
    }
    // shortcut: any competing declaration hides a binding throughout the file; use lexical scopes when this suppresses useful evidence.
    for _ in 0..4 {
        let mut changed = false;
        for node in nodes.iter().filter(|n| n.kind() == "variable_declarator") {
            let Some(name) = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "identifier")
            else {
                continue;
            };
            let name = text(name, source);
            if declarations.get(name) != Some(&1) || bindings.contains_key(name) {
                continue;
            }
            if let Some(mut binding) = node
                .child_by_field_name("value")
                .and_then(|n| resolve(n, source, &bindings))
            {
                binding.scope = Some(binding_range(*node));
                bindings.insert(name.to_owned(), binding);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let called = nodes
        .iter()
        .filter(|n| n.kind() == "call_expression" && !inside_function(**n))
        .filter_map(|n| {
            n.child_by_field_name("function")
                .filter(|n| n.kind() == "identifier")
        })
        .map(|n| text(n, source).to_owned())
        .collect::<BTreeSet<_>>();
    let mut out = AppFacts {
        incomplete: root.has_error(),
        ..Default::default()
    };
    let fetch_bound = bound_names(root, source).contains("fetch");
    for node in &nodes {
        if !matches!(node.kind(), "call_expression" | "new_expression") {
            continue;
        }
        let Some(callee) = node.child_by_field_name(if node.kind() == "new_expression" {
            "constructor"
        } else {
            "function"
        }) else {
            continue;
        };
        let args = arguments(*node);
        let first = args.first().copied().and_then(|n| operation(n, source));
        if callee.kind() == "identifier"
            && text(callee, source) == "fetch"
            && !fetch_bound
            && node.kind() == "call_expression"
        {
            let method = match args.get(1).copied() {
                None => Some("GET".into()),
                Some(options) if options.kind() == "object" => {
                    if named_children(options)
                        .iter()
                        .any(|n| n.kind() == "spread_element")
                    {
                        None
                    } else {
                        property(options, "method", source)
                            .map(|value| literal(value, source).map(|s| s.to_uppercase()))
                            .unwrap_or(Some("GET".into()))
                    }
                }
                Some(_) => None,
            };
            push(
                &mut out,
                FactKind::Http,
                "fetch",
                first.clone(),
                method,
                *node,
            );
        }
        if let Some(binding) = resolve(callee, source, &bindings)
            && let Role::Import(export) = &binding.role
        {
            if binding.package == "axios"
                && callee.kind() == "identifier"
                && node.kind() == "call_expression"
                && let Some(method) = verb(export)
            {
                push(
                    &mut out,
                    FactKind::Http,
                    &binding.package,
                    first.clone(),
                    Some(method),
                    *node,
                );
            }
            if let Some(kind) =
                client_kind(&binding.package, export, node.kind() == "new_expression")
            {
                let topic = (binding.package == "bullmq")
                    .then_some(first.clone())
                    .flatten();
                push(&mut out, kind, &binding.package, topic.clone(), None, *node);
                if binding.package == "bullmq" {
                    push(
                        &mut out,
                        if export == "Worker" {
                            FactKind::Subscribe
                        } else {
                            FactKind::Publish
                        },
                        &binding.package,
                        topic,
                        None,
                        *node,
                    );
                    if export == "Worker" && executed(*node, source, &called) {
                        out.entrypoint = true
                    }
                }
            }
        }
        if callee.kind() != "member_expression" || node.kind() != "call_expression" {
            continue;
        }
        let Some(receiver) = callee
            .child_by_field_name("object")
            .and_then(|n| resolve(n, source, &bindings))
        else {
            continue;
        };
        let Some(member) = callee
            .child_by_field_name("property")
            .map(|n| text(n, source))
        else {
            continue;
        };
        if matches!(receiver.role, Role::Router) && verb(member).is_some() && args.len() >= 2 {
            if first.as_ref().is_some_and(|(s, _)| s.starts_with('/')) {
                push(
                    &mut out,
                    FactKind::Route,
                    &receiver.package,
                    first.clone(),
                    verb(member),
                    *node,
                );
            }
        } else if (matches!(receiver.role, Role::Http)
            || receiver.package == "axios"
                && matches!(&receiver.role, Role::Import(s) if matches!(s.as_str(), "default" | "*")))
            && verb(member).is_some()
        {
            push(
                &mut out,
                FactKind::Http,
                &receiver.package,
                first.clone(),
                verb(member),
                *node,
            );
        } else if matches!(receiver.role, Role::Producer) && member == "send" {
            let topic = args
                .first()
                .and_then(|n| property(*n, "topic", source))
                .and_then(|n| operation(n, source));
            push(
                &mut out,
                FactKind::Publish,
                &receiver.package,
                topic,
                None,
                *node,
            );
        } else if matches!(receiver.role, Role::Consumer) && member == "subscribe" {
            let topic = args
                .first()
                .and_then(|n| property(*n, "topic", source))
                .and_then(|n| operation(n, source));
            push(
                &mut out,
                FactKind::Subscribe,
                &receiver.package,
                topic,
                None,
                *node,
            );
        }
        if matches!(receiver.role, Role::Router | Role::NestApp)
            && member == "listen"
            && executed(*node, source, &called)
        {
            out.entrypoint = true;
        }
        if receiver.package == "@nestjs/core"
            && matches!(&receiver.role, Role::Import(s) if s == "NestFactory")
            && matches!(member, "create" | "createMicroservice")
            && executed(*node, source, &called)
        {
            out.entrypoint = true;
        }
    }
    nest_routes(&nodes, source, &bindings, &mut out);
    Ok(out)
}

fn descendants(root: Node<'_>) -> Vec<Node<'_>> {
    let mut nodes = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        pending.extend(named_children(node).into_iter().rev());
        nodes.push(node);
    }
    nodes
}
fn declarations(nodes: &[Node<'_>], source: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node in nodes {
        let name = match node.kind() {
            "variable_declarator"
            | "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration"
            | "class_declaration"
            | "function_signature"
            | "required_parameter"
            | "optional_parameter"
            | "internal_module"
            | "enum_declaration" => node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("pattern")),
            "arrow_function" | "catch_clause" => node.child_by_field_name("parameter"),
            "assignment_expression" | "augmented_assignment_expression" | "for_in_statement" => {
                node.child_by_field_name("left")
            }
            "update_expression" => node.child_by_field_name("argument"),
            _ => None,
        };
        if node.kind() == "formal_parameters" {
            for parameter in named_children(*node)
                .into_iter()
                .filter(|n| !matches!(n.kind(), "required_parameter" | "optional_parameter"))
            {
                for leaf in descendants(parameter).into_iter().filter(|n| {
                    matches!(
                        n.kind(),
                        "identifier" | "shorthand_property_identifier_pattern"
                    )
                }) {
                    *counts.entry(text(leaf, source).to_owned()).or_default() += 1;
                }
            }
        }
        if let Some(name) = name {
            for leaf in descendants(name).into_iter().filter(|n| {
                matches!(
                    n.kind(),
                    "identifier" | "shorthand_property_identifier_pattern"
                )
            }) {
                *counts.entry(text(leaf, source).to_owned()).or_default() += 1;
            }
        }
    }
    counts
}
fn binding_range(mut node: Node<'_>) -> (usize, usize) {
    let start = node.end_byte();
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "statement_block" | "program" | "for_statement" | "for_in_statement" | "switch_body"
        ) {
            return (start, parent.end_byte());
        }
        node = parent;
    }
    (start, node.end_byte())
}
fn arguments(node: Node<'_>) -> Vec<Node<'_>> {
    node.child_by_field_name("arguments")
        .map(named_children)
        .unwrap_or_default()
}
fn property<'a>(node: Node<'a>, key: &str, source: &str) -> Option<Node<'a>> {
    named_children(node).into_iter().rev().find_map(|pair| {
        if pair.kind() == "shorthand_property_identifier" && text(pair, source) == key {
            return Some(pair);
        }
        if pair.kind() != "pair" {
            return None;
        }
        let name = pair.child_by_field_name("key")?;
        (literal(name, source)
            .as_deref()
            .unwrap_or_else(|| text(name, source))
            == key)
            .then(|| pair.child_by_field_name("value"))
            .flatten()
    })
}

fn operation(node: Node<'_>, source: &str) -> Option<(String, bool)> {
    if let Some(value) = literal(node, source) {
        return Some((value, false));
    }
    if node.kind() != "template_string" {
        return None;
    }
    let raw = text(node, source).strip_prefix('`')?.strip_suffix('`')?;
    if raw.contains('\\') {
        return None;
    }
    if !raw.contains("${") {
        return Some((raw.to_owned(), false));
    }
    if !raw.starts_with("${") {
        return None;
    }
    let substitutions = named_children(node)
        .into_iter()
        .filter(|n| n.kind() == "template_substitution")
        .collect::<Vec<_>>();
    let [substitution] = substitutions.as_slice() else {
        return None;
    };
    if substitution.start_byte() != node.start_byte() + 1 {
        return None;
    }
    Some((
        source[substitution.end_byte()..node.end_byte() - 1].to_owned(),
        true,
    ))
}
fn verb(value: &str) -> Option<String> {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
    )
    .then(|| value.to_ascii_uppercase())
}
fn push(
    out: &mut AppFacts,
    kind: FactKind,
    package: &str,
    operation: Option<(String, bool)>,
    method: Option<String>,
    node: Node<'_>,
) {
    let (operation, dynamic) = match operation {
        Some((s, d)) => (Some(s), d),
        None => (None, false),
    };
    out.facts.push(Fact {
        kind,
        package: package.to_owned(),
        operation,
        method,
        line: node.start_position().row + 1,
        dynamic,
    });
}
fn resolve(node: Node<'_>, source: &str, bindings: &BTreeMap<String, Binding>) -> Option<Binding> {
    // shortcut: receiver chains beyond 64 syntax nodes stay unknown; use iterative resolution if real startup code needs deeper chains.
    resolve_bounded(node, source, bindings, &mut 64)
}
fn resolve_bounded(
    node: Node<'_>,
    source: &str,
    bindings: &BTreeMap<String, Binding>,
    remaining: &mut usize,
) -> Option<Binding> {
    *remaining = remaining.checked_sub(1)?;
    match node.kind() {
        "identifier" => bindings
            .get(text(node, source))
            .filter(|binding| {
                binding
                    .scope
                    .is_none_or(|(start, end)| node.start_byte() >= start && node.end_byte() <= end)
            })
            .cloned(),
        "await_expression" | "parenthesized_expression" | "non_null_expression" => {
            resolve_bounded(*named_children(node).first()?, source, bindings, remaining)
        }
        "member_expression" => {
            let mut binding = resolve_bounded(
                node.child_by_field_name("object")?,
                source,
                bindings,
                remaining,
            )?;
            let member = text(node.child_by_field_name("property")?, source);
            match &binding.role {
                Role::Import(name) if matches!(name.as_str(), "default" | "*") => {
                    binding.role = Role::Import(member.to_owned());
                    Some(binding)
                }
                _ => None,
            }
        }
        "call_expression" | "new_expression" => {
            let callee = node.child_by_field_name(if node.kind() == "new_expression" {
                "constructor"
            } else {
                "function"
            })?;
            if callee.kind() == "member_expression" {
                // Resolve this receiver once. Retrying it after resolving the whole callee doubles work at every chain link.
                let mut receiver = resolve_bounded(
                    callee.child_by_field_name("object")?,
                    source,
                    bindings,
                    remaining,
                )?;
                let member = text(callee.child_by_field_name("property")?, source);
                let role = match (&receiver.role, receiver.package.as_str(), member) {
                    (Role::Kafka, _, "producer") if node.kind() == "call_expression" => {
                        Some(Role::Producer)
                    }
                    (Role::Kafka, _, "consumer") if node.kind() == "call_expression" => {
                        Some(Role::Consumer)
                    }
                    (Role::Import(name), "axios", "create")
                        if matches!(name.as_str(), "default" | "*")
                            && node.kind() == "call_expression" =>
                    {
                        Some(Role::Http)
                    }
                    (Role::Import(name), "@nestjs/core", "create" | "createMicroservice")
                        if name == "NestFactory" && node.kind() == "call_expression" =>
                    {
                        Some(Role::NestApp)
                    }
                    (Role::Import(name), _, _) if matches!(name.as_str(), "default" | "*") => {
                        constructed_role(&receiver.package, member, node.kind())
                    }
                    _ => None,
                }?;
                receiver.role = role;
                return Some(receiver);
            }
            let mut binding = resolve_bounded(callee, source, bindings, remaining)?;
            let Role::Import(name) = &binding.role else {
                return None;
            };
            binding.role = constructed_role(&binding.package, name, node.kind())?;
            Some(binding)
        }
        _ => None,
    }
}
fn constructed_role(package: &str, name: &str, shape: &str) -> Option<Role> {
    match (package, name, shape) {
        ("express", "default" | "Router", "call_expression")
        | ("fastify", "default" | "fastify", "call_expression")
        | ("koa-router" | "@koa/router", "default", "new_expression")
        | ("hono", "Hono", "new_expression") => Some(Role::Router),
        ("kafkajs", "Kafka", "new_expression") => Some(Role::Kafka),
        _ => None,
    }
}

fn client_kind(package: &str, name: &str, construct: bool) -> Option<FactKind> {
    use FactKind::{Cache, Database, Queue};
    match (package, name, construct) {
        ("pg", "Pool" | "Client", true)
        | ("postgres", "default", false)
        | ("mysql2" | "mysql2/promise", "createPool", false)
        | ("mongodb", "MongoClient", true)
        | ("@prisma/client", "PrismaClient", true)
        | ("typeorm", "DataSource", true)
        | ("sequelize", "Sequelize" | "default", true)
        | ("kysely", "Kysely", true)
        | ("knex", "default" | "knex", false) => Some(Database),
        ("ioredis", "Redis" | "default", true) | ("redis", "createClient", false) => Some(Cache),
        ("kafkajs", "Kafka", true)
        | ("bullmq", "Queue" | "Worker", true)
        | ("amqplib", "connect", false)
        | ("@aws-sdk/client-sqs", "SQSClient", true) => Some(Queue),
        _ => None,
    }
}
fn executed(mut node: Node<'_>, source: &str, called: &BTreeSet<String>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "function_declaration" | "function_expression" | "arrow_function" | "method_definition"
        ) {
            let name = parent.child_by_field_name("name").or_else(|| {
                parent
                    .parent()
                    .filter(|n| n.kind() == "variable_declarator")
                    .and_then(|n| n.child_by_field_name("name"))
            });
            return name.is_some_and(|n| called.contains(text(n, source)));
        }
        node = parent;
    }
    true
}
fn nest_routes(
    nodes: &[Node<'_>],
    source: &str,
    bindings: &BTreeMap<String, Binding>,
    out: &mut AppFacts,
) {
    for class in nodes.iter().filter(|n| n.kind() == "class_declaration") {
        let decorators = named_children(*class)
            .into_iter()
            .filter(|n| n.kind() == "decorator")
            .collect::<Vec<_>>();
        let prefix = decorators.into_iter().find_map(|decorator| {
            decorator_call(decorator, source, bindings)
                .and_then(|(name, path)| (name == "Controller").then_some(path))
        });
        let Some(prefix) = prefix else { continue };
        let Some(body) = class.child_by_field_name("body") else {
            continue;
        };
        let mut pending = Vec::new();
        for member in named_children(body) {
            if member.kind() == "decorator" {
                pending.push(member);
                continue;
            }
            if member.kind() != "method_definition" {
                pending.clear();
                continue;
            }
            let mut decorators = std::mem::take(&mut pending);
            decorators.extend(
                named_children(member)
                    .into_iter()
                    .filter(|n| n.kind() == "decorator"),
            );
            for decorator in decorators {
                let Some((method, path)) = decorator_call(decorator, source, bindings) else {
                    continue;
                };
                let Some(method) = verb(&method) else {
                    continue;
                };
                let path = match (&prefix, path) {
                    (Some((prefix, pd)), Some((path, d))) => {
                        let path =
                            format!("/{}/{}", prefix.trim_matches('/'), path.trim_matches('/'));
                        Some((format!("/{}", path.trim_matches('/')), *pd || d))
                    }
                    _ => None,
                };
                push(
                    out,
                    FactKind::Route,
                    "@nestjs/common",
                    path,
                    Some(method),
                    decorator,
                );
            }
        }
    }
}
fn decorator_call(
    node: Node<'_>,
    source: &str,
    bindings: &BTreeMap<String, Binding>,
) -> Option<(String, Option<(String, bool)>)> {
    let call = named_children(node)
        .into_iter()
        .find(|n| n.kind() == "call_expression")?;
    let binding = resolve(call.child_by_field_name("function")?, source, bindings)?;
    let Role::Import(name) = binding.role else {
        return None;
    };
    if binding.package != "@nestjs/common" {
        return None;
    }
    let args = arguments(call);
    let path = if args.is_empty() {
        Some((String::new(), false))
    } else {
        operation(args[0], source)
    };
    Some((name, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts(source: &str) -> AppFacts {
        let packages = [
            "pg",
            "postgres",
            "mysql2",
            "mongodb",
            "@prisma/client",
            "typeorm",
            "sequelize",
            "kysely",
            "knex",
            "ioredis",
            "redis",
            "bullmq",
            "kafkajs",
            "amqplib",
            "@aws-sdk/client-sqs",
            "express",
            "fastify",
            "hono",
            "@koa/router",
            "axios",
            "@nestjs/common",
            "@nestjs/core",
        ]
        .into_iter()
        .map(|p| (p.to_owned(), p.to_owned()))
        .collect();
        app_facts("app.ts", source, &packages).unwrap()
    }
    #[test]
    fn app_facts_client_table_and_aliases() {
        let out = facts(
            "import { Pool as PgPool } from 'pg'; import Redis from 'ioredis'; import { Queue, Worker } from 'bullmq'; new PgPool(); new Redis(); new Queue('emails'); new Worker('emails', work);",
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Database)
                .count(),
            1
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
            2
        );
        assert!(
            out.facts
                .iter()
                .any(|f| f.kind == FactKind::Publish && f.operation.as_deref() == Some("emails"))
        );
        assert!(
            out.facts
                .iter()
                .any(|f| f.kind == FactKind::Subscribe && f.operation.as_deref() == Some("emails"))
        );
        assert!(out.entrypoint);
    }
    #[test]
    fn app_facts_all_supported_client_shapes() {
        let out = facts(
            "import postgres from 'postgres'; import mysql from 'mysql2'; import {MongoClient} from 'mongodb'; import {PrismaClient} from '@prisma/client'; import {DataSource} from 'typeorm'; import {Sequelize} from 'sequelize'; import {Kysely} from 'kysely'; import knex from 'knex'; import * as redis from 'redis'; import * as amqp from 'amqplib'; import {SQSClient} from '@aws-sdk/client-sqs'; postgres(); mysql.createPool({}); new MongoClient(url); new PrismaClient(); new DataSource({}); new Sequelize(url); new Kysely({}); knex({}); redis.createClient(); amqp.connect(url); new SQSClient({});",
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Database)
                .count(),
            8,
            "{:?}",
            out.facts
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
            2
        );
    }
    #[test]
    fn app_facts_reassignments_and_javascript_parameters_hide_receivers() {
        assert!(facts("import express from 'express'; let app=express(); app=unrelated; app.get('/wrong', h);").facts.is_empty());
        let packages = [("pg".into(), "pg".into())].into_iter().collect();
        let out = app_facts(
            "app.js",
            "import {Pool} from 'pg'; function f(Pool){new Pool();}",
            &packages,
        )
        .unwrap();
        assert!(out.facts.is_empty());
        assert!(
            app_facts("x.d.ts", "import {Pool} from 'pg'; new Pool();", &packages)
                .unwrap()
                .facts
                .is_empty()
        );
    }
    #[test]
    fn app_facts_reject_shadowed_type_only_and_unrelated_receivers() {
        let out = facts(
            "import type { Pool } from 'pg'; import Redis from 'ioredis'; import express from 'express'; new Pool(); function f(Redis: unknown) { new Redis(); } const x={ get() {} }; x.get('/wrong', handler); const fetch = other; fetch('/wrong');",
        );
        assert!(out.facts.is_empty(), "{:?}", out.facts);
    }
    #[test]
    fn app_facts_routes_http_and_templates() {
        let out = facts(
            "import express from 'express'; import axios from 'axios'; const app=express(); app.get('/users/:id', handler); axios.post('http://api/invoices'); fetch(`${base}/users/42`, { method: 'POST' }); fetch(url, {method}); app.listen(3000);",
        );
        assert!(out.entrypoint);
        assert!(
            out.facts
                .iter()
                .any(|f| f.kind == FactKind::Route && f.method.as_deref() == Some("GET"))
        );
        assert!(out.facts.iter().any(|f| f.kind == FactKind::Http
            && f.operation.as_deref() == Some("http://api/invoices")
            && f.method.as_deref() == Some("POST")));
        assert!(
            out.facts
                .iter()
                .any(|f| f.dynamic && f.operation.as_deref() == Some("/users/42"))
        );
        assert!(
            out.facts.last().is_some_and(|f| f.method.is_none()),
            "{:?}",
            out.facts
        );
    }
    #[test]
    fn app_facts_local_receiver_bindings_stay_inside_their_scope() {
        let out = facts(
            "import express from 'express'; import axios from 'axios'; import {Kafka} from 'kafkajs'; function setup(){const app=express(); app.get('/inside',h); const client=axios.create(); const producer=new Kafka({}).producer();} function register(){app.get('/phantom',h); client.post('http://api/phantom'); producer.send({topic:'phantom'});} app.get('/outside',h);",
        );
        let routes = out
            .facts
            .iter()
            .filter(|f| f.kind == FactKind::Route)
            .collect::<Vec<_>>();
        assert_eq!(routes.len(), 1, "{:?}", out.facts);
        assert_eq!(routes[0].operation.as_deref(), Some("/inside"));
        assert!(
            !out.facts
                .iter()
                .any(|f| matches!(f.kind, FactKind::Http | FactKind::Publish))
        );
        assert!(
            facts("import express from 'express'; {const app=express();} app.get('/phantom',h);")
                .facts
                .is_empty()
        );
    }
    #[test]
    fn app_facts_express_settings_lookups_are_not_routes() {
        let out = facts(
            "import express from 'express'; const app=express(); app.get('/setting'); app.get('/route',handler);",
        );
        assert_eq!(out.facts.len(), 1, "{:?}", out.facts);
        assert_eq!(out.facts[0].kind, FactKind::Route);
        assert_eq!(out.facts[0].operation.as_deref(), Some("/route"));
    }
    #[test]
    fn app_facts_unknown_call_chains_have_bounded_resolution_work() {
        let source = format!("unknown{};", ".step()".repeat(24));
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let expression = named_children(named_children(tree.root_node())[0])[0];
        let mut remaining = 1_000;
        assert!(resolve_bounded(expression, &source, &BTreeMap::new(), &mut remaining).is_none());
        assert_eq!(1_000 - remaining, 25);
        let mut limited = 16;
        assert!(resolve_bounded(expression, &source, &BTreeMap::new(), &mut limited).is_none());
        assert_eq!(limited, 0);
        assert!(
            facts(&format!("unknown{};", ".step()".repeat(256)))
                .facts
                .is_empty()
        );
    }
    #[test]
    fn app_facts_unknown_options_do_not_invent_a_method() {
        let out = facts(
            "fetch('/a'); fetch('/b',{method}); fetch('/c',opts); fetch('/d',{method:'POST',...opts}); fetch('/e',{method:'GET',method:'POST'}); fetch(`${base({})}/x`);",
        );
        assert_eq!(
            out.facts
                .iter()
                .map(|f| f.method.as_deref())
                .collect::<Vec<_>>(),
            [Some("GET"), None, None, None, Some("POST"), Some("GET")]
        );
        assert_eq!(out.facts.last().unwrap().operation.as_deref(), Some("/x"));
        assert!(out.facts.last().unwrap().dynamic);
    }
    #[test]
    fn app_facts_kafka_receiver_chains() {
        let out = facts(
            "import { Kafka } from 'kafkajs'; const kafka=new Kafka({}); const producer=kafka.producer(); const consumer=kafka.consumer({}); producer.send({topic:'invoice.created'}); consumer.subscribe({topic:'invoice.created'}); const unrelated={send(){}}; unrelated.send({topic:'wrong'});",
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Publish)
                .count(),
            1
        );
        assert_eq!(
            out.facts
                .iter()
                .filter(|f| f.kind == FactKind::Subscribe)
                .count(),
            1
        );
    }
    #[test]
    fn app_facts_nest_decorators_and_called_bootstrap() {
        let out = facts(
            "import { Controller, Post } from '@nestjs/common'; import { NestFactory } from '@nestjs/core'; @Controller('invoices') class C { @Post() create() {} } async function bootstrap() { const app=await NestFactory.create(C); await app.listen(3000); } bootstrap();",
        );
        assert!(out.entrypoint);
        assert!(
            out.facts.iter().any(|f| f.kind == FactKind::Route
                && f.operation.as_deref() == Some("/invoices")
                && f.method.as_deref() == Some("POST")),
            "{:?}",
            out.facts
        );
        assert!(!facts("import express from 'express'; export function start(){const app=express(); app.listen(3000);}").entrypoint);
    }
}
