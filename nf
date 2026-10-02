#!/usr/bin/env python3
"""nf — the working surface for Night Flight.

Two halves, and the line between them is not negotiable. **Rust owns the
format and the sound**: the tab parser lives in `crates/engine/src/tab.rs`, is
round-trip tested, and a second parser here would drift from it. So this never
reads a tab file — it asks `nf-engine`, which is the same code the audio
thread uses.

**Python owns everything else**: the command surface, the analysis, the
scaffolding, and printing things in a shape a person can read. That is the
half that changes every session, and it should be the half that is ten lines
to extend.

    nf ls                        what the library holds
    nf show   <piece>            the piece as tab
    nf check  <file>             read it and say what is wrong
    nf render <piece> [-o f.wav] render it
    nf measure <piece|wav>       render if needed, then the numbers
    nf new    piece|clip <name>  a tab file to start from
    nf convert <in> <out>        tab <-> json

A <piece> is a name in pieces/, a path to a .json, or a path to a .tab.
All three work anywhere one is asked for.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "tools"))

import numpy as np  # noqa: E402
from analyse import db, lufs, read_wav  # noqa: E402

BANDS = [(20, 120, "low"), (120, 1200, "mid"), (1200, 16000, "high")]


# ── talking to the engine ───────────────────────────────────────────────────

def engine(*args: str, capture: bool = True) -> str:
    """Run nf-engine. Built if it is not there, because being told to run a
    build command is a worse experience than waiting once."""
    binary = HERE / "target" / "release" / "nf-engine"
    if not binary.exists():
        print("building nf-engine…", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release", "-q", "-p", "nf-engine"],
            cwd=HERE, check=True,
        )
    r = subprocess.run(
        [str(binary), *args], cwd=HERE, text=True,
        capture_output=capture,
    )
    if capture and r.stderr:
        print(r.stderr.rstrip(), file=sys.stderr)
    if r.returncode != 0:
        raise SystemExit(r.returncode)
    return r.stdout if capture else ""


def index() -> dict:
    return json.loads(engine("index"))


def piece_facts(what: str) -> dict:
    """bpm and bars for a piece, however it was named."""
    stem = Path(what).stem if "/" in what or what.endswith((".tab", ".json")) else what
    for p in index()["pieces"]:
        if p["name"] == stem:
            return p
    # A tab file outside pieces/ is not in the index; read its header.
    head = engine("describe", what).splitlines()
    facts = {"name": stem, "bpm": 126.0, "bars": 0.0}
    for line in head[:6]:
        parts = line.split()
        if len(parts) == 2 and parts[0] in ("bpm", "bars"):
            facts[parts[0]] = float(parts[1])
    return facts


# ── the commands ────────────────────────────────────────────────────────────

def cmd_ls(args: list[str]) -> None:
    """What there is to work with."""
    data = index()
    if "--json" in args:
        print(json.dumps(data, indent=2))
        return
    want = next((a for a in args if not a.startswith("-")), None)

    if want in (None, "pieces"):
        print("pieces")
        for p in data["pieces"]:
            clips = " ".join(p["clips"])
            print(f"  {p['name']:<14}{p['bars']:>6.0f} bars  {p['bpm']:>7.1f} BPM  "
                  f"{p['lanes']:>2} lanes   {clips}")
    if want in (None, "clips"):
        print("\nclips")
        for c in data["clips"]:
            print(f"  {c['name']:<14}{c['lanes']:>6} lanes {c['steps']:>5} steps")
    if want in (None, "patches"):
        print("\npatches")
        by_instrument: dict[str, list[str]] = {}
        for p in data["patches"]:
            by_instrument.setdefault(p["instrument"], []).append(p["name"])
        for instrument, names in sorted(by_instrument.items()):
            print(f"  {instrument:<10}{' '.join(sorted(names))}")


def cmd_show(args: list[str]) -> None:
    print(engine("describe", *args), end="")


def cmd_check(args: list[str]) -> None:
    print(engine("check", *args), end="")


def cmd_convert(args: list[str]) -> None:
    print(engine("convert", *args), end="")


def cmd_render(args: list[str]) -> None:
    what, rest = args[0], args[1:]
    out = flag(rest, "-o") or flag(rest, "--out") or f"renders/{Path(what).stem}.wav"
    print(engine("render", what, out, *without(rest, "-o", "--out")), end="")


def cmd_measure(args: list[str]) -> None:
    """Render if given a piece, then say what came out.

    The thing I reach for most and used to hand-roll every time, which is the
    whole argument for this file existing.
    """
    what = args[0]
    rest = args[1:]
    if what.endswith(".wav"):
        path, facts = Path(what), {"name": Path(what).stem, "bpm": None, "bars": None}
    else:
        facts = piece_facts(what)
        tmp = Path(tempfile.gettempdir()) / f"nf-{Path(what).stem}.wav"
        engine("render", what, str(tmp), *[a for a in rest if a.startswith("--")])
        path = tmp

    sr, x = read_wav(path)
    mono = x.mean(axis=1)
    frames = len(mono)
    seconds = frames / sr
    peak = float(np.abs(x).max())
    loudest = int(np.argmax(np.abs(mono))) / sr

    head = f"{facts['name']}  {seconds:.3f} s"
    if facts.get("bpm"):
        head += f"  ·  {facts['bars']:g} bars at {facts['bpm']:g} BPM"
    print(head)
    print(f"  peak      {db(peak):+7.2f} dBFS   at {loudest:.3f} s")
    print(f"  loudness  {lufs(x, sr):+7.2f} LUFS")
    # Mono is where a phone plays it, and a wide mix loses level when summed.
    wide = float(np.sqrt((x ** 2).mean()))
    print(f"  mono sum  {db(float(np.sqrt((mono ** 2).mean()))) - db(wide):+7.2f} dB "
          f"against stereo")

    if facts.get("bpm"):
        bar = 240.0 / facts["bpm"]
        print(f"\n  bar     window        peak      rms" +
              "".join(f"{n:>8}" for _, _, n in BANDS))
        for b in range(int(np.ceil(seconds / bar))):
            lo, hi = int(b * bar * sr), min(int((b + 1) * bar * sr), frames)
            if hi - lo < sr // 100:
                break
            seg = mono[lo:hi]
            cells = "".join(f"{band(seg, sr, a, z):>8.1f}" for a, z, _ in BANDS)
            print(f"  {b + 1:>3}  {b * bar:6.2f}-{(b + 1) * bar:5.2f}s "
                  f"{db(float(np.abs(seg).max())):>8.1f} "
                  f"{db(float(np.sqrt((seg ** 2).mean()))):>8.1f}{cells}")

    if "--lanes" in rest and not what.endswith(".wav"):
        per_lane(what)

    hits = flag(rest, "--hits")
    if hits:
        print("\n  cut        music within 50 ms")
        env = smooth(np.abs(mono), int(0.002 * sr))
        for spec in hits.split(","):
            t = float(spec)
            lo, hi = max(0, int((t - 0.05) * sr)), int((t + 0.05) * sr)
            window = env[lo:hi]
            if len(window) == 0:
                continue
            at = (lo + int(np.argmax(window))) / sr
            print(f"  {t:7.3f}s  {db(float(window.max())):+7.1f} dBFS  "
                  f"{(at - t) * 30:+5.2f} frames")


def per_lane(what: str) -> None:
    """Every lane on its own, loudest first.

    The thing I most wanted and did not have. Instruments here are nothing
    like each other at the same gain — a kick at 0.9 peaks around -6 dBFS and
    a saw ensemble at 0.5 comes to -33 dB RMS — and the only way to find that
    out was to render each one by hand, four times, while wondering why a
    breakdown was silent.
    """
    lanes = json.loads(engine("lanes", what))
    bpm = piece_facts(what).get("bpm") or 120
    bar = 240.0 / bpm
    print(f"\n  {'lane':<14}{'plays':<9}{'gain':>6}{'peak':>9}{'loudest bar':>12}")
    rows = []
    for lane in lanes:
        tmp = Path(tempfile.gettempdir()) / f"nf-solo-{lane['name'].replace(' ', '_')}.wav"
        engine("render", what, str(tmp), "--solo", lane["name"])
        sr, x = read_wav(tmp)
        mono = x.mean(axis=1)
        # The loudest bar, not the whole render. A lane that only plays for
        # four bars of sixteen reads six decibels low against one that plays
        # throughout, and then the table says the hook is quieter than the
        # drums when it is not — which is exactly the wrong answer from a tool
        # whose only job is to compare lanes.
        loudest = max(
            float(np.sqrt((mono[int(b * bar * sr):int((b + 1) * bar * sr)] ** 2).mean()))
            for b in range(max(1, int(len(mono) / sr / bar)))
        )
        rows.append((lane, db(float(np.abs(x).max())), db(loudest)))
    for lane, peak, rms in sorted(rows, key=lambda r: -r[2]):
        mark = " gated" if lane["gated"] else ""
        print(f"  {lane['name']:<14}{lane['instrument']:<9}{lane['gain']:>6.2f}"
              f"{peak:>9.1f}{rms:>12.1f}{mark}")
    spread = rows and max(r[2] for r in rows) - min(r[2] for r in rows)
    if spread:
        print(f"\n  {spread:.0f} dB between the loudest lane and the quietest, "
              f"each measured in the bar where it works hardest.")


def cmd_new(args: list[str]) -> None:
    """A tab file to start from, because a blank page is not a format."""
    kind = args[0] if args else "piece"
    name = args[1] if len(args) > 1 else "untitled"
    bars = int(flag(args, "--bars") or 2)
    bpm = flag(args, "--bpm") or "126"
    out = Path(flag(args, "-o") or f"{name}.tab")

    grid = " ".join(". " * 16 for _ in range(bars)).strip()
    lines = [f"piece {name}", f"bpm {bpm}", f"bars {bars}", ""]
    if kind == "piece":
        lines += [
            "lane kick  clip=beat inst=kick root=G1 gain=0.9 len=0.4s",
            "lane hat   clip=beat inst=hat  root=C-1 gain=0.5 len=0.05s send=0.1",
            "lane bass  clip=beat inst=bass root=G1 gain=0.8 len=0.8 duck vel=8",
            "",
            f"grid beat bars 1-{bars}",
            f"  kick  {' | '.join(['X . . . X . . . X . . . X . . .'] * bars)}",
            f"  hat   {' | '.join(['. . 5 . . . 5 . . . 5 . . . 5 .'] * bars)}",
            f"  bass  {' | '.join(['G1 .  .  .  .  .  .  .  G1 .  .  .  .  .  .  . '] * bars)}",
        ]
    else:
        lines += [
            "lane lead  clip=%s inst=glass root=G3 gain=0.6 len=4 send=0.5 vel=8" % name,
            "",
            f"grid {name} bars 1-{bars}",
            f"  lead  {grid}",
        ]
    out.write_text("\n".join(lines) + "\n")
    print(out)
    print(engine("check", str(out)), end="")


# ── odds and ends ───────────────────────────────────────────────────────────

def band(seg: np.ndarray, sr: int, lo: float, hi: float) -> float:
    f = np.abs(np.fft.rfft(seg * np.hanning(len(seg))))
    hz = np.fft.rfftfreq(len(seg), 1 / sr)
    sel = (hz >= lo) & (hz < hi)
    return db(float(np.sqrt((f[sel] ** 2).sum()) / len(seg)))


def smooth(x: np.ndarray, width: int) -> np.ndarray:
    return np.convolve(x, np.ones(max(width, 1)) / max(width, 1), mode="same")


def flag(args: list[str], name: str) -> str | None:
    if name in args:
        i = args.index(name)
        if i + 1 < len(args):
            return args[i + 1]
    for a in args:
        if a.startswith(name + "="):
            return a.split("=", 1)[1]
    return None


def without(args: list[str], *names: str) -> list[str]:
    out, skip = [], False
    for a in args:
        if skip:
            skip = False
        elif a in names:
            skip = True
        elif not any(a.startswith(n + "=") for n in names):
            out.append(a)
    return out


COMMANDS = {
    "ls": cmd_ls, "show": cmd_show, "check": cmd_check, "render": cmd_render,
    "measure": cmd_measure, "new": cmd_new, "convert": cmd_convert,
}


def main() -> int:
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help", "help"):
        print(__doc__.strip())
        return 0 if args else 1
    verb, rest = args[0], args[1:]
    if verb not in COMMANDS:
        print(f"{verb!r} is not a command. Try: {' '.join(COMMANDS)}", file=sys.stderr)
        return 1
    if verb in ("show", "check", "render", "measure", "convert") and not rest:
        print(f"{verb} needs something to work on", file=sys.stderr)
        return 1
    if not shutil.which("cargo") and not (HERE / "target/release/nf-engine").exists():
        print("needs either cargo or a built nf-engine", file=sys.stderr)
        return 1
    COMMANDS[verb](rest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
