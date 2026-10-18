//! Git ownership and Go package evidence used by the impact command.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use archgraph::impact::{Impact, Reviewer};
use archgraph::model::Document;
use archgraph::{collect, git, modules};
use serde::Deserialize;

#[derive(Debug)]
struct Commit {
    author: String,
    paths: Vec<String>,
}

fn parse_commits(bytes: &[u8]) -> Result<Vec<Commit>> {
    let text = std::str::from_utf8(bytes).context("Git reviewer paths must be UTF-8")?;
    let fields: Vec<_> = text.split('\0').collect();
    let mut commits: Vec<Commit> = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        match fields[i] {
            "IMPACT" => {
                let author = fields
                    .get(i + 2)
                    .context("truncated reviewer commit header")?;
                commits.push(Commit {
                    author: (*author).into(),
                    paths: Vec::new(),
                });
                i += 3;
            }
            "" | "\n" => i += 1,
            field => {
                // Raw metadata and paths occupy separate NUL fields, preserving tabs/newlines.
                if !field.trim_start_matches('\n').starts_with(':') {
                    bail!("invalid Git reviewer raw record");
                }
                let path = fields.get(i + 1).context("truncated reviewer path")?;
                commits
                    .last_mut()
                    .context("reviewer path before commit")?
                    .paths
                    .push((*path).into());
                i += 2;
            }
        }
    }
    Ok(commits)
}

pub(super) fn reviewers(
    root: &Path,
    head: &str,
    doc: &Document,
    report: &mut Impact,
) -> Result<()> {
    let affected: BTreeSet<_> = report
        .affected_modules
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    let mut declared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &doc.files {
        if affected.contains(file.module.as_str()) {
            for owner in &file.owners {
                declared
                    .entry(owner.clone())
                    .or_default()
                    .insert(file.module.clone());
            }
        }
    }
    let owned: BTreeSet<_> = declared.values().flatten().cloned().collect();
    for (identity, modules) in declared {
        report.reviewers.push(Reviewer {
            identity,
            source: "codeowners".into(),
            commits: 0,
            modules: modules.into_iter().collect(),
        });
    }
    let unowned: BTreeSet<_> = affected
        .into_iter()
        .filter(|m| !owned.contains(*m))
        .collect();
    if unowned.is_empty() {
        return Ok(());
    }
    if git::is_shallow(root)? {
        report
            .limitations
            .push("Reviewer history is incomplete in this shallow clone.".into());
    }
    let timestamp = git::text(root, &["show", "-s", "--format=%ct", head])?;
    let end: i64 = timestamp
        .trim()
        .parse()
        .context("reading head commit date")?;
    let since = format!("--since-as-filter=@{}", end.saturating_sub(365 * 86400));
    let until = format!("--until=@{end}");
    let log = git::run(
        root,
        &[
            "log",
            "--use-mailmap",
            "--format=%x00IMPACT%x00%H%x00%aN <%aE>%x00",
            "--raw",
            "-z",
            "--no-renames",
            "--relative",
            &since,
            &until,
            head,
            "--",
            ".",
        ],
    )?;
    let paths: BTreeMap<_, _> = doc
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.module.as_str()))
        .chain(
            report
                .affected_files
                .iter()
                .map(|f| (f.path.as_str(), f.module.as_str())),
        )
        .collect();
    let rules = archgraph::rules::compile(&doc.config)?;
    let mut counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for commit in parse_commits(&log)? {
        let touched: BTreeSet<_> = commit
            .paths
            .iter()
            .map(|p| {
                paths.get(p.as_str()).map_or_else(
                    || modules::assigned(p, &doc.config, doc.source.language, rules.as_ref()),
                    |m| (*m).to_owned(),
                )
            })
            .filter(|m| unowned.contains(m.as_str()))
            .collect();
        for module in touched {
            *counts
                .entry(module)
                .or_default()
                .entry(commit.author.clone())
                .or_default() += 1;
        }
    }
    for (module, authors) in counts {
        let mut authors: Vec<_> = authors.into_iter().collect();
        authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (identity, commits) in authors.into_iter().take(5) {
            report.reviewers.push(Reviewer {
                identity,
                source: "git".into(),
                commits,
                modules: vec![module.clone()],
            });
        }
    }
    report.reviewers.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(b.commits.cmp(&a.commits))
            .then(a.identity.cmp(&b.identity))
            .then(a.modules.cmp(&b.modules))
    });
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Package {
    #[serde(default)]
    import_path: String,
    #[serde(default)]
    dir: String,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default)]
    for_test: String,
    #[serde(default)]
    test_go_files: Vec<String>,
    #[serde(default)]
    x_test_go_files: Vec<String>,
    #[serde(default)]
    embed_files: Vec<String>,
    #[serde(default)]
    test_embed_files: Vec<String>,
    #[serde(default)]
    x_test_embed_files: Vec<String>,
}

fn select_go_tests(
    root: &Path,
    changed: &[String],
    packages: &[Package],
    excludes: &globset::GlobSet,
    tests: &globset::GlobSet,
) -> (Vec<String>, Vec<String>) {
    let mut changed_packages = BTreeSet::new();
    for p in packages
        .iter()
        .filter(|p| p.for_test.is_empty() && !p.import_path.ends_with(".test"))
    {
        let Ok(dir) = Path::new(&p.dir).strip_prefix(root) else {
            continue;
        };
        if changed.iter().any(|f| {
            Path::new(f).parent() == Some(dir)
                || p.embed_files
                    .iter()
                    .chain(&p.test_embed_files)
                    .chain(&p.x_test_embed_files)
                    .any(|name| dir.join(name) == Path::new(f))
                || matches!(f.as_str(), "go.mod" | "go.sum")
                || (f.ends_with("/go.mod") || f.ends_with("/go.sum"))
                    && Path::new(&p.dir).starts_with(root.join(f).parent().unwrap_or(root))
        }) {
            changed_packages.insert(p.import_path.as_str());
        }
    }
    let mut files = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    for p in packages.iter().filter(|p| {
        p.for_test.is_empty() && (!p.test_go_files.is_empty() || !p.x_test_go_files.is_empty())
    }) {
        let binary = format!("{}.test", p.import_path);
        let selected = packages
            .iter()
            .filter(|b| b.import_path == binary)
            .any(|b| {
                b.deps
                    .iter()
                    .any(|dep| changed_packages.contains(dep.split(" [").next().unwrap_or(dep)))
            })
            || changed_packages.contains(p.import_path.as_str());
        if !selected {
            continue;
        }
        let Ok(dir) = Path::new(&p.dir).strip_prefix(root) else {
            continue;
        };
        for name in p.test_go_files.iter().chain(&p.x_test_go_files) {
            let file = dir.join(name).to_string_lossy().replace('\\', "/");
            if !excludes.is_match(&file) && (name.ends_with("_test.go") || tests.is_match(&file)) {
                files.insert(file);
                dirs.insert(if dir.as_os_str().is_empty() {
                    ".".into()
                } else {
                    format!("./{}", dir.to_string_lossy().replace('\\', "/"))
                });
            }
        }
    }
    (files.into_iter().collect(), dirs.into_iter().collect())
}

pub(super) fn go_tests(
    root: &Path,
    go_program: &str,
    changed: &[String],
    doc: &Document,
    tests: &[String],
    report: &mut Impact,
) -> Result<()> {
    let root = root.canonicalize().context("resolving Go impact root")?;
    let excludes = collect::globset(&doc.config.excludes)?;
    let patterns = collect::globset(tests)?;
    let (modules, warnings) = collect::go::module_dirs(&root, &excludes)?;
    report.limitations.extend(warnings);
    let go_program = if go_program.contains('/') {
        Path::new(go_program)
            .canonicalize()?
            .to_string_lossy()
            .into_owned()
    } else {
        go_program.to_owned()
    };
    let mut packages = Vec::new();
    for module in modules {
        let out = collect::run(
            Command::new(&go_program)
                .args(["list", "-deps", "-test", "-json", "./..."])
                .current_dir(root.join(module))
                .envs(collect::go::ENV.iter().copied())
                .env("GOENV", "off")
                .env("GOCACHEPROG", ""),
        )
        .context("running go list for impact tests")?;
        if !out.status.success() {
            bail!(
                "Go test dependency selection failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        for p in serde_json::Deserializer::from_slice(&out.stdout).into_iter::<Package>() {
            packages.push(p.context("reading go list test packages")?);
        }
    }
    let (files, dirs) = select_go_tests(&root, changed, &packages, &excludes, &patterns);
    report.selected_tests.extend(files);
    report.selected_tests.sort();
    report.selected_tests.dedup();
    report.go_test_packages = dirs;
    report.limitations.push("Go test dependencies reflect the current platform with cgo disabled and cached dependencies; test code was not executed.".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_raw_paths_preserve_delimiters_and_deduplicate_later() {
        let commits = parse_commits(b"\0IMPACT\0abc\0A <a@example.com>\0\n:100644 100644 abc def M\0dir/a\tb\nc.rs\0\0IMPACT\0def\0B <b@example.com>\0\n:000000 100644 000 abc A\0IMPACT\0").unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].paths, ["dir/a\tb\nc.rs"]);
        assert_eq!(commits[1].paths, ["IMPACT"]);
        assert_eq!(commits[1].author, "B <b@example.com>");
    }

    #[test]
    fn go_test_binary_includes_external_tests_beyond_graph_depth() {
        let packages: Vec<Package> = serde_json::Deserializer::from_str(r#"{"ImportPath":"m/lib","Dir":"/repo/lib"}
{"ImportPath":"m/app","Dir":"/repo/app","TestGoFiles":["app_test.go"],"XTestGoFiles":["external_test.go"]}
{"ImportPath":"m/app.test","Deps":["m/app [m/app.test]","m/lib"]}
{"ImportPath":"m/unrelated","Dir":"/repo/unrelated","TestGoFiles":["unrelated_test.go"]}
{"ImportPath":"m/unrelated.test","Deps":["m/unrelated"]}"#).into_iter().map(Result::unwrap).collect();
        let empty = collect::globset(&[]).unwrap();
        let (files, dirs) = select_go_tests(
            Path::new("/repo"),
            &["lib/lib.go".into()],
            &packages,
            &empty,
            &empty,
        );
        assert_eq!(files, ["app/app_test.go", "app/external_test.go"]);
        assert_eq!(dirs, ["./app"]);
    }

    #[test]
    fn go_test_only_changes_embeds_and_custom_globs_keep_conventions() {
        let packages: Vec<Package> = serde_json::Deserializer::from_str(r#"{"ImportPath":"m/app","Dir":"/repo/app","TestGoFiles":["app_test.go"],"XTestGoFiles":["external_test.go"],"TestEmbedFiles":["fixtures/input.txt"]}
{"ImportPath":"m/app.test","Deps":["m/app [m/app.test]"]}"#).into_iter().map(Result::unwrap).collect();
        let excludes = collect::globset(&["**/external_test.go".into()]).unwrap();
        let custom = collect::globset(&["**/*.spec.ts".into()]).unwrap();
        for changed in ["app/external_test.go", "app/fixtures/input.txt"] {
            let (files, dirs) = select_go_tests(
                Path::new("/repo"),
                &[changed.into()],
                &packages,
                &excludes,
                &custom,
            );
            assert_eq!(files, ["app/app_test.go"]);
            assert_eq!(dirs, ["./app"]);
        }
    }
}
