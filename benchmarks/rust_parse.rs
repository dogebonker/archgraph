//! Standalone parser benchmark; collect once, then time 20 parses without indexing or analysis.

use std::{hint::black_box, path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("pass the Cargo repository root")?;
    let input = boundrail::collect::rust::run(
        Path::new(&root),
        &boundrail::collect::rust::Options {
            rust_analyzer: None,
            features: boundrail::collect::rust::Features::Default,
            build_scripts: false,
        },
    )?;
    let graph = boundrail::collect::rust::parse(&input)?;
    let counts = (graph.files.len(), graph.edges.len());
    assert!(
        counts.0 > 0,
        "empty input is not a representative parser benchmark"
    );
    let start = Instant::now();
    for _ in 0..20 {
        let graph = boundrail::collect::rust::parse(black_box(&input))?;
        assert_eq!((graph.files.len(), graph.edges.len()), counts);
        black_box(graph);
    }
    println!(
        "{{\"iterations\":20,\"files\":{},\"edges\":{},\"seconds\":{}}}",
        counts.0,
        counts.1,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
