/* ---------- live move preview: transient, never written to the saved graph or layout ---------- */
const whatifUI = {
  box: $("whatif-panel"), file: $("whatif-file"), folder: $("whatif-folder"),
  status: $("whatif-status"), result: $("whatif-result"), graph: $("whatif-graph"),
  commands: $("whatif-commands"), cli: $("whatif-cli"), git: $("whatif-git")
};
const whatifOriginal = { stats: [...$("stats").childNodes], text: $("stats-text").textContent,
  signals: modSignal.slice(), health: modHealth.slice() };
let whatifSaved = null, whatifDrawing = null, whatifChoices = false, whatifChurn = false;
const whatifQuote = text => "'" + text.replace(/'/g, "'\\''") + "'";
function whatifCommands() {
  if (!whatifResult) return;
  const { file, to } = whatifResult.move;
  const graph = whatifUI.graph.value.startsWith("-") ? "./" + whatifUI.graph.value : whatifUI.graph.value;
  whatifUI.cli.value = `archgraph simulate ${whatifQuote(graph)} --move ${whatifQuote(file)} ${whatifQuote(to)}`;
  const directory = dirname(to);
  whatifUI.git.value = (directory ? `mkdir -p -- ${whatifQuote(directory)}\n` : "") + `git mv -- ${whatifQuote(file)} ${whatifQuote(to)}`;
}
async function whatifCopy(field) {
  try { await navigator.clipboard.writeText(field.value); whatifUI.status.textContent = "Copied. Review and run from the scanned root."; }
  catch { field.focus(); field.select(); whatifUI.status.textContent = "Clipboard unavailable. Copy the selected command."; }
}
function clearWhatif() {
  if (whatifSaved) {
    const { i, local, path } = whatifSaved, m = files[i].mod;
    files[i].path = path;
    lx[i] = local[0]; ly[i] = local[1];
    files[i].x = mx[m] + lx[i]; files[i].y = my[m] + ly[i];
    fileEdges(i, new Set());
  }
  whatifSaved = null; whatifResult = null; whatifDrawing = null;
  modSignal.splice(0, modSignal.length, ...whatifOriginal.signals);
  modHealth.splice(0, modHealth.length, ...whatifOriginal.health);
  $("stats").replaceChildren(...whatifOriginal.stats);
  $("stats-text").textContent = whatifOriginal.text;
  whatifUI.result.replaceChildren(); whatifUI.commands.hidden = true;
  whatifUI.cli.value = ""; whatifUI.git.value = "";
  whatifUI.status.textContent = "Preview cleared; original graph restored.";
  buildGrid(); applyFocus(); redraw();
}
function setWhatifMode(on) {
  clearWhatif();
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
function whatifFolder(module) {
  const dirs = modFiles[module].map(i => dirname(doc.files[i].path).split("/").filter(Boolean));
  const prefix = [...(dirs[0] || [])];
  for (const dir of dirs) while (prefix.some((part, i) => dir[i] !== part)) prefix.pop();
  return prefix.join("/") || ".";
}
function whatifDrop(i, x, y, originalLocal) {
  if (whatifSaved?.i === i && Math.hypot(x - mx[files[i].mod], y - my[files[i].mod]) <= modR[files[i].mod]) { clearWhatif(); return; }
  let target = -1;
  for (let m = 0; m < N; m++) if (m !== files[i].mod && !hiddenMods[m] && Math.hypot(x - mx[m], y - my[m]) <= modR[m] && (target < 0 || modR[m] < modR[target])) target = m;
  const m = files[i].mod;
  lx[i] = originalLocal[0]; ly[i] = originalLocal[1];
  files[i].x = mx[m] + lx[i]; files[i].y = my[m] + ly[i]; fileEdges(i, new Set());
  if (target < 0) {
    clearWhatif();
    whatifUI.status.textContent = "Drop inside another module, or enter a destination folder.";
    buildGrid(); redraw(); return;
  }
  whatifUI.file.value = doc.files[i].path;
  whatifUI.folder.value = whatifFolder(target);
  runWhatif([x, y]);
}
function whatifTable(rows) {
  const table = el("table", { class: "sim" });
  const heading = el("tr");
  for (const text of ["Preview", "Before", "After"]) heading.append(el("th", { scope: "col" }, text));
  table.append(heading);
  for (const [label, before, after] of rows) {
    const row = el("tr"); row.append(el("th", { scope: "row" }, label), el("td", {}, fmt(before)), el("td", {}, fmt(after))); table.append(row);
  }
  return table;
}
function whatifFindings(result) {
  const list = el("ul"), findings = [];
  for (const cycle of result.after.cycles) findings.push(`${cycle.level} cycle: ${cycle.members.join(" ↔ ")}`);
  for (const edge of result.after.verdicts) if (edge.verdict === "forbidden") findings.push(`Forbidden: ${edge.from} → ${edge.to} (${edge.rule || "rule"})`);
  for (const text of findings.slice(0, 12)) list.append(el("li", {}, text));
  if (!findings.length) list.append(el("li", {}, "No cycle or forbidden-import findings in this preview."));
  if (findings.length > 12) list.append(el("li", {}, `${findings.length - 12} more; run the CLI for the full report.`));
  return list;
}
function runWhatif(at) {
  clearWhatif();
  try {
    const file = whatifUI.file.value, folder = whatifUI.folder.value.replace(/\/$/, "");
    const to = (folder === "." || folder === "" ? "" : folder + "/") + basename(file);
    const result = ArchgraphPreview.simulate(doc, { file, to });
    const i = fileIndex.get(file), m = modIndex.get(result.move.module);
    cancelSettle("f" + i);
    whatifSaved = { i, local: [lx[i], ly[i]], path: files[i].path };
    whatifResult = result;
    files[i].path = to;
    let point = at || (m === undefined ? [files[i].x, files[i].y] : [mx[m], my[m]]);
    const open = point => files.every(f => f.i === i || Math.hypot(f.x - point[0], f.y - point[1]) >= R_DOT * 2 + 2);
    if (m !== undefined && !open(point)) {
      const slot = sunflower(48, SP * .8).map(p => [point[0] + p.x, point[1] + p.y])
        .find(p => Math.hypot(p[0] - mx[m], p[1] - my[m]) <= fileRoom(m) && open(p));
      if (slot) point = slot;
    }
    files[i].x = point[0]; files[i].y = point[1];
    lx[i] = point[0] - mx[files[i].mod]; ly[i] = point[1] - my[files[i].mod];
    fileEdges(i, new Set());
    whatifBuildDrawing(result);
    const before = result.before, after = result.after;
    const pairKeys = new Set(after.two_way_pairs.map(pair => JSON.stringify(pair)));
    const removed = before.two_way_pairs.filter(pair => !pairKeys.has(JSON.stringify(pair))).length;
    const headline = `${count(removed, "two-way pair", "two-way pairs")} removed · largest module cycle ${before.largest_cycle} → ${after.largest_cycle} · ` +
      (result.partial ? "limited preview: verify cycles in Rust" : result.new_cycles.length ? `${count(result.new_cycles.length, "new or expanded cycle", "new or expanded cycles")}` : "no new cycle membership");
    whatifUI.result.append(el("h3", {}, headline), el("p", { class: "path" }, `${file} → ${to}`), el("p", {}, `Destination module: ${result.move.module}`));
    for (const warning of result.warnings) whatifUI.result.append(el("p", { class: "note" }, warning));
    if (m === undefined) whatifUI.result.append(el("p", { class: "note" }, "New module: results include it; the map retains its original module positions."));
    whatifUI.result.append(whatifTable([
      ["Cycles", before.cycles.length, after.cycles.length], ["Largest module cycle", before.largest_cycle, after.largest_cycle],
      ["Two-way pairs", before.two_way_pairs.length, after.two_way_pairs.length], ["Forbidden imports", before.forbidden, after.forbidden], ["Unplanned imports", before.unplanned, after.unplanned]
    ]));
    const changed = [...new Set([...Object.keys(before.modules), ...Object.keys(after.modules)])].filter(id => JSON.stringify(before.modules[id]) !== JSON.stringify(after.modules[id]));
    const metrics = changed.slice(0, 12).map(id => [`${id} instability`, before.modules[id]?.instability ?? null, after.modules[id]?.instability ?? null]);
    if (metrics.length) whatifUI.result.append(whatifTable(metrics));
    whatifUI.result.append(el("h3", {}, "Preview cycle and rule findings"), whatifFindings(result));
    whatifUI.commands.hidden = false; whatifCommands();
    const text = `Preview: ${count(after.cycles.length, "cycle", "cycles")}, ${count(after.forbidden, "forbidden import", "forbidden imports")}. Original report retained; Rust verification required.`;
    $("stats").replaceChildren(el("span", {}, text)); $("stats-text").textContent = text;
    whatifUI.status.textContent = result.partial ? "Limited preview. Review the warnings and verify with Rust." : "Preview ready. The original graph is retained.";
    $("announce").textContent = headline;
    buildGrid(); applyFocus(); redraw();
  } catch (error) {
    clearWhatif(); whatifUI.status.textContent = "Cannot preview: " + error.message;
  }
}
function whatifBuildDrawing(result) {
  const cycleOf = new Map();
  result.after.cycles.filter(c => c.level === "module").forEach((cycle, i) => cycle.members.forEach(id => cycleOf.set(id, i)));
  const fileCycles = new Map();
  result.after.cycles.filter(c => c.level === "file").forEach((cycle, i) => cycle.members.forEach(id => fileCycles.set(id, i)));
  const renamed = path => path === result.move.file ? result.move.to : path;
  const moduleOf = i => i === whatifSaved.i ? result.move.module : doc.files[i].module;
  const verdicts = new Map();
  for (const edge of result.after.verdicts) {
    const key = JSON.stringify([edge.from, edge.to]), old = verdicts.get(key);
    if (!old || edge.verdict === "forbidden" || (edge.verdict === "unplanned" && old !== "forbidden")) verdicts.set(key, edge.verdict);
  }
  const incident = new Set(), aggregate = new Map(), previewFiles = [], counts = new Map();
  for (const f of files) { const id = moduleOf(f.i); counts.set(id, (counts.get(id) || 0) + 1); }
  for (const edge of edges) {
    const from = moduleOf(edge.a), to = moduleOf(edge.b), a = renamed(doc.files[edge.a].path), b = renamed(doc.files[edge.b].path);
    const verdict = verdicts.get(JSON.stringify([a, b])), tone = verdict === "forbidden" || verdict === "unplanned" ? verdict : "neutral";
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
  if (h.type === "file") return [files[h.i].path, "Preview module: " + (h.i === whatifSaved.i ? whatifResult.move.module : doc.files[h.i].module)];
  return ["Move preview", "Use the preview panel for current cycle and rule findings."];
}
$("whatif-toggle").addEventListener("click", () => setWhatifMode(!whatifMode));
$("whatif-close").addEventListener("click", () => setWhatifMode(false));
$("whatif-clear").addEventListener("click", clearWhatif);
$("whatif-form").addEventListener("submit", event => { event.preventDefault(); runWhatif(); });
whatifUI.graph.addEventListener("input", whatifCommands);
$("whatif-copy-cli").addEventListener("click", () => whatifCopy(whatifUI.cli));
$("whatif-copy-git").addEventListener("click", () => whatifCopy(whatifUI.git));
