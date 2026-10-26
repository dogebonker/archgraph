# Architecture benchmarks

These are pinned public-source workloads for Java, Rust and Git history. They check known source references and historical changes as well as scan time. They are not an exhaustive dependency-graph oracle.

The improvement loop is: freeze a release binary, establish a failing correctness or size assertion, make one small change, rerun the assertion, then compare release measurements. Changes which preserve semantics must preserve the graph fingerprint. No dependency was added for these benchmarks.

## Workloads

| Workload | Revision | What it checks |
| --- | --- | --- |
| [JSON-java 20240303](https://github.com/stleary/JSON-java/tree/f9b5587c87aaf02e4a5ed1991d48d6c05993624a) | `f9b5587c` | 25 production files, known same-package/inheritance references, zero unresolved diagnostics. Its [README](https://github.com/stleary/JSON-java/blob/f9b5587c87aaf02e4a5ed1991d48d6c05993624a/README.md) states that it has no external dependencies. |
| [Gson 2.10.1](https://github.com/google/gson/tree/2ce6a6106c22bf0cce2e39f68b0da2f117d6e66b) | `2ce6a610` | 99 production files, cross-package references, no false dependencies on package declarations. Its [generated GsonBuildConfig](https://github.com/google/gson/blob/2ce6a6106c22bf0cce2e39f68b0da2f117d6e66b/gson/pom.xml) makes it a partial-resolution workload. |
| [ripgrep 14.1.1](https://github.com/BurntSushi/ripgrep/tree/4649aa9700619f94cf9c66876e9549d83420e16c) | `4649aa97` | A Cargo workspace: 82 indexed production files. Known `main.rs` references resolve to `HiArgs` and `WalkState` in their declaring files, rather than facade re-exports. |
| JSON-java history | same pin, 4 commits | [Deletion of NumberConversionUtil](https://github.com/stleary/JSON-java/commit/3f97826462221975d3582b748024701ebd2df8f7): 26 → 25 files. The next two commits change only CI/docs/POM; graph fields must remain identical. |

The assertions and full revisions live in [repos.json](repos.json). Java expectations were checked against source declarations and uses. [jdeps](https://docs.oracle.com/en/java/javase/21/docs/specs/man/jdeps.html) consumes compiled classes/JARs, so it is not a no-build source oracle.

## Results, October 16, 2026

Measured locally on macOS arm64, optimized release binaries, JDK 17.0.16 and rust-analyzer 1.95.0. Each end-to-end result is the median of three sequential runs after one warm-up. Cloning and dependency fetching are outside the timed region. These are local observations, not portable performance guarantees.

| Workload | Before | After | Correctness/result |
| --- | ---: | ---: | --- |
| JSON-java scan | 0.826 s | 0.783 s | Same 75 edges and graph fingerprint; zero diagnostics. |
| Gson scan | 1.252 s | 1.078 s | 682 → 452 edges: exactly 230 false package edges removed, all other edges unchanged. |
| ripgrep scan | 3.993 s | 3.916 s | Same 282 edges and graph fingerprint. The small timing difference does not establish a CLI speedup. |
| JSON-java history, 4 commits | 3.456 s | 3.134 s | Same graph fields; HTML 1,067,890 → 305,067 bytes, **71.4% smaller**. |

Three changes survived verification:

1. Java package elements no longer become dependencies on `package-info.java`. A three-file regression failed before the fix. Gson's 312 compiler diagnostics remain: unavailable generated/external code means its graph is still partial.
2. History embeds one graph template and one copy of each snapshot, creating only the selected page when needed. A size-budget regression failed on the old renderer. JavaScript unit tests check navigation and preserve literal `$&`, `{{GRAPH}}` and script-end strings in paths.
3. Rust precomputes each document's module namespaces and skips namespace searching for ordinary symbols. An isolated parser benchmark (20 parses per sample, seven interleaved samples per binary) fell from 0.130295 s to 0.115404 s, **11.4%**. End-to-end indexing remains the dominant cost.

Raw samples, binary hashes and graph fingerprints: [before](results/before.json), [after](results/after.json), [Java edge comparison](results/java-correctness.json), [Rust parser samples](results/rust-parser.json). The baseline is the uncommitted implementation from the preceding development pass, identified by its binary hash; it is not a released boundrail version. The isolated Rust samples used a retained SCIP dump; the standalone parser benchmark below regenerates that input before timing.

## Reproduce

Needs Python 3, Git, JDK 11+, and rustup with rust-analyzer/rust-src. Build boundrail first:

```sh
cargo +stable build --release --locked
RUSTUP_TOOLCHAIN=stable python3 benchmarks/run.py \
  --work /tmp/boundrail-benchmarks --output /tmp/boundrail-results.json --runs 3
```

The runner fetches each pinned commit with eight commits of history, reuses an existing clean checkout at that pin, and preserves the final raw graphs and collector logs under `--work/outputs`. `--only gson` selects one workload and can be repeated. `--binary /path/to/boundrail` compares another binary. A source assertion failure or scan failure exits 1 but still writes the other results.

Rust needs cached dependencies; the first run can record a missing-cache failure. Fetch the pinned ripgrep dependencies without building it, then rerun:

```sh
cargo +stable fetch --locked --manifest-path /tmp/boundrail-benchmarks/repos/ripgrep/Cargo.toml
```

Use the same Cargo registry protocol for fetching and scanning. This machine's cache uses `CARGO_REGISTRIES_CRATES_IO_PROTOCOL=git`; that variable was set for the Rust measurements. No Maven/Gradle build, annotation processor, Rust build script or proc macro runs during these scans.

For the isolated Rust parser, compile against the optimized library. Indexing happens once before the timed loop:

```sh
rustc +stable --edition=2024 -O benchmarks/rust_parse.rs \
  --extern boundrail=target/release/libboundrail.rlib \
  -L dependency=target/release/deps -o /tmp/boundrail-rust-parse
RUSTUP_TOOLCHAIN=stable /tmp/boundrail-rust-parse /tmp/boundrail-benchmarks/repos/ripgrep
```

Check the benchmark assertions and timeline logic without fetching projects:

```sh
python3 -m unittest discover -s benchmarks
node --test tests/history-ui.test.cjs
```

## Limits

Known-reference assertions cover important relationships, not all edges. Rust SCIP can include references in inactive inline tests and omit `pub extern crate` facade links. Missing Java libraries and generated sources leave partial graphs. Timings include JVM/indexer startup and normal filesystem/cache variation. Browser rendering and memory use were not benchmarked.

## Hotspot association across the original showcase corpus

The [20-repository validation report](HOTSPOTS.md) records complete coverage, per-project pins, commit-size-adjusted random comparisons and limitations. This is a separate study from the collector/runtime measurements above.
