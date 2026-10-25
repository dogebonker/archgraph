//! Scripted clients exercise the published stdio protocol and the authenticated daemon bridge.
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    daemon: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let temp = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let root = temp.join(format!(
            "archgraph-mcp-stdio-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src/a")).unwrap();
        fs::create_dir_all(root.join("src/b")).unwrap();
        fs::create_dir_all(root.join(".archgraph")).unwrap();
        fs::write(
            root.join("package.json"),
            "{\"name\":\"mcp-fixture\",\"type\":\"module\"}\n",
        )
        .unwrap();
        fs::write(root.join(".gitignore"), ".archgraph/\n").unwrap();
        fs::write(root.join("layout.yaml"),"version: 1\nforbid:\n  - { from: 'src/b/**', to: 'src/a/**', reason: 'keep b independent of a' }\n").unwrap();
        fs::write(
            root.join("src/a/a.ts"),
            "import { b } from '../b/b';\nexport const a = b;\n",
        )
        .unwrap();
        fs::write(root.join("src/b/b.ts"), "export const b = 1;\n").unwrap();
        let f = Self { root, daemon: None };
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=MCP Fixture",
                "-c",
                "user.email=mcp@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "baseline",
            ],
        ] {
            f.git(&args);
        }
        f
    }
    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_boundrail"));
        c.current_dir(&self.root);
        c
    }
    fn options(&self) -> Vec<String> {
        vec![
            ".".into(),
            "--language".into(),
            "typescript".into(),
            "--engine".into(),
            "native".into(),
            "--symbols".into(),
            "--rules".into(),
            self.root.join("layout.yaml").to_string_lossy().into_owned(),
        ]
    }
    fn cycle(&self, enabled: bool) {
        fs::write(
            self.root.join("src/b/b.ts"),
            if enabled {
                "import { a } from '../a/a';\nexport const b = a;\n"
            } else {
                "export const b = 1;\n"
            },
        )
        .unwrap();
    }
    fn services(&self) {
        fs::write(
            self.root.join("package.json"),
            r#"{"dependencies":{"express":"*","pg":"*"}}"#,
        )
        .unwrap();
        for service in ["api", "worker"] {
            fs::create_dir_all(self.root.join(service)).unwrap();
            fs::write(self.root.join(service).join("Dockerfile"), "FROM node\n").unwrap();
        }
        fs::write(
            self.root.join("api/main.ts"),
            "import express from 'express'; const app = express(); app.get('/x', () => {});\n",
        )
        .unwrap();
        self.service_worker("x");
    }
    fn service_worker(&self, path: &str) {
        fs::write(
            self.root.join("worker/main.ts"),
            format!("import {{Pool}} from 'pg'; new Pool(); fetch('http://api/{path}');\n"),
        )
        .unwrap();
    }
    fn start_daemon(&mut self) {
        self.start_daemon_options(self.options());
    }
    fn start_daemon_options(&mut self, options: Vec<String>) {
        self.daemon = Some(
            self.command()
                .arg("serve")
                .args(options)
                .stdout(Stdio::null())
                .stderr(fs::File::create(self.root.join(".archgraph/serve.log")).unwrap())
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while !self.root.join(".archgraph/daemon.json").is_file() {
            assert!(
                self.daemon.as_mut().unwrap().try_wait().unwrap().is_none(),
                "daemon failed: {}",
                fs::read_to_string(self.root.join(".archgraph/serve.log")).unwrap()
            );
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "daemon readiness timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn status(&self) -> Value {
        let out = self.command().arg("status").output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut c) = self.daemon.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Client {
    child: Child,
    input: ChildStdin,
    rx: mpsc::Receiver<String>,
    id: u64,
    schemas: BTreeMap<String, Value>,
}
impl Client {
    fn start(f: &Fixture, standalone: bool) -> Self {
        Self::start_options(f, standalone, f.options(), "2025-11-25")
    }
    fn start_options(f: &Fixture, standalone: bool, options: Vec<String>, protocol: &str) -> Self {
        let mut command = f.command();
        if standalone {
            command.arg("--no-daemon");
        }
        let mut child = command
            .arg("mcp")
            .args(options)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(
                fs::File::create(f.root.join(format!(
                    ".archgraph/mcp-{}.log",
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )))
                .unwrap(),
            )
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut c = Self {
            child,
            input,
            rx,
            id: 0,
            schemas: BTreeMap::new(),
        };
        let init=c.request("initialize",json!({"protocolVersion":protocol,"capabilities":{},"clientInfo":{"name":"archgraph-conformance","version":"1"}}));
        // The SDK advertises 2025-11-25; newer clients negotiate that supported version.
        let negotiated = if protocol == "2026-07-28" {
            "2025-11-25"
        } else {
            protocol
        };
        assert_eq!(init["protocolVersion"], negotiated);
        assert_eq!(init["serverInfo"]["name"], "boundrail");
        c.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        let list = c.request("tools/list", json!({}));
        let tools = list["tools"].as_array().expect("tools/list array");
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            [
                "check_diff",
                "context",
                "explain",
                "propose",
                "review",
                "rules",
                "services",
                "simulate",
                "target",
                "verify"
            ]
        );
        let contracts: Value =
            serde_json::from_str(include_str!("../schema/mcp.v1.schema.json")).unwrap();
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert_eq!(
                contracts["$defs"][format!("{name}_input")],
                tool["inputSchema"],
                "input schema drift: {name}"
            );
            assert_eq!(
                contracts["$defs"][format!("{name}_output")],
                tool["outputSchema"],
                "output schema drift: {name}"
            );
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["properties"]["brief"]["default"], true);
            jsonschema::validator_for(&tool["inputSchema"]).unwrap();
            jsonschema::validator_for(&tool["outputSchema"]).unwrap();
            c.schemas
                .insert(tool["name"].as_str().unwrap().into(), tool.clone());
        }
        c
    }
    fn send(&mut self, value: Value) {
        serde_json::to_writer(&mut self.input, &value).unwrap();
        self.input.write_all(b"\n").unwrap();
        self.input.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let line = self
                .rx
                .recv_timeout(Duration::from_secs(30))
                .expect("MCP response timed out or server stdout closed");
            let response: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("non-protocol stdout {line:?}: {e}"));
            if response.get("id").is_none() {
                continue;
            }
            assert_eq!(response["id"], id);
            assert!(
                response.get("error").is_none(),
                "protocol error: {response}"
            );
            return response["result"].clone();
        }
    }
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let result = self.request("tools/call", json!({"name":name,"arguments":args}));
        let limit = match name {
            "explain" => 300,
            "review" | "services" => 4000,
            _ => 2000,
        };
        assert!(
            serde_json::to_vec(&result).unwrap().len() <= limit,
            "{name} exceeds cap: {result}"
        );
        let envelope = &result["structuredContent"];
        assert!(envelope["truncated"].is_boolean());
        assert!(envelope["omitted"].is_u64());
        if envelope["truncated"] == true {
            assert!(envelope["hint"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(envelope["omitted"].as_u64().unwrap() > 0);
        }
        if let Some(schema) = self.schemas.get(name) {
            let validator = jsonschema::validator_for(&schema["outputSchema"]).unwrap();
            let errors: Vec<_> = validator
                .iter_errors(envelope)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{name}: {errors:?}: {envelope}");
        }
        result
    }
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let result = self.tool(name, args);
        assert_ne!(result["isError"], true, "{name}: {result}");
        result["structuredContent"]["data"].clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn services_stdio_bypasses_collectors_and_layout_and_refreshes_live_source() {
    let f = Fixture::new();
    f.services();
    fs::write(f.root.join("layout.yaml"), "invalid: [unterminated\n").unwrap();
    let options = vec![
        ".".into(),
        "--depcruise".into(),
        f.root
            .join("missing-depcruise")
            .to_string_lossy()
            .into_owned(),
    ];
    let mut c = Client::start_options(&f, true, options, "2025-11-25");
    let all = c.ok("services", json!({"brief":false}));
    assert_eq!(all["source"]["collector"], "inferred");
    assert_eq!(all["scope"]["service"], Value::Null);
    assert_eq!(all["global_summary"]["services"], 2);
    assert_eq!(all["calls"].as_array().unwrap().len(), 1);
    assert_eq!(all["resources"].as_array().unwrap().len(), 1);
    assert_eq!(all, c.ok("services", json!({"brief":false})));

    let worker = c.ok("services", json!({"service":"worker","brief":false}));
    assert_eq!(worker["scope"]["service"], "worker");
    assert_eq!(worker["global_summary"]["services"], 2);
    assert_eq!(worker["services"].as_array().unwrap().len(), 1);
    assert_eq!(worker["services"][0]["id"], "worker");
    assert_eq!(worker["calls"][0]["operation"], "GET /x");
    assert_eq!(worker["resources"][0]["service"], "worker");
    for arguments in [
        json!({"service":"missing"}),
        json!({"service":"../escape"}),
        json!({"manifest":"../escape.yaml"}),
        json!({"brief":"wrong"}),
        json!({"unknown":true}),
    ] {
        assert_eq!(c.tool("services", arguments)["isError"], true);
    }
    f.service_worker("unmatched");
    let refreshed = c.ok("services", json!({"service":"worker","brief":false}));
    assert_eq!(refreshed["global_summary"]["calls"], 0);
    assert!(refreshed["calls"].as_array().unwrap().is_empty());
    assert_eq!(refreshed["resources"].as_array().unwrap().len(), 2);
}

#[test]
fn services_stdio_applies_contained_manifest_rules_and_rejects_escapes() {
    let f = Fixture::new();
    f.services();
    let manifest = json!({"version":1,"services":["api","worker"],"edges":[],"forbid":[
        {"id":"no-worker-api","from":"worker","to":"api"},
        {"id":"no-db","from":"worker","to":"pkg:pg"}
    ]});
    fs::write(f.root.join("services.yaml"), manifest.to_string()).unwrap();
    let mut c = Client::start(&f, true);
    let report = c.ok(
        "services",
        json!({"manifest":"services.yaml","service":"worker","brief":false}),
    );
    assert_eq!(report["source"]["collector"], "inferred+declarative");
    assert_eq!(report["global_summary"]["forbidden_calls"], 1);
    assert_eq!(report["global_summary"]["forbidden_resources"], 1);
    assert_eq!(report["calls"][0]["rule"], "no-worker-api");
    assert_eq!(report["resources"][0]["rule"], "no-db");
    assert_eq!(report["forbid"], manifest["forbid"]);
    assert_eq!(report["policy_source"]["path"], "services.yaml");
    for arguments in [
        json!({"manifest":"services.yaml/../services.yaml"}),
        json!({"manifest":f.root.join("services.yaml").to_str().unwrap()}),
        json!({"manifest":"api"}),
    ] {
        assert_eq!(c.tool("services", arguments)["isError"], true);
    }
    #[cfg(unix)]
    {
        let outside = Fixture::new();
        let secret = "OUTSIDE_MANIFEST_MUST_NOT_BE_READ";
        fs::write(outside.root.join("outside.yaml"), secret).unwrap();
        std::os::unix::fs::symlink(
            outside.root.join("outside.yaml"),
            f.root.join("escape.yaml"),
        )
        .unwrap();
        let escaped = c.tool("services", json!({"manifest":"escape.yaml"}));
        assert_eq!(escaped["isError"], true);
        assert!(!escaped.to_string().contains(secret));
    }
    assert_eq!(c.ok("services", json!({}))["global_summary"]["calls"], 1);
}

#[test]
fn services_stdio_discloses_omitted_report_wide_policy_even_without_matches() {
    let f = Fixture::new();
    f.services();
    let rules: Vec<_> = (0..80)
        .map(|i| json!({"id":format!("unmatched-{i}"),"from":"api","to":"worker","reason":"Keep this authored policy even without observed calls"}))
        .collect();
    fs::write(
        f.root.join("services.yaml"),
        json!({
            "version":1,"services":["api","worker"],"edges":[],"forbid":rules,
        })
        .to_string(),
    )
    .unwrap();
    let mut c = Client::start(&f, true);
    for brief in [true, false] {
        let result = c.tool(
            "services",
            json!({"manifest":"services.yaml","service":"api","brief":brief}),
        );
        assert!(serde_json::to_vec(&result).unwrap().len() <= 4000);
        assert_ne!(result["isError"], true, "{result}");
        let envelope = &result["structuredContent"];
        assert_eq!(envelope["counts"]["forbid"], 80);
        assert_eq!(envelope["truncated"], true);
        assert!(envelope["omitted"].as_u64().unwrap() > 0);
        assert!(envelope["data"]["forbid"].as_array().unwrap().len() < 80);
        assert_eq!(envelope["data"]["policy_source"]["path"], "services.yaml");
        assert!(
            envelope["hint"]
                .as_str()
                .unwrap()
                .contains("--manifest <file> --format json")
        );
    }
}

#[test]
fn services_stdio_caps_large_results_without_losing_scope_or_original_counts() {
    let f = Fixture::new();
    f.services();
    let many = (0..80)
        .map(|i| format!("new Pool(); fetch('http://api/unknown-{i}');\n"))
        .collect::<String>();
    fs::write(
        f.root.join("worker/main.ts"),
        format!("import {{Pool}} from 'pg';\n{many}"),
    )
    .unwrap();
    let mut c = Client::start(&f, true);
    for brief in [true, false] {
        let result = c.tool("services", json!({"service":"worker","brief":brief}));
        assert_ne!(result["isError"], true, "{result}");
        let envelope = &result["structuredContent"];
        assert_eq!(envelope["truncated"], true);
        assert_eq!(envelope["counts"]["resources"], 160);
        assert_eq!(envelope["counts"]["services"], 1);
        assert_eq!(envelope["data"]["scope"]["service"], "worker");
        assert_eq!(envelope["data"]["global_summary"]["services"], 2);
        assert_eq!(envelope["data"]["global_summary"]["resources"], 160);
        assert!(envelope["data"]["resources"].as_array().unwrap().len() < 160);
        assert!(envelope["omitted"].as_u64().unwrap() > 0);
    }
}

#[test]
fn services_mcp_call_does_not_require_an_import_collector() {
    let f = Fixture::new();
    f.services();
    fs::write(f.root.join("layout.yaml"), "broken: [\n").unwrap();
    let output = f
        .command()
        .args([
            "--no-daemon",
            "mcp-call",
            ".",
            "--depcruise",
            "missing-reference-collector",
            "--tool",
            "services",
            "--arguments",
            r#"{"service":"worker"}"#,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["scope"]["service"], "worker");
    assert_eq!(data["global_summary"]["calls"], 1);
    assert_eq!(data["calls"][0]["operation"], "GET /x");
}

#[test]
fn services_stay_fresh_with_a_matching_daemon_and_broken_scan_rules() {
    let mut f = Fixture::new();
    f.services();
    f.start_daemon();
    let mut c = Client::start(&f, false);
    assert_eq!(c.ok("services", json!({}))["global_summary"]["calls"], 1);
    fs::write(f.root.join("layout.yaml"), "broken: [\n").unwrap();
    f.service_worker("unmatched");
    assert_eq!(c.ok("services", json!({}))["global_summary"]["calls"], 0);
    let output = f
        .command()
        .args(["mcp-call"])
        .args(f.options())
        .args(["--tool", "services", "--arguments", "{}"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(data["global_summary"]["calls"], 0);
}

#[test]
fn stdio_tools_conform_fix_cycle_and_keep_deterministic_bounded_results() {
    let f = Fixture::new();
    let mut c = Client::start(&f, true);
    let baseline = c.ok(
        "verify",
        json!({"expect":{"cycles_in":["a","b"],"equals":0,"forbidden":0}}),
    );
    assert_eq!(baseline["pass"], true);
    let context = c.ok(
        "context",
        json!({"task":"change a","files":["src/a/a.ts"],"symbols":["a"],"budget_tokens":4000}),
    );
    assert!(context["entries"].is_array());
    let rules = c.ok("rules", json!({"paths":["src/b/b.ts"]}));
    assert_eq!(rules["forbid"][0]["reason"], "keep b independent of a");
    assert_eq!(c.ok("target", json!({})), Value::Null);
    let explain = c.ok("explain", json!({"path":"src/a/a.ts"}));
    assert!(explain["summary"].as_str().unwrap().contains("imports:"));
    let simulation=c.ok("simulate",json!({"moves":[{"from":"src/a/a.ts","to":"src/a/renamed.ts"},{"from":"src/b/b.ts","to":"src/b/renamed.ts"}]}));
    assert_eq!(simulation["before"]["cycles"], 0);
    assert_eq!(simulation["after"]["cycles"], 0);
    f.cycle(true);
    let args = json!({"base":"HEAD"});
    let diff = c.tool("check_diff", args.clone());
    let again = c.tool("check_diff", args);
    assert_eq!(
        serde_json::to_vec(&diff).unwrap(),
        serde_json::to_vec(&again).unwrap()
    );
    let data = &diff["structuredContent"]["data"];
    assert_ne!(diff["isError"], true, "{diff}");
    assert!(!data["new_cycles"].as_array().unwrap().is_empty(), "{data}");
    // The new concrete-cycle witnesses share the hard response cap with edge
    // details. Omission must retain full counts, not imply a clean policy.
    assert!(data["new_forbidden"].as_array().unwrap().len() <= 1);
    assert_eq!(diff["structuredContent"]["counts"]["new_forbidden"], 1);
    assert_eq!(diff["structuredContent"]["counts"]["new_cyclic_imports"], 2);
    assert_eq!(diff["structuredContent"]["truncated"], true);
    let review = c.tool("review", json!({"base":"HEAD"}));
    let markdown = review["structuredContent"]["data"]["markdown"]
        .as_str()
        .unwrap_or_else(|| panic!("{review}"));
    assert_eq!(review["content"][0]["text"], markdown);
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&f.root)
        .output()
        .unwrap()
        .stdout;
    let head = String::from_utf8(head).unwrap();
    assert!(
        markdown.contains("1 other new forbidden import")
            && markdown.contains(&format!("\nBase {} (explicit).\n", &head[..12])),
        "{markdown}"
    );

    let id = data["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "cycle")
        .unwrap_or_else(|| panic!("cycle finding missing from brief projection: {data}"))["finding_id"]
        .clone();
    let recipe = c.ok("propose", json!({"finding_id":id}));
    assert!(!recipe["steps"].as_array().unwrap().is_empty(), "{recipe}");
    assert_eq!(recipe["steps"][0]["kind"], "cut_import");
    let failed = c.ok(
        "verify",
        json!({"expect":{"cycles_in":["a","b"],"equals":0}}),
    );
    assert_eq!(failed["pass"], false);
    f.cycle(false);
    let args =
        json!({"expect":{"cycles_in":["a","b"],"equals":0,"forbidden":0,"findings_absent":[id]}});
    let verified = c.tool("verify", args.clone());
    assert_eq!(verified["structuredContent"]["data"]["pass"], true);
    assert_eq!(verified, c.tool("verify", args));
    for (tool, args) in [
        ("verify", json!({"expect":{"cycles_in":["unknown"]}})),
        ("rules", json!({"paths":["../escape.ts"]})),
        ("context", json!({"task":"x","budget_tokens":"wrong"})),
        ("explain", json!({"module":"unknown"})),
    ] {
        let result = c.tool(tool, args);
        assert_eq!(result["isError"], true, "{result}");
        assert!(
            result["structuredContent"]["error"]
                .as_str()
                .is_some_and(|s| s.len() > 10),
            "{result}"
        );
    }
    let long = c.tool(
        "context",
        json!({"task":"a".repeat(10000),"files":["src/a/a.ts"],"budget_tokens":4000,"brief":false}),
    );
    assert_eq!(long["structuredContent"]["truncated"], true);
    assert!(long["structuredContent"]["hint"].is_string());
    assert_eq!(
        c.ok("verify", json!({"expect":{"forbidden":0}}))["pass"],
        true,
        "server survives malformed calls"
    );
}

#[test]
fn stdio_uses_matching_daemon_and_refreshes_its_generation() {
    let mut f = Fixture::new();
    f.start_daemon();
    let start = f.status()["generation"].as_u64().unwrap();
    let mut c = Client::start(&f, false);
    assert_eq!(
        c.ok("verify", json!({"expect":{"cycles_in":["a"],"equals":0}}))["pass"],
        true
    );
    f.cycle(true);
    assert_eq!(
        c.ok("verify", json!({"expect":{"cycles_in":["a"],"equals":1}}))["pass"],
        true
    );
    let status = f.status();
    assert!(
        status["generation"].as_u64().unwrap() > start,
        "MCP did not refresh matching daemon: {status}"
    );
    assert_eq!(status["parsed_files"], 1);
    f.cycle(false);
    assert_eq!(
        c.ok("verify", json!({"expect":{"cycles_in":["a"],"equals":0}}))["pass"],
        true
    );
}

#[test]
fn warm_snapshot_query_does_not_wait_for_a_native_transaction_lock() {
    let mut f = Fixture::new();
    let output = f
        .command()
        .args([
            "--no-daemon",
            "hook",
            "capture",
            "--agent",
            "codex",
            "--language",
            "typescript",
            "--files",
            "src/b/b.ts",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured: Value = serde_json::from_slice(&output.stdout).unwrap();
    f.start_daemon();
    f.cycle(true);
    let mut client = Client::start(&f, false);
    // A native hook can own this lock while waiting for its scan RPC in the serial daemon.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.root.join(".archgraph/hooks/lock"))
        .unwrap();
    lock.lock().unwrap();
    let response = client.tool(
        "check_diff",
        json!({"snapshot":captured["snapshot"], "brief":false}),
    );
    assert_ne!(response["isError"], true, "{response}");
    let envelope = &response["structuredContent"];
    let result = &envelope["data"];
    assert_eq!(result["verdict"], "BLOCK", "{result}");
    assert!(
        envelope["counts"]["new_forbidden"].as_u64().unwrap() > 0,
        "{response}"
    );
    assert_eq!(result["base_selection"], "pre-edit snapshot");
    drop(lock);
    // The reason changes the environment: a bypass must not require weakening daemon trust.
    let bypass = f
        .command()
        .args([
            "hook",
            "check",
            "--baseline",
            captured["snapshot"].as_str().unwrap(),
            "--brief",
        ])
        .env("ARCHGRAPH_BYPASS", "accept fixture boundary once")
        .output()
        .unwrap();
    assert!(
        bypass.status.success(),
        "{}",
        String::from_utf8_lossy(&bypass.stderr)
    );
    assert!(bypass.stdout.starts_with(b"PASS\n"));
    let overrides = fs::read_to_string(f.root.join(".archgraph/overrides.log")).unwrap();
    let record: Value = serde_json::from_str(overrides.lines().next().unwrap()).unwrap();
    assert_eq!(record["reason"], "accept fixture boundary once");
    assert!(!record["finding_ids"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn mcp_reuses_reference_daemon_cache_when_collector_cannot_run_again() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    fs::write(f.root.join("src/a/a.ts"), "export const a = 1;\n").unwrap();
    let collector = f.root.join(".archgraph/depcruise");
    fs::write(&collector,r##"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '18.4.0\n'
  exit 0
fi
printf 'collect\n' >> .archgraph/collector-calls
printf '%s\n' '{"modules":[{"source":"src/a/a.ts","dependencies":[]},{"source":"src/b/b.ts","dependencies":[]}],"summary":{"totalCruised":2,"environment":{"version":"18.4.0"}}}'
"##).unwrap();
    fs::set_permissions(&collector, fs::Permissions::from_mode(0o700)).unwrap();
    let options = vec![
        ".".into(),
        "--language".into(),
        "typescript".into(),
        "--engine".into(),
        "reference".into(),
        "--depcruise".into(),
        collector.to_string_lossy().into_owned(),
        "--rules".into(),
        f.root.join("layout.yaml").to_string_lossy().into_owned(),
    ];
    f.start_daemon_options(options.clone());
    let calls = fs::read(f.root.join(".archgraph/collector-calls")).unwrap();
    assert!(!calls.is_empty(), "warmup must run the collector");
    fs::set_permissions(&collector, fs::Permissions::from_mode(0o000)).unwrap();
    let mut client = Client::start_options(&f, false, options, "2025-11-25");
    let result = client.ok(
        "verify",
        json!({"expect":{"cycles_in":["a","b"],"equals":0,"forbidden":0}}),
    );
    assert_eq!(result["pass"], true);
    assert_eq!(
        fs::read(f.root.join(".archgraph/collector-calls")).unwrap(),
        calls,
        "MCP must reuse the matching warm graph without collecting again"
    );
}

#[test]
fn current_mcp_client_negotiates_supported_protocol_and_preserves_output_caps() {
    let f = Fixture::new();
    let mut client = Client::start_options(&f, true, f.options(), "2026-07-28");
    let verified = client.ok("verify", json!({"expect":{"cycles_in":["a"],"equals":0}}));
    assert_eq!(verified["pass"], true);
    let explanation = client.ok("explain", json!({"path":"src/a/a.ts"}));
    assert!(explanation["summary"].is_string());
}

#[test]
fn missing_git_baseline_is_actionable_and_other_tools_remain_available() {
    let f = Fixture::new();
    fs::rename(f.root.join(".git"), f.root.join(".archgraph/fixture-git")).unwrap();
    let mut c = Client::start(&f, true);
    let error = c.tool("check_diff", json!({}));
    assert_eq!(error["isError"], true);
    let message = error["structuredContent"]["error"].as_str().unwrap();
    assert!(
        message.contains("not a git repo") && message.contains("cwd=<repo>"),
        "{message}"
    );
    assert!(
        c.ok(
            "context",
            json!({"task":"read a","files":["src/a/a.ts"],"budget_tokens":4000})
        )["entries"]
            .is_array()
    );
}

#[test]
fn review_base_line_tells_external_rules_from_changed_rules() {
    let f = Fixture::new();
    let review = |rules: &std::path::Path, base: &[&str]| {
        let out = f
            .command()
            .args(["--no-daemon", "review", ".", "--language", "typescript"])
            .args(["--engine", "native", "--rules"])
            .arg(rules)
            .args(base)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    // No origin/HEAD, main or master: the base is HEAD.
    f.git(&["branch", "-m", "topic"]);
    let markdown = review(&f.root.join("layout.yaml"), &[]);
    assert!(
        markdown.ends_with(
            " (HEAD fallback: no origin/HEAD, main or master reference; includes uncommitted edits).\n"
        ),
        "{markdown}"
    );
    // Outside the repository: no file at base to compare with.
    let external = f.root.with_extension("yaml");
    fs::copy(f.root.join("layout.yaml"), &external).unwrap();
    let markdown = review(&external, &["--base", "HEAD"]);
    fs::remove_file(&external).unwrap();
    assert!(
        markdown.ends_with(
            " (explicit); rules from outside the repository: both commits are judged by them.\n"
        ),
        "{markdown}"
    );
    // In the repository and edited since base.
    fs::write(f.root.join("layout.yaml"), "version: 1\n").unwrap();
    let markdown = review(&f.root.join("layout.yaml"), &["--base", "HEAD"]);
    assert!(
        markdown.ends_with(
            " (explicit); rules changed since base: both commits are judged by the current rules.\n"
        ),
        "{markdown}"
    );
}
