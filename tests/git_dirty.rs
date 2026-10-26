use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use boundrail::git::{has_tracked_changes, run};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn tracked_dirty_proof_compares_content_without_refreshing_the_index() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "archgraph-git-dirty-{}-{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    )));
    let root = &fixture.0;
    fs::create_dir_all(root).unwrap();
    let file = root.join("[source].ts");
    fs::write(&file, "export const value = 1;\n").unwrap();
    fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
    run(root, &["init", "--quiet", "--template="]).unwrap();
    run(root, &["add", "."]).unwrap();
    run(
        root,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    )
    .unwrap();
    let index = root.join(".git/index");
    let original_index = fs::read(&index).unwrap();
    let original_time = fs::metadata(&index).unwrap().modified().unwrap();
    let path = vec!["[source].ts".into()];

    // A clean file whose stat differs from the index reproduces the old quiet-diff false proof.
    let changed_time = fs::metadata(&file).unwrap().modified().unwrap() + Duration::from_secs(3);
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(changed_time))
        .unwrap();
    assert!(!has_tracked_changes(root, &path).unwrap());
    fs::write(&file, "export const value = 2;\n").unwrap();
    assert!(has_tracked_changes(root, &path).unwrap());
    fs::write(&file, "export const value = 1;\n").unwrap();
    assert!(!has_tracked_changes(root, &path).unwrap());
    fs::write(root.join("untracked.ts"), "untracked").unwrap();
    assert!(!has_tracked_changes(root, &["untracked.ts".into()]).unwrap());
    assert!(!has_tracked_changes(root, &[]).unwrap());
    fs::File::options()
        .write(true)
        .open(root.join("binary.dat"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(changed_time))
        .unwrap();
    assert!(!has_tracked_changes(root, &["binary.dat".into()]).unwrap());
    fs::write(root.join("binary.dat"), [0, 1, 3]).unwrap();
    assert!(!has_tracked_changes(root, &["binary.dat".into()]).unwrap());
    assert!(!run(root, &["status", "--porcelain"]).unwrap().is_empty());
    assert_eq!(fs::read(&index).unwrap(), original_index);
    assert_eq!(
        fs::metadata(&index).unwrap().modified().unwrap(),
        original_time
    );

    run(root, &["add", "binary.dat"]).unwrap();
    assert!(!has_tracked_changes(root, &["binary.dat".into()]).unwrap());
    // Binary changes defer to full status, including staged edits with worktree content restored.
    fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
    assert!(!has_tracked_changes(root, &["binary.dat".into()]).unwrap());
    assert!(!run(root, &["status", "--porcelain"]).unwrap().is_empty());

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::create_dir(root.join("data")).unwrap();
        let non_utf8 = root
            .join("data")
            .join(std::ffi::OsString::from_vec(vec![0xff]));
        fs::write(&non_utf8, "value = 1\n").unwrap();
        run(root, &["add", "data"]).unwrap();
        run(
            root,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "binary path",
            ],
        )
        .unwrap();
        fs::write(&non_utf8, "value = 2\n").unwrap();
        assert!(has_tracked_changes(root, &["data".into()]).unwrap());
    }
}
