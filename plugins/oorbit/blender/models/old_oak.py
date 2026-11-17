"""An ancient oak, far bigger than the rest: a little round door between its
roots, and a swing on its lowest branch (game: the swing sways)."""
from lib import blob, box, cylinder, jitter


def build():
    jitter(cylinder("trunk", "5b4a42", 0.35, 1.3, 7, r_top=0.22), 0.04, 31)
    for k, (a, tilt) in enumerate([(0.3, 0.9), (2.4, 1.0), (4.2, 0.8)]):
        cylinder("root%d" % k, "5b4a42", 0.12, 0.45, 4, loc=(0, 0, 0.0), rot=(tilt + 0.6, 0, a), r_top=0.04)
    cylinder("bough", "5b4a42", 0.12, 0.9, 5, loc=(0.1, 0, 1.0), rot=(0, 1.2, 0), r_top=0.07)
    for k, (x, y, z, r) in enumerate([(0, 0, 1.75, 0.95), (0.7, 0.2, 1.5, 0.6), (-0.6, -0.3, 1.6, 0.6), (0.1, 0.4, 2.2, 0.55)]):
        blob("crown%d" % k, "tint1", r, loc=(x, y, z), squash=0.8, subdiv=1, seed=40 + k, rough=0.16)
    box("door", "a35d5d", (0.22, 0.05, 0.3), loc=(0, -0.34, 0))
    box("detail_knob", "f2c078", (0.03, 0.03, 0.03), loc=(0.06, -0.37, 0.13))
    box("detail_window", "glow_f2e1a0", (0.1, 0.05, 0.1), loc=(0.0, -0.3, 0.6))
    box("detail_rope1", "d9c8b0", (0.015, 0.015, 0.6), loc=(0.62, -0.12, 0.38))
    box("detail_rope2", "d9c8b0", (0.015, 0.015, 0.6), loc=(0.62, 0.12, 0.38))
    box("detail_seat", "8f6f5a", (0.12, 0.3, 0.03), loc=(0.62, 0, 0.36))
    blob("far", "tint1", 1.0, loc=(0, 0, 1.75), squash=0.8, subdiv=0, seed=40)
    cylinder("far_trunk", "5b4a42", 0.3, 1.3, 5, cap=False)
