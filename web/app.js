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

/** The last frame that arrived, and the set description from the hello. */
let telemetry = null;
let description = null;
/** Controls a hand is currently on, by name — these stop following telemetry. */
const held = new Set();
/** Macro values this page is holding, or undefined where the curve is driving. */
const macroHeld = new Map();

let socket = null;

function connect() {
  socket = new WebSocket(`ws://${location.host}/`);
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.t === "hello") {
      description = message;
      $("set").textContent = message.set;
      $("device").textContent = `${message.device} · ${message.buffer_frames} frames`;
      $("scrub").max = message.length_bars;
      buildParts(message.parts);
      buildMacros(message.macros);
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

function buildParts(parts) {
  const host = $("parts");
  host.replaceChildren();
  parts.forEach((part, index) => {
    const name = document.createElement("button");
    name.className = "name" + (part.muted ? " muted" : "");
    name.textContent = part.name;
    name.title = `${part.instrument}${part.placed ? " · placed" : " · centre"}${
      part.ducked ? " · ducked by the kick" : ""
    }`;
    name.onclick = () => {
      const muted = !name.classList.contains("muted");
      name.classList.toggle("muted", muted);
      send({ t: "mute", index, muted });
    };

    const fader = document.createElement("input");
    Object.assign(fader, { type: "range", min: 0, max: 2, step: 0.01, value: part.gain });
    const readout = document.createElement("span");
    readout.className = "gain";
    readout.textContent = part.gain.toFixed(2);
    fader.oninput = () => {
      readout.textContent = Number(fader.value).toFixed(2);
      send({ t: "level", index, gain: Number(fader.value) });
    };

    const meter = document.createElement("span");
    meter.className = "meter";
    const fill = document.createElement("i");
    meter.append(fill);
    meter.dataset.part = index;

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

/** The arrangement: one row per part, its spans drawn, with the playhead. */
function drawTimeline() {
  const { context, width, height } = fit($("timeline"));
  context.clearRect(0, 0, width, height);
  if (!description) return;

  // Names live in a gutter rather than on top of the bars: a label over a bar
  // is unreadable against either colour.
  const gutter = 74;
  const top = 4;
  const bars = description.length_bars;
  const rows = description.parts.length;
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

  description.parts.forEach((part, index) => {
    const y = top + index * rowHeight;
    const middle = y + rowHeight / 2 - 1;
    const muted = telemetry?.parts?.[index]?.muted;

    // A part with no spans plays for ever, which a set for jamming wants.
    const spans = part.spans.length
      ? part.spans
      : [{ start: 0, end: bars, first_bar: 0 }];
    // Whether a block is lit follows the playhead being inside it, not whether
    // a voice happens to be ringing this instant: a closed hat sounds for a
    // third of the time it is playing, so voice activity makes a block flicker
    // and says nothing about the arrangement.
    const live = spans.some((span) => inside(span.first_bar, span.end));

    context.fillStyle = muted ? colour("--line") : live ? colour("--text") : colour("--weak");
    context.textAlign = "right";
    context.fillText(part.name, gutter - 6, middle);
    context.textAlign = "left";

    for (const span of spans) {
      // A flight's approach: the part is sounding, but from somewhere else. It
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
  const parts = telemetry.parts.slice(0, telemetry.part_count);
  const fade = (base, alpha) =>
    colour(base) + Math.round(Math.min(alpha, 1) * 255).toString(16).padStart(2, "0");

  // Parts in the centre of your head — kick and bass, which is where low
  // frequencies belong — get a list rather than a dot, since they would all be
  // the same dot.
  context.font = `10px ${colour("--font")}`;
  let centred = 0;
  parts.forEach((part, index) => {
    if (part.placed) return;
    const name = description.parts[index]?.name ?? "?";
    const level = Math.sqrt(Math.min(Math.max(part.level, 0), 1));
    const y = height - 10 - 13 * centred++;
    context.fillStyle = fade("--weak", part.muted ? 0.25 : 0.4 + 0.6 * level);
    context.beginPath();
    context.arc(10, y - 3, 2 + 4 * level, 0, Math.PI * 2);
    context.fill();
    context.fillText(`${name} · centre`, 20, y);
  });

  parts.forEach((part, index) => {
    if (!part.placed) return;
    const name = description.parts[index]?.name ?? "?";
    const [x, , z] = part.position;
    const distance = Math.hypot(x, z);
    if (distance < 0.001) return;
    const at = {
      x: centre.x + (x / distance) * scale(distance),
      y: centre.y + (z / distance) * scale(distance),
    };
    const level = Math.sqrt(Math.min(Math.max(part.level, 0), 1));
    const size = 3 + 9 * level;
    context.fillStyle = fade("--accent", part.muted ? 0.2 : 0.3 + 0.7 * level);
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
    context.fillStyle = fade("--text", part.muted ? 0.3 : 0.55 + 0.45 * level);
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
      `bar ${t.bar.toFixed(2).padStart(7)} / ${String(description?.length_bars ?? 0).padEnd(3)}` +
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

    for (const meter of document.querySelectorAll(".parts .meter")) {
      const part = t.parts[Number(meter.dataset.part)];
      const fill = meter.firstElementChild;
      fill.style.width = `${Math.sqrt(Math.min(Math.max(part.level, 0), 1)) * 100}%`;
      fill.style.background = part.sounding ? colour("--good") : colour("--weak");
    }
    for (const box of document.querySelectorAll(".macro")) {
      const index = Number(box.dataset.macro);
      const value = macroHeld.has(index) ? macroHeld.get(index) : t.macros[index];
      if (!macroHeld.has(index)) box.querySelector("input").value = value;
      box.querySelector(".value").textContent = value.toFixed(2);
    }
  }
  drawTimeline();
  drawPlan();
  requestAnimationFrame(frame);
}

connect();
requestAnimationFrame(frame);
