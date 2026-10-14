"""An original 8-channel ProTracker MOD for the store screenshots:
synthesized samples, a short A-minor groove. Writes demo.mod."""
import math, random, struct, sys

random.seed(7)
RATE = 8287  # C-2 at period 428

def clip(x):
    return max(-128, min(127, int(round(x))))

def kick():
    out, ph = [], 0.0
    for i in range(3000):
        t = i / RATE
        f = 40 + 140 * math.exp(-t * 28)
        ph += 2 * math.pi * f / RATE
        out.append(clip(120 * math.sin(ph) * math.exp(-t * 7)))
    return out, 0, 0

def snare():
    out = []
    for i in range(2600):
        t = i / RATE
        n = random.uniform(-1, 1)
        tone = math.sin(2 * math.pi * 190 * t)
        out.append(clip((90 * n * math.exp(-t * 14)) + 50 * tone * math.exp(-t * 30)))
    return out, 0, 0

def hat():
    out, prev = [], 0.0
    for i in range(900):
        t = i / RATE
        n = random.uniform(-1, 1)
        hp = n - prev
        prev = n
        out.append(clip(60 * hp * math.exp(-t * 45)))
    return out, 0, 0

def tonal(wave, n=4096, loop_from=2048, cycle=32.0, detune=None, decay=0.0):
    out = []
    for i in range(n):
        p = (i % cycle) / cycle
        v = wave(p)
        if detune:
            v = 0.6 * v + 0.4 * wave(((i % detune) / detune))
        env = math.exp(-decay * i / n) if decay else 1.0
        out.append(clip(100 * v * env))
    return out, loop_from, n - loop_from

saw = lambda p: 2 * p - 1
square = lambda p: 1.0 if p < 0.5 else -1.0
pulse = lambda p: 1.0 if p < 0.25 else -1.0
tri = lambda p: 4 * p - 1 if p < 0.5 else 3 - 4 * p
sine2 = lambda p: 0.8 * math.sin(2 * math.pi * p) + 0.2 * math.sin(6 * math.pi * p)

SAMPLES = [
    ("kick drum", kick(), 64),
    ("snare", snare(), 56),
    ("closed hat", hat(), 40),
    ("bass saw", tonal(saw, cycle=64.0, detune=64.6), 60),
    ("lead pulse", tonal(pulse, detune=32.25), 48),
    ("chord square", tonal(square, cycle=32.0), 30),
    ("pad tri", tonal(tri, detune=32.15), 36),
    ("bell sine", tonal(sine2, n=4096, loop_from=3072, decay=1.2), 44),
]

# Periods, ProTracker octaves 1-3 (finetune 0).
PERIODS = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
           428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226,
           214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113]
NAMES = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"]

def note(name):
    """'A-2' -> period (octave 1..3)."""
    i = NAMES.index(name[:2]) + 12 * (int(name[2]) - 1)
    return PERIODS[i]

CH = 8
ROWS = 64

def empty():
    return [[None] * CH for _ in range(ROWS)]

def put(p, row, ch, n=None, smp=0, eff=0, prm=0):
    p[row][ch] = (note(n) if n else 0, smp, eff, prm)

BASS = ["A-1", "A-1", "A-2", "A-1", "F-1", "F-1", "F-2", "F-1",
        "C-2", "C-2", "C-3", "C-2", "G-1", "G-1", "G-2", "G-1"]
CHORDS = [("A-2", 0x37), ("F-2", 0x47), ("C-2", 0x47), ("G-2", 0x47)]
LEAD_A = ["E-3", None, "C-3", None, "D-3", "E-3", None, "G-3",
          "A-3", None, "G-3", "E-3", None, "D-3", "C-3", None]
LEAD_B = ["C-3", None, "D-3", "E-3", None, "G-3", "E-3", None,
          "D-3", None, "C-3", "A-2", None, "C-3", "D-3", None]

def drums(p, fill=False):
    for r in range(ROWS):
        if r % 16 in (0, 10):
            put(p, r, 0, "C-2", 1)
        if r % 16 == 8 or (fill and r >= 56 and r % 2 == 0):
            put(p, r, 1, "C-2", 2, 0xC, 0x30 if r % 16 != 8 else 0x40)
        if r % 4 == 2:
            put(p, r, 2, "C-3", 3, 0xC, 0x20 + (r % 8) * 4)

def bass(p):
    for i, r in enumerate(range(0, ROWS, 4)):
        put(p, r, 3, BASS[i], 4)
        if i % 4 == 3:
            put(p, r + 2, 3, None, 0, 0xC, 0x10)

def chords(p, arp=True):
    for b, (n, a) in enumerate(CHORDS):
        r = b * 16
        put(p, r, 5, n, 6, 0x0 if arp else 0xC, a if arp else 0x28)
        for k in range(1, 15):
            if arp:
                put(p, r + k, 5, None, 0, 0x0, a)
        put(p, r + 15, 5, None, 0, 0xA, 0x04)
        put(p, r, 6, n.replace("2", "1"), 7, 0xC, 0x24)

def lead(p, line, ch=4, echo=7):
    for i, n in enumerate(line):
        r = i * 4
        if n:
            put(p, r, ch, n, 5)
            put(p, r + 2, ch, None, 0, 0x4, 0x46)
            if r + 3 < ROWS:
                put(p, r + 3, echo, n, 8, 0xC, 0x18)

def pattern(kind):
    p = empty()
    if kind == 0:  # intro: drums, bass, pad
        drums(p)
        bass(p)
        chords(p, arp=False)
        put(p, 0, 7, None, 0, 0xF, 0x06)
    elif kind == 1:  # verse
        drums(p)
        bass(p)
        chords(p)
        lead(p, LEAD_A)
    elif kind == 2:  # verse 2
        drums(p, fill=True)
        bass(p)
        chords(p)
        lead(p, LEAD_B)
    else:  # break: bells
        drums(p)
        bass(p)
        for i, n in enumerate(["A-2", "C-3", "E-3", "G-3", "E-3", "C-3", "D-3", "B-2",
                               "C-3", "E-3", "G-3", "A-3", "G-3", "E-3", "D-3", "C-3"]):
            put(p, i * 4, 7, n, 8)
            put(p, i * 4 + 2, 7, None, 0, 0xA, 0x02)
        chords(p, arp=False)
    return p

PATS = [pattern(k) for k in range(4)]
ORDER = [0, 1, 2, 1, 3, 2, 1, 2]

def pack():
    b = bytearray(b"Neumetik demo".ljust(20, b"\0"))
    for i in range(31):
        if i < len(SAMPLES):
            name, (data, ls, ll), vol = SAMPLES[i]
            if len(data) % 2:
                data = data + [0]
            b += name.encode().ljust(22, b"\0")
            b += struct.pack(">HBBHH", len(data) // 2, 0, vol, ls // 2, max(1, ll // 2))
        else:
            b += b"\0" * 22 + struct.pack(">HBBHH", 0, 0, 0, 0, 1)
    b += bytes([len(ORDER), 127]) + bytes(ORDER + [0] * (128 - len(ORDER)))
    b += b"8CHN"
    for p in PATS:
        for row in p:
            for cell in row:
                per, smp, eff, prm = cell or (0, 0, 0, 0)
                b += bytes([(smp & 0xF0) | (per >> 8), per & 0xFF, ((smp & 0xF) << 4) | eff, prm])
    for name, (data, ls, ll), vol in SAMPLES:
        if len(data) % 2:
            data = data + [0]
        b += bytes((v + 256) % 256 for v in data)
    return bytes(b)

open(sys.argv[1] if len(sys.argv) > 1 else "demo.mod", "wb").write(pack())
