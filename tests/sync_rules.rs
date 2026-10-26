use boundrail::sync_rules::{self, FILES};
use std::fs;
use std::path::Path;
use std::process::Command;

const LAYOUT: &str = "version: 1\nlayer-reason: Keep foundations independent\nlayers: ['src/domain/**', 'src/adapters/**']\nmodules: [{id: domain, path: 'src/domain/**'}]\nentrypoints: {'src/domain/**': 'src/domain/index.ts'}\npublic: [{module: 'src/domain/**', entries: ['src/domain/index.ts']}]\npkg-category: {db: [typeorm]}\nforbid: [{id: domain-no-db, from: 'src/domain/**', to: 'category:db', reason: 'Domain excludes database drivers'}]\n";
fn fixture() -> boundrail::collect::TempDir {
    let dir = boundrail::collect::TempDir::new("sync-rules-test").unwrap();
    fs::write(dir.path().join("layout.yaml"), LAYOUT).unwrap();
    dir
}
fn contents(root: &Path) -> Vec<Vec<u8>> {
    FILES
        .iter()
        .map(|file| fs::read(root.join(file)).unwrap())
        .collect()
}

#[test]
fn foreign_bytes_survive_stale_round_trip_and_repeated_sync_is_identical() {
    let dir = fixture();
    let root = dir.path();
    let rules = boundrail::rules::load(&root.join("layout.yaml")).unwrap();
    let original = sync_rules::render(&rules, false);
    let prefix = b"\xffForeign instructions\r\n";
    let suffix = b"\r\nForeign ending\xfe";
    for file in FILES {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            [prefix.as_slice(), original.as_bytes(), suffix.as_slice()].concat(),
        )
        .unwrap();
    }
    fs::write(
        root.join("layout.yaml"),
        format!("{LAYOUT}# changed source\n"),
    )
    .unwrap();
    assert_eq!(sync_rules::run(root, false, true).unwrap().stale.len(), 4);
    let report = sync_rules::run(root, false, false).unwrap();
    assert_eq!(report.written.len(), 4);
    assert!(report.stale.is_empty());
    for data in contents(root) {
        assert!(data.starts_with(prefix));
        assert!(data.ends_with(suffix));
    }
    let before = contents(root);
    assert!(
        sync_rules::run(root, false, false)
            .unwrap()
            .written
            .is_empty()
    );
    assert_eq!(before, contents(root));
    assert!(sync_rules::run(root, false, true).unwrap().stale.is_empty());
}

#[test]
fn hand_edited_block_refuses_all_writes_and_reports_a_diff() {
    let dir = fixture();
    let root = dir.path();
    sync_rules::run(root, false, false).unwrap();
    let path = root.join("AGENTS.md");
    let old = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        old.replace("Keep foundations independent", "Hand edit"),
    )
    .unwrap();
    fs::write(
        root.join("layout.yaml"),
        format!("{LAYOUT}# changed source\n"),
    )
    .unwrap();
    let before = contents(root);
    let report = sync_rules::run(root, false, false).unwrap();
    assert!(report.written.is_empty());
    assert_eq!(report.conflicts.len(), 1);
    assert!(report.conflicts[0].diff.contains("--- AGENTS.md"));
    assert!(report.conflicts[0].diff.contains("Hand edit"));
    assert_eq!(before, contents(root));
}

#[test]
fn brief_budget_is_explicit_full_mode_survives_check_and_rule_sections_are_present() {
    let dir = fixture();
    let root = dir.path();
    let rules = boundrail::rules::load(&root.join("layout.yaml")).unwrap();
    let body = sync_rules::render(&rules, false);
    for expected in [
        "foundation first",
        "src/domain/index.ts",
        "Package category db",
        "Domain excludes database drivers",
        "MCP rules/explain",
    ] {
        assert!(body.contains(expected), "missing {expected}: {body}");
    }
    let mut large = rules.clone();
    large.doc.forbid = (0..100)
        .map(|index| boundrail::rules::Forbid {
            id: Some(format!("rule-{index}")),
            from: format!("src/domain/f{index}.ts"),
            to: "src/adapters/**".into(),
            reason: Some("Keep boundary responsibilities separate".into()),
            ..Default::default()
        })
        .collect();
    assert!(sync_rules::render(&large, false).len().div_ceil(4) <= 600);
    assert!(sync_rules::render(&large, false).contains("omitted rule lines"));
    assert!(sync_rules::render(&large, true).len() > 2400);
    sync_rules::run(root, true, false).unwrap();
    assert!(sync_rules::run(root, false, true).unwrap().stale.is_empty());
    sync_rules::run(root, false, false).unwrap();
    assert_eq!(sync_rules::run(root, true, true).unwrap().stale.len(), 4);
}

#[cfg(unix)]
#[test]
fn symlink_parent_preflight_preserves_outside_files_and_private_modes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = fixture();
    let outside = fixture();
    let protected = outside.path().join("copilot-instructions.md");
    fs::write(&protected, "private outside").unwrap();
    symlink(outside.path(), dir.path().join(".github")).unwrap();
    assert!(sync_rules::run(dir.path(), false, false).is_err());
    assert!(!dir.path().join("CLAUDE.md").exists());
    assert_eq!(fs::read_to_string(protected).unwrap(), "private outside");
    fs::remove_file(dir.path().join(".github")).unwrap();
    fs::write(dir.path().join("AGENTS.md"), "private foreign content").unwrap();
    fs::set_permissions(
        dir.path().join("AGENTS.md"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    sync_rules::run(dir.path(), false, false).unwrap();
    assert_eq!(
        fs::metadata(dir.path().join("AGENTS.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn duplicate_fences_fail_without_changing_any_file() {
    let dir = fixture();
    sync_rules::run(dir.path(), false, false).unwrap();
    let path = dir.path().join("AGENTS.md");
    let old = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{old}{old}")).unwrap();
    let before = contents(dir.path());
    assert!(sync_rules::run(dir.path(), false, false).is_err());
    assert_eq!(before, contents(dir.path()));
}

#[test]
fn cli_check_reports_staleness_and_live_check_warns_without_changing_policy_exit() {
    let dir = fixture();
    let root = dir.path();
    let command = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    assert_eq!(command(&["sync-rules", "--check"]).status.code(), Some(1));
    assert!(command(&["sync-rules"]).status.success());
    assert!(command(&["sync-rules", "--check"]).status.success());
    fs::write(root.join("layout.yaml"), format!("{LAYOUT}# edited\n")).unwrap();
    assert_eq!(command(&["sync-rules", "--check"]).status.code(), Some(1));
    fs::create_dir_all(root.join("src/domain")).unwrap();
    fs::write(root.join("src/domain/a.ts"), "export const a=1;\n").unwrap();
    let checked = command(&["--no-daemon", "check"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stderr).contains("generated agent rules are stale"));
    let explicit = command(&["--no-daemon", "check", ".", "--rules", "layout.yaml"]);
    assert!(explicit.status.success());
    assert!(String::from_utf8_lossy(&explicit.stderr).contains("generated agent rules are stale"));
    let path = root.join("AGENTS.md");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("Keep foundations independent", "Hand edit"),
    )
    .unwrap();
    let refused = command(&["sync-rules"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--- AGENTS.md"));
}
