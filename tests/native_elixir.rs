mod common;

use boundrail::collect::{
    excludes::{Options, Policy},
    native::elixir,
};
use boundrail::model::EdgeKind;
use std::path::Path;

#[test]
fn native_elixir_matches_recorded_fixture_without_a_vm() {
    let root = common::fixture("elixir-basic");
    let policy = Policy::new(
        &root,
        boundrail::model::Language::Elixir,
        Options::default(),
    )
    .unwrap();
    let out = elixir::collect(&root, &policy).unwrap();
    assert_eq!(out.raw.edges.len(), 5);
    assert_eq!(
        out.raw
            .imports
            .as_ref()
            .unwrap()
            .iter()
            .filter(|r| r.unresolved.is_some())
            .count(),
        0
    );
    assert_eq!(
        out.raw
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Type)
            .count(),
        2
    );
    assert!(
        out.raw
            .imports
            .unwrap()
            .iter()
            .any(|r| r.external.as_deref() == Some("elixir"))
    );
    let output = common::archgraph(&[
        "scan",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--language",
        "elixir",
        "--elixir",
        "/nonexistent/elixir",
        "--symbols",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    common::validate(&serde_json::from_slice(&output.stdout).unwrap());
}

#[test]
fn native_elixir_scopes_types_quotes_names_and_dynamic_calls_are_explicit() {
    let file = common::write_tmp(
        "native-elixir-scopes/lib/client.ex",
        r#"
defmodule Demo.Client do
  alias Demo.{Store, Value}
  require Demo.Macros, as: M
  @spec run(Value.t()) :: Value.t()
  def run(receiver) do
    Store.get(%Value{})
    &M.run/1
    :lists.reverse([])
    receiver.run()
    Module.concat(Demo.Store, "Child")
  end
  quote do Hidden.run() end
end
"#,
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp(
        "native-elixir-scopes/lib/store.ex",
        "defmodule Demo.Store do end\n",
    );
    common::write_tmp(
        "native-elixir-scopes/lib/value.ex",
        "defmodule Demo.Value do end\n",
    );
    common::write_tmp(
        "native-elixir-scopes/lib/macros.ex",
        "defmodule Demo.Macros do end\n",
    );
    let out = elixir::collect(
        root,
        &Policy::new(root, boundrail::model::Language::Elixir, Options::default()).unwrap(),
    )
    .unwrap();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.to == "lib/value.ex" && e.kind == EdgeKind::Type)
    );
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.to == "lib/value.ex" && e.kind == EdgeKind::Static)
    );
    assert!(out.raw.edges.iter().any(|e| e.to == "lib/macros.ex"));
    let imports = out.raw.imports.unwrap();
    assert!(
        imports
            .iter()
            .any(|r| r.external.as_deref() == Some("stdlib"))
    );
    assert_eq!(
        imports
            .iter()
            .filter(|r| matches!(r.unresolved, Some(boundrail::model::ImportReason::Dynamic)))
            .count(),
        2
    );
    assert!(!imports.iter().any(|r| r.specifier.contains("Hidden")));
}

#[test]
fn native_elixir_nested_definitions_and_exclusions_never_execute_source() {
    let file = common::write_tmp(
        "native-elixir-nested/lib/a.ex",
        "defmodule Outer do\n defmodule Inner.Child do end\n def run, do: Inner.Child.run()\nend\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp(
        "native-elixir-nested/lib/use.ex",
        "defmodule Consumer do\n def run, do: Outer.Inner.Child.run()\nend\n",
    );
    common::write_tmp(
        "native-elixir-nested/mix.exs",
        "raise \"must not run Mix\"\n",
    );
    common::write_tmp(
        "native-elixir-nested/test/bad_test.exs",
        "not valid \"syntax\n",
    );
    let out = elixir::collect(
        root,
        &Policy::new(root, boundrail::model::Language::Elixir, Options::default()).unwrap(),
    )
    .unwrap();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "lib/use.ex" && e.to == "lib/a.ex")
    );
    assert!(!out.raw.edges.iter().any(|e| e.from.contains("bad_test")));
}

#[test]
fn native_elixir_multi_alias_binds_last_segment_and_indexes_excluded_modules() {
    let file = common::write_tmp(
        "native-elixir-multi-alias/lib/client.ex",
        "defmodule Demo.Client do\n  alias Demo.{Accounts.User}\n  def run, do: User.get() && Demo.Support.run()\nend\n",
    );
    let root = Path::new(&file).ancestors().nth(2).unwrap();
    common::write_tmp(
        "native-elixir-multi-alias/lib/user.ex",
        "defmodule Demo.Accounts.User do end\n",
    );
    common::write_tmp(
        "native-elixir-multi-alias/test/support.ex",
        "defmodule Demo.Support do end\n",
    );
    let out = elixir::collect(
        root,
        &Policy::new(root, boundrail::model::Language::Elixir, Options::default()).unwrap(),
    )
    .unwrap();
    assert!(
        out.raw
            .edges
            .iter()
            .any(|e| e.from == "lib/client.ex" && e.to == "lib/user.ex")
    );
    assert!(!out.raw.edges.iter().any(|e| e.to == "test/support.ex"));
    assert!(out.raw.imports.unwrap().iter().any(|r| {
        r.specifier == "Demo.Support" && r.targets == ["test/support.ex"] && r.unresolved.is_none()
    }));
}
