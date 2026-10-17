//! Repository CODEOWNERS discovery and local ownership matching; no GitHub permission checks.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};

const MAX_BYTES: usize = 3 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Codeowners {
    pub path: String,
    pub prefix: String,
    pub rules: Vec<OwnerRule>,
    pub warnings: Vec<OwnerWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerRule {
    pub pattern: String,
    pub owners: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerWarning {
    pub line: usize,
    pub message: String,
}

/// First existing CODEOWNERS location wins, including when it cannot be read.
pub fn load(root: &Path) -> Result<Option<Codeowners>> {
    let root = fs::canonicalize(root).context("resolving CODEOWNERS scan root")?;
    let git = Command::new("git")
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(&root)
        .args(["rev-parse", "--show-toplevel"])
        .output();
    let repo = match git {
        Ok(out) if out.status.success() => {
            let path = String::from_utf8(out.stdout).context("Git root is not UTF-8")?;
            fs::canonicalize(path.trim_end_matches(['\n', '\r']))?
        }
        _ => root.clone(),
    };
    let prefix = root
        .strip_prefix(&repo)
        .context("scan root outside Git root")?;
    let prefix = prefix
        .to_str()
        .context("CODEOWNERS scan root is not UTF-8")?
        .replace('\\', "/");
    let prefix = if prefix.is_empty() {
        "."
    } else {
        prefix.as_str()
    };
    for location in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
        let path = repo.join(location);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {location}")),
            Ok(_) => {}
        }
        if !fs::canonicalize(&path)
            .with_context(|| format!("resolving {location}"))?
            .starts_with(&repo)
        {
            bail!("{location} points outside the repository");
        }
        if !fs::metadata(&path)
            .with_context(|| format!("reading {location}"))?
            .is_file()
        {
            bail!("{location} is not a regular file");
        }
        let file = File::open(&path).with_context(|| format!("opening {location}"))?;
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading {location}"))?;
        if bytes.len() > MAX_BYTES {
            bail!("{location} exceeds the 3 MiB CODEOWNERS limit");
        }
        let text = String::from_utf8(bytes).with_context(|| format!("{location} is not UTF-8"))?;
        return Ok(Some(parse(location, prefix, &text)));
    }
    Ok(None)
}

/// Invalid rules are skipped with line diagnostics; an ownerless rule clears ownership.
pub fn parse(path: &str, prefix: &str, text: &str) -> Codeowners {
    let mut data = Codeowners {
        path: path.to_owned(),
        prefix: prefix.to_owned(),
        rules: Vec::new(),
        warnings: Vec::new(),
    };
    if text.len() > MAX_BYTES {
        data.warnings.push(OwnerWarning {
            line: 0,
            message: "CODEOWNERS exceeds the 3 MiB limit".into(),
        });
        return data;
    }
    for (i, line) in text.lines().enumerate() {
        let number = i + 1;
        let parsed = parse_line(line);
        match parsed {
            Ok(Some((pattern, owners))) => data.rules.push(OwnerRule {
                pattern,
                owners,
                line: number,
            }),
            Ok(None) => {}
            Err(e) => data.warnings.push(OwnerWarning {
                line: number,
                message: e.to_string(),
            }),
        }
    }
    data
}

fn parse_line(line: &str) -> Result<Option<(String, Vec<String>)>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    if line.contains("\\#") {
        bail!("escaped # patterns are unsupported by CODEOWNERS");
    }
    let line = line.split('#').next().unwrap_or_default().trim_end();
    let mut chars = line.chars().peekable();
    let mut pattern = String::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            break;
        }
        if c == '\\' {
            if chars.next() != Some(' ') {
                bail!("only escaped spaces are supported in patterns");
            }
            pattern.push(' ');
        } else {
            pattern.push(c);
        }
    }
    let mut owners: Vec<String> = chars
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    validate_pattern(&pattern)?;
    for owner in &owners {
        validate_owner(owner)?;
    }
    owners.sort();
    owners.dedup();
    Ok(Some((pattern, owners)))
}

fn relative(path: &str, dot: bool) -> Result<()> {
    if dot && path == "." {
        return Ok(());
    }
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("expected a repository-relative path without traversal: {path:?}");
    }
    Ok(())
}

fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern.starts_with('!') || pattern.contains(['[', ']', '{', '}', '#']) {
        bail!(
            "unsupported CODEOWNERS pattern {pattern:?}: negation, ranges and braces are not supported"
        );
    }
    if pattern.contains("//") {
        bail!("empty path component in CODEOWNERS pattern");
    }
    relative(
        pattern
            .strip_prefix('/')
            .unwrap_or(pattern)
            .trim_end_matches('/'),
        false,
    )?;
    if pattern.contains(['\n', '\r', '\t']) {
        bail!("invalid whitespace in CODEOWNERS pattern");
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<()> {
    let valid = if let Some(name) = owner.strip_prefix('@') {
        let parts: Vec<&str> = name.split('/').collect();
        parts.len() <= 2
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            })
    } else {
        owner.split_once('@').is_some_and(|(name, domain)| {
            !name.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
                && !owner
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '/')
        })
    };
    if !valid {
        bail!("invalid CODEOWNERS owner {owner:?}");
    }
    Ok(())
}

pub struct Compiled {
    prefix: String,
    rules: Vec<(Vec<GlobMatcher>, Vec<String>)>,
}

/// Revalidates embedded data as well as parsed files before compiling matchers.
pub fn compile(data: &Codeowners) -> Result<Compiled> {
    relative(&data.path, false).context("CODEOWNERS location")?;
    relative(&data.prefix, true).context("CODEOWNERS prefix")?;
    let mut rules = Vec::new();
    for rule in &data.rules {
        let build = || -> Result<_> {
            validate_pattern(&rule.pattern)?;
            for owner in &rule.owners {
                validate_owner(owner)?;
            }
            let directory = rule.pattern.ends_with('/');
            let stripped = rule
                .pattern
                .strip_prefix('/')
                .unwrap_or(&rule.pattern)
                .trim_end_matches('/');
            let anywhere = !rule.pattern.starts_with('/') && !stripped.contains('/');
            let base = if anywhere {
                format!("**/{stripped}")
            } else {
                stripped.to_owned()
            };
            let mut patterns = if directory {
                Vec::new()
            } else {
                vec![base.clone()]
            };
            // A literal final segment can name a directory; docs/* stays direct.
            if directory
                || stripped
                    .rsplit('/')
                    .next()
                    .is_some_and(|last| !last.contains(['*', '?']))
            {
                patterns.push(format!("{base}/**"));
            }
            let matchers = patterns
                .into_iter()
                .map(|pattern| {
                    GlobBuilder::new(&pattern)
                        .literal_separator(true)
                        .backslash_escape(false)
                        .build()
                        .with_context(|| format!("glob {pattern:?}"))
                        .map(|g| g.compile_matcher())
                })
                .collect::<Result<Vec<_>>>()?;
            let mut owners = rule.owners.clone();
            owners.sort();
            owners.dedup();
            Ok((matchers, owners))
        };
        rules.push(build().with_context(|| format!("{}:{}", data.path, rule.line))?);
    }
    Ok(Compiled {
        prefix: data.prefix.clone(),
        rules,
    })
}

impl Compiled {
    /// Paths are scan-root-relative; malformed or escaping paths have no owners.
    pub fn owners(&self, path: &str) -> Vec<String> {
        if relative(path, false).is_err() {
            return Vec::new();
        }
        let path = if self.prefix == "." {
            path.to_owned()
        } else {
            format!("{}/{path}", self.prefix)
        };
        self.rules
            .iter()
            .rev()
            .find(|(patterns, _)| patterns.iter().any(|p| p.is_match(&path)))
            .map(|(_, owners)| owners.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_final_segments_own_descendants_after_wildcard_directories() {
        let compiled = compile(&parse(
            "CODEOWNERS",
            ".",
            "**/logs @logs-team\npackages/*/config @config-team\ndocs/* @docs-team\n",
        ))
        .unwrap();
        for path in ["logs/a.ts", "src/logs/a.ts", "src/logs/nested/a.ts"] {
            assert_eq!(compiled.owners(path), ["@logs-team"], "{path}");
        }
        assert_eq!(
            compiled.owners("packages/app/config/settings.json"),
            ["@config-team"]
        );
        assert_eq!(compiled.owners("docs/readme.md"), ["@docs-team"]);
        for path in ["src/logstash/a.ts", "docs/nested/readme.md"] {
            assert!(compiled.owners(path).is_empty(), "{path}");
        }
    }

    #[test]
    fn precedence_clearing_anchors_and_direct_wildcards() {
        let data = parse(
            "CODEOWNERS",
            ".",
            "* @all\n*.rs @rust @rust\n/docs/ @docs\ndocs/* @direct\n/apps/ @apps\n/apps/github\n/root.txt @root\n",
        );
        assert!(data.warnings.is_empty());
        let compiled = compile(&data).unwrap();
        for (path, owners) in [
            ("src/main.rs", vec!["@rust"]),
            ("docs/readme.md", vec!["@direct"]),
            ("docs/nested/file.md", vec!["@docs"]),
            ("other/docs/readme.md", vec!["@all"]),
            ("apps/github/src/main.rs", vec![]),
            ("apps/other/main.rs", vec!["@apps"]),
            ("root.txt", vec!["@root"]),
            ("sub/root.txt", vec!["@all"]),
        ] {
            assert_eq!(compiled.owners(path), owners, "{path}");
        }
        let directories = compile(&parse("CODEOWNERS", ".", "apps/ @apps")).unwrap();
        assert!(directories.owners("apps").is_empty());
        assert_eq!(directories.owners("apps/main.rs"), ["@apps"]);
    }

    #[test]
    fn subdirectory_prefix_comments_spaces_double_star_and_question_mark() {
        let data = parse(
            ".github/CODEOWNERS",
            "projects/app",
            "# comment\n/projects/app/src/**/a?.rs @org/team owner@example.org # comment\n/projects/app/a\\ b.txt @spaces\n",
        );
        assert!(data.warnings.is_empty());
        let compiled = compile(&data).unwrap();
        assert_eq!(
            compiled.owners("src/a1.rs"),
            ["@org/team", "owner@example.org"]
        );
        assert_eq!(
            compiled.owners("src/nested/a2.rs"),
            ["@org/team", "owner@example.org"]
        );
        assert_eq!(compiled.owners("a b.txt"), ["@spaces"]);
        for path in [
            "src/ab1.rs",
            "src/A1.rs",
            "missing",
            "../src/a1.rs",
            "/src/a1.rs",
            "C:/src/a1.rs",
            "src\\a1.rs",
        ] {
            assert!(compiled.owners(path).is_empty(), "{path}");
        }
    }

    #[test]
    fn invalid_rules_warn_without_shadowing_valid_owners() {
        let data = parse(
            "CODEOWNERS",
            ".",
            "* @valid\n!secret @bad\n[a-z] @bad\n../escape @bad\nC:/foo @bad\nx nobody\ny @\n\\#secret @bad\nz @org/team/extra\n",
        );
        assert_eq!(data.rules.len(), 1);
        assert_eq!(data.warnings.len(), 8);
        assert_eq!(data.warnings[0].line, 2);
        assert_eq!(compile(&data).unwrap().owners("secret"), ["@valid"]);
    }

    #[test]
    fn embedded_data_is_revalidated() {
        for prefix in ["..", "/tmp", "C:/tmp", "a\\b", "a/../b"] {
            assert!(compile(&parse("CODEOWNERS", prefix, "* @owner")).is_err());
        }
        let mut data = parse("CODEOWNERS", ".", "* @owner");
        data.rules[0].pattern = "../outside".into();
        assert!(format!("{:#}", compile(&data).err().unwrap()).contains("CODEOWNERS:1"));
        data.rules[0].pattern = "*".into();
        data.rules[0].owners = vec!["not an owner".into()];
        assert!(compile(&data).is_err());
        assert!(
            parse("CODEOWNERS", ".", &"x".repeat(MAX_BYTES + 1))
                .rules
                .is_empty()
        );
    }

    #[test]
    fn nongit_loader_refuses_invalid_selected_file_without_falling_back() {
        let root = std::env::temp_dir().join(format!(
            "archgraph-codeowners-nongit-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join(".github")).unwrap();
        fs::write(root.join("CODEOWNERS"), "* @z @a @z").unwrap();
        let data = load(&root).unwrap().unwrap();
        assert_eq!(data.prefix, ".");
        assert_eq!(compile(&data).unwrap().owners("a.rs"), ["@a", "@z"]);
        fs::write(root.join(".github/CODEOWNERS"), [0xff]).unwrap();
        assert!(load(&root).unwrap_err().to_string().contains("not UTF-8"));
        fs::remove_file(root.join(".github/CODEOWNERS")).unwrap();
        fs::create_dir(root.join(".github/CODEOWNERS")).unwrap();
        assert!(
            load(&root)
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loader_honors_precedence_and_git_root_for_nested_scans() {
        let root =
            std::env::temp_dir().join(format!("archgraph-codeowners-{}", std::process::id()));
        fs::create_dir_all(root.join(".github")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("project/src")).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join("CODEOWNERS"), "* @root").unwrap();
        fs::write(root.join("docs/CODEOWNERS"), "* @docs").unwrap();
        fs::write(root.join(".github/CODEOWNERS"), "/project/ @github").unwrap();
        let data = load(&root.join("project")).unwrap().unwrap();
        assert_eq!(data.path, ".github/CODEOWNERS");
        assert_eq!(data.prefix, "project");
        assert_eq!(compile(&data).unwrap().owners("src/a.rs"), ["@github"]);
        fs::remove_file(root.join(".github/CODEOWNERS")).unwrap();
        assert_eq!(load(&root).unwrap().unwrap().path, "CODEOWNERS");
        fs::remove_file(root.join("CODEOWNERS")).unwrap();
        assert_eq!(load(&root).unwrap().unwrap().path, "docs/CODEOWNERS");
        fs::remove_file(root.join("docs/CODEOWNERS")).unwrap();
        assert!(load(&root).unwrap().is_none());
        fs::write(root.join("CODEOWNERS"), vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(load(&root).unwrap_err().to_string().contains("3 MiB"));
        fs::remove_dir_all(&root).unwrap();
    }
}
