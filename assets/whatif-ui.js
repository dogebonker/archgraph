/* ---------- live move preview: transient, never written to the saved graph or layout ---------- */
const whatifUI = {
  box: $("whatif-panel"), file: $("whatif-file"), folder: $("whatif-folder"),
  status: $("whatif-status"), result: $("whatif-result"), graph: $("whatif-graph"),
  commands: $("whatif-commands"), cli: $("whatif-cli"), git: $("whatif-git"), rules: $("whatif-rules"), moves: $("whatif-moves"),
  plan: $("whatif-plan"), list: $("whatif-list"), add: $("whatif-add")
};
const whatifOriginal = { stats: [...$("stats").childNodes], text: $("stats-text").textContent,
  signals: modSignal.slice(), health: modHealth.slice() };
/* whatifMoves is the ordered move list; whatifPending the previewed move not yet added to it. */
let whatifSaved = [], whatifMoves = [], whatifPending = null, whatifRemove = [], whatifDrawing = null, whatifChoices = false, whatifChurn = false;
const whatifQuote = text => "'" + text.replace(/'/g, "'\\''") + "'";
function whatifCommands() {
  if (!whatifResult) return;
  const moves = whatifResult.moves, q = whatifQuote;
  const graph = whatifUI.graph.value.startsWith("-") ? "./" + whatifUI.graph.value : whatifUI.graph.value;
  whatifUI.cli.value = `archgraph simulate ${q(graph)}` + moves.map(m => ` --move ${q(m.file)} ${q(m.to)}`).join("");
  const fresh = [...new Set(moves.map(m => dirname(m.to)))].filter(d => d && !scanDirs.has(d));
  whatifUI.git.value = ["set -e", ...(fresh.length ? [`mkdir -p -- ${fresh.map(q).join(" ")}`] : []), ...moves.map(m => `git mv -- ${q(m.file)} ${q(m.to)}`)].join("\n");
  whatifUI.rules.value = whatifRuleShifts(moves);
  whatifUI.moves.value = whatifLayoutMoves(moves);
}
/* Each move as a layout `moves` rule matching the exact path. Rust keeps the source's literal folders and the file name,
   so a rule cannot express a root destination, a rename, glob characters or captures in a folder, or a capture anywhere. */
function whatifLayoutMoves(moves) {
  if (!moves.length) return "";
  const glob = /[*?[\]{}]/, rows = moves.map(({ file, to }) => {
    const from = file.split("/"), name = from.pop(), dir = dirname(to);
    if (!dir || /[<>]/.test(file + "/" + to) || from.some(d => glob.test(d)) || glob.test(dir) || name !== to.split("/").pop())
      return `  # ${file} → ${to}: not expressible as a move rule`;
    return `  - { from: ${JSON.stringify([...from, name.replace(/[*?[\]{}]/g, "[$&]")].join("/"))}, to: ${JSON.stringify(dir + "/")} }`;
  });
  return ["moves:   # exact paths, first match wins: keep these above other moves", ...rows].join("\n");
}
/* The layer (first matching glob, as the checker reads it) and forbid globs each moved file stops and starts
   matching; a capture glob shifts when the bound value changes. */
function whatifRuleShifts(moves) {
  const rules = doc.config?.rules?.doc;
  if (!rules) return "No layout rules are embedded in this scan.";
  const globs = [];
  (rules.forbid || []).forEach((r, k) => {
    const id = r.id ? `forbid ${r.id}` : `forbid[${k}]`;
    globs.push([`${id} from`, r.from]);
    if (!/^(?:pkg|category):/.test(r.to || "")) globs.push([`${id} to`, r.to]);
  });
  const bind = ArchgraphPreview.bind, layer = path => (rules.layers || []).find(g => bind(g, path) !== undefined);
  const shown = (label, glob, value) => `${label} "${glob}"` + (typeof value === "string" ? ` (${glob.match(/<[^>]*>/)[0]} = ${value})` : "");
  try {
    return moves.map(({ file, to }) => {
      const leaves = [], enters = [], was = layer(file), now = layer(to);
      if (was !== now) { if (was !== undefined) leaves.push(`leaves layer "${was}"`); if (now !== undefined) enters.push(`enters layer "${now}"`); }
      for (const [label, glob] of globs) {
        const was = bind(glob, file), now = bind(glob, to);
        if (was === now) continue;
        if (was !== undefined) leaves.push("leaves " + shown(label, glob, was));
        if (now !== undefined) enters.push("enters " + shown(label, glob, now));
      }
      return `${file}: ` + ([...leaves, ...enters].join(", ") || "no layer or forbid glob changes");
    }).join("\n");
  } catch (error) { return "Rule changes unavailable: " + error.message; }
}
function whatifListRender() {
  whatifRemove = whatifMoves.map((move, k) => {
    const remove = el("button", { type: "button", "aria-label": `Remove move of ${move.file} to ${move.to}` }, "Remove");
    remove.addEventListener("click", () => { whatifForget(move.file); whatifListFocus(k); });
    return remove;
  });
  whatifUI.list.replaceChildren(...whatifMoves.map((move, k) => {
    const item = el("li");
    item.append(el("span", { class: "path" }, `${move.file} → ${move.to}`), whatifRemove[k]);
    return item;
  }));
  whatifUI.plan.hidden = !whatifMoves.length;
  whatifUI.add.disabled = !(whatifPending && whatifResult);
}
/* Focus stays in the list after a removal: the next item, else the previous, else the file field. */
function whatifListFocus(k) {
  (whatifRemove[k] || whatifRemove[k - 1] || whatifUI.file).focus();
}
/* The list with the pending move applied: a listed file moved again keeps its place with the new destination. */
function whatifPlan() {
  const p = whatifPending, k = p ? whatifMoves.findIndex(m => m.file === p.file) : -1;
  return !p ? whatifMoves.slice() : k < 0 ? [...whatifMoves, p] : whatifMoves.map((m, j) => j === k ? p : m);
}
function whatifAdd() {
  if (!whatifPending || !whatifResult) return;
  const file = whatifPending.file;
  whatifMoves = whatifPlan(); whatifPending = null; whatifListRender();
  /* The disabled Add button would drop focus: move it to the added item. */
  whatifRemove[whatifMoves.findIndex(m => m.file === file)].focus();
  whatifUI.status.textContent = `Added. ${count(whatifMoves.length, "move", "moves")} in the list.`;
}
function whatifForget(file) {
  whatifMoves = whatifMoves.filter(m => m.file !== file);
  if (whatifPending?.file === file) whatifPending = null;
  whatifShow();
  whatifUI.status.textContent = `Removed ${file}. ${count(whatifMoves.length, "move", "moves")} in the list.`;
}
function whatifClearAll() {
  whatifMoves = []; whatifPending = null; clearWhatif(); whatifListRender();
}
async function whatifCopy(field) {
  try { await navigator.clipboard.writeText(field.value); whatifUI.status.textContent = "Copied. Review and run from the scanned root."; }
  catch { field.focus(); field.select(); whatifUI.status.textContent = "Clipboard unavailable. Copy the selected command."; }
}
function clearWhatif() {
  for (const { i, local, path } of whatifSaved) {
    const m = files[i].mod;
    files[i].path = path;
    lx[i] = local[0]; ly[i] = local[1];
    files[i].x = mx[m] + lx[i]; files[i].y = my[m] + ly[i];
    fileEdges(i, new Set());
  }
  whatifSaved = []; whatifResult = null; whatifDrawing = null;
  modSignal.splice(0, modSignal.length, ...whatifOriginal.signals);
  modHealth.splice(0, modHealth.length, ...whatifOriginal.health);
  $("stats").replaceChildren(...whatifOriginal.stats);
  $("stats-text").textContent = whatifOriginal.text;
  whatifUI.result.replaceChildren(); whatifUI.commands.hidden = true;
  whatifUI.cli.value = ""; whatifUI.git.value = ""; whatifUI.rules.value = ""; whatifUI.moves.value = "";
  whatifUI.status.textContent = "Preview cleared; original graph restored.";
  buildGrid(); applyFocus(); redraw();
}
function setWhatifMode(on) {
  whatifClearAll();
  whatifMode = on;
  whatifUI.box.hidden = !on; panel.hidden = on;
  $("whatif-toggle").setAttribute("aria-pressed", String(on));
  for (const id of ["t-all", "t-problems", "t-cycles", "t-types", "t-churn", "reset"]) $(id).disabled = on;
  if (on) { whatifChurn = churnView; churnView = false; }
  else churnView = whatifChurn;
  $("t-churn").setAttribute("aria-pressed", String(churnView)); $("churn-legend").hidden = !churnView;
  if (on) {
    if (!whatifChoices) {
      const folders = new Set();
      for (const f of doc.files) { $("whatif-files").append(el("option", { value: f.path })); folders.add(dirname(f.path) || "."); }
      for (const folder of [...folders].sort()) $("whatif-folders").append(el("option", { value: folder }));
      whatifChoices = true;
    }
    if (sel?.type === "file") whatifUI.file.value = doc.files[sel.i].path;
    else if (!whatifUI.file.value) whatifUI.file.value = doc.files[0]?.path || "";
    whatifUI.status.textContent = "Choose a destination or zoom to files and drag one into another module.";
    if (sideHidden) setSideHidden(false);
    if (sheetMode) snapSheet(1, 0);
    whatifUI.file.focus();
  } else { showPanel(); $("whatif-toggle").focus(); }
  applyFocus(); redraw();
}
function whatifDrop(i, x, y, originalLocal) {
  if (whatifSaved.some(s => s.i === i) && Math.hypot(x - mx[files[i].mod], y - my[files[i].mod]) <= modR[files[i].mod]) { whatifForget(doc.files[i].path); return; }
  let target = -1;
  for (let m = 0; m < N; m++) if (m !== files[i].mod && !hiddenMods[m] && Math.hypot(x - mx[m], y - my[m]) <= modR[m] && (target < 0 || modR[m] < modR[target])) target = m;
  const m = files[i].mod;
  lx[i] = originalLocal[0]; ly[i] = originalLocal[1];
  files[i].x = mx[m] + lx[i]; files[i].y = my[m] + ly[i]; fileEdges(i, new Set());
  if (target < 0) {
    whatifPending = null; whatifShow();
    whatifUI.status.textContent = "Drop inside another module, or enter a destination folder.";
    buildGrid(); redraw(); return;
  }
  whatifUI.file.value = doc.files[i].path;
  whatifUI.folder.value = modRoot[target].slice(0, -1) || ".";
  runWhatif([x, y]);
}
/* Every counted metric here is lower-is-better, so a rise is red and a drop is green. */
function whatifTable(rows, scored) {
  const table = el("table", { class: "sim" });
  const heading = el("tr");
  for (const text of ["", "Before", "After"]) heading.append(el("th", { scope: "col" }, text));
  table.append(heading);
  for (const [label, before, after] of rows) {
    const tone = !scored || before === after ? "" : after > before ? "worse" : "better";
    const row = el("tr", before === after ? { class: "same" } : {});
    row.append(el("th", { scope: "row" }, label), el("td", {}, fmt(before)), el("td", tone ? { class: tone } : {}, fmt(after))); table.append(row);
  }
  return table;
}
function whatifVerdict(result) {
  const { before, after } = result, good = [], bad = [];
  const pairKey = pair => JSON.stringify(pair), was = new Set(before.two_way_pairs.map(pairKey)), now = new Set(after.two_way_pairs.map(pairKey));
  const removed = before.two_way_pairs.filter(p => !now.has(pairKey(p))).length, added = after.two_way_pairs.filter(p => !was.has(pairKey(p))).length;
  if (result.new_cycles.length) bad.push(count(result.new_cycles.length, "new or expanded cycle", "new or expanded cycles"));
  if (result.new_forbidden.length) bad.push(count(result.new_forbidden.length, "new forbidden import", "new forbidden imports"));
  if (added) bad.push(count(added, "new two-way pair", "new two-way pairs"));
  if (result.removed_cycles.length) good.push(count(result.removed_cycles.length, "cycle", "cycles") + " broken");
  if (result.removed_forbidden.length) good.push(count(result.removed_forbidden.length, "forbidden import", "forbidden imports") + " fixed");
  if (removed) good.push(count(removed, "two-way pair", "two-way pairs") + " removed");
  if (after.largest_cycle !== before.largest_cycle) (after.largest_cycle > before.largest_cycle ? bad : good).push(`largest module cycle ${before.largest_cycle} → ${after.largest_cycle}`);
  if (!unflagged && after.unplanned !== before.unplanned) (after.unplanned > before.unplanned ? bad : good).push(`unplanned imports ${before.unplanned} → ${after.unplanned}`);
  const [tone, title] = bad.length && good.length ? ["warn", "Mixed: some better, some worse"] : bad.length ? ["bad", "Makes the structure worse"]
    : good.length ? ["good", "Improves the structure"] : ["same", "No structural change"];
  return { tone: result.partial && tone !== "bad" ? "warn" : tone, title: title + (result.partial ? " (unverified)" : ""), good, bad };
}
/* "packages/common/{decorators, utils}" reads faster than repeating the shared prefix on every member. */
function whatifCycle(cycle) {
  const parts = cycle.members.map(m => m.split("/")), keep = cycle.level === "file" ? 1 : 0;
  let n = 0;
  while (parts.every(p => n < p.length - keep && p[n] === parts[0][n])) n++;
  const prefix = parts[0].slice(0, n).join("/");
  const item = el("li");
  item.append(el("b", {}, `${cycle.level === "file" ? "File" : "Module"} cycle · ${cycle.members.length}`),
    el("span", { class: "muted" }, prefix ? ` in ${prefix}/` : ""),
    el("div", { class: "path" }, parts.map(p => p.slice(n).join("/") || "(root)").join(" · ")));
  return item;
}
function whatifForbidden(edge) {
  const item = el("li");
  item.append(el("b", {}, `Forbidden · ${edge.rule || "rule"}`), el("div", { class: "path" }, `${edge.from} → ${edge.to}`));
  return item;
}
function whatifList(cycles, forbidden, tone) {
  const list = el("ul", { class: "wi-findings " + tone });
  for (const cycle of cycles) list.append(whatifCycle(cycle));
  for (const edge of forbidden) list.append(whatifForbidden(edge));
  return list;
}
function whatifDetails(summary, tone, ...body) {
  const box = el("details", tone ? { class: tone } : {});
  box.append(el("summary", {}, summary), ...body);
  return box;
}
/* The engine reports one grab-bag warning per module; one line naming them all is enough. */
function whatifWarnings(warnings) {
  const list = el("ul"), grabbag = [];
  for (const text of warnings) {
    const hit = /^Grab-bag refinement required for (.+); raw cycle preview only\.$/.exec(text);
    if (hit) grabbag.push(hit[1]); else list.append(el("li", {}, text));
  }
  if (grabbag.length) list.append(el("li", {}, `Grab-bag refinement skipped, raw cycles only: ${grabbag.join(", ")}`));
  return list;
}
/* A file already in the list may be named by its previewed path; moving it home removes it. */
function runWhatif(at) {
  const file = whatifUI.file.value, folder = whatifUI.folder.value.replace(/\/$/, "");
  const source = whatifMoves.find(m => m.to === file)?.file ?? file;
  const to = (folder === "." || folder === "" ? "" : folder + "/") + basename(file);
  if (to === source && whatifMoves.some(m => m.file === source)) { whatifForget(source); return; }
  whatifPending = { file: source, to, at };
  whatifShow();
}
/* Previews every listed move plus the pending one, applied together; the saved graph is never changed. */
function whatifShow() {
  clearWhatif();
  const moves = whatifPlan();
  if (!moves.length) { whatifListRender(); return; }
  try {
    const result = ArchgraphPreview.simulate(doc, moves.map(({ file, to }) => ({ file, to })));
    whatifResult = result;
    result.moves.forEach((move, k) => {
      const i = fileIndex.get(move.file), m = modIndex.get(move.module);
      cancelSettle("f" + i);
      whatifSaved.push({ i, local: [lx[i], ly[i]], path: files[i].path });
      files[i].path = move.to;
      let point = moves[k].at || (m === undefined ? [files[i].x, files[i].y] : [mx[m], my[m]]);
      const open = point => files.every(f => f.i === i || Math.hypot(f.x - point[0], f.y - point[1]) >= R_DOT * 2 + 2);
      if (m !== undefined && !open(point)) {
        const slot = sunflower(48, SP * .8).map(p => [point[0] + p.x, point[1] + p.y])
          .find(p => Math.hypot(p[0] - mx[m], p[1] - my[m]) <= fileRoom(m) && open(p));
        if (slot) point = slot;
      }
      files[i].x = point[0]; files[i].y = point[1];
      lx[i] = point[0] - mx[files[i].mod]; ly[i] = point[1] - my[files[i].mod];
      fileEdges(i, new Set());
    });
    whatifBuildDrawing(result);
    const before = result.before, after = result.after, verdict = whatifVerdict(result);
    const banner = el("div", { class: "verdict " + verdict.tone });
    banner.append(el("strong", {}, verdict.title));
    const chips = el("div", { class: "chips" });
    for (const text of verdict.bad) chips.append(el("span", { class: "chip bad" }, text));
    for (const text of verdict.good) chips.append(el("span", { class: "chip good" }, text));
    if (verdict.bad.length || verdict.good.length) banner.append(chips);
    whatifUI.result.append(banner, ...result.moves.map(({ file, to, module }) => el("p", { class: "path muted" }, `${file} → ${to}` + (module === dirname(to) ? "" : `  (module ${module})`))));
    if (result.partial) whatifUI.result.append(whatifDetails("⚠ Estimate only: confirm with the CLI", "warn", whatifWarnings(result.warnings)));
    if (result.moves.some(move => !modIndex.has(move.module))) whatifUI.result.append(el("p", { class: "muted" }, "Creates a new module. The map keeps the original module positions."));
    whatifUI.result.append(whatifTable([
      ["Cycles", before.cycles.length, after.cycles.length], ["Largest module cycle", before.largest_cycle, after.largest_cycle],
      ["Two-way pairs", before.two_way_pairs.length, after.two_way_pairs.length], ["Forbidden imports", before.forbidden, after.forbidden], ["Unplanned imports", before.unplanned, after.unplanned]
    ], true));
    const fresh = new Set([...result.new_cycles, ...result.new_forbidden]);
    if (fresh.size) whatifUI.result.append(el("h3", { class: "worse" }, "New problems"), whatifList(result.new_cycles, result.new_forbidden, "bad"));
    if (result.removed_cycles.length || result.removed_forbidden.length) whatifUI.result.append(el("h3", { class: "better" }, "Resolved"), whatifList(result.removed_cycles, result.removed_forbidden, "good"));
    const cycles = after.cycles.filter(c => !fresh.has(c)), forbidden = after.verdicts.filter(v => v.verdict === "forbidden" && !fresh.has(v));
    const rest = cycles.length + forbidden.length;
    if (rest) {
      const shown = cycles.slice(0, 12), list = whatifList(shown, forbidden.slice(0, 12 - shown.length), "");
      if (rest > 12) list.append(el("li", { class: "muted" }, `${rest - 12} more in the CLI report.`));
      whatifUI.result.append(whatifDetails(`Unchanged findings · ${rest}`, "", list));
    } else if (!fresh.size) whatifUI.result.append(el("p", { class: "muted" }, "No cycle or forbidden-import findings after this move."));
    const changed = [...new Set([...Object.keys(before.modules), ...Object.keys(after.modules)])].filter(id => JSON.stringify(before.modules[id]) !== JSON.stringify(after.modules[id]));
    const metrics = changed.slice(0, 12).map(id => [id, before.modules[id]?.instability ?? null, after.modules[id]?.instability ?? null]);
    if (metrics.length) whatifUI.result.append(whatifDetails(`Instability · ${count(changed.length, "module", "modules")} changed`, "", whatifTable(metrics, false)));
    if (doc.target && after.target) {
      // Live verdicts and progress; reasons, sources, the move cost and what the preview cannot recompute come from the scan.
      const scan = new Map(doc.target.claims.map(c => [c.id, c])), box = el("div"), { conforming, moved } = after.target.progress;
      targetContext({ ...doc.target, claims: after.target.claims.map(c => c.preview === false
        ? { ...scan.get(c.id), notes: ["package rules are not previewed"] }
        : { ...c, reason: scan.get(c.id).reason, source: scan.get(c.id).source, notes: [] }),
      progress: { conforming: conforming.preview === false ? { ...doc.target.progress.conforming, preview: false } : conforming, ...(moved && { moved }) },
      notes: [] }, box, doc.target.progress);
      whatifUI.result.append(box);
    }
    whatifUI.commands.hidden = false; whatifCommands();
    const text = `Preview: ${count(after.cycles.length, "cycle", "cycles")}, ${count(after.forbidden, "forbidden import", "forbidden imports")}. Original report retained; Rust verification required.`;
    $("stats").replaceChildren(el("span", {}, text)); $("stats-text").textContent = text;
    whatifUI.status.textContent = "";
    $("announce").textContent = [verdict.title, ...verdict.bad, ...verdict.good].join(". ");
    whatifListRender();
    buildGrid(); applyFocus(); redraw();
  } catch (error) {
    /* A rejected pending move drops back to the list's preview; the list was valid when added. */
    const pending = whatifPending;
    whatifPending = null;
    if (pending && whatifMoves.length) whatifShow(); else { clearWhatif(); whatifListRender(); }
    whatifUI.status.textContent = "Cannot preview: " + error.message;
  }
}
function whatifBuildDrawing(result) {
  const cycleOf = new Map();
  result.after.cycles.filter(c => c.level === "module").forEach((cycle, i) => cycle.members.forEach(id => cycleOf.set(id, i)));
  const fileCycles = new Map();
  result.after.cycles.filter(c => c.level === "file").forEach((cycle, i) => cycle.members.forEach(id => fileCycles.set(id, i)));
  const moved = new Map(result.moves.map(m => [m.file, m]));
  const renamed = path => moved.get(path)?.to ?? path;
  const moduleOf = i => moved.get(doc.files[i].path)?.module ?? doc.files[i].module;
  const verdicts = new Map();
  for (const edge of result.after.verdicts) {
    const key = JSON.stringify([edge.from, edge.to]), old = verdicts.get(key);
    if (!old || edge.verdict === "forbidden" || (edge.verdict === "unplanned" && old !== "forbidden")) verdicts.set(key, edge.verdict);
  }
  const incident = new Set(), aggregate = new Map(), previewFiles = [], counts = new Map();
  for (const f of files) { const id = moduleOf(f.i); counts.set(id, (counts.get(id) || 0) + 1); }
  for (const edge of edges) {
    const from = moduleOf(edge.a), to = moduleOf(edge.b), a = renamed(doc.files[edge.a].path), b = renamed(doc.files[edge.b].path);
    const verdict = verdicts.get(JSON.stringify([a, b])), tone = verdict === "forbidden" || (verdict === "unplanned" && !unflagged) ? verdict : "neutral";
    if (tone === "forbidden") { incident.add(from); incident.add(to); }
    const cycle = runtime(edge) && (cycleOf.has(from) && cycleOf.get(from) === cycleOf.get(to) || fileCycles.has(a) && fileCycles.get(a) === fileCycles.get(b));
    previewFiles.push({ ...edge, tone, cycle, previewA: modIndex.get(from), previewB: modIndex.get(to) });
    if (from === to || !modIndex.has(from) || !modIndex.has(to)) continue;
    const key = JSON.stringify([from, to]);
    if (!aggregate.has(key)) aggregate.set(key, { a: modIndex.get(from), b: modIndex.get(to), tone: "neutral", cycle: false, typeOnly: true });
    const row = aggregate.get(key);
    if (tone === "forbidden" || tone === "unplanned" && row.tone !== "forbidden") row.tone = tone;
    row.cycle ||= cycle;
    row.typeOnly &&= edge.kind === "type" || edge.kind === "type-reexport";
  }
  mods.forEach((module, i) => { modSignal[i] = incident.has(module.id) ? "forbidden" : cycleOf.has(module.id) ? "cycle" : "none"; modHealth[i] = RING[modSignal[i]]; });
  const rows = [...aggregate.values()], geometry = new Float64Array(rows.length * 6), bounds = new Float64Array(rows.length * 4);
  rows.forEach((row, i) => curve(geometry, bounds, i, mx[row.a], my[row.a], mx[row.b], my[row.b], modR[row.a], modR[row.b] + 1, .12));
  whatifDrawing = { rows, geometry, bounds, files: previewFiles, counts };
}
function putWhatifLinks(put, marks, fileT, seen) {
  if (fileT < 1) whatifDrawing.rows.forEach((row, i) => {
    if (hiddenMods[row.a] || hiddenMods[row.b] || !seen(whatifDrawing.bounds, i * 4, 2)) return;
    const color = row.tone === "neutral" && row.cycle ? C.amber : TONE[row.tone];
    put(whatifDrawing.geometry, i, row.tone === "neutral" ? 1 : 2, color, .7 * (1 - fileT), 1.4, row.typeOnly, 6);
  });
  if (fileT > 0) for (const edge of whatifDrawing.files) {
    if (hiddenMods[edge.previewA] || hiddenMods[edge.previewB] || !seen(FB, edge.i * 4, 2)) continue;
    const color = edge.tone === "neutral" && edge.cycle ? C.amber : TONE[edge.tone];
    put(FE, edge.i, edge.tone === "neutral" ? 1 : 2, color, .7 * fileT, 1, edge.kind === "type" || edge.kind === "type-reexport", 5);
  }
}
function whatifTip(h) {
  if (h.type === "module") {
    const module = mods[h.i], metrics = whatifResult.after.modules[module.id];
    return [module.id, "Move preview: " + HEALTH_TEXT[modSignal[h.i]], metrics ? `imported by ${metrics.ca}; imports ${metrics.ce}; instability ${fmt(metrics.instability)}` : "Module disappears in the preview."];
  }
  if (h.type === "file") return [files[h.i].path, "Preview module: " + (whatifResult.moves.find(m => m.file === doc.files[h.i].path)?.module ?? doc.files[h.i].module)];
  return ["Move preview", "Use the preview panel for current cycle and rule findings."];
}
$("whatif-toggle").addEventListener("click", () => setWhatifMode(!whatifMode));
$("whatif-close").addEventListener("click", () => setWhatifMode(false));
$("whatif-clear").addEventListener("click", whatifClearAll);
whatifUI.add.addEventListener("click", whatifAdd);
$("whatif-form").addEventListener("submit", event => { event.preventDefault(); runWhatif(); });
whatifUI.graph.addEventListener("input", whatifCommands);
$("whatif-copy-cli").addEventListener("click", () => whatifCopy(whatifUI.cli));
$("whatif-copy-git").addEventListener("click", () => whatifCopy(whatifUI.git));
$("whatif-copy-rules").addEventListener("click", () => whatifCopy(whatifUI.rules));
$("whatif-copy-moves").addEventListener("click", () => whatifCopy(whatifUI.moves));
