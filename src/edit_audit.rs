//! Locked JSONL evidence for edit blocks and reasoned overrides.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::edit_gate::Violation;

fn timestamp() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn checked_path(root: &Path, name: &str, create: bool) -> Result<PathBuf> {
    ensure!(
        root.is_dir(),
        "audit root is not a directory: {}",
        root.display()
    );
    let directory = root.join(".boundrail");
    if create {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.is_symlink(),
            "audit directory must be a real directory: {}",
            directory.display()
        ),
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(directory.join(name));
        }
        Err(error) => return Err(error.into()),
    }
    let path = directory.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !metadata.is_symlink(),
            "audit log must be a regular file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

fn records(file: &mut File, path: &Path) -> Result<Vec<Value>> {
    file.seek(SeekFrom::Start(0))?;
    let mut body = String::new();
    file.read_to_string(&mut body)?;
    body.lines()
        .enumerate()
        .map(|(index, line)| {
            let record: Value = serde_json::from_str(line).with_context(|| {
                format!("malformed audit record {}:{}", path.display(), index + 1)
            })?;
            ensure!(
                record.is_object(),
                "audit record must be an object at {}:{}",
                path.display(),
                index + 1
            );
            ensure!(
                record.get("ts").and_then(Value::as_u64).is_some(),
                "audit record is missing timestamp at {}:{}",
                path.display(),
                index + 1
            );
            for field in ["agent", "snapshot"] {
                ensure!(
                    record.get(field).and_then(Value::as_str).is_some(),
                    "audit record is missing {field} at {}:{}",
                    path.display(),
                    index + 1
                );
            }
            if path.file_name().is_some_and(|name| name == "fires.log") {
                for field in ["rule", "file", "finding_id"] {
                    ensure!(
                        record.get(field).and_then(Value::as_str).is_some(),
                        "fire record is missing {field} at {}:{}",
                        path.display(),
                        index + 1
                    );
                }
            } else {
                ensure!(
                    record
                        .get("reason")
                        .and_then(Value::as_str)
                        .is_some_and(|reason| !reason.trim().is_empty()),
                    "override record has no reason at {}:{}",
                    path.display(),
                    index + 1
                );
                ensure!(
                    record
                        .get("finding_ids")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| ids.iter().all(Value::is_string)),
                    "override record has invalid finding_ids at {}:{}",
                    path.display(),
                    index + 1
                );
            }
            Ok(record)
        })
        .collect()
}

fn verify_open_file(root: &Path, name: &str, file: &File) -> Result<()> {
    let path = checked_path(root, name, false)?;
    let opened = file.metadata()?;
    ensure!(opened.is_file(), "audit log must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current = fs::metadata(&path)?;
        ensure!(
            opened.dev() == current.dev() && opened.ino() == current.ino(),
            "audit log changed while opening: {}",
            path.display()
        );
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn append(root: &Path, name: &str, entries: &[Value]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let path = checked_path(root, name, true)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            OpenOptions::new().read(true).write(true).open(&path)?
        }
        Err(error) => return Err(error.into()),
    };
    file.lock()?;
    // Recheck after opening; never write through an observed symlink or special file.
    verify_open_file(root, name, &file)?;
    records(&mut file, &path)?;
    let end = file.seek(SeekFrom::End(0))?;
    if end > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        file.seek(SeekFrom::End(0))?;
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
        }
    }
    let mut bytes = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut bytes, entry)?;
        bytes.push(b'\n');
    }
    file.write_all(&bytes)?;
    file.sync_data()?;
    Ok(())
}

/// Append all violations from one blocked edit under one exclusive file lock.
pub fn fires(root: &Path, agent: &str, snapshot: &str, violations: &[Violation]) -> Result<()> {
    let ts = timestamp()?;
    let entries: Vec<_> = violations
        .iter()
        .map(|violation| {
            json!({
                "rule": violation.rule, "file": violation.file, "agent": agent, "ts": ts,
                "finding_id": violation.finding_id, "snapshot": snapshot,
            })
        })
        .collect();
    append(root, "fires.log", &entries)
}

/// Record an explicitly reasoned bypass; consuming single-use overrides is the caller's job.
pub fn override_event(
    root: &Path,
    agent: &str,
    snapshot: &str,
    reason: &str,
    finding_ids: &[String],
) -> Result<()> {
    let reason = reason.trim();
    ensure!(!reason.is_empty(), "override requires a nonempty reason");
    append(
        root,
        "overrides.log",
        &[json!({
            "agent": agent, "snapshot": snapshot, "reason": reason,
            "finding_ids": finding_ids, "ts": timestamp()?,
        })],
    )
}

/// Count recorded blocks by rule, highest count first and lexical order for ties.
pub fn top(root: &Path) -> Result<Value> {
    let path = checked_path(root, "fires.log", false)?;
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"total": 0, "rules": []}));
        }
        Err(error) => return Err(error.into()),
    };
    file.lock_shared()?;
    verify_open_file(root, "fires.log", &file)?;
    let entries = records(&mut file, &path)?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        *counts
            .entry(
                entry["rule"]
                    .as_str()
                    .context("fire rule must be text")?
                    .into(),
            )
            .or_default() += 1;
    }
    let mut rules: Vec<_> = counts.into_iter().collect();
    rules.sort_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
    Ok(
        json!({"total": entries.len(), "rules": rules.into_iter().map(|(rule, count)| json!({"rule": rule, "count": count})).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "boundrail-edit-audit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn violation(rule: &str) -> Violation {
        Violation {
            finding_id: format!("finding-{rule}"),
            kind: "forbidden-edge".into(),
            rule: rule.into(),
            file: "src/domain.ts".into(),
            recipe: json!({}),
        }
    }

    #[test]
    fn counts_rules_and_preserves_audit_identity() {
        let fixture = Fixture::new();
        assert_eq!(top(&fixture.0).unwrap(), json!({"total":0,"rules":[]}));
        fires(
            &fixture.0,
            "codex",
            "snapshot-a",
            &[violation("z"), violation("a"), violation("z")],
        )
        .unwrap();
        fires(&fixture.0, "claude", "snapshot-b", &[violation("b")]).unwrap();
        assert_eq!(
            top(&fixture.0).unwrap(),
            json!({"total":4,"rules":[{"rule":"z","count":2},{"rule":"a","count":1},{"rule":"b","count":1}]})
        );
        let first: Value = serde_json::from_str(
            fs::read_to_string(fixture.0.join(".boundrail/fires.log"))
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first["snapshot"], "snapshot-a");
        assert_eq!(first["finding_id"], "finding-z");
        assert!(first["ts"].as_u64().unwrap() > 0);
    }

    #[test]
    fn requires_reason_and_escapes_newlines() {
        let fixture = Fixture::new();
        assert!(override_event(&fixture.0, "codex", "s", " \n", &[]).is_err());
        override_event(
            &fixture.0,
            "codex",
            "s",
            " temporary\nexception ",
            &["id".into()],
        )
        .unwrap();
        let body = fs::read_to_string(fixture.0.join(".boundrail/overrides.log")).unwrap();
        assert_eq!(body.lines().count(), 1);
        let record: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(record["reason"], "temporary\nexception");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(fixture.0.join(".boundrail/overrides.log"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn refuses_corruption_without_appending() {
        let fixture = Fixture::new();
        fires(&fixture.0, "codex", "s", &[violation("a")]).unwrap();
        let path = fixture.0.join(".boundrail/fires.log");
        for malformed in ["not json\n", "{}\n", "\n", "[]\n"] {
            fs::write(&path, malformed).unwrap();
            assert!(top(&fixture.0).is_err());
            assert!(fires(&fixture.0, "codex", "s", &[violation("a")]).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), malformed);
        }
    }

    #[test]
    fn appends_after_valid_record_without_final_newline() {
        let fixture = Fixture::new();
        fires(&fixture.0, "codex", "s", &[violation("a")]).unwrap();
        let path = fixture.0.join(".boundrail/fires.log");
        let body = fs::read_to_string(&path).unwrap();
        fs::write(&path, body.trim_end()).unwrap();
        fires(&fixture.0, "codex", "s", &[violation("b")]).unwrap();
        assert_eq!(top(&fixture.0).unwrap()["total"], 2);
    }

    #[test]
    fn concurrent_handles_append_complete_records() {
        let fixture = Fixture::new();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let root = &fixture.0;
                scope.spawn(move || {
                    for _ in 0..10 {
                        fires(root, "codex", "s", &[violation("a"), violation("b")]).unwrap();
                    }
                });
            }
        });
        assert_eq!(top(&fixture.0).unwrap()["total"], 160);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_directory_and_log() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        symlink(&outside.0, fixture.0.join(".boundrail")).unwrap();
        assert!(fires(&fixture.0, "codex", "s", &[violation("a")]).is_err());
        assert!(top(&fixture.0).is_err());
        fs::remove_file(fixture.0.join(".boundrail")).unwrap();
        fs::create_dir(fixture.0.join(".boundrail")).unwrap();
        fs::write(outside.0.join("secret"), "untouched").unwrap();
        symlink(
            outside.0.join("secret"),
            fixture.0.join(".boundrail/fires.log"),
        )
        .unwrap();
        assert!(fires(&fixture.0, "codex", "s", &[violation("a")]).is_err());
        assert!(top(&fixture.0).is_err());
        assert_eq!(
            fs::read_to_string(outside.0.join("secret")).unwrap(),
            "untouched"
        );
    }

    #[test]
    fn rejects_directory_log() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join(".boundrail/fires.log")).unwrap();
        assert!(fires(&fixture.0, "codex", "s", &[violation("a")]).is_err());
        assert!(top(&fixture.0).is_err());
    }
}
