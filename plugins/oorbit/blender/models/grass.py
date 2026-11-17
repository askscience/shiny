"""A tuft of grass blades: only seen close up."""
import math

import bmesh

from lib import _bm_object, _finish


def build():
    for k in range(5):
        a = k * math.tau / 5 + 0.4
        bm = bmesh.new()
        p = bm.verts.new((-0.03, 0, 0))
        q = bm.verts.new((0.03, 0, 0))
        r = bm.verts.new((0, 0.02, 0))
        t = bm.verts.new((math.cos(a) * 0.08, math.sin(a) * 0.08, 0.2 + 0.05 * (k % 2)))
        bm.faces.new([p, q, t])
        bm.faces.new([q, r, t])
        bm.faces.new([r, p, t])
        o = _finish(_bm_object("detail_blade%d" % k, bm), "detail_blade%d" % k, "7fa070" if k % 2 else "94b37e", (0, 0, 0), (0, 0, a))
