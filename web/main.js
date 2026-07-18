import init, { generate, random_tags } from "./pkg/maps_wasm.js";

const TAG_GROUPS = [
  { name: "size", options: ["small", "medium", "large"] },
  { name: "layout", options: ["hub", "chamber", "burrow"] },
  { name: "shape", options: ["cavities", "coral", "chaotic"] },
  { name: "links", options: ["tree", "connected"] },
  { name: "exits", options: ["sealed", "entrance", "passage", "junction"] },
  { name: "water", options: ["wet", "dry"] },
  { name: "ruins", options: ["ruins", "organic"] },
];
const MODES = ["cave", "forest"];
const GRIDS = ["hex", "square", "none"];

const $ = (id) => document.getElementById(id);
// Seeds are u64 — always strings on this side of the boundary.
const randSeed = () =>
  crypto.getRandomValues(new BigUint64Array(1))[0].toString();

const state = {
  seed: randSeed(),
  shapeSeed: null, // pinned sub-seeds (null = derive from master)
  decorSeed: null,
  nameSeed: null,
  last: null, // last generate() output
};

/* ---------- build controls ---------- */

function radioRow(container, name, options, checked, labels = {}) {
  container.insertAdjacentHTML(
    "beforeend",
    `<b>${name}<span class="seed-default"></span></b>` +
      options
        .map(
          (o) =>
            `<label data-value="${o}"><input type="radio" name="${name}" value="${o}"` +
            (o === checked ? " checked" : "") +
            `> ${labels[o] ?? (o || "—")}</label>`,
        )
        .join(""),
  );
}

for (const g of TAG_GROUPS) {
  const div = document.createElement("div");
  div.className = "tag-group";
  div.dataset.group = g.name;
  // "" = auto: use whatever this seed rolled for the family.
  radioRow(div, g.name, ["", ...g.options], "");
  $("tag-groups").appendChild(div);
}
radioRow($("mode-row"), "mode", MODES, "cave");
radioRow($("grid-row"), "grid", GRIDS, "hex");

const radioValue = (name) =>
  document.querySelector(`input[name="${name}"]:checked`)?.value ?? "";
const setRadio = (name, value) => {
  const el = document.querySelector(`input[name="${name}"][value="${value}"]`);
  if (el) el.checked = true;
};

/* ---------- seed tag defaults ---------- */

// family -> tag the current seed rolls (undefined = family untagged).
function seedDefaults() {
  const rolled = random_tags(state.seed).split(",").filter(Boolean);
  const byFamily = {};
  for (const g of TAG_GROUPS) {
    byFamily[g.name] = g.options.find((o) => rolled.includes(o));
  }
  return byFamily;
}

function annotateDefaults(defaults) {
  for (const g of TAG_GROUPS) {
    const group = document.querySelector(`.tag-group[data-group="${g.name}"]`);
    group.querySelector(".seed-default").textContent =
      ` · seed: ${defaults[g.name] ?? "—"}`;
    for (const label of group.querySelectorAll("label")) {
      label.classList.toggle(
        "seed-pick",
        label.dataset.value === (defaults[g.name] ?? ""),
      );
    }
  }
}

/* ---------- generate ---------- */

function collectOptions(defaults) {
  const o = { seed: state.seed, mode: radioValue("mode"), grid: radioValue("grid") };
  if (state.shapeSeed) o.shapeSeed = state.shapeSeed;
  if (state.decorSeed) o.decorSeed = state.decorSeed;
  if (state.nameSeed) o.nameSeed = state.nameSeed;
  // Merge: auto ("") families take the seed's roll, others the selection.
  o.tags = TAG_GROUPS.map((g) => radioValue(g.name) || defaults[g.name])
    .filter(Boolean)
    .join(",");
  if (!$("water-auto").checked) o.waterLevel = $("water").value / 100;
  if (!$("ruins-auto").checked) o.ruinsLevel = $("ruins").value / 100;
  const name = $("map-name").value.trim();
  if (name) o.title = name;
  return o;
}

function render() {
  const defaults = seedDefaults();
  annotateDefaults(defaults);
  $("reroll-name").disabled = $("map-name").value.trim() !== "";
  let out;
  try {
    out = generate(collectOptions(defaults));
  } catch (e) {
    $("map-pane").innerHTML = `<p class="hint">${e}</p>`;
    return;
  }
  state.last = out;
  $("map-pane").innerHTML = out.svg;
  $("map-title").textContent = out.title;
  $("map-tags").textContent = out.tags === "(none)" ? "untagged" : out.tags;
  $("map-seeds").textContent =
    `seed ${out.seed} · shape ${out.shapeSeed} · decor ${out.decorSeed} · name ${out.nameSeed}`;
  updateHash(out);
}

/* ---------- permalink ---------- */

function updateHash(out) {
  const p = new URLSearchParams({
    seed: out.seed,
    shape: out.shapeSeed,
    decor: out.decorSeed,
    name: out.nameSeed,
    tags: out.tags === "(none)" ? "" : out.tags.replaceAll(" ", ","),
    auto: TAG_GROUPS.filter((g) => radioValue(g.name) === "")
      .map((g) => g.name)
      .join(","),
    mode: radioValue("mode"),
    grid: radioValue("grid"),
  });
  if (!$("water-auto").checked) p.set("water", $("water").value / 100);
  if (!$("ruins-auto").checked) p.set("ruins", $("ruins").value / 100);
  const name = $("map-name").value.trim();
  if (name) p.set("title", name);
  history.replaceState(null, "", "#" + p.toString());
}

function loadHash() {
  if (location.hash.length < 2) return;
  const p = new URLSearchParams(location.hash.slice(1));
  if (p.get("seed")) state.seed = p.get("seed");
  state.shapeSeed = p.get("shape");
  state.decorSeed = p.get("decor");
  state.nameSeed = p.get("name");
  if (p.has("tags")) {
    const tags = p.get("tags").split(",").filter(Boolean);
    const auto = (p.get("auto") ?? "").split(",");
    for (const g of TAG_GROUPS) {
      const explicit = g.options.find((o) => tags.includes(o));
      setRadio(g.name, auto.includes(g.name) ? "" : (explicit ?? ""));
    }
  }
  if (p.get("mode")) setRadio("mode", p.get("mode"));
  if (p.get("grid")) setRadio("grid", p.get("grid"));
  if (p.get("title")) $("map-name").value = p.get("title");
  for (const [key, auto, slider, label] of [
    ["water", "water-auto", "water", "water-val"],
    ["ruins", "ruins-auto", "ruins", "ruins-val"],
  ]) {
    if (p.get(key) !== null) {
      $(auto).checked = false;
      $(slider).disabled = false;
      $(slider).value = Math.round(parseFloat(p.get(key)) * 100);
      $(label).textContent = parseFloat(p.get(key)).toFixed(2);
    }
  }
}

/* ---------- wire up ---------- */

$("new-map").onclick = () => {
  state.seed = randSeed();
  state.shapeSeed = state.decorSeed = state.nameSeed = null;
  $("seed").value = state.seed;
  render();
};
$("seed").onchange = () => {
  state.seed = $("seed").value.trim() || "0";
  state.shapeSeed = state.decorSeed = state.nameSeed = null;
  render();
};
for (const stream of ["shape", "decor", "name"]) {
  $(`reroll-${stream}`).onclick = () => {
    if (!state.last) return;
    // Pin the other two streams to their last effective values.
    state.shapeSeed = state.last.shapeSeed;
    state.decorSeed = state.last.decorSeed;
    state.nameSeed = state.last.nameSeed;
    state[`${stream}Seed`] = randSeed();
    render();
  };
}
document
  .querySelectorAll("#tag-groups input, #mode-row input, #grid-row input")
  .forEach((el) => (el.onchange = render));
$("map-name").onchange = render;
for (const [auto, slider, label] of [
  ["water-auto", "water", "water-val"],
  ["ruins-auto", "ruins", "ruins-val"],
]) {
  $(auto).onchange = () => {
    $(slider).disabled = $(auto).checked;
    render();
  };
  $(slider).oninput = () => {
    $(label).textContent = ($(slider).value / 100).toFixed(2);
  };
  $(slider).onchange = render;
}
$("export-svg").onclick = () => {
  if (!state.last) return;
  const blob = new Blob([state.last.svg], { type: "image/svg+xml" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download =
    state.last.title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") +
    ".svg";
  a.click();
  URL.revokeObjectURL(a.href);
};

/* ---------- boot ---------- */

await init();
loadHash();
$("seed").value = state.seed;
render();
