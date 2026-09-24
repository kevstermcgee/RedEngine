"""Draws the Test Lab shortcut icon (Cheddar the rat on a red tile) -> scripts/test_lab.ico. Needs Pillow."""
import pathlib
from PIL import Image, ImageDraw

S = 1024  # draw big, shrink for smooth edges
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# tile: red rounded square with a lighter inner glow band and a dark rim
d.rounded_rectangle((20, 20, S - 20, S - 20), radius=210, fill=(120, 20, 28, 255))
d.rounded_rectangle((44, 44, S - 44, S - 44), radius=190, fill=(214, 48, 49, 255))
d.rounded_rectangle((70, 60, S - 70, S // 2), radius=170, fill=(232, 84, 76, 255))
d.rounded_rectangle((44, S // 2 - 30, S - 44, S - 44), radius=190, fill=(214, 48, 49, 255))

fur, fur_dark, pink = (176, 148, 122), (120, 96, 78), (244, 160, 176)
cx, cy = S // 2, 560

# ears (big round rat ears, pink inside)
for ex in (cx - 270, cx + 270):
    d.ellipse((ex - 190, 170, ex + 190, 550), fill=fur_dark)
    d.ellipse((ex - 150, 210, ex + 150, 510), fill=pink)
# head
d.ellipse((cx - 330, cy - 290, cx + 330, cy + 320), fill=fur_dark)
d.ellipse((cx - 315, cy - 290, cx + 315, cy + 300), fill=fur)
# muzzle
d.ellipse((cx - 175, cy + 20, cx + 175, cy + 300), fill=(226, 204, 182))
# eyes with highlights
for ex in (cx - 155, cx + 155):
    d.ellipse((ex - 62, cy - 120, ex + 62, cy + 30), fill=(24, 20, 22))
    d.ellipse((ex - 30, cy - 100, ex + 6, cy - 62), fill=(255, 255, 255))
# nose and grin
d.ellipse((cx - 52, cy + 55, cx + 52, cy + 130), fill=pink)
d.line((cx, cy + 130, cx, cy + 175), fill=(90, 60, 60), width=14)
d.arc((cx - 110, cy + 110, cx, cy + 230), 20, 160, fill=(90, 60, 60), width=14)
d.arc((cx, cy + 110, cx + 110, cy + 230), 20, 160, fill=(90, 60, 60), width=14)
# two buck teeth
for tx in (cx - 46, cx + 4):
    d.rounded_rectangle((tx, cy + 190, tx + 42, cy + 262), radius=10, fill=(255, 252, 240), outline=(120, 100, 90), width=6)
# whiskers
for sign in (-1, 1):
    for dy, dx in ((-10, 250), (40, 270), (90, 240)):
        d.line((cx + sign * 130, cy + 110 + dy // 2, cx + sign * (130 + dx), cy + 110 + dy * 2 - 30), fill=(250, 245, 235), width=10)

out = pathlib.Path(__file__).with_name("test_lab.ico")
img.resize((256, 256), Image.LANCZOS).save(out, sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)])
img.resize((512, 512), Image.LANCZOS).save(out.with_suffix(".png"))
print("wrote", out)
