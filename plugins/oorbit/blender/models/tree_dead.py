"""A bare, dead tree with crooked branches (dry land, burnt woods)."""
from lib import cylinder, jitter


def build():
    jitter(cylinder("trunk", "6b5a4e", 0.08, 0.9, 5, r_top=0.03, cap=False), 0.03, 5)
    for k, (z, a, tilt) in enumerate([(0.45, 0.0, 0.9), (0.6, 2.4, 1.0), (0.75, 4.1, 0.7)]):
        cylinder("branch%d" % k, "5b4a42", 0.035, 0.4, 4, loc=(0, 0, z), rot=(tilt, 0, a), r_top=0.01, cap=False)
    cylinder("far", "6b5a4e", 0.07, 0.9, 3, r_top=0.02, cap=False)
