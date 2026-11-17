"""A temple: a stepped base, two rows of columns, an entablature, a red roof."""
from lib import box, cylinder, prism


def build():
    box("base", "857c8a", (2.0, 1.5, 0.3))
    box("step", "d9d0cc", (1.6, 1.2, 0.12), loc=(0, 0, 0.3))
    for k in range(5):
        for row in (-1, 1):
            cylinder("column%d%d" % (k, row), "eae2d6", 0.09, 0.9, 6, loc=(-0.7 + k * 0.35, row * 0.5, 0.42))
    box("entablature", "d9d0cc", (1.8, 1.3, 0.15), loc=(0, 0, 1.32))
    prism("roof", "c07a54", 1.9, 0.4, 1.4, loc=(0, 0, 1.47))
    box("detail_statue", "glow_f2c078", (0.16, 0.16, 0.4), loc=(0, 0, 0.42))
