use archgraph::collect::imports::{ImportStatement, analyze};
use archgraph::model::EdgeKind;

fn typescript(source: &str) -> Vec<ImportStatement> {
    analyze(source).statements
}

#[test]
fn multiline_aliases_and_type_only_clauses() {
    let imports = typescript(
        "import Default, {\n foo as renamed, type Shape\n} from './api';\nimport type { A } from './types';\nimport {type B, type C as D} from './other';",
    );
    assert_eq!(imports.len(), 3);
    assert_eq!(imports[0].kind, EdgeKind::Static);
    assert_eq!(imports[0].line, 1);
    assert_eq!(imports[0].names[0].imported, "default");
    assert_eq!(imports[0].names[1].local, "renamed");
    assert_eq!(imports[1].kind, EdgeKind::Type);
    assert_eq!(imports[2].kind, EdgeKind::Type);
    assert_eq!(imports[2].names[1].local, "D");
}

#[test]
fn reexports_preserve_source_and_exposed_names() {
    let imports = typescript(
        "export { original as publicName } from './first';\nexport * from './second';\nexport * as namespace from './third';\nexport type {Shape} from './types';",
    );
    assert_eq!(imports.len(), 4);
    assert!(imports.iter().all(|i| i.reexport));
    assert_eq!(imports[0].names[0].imported, "original");
    assert_eq!(imports[0].names[0].local, "publicName");
    assert!(imports[1].wildcard);
    assert_eq!(imports[2].names[0].local, "namespace");
    assert_eq!(imports[3].kind, EdgeKind::TypeReexport);
}

#[test]
fn require_scope_functions_arrows_methods_and_blocks() {
    let imports = typescript(
        r#"
const top = require('top');
if (enabled) { require('conditional'); }
const object = { item: require('object'), method() { require('method'); } };
function load() { require('function'); if (ok) { require('nested'); } }
const arrow = () => require('arrow');
const block = () => { require('block'); };
require('last');
"#,
    );
    let kinds: Vec<_> = imports
        .iter()
        .map(|i| (i.specifier.as_deref().unwrap(), i.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("top", EdgeKind::Static),
            ("conditional", EdgeKind::Static),
            ("object", EdgeKind::Static),
            ("method", EdgeKind::Dynamic),
            ("function", EdgeKind::Dynamic),
            ("nested", EdgeKind::Dynamic),
            ("arrow", EdgeKind::Dynamic),
            ("block", EdgeKind::Dynamic),
            ("last", EdgeKind::Static)
        ]
    );
}

#[test]
fn dynamic_computed_static_template_and_escaped_paths() {
    let imports = typescript(
        r#"import(`./fixed`); import(`./${name}`); import(prefix + '/file'); require('./\x66oo'); import './side';"#,
    );
    assert_eq!(imports.len(), 5);
    assert_eq!(imports[0].specifier.as_deref(), Some("./fixed"));
    assert_eq!(imports[0].kind, EdgeKind::Dynamic);
    assert_eq!(imports[1].specifier, None);
    assert_eq!(imports[2].specifier, None);
    assert_eq!(imports[3].specifier.as_deref(), Some("./foo"));
    assert!(imports[4].side_effect);
}

#[test]
fn ignores_comments_strings_regex_and_property_calls() {
    let imports = typescript(
        r#"
// import 'comment';
/* require('comment') */
const text = "import('string')";
const regex = /require\('regex'\)/;
obj.require('property'); obj?.import('property');
import { type as renamed } from 'actual';
"#,
    );
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].names[0].imported, "type");
    assert_eq!(imports[0].kind, EdgeKind::Static);
    assert_eq!(imports[0].line, 7);
}

#[test]
fn annotated_methods_and_control_statement_regex() {
    let imports = typescript(
        r#"
function load<T>(): Promise<T> { require('function'); }
class Loader { load(): string { return require('method'); } }
if (enabled) /require('not-an-import')/.test(text);
const divided = value / require('division');
"#,
    );
    assert_eq!(imports.len(), 3);
    assert_eq!(imports[0].kind, EdgeKind::Dynamic);
    assert_eq!(imports[1].kind, EdgeKind::Dynamic);
    assert_eq!(imports[2].specifier.as_deref(), Some("division"));
    assert_eq!(imports[2].kind, EdgeKind::Static);
}

#[test]
fn template_interpolations_scan_code_and_skip_raw_text() {
    let imports = typescript(
        r#"
const raw = `require('not-code') ${import('./inside')} tail`;
const nested = `${ { value: `${import('./nested')}` } } require('also-not-code')`;
import(`./${prefix}/${import('./loader')}`);
"#,
    );
    let specifiers: Vec<_> = imports.iter().map(|i| i.specifier.as_deref()).collect();
    assert_eq!(
        specifiers,
        vec![Some("./inside"), Some("./nested"), None, Some("./loader")]
    );
    assert!(imports.iter().all(|i| i.kind == EdgeKind::Dynamic));
    assert_eq!(imports[0].line, 2);
    assert_eq!(imports[1].line, 3);
}

#[test]
fn arrow_expression_scope_ends_at_the_surrounding_expression() {
    let imports = typescript(
        "call(() => require('lazy'), require('eager')); const f = () => require('other-lazy'), eager = require('other-eager');",
    );
    let kinds: Vec<_> = imports.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        vec![
            EdgeKind::Dynamic,
            EdgeKind::Static,
            EdgeKind::Dynamic,
            EdgeKind::Static
        ]
    );
}

#[test]
fn automatic_semicolons_end_arrow_scopes_without_ending_continuations() {
    let imports = typescript(
        r#"
const load = () => require('lazy')
const eager = require('eager')
const another = () => require('lazy-expression')
require('eager-expression')
const multiline = () =>
    require('lazy-first')
    + require('lazy-second')
const array = () => [
    require('lazy-array-first'),
    require('lazy-array-second')
]
require('eager-after-array')
const invoked = () => require('lazy-call')
    (require('lazy-call-argument'))
"#,
    );
    let kinds: Vec<_> = imports
        .iter()
        .map(|i| (i.specifier.as_deref().unwrap(), i.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("lazy", EdgeKind::Dynamic),
            ("eager", EdgeKind::Static),
            ("lazy-expression", EdgeKind::Dynamic),
            ("eager-expression", EdgeKind::Static),
            ("lazy-first", EdgeKind::Dynamic),
            ("lazy-second", EdgeKind::Dynamic),
            ("lazy-array-first", EdgeKind::Dynamic),
            ("lazy-array-second", EdgeKind::Dynamic),
            ("eager-after-array", EdgeKind::Static),
            ("lazy-call", EdgeKind::Dynamic),
            ("lazy-call-argument", EdgeKind::Dynamic),
        ]
    );
}

#[test]
fn default_parameters_run_with_the_function_not_at_declaration_time() {
    let imports = typescript(
        r#"
function named(value = require('function-default')) { return require('function-body'); }
const arrow = (value = require('arrow-default')) => require('arrow-body');
const typed = (value = require('typed-default')): string => value;
const object = { method(value = require('method-default')) { return value; } };
class Loader { load(value = require('class-default')): string { return value; } }
const nested = () => function inside(value = require('nested-default')) { return require('nested-body'); };
call(require('ordinary-call'))
{ require('ordinary-block'); }
"#,
    );
    let kinds: Vec<_> = imports
        .iter()
        .map(|i| (i.specifier.as_deref().unwrap(), i.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("function-default", EdgeKind::Dynamic),
            ("function-body", EdgeKind::Dynamic),
            ("arrow-default", EdgeKind::Dynamic),
            ("arrow-body", EdgeKind::Dynamic),
            ("typed-default", EdgeKind::Dynamic),
            ("method-default", EdgeKind::Dynamic),
            ("class-default", EdgeKind::Dynamic),
            ("nested-default", EdgeKind::Dynamic),
            ("nested-body", EdgeKind::Dynamic),
            ("ordinary-call", EdgeKind::Static),
            ("ordinary-block", EdgeKind::Static),
        ]
    );
}

#[test]
fn mixed_inline_type_flags_and_require_syntax_are_preserved_per_binding() {
    let statements = typescript(
        "import {type Shape, Runtime, type Other as Renamed} from './api'; const required=require('./api'); import('./api');",
    );
    assert_eq!(
        statements[0]
            .names
            .iter()
            .map(|name| name.type_only)
            .collect::<Vec<_>>(),
        [true, false, true]
    );
    assert!(!statements[0].require);
    assert!(statements[1].require);
    assert!(!statements[2].require);
}

#[test]
fn erased_use_proofs_do_not_accept_object_values_or_parameter_defaults() {
    let names = vec!["Guard".into()];
    assert!(
        analyze("import {Guard} from './guard'; function use(...guards: (Guard | Function)[]) {}")
            .type_only_uses(&names)
            .contains("Guard")
    );
    for source in [
        "import {Guard} from './guard'; const value={guard:Guard};",
        "import {Guard} from './guard'; function use(value = Guard) {}",
        "import {Guard} from './guard'; const typed: Guard = new Guard();",
    ] {
        assert!(
            analyze(source).type_only_uses(&names).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn a_function_parameter_named_require_is_a_local_call_not_an_import() {
    let imports = typescript(
        "function local(require) { require('./not-an-import'); } const object={load(require){require('./also-local')}}; require('./actual');",
    );
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].specifier.as_deref(), Some("./actual"));
    assert_eq!(imports[0].kind, EdgeKind::Static);
}
