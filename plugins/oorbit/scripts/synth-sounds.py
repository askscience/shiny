"""Synthesize Oorbit's sounds: ambient loops and short effects, as mono WAV.

    python3 -I plugins/oorbit/scripts/synth-sounds.py [out_dir]

Writes into plugins/oorbit/game/sounds/ by default. Only the standard
library: every sound is made from sines, noise, filters and envelopes, from a
fixed seed, so running it again gives the same files. Ambient loops are
seamless (their end fades into their start).
"""
import math
import os
import random
import struct
import sys
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "game", "sounds")
AMBIENT_RATE = 11025
EFFECT_RATE = 22050
TAU = 2.0 * math.pi


def write(name, samples, rate):
    os.makedirs(OUT, exist_ok=True)
    peak = max(1e-9, max(abs(s) for s in samples))
    gain = 0.89 / peak
    with wave.open(os.path.join(OUT, name + ".wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s * gain)) * 32767)) for s in samples))


def loop(samples, rate, fade=0.5):
    """Make a loop seamless: the tail fades into the head."""
    n = int(fade * rate)
    body = samples[:-n]
    tail = samples[-n:]
    for i in range(n):
        t = i / n
        body[i] = body[i] * t + tail[i] * (1.0 - t)
    return body


def noise(rng, n):
    return [rng.uniform(-1.0, 1.0) for _ in range(n)]


def lowpass(x, cutoff, rate):
    a = 1.0 - math.exp(-TAU * cutoff / rate)
    y, out = 0.0, []
    for s in x:
        y += a * (s - y)
        out.append(y)
    return out


def highpass(x, cutoff, rate):
    low = lowpass(x, cutoff, rate)
    return [a - b for a, b in zip(x, low)]


def bandpass(x, low, high, rate):
    return highpass(lowpass(x, high, rate), low, rate)


def env(n, attack, release, rate):
    a, r = max(1, int(attack * rate)), max(1, int(release * rate))
    return [min(1.0, i / a) * min(1.0, (n - i) / r) for i in range(n)]


def bell(freq, seconds, rate, decay=3.0, partials=((1, 1.0), (2.01, 0.5), (3.02, 0.25), (4.2, 0.12))):
    n = int(seconds * rate)
    return [
        sum(a * math.sin(TAU * freq * k * i / rate) for k, a in partials) * math.exp(-decay * i / rate)
        for i in range(n)
    ]


def mix(*tracks):
    n = max(len(t) for t in tracks)
    return [sum(t[i] for t in tracks if i < len(t)) for i in range(n)]


def place(track, sound, at, rate, gain=1.0):
    start = int(at * rate)
    for i, s in enumerate(sound):
        if start + i < len(track):
            track[start + i] += s * gain


# ---------------------------------------------------------------- ambients

def space(rng):
    r, n = AMBIENT_RATE, int(8.5 * AMBIENT_RATE)
    hum = [
        0.5 * math.sin(TAU * 55 * i / r) * (0.7 + 0.3 * math.sin(TAU * 0.125 * i / r))
        + 0.3 * math.sin(TAU * 82.5 * i / r + 1.0) * (0.6 + 0.4 * math.sin(TAU * 0.25 * i / r + 2.0))
        + 0.15 * math.sin(TAU * 110.3 * i / r)
        for i in range(n)
    ]
    air = lowpass(noise(rng, n), 300, r)
    return loop(mix(hum, [a * 0.6 for a in air]), r)


def wind(rng):
    r, n = AMBIENT_RATE, int(8.5 * AMBIENT_RATE)
    x = lowpass(lowpass(noise(rng, n), 500, r), 700, r)
    gust = [0.5 + 0.5 * math.sin(TAU * i / n * 2 + 0.7) * math.sin(TAU * i / n * 3) for i in range(n)]
    return loop([a * (0.3 + 0.7 * g) for a, g in zip(x, gust)], r)


def waves(rng):
    r, n = AMBIENT_RATE, int(8.5 * AMBIENT_RATE)
    x = bandpass(noise(rng, n), 120, 1400, r)
    swell = []
    for i in range(n):
        t = (i / r) % 4.0
        swell.append((t / 1.2) ** 2 if t < 1.2 else math.exp(-(t - 1.2) * 1.3))
    return loop([a * (0.15 + s) for a, s in zip(x, swell)], r)


def birds(rng):
    r, n = AMBIENT_RATE, int(8.5 * AMBIENT_RATE)
    track = [0.0] * n
    for _ in range(9):
        at = rng.uniform(0.2, 7.6)
        base = rng.uniform(2000, 3600)
        for k in range(rng.randint(2, 4)):
            m = int(0.09 * r)
            chirp = [
                math.sin(TAU * (base + 900 * math.sin(math.pi * i / m)) * i / r) * math.sin(math.pi * i / m)
                for i in range(m)
            ]
            place(track, chirp, at + k * 0.13, r, rng.uniform(0.3, 0.7))
    leaves = lowpass(noise(rng, n), 900, r)
    return loop(mix(track, [a * 0.25 for a in leaves]), r)


def village(rng):
    r, n = AMBIENT_RATE, int(8.5 * AMBIENT_RATE)
    talk = bandpass(noise(rng, n), 250, 1100, r)
    syll = [0.4 + 0.6 * abs(math.sin(TAU * 3.1 * i / r)) * (0.5 + 0.5 * math.sin(TAU * 0.4 * i / r)) for i in range(n)]
    track = [a * s for a, s in zip(talk, syll)]
    for _ in range(6):
        knock = [math.sin(TAU * 180 * i / r) * math.exp(-40 * i / r) for i in range(int(0.12 * r))]
        place(track, knock, rng.uniform(0.3, 7.8), r, 1.2)
    return loop(track, r)


def crickets(rng):
    r, n = AMBIENT_RATE, int(6.5 * AMBIENT_RATE)
    track = [0.0] * n
    t = 0.1
    while t < 6.3:
        pulse = [math.sin(TAU * 4200 * i / r) * math.sin(math.pi * i / int(0.03 * r)) for i in range(int(0.03 * r))]
        for k in range(3):
            place(track, pulse, t + k * 0.05, r, 0.5)
        t += rng.uniform(0.35, 0.6)
    return loop(track, r)


# ---------------------------------------------------------------- effects

def tap(rng):
    r = EFFECT_RATE
    n = int(0.06 * r)
    return [math.sin(TAU * (1400 - 6000 * i / r) * i / r) * math.exp(-60 * i / r) for i in range(n)]


def knock(rng, freq=150, seconds=0.16):
    r = EFFECT_RATE
    n = int(seconds * r)
    thump = [math.sin(TAU * freq * i / r) * math.exp(-28 * i / r) for i in range(n)]
    click = [a * math.exp(-90 * i / r) for i, a in enumerate(lowpass(noise(rng, n), 2500, r))]
    return mix(thump, [c * 0.6 for c in click])


def build(rng):
    r = EFFECT_RATE
    track = [0.0] * int(0.45 * r)
    place(track, knock(rng, 140), 0.0, r)
    place(track, knock(rng, 170), 0.2, r, 0.8)
    return track


def hammer(rng):
    r = EFFECT_RATE
    track = [0.0] * int(0.6 * r)
    for k in range(3):
        ping = bell(880 + 40 * k, 0.15, r, decay=30, partials=((1, 1.0), (2.7, 0.6), (5.1, 0.3)))
        place(track, mix(ping, knock(rng, 220, 0.1)), k * 0.18, r, 0.6)
    return track


def done(rng):
    r = EFFECT_RATE
    track = [0.0] * int(1.2 * r)
    place(track, bell(523.25, 1.0, r), 0.0, r)
    place(track, bell(783.99, 0.9, r), 0.14, r, 0.8)
    return track


def discovery(rng):
    r = EFFECT_RATE
    track = [0.0] * int(1.6 * r)
    for k, f in enumerate([523.25, 659.25, 783.99, 1046.5]):
        place(track, bell(f, 1.0, r, decay=4.0), k * 0.11, r, 0.7)
    return track


def era(rng):
    r = EFFECT_RATE
    track = [0.0] * int(2.6 * r)
    for k, chord in enumerate([(261.6, 329.6, 392.0), (349.2, 440.0, 523.3), (392.0, 493.9, 587.3), (523.3, 659.3, 784.0)]):
        length = 0.9 if k == 3 else 0.42
        n = int(length * r)
        e = env(n, 0.02, 0.3 if k == 3 else 0.08, r)
        tone = [
            sum(math.sin(TAU * f * i / r) + 0.3 * math.sin(TAU * 2 * f * i / r) + 0.15 * math.sin(TAU * 3 * f * i / r) for f in chord) * e[i]
            for i in range(n)
        ]
        place(track, tone, k * 0.45, r, 0.5)
    return track


def deliver(rng):
    r = EFFECT_RATE
    n = int(0.12 * r)
    return [math.sin(TAU * (500 + 2500 * i / n) * i / r) * math.sin(math.pi * i / n) * 0.7 for i in range(n)]


def boat(rng):
    r = EFFECT_RATE
    n = int(1.1 * r)
    e = env(n, 0.08, 0.4, r)
    horn = [(math.sin(TAU * 147 * i / r) + 0.5 * math.sin(TAU * 294 * i / r) + 0.25 * math.sin(TAU * 441 * i / r)) * e[i] for i in range(n)]
    return horn


def find(rng):
    r = EFFECT_RATE
    track = [0.0] * int(1.0 * r)
    for k in range(6):
        place(track, bell(1318.5 * (1.0 + k * 0.12), 0.5, r, decay=7.0), k * 0.06, r, 0.5)
    return track


def error(rng):
    r = EFFECT_RATE
    n = int(0.22 * r)
    e = env(n, 0.005, 0.05, r)
    return [(1.0 if math.sin(TAU * 110 * i / r) > 0 else -1.0) * 0.3 * e[i] for i in range(n)]


def main():
    for name, make in [("space", space), ("wind", wind), ("waves", waves), ("birds", birds), ("village", village), ("crickets", crickets)]:
        write("amb_" + name, make(random.Random(name)), AMBIENT_RATE)
    for name, make in [("tap", tap), ("build", build), ("hammer", hammer), ("done", done), ("discovery", discovery), ("era", era), ("deliver", deliver), ("boat", boat), ("find", find), ("error", error)]:
        write("sfx_" + name, make(random.Random(name)), EFFECT_RATE)
    print("sounds written to", os.path.abspath(OUT))


if __name__ == "__main__":
    main()
