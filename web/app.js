// The interface. It holds no state the engine owns and owns no state the engine
// holds: commands go one way, telemetry comes the other, and the only thing kept
// here is what a hand is currently touching.
//
// Two rules that follow from that, and both matter:
//
//  * A control that a hand is holding must not follow telemetry, or it fights
//    the finger dragging it. Hence `held`.
//  * Everything else is drawn from the newest frame and nothing older. A meter
//    has no use for a stale reading.

const $ = (id) => document.getElementById(id);

/** What the engine is playing: {kind, name}. kind is piece, clip, lane or patch. */
let stage = { kind: "piece", name: "" };
/** The library index, as the server last listed it. */
let library = { pieces: [], patches: [], clips: [] };
/** Bars the staged material runs for — the piece's length, or one clip's. */
let stageBars = 0;

/** The last frame that arrived, and the set description from the hello. */
let telemetry = null;
let description = null;
/** Controls a hand is currently on, by name — these stop following telemetry. */
const held = new Set();
/** Macro values this page is holding, or undefined where the curve is driving. */
const macroHeld = new Map();

let socket = null;
/** Which view is showing: "conductor", or a clip's name. */
let tab = "conductor";

function connect() {
  socket = new WebSocket(`ws://${location.host}/`);
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.t === "hello") {
      description = message;
      $("set").textContent = message.set;
      $("device").textContent = `${message.device} · ${message.buffer_frames} frames`;
      buildLanes(message.lanes);
      buildMacros(message.macros);
      send({ t: "library" });
      // The engine outlives the page, so it may already be looping a clip some
      // earlier page opened. Adopt what it is doing rather than assume: `show`
      // sends an audition only when the stage would change, so following it here
      // costs nothing and interrupts nothing.
      adoptStage(message.stage);
      // A page survives a reconnect with the tab it had open, and the engine
      // may have come back playing something else entirely — a different
      // piece, with different clips. Follow what is actually there; asking for
      // a clip that no longer exists just earns a complaint.
      // A hello also arrives when the piece changes, and the conductor and the
      // library are about the session rather than any one piece — staying put
      // is what you want when you are trying pieces one after another. Only a
      // clip tab can be orphaned by a new piece.
      const open = !isClip(tab) || clips().has(tab) ? tab : "conductor";
      show(stage.kind === "clip" && clips().has(stage.name) ? stage.name : open);
    } else if (message.t === "stage") {
      adoptStage(message);
    } else if (message.t === "library") {
      library = { pieces: message.pieces, patches: message.patches, clips: message.clips };
      if (tab === "library") buildLibrary();
    } else if (message.t === "complaint") {
      say(message.why, true);
    } else if (message.t === "telemetry") {
      telemetry = message.Telemetry ?? message;
    }
  };
  socket.onclose = () => {
    $("set").textContent = "reconnecting…";
    // The engine restarting should not need a refresh: that is most of the
    // reason the interface is a separate process.
    setTimeout(connect, 700);
  };
}

function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(message));
  }
}

// ── Building the controls, once the hello says what there is ─────────────────

function buildLanes(lanes) {
  const host = $("lanes");
  host.replaceChildren();
  lanes.forEach((lane, index) => {
    const name = document.createElement("button");
    name.className = "name" + (lane.muted ? " muted" : "");
    name.textContent = lane.name;
    name.title = `${lane.instrument}${lane.placed ? " · placed" : " · centre"}${
      lane.ducked ? " · ducked by the kick" : ""
    }`;
    name.onclick = () => {
      const muted = !name.classList.contains("muted");
      name.classList.toggle("muted", muted);
      send({ t: "mute", index, muted });
    };

    const fader = document.createElement("input");
    Object.assign(fader, { type: "range", min: 0, max: 2, step: 0.01, value: lane.gain });
    const readout = document.createElement("span");
    readout.className = "gain";
    readout.textContent = lane.gain.toFixed(2);
    fader.oninput = () => {
      readout.textContent = Number(fader.value).toFixed(2);
      send({ t: "level", index, gain: Number(fader.value) });
    };

    const meter = document.createElement("span");
    meter.className = "meter";
    const fill = document.createElement("i");
    meter.append(fill);
    meter.dataset.lane = index;

    host.append(name, fader, readout, meter);
  });
}

function buildMacros(macros) {
  const host = $("macros");
  host.replaceChildren();
  macros.forEach((macro, index) => {
    const box = document.createElement("div");
    box.className = "macro";

    const fader = document.createElement("input");
    Object.assign(fader, { type: "range", min: 0, max: 1, step: 0.005, value: macro.value ?? 0 });
    const value = document.createElement("span");
    value.className = "value";
    const label = document.createElement("span");
    label.className = "name";
    label.textContent = macro.name;

    // Under automation the curve owns the value and the fader follows it.
    // Touching the fader takes over; the button hands it back.
    const auto = document.createElement("button");
    auto.className = "auto";
    auto.textContent = "auto";
    auto.onclick = () => {
      if (macroHeld.has(index)) {
        macroHeld.delete(index);
        auto.classList.add("auto");
        auto.textContent = "auto";
        send({ t: "macro", index, value: null });
      } else {
        macroHeld.set(index, Number(fader.value));
        auto.classList.remove("auto");
        auto.textContent = "held";
        send({ t: "macro", index, value: Number(fader.value) });
      }
    };

    fader.oninput = () => {
      macroHeld.set(index, Number(fader.value));
      auto.classList.remove("auto");
      auto.textContent = "held";
      send({ t: "macro", index, value: Number(fader.value) });
    };

    box.append(fader, value, label, auto);
    box.dataset.macro = index;
    host.append(box);
  });
}

// ── Tabs ────────────────────────────────────────────────────────────────────

/** Lanes grouped by the clip they belong to, in the order they first appear. */
function clips() {
  const grouped = new Map();
  description?.lanes.forEach((lane, index) => {
    if (!grouped.has(lane.clip)) grouped.set(lane.clip, []);
    grouped.get(lane.clip).push({ ...lane, index });
  });
  return grouped;
}

function buildTabs() {
  const host = $("tabs");
  host.replaceChildren();
  const names = ["conductor", "library", ...clips().keys()];
  for (const name of names) {
    const button = document.createElement("button");
    button.textContent = name;
    button.className = name === tab ? "on" : "";
    button.onclick = () => show(name);
    host.append(button);
  }
}

// ── The library ─────────────────────────────────────────────────────────────

/**
 * A line of feedback, shown where the work is rather than in an alert.
 *
 * A save that worked and a name that was refused are both things you want to
 * read and then forget, so they say so for a few seconds and go. An alert would
 * block the page, which with audio running is exactly wrong.
 */
let noticeTimer = null;
function say(words, bad = false) {
  const notice = $("notice");
  notice.textContent = words;
  notice.classList.toggle("bad", bad);
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => (notice.textContent = ""), bad ? 8000 : 4000);
}


/** Everything saved under a name, each row a way into its editor. */
function buildLibrary() {
  const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  $("libraryInfo").textContent =
    `${count(library.pieces.length, "piece", "pieces")} · ` +
    `${count(library.clips.length, "clip", "clips")} · ` +
    `${count(library.patches.length, "patch", "patches")}`;

  // Pieces first, because this is now where you choose what is playing.
  const pieces = $("libraryPieces");
  pieces.replaceChildren();
  for (const piece of library.pieces) {
    const row = document.createElement("div");
    row.className = "libitem";
    row.dataset.piece = piece.name;

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = piece.name;
    const what = document.createElement("span");
    what.className = piece.built_in ? "weak built-in" : "weak";
    what.textContent = `${piece.bars.toFixed(0)} bars · ${piece.bpm.toFixed(0)} BPM`;
    what.title = piece.built_in
      ? `a generator in sets.rs, with no file to edit · ${piece.clips.join(", ")}`
      : piece.clips.join(", ");

    const play = document.createElement("button");
    const on = piece.name === $("set").textContent;
    play.textContent = on ? "playing" : "play";
    play.disabled = on;
    // Switching is a whole new engine, built off the audio thread and swapped
    // in over the same ten-millisecond fade an audition uses. The server
    // answers with a fresh hello and the page rebuilds from it.
    play.onclick = () => send({ t: "load", piece: piece.name });

    row.append(name, what, play);
    pieces.append(row);
  }
  if (!library.pieces.length) {
    pieces.append(empty("nothing in pieces/ and no built-ins"));
  }

  const patches = $("libraryPatches");
  patches.replaceChildren();
  for (const patch of library.patches) {
    const row = document.createElement("div");
    row.className = "libitem";
    row.dataset.patch = patch.name;

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = patch.name;
    const what = document.createElement("span");
    what.className = "weak";
    what.textContent = patch.instrument;

    // Hearing it is the only way to know what a saved patch is, so that is the
    // click: a plain test line, looping, nothing else playing.
    const hear = document.createElement("button");
    hear.textContent = "hear it";
    hear.title = "play this patch on a test line";
    hear.onclick = () => send({ t: "audition", patch: patch.name });

    row.append(name, what, hear);
    patches.append(row);
  }
  if (!library.patches.length) {
    patches.append(empty("nothing saved yet — open a clip, turn a fader, save the patch"));
  }

  const host = $("libraryClips");
  host.replaceChildren();
  const inPiece = new Set(clips().keys());
  for (const clip of library.clips) {
    const row = document.createElement("div");
    row.className = "libitem";

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = clip.name;
    const what = document.createElement("span");
    what.className = "weak";
    what.textContent =
      `${count(clip.lanes, "lane", "lanes")} · ${count(clip.steps, "step", "steps")}`;

    // A clip in the library that this piece does not use has no editor tab to
    // open: the piece is what an editor edits. Say so rather than offer a
    // button that cannot work.
    const open = document.createElement("button");
    if (inPiece.has(clip.name)) {
      open.textContent = "open";
      open.onclick = () => show(clip.name);
    } else {
      open.textContent = "not in this piece";
      open.disabled = true;
      open.title = "this piece does not use that clip";
    }

    row.append(name, what, open);
    host.append(row);
  }
  if (!library.clips.length) {
    host.append(empty("nothing saved yet — open a clip editor and save it"));
  }
  paintStage();
}

function empty(words) {
  const line = document.createElement("div");
  line.className = "weak";
  line.textContent = words;
  return line;
}

/** Mark whatever is currently sounding, wherever it is drawn. */
function paintStage() {
  document.querySelectorAll("[data-patch]").forEach((row) => {
    row.classList.toggle(
      "sounding",
      stage.kind === "patch" && row.dataset.patch === stage.name,
    );
  });
  const playing = $("set").textContent;
  document.querySelectorAll("[data-piece]").forEach((row) => {
    row.classList.toggle("playing", row.dataset.piece === playing);
  });
}

function show(name) {
  // An editor is not a view onto the piece, it is the only thing playing. The
  // server builds a small piece from this clip alone and hands it to the engine
  // in place of the real one; asking for the conductor hands the piece back, at
  // the bar it had reached.
  //
  // The library is an index, not an editor: it plays whatever you press "hear
  // it" on, so arriving there leaves the piece running.
  if (name === "conductor" && stage.kind !== "piece") {
    send({ t: "audition" });
  } else if (isClip(name) && !(stage.kind === "clip" && stage.name === name)) {
    send({ t: "audition", clip: name });
  }
  tab = name;
  for (const view of ["conductor", "clip", "library"]) {
    $(`view-${view}`).classList.toggle("hidden", view !== viewFor(name));
  }
  if (isClip(name)) buildClip(name);
  if (name === "library") {
    send({ t: "library" });
    buildLibrary();
  }
  buildTabs();
}

/** Take the server's word for what is playing, and say so. */
function adoptStage({ kind, name, bars }) {
  stage = { kind, name };
  stageBars = bars;
  $("scrub").max = bars;
  const length = `${bars} ${bars === 1 ? "bar" : "bars"}`;
  $("stage").textContent =
    {
      piece: "",
      clip: `playing this clip alone · ${length}`,
      lane: `playing ${name} alone · ${length}`,
      patch: `auditioning the ${name} patch · ${length}`,
    }[kind] ?? "";
  paintStage();
}


const isClip = (name) => name !== "conductor" && name !== "library";
const viewFor = (name) => (isClip(name) ? "clip" : name);

/**
 * The step grid: one row per lane of the clip, one cell per step.
 *
 * Clicking a cell places or clears a step, which goes straight to the engine as
 * a single `set_step` — the slot already exists there, so nothing has to be
 * allocated on the audio thread to change a loop while it plays.
 */
/**
 * A clip, drawn but not edited.
 *
 * The grid and the roll used to be editors — click a cell, place a step. They
 * are a viewer now: writing music happens in tab files through `nf`, and a
 * browser that could also write it meant the server owned a mutable document,
 * a dirty flag and a save path for the privilege of being the worse of the two
 * ways to do it.
 *
 * What is left is worth keeping. You cannot choose a clip to bring into a mix
 * without seeing what is in it, and you cannot judge a patch without hearing
 * it, so the shape stays and so does "hear it".
 */
function buildClip(name) {
  const lanes = clips().get(name) ?? [];
  $("clipName").textContent = name;
  const steps = Math.max(0, ...lanes.map((lane) => lane.steps.length));
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  $("clipInfo").textContent =
    `${plural(lanes.length, "lane", "lanes")} · ` +
    `${plural(clipBars(lanes), "bar", "bars")} · ${steps} steps`;

  const host = $("clipLanes");
  host.replaceChildren();
  for (const lane of lanes) {
    const row = document.createElement("div");
    row.className = "lane";
    row.dataset.lane = lane.index;

    const label = document.createElement("span");
    label.className = "name";
    label.textContent = lane.name;
    const what = document.createElement("span");
    what.className = "patch";
    what.textContent = lane.patch ?? lane.instrument;
    what.title = lane.patch
      ? `${lane.instrument} · patch ${lane.patch}`
      : `${lane.instrument} · no saved patch`;

    // A patch's parameters, hidden until asked for: a grid is about rhythm,
    // and ten faders per lane would bury it.
    const toggle = document.createElement("button");
    toggle.className = "expand";
    toggle.textContent = "▸";
    toggle.title = `show this patch's ${lane.params.length} parameters`;

    const head = document.createElement("div");
    head.className = "lanehead";
    head.append(label, what, toggle);

    row.append(head, lane.pitched ? pianoRoll(lane) : stepRow(lane));
    row.classList.toggle("tall", lane.pitched);
    host.append(row);

    const panel = buildPatch(lane);
    panel.hidden = true;
    host.append(panel);
    toggle.onclick = () => {
      panel.hidden = !panel.hidden;
      toggle.classList.toggle("on", !panel.hidden);
      toggle.textContent = panel.hidden ? "▸" : "▾";
    };
  }
}

/**
 * One fader per parameter, and they move the engine rather than the file.
 *
 * Turning a knob while it plays is a performance, not an edit: nothing is
 * written down and reloading the piece puts it back. That is the same thing a
 * desk means by the distinction, and it is why these can stay when the save
 * buttons went.
 *
 * The fader moves in the scale the parameter declared. A cutoff from 20 Hz to
 * 8 kHz on a linear fader spends nine tenths of its travel above 800 Hz, where
 * the ear hears almost nothing change; on a logarithmic one every octave gets
 * equal room, which is how pitch works.
 */
function buildPatch(lane) {
  const panel = document.createElement("div");
  panel.className = "patch";

  const head = document.createElement("div");
  head.className = "patchhead";
  const label = document.createElement("span");
  label.className = "pname";
  label.textContent = lane.patch ?? `${lane.instrument} · unsaved`;
  const solo = document.createElement("button");
  solo.textContent = "hear it";
  solo.title = "play this lane alone";
  solo.onclick = () => send({ t: "audition", lane: lane.index });
  head.append(label, solo);
  panel.append(head);

  lane.params.forEach((spec, index) => {
    const value = lane.values[index];
    const box = document.createElement("div");
    box.className = "param";
    box.title = spec.doc;

    const name = document.createElement("span");
    name.className = "pname";
    name.textContent = spec.name.replace(/_/g, " ");

    const fader = document.createElement("input");
    Object.assign(fader, { type: "range", min: 0, max: 1, step: 0.001 });
    const readout = document.createElement("span");
    readout.className = "pvalue";

    const toFader = (v) =>
      spec.logarithmic
        ? Math.log(Math.max(v, spec.min) / spec.min) / Math.log(spec.max / spec.min)
        : (v - spec.min) / (spec.max - spec.min);
    const fromFader = (f) =>
      spec.logarithmic
        ? spec.min * (spec.max / spec.min) ** f
        : spec.min + f * (spec.max - spec.min);

    const show = (v) => {
      const decimals = Math.abs(v) >= 100 ? 0 : Math.abs(v) >= 1 ? 2 : 3;
      readout.textContent = `${v.toFixed(decimals)}${spec.unit ? " " + spec.unit : ""}`.padStart(9);
      box.classList.toggle("moved", Math.abs(v - spec.default) > 1e-6);
    };

    fader.value = toFader(value);
    show(value);
    fader.oninput = () => {
      const now = fromFader(Number(fader.value));
      show(now);
      send({ t: "set_param", lane: lane.index, param: index, value: now });
    };

    box.append(name, fader, readout);
    panel.append(box);
  });
  return panel;
}

/**
 * A line every four steps and a double line every sixteen.
 *
 * A grid this long is read by counting, and counting past four without a mark
 * is how you lose your place — so the marks are where the beats and the bars
 * are, and the doubled one says which is which.
 */
function tickBefore(step) {
  if (step === 0 || step % 4 !== 0) return null;
  const tick = document.createElement("i");
  tick.className = step % 16 === 0 ? "tick bar" : "tick";
  return tick;
}

/** One row of boxes: a drum, where the only question per step is whether. */
function stepRow(lane) {
  const cells = document.createElement("div");
  cells.className = "steps";
  const gutter = document.createElement("span");
  gutter.className = "key";
  cells.append(gutter);
  lane.steps.forEach(([velocity], step) => {
    const tick = tickBefore(step);
    if (tick) cells.append(tick);
    const cell = document.createElement("div");
    cell.className = "step" + (step % 4 === 0 ? " beat" : "");
    cell.dataset.step = step;
    paint(cell, velocity);
    cells.append(cell);
  });
  return cells;
}

/**
 * A keyboard turned on its side: rows are semitones from the lane's root, high
 * at the top, columns are steps.
 *
 * A lane holds one offset per step, so a column can never have two notes in
 * it. Chords are lanes, which is why the prelude is three voices and the pad
 * is five. The rows span what the lane actually plays, padded out to at least
 * an octave: a hundred and twenty-eight rows of nothing would be honest and
 * useless.
 */
function pianoRoll(lane) {
  const played = lane.steps.filter(([v]) => v > 0).map(([, o]) => o);
  let low = played.length ? Math.min(...played) : 0;
  let high = played.length ? Math.max(...played) : 12;
  low -= 2;
  high += 2;
  while (high - low < 12) high += 1;

  const roll = document.createElement("div");
  roll.className = "roll";
  for (let offset = high; offset >= low; offset -= 1) {
    const line = document.createElement("div");
    line.className = "rollrow";
    const semitone = ((offset % 12) + 12) % 12;
    if ([1, 3, 6, 8, 10].includes(semitone)) line.classList.add("black");
    if (offset === 0) line.classList.add("root");

    const key = document.createElement("span");
    key.className = "key";
    key.textContent = NOTE_NAMES[semitone];
    key.title = `${offset >= 0 ? "+" : ""}${offset} semitones from the root`;
    line.append(key);

    lane.steps.forEach(([velocity, at], step) => {
      const tick = tickBefore(step);
      if (tick) line.append(tick);
      const cell = document.createElement("div");
      cell.className = "step" + (step % 4 === 0 ? " beat" : "");
      cell.dataset.step = step;
      cell.dataset.offset = offset;
      paint(cell, at === offset ? velocity : 0);
      line.append(cell);
    });
    roll.append(line);
  }
  return roll;
}

/** Bars a clip's longest lane covers, rounded up to a whole one. */
function clipBars(lanes) {
  const steps = Math.max(0, ...lanes.map((lane) => lane.steps.length));
  return Math.max(1, Math.ceil(steps / 16));
}

/** Semitone names from the root, so a row can say what it is. */
const NOTE_NAMES = ["R", "♭2", "2", "♭3", "3", "4", "♯4", "5", "♭6", "6", "♭7", "7"];

function paint(cell, velocity) {
  cell.dataset.velocity = velocity;
  cell.classList.toggle("on", velocity > 0);
  cell.classList.toggle("accent", velocity >= 1);
}

// ── Transport ───────────────────────────────────────────────────────────────

$("play").onclick = () => send({ t: "playing", value: !(telemetry?.playing ?? true) });
$("start").onclick = () => send({ t: "seek", bar: 0 });

const scrub = $("scrub");
for (const event of ["pointerdown", "keydown"]) scrub.addEventListener(event, () => held.add("scrub"));
for (const event of ["pointerup", "pointercancel", "keyup", "blur"])
  scrub.addEventListener(event, () => held.delete("scrub"));
scrub.oninput = () => send({ t: "seek", bar: Number(scrub.value) });

const bpm = $("bpm");
bpm.oninput = () => {
  $("bpmText").textContent = Number(bpm.value).toFixed(1);
  send({ t: "bpm", value: Number(bpm.value) });
};
$("master").oninput = (event) => send({ t: "master", value: Number(event.target.value) });

document.addEventListener("keydown", (event) => {
  if (event.code === "Space" && event.target.tagName !== "INPUT") {
    event.preventDefault();
    send({ t: "playing", value: !(telemetry?.playing ?? true) });
  }
});

// ── Drawing ─────────────────────────────────────────────────────────────────

/** Size a canvas to its CSS box at the screen's real pixel density. */
function fit(canvas) {
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth, height = canvas.clientHeight;
  if (canvas.width !== width * ratio || canvas.height !== height * ratio) {
    canvas.width = width * ratio;
    canvas.height = height * ratio;
  }
  const context = canvas.getContext("2d");
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  return { context, width, height };
}

const style = getComputedStyle(document.documentElement);
const colour = (name) => style.getPropertyValue(name).trim();

/** The arrangement: one row per lane, its spans drawn, with the playhead. */
function drawTimeline() {
  const { context, width, height } = fit($("timeline"));
  context.clearRect(0, 0, width, height);
  if (!description) return;

  // Names live in a gutter rather than on top of the bars: a label over a bar
  // is unreadable against either colour.
  const gutter = 74;
  const top = 4;
  const bars = description.length_bars;
  const rows = clips().size;
  const rowHeight = Math.min(16, (height - top - 16) / Math.max(rows, 1));
  const x = (bar) => gutter + (bar / bars) * (width - gutter - 4);

  context.font = `10px ${colour("--mono")}`;
  context.textBaseline = "middle";

  // Bar lines every four bars: the block dance music is written in.
  context.strokeStyle = colour("--line");
  context.lineWidth = 1;
  for (let bar = 0; bar <= bars; bar += 4) {
    context.beginPath();
    context.moveTo(x(bar), top);
    context.lineTo(x(bar), height - 12);
    context.stroke();
    context.fillStyle = colour("--line");
    context.fillText(bar, x(bar) + 2, height - 6);
  }

  const playhead = telemetry?.bar ?? -1;
  const inside = (from, to) => playhead >= from && playhead < to;

  [...clips()].forEach(([clip, lanes], index) => {
    const y = top + index * rowHeight;
    const middle = y + rowHeight / 2 - 1;
    // A clip is muted when every lane of it is.
    const muted = lanes.every((lane) => telemetry?.lanes?.[lane.index]?.muted);

    // Every span of every lane in the clip. A lane with no spans plays for
    // ever, which a piece for jamming wants.
    const spans = lanes.flatMap((lane) =>
      lane.spans.length ? lane.spans : [{ start: 0, end: bars, first_bar: 0 }],
    );
    // Whether a block is lit follows the playhead being inside it, not whether
    // a voice happens to be ringing this instant: a closed hat sounds for a
    // third of the time it is playing, so voice activity makes a block flicker
    // and says nothing about the arrangement.
    const live = spans.some((span) => inside(span.first_bar, span.end));

    context.fillStyle = muted ? colour("--line") : live ? colour("--text") : colour("--weak");
    context.textAlign = "right";
    context.fillText(clip, gutter - 6, middle);
    context.textAlign = "left";

    for (const span of spans) {
      // A flight's approach: the lane is sounding, but from somewhere else. It
      // is drawn thinner rather than fainter, so that it can still light up
      // when the playhead is in it.
      if (span.first_bar < span.start) {
        const flying = inside(span.first_bar, span.start);
        context.fillStyle = muted
          ? colour("--line")
          : colour("--accent") + (flying ? "aa" : "3a");
        context.fillRect(
          x(span.first_bar),
          y + rowHeight * 0.32,
          x(span.start) - x(span.first_bar),
          rowHeight * 0.36,
        );
      }
      const landed = inside(span.start, span.end);
      context.fillStyle = muted ? colour("--line") : colour("--accent") + (landed ? "e6" : "55");
      context.fillRect(x(span.start), y + 1, Math.max(x(span.end) - x(span.start), 1.5), rowHeight - 3);
    }
  });

  // The macro curves over the top, so the shape of the piece is visible at a
  // glance. Labelled at their left end, since four unlabelled lines say little.
  const curveTop = top;
  const curveBottom = height - 14;
  description.macros.forEach((macro, index) => {
    if (!macro.curve.length) return;
    const shade = ["--warn", "--good", "--bad", "--weak"][index % 4];
    context.strokeStyle = colour(shade) + "cc";
    context.lineWidth = 1.5;
    context.beginPath();
    macro.curve.forEach(([bar, value], i) => {
      const at = [x(bar), curveBottom - (curveBottom - curveTop) * value];
      i ? context.lineTo(...at) : context.moveTo(...at);
    });
    context.stroke();
    const [firstBar, firstValue] = macro.curve[0];
    context.fillStyle = colour(shade);
    context.fillText(macro.name, x(firstBar) + 3, curveBottom - (curveBottom - curveTop) * firstValue - 7);
  });

  if (telemetry && telemetry.loop_to > telemetry.loop_from) {
    context.fillStyle = colour("--good") + "22";
    context.fillRect(
      x(telemetry.loop_from),
      top,
      x(telemetry.loop_to) - x(telemetry.loop_from),
      height - 14 - top,
    );
  }

  if (telemetry) {
    context.strokeStyle = colour("--good");
    context.lineWidth = 1.5;
    context.beginPath();
    context.moveTo(x(telemetry.bar), top);
    context.lineTo(x(telemetry.bar), height - 12);
    context.stroke();
  }
}

/**
 * Where the sources are, from above. The radial scale is r/(r+8) rather than
 * linear: a helicopter starts 42 m out and lands on your head, and a linear
 * scale would spend its whole range on the approach.
 */
function drawPlan() {
  const { context, width, height } = fit($("plan"));
  context.clearRect(0, 0, width, height);
  const centre = { x: width / 2, y: height / 2 };
  const radius = Math.min(width, height) * 0.44;
  // Metres that map to half the radius. Three, not eight: almost everything
  // lives between one and three metres, and a helicopter at forty still has to
  // fit on the page. A linear scale would put the whole piece in a dot.
  const half = 3;
  const scale = (metres) => radius * (metres / (metres + half));

  context.strokeStyle = colour("--line");
  context.lineWidth = 1;
  context.font = `9px ${colour("--mono")}`;
  for (const metres of [1, 2, 4, 10, 40]) {
    const r = scale(metres);
    context.beginPath();
    context.arc(centre.x, centre.y, r, 0, Math.PI * 2);
    context.stroke();
    // On the diagonal, where nothing else wants to be.
    context.fillStyle = colour("--line");
    context.fillText(`${metres} m`, centre.x + r * 0.71 + 2, centre.y - r * 0.71 - 2);
  }
  for (const [dx, dy] of [[0, 1], [1, 0]]) {
    context.beginPath();
    context.moveTo(centre.x - dx * radius, centre.y - dy * radius);
    context.lineTo(centre.x + dx * radius, centre.y + dy * radius);
    context.stroke();
  }
  context.fillStyle = colour("--weak");
  context.font = "10px " + colour("--font");
  context.fillText("ahead", centre.x - 14, 12);

  // The listener, with a nose so the view has a direction.
  context.strokeStyle = colour("--weak");
  context.lineWidth = 1.5;
  context.beginPath();
  context.arc(centre.x, centre.y, 9, 0, Math.PI * 2);
  context.moveTo(centre.x, centre.y - 9);
  context.lineTo(centre.x, centre.y - 15);
  context.stroke();

  if (!telemetry || !description) return;
  const lanes = telemetry.lanes.slice(0, telemetry.lane_count);
  const fade = (base, alpha) =>
    colour(base) + Math.round(Math.min(alpha, 1) * 255).toString(16).padStart(2, "0");

  // Lanes in the centre of your head — kick and bass, which is where low
  // frequencies belong — get a list rather than a dot, since they would all be
  // the same dot.
  context.font = `10px ${colour("--font")}`;
  let centred = 0;
  lanes.forEach((lane, index) => {
    if (lane.placed) return;
    const name = description.lanes[index]?.name ?? "?";
    const level = Math.sqrt(Math.min(Math.max(lane.level, 0), 1));
    const y = height - 10 - 13 * centred++;
    context.fillStyle = fade("--weak", lane.muted ? 0.25 : 0.4 + 0.6 * level);
    context.beginPath();
    context.arc(10, y - 3, 2 + 4 * level, 0, Math.PI * 2);
    context.fill();
    context.fillText(`${name} · centre`, 20, y);
  });

  lanes.forEach((lane, index) => {
    if (!lane.placed) return;
    const name = description.lanes[index]?.name ?? "?";
    const [x, , z] = lane.position;
    const distance = Math.hypot(x, z);
    if (distance < 0.001) return;
    const at = {
      x: centre.x + (x / distance) * scale(distance),
      y: centre.y + (z / distance) * scale(distance),
    };
    const level = Math.sqrt(Math.min(Math.max(lane.level, 0), 1));
    const size = 3 + 9 * level;
    context.fillStyle = fade("--accent", lane.muted ? 0.2 : 0.3 + 0.7 * level);
    context.beginPath();
    context.arc(at.x, at.y, size, 0, Math.PI * 2);
    context.fill();
    // The label sits further out along the same radius, so labels spread apart
    // exactly where the dots crowd together.
    // Outlined, because sources bunch together and two labels on top of each
    // other are worse than none.
    const label = {
      x: at.x + (x / distance) * (size + 5) + (x < 0 ? -2 : 2),
      y: at.y + (z / distance) * (size + 5) + 3,
    };
    context.textAlign = x < 0 ? "right" : "left";
    context.lineWidth = 3;
    context.strokeStyle = colour("--panel");
    context.strokeText(name, label.x, label.y);
    context.fillStyle = fade("--text", lane.muted ? 0.3 : 0.55 + 0.45 * level);
    context.fillText(name, label.x, label.y);
    context.textAlign = "left";
  });
}

// ── The frame ───────────────────────────────────────────────────────────────

function frame() {
  if (telemetry) {
    const t = telemetry;
    $("play").textContent = t.playing ? "stop" : "play";
    if (!held.has("scrub")) scrub.value = t.bar;
    const seconds = (t.bar * 4 * 60) / Math.max(t.bpm, 1);
    const minutes = String(Math.floor(seconds / 60)).padStart(2, "0");
    $("position").textContent =
      `bar ${t.bar.toFixed(2).padStart(7)} / ${String(stageBars).padEnd(3)}` +
      ` ${minutes}:${(seconds % 60).toFixed(1).padStart(4, "0")}`;

    const load = Math.min(t.load, 1);
    $("load").style.width = `${load * 100}%`;
    $("load").style.background = load > 0.7 ? colour("--bad") : load > 0.4 ? colour("--warn") : colour("--good");
    $("loadText").textContent = `${String(Math.round(t.load * 100)).padStart(3)}%`;
    $("voices").textContent = `${String(t.voices).padStart(2)} voices`;
    const peak = 20 * Math.log10(Math.max(t.peak, 1e-6));
    $("peak").textContent = `peak ${peak.toFixed(1).padStart(6)} dBFS`;
    $("peak").style.color = peak > -0.5 ? colour("--bad") : colour("--text");
    $("dropouts").textContent =
      `${String(t.xruns).padStart(2)} dropouts · ${String(t.dropped).padStart(2)} voices dropped`;
    $("dropouts").style.color = t.xruns > 0 ? colour("--bad") : t.dropped > 0 ? colour("--warn") : colour("--weak");
    if (!held.has("bpm")) $("bpmText").textContent = t.bpm.toFixed(1);

    for (const meter of document.querySelectorAll(".lanes .meter")) {
      const lane = t.lanes[Number(meter.dataset.lane)];
      const fill = meter.firstElementChild;
      fill.style.width = `${Math.sqrt(Math.min(Math.max(lane.level, 0), 1)) * 100}%`;
      fill.style.background = lane.sounding ? colour("--good") : colour("--weak");
    }
    for (const box of document.querySelectorAll(".macro")) {
      const index = Number(box.dataset.macro);
      const value = macroHeld.has(index) ? macroHeld.get(index) : t.macros[index];
      if (!macroHeld.has(index)) box.querySelector("input").value = value;
      box.querySelector(".value").textContent = value.toFixed(2);
    }
  }
  if (tab === "conductor") {
    drawTimeline();
    drawPlan();
  } else if (telemetry) {
    // Which step is sounding, so the grid reads as a machine running rather
    // than a spreadsheet.
    const sixteenth = Math.floor(telemetry.bar * 16);
    for (const row of document.querySelectorAll(".lane")) {
      const lane = description?.lanes[Number(row.dataset.lane)];
      if (!lane) continue;
      const here = ((sixteenth % lane.steps.length) + lane.steps.length) % lane.steps.length;
      for (const cell of row.querySelectorAll(".step")) {
        cell.classList.toggle("playing", Number(cell.dataset.step) === here);
      }
    }
  }
  requestAnimationFrame(frame);
}

connect();
requestAnimationFrame(frame);
