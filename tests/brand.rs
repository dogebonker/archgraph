mod common;

use common::{archgraph, scan_recorded, validate, write_tmp};

#[test]
fn executable_identity_changes_without_changing_persisted_report_identity() {
    let version = archgraph(&["--version"]);
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .starts_with("boundrail ")
    );
    let help = archgraph(&["--help"]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: boundrail")
    );

    let (mut report, _, _) = scan_recorded("ts-cycles", &[]);
    assert_eq!(report["schema"], "archgraph/v1.2");
    assert_eq!(report["tool"]["name"], "archgraph");
    let doc = boundrail::restore_scan(serde_json::from_value(report.clone()).unwrap()).unwrap();
    let sarif = boundrail::sarif::sarif(&doc);
    common::validate_sarif(&sarif);
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "archgraph");
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["fullName"], "Boundrail");
    boundrail::compat::v1(&mut report).unwrap();
    validate(&report);
    let input = write_tmp("brand/legacy.json", &report.to_string());
    let rendered = archgraph(&["render", &input]);
    assert!(rendered.status.success());
    let html = String::from_utf8(rendered.stdout).unwrap();
    assert!(html.contains("<title>Boundrail</title>"));
    assert!(html.contains("archgraph/v1.2"));
    assert!(html.contains("archgraph.layout."));
}
