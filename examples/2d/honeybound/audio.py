"""Honeybound's sound: effects as synthesized voices and an original soundtrack of named scores (`red_engine2 describe audio`).

Every tune is written here as scale degrees, bar by bar ("5 . 5 6 | 8 . . ."; "." rests; the number of tokens in a bar is its grid), and becomes one
`pattern` track per bar. Each track has a `title`: the pause menu shows it and "save this song" names the file after it.

The soundtrack grows from one motif, Pip's: 1 3 5 6 5 (a small rising question), heard plainly on the title, bravely in the meadow, darkly in the keeps,
and whole again at the end. Every world has its own key, mode, tempo and band, so a world sounds like where it is.
"""

# ---- instruments -----------------------------------------------------------------------------------------------------------------------------------------
SOFT = [[2, 0.18], [3, 0.12], [4, 0.05]]
REED = [[2, 0.45], [3, 0.3], [4, 0.18], [5, 0.12], [6, 0.06]]  # a fiddle/oboe-ish buzz
HOLLOW = [[3, 0.3], [5, 0.14], [7, 0.07]]  # clarinet-ish (odd harmonics), never the bright NES square

INST = {
    "flute": {"seconds": 0.9, "level": 0.5, "layers": [{"sine": 1, "attack": 0.03, "decay": 2.2, "release": 0.12, "harmonics": SOFT}, {"noise": "lp 0.3", "attack": 0.01, "decay": 30, "gain": 0.05}]},
    "clar": {"seconds": 0.8, "level": 0.45, "layers": [{"sine": 1, "attack": 0.02, "decay": 2.5, "release": 0.1, "harmonics": HOLLOW}]},
    "fiddle": {"seconds": 0.8, "level": 0.42, "layers": [{"sine": 1, "attack": 0.05, "decay": 1.8, "release": 0.12, "harmonics": REED}]},
    "pluck": {"seconds": 0.9, "level": 0.5, "layers": [{"sine": 1, "decay": 7, "attack": 0.002, "harmonics": [[2, 0.4], [3, 0.2], [4, 0.1]]}]},
    "harp": {"seconds": 1.6, "level": 0.45, "layers": [{"sine": 1, "decay": 3.2, "attack": 0.002, "harmonics": [[2, 0.25], [3, 0.08]]}]},
    "bell": {"seconds": 2.0, "level": 0.4, "layers": [{"sine": 1, "decay": 2.6}, {"sine": 2.76, "decay": 5, "gain": 0.22}, {"sine": 5.4, "decay": 9, "gain": 0.08}]},
    "celesta": {"seconds": 1.4, "level": 0.38, "layers": [{"sine": 1, "decay": 4}, {"sine": 4, "decay": 9, "gain": 0.18}]},
    "marimba": {"seconds": 0.7, "level": 0.5, "layers": [{"sine": 1, "decay": 9, "attack": 0.002}, {"sine": 4, "decay": 26, "gain": 0.3}]},
    "pad": {"seconds": 3, "level": 0.32, "layers": [{"sine": 1, "attack": 0.35, "release": 1.0}, {"sine": 2, "attack": 0.45, "release": 1.0, "gain": 0.22}, {"sine": 3, "attack": 0.5, "release": 1.0, "gain": 0.06}]},
    "organ": {"seconds": 2, "level": 0.3, "layers": [{"sine": 1, "attack": 0.04, "release": 0.3}, {"sine": 2, "attack": 0.04, "release": 0.3, "gain": 0.5}, {"sine": 3, "attack": 0.04, "release": 0.3, "gain": 0.3}, {"sine": 4, "attack": 0.04, "release": 0.3, "gain": 0.15}]},
    "strings": {"seconds": 2.4, "level": 0.3, "layers": [{"sine": 1, "attack": 0.18, "release": 0.6, "harmonics": [[2, 0.35], [3, 0.2], [4, 0.1], [5, 0.06]]}]},
    "bass": {"seconds": 0.6, "level": 0.6, "layers": [{"sine": 1, "decay": 5, "attack": 0.004, "release": 0.04, "harmonics": [[2, 0.3], [3, 0.12]]}]},
    "upright": {"seconds": 0.8, "level": 0.6, "layers": [{"sine": 1, "decay": 6, "attack": 0.006, "harmonics": [[2, 0.2]]}, {"noise": "lp 0.2", "decay": 60, "gain": 0.1}]},
    "kick": {"seconds": 0.3, "level": 0.7, "layers": [{"glide": [140, 45, 18], "decay": 14, "attack": 0.001}]},
    "snare": {"seconds": 0.2, "level": 0.4, "layers": [{"noise": "hp 0.25", "decay": 24, "attack": 0.001}, {"glide": [220, 160, 20], "decay": 30, "gain": 0.4}]},
    "hat": {"seconds": 0.08, "level": 0.22, "layers": [{"noise": "hp 0.6", "decay": 60, "attack": 0.001}]},
    "shaker": {"seconds": 0.12, "level": 0.2, "layers": [{"noise": "hp 0.45", "attack": 0.02, "decay": 40}]},
    "tom": {"seconds": 0.4, "level": 0.5, "layers": [{"glide": [180, 90, 8], "decay": 9, "attack": 0.002}, {"noise": "lp 0.3", "decay": 30, "gain": 0.15}]},
}
# drums ignore pitch: a fixed sine is its own frequency, so these play with any note
DRUMS = ("kick", "snare", "hat", "shaker", "tom")


def mel(bars, inst, octave, gain=0.8, hold=None, vel=(0.8, 1.0), first=1, pan=None, send=None):
    """A line, one pattern track per bar: `bars` is "1 . 3 5 | 6 . . ." (a bar's token count is its grid), `first` the bar it starts in."""
    tracks = []
    for i, bar in enumerate(b for b in bars.split("|")):
        toks = bar.split()
        degs = [t for t in toks if t != "."]
        if not degs:
            continue
        t = {"inst": inst, "play": "pattern", "pattern": "".join("x" if t != "." else "." for t in toks), "notes": " ".join(degs), "from": first + i, "to": first + i,
             "octave": octave, "gain": gain, "vel": list(vel)}
        if hold:
            t["hold"] = hold
        if pan is not None:
            t["pan"] = pan
        if send is not None:
            t["send"] = send
        tracks.append(t)
    return tracks


def beat(inst, pattern, gain=0.6, frm=None, to=None, vel=(0.6, 1.0)):
    t = {"inst": inst, "play": "pattern", "pattern": pattern, "notes": "1", "octave": 3, "gain": gain, "vel": list(vel), "send": 0.2}
    if frm:
        t["from"] = frm
    if to:
        t["to"] = to
    return t


def chords(prog, inst="pad", octave=3, gain=0.35, every="1 bar", strum=0.0, frm=None, to=None, voicing=None):
    t = {"inst": inst, "play": "chords", "chords": prog, "every": every, "octave": octave, "gain": gain}
    if strum:
        t["strum"] = strum
    if frm:
        t["from"] = frm
    if to:
        t["to"] = to
    if voicing:
        t["voicing"] = voicing
    return t


def score(title, bpm, key, scale, bars, tracks, beats=4, lufs=-21, reverb=(1.4, 0.16), delay=None, seed=1):
    used = sorted({t["inst"] for t in tracks})
    s = {"title": title, "bpm": bpm, "beats": beats, "bars": bars, "key": key, "scale": scale, "seed": seed, "lufs": lufs,
         "reverb": {"decay": reverb[0], "mix": reverb[1]}, "instruments": {k: INST[k] for k in used}, "tracks": tracks}
    if delay:
        s["delay"] = {"time": delay[0], "feedback": delay[1], "mix": delay[2]}
    return s


# ---- the soundtrack -------------------------------------------------------------------------------------------------------------------------------------

def music():
    M = {}
    # Pip's motif: 1 3 5 6 5. The title states it simply, over a gentle sway.
    M["title"] = score("Honeybound", 104, "F", "major", 16, [
        *mel("1 . 3 . 5 . 6 5 | 3 . . . . . . . | 4 . 6 . 8 . 9 8 | 6 . . . 5 . . . | 1 . 3 . 5 . 6 5 | 8 . . . 7 . 6 . | 5 . 6 . 4 . 3 . | 2 . . . . . . . "
             "| 3 . 4 . 5 . 8 . | 7 . 6 . 5 . . . | 4 . 5 . 6 . 9 . | 8 . . . . . 5 . | 6 . 5 . 4 . 3 . | 2 . 3 . 4 . 7 . | 8 . . . 5 . 3 . | 1 . . . . . . .", "bell", 5, 0.7),
        *mel("1 . 5 . 8 . 5 . | 6 . 3 . 8 . 3 . | 4 . 8 . 6 . 8 . | 5 . 2 . 7 . 2 . " * 1 + "| 1 . 5 . 8 . 5 . | 3 . 7 . 5 . 7 . | 4 . 8 . 6 . 8 . | 5 . 2 . 7 . 5 . "
             "| 6 . 3 . 8 . 3 . | 3 . 7 . 5 . 7 . | 4 . 8 . 6 . 8 . | 1 . 5 . 8 . 5 . | 4 . 8 . 6 . 8 . | 5 . 2 . 7 . 2 . | 1 . 5 . 8 . 5 . | 1 . 5 . 8 . . .", "harp", 3, 0.55),
        chords("I vi IV V I iii IV V vi iii IV I IV V I I", "pad", 3, 0.3),
        *mel("1 . . . . . . . | 6 . . . . . . . | 4 . . . . . . . | 5 . . . . . . . " * 4, "upright", 2, 0.7),
    ], lufs=-21, reverb=(2.4, 0.28), seed=4)
    M["hive"] = score("The Hive at Dusk", 70, "D", "dorian", 8, [
        chords("i IV i VII i IV VII i", "pad", 3, 0.42),
        *mel("5 . . . 4 . 3 . | 1 . . . . . . . | 3 . . . 4 . 5 . | 4 . . . . . . . | 5 . . . 6 . 8 . | 7 . . . 5 . . . | 4 . . . 3 . 2 . | 1 . . . . . . .", "celesta", 5, 0.5),
        *mel("1 . . . | 4 . . . | 1 . . . | 7 . . . | 1 . . . | 4 . . . | 7 . . . | 1 . . .", "bass", 2, 0.5),
    ], lufs=-24, reverb=(4, 0.4), seed=6)
    M["meadow"] = score("Meadow Path", 138, "C", "major", 16, [
        # A: the motif, brave and bouncy; B: it climbs; A': home
        *mel("1 . 3 . 5 . 6 5 | . 3 . 1 2 . . . | 2 . 4 . 6 . 7 6 | . 4 . 2 3 . . . | 1 . 3 . 5 . 6 5 | . 8 . 6 5 . 3 . | 4 . 6 . 5 . 2 . | 1 . . . . . . . "
             "| 6 . . 5 6 . 8 . | 7 . . 6 5 . . . | 4 . . 3 4 . 6 . | 5 . . . . . . . | 6 . . 5 6 . 8 . | 9 . . 8 7 . 6 . | 5 . 6 . 7 . 2 . | 1 . . . 5 . 8 .", "flute", 5, 0.75),
        *mel("1 . 5 . 1 . 5 . | 1 . 5 . 6 . 5 . | 2 . 6 . 2 . 6 . | 5 . 2 . 5 . 7 . | 1 . 5 . 1 . 5 . | 1 . 5 . 6 . 5 . | 4 . 1 . 5 . 2 . | 1 . 5 . 1 . . . "
             "| 6 . 3 . 6 . 3 . | 3 . 7 . 3 . 7 . | 4 . 1 . 4 . 1 . | 5 . 2 . 5 . 7 . | 6 . 3 . 6 . 3 . | 3 . 7 . 3 . 7 . | 4 . 1 . 5 . 2 . | 1 . 5 . 1 . 5 .", "upright", 2, 0.85),
        chords("I I ii V I IV ii I vi iii IV V vi iii ii V", "pluck", 4, 0.22, strum=0.06),
        beat("kick", "x...x...", 0.5), beat("snare", "....x...", 0.35), beat("shaker", "..x...x.", 0.5),
    ], lufs=-20, reverb=(1.3, 0.14), seed=3)
    M["roots"] = score("Rootways", 108, "A", "minor", 16, [
        *mel("1 . 8 . 7 . 5 . | 6 . . 5 . 3 . . | 1 . 8 . 7 . 5 . | 4 . 3 . 2 . . . | 1 . 8 . 7 . 5 . | 6 . . 8 . 10 . . | 9 . 8 . 7 . 5 . | 1 . . . . . . . "
             "| 3 . . 4 5 . . . | 6 . . 5 3 . . . | 2 . . 3 4 . . . | 5 . . . . . . . | 3 . . 4 5 . . . | 6 . . 8 7 . . . | 5 . 4 . 2 . 7 . | 1 . . . . . . .", "pluck", 4, 0.8),
        *mel("1 . . 1 . . 5 . | 6 . . 6 . . 3 . | 1 . . 1 . . 5 . | 4 . . 4 . . 5 . " * 2 + "| 3 . . 3 . . 7 . | 4 . . 4 . . 1 . | 2 . . 2 . . 6 . | 5 . . 5 . . 7 . " * 2, "bass", 2, 0.85),
        beat("hat", "..x...x.", 0.4), beat("tom", "x.......", 0.3),
    ], lufs=-22, reverb=(2.2, 0.2), delay=(0.416, 0.35, 0.22), seed=5)
    M["canopy"] = score("Canopy Breeze", 126, "F", "lydian", 16, [
        *mel("5 . 6 . 8 . | 9 . 8 . 6 . | 5 . 4 . 2 . | 3 . . . . . | 5 . 6 . 8 . | 10 . 9 . 8 . | 6 . 8 . 9 . | 8 . . . . . "
             "| 3 . 4 . 5 . | 6 . 5 . 3 . | 2 . 3 . 4 . | 5 . . . 8 . | 9 . 8 . 6 . | 5 . 6 . 8 . | 7 . 5 . 2 . | 1 . . . . .", "flute", 5, 0.7),
        *mel("1 5 8 | 2 6 9 | 1 5 8 | 2 6 9 | 1 5 8 | 4 8 11 | 2 6 9 | 5 9 12 | 3 7 10 | 4 8 11 | 2 6 9 | 5 9 12 | 4 8 11 | 3 7 10 | 2 6 9 | 1 5 8", "harp", 3, 0.5),
        *mel("1 . . | 2 . . | 1 . . | 2 . . | 1 . . | 4 . . | 2 . . | 5 . . | 3 . . | 4 . . | 2 . . | 5 . . | 4 . . | 3 . . | 2 . . | 1 . .", "upright", 2, 0.7),
    ], beats=3, lufs=-21, reverb=(1.8, 0.2), seed=7)
    M["keep"] = score("Thornkeep", 96, "D", "harmonic_minor", 16, [
        *mel("1 . . 2 3 . . 2 | 1 . . . 7 . . . | 1 . . 2 3 . 4 . | 5 . . . . . . . | 6 . . 5 4 . . 3 | 2 . . . 5 . . . | 4 . 3 . 2 . 1 . | 7 . . . . . . . "
             "| 1 . . 2 3 . . 2 | 1 . . . 7 . . . | 3 . . 4 5 . 6 . | 8 . . . 7 . . . | 6 . . 5 4 . 3 . | 2 . . 3 4 . 5 . | 4 . 3 . 2 . 7 . | 1 . . . . . . .", "clar", 4, 0.7),
        *mel("1 . 1 . 1 . 1 . | 6 . 6 . 6 . 6 . | 4 . 4 . 4 . 4 . | 5 . 5 . 5 . 5 . " * 4, "bass", 2, 0.8),
        chords("i VI iv V i VI iv V i VI III V iv VI iv i", "organ", 3, 0.22),
        beat("tom", "x...x..x", 0.45), beat("snare", "......x.", 0.25),
    ], lufs=-21, reverb=(2.8, 0.28), seed=9)
    M["dusk"] = score("Duskwood Lanterns", 116, "E", "dorian", 16, [
        *mel("1 . 3 . 5 . 6 5 | 4 . . 3 . . 2 . | 1 . 3 . 5 . 8 7 | 6 . . . 5 . . . | 1 . 3 . 5 . 6 5 | 4 . . 6 . . 8 . | 7 . 6 . 4 . 2 . | 1 . . . . . . . "
             "| 8 . . 7 6 . 5 . | 6 . . 5 4 . . . | 7 . . 6 5 . 4 . | 5 . . . . . . . | 8 . . 7 6 . 5 . | 6 . . 8 9 . . . | 8 . 6 . 5 . 2 . | 1 . . . . . . .", "clar", 5, 0.7),
        *mel("1 . . 1 5 . 1 . | 4 . . 4 1 . 4 . | 1 . . 1 5 . 1 . | 6 . . 6 3 . 6 . " * 4, "upright", 2, 0.8),
        chords("i IV i VII i IV VII i iv VII III VI iv VII IV i", "pad", 3, 0.3),
        beat("hat", "x.x.x.x.", 0.35, vel=(0.3, 0.8)), beat("kick", "x.....x.", 0.4), beat("snare", "....x...", 0.25),
    ], lufs=-21, reverb=(1.9, 0.22), delay=(0.517, 0.25, 0.14), seed=13)
    M["autumn"] = _jig()
    M["frost"] = score("Frost on the Clover", 92, "B", "minor", 16, [
        *mel("5 . . 4 3 . . . | 1 . . . . . . . | 3 . . 4 5 . . 8 | 7 . . . . . . . | 6 . . 5 4 . . . | 3 . . . 5 . . . | 4 . . 3 2 . . 7 | 1 . . . . . . . "
             "| 8 . . 7 8 . 10 . | 9 . . . 8 . . . | 6 . . 5 6 . 8 . | 7 . . . . . . . | 8 . . 7 6 . 5 . | 4 . . 5 6 . . . | 5 . . 4 2 . . . | 1 . . . . . . .", "celesta", 5, 0.75),
        *mel("1 5 8 5 1 5 8 5 | 6 3 8 3 6 3 8 3 | 3 7 10 7 3 7 10 7 | 7 4 9 4 7 4 9 4 " * 4, "harp", 3, 0.35, vel=(0.5, 0.8)),
        chords("i VI III VII i VI iv VII III VII VI VII iv VI iv i", "strings", 3, 0.25),
    ], lufs=-22, reverb=(4.0, 0.36), delay=(0.652, 0.3, 0.18), seed=19)
    M["dunes"] = score("Sunset Dunes", 112, "E", "phrygian", 16, [
        *mel("1 . 2 . 1 . 7 . | 1 . . . 3 . 2 . | 1 . 2 . 3 . 4 . | 5 . . . . . . . | 5 . 6 . 5 . 4 . | 3 . . . 2 . . . | 3 . 2 . 1 . 7 . | 1 . . . . . . . "
             "| 8 . 7 . 6 . 5 . | 6 . . 5 4 . . . | 5 . 4 . 3 . 2 . | 3 . . . . . . . | 4 . 5 . 6 . 8 . | 9 . 8 . 6 . 5 . | 4 . 3 . 2 . 2 . | 1 . . . . . . .", "clar", 4, 0.75),
        *mel("1 . 1 . . 1 . . | 2 . 2 . . 2 . . | 1 . 1 . . 1 . . | 7 . 7 . . 5 . . " * 4, "pluck", 2, 0.75),
        beat("tom", "x..x..x.", 0.45), beat("shaker", "xx.xx.xx", 0.35, vel=(0.4, 0.9)), beat("snare", "........x.......", 0.25),
        chords("i II i VII i II VII i iv III II i iv V II i", "pad", 3, 0.2),
    ], lufs=-21, reverb=(1.6, 0.2), seed=23)
    M["night"] = score("Starlit Meadow", 80, "Ab", "major", 16, [
        *mel("5 . 6 | 5 . 3 | 1 . 3 | 2 . . | 3 . 4 | 5 . 8 | 7 . 6 | 5 . . | 5 . 6 | 5 . 3 | 1 . 3 | 6 . . | 5 . 4 | 3 . 2 | 4 . 3 | 1 . .", "bell", 5, 0.6),
        *mel("1 5 8 | 1 5 8 | 6 3 8 | 5 2 7 | 4 1 6 | 1 5 8 | 4 1 6 | 5 2 7 | 1 5 8 | 3 7 10 | 6 3 8 | 2 6 9 | 4 1 6 | 1 5 8 | 5 2 7 | 1 5 8", "harp", 3, 0.45, vel=(0.5, 0.8)),
        chords("I I vi V IV I IV V I iii vi ii IV I V I", "pad", 3, 0.32),
    ], beats=3, lufs=-23, reverb=(4.5, 0.4), seed=29)
    M["blossom"] = score("Blossom Ridge", 148, "D", "major", 16, [
        *mel("5 . 6 8 . 6 5 . | 3 . 5 . 2 . . . | 5 . 6 8 . 9 10 . | 9 . 8 . 6 . . . | 5 . 6 8 . 6 5 . | 3 . 5 . 6 . 8 . | 9 . 8 . 6 . 5 . | 8 . . . . . . . "
             "| 10 . 9 . 8 . 6 . | 8 . 6 . 5 . . . | 6 . 5 . 3 . 5 . | 2 . . . . . . . | 10 . 9 . 8 . 6 . | 8 . 9 . 10 . 12 . | 10 . 9 . 8 . 6 . | 8 . . . 5 . 8 .", "flute", 5, 0.72),
        *mel("1 . 1 5 . 5 1 . | 6 . 6 3 . 3 6 . | 4 . 4 1 . 1 4 . | 5 . 5 2 . 2 5 . " * 4, "bass", 2, 0.8),
        chords("I vi IV V I vi IV I vi IV I V vi IV V I", "pluck", 4, 0.25, strum=0.04, every="2 beats"),
        beat("kick", "x...x...", 0.5), beat("snare", "....x...", 0.4), beat("hat", "x.xxx.xx", 0.3, vel=(0.3, 0.8)),
    ], lufs=-20, reverb=(1.2, 0.14), seed=31)
    M["ash"] = score("Ashen Wind", 104, "C", "minor", 16, [
        *mel("1 . . 3 . . 5 . | 6 . . 5 . . 3 . | 4 . . 3 . . 2 . | 7 . . . . . . . | 1 . . 3 . . 5 . | 6 . . 8 . . 10 . | 9 . 8 . 6 . 5 . | 1 . . . . . . . "
             "| 6 . . 5 . . 4 . | 5 . . 4 . . 3 . | 4 . . 3 . . 2 . | 3 . . . 2 . . . | 6 . . 5 . . 8 . | 7 . . 6 . . 5 . | 4 . 3 . 2 . 7 . | 1 . . . . . . .", "fiddle", 4, 0.7),
        *mel("1 1 . 1 1 . 1 . | 6 6 . 6 6 . 6 . | 4 4 . 4 4 . 4 . | 5 5 . 5 5 . 7 . " * 4, "bass", 2, 0.85),
        chords("i VI iv VII i VI iv i VI III iv V VI iv V i", "strings", 3, 0.25),
        beat("tom", "x..x..x.", 0.5), beat("snare", "....x..x", 0.3),
    ], lufs=-20, reverb=(2.2, 0.24), seed=37)
    M["boss"] = score("The Lieutenant", 152, "E", "minor", 16, [
        *mel("1 . 1 2 3 . 1 . | 7 . . 5 . . 7 . | 1 . 1 2 3 . 5 . | 4 . . 3 . . 2 . | 6 . 6 5 4 . 3 . | 2 . 2 1 7 . . . | 1 . 3 . 5 . 8 . | 7 . . . . . . . "
             "| 8 . 7 . 8 . 10 . | 9 . . 8 7 . . . | 6 . 5 . 6 . 8 . | 7 . . . 5 . . . | 4 . 5 . 6 . 7 . | 8 . 7 . 6 . 5 . | 4 . 3 . 2 . 7 . | 1 . . . 5 . 7 .", "clar", 4, 0.8),
        *mel("1 1 1 1 1 1 1 1 | 6 6 6 6 6 6 6 6 | 1 1 1 1 1 1 1 1 | 4 4 4 4 5 5 5 5 | 6 6 6 6 6 6 6 6 | 7 7 7 7 5 5 5 5 | 1 1 1 1 1 1 1 1 | 7 7 7 7 5 5 7 7 "
             "| 6 6 6 6 6 6 6 6 | 4 4 4 4 4 4 4 4 | 6 6 6 6 6 6 6 6 | 5 5 5 5 5 5 5 5 | 4 4 4 4 4 4 4 4 | 6 6 6 6 6 6 6 6 | 4 4 4 4 5 5 5 5 | 1 1 1 1 5 5 7 7", "bass", 2, 0.8),
        chords("i VI i iv VI VII i V VI iv VI V iv VI iv i", "organ", 3, 0.2),
        beat("kick", "x.x.x.x.", 0.55), beat("snare", "....x...", 0.45), beat("hat", "xxxxxxxx", 0.25, vel=(0.3, 0.9)),
    ], lufs=-19, reverb=(1.2, 0.14), seed=41)
    M["baron"] = score("Baron Thornwick", 138, "C", "harmonic_minor", 16, [
        *mel("1 . . 7 1 . . 2 | 3 . . 2 1 . . . | 1 . . 7 1 . 3 . | 5 . . . . . . . | 6 . . 5 4 . . 3 | 2 . . 3 4 . . . | 5 . . 4 3 . . 2 | 7 . . . . . . . "
             "| 8 . . 7 8 . . 9 | 10 . . 9 8 . . . | 8 . . 7 6 . 5 . | 4 . . . . . . . | 6 . 5 . 4 . 3 . | 2 . 3 . 4 . 5 . | 6 . . 5 4 . 2 . | 7 . . . . . . .", "organ", 4, 0.55),
        *mel("1 . 1 . 1 . 1 . | 1 . 1 . 1 . 1 . | 6 . 6 . 6 . 6 . | 5 . 5 . 5 . 5 . " * 4, "bass", 1, 0.9),
        *mel("1 . . 7 1 . . 2 | 3 . . 2 1 . . . | 1 . . 7 1 . 3 . | 5 . . . . . . . | 6 . . 5 4 . . 3 | 2 . . 3 4 . . . | 5 . . 4 3 . . 2 | 7 . . . . . . . "
             "| 8 . . 7 8 . . 9 | 10 . . 9 8 . . . | 8 . . 7 6 . 5 . | 4 . . . . . . . | 6 . 5 . 4 . 3 . | 2 . 3 . 4 . 5 . | 6 . . 5 4 . 2 . | 7 . . . . . . .", "strings", 3, 0.35),
        beat("kick", "x..x..x.", 0.6), beat("snare", "....x...", 0.45), beat("tom", "......xx", 0.4), beat("hat", "x.x.x.x.", 0.25),
    ], lufs=-19, reverb=(2.4, 0.22), seed=43)
    M["star"] = score("Sunpetal Dash", 180, "C", "major", 4, [
        *mel("1 3 5 8 5 3 5 8 | 2 4 6 9 6 4 6 9 | 3 5 8 10 8 5 8 10 | 9 8 7 6 5 4 3 2", "marimba", 5, 0.75),
        *mel("1 . 1 . 1 . 1 . | 2 . 2 . 2 . 2 . | 3 . 3 . 3 . 3 . | 5 . 5 . 5 . 5 .", "bass", 2, 0.8),
        beat("kick", "x...x...", 0.5), beat("hat", "xxxxxxxx", 0.3, vel=(0.4, 1.0)), beat("snare", "..x...x.", 0.3),
    ], lufs=-19, reverb=(0.8, 0.1), seed=2)
    M["freed"] = score("A Friend Set Free", 92, "G", "major", 8, [
        *mel("1 . 3 . 5 . 6 5 | 4 . . . 3 . . . | 2 . 4 . 6 . 8 7 | 6 . . . 5 . . . | 1 . 3 . 5 . 6 8 | 9 . 8 . 6 . . . | 5 . 6 . 7 . 2 . | 1 . . . . . . .", "harp", 4, 0.7),
        chords("I IV ii V I IV V I", "strings", 3, 0.3),
        *mel("1 . . . | 4 . . . | 2 . . . | 5 . . . | 1 . . . | 4 . . . | 5 . . . | 1 . . .", "upright", 2, 0.6),
    ], lufs=-22, reverb=(2.6, 0.3), seed=47)
    M["ending"] = score("Home Before the Snow", 86, "G", "major", 16, [
        *mel("1 . 3 . 5 . 6 5 | 3 . . . . . . . | 4 . 6 . 8 . 9 8 | 6 . . . 5 . . . | 1 . 3 . 5 . 6 5 | 8 . . . 10 . 9 . | 8 . 6 . 5 . 6 . | 5 . . . . . . . "
             "| 8 . 9 . 10 . 12 10 | 8 . . . . . . . | 9 . 10 . 11 . 13 11 | 10 . . . 9 . . . | 8 . 7 . 6 . 5 . | 4 . 5 . 6 . 9 . | 8 . . . 7 . . . | 8 . . . . . . .", "flute", 5, 0.7),
        *mel("1 5 8 5 1 5 8 5 | 6 3 8 3 6 3 8 3 | 4 1 6 1 4 1 6 1 | 5 2 7 2 5 2 7 2 " * 4, "harp", 3, 0.4, vel=(0.5, 0.8)),
        chords("I vi IV V I iii IV V I vi IV V vi IV V I", "strings", 3, 0.3),
        *mel("1 . . . | 6 . . . | 4 . . . | 5 . . . " * 4, "upright", 2, 0.6),
    ], lufs=-21, reverb=(3.0, 0.32), seed=53)
    M["hush"] = score("Quiet", 60, "C", "major", 2, [chords("I", "pad", 3, 0.3, every="2 bars")], lufs=-36, reverb=(4, 0.4), seed=1)
    return M


def _jig():
    return score("Falling Leaves", 112, "G", "mixolydian", 32, [
        *mel("5 6 5 | 3 . 1 | 2 3 4 | 5 . . | 5 6 8 | 7 6 5 | 4 3 2 | 1 . . | 8 . 7 | 8 . 5 | 6 . 5 | 6 . 4 | 5 4 3 | 2 3 4 | 2 . 7 | 1 . . "
             "| 3 4 5 | 6 . 5 | 4 5 6 | 7 . 6 | 5 6 7 | 8 . 10 | 9 8 7 | 8 . . | 10 9 8 | 7 . 5 | 8 7 6 | 5 . 3 | 4 5 6 | 5 4 2 | 3 . 2 | 1 . .", "fiddle", 5, 0.7),
        *mel("1 . 5 | 1 . 5 | 7 . 4 | 1 . 5 | 1 . 5 | 7 . 4 | 4 . 1 | 1 . 5 | 1 . 5 | 1 . 5 | 4 . 1 | 4 . 1 | 1 . 5 | 7 . 4 | 5 . 2 | 1 . 5 "
             "| 6 . 3 | 4 . 1 | 4 . 1 | 5 . 2 | 1 . 5 | 1 . 5 | 5 . 2 | 1 . 5 | 6 . 3 | 5 . 2 | 4 . 1 | 1 . 5 | 4 . 1 | 7 . 4 | 5 . 2 | 1 . 5", "upright", 2, 0.8),
        chords("I I VII I I VII IV I I I IV IV I VII V I vi IV IV V I I V I vi V IV I IV VII V I", "pluck", 4, 0.22, every="1 bar", strum=0.05),
        beat("tom", "x..x..", 0.35), beat("shaker", "..x..x", 0.4),
    ], beats=2, lufs=-20, reverb=(1.4, 0.16), seed=17)


# ---- sound effects -----------------------------------------------------------------------------------------------------------------------------------
# Original cues, soft-edged (sines and gentle harmonics, no NES square): a jump is a little wing-whirr upwards, honey is a glassy drip, a stomp a squelchy boop.

N = {"C4": 261.63, "D4": 293.66, "E4": 329.63, "F4": 349.23, "G4": 392.0, "A4": 440.0, "B4": 493.88, "C5": 523.25, "D5": 587.33, "E5": 659.25, "F5": 698.46,
     "G5": 783.99, "A5": 880.0, "B5": 987.77, "C6": 1046.5, "D6": 1174.66, "E6": 1318.51, "F6": 1396.9, "G6": 1567.98, "A3": 220.0, "G3": 196.0, "E3": 164.81,
     "C3": 130.81, "D3": 146.83, "F3": 174.61, "Bb4": 466.16, "Eb5": 622.25, "Ab4": 415.3, "Bb3": 233.08}
GLASS = [[2, 0.2], [3, 0.06]]


def tone(hz, decay=12.0, delay=0.0, gain=1.0, harm=GLASS, attack=0.003):
    layer = {"sine": hz, "decay": decay, "attack": attack, "harmonics": harm}
    if delay:
        layer["delay"] = round(delay, 4)
    if gain != 1.0:
        layer["gain"] = gain
    return layer


def notes(seq, step, decay=10.0, harm=GLASS, gain=1.0):
    out, t = [], 0.0
    for hz, n in seq:
        if hz:
            out.append(tone(N[hz] if isinstance(hz, str) else hz, decay=decay, delay=t, gain=gain, harm=harm))
        t += n * step
    return out


def sounds():
    S = {
        "jump": {"seconds": 0.22, "level": 0.26, "layers": [{"glide": [380, 820, 11], "decay": 10, "attack": 0.004, "harmonics": GLASS}, {"noise": "hp 0.5", "decay": 30, "gain": 0.08}]},
        "jump_big": {"seconds": 0.26, "level": 0.26, "layers": [{"glide": [280, 640, 9], "decay": 9, "attack": 0.004, "harmonics": GLASS}, {"noise": "hp 0.4", "decay": 26, "gain": 0.08}]},
        "flutter": {"seconds": 0.3, "level": 0.16, "layers": [{"noise": "lp 0.35", "attack": 0.03, "decay": 10, "gain": 0.6}, {"sine": 180, "decay": 12, "attack": 0.02, "harmonics": [[2, 0.5], [3, 0.3]], "gain": 0.5}]},
        "honey": {"seconds": 0.5, "level": 0.24, "layers": [tone(N["A5"], 22), tone(N["E6"], 9, delay=0.06), tone(N["A5"] * 3, 30, delay=0.06, gain=0.2)]},
        "stomp": {"seconds": 0.22, "level": 0.45, "layers": [{"glide": [420, 150, 16], "decay": 15, "attack": 0.002, "harmonics": GLASS}, {"noise": "lp 0.25", "decay": 28, "gain": 0.4}]},
        "bump": {"seconds": 0.14, "level": 0.42, "layers": [{"glide": [160, 100, 14], "decay": 22, "attack": 0.002}, {"noise": "lp 0.2", "decay": 40, "gain": 0.35}]},
        "break": {"seconds": 0.45, "level": 0.4, "layers": [{"noise": "lp 0.5", "decay": 10, "attack": 0.002, "gain": 0.6}, tone(N["E5"], 18, gain=0.3), tone(N["B5"], 22, delay=0.03, gain=0.2)]},
        "powerup": {"seconds": 0.9, "level": 0.24, "layers": notes([("C5", 1), ("G5", 1), ("E5", 1), ("C6", 1), ("G5", 1), ("E6", 2)], 0.07, decay=9, harm=GLASS) + [{"noise": "hp 0.6", "fade_in": 4, "decay": 6, "gain": 0.05}]},
        "sprout": {"seconds": 0.55, "level": 0.26, "layers": [{"sweep": [260, 780, 3], "decay": 4, "attack": 0.01, "harmonics": GLASS}, {"noise": "lp 0.4", "decay": 9, "gain": 0.1}]},
        "shrink": {"seconds": 0.6, "level": 0.25, "layers": notes([("E6", 1), ("B5", 1), ("G5", 1), ("E5", 1), ("B4", 2)], 0.07, decay=10)},
        "seed": {"seconds": 0.18, "level": 0.25, "layers": [{"noise": "lp 0.5", "decay": 18, "attack": 0.004, "gain": 0.6}, {"glide": [700, 380, 14], "decay": 16, "harmonics": GLASS, "gain": 0.4}]},
        "kick": {"seconds": 0.12, "level": 0.4, "layers": [{"glide": [600, 260, 26], "decay": 26, "attack": 0.001, "harmonics": GLASS}]},
        "pipe": {"seconds": 0.7, "level": 0.28, "layers": [{"sweep": [520, 160, 2.5], "decay": 3, "attack": 0.01, "harmonics": GLASS}, {"noise": "lp 0.25", "decay": 5, "gain": 0.2}]},
        "oneup": {"seconds": 1.0, "level": 0.24, "layers": notes([("G5", 1), ("D6", 1), ("B5", 1), ("G6", 3)], 0.09, decay=6, harm=[[2, 0.3], [4, 0.1]])},
        "lose": {"seconds": 2.2, "level": 0.26, "layers": notes([("E5", 1), ("D5", 1), ("B4", 1), ("A4", 1.5), ("G4", 1.5), ("E4", 4)], 0.16, decay=3.6, harm=[[2, 0.2]])},
        "bloom": {"seconds": 2.6, "level": 0.26, "layers": notes([("C5", 1), ("E5", 1), ("G5", 1), ("C6", 1), ("D6", 1), ("E6", 1), ("G6", 4)], 0.1, decay=3.2, harm=[[2, 0.25], [3, 0.1]])
                  + [{"noise": "hp 0.7", "fade_in": 3, "decay": 2.5, "gain": 0.06}, tone(N["C4"], 1.6, gain=0.5, harm=[[2, 0.3]]), tone(N["G4"], 1.6, delay=0.3, gain=0.4)]},
        "gameover": {"seconds": 3.0, "level": 0.24, "layers": notes([("A4", 2), ("F4", 2), ("D4", 2), ("E4", 2), ("C4", 6)], 0.16, decay=2.4, harm=[[2, 0.2]])},
        "tick": {"seconds": 0.05, "level": 0.15, "layers": [tone(N["C6"], 60)]},
        "pause": {"seconds": 0.3, "level": 0.22, "layers": notes([("G5", 1), ("D5", 2)], 0.06, decay=12)},
        "unpause": {"seconds": 0.3, "level": 0.22, "layers": notes([("D5", 1), ("G5", 2)], 0.06, decay=12)},
        "menu": {"seconds": 0.08, "level": 0.18, "layers": [tone(N["E6"], 50)]},
        "select": {"seconds": 0.25, "level": 0.22, "layers": notes([("C6", 1), ("G6", 2)], 0.05, decay=16)},
        "saved": {"seconds": 0.7, "level": 0.22, "layers": notes([("C6", 1), ("E6", 1), ("G6", 1), ("C6", 3)], 0.07, decay=8)},
        "talk": {"seconds": 0.12, "level": 0.18, "layers": [{"glide": [600, 760, 20], "decay": 22, "attack": 0.004, "harmonics": GLASS}]},
        "start": {"seconds": 1.1, "level": 0.24, "layers": notes([("F4", 1), ("A4", 1), ("C5", 1), ("D5", 1), ("C5", 3)], 0.11, decay=6, harm=[[2, 0.3]])},
        "gate": {"seconds": 0.8, "level": 0.5, "layers": [{"noise": "lp 0.15", "decay": 5, "attack": 0.003, "gain": 0.9}, {"glide": [90, 50, 4], "decay": 5, "gain": 0.6}]},
        "boss_hit": {"seconds": 0.5, "level": 0.5, "layers": [{"glide": [300, 90, 8], "decay": 8, "attack": 0.002, "harmonics": [[2, 0.4], [3, 0.2]]}, {"noise": "lp 0.4", "decay": 14, "gain": 0.45}]},
        "boss_roar": {"seconds": 1.2, "level": 0.4, "layers": [{"glide": [110, 70, 2], "decay": 2.4, "attack": 0.06, "harmonics": [[2, 0.5], [3, 0.4], [5, 0.2]]}, {"noise": "lp 0.3", "attack": 0.05, "decay": 3, "gain": 0.4}]},
        "bossfall": {"seconds": 2.0, "level": 0.45, "layers": [{"sweep": [380, 60, 1.2], "decay": 1.4, "attack": 0.01, "harmonics": [[2, 0.4], [3, 0.2]]}, {"noise": "lp 0.3", "decay": 1.8, "gain": 0.4}]},
        "thud": {"seconds": 0.5, "level": 0.55, "layers": [{"glide": [120, 40, 8], "decay": 8, "attack": 0.002}, {"noise": "lp 0.15", "decay": 10, "gain": 0.6}]},
        "whoosh": {"seconds": 0.45, "level": 0.25, "layers": [{"noise": "lp 0.2..0.6", "attack": 0.12, "release": 0.2, "gain": 0.8}]},
        "charge": {"seconds": 0.7, "level": 0.25, "layers": [{"sweep": [200, 900, 2], "attack": 0.3, "release": 0.1, "harmonics": GLASS}]},
        "zap": {"seconds": 0.2, "level": 0.25, "layers": [{"glide": [1400, 500, 18], "decay": 16, "attack": 0.002, "harmonics": GLASS}]},
        "shard": {"seconds": 2.4, "level": 0.28, "layers": notes([("G5", 1), ("B5", 1), ("D6", 1), ("G6", 1), ("B5", 1), ("D6", 1), ("G6", 4)], 0.08, decay=3, harm=[[2, 0.3], [3, 0.15]])
                  + [{"noise": "hp 0.75", "fade_in": 2, "decay": 2, "gain": 0.08}]},
        "pop": {"seconds": 0.3, "level": 0.3, "layers": [{"glide": [300, 900, 18], "decay": 14, "attack": 0.002, "harmonics": GLASS}, {"noise": "hp 0.4", "decay": 30, "gain": 0.2}]},
    }
    for v in S.values():
        for layer in v["layers"]:
            if "sine" not in layer:
                layer.pop("harmonics", None)
            # low-passed noise drifts off centre over a short sound (`verify` measures a DC offset and a click at the ends): a very gentle
            # high-pass keeps the rumble and loses the drift
            if str(layer.get("noise", "")).startswith("lp"):
                layer["noise"] = "hp 0.03"
    return S
