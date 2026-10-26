//! Exercise the real CLI transport, watcher and stale-discovery recovery in isolated checkouts.
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let temp = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let root = temp.join(format!(
            "boundrail-daemon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src/a")).unwrap();
        fs::create_dir_all(root.join("src/b")).unwrap();
        fs::create_dir_all(root.join(".boundrail")).unwrap();
        fs::write(
            root.join("package.json"),
            "{\"name\":\"daemon-fixture\",\"type\":\"module\"}\n",
        )
        .unwrap();
        fs::write(root.join("layout.yaml"), "version: 1\n").unwrap();
        fs::write(
            root.join("src/a/a.ts"),
            "import '../b/b';\nexport const a = 1;\n",
        )
        .unwrap();
        fs::write(root.join("src/b/b.ts"), "export const b = 1;\n").unwrap();
        Self { root, child: None }
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_boundrail"))
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let out = self.command(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
    }
    fn start(&mut self, extra: &[&str]) {
        let previous_endpoint = fs::read(self.root.join(".boundrail/daemon.json")).ok();
        let log = fs::File::create(self.root.join(".boundrail/server.log")).unwrap();
        self.child = Some(
            Command::new(env!("CARGO_BIN_EXE_boundrail"))
                .args([
                    "serve",
                    ".",
                    "--engine",
                    "native",
                    "--language",
                    "typescript",
                    "--rules",
                    "layout.yaml",
                ])
                .args(extra)
                .current_dir(&self.root)
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        loop {
            if fs::read(self.root.join(".boundrail/daemon.json"))
                .ok()
                .is_some_and(|bytes| Some(bytes) != previous_endpoint)
            {
                break;
            }
            let failed = self.child.as_mut().unwrap().try_wait().unwrap();
            assert!(
                failed.is_none(),
                "serve exited {failed:?}: {}",
                fs::read_to_string(self.root.join(".boundrail/server.log")).unwrap()
            );
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "serve readiness timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
    fn wait_cycle_count(&self, expected: usize) -> Duration {
        let start = Instant::now();
        loop {
            let value = self.json(&["--brief", "check", ".boundrail/daemon-graph.json"]);
            if value["result"]["cycles"].as_array().unwrap().len() == expected {
                return start.elapsed();
            }
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "cycle update exceeded 1 second: {value}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn watched_cycle_revert_and_standalone_bytes_match() {
    let mut f = Fixture::new();
    f.start(&["--port"]);
    assert_eq!(f.json(&["status"])["snapshot_limit"], 2);
    f.wait_cycle_count(0);
    let warm_scan = f.command(&[
        "--brief",
        "scan",
        ".",
        "--engine",
        "native",
        "--rules",
        "layout.yaml",
    ]);
    let cold_scan = f.command(&[
        "--no-daemon",
        "--brief",
        "scan",
        ".",
        "--engine",
        "native",
        "--rules",
        "layout.yaml",
    ]);
    assert!(
        warm_scan.status.success(),
        "{}",
        String::from_utf8_lossy(&warm_scan.stderr)
    );
    assert_eq!(warm_scan.stdout, cold_scan.stdout);
    fs::write(
        f.root.join("src/b/b.ts"),
        "import '../a/a';\nexport const b = 1;\n",
    )
    .unwrap();
    let changed = f.wait_cycle_count(1);
    assert!(changed < Duration::from_secs(1));
    let status = f.json(&["status"]);
    assert!(status["generation"].as_u64().unwrap() >= 2);
    assert!(status["parsed_files"].as_u64().unwrap() <= 1, "{status}");
    assert_eq!(status["cached_snapshots"], 2);
    let warm = f.command(&["--brief", "check", ".boundrail/daemon-graph.json"]);
    let cold = f.command(&[
        "--no-daemon",
        "--brief",
        "check",
        ".boundrail/daemon-graph.json",
    ]);
    assert_eq!(warm.status.code(), cold.status.code());
    assert_eq!(warm.stdout, cold.stdout);
    assert_eq!(warm.stderr, cold.stderr);
    fs::write(f.root.join("src/b/b.ts"), "export const b = 1;\n").unwrap();
    assert!(f.wait_cycle_count(0) < Duration::from_secs(1));
    assert_eq!(f.json(&["status"])["snapshot_limit"], 2);
}

#[cfg(unix)]
#[test]
fn killed_socket_falls_back_and_a_new_server_reclaims_it() {
    let mut f = Fixture::new();
    let socket = f.root.join("test.sock");
    f.start(&["--socket", socket.to_str().unwrap()]);
    assert_eq!(f.json(&["status"])["running"], true);
    f.kill();
    assert!(
        socket.exists(),
        "SIGKILL must leave a stale socket for this test"
    );
    assert_eq!(f.json(&["status"])["running"], false);
    f.json(&["--brief", "check", ".boundrail/daemon-graph.json"]);
    // Existing socket is intentionally retained to exercise safe stale socket replacement.
    f.start(&["--socket", socket.to_str().unwrap()]);
    assert_eq!(f.json(&["status"])["running"], true);
    f.kill();
    fs::remove_file(socket).unwrap();
}

#[test]
fn once_answers_one_tcp_request_and_exits() {
    let mut f = Fixture::new();
    f.start(&["--port=0", "--once"]);
    let endpoint: Value =
        serde_json::from_slice(&fs::read(f.root.join(".boundrail/daemon.json")).unwrap()).unwrap();
    assert!(endpoint["port"].as_u64().unwrap() > 0);
    assert_eq!(f.json(&["status"])["running"], true);
    let start = Instant::now();
    loop {
        if let Some(status) = f.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "--once did not exit after request"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    f.child = None;
    assert!(!f.root.join(".boundrail/daemon.json").exists());
    assert_eq!(f.json(&["status"])["running"], false);
}

#[cfg(unix)]
#[test]
fn delayed_partial_socket_frames_are_read_before_authentication() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut f = Fixture::new();
    let socket = f.root.join("partial.sock");
    f.start(&["--socket", socket.to_str().unwrap()]);
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // An accepted nonblocking macOS stream used to close before the client wrote its frame.
    std::thread::sleep(Duration::from_millis(20));
    let request = serde_json::to_vec(
        &serde_json::json!({"jsonrpc":"2.0","id":77,"method":"command",
        "params":{"token":"incorrect","args":["status"],"cwd":f.root,"env":{},"tty":false}}),
    )
    .unwrap();
    let header = (request.len() as u32).to_be_bytes();
    stream.write_all(&header[..2]).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    stream.write_all(&header[2..]).unwrap();
    stream.write_all(&request[..request.len() / 2]).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    stream.write_all(&request[request.len() / 2..]).unwrap();
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let mut data = vec![0; u32::from_be_bytes(length) as usize];
    stream.read_exact(&mut data).unwrap();
    let response: Value = serde_json::from_slice(&data).unwrap();
    assert_eq!(response["id"], 77);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid local daemon request")
    );
    assert!(response.get("result").is_none());
    let args = [
        "--brief",
        "scan",
        ".",
        "--engine",
        "native",
        "--rules",
        "layout.yaml",
    ];
    let baseline = f.command(&args);
    assert!(baseline.status.success());
    for _ in 0..10 {
        let repeated = f.command(&args);
        assert_eq!(repeated.status.code(), baseline.status.code());
        assert_eq!(repeated.stdout, baseline.stdout);
    }
}

#[test]
fn changed_caller_environment_requires_restart_or_standalone() {
    let mut f = Fixture::new();
    f.start(&["--port"]);
    let changed = Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .arg("status")
        .current_dir(&f.root)
        .env("BOUNDRAIL_DAEMON_TEST_ENV", "caller-only-change")
        .output()
        .unwrap();
    assert!(!changed.status.success());
    let error = String::from_utf8_lossy(&changed.stderr);
    assert!(error.contains("environment differs"), "{error}");
    assert!(error.contains("--no-daemon"), "{error}");
    let standalone = Command::new(env!("CARGO_BIN_EXE_boundrail"))
        .args(["--no-daemon", "status"])
        .current_dir(&f.root)
        .env("BOUNDRAIL_DAEMON_TEST_ENV", "caller-only-change")
        .output()
        .unwrap();
    assert!(standalone.status.success());
    assert_eq!(f.json(&["status"])["running"], true);
}
