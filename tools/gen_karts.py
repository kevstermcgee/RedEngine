#!/usr/bin/env python3
"""Generates assets/karts.json: the eight Great Outdoors drivers, each an animal in its own kart, as game-local Red prefabs.

Every kart faces -Z (yaw 0 looks along -Z in Red) and sits on the ground at y = 0. Primitives only (box, sphere, cylinder, cone, capsule) in a pastel
palette. Run from the repository root:   python3 tools/gen_karts.py
Then look at them:   red_engine2 catalog --library assets/karts.json --sheet out/karts.png karts
"""
import json, sys

def mat(color, roughness=0.75, metal=0.0, emissive=None):
    m = {"color": color, "roughness": roughness}
    if metal: m["metallic"] = metal
    if emissive: m["emissive"] = emissive
    return m

class Kart:
    def __init__(self): self.objs = []; self.n = 0
    def _id(self, kind): self.n += 1; return f"{kind}_{self.n}"
    def box(self, size, pos, color, rot=None, **kw):
        o = {"id": self._id("b"), "type": "box", "size": size, "position": pos, "material": mat(color, **kw)}
        if rot: o["rotation"] = rot
        self.objs.append(o); return self
    def sphere(self, r, pos, color, **kw):
        self.objs.append({"id": self._id("s"), "type": "sphere", "radius": r, "position": pos, "material": mat(color, **kw)}); return self
    def cyl(self, r, h, pos, color, rot=None, **kw):
        o = {"id": self._id("c"), "type": "cylinder", "radius": r, "height": h, "position": pos, "material": mat(color, **kw)}
        if rot: o["rotation"] = rot
        self.objs.append(o); return self
    def cone(self, r, h, pos, color, rot=None, **kw):
        o = {"id": self._id("k"), "type": "cone", "radius": r, "height": h, "position": pos, "material": mat(color, **kw)}
        if rot: o["rotation"] = rot
        self.objs.append(o); return self
    def cap(self, r, h, pos, color, rot=None, **kw):
        o = {"id": self._id("p"), "type": "capsule", "radius": r, "height": h, "position": pos, "material": mat(color, **kw)}
        if rot: o["rotation"] = rot
        self.objs.append(o); return self

DARK = "#5b5470"; CREAM = "#fff4e6"; BLACK = "#3a3348"

def scaled(k, f):
    """Scales a finished kart uniformly by f about its ground point (bigger animals): positions, box sizes, radii and heights."""
    for o in k.objs:
        o["position"] = [round(v * f, 4) for v in o["position"]]
        if "size" in o: o["size"] = [round(v * f, 4) for v in o["size"]]
        for key in ("radius", "height"):
            if key in o: o[key] = round(o[key] * f, 4)
    return k

def chassis(k, body, accent, wheel=DARK, hub="#f5f0ff", width=1.2, wheel_r=0.3, wheel_w=0.24):
    hw = width / 2
    k.box([width - 0.2, 0.26, 1.8], [0, 0.34, 0.0], body)                    # tub
    k.box([width - 0.5, 0.22, 0.7], [0, 0.36, -1.05], accent)                  # nose
    k.box([width, 0.32, 0.4], [0, 0.46, 0.88], accent)                         # tail
    k.box([0.14, 0.24, 1.3], [hw - 0.02, 0.44, 0.05], accent)                  # side pods
    k.box([0.14, 0.24, 1.3], [-hw + 0.02, 0.44, 0.05], accent)
    k.box([0.62, 0.3, 0.5], [0, 0.6, 0.28], body)                              # seat back
    k.cyl(0.13, 0.05, [0, 0.9, -0.38], BLACK, rot=[65, 0, 0])                  # steering wheel
    k.cyl(0.03, 0.34, [0, 0.72, -0.36], BLACK, rot=[65, 0, 0])
    for sx in (-1, 1):
        for z in (-0.68, 0.68):
            x = sx * (hw + 0.02)
            k.cyl(wheel_r, wheel_w, [x, wheel_r, z], wheel, rot=[0, 0, 90], roughness=0.95)
            k.cyl(wheel_r * 0.45, wheel_w + 0.03, [x, wheel_r, z], hub, rot=[0, 0, 90])

def eyes(k, y, z, x=0.13, r=0.05, color=BLACK):
    for sx in (-1, 1):
        k.sphere(r, [sx * x, y, z], color)
        k.sphere(r * 0.35, [sx * x - sx * 0.012, y + 0.02, z - r * 0.8], "#ffffff")

def torso(k, color, belly=None, y=0.98, r=0.3):
    k.sphere(r, [0, y, 0.22], color)
    if belly: k.sphere(r * 0.7, [0, y - 0.02, 0.22 - r * 0.55], belly)
    for sx in (-1, 1):                                                         # arms on the wheel
        k.cap(0.07, 0.42, [sx * 0.26, y + 0.02, -0.05], color, rot=[70, 0, sx * 12])

def duck():
    k = Kart(); chassis(k, "#ffe9a0", "#ffd166"); torso(k, "#ffe27a", "#fff6cf")
    k.sphere(0.3, [0, 1.42, 0.1], "#ffe27a"); eyes(k, 1.48, -0.13)
    k.box([0.26, 0.07, 0.3], [0, 1.36, -0.27], "#ffb35c"); k.box([0.22, 0.05, 0.22], [0, 1.31, -0.25], "#ff9f45")   # bill
    k.sphere(0.06, [0, 1.42, 0.38], "#fff2b8")                                                                       # tuft
    for sx in (-1, 1): k.box([0.05, 0.22, 0.34], [sx * 0.33, 1.0, 0.3], "#ffd84d", rot=[0, 0, sx * 18])           # wings
    k.cone(0.16, 0.3, [0, 0.72, 1.05], "#ffe27a", rot=[-60, 0, 0])                                                   # duck tail on the kart
    return k

def bunny():
    k = Kart(); chassis(k, "#c7f0dc", "#ffb59a"); torso(k, "#fdeef4", "#ffffff")
    k.sphere(0.3, [0, 1.42, 0.1], "#fdeef4"); eyes(k, 1.48, -0.14, r=0.055)
    k.sphere(0.055, [0, 1.4, -0.29], "#ffa6c1")                                                                      # nose
    for sx in (-1, 1):
        k.cap(0.09, 0.68, [sx * 0.13, 1.95, 0.1], "#fdeef4", rot=[0, 0, sx * 8])
        k.cap(0.045, 0.5, [sx * 0.13, 1.95, 0.04], "#ffc2d6", rot=[0, 0, sx * 8])
    k.sphere(0.15, [0, 0.9, 0.8], "#ffffff")                                                                         # cotton tail
    k.cone(0.06, 0.22, [0.25, 0.62, -1.05], "#ff9b54", rot=[-90, 0, 0])                                              # carrot on the nose
    k.cone(0.03, 0.08, [0.25, 0.62, -1.2], "#8fd694", rot=[-90, 0, 0])
    return k

def deer():
    k = Kart(); chassis(k, "#c9edd0", "#f6d6a8"); torso(k, "#e8c39e", "#fff1de")
    k.sphere(0.28, [0, 1.42, 0.1], "#e8c39e"); k.box([0.2, 0.18, 0.28], [0, 1.34, -0.24], "#f0d2b0"); k.sphere(0.05, [0, 1.36, -0.39], BLACK)
    eyes(k, 1.5, -0.12)
    for sx in (-1, 1):
        k.cone(0.09, 0.28, [sx * 0.3, 1.5, 0.12], "#e8c39e", rot=[0, 0, sx * -70])                                    # ears
        k.cyl(0.028, 0.5, [sx * 0.15, 1.9, 0.1], "#b58863", rot=[0, 0, sx * 12])                                       # antlers
        k.cyl(0.022, 0.26, [sx * 0.24, 2.0, 0.1], "#b58863", rot=[0, 0, sx * 50])
        k.cyl(0.022, 0.22, [sx * 0.21, 1.82, 0.03], "#b58863", rot=[35, 0, sx * 30])
    for i, (x, y, z) in enumerate([(0.12, 1.02, 0.28), (-0.14, 0.94, 0.3), (0.05, 1.12, 0.36)]): k.sphere(0.035, [x, y, z], "#fff8ee")  # spots
    k.cone(0.13, 0.32, [0, 0.62, 1.0], "#a8d5ba", rot=[-70, 0, 0])
    return k

def coyote():
    k = Kart(); chassis(k, "#f4c7b0", "#e9a98a", wheel="#6e5a55", wheel_r=0.36, wheel_w=0.3); torso(k, "#d9b38c", "#f1dcc0")
    k.sphere(0.28, [0, 1.42, 0.1], "#d9b38c"); k.box([0.2, 0.17, 0.4], [0, 1.34, -0.34], "#e6c8a4"); k.sphere(0.055, [0, 1.38, -0.55], BLACK)
    eyes(k, 1.5, -0.12, color="#5b3d1e")
    for sx in (-1, 1): k.cone(0.1, 0.34, [sx * 0.15, 1.78, 0.1], "#c49a6c", rot=[0, 0, sx * 10]); k.cone(0.05, 0.2, [sx * 0.15, 1.75, 0.04], "#f1c7b0")
    k.cap(0.14, 0.62, [0, 0.9, 0.98], "#c9a37a", rot=[-70, 0, 0]); k.sphere(0.1, [0, 0.62, 1.32], "#f5e9d8")
    return k

def hawk():
    k = Kart(); chassis(k, "#bfe3ff", "#9cc7f0"); torso(k, "#b08968", "#f3e9dc")
    k.sphere(0.28, [0, 1.42, 0.1], "#f3e9dc"); eyes(k, 1.5, -0.14, r=0.06, color="#7a4d1d")
    k.cone(0.09, 0.3, [0, 1.36, -0.34], "#ffd166", rot=[-90, 0, 0])                                                   # hooked beak
    k.cone(0.05, 0.14, [0, 1.3, -0.5], "#f0b429", rot=[-120, 0, 0])
    k.box([0.34, 0.05, 0.12], [0, 1.66, 0.0], "#a1795a")                                                             # brow
    for sx in (-1, 1):
        k.box([0.09, 0.56, 0.66], [sx * 0.36, 1.0, 0.34], "#a1795a", rot=[0, 0, sx * -10])                              # folded wings
        k.box([0.08, 0.4, 0.5], [sx * 0.4, 0.82, 0.56], "#8d6e63", rot=[8, 0, sx * -12])
        k.box([0.05, 0.4, 0.4], [sx * 0.62, 0.75, 0.95], "#7fb2e5")                                                     # kart fins
    for i, a in enumerate((-24, 0, 24)):                                                                                # tail feather fan
        k.box([0.1, 0.05, 0.5], [a * 0.011, 0.86, 1.18], "#8d6e63" if i == 1 else "#a1795a", rot=[-14, a, 0])
    k.box([0.5, 0.05, 0.26], [0, 1.02, 1.24], "#9cc7f0")                                                                # rear wing
    return k

def bear():
    # The biggest animal: cocoa-coloured fur (darker and cooler than the Beaver's), a cream-and-coral kart, a picnic basket on the back and a honey pot.
    FUR = "#8c6b5e"; MUZZLE = "#d9bfae"
    k = Kart(); chassis(k, "#ffe3d6", "#ff9f9a", width=1.36, wheel_r=0.34, wheel_w=0.32); torso(k, FUR, "#c9a898", r=0.38)
    k.sphere(0.36, [0, 1.52, 0.1], FUR); k.sphere(0.16, [0, 1.43, -0.28], MUZZLE); k.sphere(0.06, [0, 1.47, -0.42], BLACK)
    eyes(k, 1.6, -0.18, x=0.17, r=0.05)
    for sx in (-1, 1): k.sphere(0.13, [sx * 0.28, 1.86, 0.1], FUR); k.sphere(0.07, [sx * 0.28, 1.86, 0.0], MUZZLE)
    k.sphere(0.16, [0, 0.98, 0.62], FUR)                                                                                  # a round stub of a tail
    k.box([1.3, 0.16, 0.22], [0, 0.34, -1.25], "#ff9f9a")                                                                # bumper
    # picnic basket on the back: a red-and-white checked box with a lid and handle (from behind this says BEAR at once)
    k.box([0.62, 0.36, 0.44], [0, 0.78, 0.98], "#ff8f8f"); k.box([0.64, 0.05, 0.46], [0, 0.98, 0.98], "#ffffff")
    for i in (-1, 0, 1): k.box([0.1, 0.37, 0.45], [i * 0.2, 0.78, 0.98], "#ffffff")
    k.cyl(0.03, 0.4, [0, 1.06, 0.98], "#c9a173", rot=[0, 0, 90])
    k.cyl(0.12, 0.14, [0.46, 0.78, 0.6], "#e8a86a"); k.sphere(0.1, [0.46, 0.9, 0.6], "#ffd166")                        # honey pot
    return scaled(k, 1.22)

def wolf():
    k = Kart(); chassis(k, "#e3d3f0", "#cdb4db", hub="#e8ecf5"); torso(k, "#b8c0d8", "#eef1f8")
    k.sphere(0.28, [0, 1.42, 0.1], "#b8c0d8"); k.box([0.2, 0.17, 0.42], [0, 1.34, -0.35], "#d3d8ea"); k.sphere(0.055, [0, 1.38, -0.57], BLACK)
    eyes(k, 1.5, -0.12, color="#e0a800")
    for sx in (-1, 1): k.cone(0.1, 0.36, [sx * 0.16, 1.8, 0.1], "#8e97b5", rot=[0, 0, sx * 8]); k.cone(0.05, 0.2, [sx * 0.16, 1.77, 0.04], "#f0d0d8")
    k.cap(0.13, 0.7, [0, 0.92, 1.0], "#a4adc9", rot=[-72, 0, 0]); k.sphere(0.1, [0, 0.6, 1.36], "#eef1f8")
    k.cone(0.07, 0.22, [0, 1.1, 0.5], "#c8cfe6", rot=[-30, 0, 0])
    return k

def beaver():
    k = Kart()
    # A kart made of wood: two logs for the body, planks on top, log-slice wheels.
    LOG = "#b8895a"; PLANK = "#d2a679"; PLANK2 = "#a97c50"; END = "#e6c9a0"
    for sx in (-1, 1):
        k.cyl(0.26, 1.9, [sx * 0.34, 0.36, 0.0], LOG, rot=[90, 0, 0], roughness=0.95)
        k.cyl(0.2, 0.03, [sx * 0.34, 0.36, -0.96], END, rot=[90, 0, 0]); k.cyl(0.2, 0.03, [sx * 0.34, 0.36, 0.96], END, rot=[90, 0, 0])
    for i, z in enumerate((-0.6, -0.1, 0.4, 0.85)): k.box([1.1, 0.06, 0.36], [0, 0.62, z], PLANK if i % 2 == 0 else PLANK2)
    k.box([0.7, 0.42, 0.14], [0, 0.85, 0.72], PLANK2)                                                                 # seat back
    k.box([0.08, 0.5, 0.08], [0.32, 0.8, -0.62], PLANK2); k.box([0.5, 0.05, 0.05], [0, 1.02, -0.62], PLANK)          # windscreen frame
    k.cyl(0.13, 0.05, [0, 0.95, -0.34], "#8a6a45", rot=[65, 0, 0])
    for sx in (-1, 1):
        for z in (-0.68, 0.68):
            k.cyl(0.32, 0.26, [sx * 0.7, 0.32, z], "#a67c52", rot=[0, 0, 90], roughness=0.95)
            k.cyl(0.24, 0.29, [sx * 0.7, 0.32, z], "#c79a6b", rot=[0, 0, 90]); k.cyl(0.07, 0.32, [sx * 0.7, 0.32, z], "#6f4e37", rot=[0, 0, 90])
    k.sphere(0.3, [0, 0.98, 0.3], "#b08560")
    for sx in (-1, 1): k.cap(0.07, 0.4, [sx * 0.25, 1.0, -0.05], "#b08560", rot=[70, 0, sx * 12])
    k.sphere(0.29, [0, 1.42, 0.14], "#b08560"); k.sphere(0.17, [0, 1.34, -0.2], "#c9a37a"); k.sphere(0.055, [0, 1.4, -0.34], BLACK)
    eyes(k, 1.5, -0.12)
    for sx in (-1, 1): k.box([0.055, 0.1, 0.03], [sx * 0.035, 1.24, -0.34], "#ffffff"); k.sphere(0.09, [sx * 0.24, 1.7, 0.14], "#9c7350")
    k.box([0.55, 0.06, 0.85], [0, 0.92, 1.15], "#5a4331", rot=[-15, 0, 0])                                            # the flat tail
    return k

DRIVERS = [("duck", duck, "Duck", "a round yellow duck with an orange bill in a rubber-duck kart"),
           ("bunny", bunny, "Bunny", "a pink-white bunny with tall ears and a carrot on the nose of a mint kart"),
           ("deer", deer, "Deer", "a fawn deer with antlers in a leaf-green kart"),
           ("coyote", coyote, "Coyote", "a sandy coyote with a bushy tail in a terracotta kart on big dirt tyres"),
           ("hawk", hawk, "Hawk", "a brown hawk with spread wings in a sky-blue kart with fins"),
           ("bear", bear, "Bear", "a big brown bear in a wide honey-coloured kart with a bumper"),
           ("wolf", wolf, "Wolf", "a grey-blue wolf with pointed ears in a lavender kart"),
           ("beaver", beaver, "Beaver", "a beaver in a kart made of logs and planks, with a flat tail")]

out = []
for slug, build, name, desc in DRIVERS:
    k = build()
    out.append({"name": f"kart_{slug}", "tags": ["vehicle", "character", "great-outdoors", "animal"], "desc": f"{name}: {desc}. Faces -Z, sits on y = 0.", "collide": False,
                "meta": {"status": "experimental", "origin": "generated", "revision": 1, "license": "MIT", "provenance": "tools/gen_karts.py"}, "objects": k.objs})
json.dump(out, open(sys.argv[1] if len(sys.argv) > 1 else "assets/karts.json", "w"), indent=1)
print(f"wrote {len(out)} kart prefabs, {sum(len(p['objects']) for p in out)} objects")
