"""Honeybound's sound: effects as synthesized voices and the music as scores (`red_engine2 describe audio`).

Every tune is original. A melody is written bar by bar as scale degrees ("5 . 5 6 5 . 3 .": eighth notes, "." rests, 8 = the octave) and becomes
one `pattern` track per bar (a pattern repeats each bar, so a bar of its own is how a line of melody is written in this score format).
"""

SQUARE = [[3, 0.33], [5, 0.2], [7, 0.14], [9, 0.11]]  # odd harmonics: a chiptune square
SOFT_SQUARE = [[3, 0.2], [5, 0.08]]


def tone(hz, decay=12.0, delay=0.0, gain=1.0, harm=SQUARE, attack=0.002):
    layer = {"sine": hz, "decay": decay, "attack": attack, "harmonics": harm}
    if delay:
        layer["delay"] = delay
    if gain != 1.0:
        layer["gain"] = gain
    return layer


def notes(seq, step, decay=10.0, harm=SQUARE, gain=1.0):
    """A little tune as delayed tones: seq is [(hz, steps)...] played one after another."""
    out, t = [], 0.0
    for hz, n in seq:
        if hz:
            out.append(tone(hz, decay=decay, delay=round(t, 4), gain=gain, harm=harm))
        t += n * step
    return out


N = {  # equal-tempered note frequencies used by the jingles
    "C4": 261.63, "D4": 293.66, "E4": 329.63, "F4": 349.23, "G4": 392.0, "A4": 440.0, "B4": 493.88,
    "C5": 523.25, "D5": 587.33, "E5": 659.25, "F5": 698.46, "G5": 783.99, "A5": 880.0, "B5": 987.77,
    "C6": 1046.5, "D6": 1174.66, "E6": 1318.51, "G6": 1567.98, "A3": 220.0, "G3": 196.0, "E3": 164.81, "C3": 130.81, "F3": 174.61, "D3": 146.83,
    "Bb4": 466.16, "Ab4": 415.3, "Eb5": 622.25, "Bb5": 932.33,
}


def sounds():
    out = _sounds()
    for v in out.values():
        for layer in v["layers"]:
            if "sine" not in layer:
                layer.pop("harmonics", None)  # only a sine layer has harmonics
    return out


def _sounds():
    return {
        "jump": {"seconds": 0.22, "level": 0.32, "layers": [{"glide": [330, 760, 9], "decay": 9, "attack": 0.003, "harmonics": SOFT_SQUARE}]},
        "jump_big": {"seconds": 0.26, "level": 0.32, "layers": [{"glide": [220, 560, 8], "decay": 8, "attack": 0.003, "harmonics": SOFT_SQUARE}]},
        "honey": {"seconds": 0.5, "level": 0.3, "layers": [tone(N["B5"], 30), tone(N["E6"], 7, delay=0.07)]},
        "stomp": {"seconds": 0.2, "level": 0.5, "layers": [{"glide": [520, 140, 18], "decay": 16, "attack": 0.002, "harmonics": SOFT_SQUARE}, {"noise": "hp 0.12", "decay": 30, "gain": 0.35}]},
        "bump": {"seconds": 0.14, "level": 0.5, "layers": [{"glide": [180, 90, 14], "decay": 20, "attack": 0.002, "harmonics": SOFT_SQUARE}, {"noise": "hp 0.1", "decay": 40, "gain": 0.3}]},
        "break": {"seconds": 0.45, "level": 0.45, "layers": [{"noise": "hp 0.15", "decay": 9, "attack": 0.002, "gain": 0.7}, {"glide": [300, 80, 6], "decay": 10, "gain": 0.5, "harmonics": SOFT_SQUARE}]},
        "powerup": {"seconds": 0.8, "level": 0.3, "layers": notes([(N["C5"], 1), (N["E5"], 1), (N["G5"], 1), (N["C6"], 1), (N["E5"], 1), (N["G5"], 1), (N["C6"], 1), (N["E6"], 2)], 0.06, decay=14)},
        "sprout": {"seconds": 0.6, "level": 0.32, "layers": [{"sweep": [180, 720, 3], "decay": 3, "attack": 0.01, "harmonics": SOFT_SQUARE}]},
        "shrink": {"seconds": 0.7, "level": 0.3, "layers": notes([(N["E6"], 1), (N["C6"], 1), (N["G5"], 1), (N["E5"], 1), (N["C5"], 1), (N["G4"], 1), (N["E4"], 2)], 0.06, decay=14)},
        "seed": {"seconds": 0.14, "level": 0.32, "layers": [{"glide": [900, 300, 16], "decay": 16, "attack": 0.002, "harmonics": SOFT_SQUARE}]},
        "kick": {"seconds": 0.1, "level": 0.45, "layers": [{"glide": [700, 200, 30], "decay": 30, "attack": 0.001, "harmonics": SOFT_SQUARE}]},
        "pipe": {"seconds": 0.9, "level": 0.32, "layers": notes([(N["G4"], 1), (N["C4"], 1), (N["G4"], 1), (N["C4"], 1), (N["G4"], 1), (N["C4"], 1)], 0.11, decay=10, harm=SOFT_SQUARE)},
        "oneup": {"seconds": 0.9, "level": 0.3, "layers": notes([(N["E5"], 1), (N["G5"], 1), (N["E6"], 1), (N["C6"], 1), (N["D6"], 1), (N["G6"], 2)], 0.09, decay=9)},
        "lose": {
            "seconds": 3.0,
            "level": 0.3,
            "layers": notes([(N["C5"], 1), (N["C5"], 1), (None, 1), (N["G4"], 2), (N["E4"], 2), (N["A4"], 1.5), (N["B4"], 1.5), (N["A4"], 1.5), (N["Ab4"], 1.5), (N["Bb4"], 1.5), (N["Ab4"], 1.5), (N["G4"], 1), (N["F4"], 1), (N["G4"], 4)], 0.1, decay=5),
        },
        "clear": {
            "seconds": 3.4,
            "level": 0.3,
            "layers": notes([(N["G4"], 1), (N["C5"], 1), (N["E5"], 1), (N["G5"], 1), (N["C6"], 1), (N["E6"], 1), (N["G6"], 3), (N["E6"], 3), (N["Ab4"], 1), (N["C5"], 1), (N["Eb5"], 1), (N["Ab4"], 1), (N["C5"], 1), (N["Eb5"], 1), (N["Bb4"], 1), (N["D5"], 1), (N["F5"], 1), (N["Bb5"], 3), (N["C6"], 6)], 0.09, decay=4),
        },
        "gameover": {
            "seconds": 4.0,
            "level": 0.3,
            "layers": notes([(N["C5"], 2), (N["G4"], 2), (N["E4"], 2), (N["A4"], 1.5), (N["B4"], 1.5), (N["A4"], 1.5), (N["Ab4"], 1.5), (N["Bb4"], 1.5), (N["Ab4"], 1.5), (N["G4"], 1), (N["D4"], 1), (N["E4"], 6)], 0.14, decay=3.5),
        },
        "tick": {"seconds": 0.05, "level": 0.18, "layers": [tone(N["C6"], 60)]},
        "flag": {"seconds": 1.2, "level": 0.3, "layers": [{"sweep": [1200, 300, 1.4], "decay": 1.8, "attack": 0.004, "harmonics": SOFT_SQUARE}]},
        "bossfall": {"seconds": 2.0, "level": 0.5, "layers": [{"sweep": [400, 60, 1.2], "decay": 1.4, "attack": 0.01, "harmonics": SQUARE}, {"noise": "hp 0.08", "decay": 1.8, "gain": 0.4}]},
        "hurry": {"seconds": 1.2, "level": 0.3, "layers": notes([(N["E6"], 1), (N["C6"], 1), (N["E6"], 1), (N["C6"], 1), (N["E6"], 1), (N["C6"], 1), (N["G6"], 3)], 0.09, decay=12)},
        "pause": {"seconds": 0.35, "level": 0.3, "layers": notes([(N["E6"], 1), (N["C6"], 1), (N["E6"], 1), (N["C6"], 1)], 0.06, decay=20)},
        "lever": {"seconds": 0.5, "level": 0.45, "layers": [{"noise": "hp 0.4", "decay": 20, "attack": 0.001}, tone(N["G3"], 8, gain=0.8, harm=SQUARE)]},
        "breathe": {"seconds": 0.6, "level": 0.35, "layers": [{"noise": "hp 0.12", "decay": 4, "attack": 0.03, "gain": 0.6}, {"glide": [160, 90, 3], "decay": 5, "gain": 0.4, "harmonics": SQUARE}]},
        "start": {"seconds": 0.9, "level": 0.3, "layers": notes([(N["C5"], 1), (N["G5"], 1), (N["E5"], 1), (N["C6"], 3)], 0.1, decay=8)},
    }


# ---- music --------------------------------------------------------------------------------------------------------------------------------------


def melody(bars, inst, octave, gain=0.8, hold=None, vel=(0.85, 1.0)):
    """One pattern track per bar: tokens are scale degrees or '.' (rest)."""
    tracks = []
    for i, bar in enumerate(bars):
        toks = bar.split()
        degs = [t for t in toks if t != "."]
        if not degs:
            continue
        t = {"inst": inst, "play": "pattern", "pattern": "".join("x" if t != "." else "." for t in toks), "notes": " ".join(degs), "from": i + 1, "to": i + 1, "octave": octave, "gain": gain, "vel": list(vel)}
        if hold:
            t["hold"] = hold
        tracks.append(t)
    return tracks


LEAD = {"seconds": 0.6, "level": 0.5, "layers": [{"sine": 1, "decay": 5, "attack": 0.004, "release": 0.05, "harmonics": SQUARE}]}
PULSE = {"seconds": 0.5, "level": 0.45, "layers": [{"sine": 1, "decay": 7, "attack": 0.003, "release": 0.05, "harmonics": [[2, 0.5], [3, 0.3], [4, 0.2], [5, 0.15]]}]}
BASS = {"seconds": 0.5, "level": 0.6, "layers": [{"sine": 1, "decay": 6, "attack": 0.004, "release": 0.04, "harmonics": [[3, 0.25], [5, 0.1]]}]}
HAT = {"seconds": 0.08, "level": 0.25, "layers": [{"noise": "hp 0.6", "decay": 60, "attack": 0.001}]}
PAD = {"seconds": 3, "level": 0.35, "layers": [{"sine": 1, "attack": 0.3, "release": 1.0}, {"sine": 2, "attack": 0.4, "release": 1.0, "gain": 0.25}, {"sine": 3, "attack": 0.5, "release": 1.0, "gain": 0.08}]}
BELL = {"seconds": 1.6, "level": 0.4, "layers": [{"sine": 1, "decay": 3}, {"sine": 2.76, "decay": 5, "gain": 0.25}, {"sine": 5.4, "decay": 8, "gain": 0.1}]}
PLUCK = {"seconds": 0.8, "level": 0.5, "layers": [{"sine": 1, "decay": 9, "attack": 0.002, "harmonics": [[2, 0.4], [3, 0.2], [4, 0.1]]}]}


def music():
    meadow = {
        "bpm": 150, "beats": 4, "bars": 8, "key": "C", "scale": "major", "seed": 3, "lufs": -21,
        "reverb": {"decay": 1.2, "mix": 0.12},
        "instruments": {"lead": LEAD, "bass": BASS, "hat": HAT, "pad": PAD, "pulse": PULSE},
        "tracks": melody(
            [
                "5 . 5 6 5 . 3 .",
                "4 . 4 5 4 . 2 .",
                "3 . 3 4 5 . 8 .",
                "7 . 6 . 5 . . .",
                "5 . 5 6 5 . 3 .",
                "4 . 4 5 4 . 9 8",
                "7 . 5 . 6 . 7 .",
                "8 . . 5 8 . . .",
            ],
            "lead", 5, gain=0.75,
        )
        + melody(["1 . 5 . 1 . 5 .", "4 . 1 . 4 . 1 .", "1 . 5 . 1 . 5 .", "5 . 2 . 5 . 5 .", "1 . 5 . 1 . 5 .", "4 . 1 . 2 . 6 .", "5 . 2 . 5 . 7 .", "1 . 5 . 1 . . ."], "bass", 2, gain=0.9)
        + [
            {"inst": "hat", "play": "pattern", "pattern": "x.x.x.x.", "gain": 0.6, "octave": 6, "vel": [0.5, 1.0]},
            {"inst": "pulse", "play": "chords", "chords": "I IV I V I IV ii V", "every": "1 bar", "octave": 4, "gain": 0.28, "strum": 0.08},
        ],
    }
    roots = {
        "bpm": 116, "beats": 4, "bars": 8, "key": "A", "scale": "minor", "seed": 5, "lufs": -23,
        "delay": {"time": 0.39, "feedback": 0.35, "mix": 0.25}, "reverb": {"decay": 2, "mix": 0.2},
        "instruments": {"pluck": PLUCK, "bass": BASS},
        "tracks": melody(
            ["1 . 8 . 7 . 5 .", "6 . . 5 . 3 . .", "1 . 8 . 7 . 5 .", "4 . 3 . 2 . . .", "1 . 8 . 7 . 5 .", "6 . . 8 . 10 . .", "9 . 8 . 7 . 5 .", "1 . . . . . . ."],
            "pluck", 4, gain=0.8,
        )
        + melody(["1 . . 1 . . 1 .", "6 . . 6 . . 6 .", "1 . . 1 . . 1 .", "4 . . 4 . . 5 .", "1 . . 1 . . 1 .", "6 . . 6 . . 6 .", "7 . . 7 . . 5 .", "1 . . 1 . . 1 ."], "bass", 2, gain=0.9),
    }
    canopy = {
        "bpm": 132, "beats": 3, "bars": 8, "key": "F", "scale": "major", "seed": 7, "lufs": -22,
        "reverb": {"decay": 1.6, "mix": 0.18},
        "instruments": {"lead": PULSE, "bass": BASS, "bell": BELL},
        "tracks": melody(["3 . 5 . 8 .", "7 . 5 . 3 .", "4 . 6 . 9 .", "8 . . . . .", "3 . 5 . 8 .", "10 . 9 . 8 .", "7 . 6 . 7 .", "8 . . . 5 ."], "lead", 5, gain=0.7)
        + melody(["1 . 5 . 5 .", "1 . 5 . 5 .", "4 . 1 . 1 .", "5 . 2 . 2 .", "1 . 5 . 5 .", "6 . 3 . 3 .", "5 . 2 . 2 .", "1 . 5 . 5 ."], "bass", 2, gain=0.85)
        + [{"inst": "bell", "play": "pattern", "pattern": "..x...", "notes": "8 10 12 10", "octave": 5, "gain": 0.25}],
    }
    keep = {
        "bpm": 104, "beats": 4, "bars": 4, "key": "D", "scale": "harmonic_minor", "seed": 9, "lufs": -22,
        "reverb": {"decay": 2.4, "mix": 0.25},
        "instruments": {"lead": LEAD, "bass": BASS, "pad": PAD},
        "tracks": melody(["1 2 1 2 1 2 3 2", "1 2 1 2 1 2 3 2", "4 5 4 5 4 5 6 5", "7 6 5 4 3 2 1 7"], "lead", 4, gain=0.55)
        + melody(["1 . 1 . 1 . 1 .", "6 . 6 . 6 . 6 .", "4 . 4 . 4 . 4 .", "5 . 5 . 5 . 5 ."], "bass", 2, gain=0.9)
        + [{"inst": "pad", "play": "chords", "chords": "i VI iv V", "every": "1 bar", "octave": 3, "gain": 0.4}],
    }
    star = {
        "bpm": 184, "beats": 4, "bars": 4, "key": "C", "scale": "major", "seed": 2, "lufs": -20,
        "instruments": {"lead": LEAD, "bass": BASS, "hat": HAT},
        "tracks": melody(["1 . 1 . 1 3 . 1", ". 3 . 1 3 . 5 .", "2 . 2 . 2 4 . 2", ". 4 . 2 4 . 6 ."], "lead", 5, gain=0.7)
        + melody(["1 5 1 5 1 5 1 5", "1 5 1 5 1 5 1 5", "2 6 2 6 2 6 2 6", "5 2 5 2 5 2 5 2"], "bass", 2, gain=0.85)
        + [{"inst": "hat", "play": "pattern", "pattern": "xxxxxxxx", "gain": 0.5, "octave": 6, "vel": [0.4, 1.0]}],
    }
    dusk = {
        "bpm": 128, "beats": 4, "bars": 8, "key": "E", "scale": "dorian", "seed": 13, "lufs": -22,
        "reverb": {"decay": 1.8, "mix": 0.2}, "delay": {"time": 0.47, "feedback": 0.25, "mix": 0.15},
        "instruments": {"lead": PULSE, "bass": BASS, "hat": HAT, "pad": PAD},
        "tracks": melody(
            ["1 . 3 . 5 . 6 5", "4 . . 3 . . 2 .", "1 . 3 . 5 . 8 7", "6 . . . 5 . . .", "1 . 3 . 5 . 6 5", "4 . . 6 . . 8 .", "7 . 6 . 4 . 2 .", "1 . . . . . . ."],
            "lead", 5, gain=0.7,
        )
        + melody(["1 . . 1 5 . 1 .", "4 . . 4 1 . 4 .", "1 . . 1 5 . 1 .", "6 . . 6 3 . 6 .", "1 . . 1 5 . 1 .", "4 . . 4 1 . 4 .", "7 . . 7 4 . 5 .", "1 . . 1 5 . 1 ."], "bass", 2, gain=0.9)
        + [{"inst": "hat", "play": "pattern", "pattern": "..x...x.", "gain": 0.5, "octave": 6}, {"inst": "pad", "play": "chords", "chords": "i IV i VII i IV VII i", "every": "1 bar", "octave": 3, "gain": 0.3}],
    }
    hush = {
        "bpm": 60, "beats": 4, "bars": 2, "key": "C", "scale": "major", "seed": 1, "lufs": -36,
        "reverb": {"decay": 4, "mix": 0.4},
        "instruments": {"pad": PAD},
        "tracks": [{"inst": "pad", "play": "chords", "chords": "I", "every": "2 bars", "octave": 3, "gain": 0.3}],
    }
    ending = {
        "bpm": 92, "beats": 4, "bars": 8, "key": "G", "scale": "major", "seed": 21, "lufs": -22,
        "reverb": {"decay": 3, "mix": 0.3},
        "instruments": {"bell": BELL, "pad": PAD, "bass": BASS},
        "tracks": melody(["5 . 6 . 8 . . .", "7 . 6 . 5 . . .", "4 . 5 . 6 . 8 .", "5 . . . . . . .", "5 . 6 . 8 . 10 .", "9 . 8 . 7 . 6 .", "5 . 4 . 2 . 7 .", "8 . . . . . . ."], "bell", 5, gain=0.7)
        + [{"inst": "pad", "play": "chords", "chords": "I vi IV V I iii IV I", "every": "1 bar", "octave": 3, "gain": 0.45}]
        + melody(["1 . . . 5 . . .", "6 . . . 3 . . .", "4 . . . 1 . . .", "5 . . . 2 . . .", "1 . . . 5 . . .", "3 . . . 7 . . .", "4 . . . 5 . . .", "1 . . . . . . ."], "bass", 2, gain=0.8),
    }
    return {"meadow": meadow, "roots": roots, "canopy": canopy, "keep": keep, "star": star, "dusk": dusk, "hush": hush, "ending": ending}
