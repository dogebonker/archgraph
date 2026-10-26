mod common;

use common::{boundrail, scan_recorded, validate, write_tmp};

#[test]
fn boundrail_is_the_canonical_executable_and_report_identity() {
    let version = boundrail(&["--version"]);
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .starts_with("boundrail ")
    );
    let help = boundrail(&["--help"]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: boundrail")
    );

    let (mut report, _, _) = scan_recorded("ts-cycles", &[]);
    assert_eq!(report["schema"], "boundrail/v1.2");
    assert_eq!(report["tool"]["name"], "boundrail");
    let doc = boundrail::restore_scan(serde_json::from_value(report.clone()).unwrap()).unwrap();
    let sarif = boundrail::sarif::sarif(&doc);
    common::validate_sarif(&sarif);
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "boundrail");
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["fullName"], "Boundrail");
    boundrail::compat::v1(&mut report).unwrap();
    validate(&report);
    let input = write_tmp("brand/v1.json", &report.to_string());
    let rendered = boundrail(&["render", &input]);
    assert!(rendered.status.success());
    let html = String::from_utf8(rendered.stdout).unwrap();
    assert!(html.contains("<title>Boundrail</title>"));
    assert!(html.contains("boundrail/v1.2"));
    assert!(html.contains("boundrail.layout."));
}

#[test]
fn previous_product_namespaces_are_not_supported() {
    for schema in ["archgraph/v1", "archgraph/v1.1", "archgraph/v1.2"] {
        assert!(!boundrail::model::supported_schema(schema));
    }
    let (mut report, _, _) = scan_recorded("ts-cycles", &[]);
    report["schema"] = serde_json::json!("archgraph/v1.2");
    let path = write_tmp("brand/unsupported.json", &report.to_string());
    let result = boundrail(&["render", &path]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
}
