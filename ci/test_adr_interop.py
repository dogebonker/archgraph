#!/usr/bin/env python3
"""Reproduce conservative import coverage on pinned public ADR corpora (no agent/model)."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def verify_tree(corpus, checkout):
    listing = run("git", "-C", str(checkout), "ls-tree", "-rz", corpus["commit"], "--", corpus["directory"], "layout.yaml")
    expected = {}
    for record in listing.split("\0"):
        if not record:
            continue
        metadata, path = record.split("\t", 1)
        mode, kind, blob = metadata.split()
        if path.endswith(".md") or path == "layout.yaml":
            if kind != "blob" or mode not in {"100644", "100755"}:
                raise RuntimeError(f"unsupported pinned corpus file: {path}")
            expected[path] = blob
    actual = {str(path.relative_to(checkout)) for path in (checkout / corpus["directory"]).rglob("*.md")}
    if (checkout / "layout.yaml").exists() or (checkout / "layout.yaml").is_symlink():
        actual.add("layout.yaml")
    if actual != set(expected):
        raise RuntimeError("corpus file inventory differs from pinned tree (including untracked/ignored ADRs or layout.yaml)")
    for path, blob in expected.items():
        current = checkout / path
        if current.is_symlink() or not current.is_file():
            raise RuntimeError(f"unsafe corpus file: {path}")
        pinned = subprocess.run(["git", "-C", str(checkout), "cat-file", "blob", blob], check=True, capture_output=True).stdout
        if current.read_bytes() != pinned:
            raise RuntimeError(f"corpus bytes differ from pinned tree: {path}")


def verify(binary, corpus, checkout):
    actual = run("git", "-C", str(checkout), "rev-parse", "HEAD").strip()
    if actual != corpus["commit"]:
        raise RuntimeError(f'{corpus["name"]}: checkout {actual} != pin {corpus["commit"]}')
    verify_tree(corpus, checkout)
    report = json.loads(run(str(binary), "adr", "import", corpus["directory"], "--json", "--root", str(checkout)))
    verify_tree(corpus, checkout)
    assert report["documents"] == corpus["expected_documents"], report
    assert len(report["proposals"]) == corpus["expected_proposals"], report
    mapped = {p["source"] for p in report["proposals"]}
    assert len(mapped) == corpus["expected_mapped_documents"], report
    gaps = {}
    for note in report["informational"]:
        gaps.setdefault(note["source"], []).append(note["message"])
    assert gaps == corpus["gaps"], "pinned gap inventory changed"
    architecture = set(corpus.get("architecture_sources", []))
    return {"name": corpus["name"], "commit": actual, "documents": report["documents"],
            "mapped_documents": len(mapped), "architecture_documents": len(architecture) if architecture else None,
            "mapped_architecture_documents": len(mapped & architecture) if architecture else None,
            "proposals": len(report["proposals"]), "target_met": len(mapped & architecture) >= len(architecture) / 2 if architecture else len(mapped) >= report["documents"] / 2}



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/boundrail"))
    parser.add_argument("--checkouts", type=Path, help="existing pinned checkouts named adr-kit and archgate; otherwise clone to a temporary directory")
    args = parser.parse_args()
    binary = args.binary.resolve()
    corpora = json.loads(Path(__file__).with_name("adr-corpora.json").read_text())["corpora"]
    with tempfile.TemporaryDirectory(prefix="boundrail-adr-interop-") as temporary:
        results = []
        for corpus in corpora:
            checkout = (args.checkouts or Path(temporary)) / corpus["name"]
            if args.checkouts is None:
                run("git", "clone", "--filter=blob:none", "--no-checkout", corpus["url"], str(checkout))
                run("git", "-C", str(checkout), "checkout", "--detach", corpus["commit"])
            results.append(verify(binary, corpus, checkout))
        print(json.dumps({"results": results, "acceptance_met": all(r["target_met"] for r in results)}, indent=2))


class ProvenanceTests(unittest.TestCase):
    def test_dirty_tracked_and_ignored_untracked_corpus_is_rejected_at_same_head(self):
        with tempfile.TemporaryDirectory(prefix="adr-corpus-proof-") as temporary:
            root = Path(temporary)
            run("git", "init", "-q", str(root))
            (root / "docs").mkdir()
            adr = root / "docs/ADR-001.md"
            original = "# ADR-001\n## Decision\nNo imports.\n"
            adr.write_text(original)
            (root / ".gitignore").write_text("docs/extra.md\n")
            run("git", "-C", str(root), "add", ".")
            run("git", "-C", str(root), "-c", "user.name=ADR test", "-c", "user.email=adr@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
            pin = run("git", "-C", str(root), "rev-parse", "HEAD").strip()
            corpus = {"commit": pin, "directory": "docs"}
            verify_tree(corpus, root)
            adr.write_text(original + "Modified decision.\n")
            with self.assertRaisesRegex(RuntimeError, "bytes differ"):
                verify_tree(corpus, root)
            adr.write_text(original)
            (root / "docs/extra.md").write_text("# Ignored untracked ADR\n")
            with self.assertRaisesRegex(RuntimeError, "inventory differs"):
                verify_tree(corpus, root)
            (root / "docs/extra.md").unlink()
            (root / "layout.yaml").write_text("version: 1\n")
            with self.assertRaisesRegex(RuntimeError, "inventory differs"):
                verify_tree(corpus, root)
            self.assertEqual(run("git", "-C", str(root), "rev-parse", "HEAD").strip(), pin)


if __name__ == "__main__":
    main()
