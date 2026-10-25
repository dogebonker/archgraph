# Import comparison view contract

Implementation contract for W3, approved by the architecture and critic reviews. The existing
JSON Diff stays unchanged. This object is embedded in the offline HTML from `diff --html FILE`.

```typescript
type Snapshot = {
  label: string;
  commit: string | null;
  dirty: boolean | null;
  tool_version: string | null;
  language: string;
  collector: { name: string; version: string | null };
  scan_root: string | null;
  policy: { path: string; sha256: string } | null;
};
type Node = {
  id: string;
  kind: "module" | "package" | "global";
  label: string;
  status: "added" | "removed" | "changed" | "unchanged";
  before: { files: string[] } | null;
  after: { files: string[] } | null;
};
type Evidence = {
  line: number | null;
  specifier: string | null;
  original_to: string | null;
  is_reexport: boolean;
  verdict: "allowed" | "forbidden" | "unplanned" | null;
  rule: string | null;
  reasons: string[];
  sources: string[];
  known: boolean;
};
type ImportState = {
  from_node: string;
  to_node: string;
  evidence: Evidence[];
};
type Import = {
  from: string;
  to: string;
  kind: "runtime" | "lazy" | "type" | "reexport";
  status: "added" | "removed" | "changed" | "unchanged";
  changes: ("verdict" | "rule" | "module" | "evidence")[];
  before: ImportState | null;
  after: ImportState | null;
};
type View = {
  comparison: {
    status: "recorded-compatible" | "unverified";
    policy_changed: boolean;
    warnings: string[];
    before: Snapshot;
    after: Snapshot;
  };
  nodes: Node[];
  imports: Import[];
};
```

Module node IDs are `module:` followed by the exact module ID. External IDs are `external:`
followed by the authoritative ExternalNode ID. Treat these strings as opaque. Package/global
kind and labels come from ExternalNode metadata. Use arrays and Map for repository IDs.

Node status changes only for added/removed nodes or changed sorted file membership. Label the
latter "Membership changed". Do not guess renames. Package/global presence uses empty file lists.

Import identity is `(from, to, kind.as_str())`, including both internal and external edges.
Retain every distinct original-destination and policy-evidence occurrence in deterministic order.
Source file and kind are inherited from the import row. Type reexports retain their reexport bit.
Added and removed rows have only one side. Other rows compare verdict sets, rule/reason/source/known
sets, module-node assignments, and source evidence separately. Only differing categories appear
in `changes`. Line-only movement gets the "Evidence only" label, not an added or removed import.
Decision comparisons also preserve their association with shared stable original-target,
specifier, and reexport occurrences. Equal independent decision sets must not hide a swap between
two entrypoints. Occurrence identity excludes the source line, so line movement stays evidence only.

Include all module/external nodes and import rows. Default to changes, with an "All imports"
control. Global totals refer to the full payload. Filter totals say "Showing X of Y". Distinguish
no matching rows, evidence-only changes, and no structural/rule changes. Provenance warnings stay
visible even with no changes.

The graph aggregates connections with nested Map keyed by the opaque node IDs. Retain before
and after row-index sets, including self-connections. A connection with one added and one removed
import shows both counts even if its total is unchanged. Module reassignment is moved in/out,
rather than a new import. Rule and evidence updates have separate labels. Selecting a connection
filters its rows, including removed ones. Selecting a node or row opens before/after evidence.

Read thin raw provenance and restore each graph from the same bytes. Capture original tool.version
before restoration. Compare recorded language, collector name/version/type-marking, scan root,
collection and selection settings, depth/granularity, and package grouping. Exclude derived file
inventories, counts, timestamps, commits, and excluded_files. Compare policy content, recorded SHA,
and known lists separately. Missing provenance is unverified with specific warnings. Known
incompatible language, collector, or collection settings fail before HTML export. Different recorded
analyzer versions are disclosed as unverified. Matching recorded settings do not prove repository
identity, identical analyzer binaries, or complete dependency coverage.

Show the recorded Git dirty state beside a commit. An uncommitted snapshot must not look like
the exact contents of that commit. With no Git metadata, the revision and dirty state are unknown.

Reject service HTML, legacy-v1 HTML, and outputs aliasing either input before writing. Embed JSON
with every `<` escaped. Repository text enters the DOM through textContent and safe attributes.
Follow DESIGN.md for offline behavior, focus, readable mobile graph scrolling, and status labels.
