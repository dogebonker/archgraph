use archgraph::collect::{barrels, depcruise, imports, typescript_resolution::enrich};
use archgraph::model::{EdgeKind, ImportReason};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
fn root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ts-resolution-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}
fn write(root: &Path, file: &str, source: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}
fn raw(file: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"modules":[{"source":file,"dependencies":[]}],"summary":{"totalCruised":1,"environment":{"version":"fixture"}}})).unwrap()
}
fn scan(root: &Path, file: &str) -> archgraph::collect::RawGraph {
    let (json, ledger) = enrich(root, &raw(file)).unwrap();
    let mut graph = depcruise::parse_at(&json, root).unwrap();
    graph.imports = ledger;
    barrels::typescript(root, &json, &mut graph).unwrap();
    graph
}

#[test]
fn nearest_jsonc_config_paths_extends_and_baseurl_recover_and_scan_new_targets() {
    let root = root("config");
    write(
        &root,
        "tsconfig.base.json",
        r##"{ // shared settings
 "compilerOptions":{"baseUrl":".","paths":{"@base/*":["shared/*"],},},}"##,
    );
    write(
        &root,
        "tsconfig.json",
        r##"{"extends":"./tsconfig.base.json"}"##,
    );
    write(&root, "shared/base.ts", "export const Base=1;");
    write(
        &root,
        "packages/app/tsconfig.json",
        r##"{"extends":"../../tsconfig.base.json","compilerOptions":{"baseUrl":".","paths":{"@local/*":["src/*"]}}}"##,
    );
    write(
        &root,
        "packages/app/src/main.ts",
        "import {Local} from '@local/local'; import {Extra} from 'src/extra'; import {Base} from '@base/base';",
    );
    write(
        &root,
        "packages/app/src/local.ts",
        "import {Extra} from './extra.js'; export const Local=Extra;",
    );
    write(&root, "packages/app/src/extra.ts", "export const Extra=1;");
    let graph = scan(&root, "packages/app/src/main.ts");
    let records = graph.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "@local/local" && r.targets == ["packages/app/src/local.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "src/extra" && r.targets == ["packages/app/src/extra.ts"])
    );
    // Child paths replace the inherited map, rather than silently merging aliases.
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "@base/base" && r.unresolved.is_some())
    );
    assert!(
        records
            .iter()
            .any(|r| r.from.ends_with("local.ts") && r.targets == ["packages/app/src/extra.ts"])
    );
    let graph = scan(&root, "shared/base.ts");
    assert!(graph.imports.unwrap().is_empty());
    write(&root, "src/use.ts", "import {Base} from '@base/base';");
    assert!(
        scan(&root, "src/use.ts")
            .imports
            .unwrap()
            .iter()
            .any(|r| r.targets == ["shared/base.ts"])
    );
}

#[test]
fn workspaces_exports_imports_patterns_conditions_and_manifest_externals() {
    let root = root("packages");
    write(
        &root,
        "package.json",
        r##"{"private":true,"workspaces":{"packages":["packages/*"]},"dependencies":{"react":"*"}}"##,
    );
    write(
        &root,
        "packages/lib/package.json",
        r##"{"name":"@demo/lib","exports":{".":{"import":"./dist/index.js","require":"./src/require.ts"},"./features/*":"./src/features/*.ts"}}"##,
    );
    write(
        &root,
        "packages/lib/src/index.ts",
        "export {Thing} from './thing';",
    );
    write(&root, "packages/lib/src/thing.ts", "export const Thing=1;");
    write(
        &root,
        "packages/lib/src/require.ts",
        "export const Thing=2;",
    );
    write(&root, "packages/lib/src/features/a.ts", "export const A=1;");
    write(
        &root,
        "packages/app/package.json",
        r##"{"name":"@demo/app","imports":{"#local/*":"./src/*.ts","#react":"react"}}"##,
    );
    write(
        &root,
        "packages/app/src/main.ts",
        "import {Thing} from '@demo/lib'; const other=require('@demo/lib'); import {A} from '@demo/lib/features/a'; import {Local} from '#local/local'; import React from '#react'; import fs from 'node:fs'; import './theme.css'; import missing from 'missing-package'; import(`./${name}`);",
    );
    write(&root, "packages/app/src/local.ts", "export const Local=1;");
    write(&root, "packages/app/src/theme.css", "body{}");
    let graph = scan(&root, "packages/app/src/main.ts");
    let records = graph.imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "@demo/lib" && r.targets == ["packages/lib/src/thing.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "@demo/lib" && r.targets == ["packages/lib/src/require.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.targets == ["packages/lib/src/features/a.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.targets == ["packages/app/src/local.ts"])
    );
    assert!(
        records
            .iter()
            .any(|r| r.external.as_deref() == Some("react"))
    );
    assert!(
        records
            .iter()
            .any(|r| r.external.as_deref() == Some("node:fs"))
    );
    assert!(
        records
            .iter()
            .any(|r| r.targets == ["packages/app/src/theme.css"])
    );
    assert!(
        records
            .iter()
            .any(|r| matches!(r.unresolved, Some(ImportReason::Dynamic)))
    );
    assert!(
        records.iter().any(|r| r.specifier == "missing-package"
            && matches!(r.unresolved, Some(ImportReason::Missing)))
    );
}

#[test]
fn inherited_repo_config_and_outside_scan_workspace_are_bounded_and_external() {
    let root = root("scan-boundary");
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    write(&root, "package.json", r##"{"workspaces":["packages/*"]}"##);
    write(
        &root,
        "tsconfig.json",
        r##"{"compilerOptions":{"baseUrl":".","paths":{"@shared/*":["packages/shared/src/*"]}}}"##,
    );
    write(
        &root,
        "packages/shared/package.json",
        r##"{"name":"@demo/shared","exports":"./src/index.ts"}"##,
    );
    write(
        &root,
        "packages/shared/src/index.ts",
        "export const Shared=1;",
    );
    write(
        &root,
        "packages/app/package.json",
        r##"{"name":"@demo/app"}"##,
    );
    write(
        &root,
        "packages/app/tsconfig.json",
        r##"{"extends":"../../tsconfig.json"}"##,
    );
    write(
        &root,
        "packages/app/src/main.ts",
        "import {Shared} from '@demo/shared'; import {Shared as Alias} from '@shared/index';",
    );
    let graph = scan(&root.join("packages/app"), "src/main.ts");
    assert_eq!(graph.files, ["src/main.ts"]);
    let records = graph.imports.unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.external.as_deref() == Some("@demo/shared"))
    );
}

#[test]
fn source_less_recordings_do_not_claim_occurrence_completeness() {
    let root = root("recording");
    let (_, ledger) = enrich(&root, &raw("missing.ts")).unwrap();
    assert!(ledger.is_none());
}

#[test]
fn implicit_interfaces_and_explicit_mixed_bindings_refine_only_proven_type_uses() {
    let root = root("types");
    write(
        &root,
        "src/main.ts",
        "import { CanActivate, type HttpServer, Logger } from './facade'; export function use(...guards: (CanActivate | Function)[]) { const logger=new Logger(); return logger; } function server(value: HttpServer) { return value; }",
    );
    write(
        &root,
        "src/facade.ts",
        "export {CanActivate} from './guard'; export {HttpServer} from './server'; export {Logger} from './logger';",
    );
    write(
        &root,
        "src/guard.ts",
        "export interface CanActivate { activate(): boolean; }",
    );
    write(&root, "src/server.ts", "export interface HttpServer {}");
    write(&root, "src/logger.ts", "export class Logger {}");
    let graph = scan(&root, "src/main.ts");
    for target in ["src/guard.ts", "src/server.ts"] {
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == "src/main.ts" && e.to == target)
            .unwrap();
        assert_eq!(edge.kind, EdgeKind::Type, "{target}");
        assert_eq!(edge.names.len(), 1);
    }
    assert_eq!(
        graph
            .edges
            .iter()
            .find(|e| e.from == "src/main.ts" && e.to == "src/logger.ts")
            .unwrap()
            .kind,
        EdgeKind::Static
    );
    write(
        &root,
        "src/main.ts",
        "import {CanActivate} from './facade'; const value={guard:CanActivate}; export const ref=CanActivate;",
    );
    assert_eq!(
        scan(&root, "src/main.ts")
            .edges
            .iter()
            .find(|e| e.from == "src/main.ts")
            .unwrap()
            .kind,
        EdgeKind::Static
    );
    assert!(
        imports::type_only_exports("export interface Token {} export const Token=1;").is_empty()
    );
}

#[test]
fn pnpm_workspaces_are_discovered_and_unknown_names_stay_unresolved() {
    let root = root("pnpm");
    write(&root, "pnpm-workspace.yaml", "packages:\n  - 'modules/*'\n");
    write(
        &root,
        "modules/lib/package.json",
        r##"{"name":"local-lib","main":"src/index.ts"}"##,
    );
    write(&root, "modules/lib/src/index.ts", "export const X=1;");
    write(
        &root,
        "src/main.ts",
        "import {X} from 'local-lib'; import {Y} from '@/missing';",
    );
    let records = scan(&root, "src/main.ts").imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.targets == ["modules/lib/src/index.ts"])
    );
    assert!(
        records.iter().any(
            |r| r.specifier == "@/missing" && matches!(r.unresolved, Some(ImportReason::Alias))
        )
    );
}

#[test]
fn config_cycles_missing_imports_and_real_native_packages_are_accounted_honestly() {
    let root = root("gaps");
    write(
        &root,
        "tsconfig.json",
        r##"{"extends":"./other.json","compilerOptions":{"paths":{"@alias/*":["missing/*"]}}}"##,
    );
    write(&root, "other.json", r##"{"extends":"./tsconfig.json"}"##);
    write(
        &root,
        "src/main.ts",
        "import './missing'; import x from '@alias/missing'; import y from 'test'; import fs from 'node:fs/promises'; const binding=require('missing.node');",
    );
    let records = scan(&root, "src/main.ts").imports.unwrap();
    assert_eq!(records.len(), 5);
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "./missing"
                && matches!(r.unresolved, Some(ImportReason::Missing)))
    );
    assert!(
        records.iter().any(|r| r.specifier == "@alias/missing"
            && matches!(r.unresolved, Some(ImportReason::Alias)))
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "test" && matches!(r.unresolved, Some(ImportReason::Missing)))
    );
    assert!(
        records.iter().any(|r| r.specifier == "node:fs/promises"
            && r.external.as_deref() == Some("node:fs/promises"))
    );
    assert!(records.iter().any(
        |r| r.specifier == "missing.node" && matches!(r.unresolved, Some(ImportReason::Native))
    ));
}

#[cfg(unix)]
#[test]
fn source_and_config_symlinks_cannot_escape_the_enclosing_repository() {
    let root = root("escape");
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let outside = root.with_extension("outside.ts");
    std::fs::write(&outside, "export const Secret=1;").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape.ts")).unwrap();
    write(&root, "main.ts", "import {Secret} from './escape';");
    let json = json!({"modules":[{"source":"main.ts","dependencies":[{"module":"./escape","resolved":"escape.ts"}]},{"source":"escape.ts","dependencies":[]}],"summary":{"totalCruised":2}});
    let (json, records) = enrich(&root, &serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(records.is_none());
    let graph = depcruise::parse(&json).unwrap();
    assert!(graph.edges.is_empty());
}

#[test]
fn local_interface_does_not_override_a_forwarded_runtime_export() {
    let root = root("forwarded-value");
    write(
        &root,
        "facade.ts",
        "export {Foo} from './value'; interface Foo {}",
    );
    write(
        &root,
        "value.ts",
        "import {X} from './mixed'; export class Foo {} console.log(X);",
    );
    write(
        &root,
        "mixed.ts",
        "export {Foo} from './facade'; export const X=1;",
    );
    let graph = scan(&root, "mixed.ts");
    assert!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.from == "mixed.ts")
            .all(|edge| edge.kind != EdgeKind::Type)
    );
    assert!(imports::type_only_exports("export {Foo} from './value'; interface Foo {}").is_empty());
    assert!(
        imports::type_only_exports("export interface Foo {} export {Foo} from './value';")
            .is_empty()
    );
    assert!(graph.edges.iter().any(|edge| edge.from == "mixed.ts"
        && edge.to == "facade.ts"
        && edge.kind == EdgeKind::Static));
    assert!(graph.edges.iter().any(|edge| edge.from == "value.ts"
        && edge.to == "mixed.ts"
        && edge.kind == EdgeKind::Static));
    let module_of = graph
        .files
        .iter()
        .map(|file| (file.clone(), file.clone()))
        .collect();
    let cycles = archgraph::graph::cycles(
        &module_of,
        &graph.edges,
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::new(),
        false,
    );
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].members, ["facade.ts", "mixed.ts", "value.ts"]);
}

#[test]
fn runtime_export_conditions_preserve_order_and_do_not_enable_development() {
    let root = root("conditions");
    write(&root, "package.json", r##"{"workspaces":["packages/*"]}"##);
    write(
        &root,
        "packages/lib/package.json",
        r##"{"name":"demo-lib","exports":{".":{"development":"./dev.mjs","source":"./dev.mjs","default":"./prod.mjs","import":"./late.mjs"},"./require":{"default":"./prod.mjs","require":"./late.mjs"}}}"##,
    );
    for file in ["prod.mjs", "dev.mjs", "late.mjs"] {
        write(&root, &format!("packages/lib/{file}"), "export const X=1;");
    }
    write(
        &root,
        "main.mjs",
        "import {X} from 'demo-lib'; const required=require('demo-lib/require');",
    );
    let records = scan(&root, "main.mjs").imports.unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.targets == ["packages/lib/prod.mjs"]),
        "{records:?}"
    );
}

#[test]
fn unknown_native_subpaths_are_unresolved_instead_of_proven_packages() {
    let root = root("native-negative");
    write(
        &root,
        "main.ts",
        "import missing from 'node:fs/definitely-missing'; import promises from 'fs/promises';",
    );
    let records = scan(&root, "main.ts").imports.unwrap();
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "node:fs/definitely-missing"
                && matches!(r.unresolved, Some(ImportReason::Native))
                && r.external.is_none())
    );
    assert!(
        records
            .iter()
            .any(|r| r.specifier == "fs/promises"
                && r.external.as_deref() == Some("node:fs/promises"))
    );
}

#[test]
fn nearest_config_alias_overrides_a_successful_but_root_based_collector_target() {
    let root = root("known-alias");
    write(
        &root,
        "tsconfig.json",
        r##"{"compilerOptions":{"baseUrl":".","paths":{"alias":["shared.ts"]}}}"##,
    );
    write(&root, "shared.ts", "export const X=0;");
    write(
        &root,
        "packages/app/tsconfig.json",
        r##"{"compilerOptions":{"baseUrl":".","paths":{"alias":["src/local.ts"]}}}"##,
    );
    write(&root, "packages/app/src/local.ts", "export const X=1;");
    write(
        &root,
        "packages/app/src/main.ts",
        "import {X} from 'alias';",
    );
    let json = json!({"modules":[{"source":"packages/app/src/main.ts","dependencies":[{"module":"alias","resolved":"shared.ts","couldNotResolve":false,"dependencyTypes":["local","import"]}]}],"summary":{"totalCruised":1}});
    let (json, records) = enrich(&root, &serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(records.unwrap()[0].targets, ["packages/app/src/local.ts"]);
    let graph = depcruise::parse(&json).unwrap();
    assert!(graph.edges.iter().all(|e| e.to != "shared.ts"));
}

#[test]
fn extends_arrays_merge_supported_options_and_effective_baseurl_applies_to_paths() {
    let root = root("extends-array");
    write(
        &root,
        "aliases.json",
        r##"{"compilerOptions":{"paths":{"alias":["src/local.ts"]}}}"##,
    );
    write(
        &root,
        "base.json",
        r##"{"compilerOptions":{"baseUrl":"packages/app"}}"##,
    );
    write(
        &root,
        "packages/app/tsconfig.json",
        r##"{"extends":["../../aliases.json","../../base.json"]}"##,
    );
    write(
        &root,
        "packages/app/src/main.ts",
        "import {X} from 'alias';",
    );
    write(&root, "packages/app/src/local.ts", "export const X=1;");
    assert_eq!(
        scan(&root, "packages/app/src/main.ts").imports.unwrap()[0].targets,
        ["packages/app/src/local.ts"]
    );
}

#[test]
fn a_shadowed_parameter_call_does_not_invent_a_recovered_local_dependency() {
    let root = root("require-parameter");
    write(
        &root,
        "main.ts",
        "export function load(require) { return require('./local'); }",
    );
    write(&root, "local.ts", "export const Local=1;");
    let graph = scan(&root, "main.ts");
    assert!(graph.edges.is_empty());
    assert!(graph.imports.unwrap().is_empty());
    assert_eq!(graph.files, ["main.ts"]);
}

#[test]
fn a_paths_miss_falls_back_to_packages_and_dotted_names_get_extensions() {
    let root = root("paths-fallback");
    write(
        &root,
        "tsconfig.json",
        r##"{"compilerOptions":{"paths":{"@app/*":["src/*"],"*":["types/*"]}}}"##,
    );
    write(
        &root,
        "package.json",
        r##"{"name":"app","dependencies":{"react":"18.0.0"}}"##,
    );
    write(
        &root,
        "src/main.ts",
        "import React from 'react'; import {S} from './user.service'; import {M} from '@app/user.model'; import {Z} from 'nowhere';",
    );
    write(&root, "src/user.service.ts", "export const S=1;");
    write(&root, "src/user.model.ts", "export const M=1;");
    let records = scan(&root, "src/main.ts").imports.unwrap();
    let record = |specifier: &str| records.iter().find(|r| r.specifier == specifier).unwrap();
    assert_eq!(record("react").external.as_deref(), Some("react"));
    assert_eq!(record("./user.service").targets, ["src/user.service.ts"]);
    assert_eq!(record("@app/user.model").targets, ["src/user.model.ts"]);
    assert!(matches!(
        record("nowhere").unresolved,
        Some(ImportReason::Alias)
    ));
}
