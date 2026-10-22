use archgraph::{
    collect::{
        excludes::{Options, Policy},
        native::typescript,
    },
    model::{EdgeKind, Language},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static SEQ: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "archgraph-native-ts-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        for (name, source) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        Self(root)
    }
    fn scan(&self) -> archgraph::collect::native::Output {
        let policy = Policy::new(&self.0, Language::Typescript, Options::default()).unwrap();
        typescript::collect(&self.0, ".", &policy, true).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn aliases_barrels_mixed_types_and_dynamic_imports() {
    let f = Fixture::new(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@app/*":["src/*"]}}}"#,
        ),
        (
            "src/def.ts",
            "export interface Shape {}\nexport const value=1;",
        ),
        (
            "src/barrel.ts",
            "export {value as renamed, type Shape} from './def';",
        ),
        (
            "src/index.ts",
            "export {renamed, type Shape} from './barrel';",
        ),
        ("src/types.ts", "import type {Shape} from './index';"),
        (
            "src/use.ts",
            "import {renamed, type Shape} from '@app/index';\nconst later=()=>import('./def');\nconst unknown=import(path);",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/use.ts" && e.to == "src/def.ts")
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/use.ts" && e.kind == EdgeKind::Dynamic)
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "src/types.ts" && e.names == ["Shape"] && e.kind == EdgeKind::Type)
    );
    assert!(
        out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .any(|i| i.specifier == "<computed>" && i.unresolved.is_some())
    );
    assert!(out.symbols.exports.iter().any(|e| e.file == "src/index.ts"
        && e.name == "renamed"
        && e.target.file == "src/def.ts"
        && e.target.name == "value"));
}
#[test]
fn js_tsx_and_require_shadowing_are_ast_driven() {
    let f = Fixture::new(&[
        ("target.js", "export default 1;"),
        (
            "view.tsx",
            "import value from './target'; export const View=()=> <div>{value}</div>;",
        ),
        ("lazy.js", "function load(){return require('./target');}"),
        (
            "shadow.js",
            "function load(require){return require('./target');}\nconst fake=\"import './nope'\";",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "view.tsx" && e.to == "target.js")
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "lazy.js" && e.kind == EdgeKind::Dynamic)
    );
    assert!(!out.raw.edges.iter().any(|e| e.from == "shadow.js"));
}
#[test]
fn package_exports_conditions_and_node_builtins() {
    let f = Fixture::new(&[
        (
            "use.ts",
            "import value from 'thing'; const other=require('thing'); import fs from 'node:fs'; import nope from 'missing';",
        ),
        (
            "node_modules/thing/package.json",
            r#"{"name":"thing","exports":{"import":"./esm.js","require":"./cjs.js"}}"#,
        ),
        ("node_modules/thing/esm.js", "export default 1"),
        ("node_modules/thing/cjs.js", "module.exports=2"),
    ]);
    let out = f.scan();
    let records = out.raw.imports.unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.external.as_deref() == Some("thing"))
            .count(),
        2
    );
    assert!(
        records
            .iter()
            .any(|r| r.external.as_deref() == Some("node:fs"))
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "missing" && r.unresolved.is_some())
    );
    assert!(!out.raw.files.iter().any(|p| p.contains("node_modules")));
}
#[test]
fn malformed_source_retains_valid_imports_and_records_limitations() {
    let f = Fixture::new(&[
        ("a.ts", "import {value} from './b';\nconst = ;"),
        ("b.ts", "export const value=1;"),
    ]);
    let out = f.scan();
    assert!(out.raw.edges.iter().any(|e| e.to == "b.ts"));
    assert!(
        out.symbols
            .limitations
            .iter()
            .any(|s| s.contains("syntax errors"))
    );
}
#[test]
fn reexport_cycles_are_explained() {
    let f = Fixture::new(&[
        ("a.ts", "export {value} from './b';"),
        ("b.ts", "export {value} from './a';"),
        ("use.ts", "import {value} from './a';"),
    ]);
    let out = f.scan();
    assert!(
        out.symbols
            .limitations
            .iter()
            .any(|s| s.contains("re-export chain cycle"))
    );
}
#[test]
fn workspace_exports_and_package_import_maps_resolve_without_installing() {
    let f = Fixture::new(&[
        (
            "package.json",
            r##"{"workspaces":["packages/*"],"imports":{"#core":"./packages/core/index.ts"},"dependencies":{"external-only":"*"}}"##,
        ),
        (
            "packages/core/package.json",
            r#"{"name":"@sample/core","exports":"./index.ts"}"#,
        ),
        ("packages/core/index.ts", "export const value=1;"),
        (
            "packages/app/main.ts",
            "import {value} from '@sample/core'; import {value as local} from '#core'; import other from 'external-only';",
        ),
    ]);
    let out = f.scan();
    let records = out.raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "@sample/core" && r.targets == ["packages/core/index.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "#core" && r.targets == ["packages/core/index.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.external.as_deref() == Some("external-only"))
    );
}
#[test]
fn parent_tsconfig_is_discovered_for_scanned_subroot() {
    let f = Fixture::new(&[
        (".git/HEAD", "ref: refs/heads/main"),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@app/*":["packages/app/*"]}}}"#,
        ),
        ("packages/app/main.ts", "import {value} from '@app/value';"),
        ("packages/app/value.ts", "export const value=1;"),
    ]);
    let subroot = f.0.join("packages/app");
    let policy = Policy::new(&subroot, Language::Typescript, Options::default()).unwrap();
    let out = typescript::collect(&subroot, ".", &policy, true).unwrap();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "main.ts" && e.to == "value.ts")
    );
}
#[cfg(unix)]
#[test]
fn symlinked_source_and_config_cannot_escape_repository() {
    use std::os::unix::fs::symlink;
    let outside = Fixture::new(&[
        ("outside.ts", "export const secret=1;"),
        ("tsconfig.json", r#"{"compilerOptions":{"baseUrl":"."}}"#),
    ]);
    let f = Fixture::new(&[("main.ts", "import {secret} from './escaped';")]);
    symlink(outside.0.join("outside.ts"), f.0.join("escaped.ts")).unwrap();
    symlink(outside.0.join("tsconfig.json"), f.0.join("tsconfig.json")).unwrap();
    let out = f.scan();
    assert!(!out.raw.files.iter().any(|p| p == "escaped.ts"));
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.unresolved.is_some())
    );
}
#[test]
fn source_scope_keeps_imported_siblings_but_not_unrelated_root_files() {
    let f = Fixture::new(&[
        ("src/main.ts", "import {value} from '../shared/value';"),
        ("shared/value.ts", "export const value=1;"),
        ("unrelated.ts", "export const unused=1;"),
    ]);
    let policy = Policy::new(&f.0, Language::Typescript, Options::default()).unwrap();
    let out = typescript::collect(&f.0, "src", &policy, false).unwrap();
    assert!(out.raw.files.contains(&"src/main.ts".into()));
    assert!(out.raw.files.contains(&"shared/value.ts".into()));
    assert!(!out.raw.files.contains(&"unrelated.ts".into()));
}
#[test]
fn unavailable_extended_tsconfig_does_not_erase_relative_imports() {
    let f = Fixture::new(&[
        (".git/HEAD", "ref: refs/heads/main"),
        (
            "packages/app/tsconfig.json",
            r#"{"extends":"@tsconfig/node20/tsconfig.json"}"#,
        ),
        (
            "packages/app/src/main.ts",
            "import {value} from './value'; import fs from 'fs';",
        ),
        ("packages/app/src/value.ts", "export const value=1;"),
    ]);
    let subroot = f.0.join("packages/app/src");
    let policy = Policy::new(&subroot, Language::Typescript, Options::default()).unwrap();
    let out = typescript::collect(&subroot, ".", &policy, true).unwrap();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "main.ts" && e.to == "value.ts")
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.external.as_deref() == Some("node:fs"))
    );
    assert!(
        out.symbols
            .limitations
            .iter()
            .any(|s| s.contains("unavailable tsconfig"))
    );
}
#[test]
fn implicit_types_require_erased_declaration_and_type_context_usage() {
    let f = Fixture::new(&[
        (
            "types.ts",
            "export interface Shape {} export type ID=string; export class Model {}",
        ),
        (
            "typed.ts",
            "import {Shape,ID} from './types'; const value:Shape={} as Shape; type Key=ID;",
        ),
        (
            "runtime.ts",
            "import {Model} from './types'; const value:Model=new Model();",
        ),
        (
            "uncertain.ts",
            "import {Shape} from './types'; const invalid=Shape;",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "typed.ts" && e.kind == EdgeKind::Type)
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "runtime.ts" && e.kind == EdgeKind::Static)
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "uncertain.ts" && e.kind == EdgeKind::Static)
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.from == "typed.ts" && r.kind == EdgeKind::Type)
    );
}
#[test]
fn resolved_sibling_workspace_is_known_external_to_scanned_package() {
    let f = Fixture::new(&[
        (".git/HEAD", "ref: refs/heads/main"),
        ("package.json", r#"{"workspaces":["packages/*"]}"#),
        (
            "packages/core/package.json",
            r#"{"name":"@sample/core","exports":"./index.ts"}"#,
        ),
        ("packages/core/index.ts", "export const value=1;"),
        ("packages/app/package.json", r#"{"name":"@sample/app"}"#),
        (
            "packages/app/main.ts",
            "import {value} from '@sample/core';",
        ),
    ]);
    let subroot = f.0.join("packages/app");
    let policy = Policy::new(&subroot, Language::Typescript, Options::default()).unwrap();
    let out = typescript::collect(&subroot, ".", &policy, true).unwrap();
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.external.as_deref() == Some("@sample/core") && r.unresolved.is_none())
    );
}
#[test]
fn implicit_type_proof_follows_renamed_barrel_exports() {
    let f = Fixture::new(&[
        ("types.ts", "export interface Shape {}"),
        ("index.ts", "export {Shape as PublicShape} from './types';"),
        (
            "use.ts",
            "import {PublicShape} from './index'; const value:PublicShape={};",
        ),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "use.ts" && e.to == "types.ts" && e.kind == EdgeKind::Type)
    );
}
#[test]
fn resolved_assets_are_accounted_without_source_graph_nodes_or_edges() {
    let f = Fixture::new(&[
        (
            "main.ts",
            "import './style.scss'; import data from './data.json'; import font from './font.woff2';",
        ),
        ("style.scss", "body { color: red; }"),
        ("data.json", r#"{"value":1}"#),
        ("font.woff2", "fixture bytes"),
    ]);
    let out = f.scan();
    let records = out.raw.imports.unwrap();
    assert_eq!(records.len(), 3);
    assert!(
        records
            .iter()
            .all(|r| r.unresolved.is_none() && r.targets.len() == 1)
    );
    assert_eq!(out.raw.files, vec!["main.ts"]);
    assert!(out.raw.edges.is_empty());
}
#[test]
fn external_all_type_specifiers_retain_type_kind_and_mixed_values_runtime() {
    let f = Fixture::new(&[
        ("package.json", r#"{"dependencies":{"pkg":"*"}}"#),
        (
            "main.ts",
            "import {type A, type B} from 'pkg';\nimport {type A, value} from 'pkg';",
        ),
    ]);
    let out = f.scan();
    let records = out.raw.imports.unwrap();
    assert!(records.iter().any(|r| r.line == Some(1)
        && r.kind == EdgeKind::Type
        && r.external.as_deref() == Some("pkg")));
    assert!(
        records
            .iter()
            .any(|r| r.line == Some(2) && r.kind == EdgeKind::Static)
    );
}
#[test]
fn unbuilt_declared_sibling_workspace_root_is_known_but_subpaths_are_unproven() {
    let f = Fixture::new(&[
        (".git/HEAD", "ref: refs/heads/main"),
        ("package.json", r#"{"workspaces":["packages/*"]}"#),
        (
            "packages/core/package.json",
            r#"{"name":"@sample/core","main":"./dist/index.js"}"#,
        ),
        ("packages/core/src/index.ts", "export const value=1;"),
        (
            "packages/app/package.json",
            r#"{"name":"@sample/app","dependencies":{"@sample/core":"workspace:*"}}"#,
        ),
        (
            "packages/app/main.ts",
            "import {value} from '@sample/core';\nimport hidden from '@sample/core/private';",
        ),
    ]);
    let subroot = f.0.join("packages/app");
    let policy = Policy::new(&subroot, Language::Typescript, Options::default()).unwrap();
    let out = typescript::collect(&subroot, ".", &policy, true).unwrap();
    let records = out.raw.imports.unwrap();
    assert!(records.iter().any(|r| r.specifier == "@sample/core"
        && r.external.as_deref() == Some("@sample/core")
        && r.unresolved.is_none()));
    assert!(records.iter().any(|r| r.specifier == "@sample/core/private"
        && r.unresolved.is_some()
        && r.external.is_none()));
    let all = f.scan();
    assert!(
        all.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.specifier == "@sample/core" && r.unresolved.is_some())
    );
}
#[test]
fn denied_package_exports_are_not_proven_by_declared_dependency() {
    let f = Fixture::new(&[
        ("package.json", r#"{"dependencies":{"pkg":"*"}}"#),
        ("main.ts", "import hidden from 'pkg/private';"),
        (
            "node_modules/pkg/package.json",
            r#"{"name":"pkg","exports":{".":"./index.js"}}"#,
        ),
        ("node_modules/pkg/index.js", "export default 1;"),
        ("node_modules/pkg/private.js", "export default 2;"),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .all(|r| r.external.is_none() && r.unresolved.is_some())
    );
}
#[test]
fn type_star_reexports_recover_the_grammar_error_type_token() {
    let f = Fixture::new(&[
        ("types.ts", "export interface Shape {}"),
        ("index.ts", "export type * from './types';"),
        ("runtime.ts", "export * from './types';"),
        ("package.json", r#"{"dependencies":{"pkg":"*"}}"#),
        ("external.ts", "export type * from 'pkg';"),
    ]);
    let out = f.scan();
    let records = out.raw.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.from == "index.ts"
                && matches!(r.kind, EdgeKind::Type | EdgeKind::TypeReexport))
    );
    assert!(records.iter().any(|r| r.from == "external.ts"
        && r.kind == EdgeKind::TypeReexport
        && r.external.as_deref() == Some("pkg")));
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "index.ts"
                && matches!(e.kind, EdgeKind::Type | EdgeKind::TypeReexport))
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "runtime.ts" && e.kind == EdgeKind::Reexport)
    );
}
#[test]
fn import_equals_require_is_a_require_edge() {
    let f = Fixture::new(&[
        ("b.ts", "export const value = 1;"),
        ("c.ts", "export interface Shape {}"),
        (
            "a.ts",
            "import b = require('./b');\nimport type c = require('./c');\nconsole.log('exports'); reexportsFoo();",
        ),
        ("cjs.js", "module.exports = {};"),
    ]);
    let out = f.scan();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "a.ts" && e.to == "b.ts" && e.kind == EdgeKind::Static)
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "a.ts" && e.to == "c.ts" && e.kind == EdgeKind::Type)
    );
    let commonjs: Vec<_> = out
        .symbols
        .limitations
        .iter()
        .filter(|s| s.contains("CommonJS exports are not symbol-indexed"))
        .collect();
    assert_eq!(
        commonjs,
        ["cjs.js: CommonJS exports are not symbol-indexed"]
    );
}
#[test]
fn host_globals_are_recorded_once_per_file_unless_declared() {
    let f = Fixture::new(&[
        (
            "counted.ts",
            "export const x = 1;\nprocess.env.A;\nprocess.exit(0);\nif (typeof window !== 'undefined') {}",
        ),
        ("timer.js", "setTimeout(() => {}, 1);"),
        ("const.ts", "const process = 1; process;"),
        ("param.ts", "function get(fetch) { return fetch('x'); }"),
        ("arrow.ts", "docs.map(document => document);"),
        ("catch.ts", "try {} catch (location) { location; }"),
        ("for.ts", "for (const location of xs) { location; }"),
        ("default.js", "function f(crypto = 1) { return crypto; }"),
        ("pair.ts", "const { a: crypto } = o; crypto;"),
        ("rest.ts", "const [...navigator] = xs; navigator;"),
        (
            "alias.ts",
            "import { x as crypto } from './counted'; crypto;",
        ),
        (
            "namespace.ts",
            "import * as console from 'c'; console.log(1);",
        ),
        ("property.ts", "x.process; const o = { process: 1 };"),
        ("types.ts", "let w: Window; type W = typeof window;"),
        (
            "cjs.js",
            "const r = require('./timer'); module.exports = r;",
        ),
        ("a.d.ts", "declare const v: string;\nprocess.env;\nwindow;"),
        ("b.d.mts", "process.env;"),
        ("c.d.cts", "process.env;"),
        (
            "reexport.ts",
            "export { fetch } from './counted'; export * as crypto from './counted';",
        ),
        (
            "casts.ts",
            "1 as unknown as typeof window; 1 satisfies typeof console.log;\nfunction g(x): x is typeof self { return true; }",
        ),
        (
            "require.ts",
            "import process = require('process'); process.env;",
        ),
        (
            "declared.ts",
            "const f = function fetch() {}; const g = function* crypto() {}; function* console() {}\ndeclare function setTimeout(): void; namespace navigator {} enum location {}\nfetch; crypto; console; setTimeout; navigator; location;",
        ),
    ]);
    // Declaration files are generated by default; include them so the collector reads a.d.ts.
    let options = Options {
        include_generated: true,
        ..Options::default()
    };
    let policy = Policy::new(&f.0, Language::Typescript, options).unwrap();
    let mut globals: Vec<_> = typescript::collect(&f.0, ".", &policy, true)
        .unwrap()
        .raw
        .globals
        .unwrap()
        .into_iter()
        .map(|g| (g.from, g.name, g.line))
        .collect();
    globals.sort();
    assert_eq!(
        globals,
        [
            ("counted.ts".into(), "process".into(), 2),
            ("counted.ts".into(), "window".into(), 4),
            ("timer.js".into(), "setTimeout".into(), 1),
        ]
    );
}
