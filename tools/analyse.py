"""Read a WAV and print what it is, since neither of us can hear it.

Level, peak, the largest sample-to-sample jump (which is how a click shows up),
where the energy sits in the spectrum, and on request K-weighted loudness,
reverberation time per octave band, note onsets, and how much of a spun tone is
not the tone.

    python3 tools/analyse.py <file.wav> [--lufs] [--rt60] [--hits] [--tone=1000]
"""

import sys
from pathlib import Path

import numpy as np


def db(x: float) -> float:
    return 20.0 * np.log10(max(x, 1e-12))


# ITU-R BS.1770 K-weighting: a high shelf that lifts everything above about
# 1.5 kHz by 4 dB, and a highpass near 38 Hz. Between them they approximate how
# loud the ear finds a sound, which raw RMS does not: a 49 Hz kick at full scale
# carries most of the energy in a techno mix and nothing like most of the
# loudness. These coefficients are specific to 48 kHz.
K_SHELF = (
    [1.53512485958697, -2.69169618940638, 1.19839281085285],
    [1.0, -1.69065929318241, 0.73248077421585],
)
K_HIGHPASS = ([1.0, -2.0, 1.0], [1.0, -1.99004745483398, 0.99007225036621])


def lufs(x: np.ndarray, sr: int) -> float:
    """Integrated loudness of (frames, channels) audio, in LUFS."""
    if sr != 48000:
        raise ValueError(f"the K-weighting coefficients here are for 48 kHz, not {sr}")
    n = len(x)
    f = np.fft.rfftfreq(n, 1 / sr)
    z = np.exp(-2j * np.pi * f / sr)
    # Applied in the frequency domain: we only want one number out, and the
    # wrap-around at the edges is irrelevant over seconds of audio.
    response = np.ones_like(z)
    for b, a in (K_SHELF, K_HIGHPASS):
        response *= (b[0] + b[1] * z + b[2] * z**2) / (a[0] + a[1] * z + a[2] * z**2)
    power = 0.0
    for channel in range(x.shape[1]):
        weighted = np.fft.irfft(np.fft.rfft(x[:, channel]) * response, n)
        power += (weighted**2).mean()
    return -0.691 + 10 * np.log10(max(power, 1e-12))


def read_wav(path: Path) -> tuple[int, np.ndarray]:
    """Samples as float in [-1, 1], shaped (frames, channels).

    Written out rather than taken from scipy, which isn't installed here (the
    numpy studio runs on the server). A WAV is a handful of chunks.
    """
    raw = path.read_bytes()
    if raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise ValueError(f"{path} is not a RIFF/WAVE file")
    fmt: tuple[int, int, int] | None = None
    data = b""
    pos = 12
    while pos + 8 <= len(raw):
        name = raw[pos : pos + 4]
        size = int.from_bytes(raw[pos + 4 : pos + 8], "little")
        body = raw[pos + 8 : pos + 8 + size]
        if name == b"fmt ":
            code = int.from_bytes(body[0:2], "little")
            channels = int.from_bytes(body[2:4], "little")
            rate = int.from_bytes(body[4:8], "little")
            bits = int.from_bytes(body[14:16], "little")
            if code == 0xFFFE:  # WAVE_FORMAT_EXTENSIBLE, which hound writes:
                # the real format is the first field of the SubFormat GUID.
                code = int.from_bytes(body[24:26], "little")
            fmt = (code, channels, rate, bits)
        elif name == b"data":
            data = body
        pos += 8 + size + (size & 1)  # chunks are word-aligned
    if fmt is None or not data:
        raise ValueError(f"{path}: no fmt/data chunk")
    code, channels, rate, bits = fmt
    if code == 3 and bits == 32:
        x = np.frombuffer(data, dtype="<f4").astype(np.float64)
    elif code == 1 and bits == 16:
        x = np.frombuffer(data, dtype="<i2").astype(np.float64) / 32768.0
    elif code == 1 and bits == 32:
        x = np.frombuffer(data, dtype="<i4").astype(np.float64) / 2147483648.0
    else:
        raise ValueError(f"{path}: unsupported format code {code}, {bits} bits")
    return rate, x.reshape(-1, channels)


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    path = Path(args[0])
    sr, x = read_wav(path)
    mono = x.mean(axis=1)

    print(f"{path.name}: {len(mono) / sr:.3f} s at {sr} Hz, {x.shape[1]} channel(s)")
    print(f"  peak      {db(np.abs(mono).max()):7.2f} dBFS")
    print(f"  rms       {db(np.sqrt((mono**2).mean())):7.2f} dBFS")
    print(f"  max jump  {np.abs(np.diff(mono)).max():7.4f}  (a click is a jump nothing explains)")
    if x.shape[1] > 1:
        left, right = x[:, 0], x[:, 1]
        corr = np.corrcoef(left, right)[0, 1] if left.std() and right.std() else 1.0
        print(f"  L/R corr  {corr:7.3f}  (1.0 = dead centre)")

    # Where the energy is. A Hann window so the peaks are not smeared by the
    # rectangular window's side lobes.
    #
    # Measured per channel and averaged, not on the mono sum: a placed source is
    # decorrelated between the ears, so summing them cancels it. Measured the
    # wrong way, orbiting hats read as a tenth of their real level.
    n = min(len(mono), 1 << 16)
    window = np.hanning(n)
    spectrum = np.mean([np.abs(np.fft.rfft(x[:n, c] * window)) for c in range(x.shape[1])], axis=0)
    freqs = np.fft.rfftfreq(n, 1 / sr)
    top = np.argsort(spectrum)[-6:][::-1]
    peaks = ", ".join(f"{freqs[i]:.0f} Hz" for i in sorted(top, key=lambda i: -spectrum[i]))
    print(f"  spectral peaks: {peaks}")
    for lo, hi in [(20, 60), (60, 120), (120, 300), (300, 1000), (1000, 4000), (4000, 20000)]:
        band = (freqs >= lo) & (freqs < hi)
        share = (spectrum[band] ** 2).sum() / (spectrum**2).sum()
        print(f"    {lo:>5}-{hi:<5} Hz  {100 * share:5.1f}%")

    if "--lufs" in flags:
        print(f"  loudness  {lufs(x, sr):7.2f} LUFS  (K-weighted, ITU BS.1770)")

    if "--rt60" in flags:
        # Reverberation time per octave band, by Schroeder's method: integrate
        # the squared response backwards from the end for a smooth decay curve,
        # fit a line to the part between -5 and -35 dB, and extrapolate to -60.
        # Fitting that stretch rather than measuring to -60 directly is the
        # standard trick — the last 25 dB is where the noise floor lives.
        #
        # The band filter is a Hann-windowed sinc, not a brick wall on the
        # spectrum. A brick wall rings in the time domain, which spreads the
        # loud midrange across the whole of a quiet low band and flattens its
        # decay curve: measured that way, a 7 s tail read 17 s at 125 Hz.
        print("  reverberation time per octave band:")
        taps = 4097
        k = np.arange(taps) - taps // 2
        window = np.hanning(taps)
        total = (mono**2).sum()
        for centre in (125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0):
            lo, hi = centre / np.sqrt(2), min(centre * np.sqrt(2), sr / 2 * 0.98)
            # A bandpass is the difference of two lowpass sincs.
            def sinc_lp(cut: float) -> np.ndarray:
                return 2 * cut / sr * np.sinc(2 * cut / sr * k)

            impulse = (sinc_lp(hi) - sinc_lp(lo)) * window
            n = len(mono) + taps
            band = np.fft.irfft(np.fft.rfft(mono, n) * np.fft.rfft(impulse, n), n)[taps // 2 :]
            share = (band**2).sum() / total
            if share < 1e-6:
                print(f"    {centre:6.0f} Hz  too little energy to measure ({share:.1e})")
                continue
            decay = np.cumsum((band**2)[::-1])[::-1]
            curve = 10 * np.log10(np.maximum(decay / decay[0], 1e-30))
            crossings = [np.flatnonzero(curve <= level) for level in (-5.0, -35.0)]
            if any(len(c) == 0 for c in crossings):
                print(f"    {centre:6.0f} Hz  decay never reached -35 dB")
                continue
            a, b = int(crossings[0][0]), int(crossings[1][0])
            slope = (curve[b] - curve[a]) / ((b - a) / sr)  # dB per second
            print(f"    {centre:6.0f} Hz  RT60 {-60.0 / slope:5.2f} s   ({100 * share:4.1f}% of energy)")

    tone = next((a for a in sys.argv[1:] if a.startswith("--tone=")), None)
    if tone:
        # The placement quality metric: a pure tone on a moving source should
        # stay a pure tone. Everything outside a narrow window around it is an
        # artefact — a stepping delay, a filter that jumps between blocks, an
        # interpolator whose dullness varies with the fraction. The window is
        # +/-30 Hz because a real orbit Doppler-shifts by a few Hz, which is
        # not an artefact but the point.
        centre = float(tone.split("=")[1])
        for name, channel in (("left", x[:, 0]), ("right", x[:, -1])):
            n = len(channel)
            spectrum = np.abs(np.fft.rfft(channel * np.hanning(n))) ** 2
            f = np.fft.rfftfreq(n, 1 / sr)
            near = np.abs(f - centre) <= 30.0
            ratio = spectrum[~near].sum() / spectrum[near].sum()
            print(f"  {name:>5}: everything but the tone is {10 * np.log10(ratio):7.1f} dB down")

    if "--hits" in flags:
        # Onsets: where the envelope jumps. The smoothing has to be longer than
        # one cycle of the lowest note or every kick retriggers on its own
        # waveform — 30 ms covers 33 Hz and up.
        env = np.abs(mono)
        window = int(0.030 * sr)
        smooth = np.convolve(env, np.ones(window) / window, mode="same")
        loud = smooth > 0.25 * smooth.max()
        edges = list(np.flatnonzero(np.diff(loud.astype(int)) == 1) + 1)
        if loud[0]:  # a hit on the very first sample has no rising edge
            edges.insert(0, 0)
        gap = int(0.050 * sr)  # two onsets closer than this are one hit
        starts = [s for i, s in enumerate(edges) if i == 0 or s - edges[i - 1] > gap]
        times = np.array(starts) / sr
        print(f"  {len(times)} hits at: {', '.join(f'{t:.3f}' for t in times[:16])}")
        if len(times) > 1:
            gaps = np.diff(times)
            print(
                f"  gaps: mean {gaps.mean():.4f} s"
                f" (a beat at {60 / gaps.mean():.2f} BPM),"
                f" spread {gaps.std() * 1000:.2f} ms"
            )


if __name__ == "__main__":
    main()
