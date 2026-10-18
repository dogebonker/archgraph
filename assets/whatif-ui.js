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
  if (after.unplanned !== before.unplanned) (after.unplanned > before.unplanned ? bad : good).push(`unplanned imports ${before.unplanned} → ${after.unplanned}`);
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
    const before = result.before, after = result.after, verdict = whatifVerdict(result);
    const banner = el("div", { class: "verdict " + verdict.tone });
    banner.append(el("strong", {}, verdict.title));
    const chips = el("div", { class: "chips" });
    for (const text of verdict.bad) chips.append(el("span", { class: "chip bad" }, text));
    for (const text of verdict.good) chips.append(el("span", { class: "chip good" }, text));
    if (verdict.bad.length || verdict.good.length) banner.append(chips);
    whatifUI.result.append(banner, el("p", { class: "path muted" }, `${file} → ${to}` + (result.move.module === dirname(to) ? "" : `  (module ${result.move.module})`)));
    if (result.partial) whatifUI.result.append(whatifDetails("⚠ Estimate only: confirm with the CLI", "warn", whatifWarnings(result.warnings)));
    if (m === undefined) whatifUI.result.append(el("p", { class: "muted" }, "Creates a new module. The map keeps the original module positions."));
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
    whatifUI.commands.hidden = false; whatifCommands();
    const text = `Preview: ${count(after.cycles.length, "cycle", "cycles")}, ${count(after.forbidden, "forbidden import", "forbidden imports")}. Original report retained; Rust verification required.`;
    $("stats").replaceChildren(el("span", {}, text)); $("stats-text").textContent = text;
    whatifUI.status.textContent = "";
    $("announce").textContent = [verdict.title, ...verdict.bad, ...verdict.good].join(". ");
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
