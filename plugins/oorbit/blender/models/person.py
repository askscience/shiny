"""A person: legs and arms that swing while walking (anim_*), a tunic the
game colours per person (tint1), a head with hair. Front -Y (game +Z); the
origin is the hips, as the game places people."""
import math

from lib import blob, box, cylinder


def build():
    for side in (-1, 1):
        leg = box("anim_x_%s70" % ("p" if side > 0 else "m"), "6b5a4e", (0.1, 0.11, 0.19),
                  loc=(side * 0.055, 0, -0.08), rot=(math.pi, 0, 0))
        arm = box("anim_x_%s55" % ("m" if side > 0 else "p"), "e8c4a0", (0.065, 0.07, 0.2),
                  loc=(side * 0.165, 0, 0.17), rot=(math.pi, 0, 0))
    cylinder("tunic", "tint1", 0.15, 0.3, 6, loc=(0, 0, -0.1), r_top=0.11)
    box("belt", "6b5a4e", (0.25, 0.25, 0.035), loc=(0, 0, 0.0), rot=(0, 0, 0.0))
    blob("head", "e8c4a0", 0.105, loc=(0, 0, 0.29), subdiv=1, seed=3, rough=0.05)
    box("detail_hair", "4a3b33", (0.2, 0.2, 0.09), loc=(0, 0.02, 0.33))
    box("detail_eyes", "15132a", (0.12, 0.02, 0.025), loc=(0, -0.1, 0.29))
