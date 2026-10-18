use archgraph::{
    model::Language,
    rewrite::{self, Edit},
};

fn rewrite(language: Language, file: &str, source: &str, from: &str, to: &str) {
    let captures = rewrite::specifiers(language, file, source.as_bytes()).unwrap();
    let found = captures
        .iter()
        .find(|s| s.value == from)
        .unwrap_or_else(|| panic!("missing {from}: {captures:?}"));
    let result = rewrite::rewrite_specifier(
        language,
        file,
        source.as_bytes(),
        Some(found.line),
        from,
        to,
    )
    .unwrap();
    let mut expected = source.as_bytes()[..found.range.start].to_vec();
    expected.extend_from_slice(to.as_bytes());
    expected.extend_from_slice(&source.as_bytes()[found.range.end..]);
    assert_eq!(result, expected);
}

#[test]
fn specifier_only_all_grammars_preserves_bytes() {
    for (language, file, source, from, to) in [
        (
            Language::Typescript,
            "a.ts",
            "\u{feff}// café\r\nimport { X } from './old'; // './old'\r\n",
            "./old",
            "./new",
        ),
        (
            Language::Typescript,
            "a.js",
            "// import 'wrong'\nconst x = require(\"./old\");\nconst other = './old';",
            "./old",
            "./new",
        ),
        (
            Language::Typescript,
            "a.tsx",
            "export {X} from './old';\nconst x = <div/>;",
            "./old",
            "./new",
        ),
        (
            Language::Python,
            "a.py",
            "# café\r\nfrom ..old import X # ..old\r\n",
            "..old",
            "..new",
        ),
        (
            Language::Python,
            "a.py",
            "import old.path as stable\n",
            "old.path",
            "new.path",
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport stable `example.com/old` // old\n",
            "example.com/old",
            "example.com/new",
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport \"example.com/old/pkg\"\n",
            "example.com/old/pkg",
            "example.com/new/pkg",
        ),
        (
            Language::Rust,
            "a.rs",
            "// café\r\nuse crate::old::X; // crate::old::X\r\n",
            "crate::old::X",
            "crate::new::X",
        ),
        (
            Language::Java,
            "A.java",
            "// café\r\nimport app.old.X;\r\nclass A {}",
            "app.old.X",
            "app.new.X",
        ),
        (
            Language::Elixir,
            "a.ex",
            "defmodule A do\n  alias Old.Thing # Old.Thing\nend\n",
            "Old.Thing",
            "New.Thing",
        ),
    ] {
        rewrite(language, file, source, from, to);
    }
}

#[test]
fn rejects_ambiguity_escapes_syntax_and_injection() {
    for (source, from, to) in [
        ("import './x'; import './x';", "./x", "./y"),
        ("import './x';", "./x", "./y';evil()//"),
        ("import './x\\u002ejs';", "./x\\u002ejs", "./y"),
        ("import './x'; function {", "./x", "./y"),
        (
            "function f(require) { return require('./x'); }",
            "./x",
            "./y",
        ),
    ] {
        assert!(
            rewrite::rewrite_specifier(
                Language::Typescript,
                "a.ts",
                source.as_bytes(),
                None,
                from,
                to
            )
            .is_err(),
            "{source}"
        );
    }
    let source = b"import './x';\nimport './x';";
    assert_eq!(
        rewrite::rewrite_specifier(Language::Typescript, "a.ts", source, Some(2), "./x", "./y")
            .unwrap(),
        b"import './x';\nimport './y';"
    );
}

#[test]
fn refuses_binding_changes_and_grouped_conditional_paths() {
    for (language, file, source, from, to) in [
        (
            Language::Python,
            "a.py",
            "import old.path\n",
            "old.path",
            "new.path",
        ),
        (
            Language::Rust,
            "a.rs",
            "use crate::old::{X,Y};",
            "crate::old::{X,Y}",
            "crate::new::X",
        ),
        (
            Language::Rust,
            "a.rs",
            "#[cfg(test)]\nuse crate::old::X;",
            "crate::old::X",
            "crate::new::X",
        ),
        (
            Language::Rust,
            "a.rs",
            "use crate::old::X;",
            "crate::old::X",
            "crate::new::Y",
        ),
        (
            Language::Java,
            "A.java",
            "import static app.old.X; class A {}",
            "app.old.X",
            "app.new.X",
        ),
        (
            Language::Java,
            "A.java",
            "import app.old.*; class A {}",
            "app.old",
            "app.new",
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport \"example.com/old\"\n",
            "example.com/old",
            "example.com/new",
        ),
        (
            Language::Elixir,
            "a.ex",
            "alias Old.Thing\n",
            "Old.Thing",
            "New.Other",
        ),
    ] {
        assert!(
            rewrite::rewrite_specifier(language, file, source.as_bytes(), None, from, to).is_err(),
            "{source}"
        );
    }
}

#[test]
fn dynamic_calls_and_elixir_declarations_are_not_import_specifiers() {
    let ts = rewrite::specifiers(
        Language::Typescript,
        "a.ts",
        b"const x = import('./ok'); const y = import(foo); const s = './ok';",
    )
    .unwrap();
    assert_eq!(ts.len(), 1);
    assert_eq!(ts[0].value, "./ok");
    let elixir = rewrite::specifiers(Language::Elixir,"a.ex",b"defmodule Old.Thing do\n @x \"Old.Thing\"\n require Other.Mod\n import Other.Foo\n use Other.Bar\nend").unwrap();
    assert_eq!(
        elixir.iter().map(|s| s.value.as_str()).collect::<Vec<_>>(),
        vec!["Other.Mod", "Other.Foo", "Other.Bar"]
    );
}

#[test]
fn byte_edits_check_fingerprints_and_overlaps() {
    let edit = |range, expected: &[u8], replacement: &[u8]| Edit {
        range,
        expected: expected.to_vec(),
        replacement: replacement.to_vec(),
    };
    assert!(
        rewrite::replace(
            b"abcdef",
            &[edit(1..4, b"bcd", b"x"), edit(3..5, b"de", b"y")]
        )
        .is_err()
    );
    assert!(rewrite::replace(b"abcdef", &[edit(1..4, b"wrong", b"x")]).is_err());
    assert!(rewrite::replace(b"abcdef", &[edit(9..10, b"", b"x")]).is_err());
    assert_eq!(
        rewrite::replace(
            b"abcdef",
            &[edit(4..6, b"ef", b"Y"), edit(0..2, b"ab", b"X")]
        )
        .unwrap(),
        b"XcdY"
    );
}

#[test]
fn appends_explicit_reexports_and_preserves_crlf() {
    let ts = "// café\r\nexport const X = 1;".as_bytes();
    assert_eq!(
        rewrite::append_reexport(
            Language::Typescript,
            "index.ts",
            ts,
            "./thing",
            &["Z".into(), "Y".into()]
        )
        .unwrap(),
        "// café\r\nexport const X = 1;\r\nexport { Y, Z } from './thing';\r\n".as_bytes()
    );
    assert_eq!(
        rewrite::append_reexport(
            Language::Python,
            "__init__.py",
            b"# hi\n",
            ".thing",
            &["X".into()]
        )
        .unwrap(),
        b"# hi\nfrom .thing import X as X\n"
    );
    assert!(
        rewrite::append_reexport(
            Language::Python,
            "__init__.py",
            b"__all__ = []\n",
            ".thing",
            &["X".into()]
        )
        .is_err()
    );
}

#[test]
fn removals_require_unused_bindings_and_complete_declarations() {
    for (language, file, source, expected) in [
        (Language::Typescript, "a.ts", "import './x';", true),
        (
            Language::Typescript,
            "a.ts",
            "import {X} from './x'; const y = 1;",
            true,
        ),
        (
            Language::Typescript,
            "a.ts",
            "import {X} from './x'; const y = X;",
            false,
        ),
        (
            Language::Typescript,
            "a.ts",
            "import {X as Y} from './x'; const y = Y;",
            false,
        ),
        (
            Language::Typescript,
            "a.ts",
            "import * as X from './x';",
            false,
        ),
        (
            Language::Typescript,
            "a.ts",
            "export {X} from './x';",
            false,
        ),
        (
            Language::Typescript,
            "a.ts",
            "const x = require('./x');",
            false,
        ),
        (
            Language::Python,
            "a.py",
            "from .x import Thing\nx = 1\n",
            true,
        ),
        (
            Language::Python,
            "a.py",
            "from .x import Thing\nx = Thing()\n",
            false,
        ),
        (Language::Python, "a.py", "from .x import *\n", false),
        (Language::Python, "a.py", "import x as stable\n", true),
        (
            Language::Python,
            "a.py",
            "import x as stable\nstable.run()\n",
            false,
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport _ \"example.com/x\"\n",
            true,
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport \"example.com/x\"\nfunc run() {x.Run()}\n",
            false,
        ),
        (
            Language::Go,
            "a.go",
            "package a\nimport . \"example.com/x\"\n",
            false,
        ),
    ] {
        let spans = rewrite::specifiers(language, file, source.as_bytes()).unwrap();
        assert_eq!(
            rewrite::can_remove_import(language, file, source.as_bytes(), &spans[0]).unwrap(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn aliases_preserve_rust_and_elixir_bindings() {
    rewrite(
        Language::Rust,
        "a.rs",
        "use crate::old::X as Stable;",
        "crate::old::X",
        "crate::new::Y",
    );
    rewrite(
        Language::Elixir,
        "a.ex",
        "alias Old.Thing, as: Stable\n",
        "Old.Thing",
        "New.Other",
    );
}

#[test]
fn go_cut_removes_declaration_keyword_or_one_group_member() {
    for (source, expected, remaining) in [
        (
            "package a\nimport _ \"example.com/x\"\n",
            "package a\n\n",
            0,
        ),
        (
            "package a\nimport (\n _ \"example.com/x\"\n)\n",
            "package a\n\n",
            0,
        ),
        (
            "package a\nimport (\n _ \"example.com/x\"\n _ \"example.com/y\"\n)\n",
            "package a\nimport (\n \n _ \"example.com/y\"\n)\n",
            1,
        ),
        (
            "package a\nimport unused \"example.com/x\"\n",
            "package a\n\n",
            0,
        ),
    ] {
        let span = rewrite::specifiers(Language::Go, "a.go", source.as_bytes())
            .unwrap()
            .remove(0);
        let range = rewrite::removal_range(Language::Go, "a.go", source.as_bytes(), &span)
            .unwrap()
            .unwrap();
        let edited = rewrite::replace(
            source.as_bytes(),
            &[Edit {
                expected: source.as_bytes()[range.clone()].to_vec(),
                range,
                replacement: Vec::new(),
            }],
        )
        .unwrap();
        assert_eq!(edited, expected.as_bytes());
        assert_eq!(
            rewrite::specifiers(Language::Go, "a.go", &edited)
                .unwrap()
                .len(),
            remaining
        );
    }
    let source = b"package a\nimport x \"example.com/x\"\nfunc run() { x.Run() }\n";
    let span = rewrite::specifiers(Language::Go, "a.go", source)
        .unwrap()
        .remove(0);
    assert!(
        rewrite::removal_range(Language::Go, "a.go", source, &span)
            .unwrap()
            .is_none()
    );
}
