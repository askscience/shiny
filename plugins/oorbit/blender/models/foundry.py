"""A bronze foundry: a stone shed, a tall furnace stack glowing at its mouth."""
from lib import box, cylinder, prism


def build():
    box("shed", "857c8a", (1.4, 1.1, 0.8))
    prism("roof", "5b5466", 1.5, 0.4, 1.2, loc=(0, 0, 0.8), rot=(0, 0, 1.5708))
    cylinder("stack", "5b5466", 0.24, 1.4, 6, loc=(0.4, 0.2, 0.0), r_top=0.18)
    cylinder("detail_stack_rim", "4a4a4f", 0.21, 0.08, 6, loc=(0.4, 0.2, 1.4))
    box("furnace_mouth", "e08f6c", (0.4, 0.05, 0.3), loc=(-0.3, -0.56, 0.1))
    box("detail_glow", "f2c078", (0.25, 0.05, 0.12), loc=(-0.3, -0.57, 0.12))
    box("detail_anvil", "4a4a4f", (0.2, 0.12, 0.14), loc=(0.5, -0.75, 0))
    for k in range(3):
        box("detail_ingot%d" % k, "d4a24c", (0.14, 0.07, 0.05), loc=(-0.75, -0.7 + k * 0.1, 0))
