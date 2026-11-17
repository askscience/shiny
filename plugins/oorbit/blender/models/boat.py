"""A fishing boat: a pointed hull, a mast, a sail. Front -Y (game +Z)."""
import bmesh

from lib import _bm_object, _finish, box


def build():
    bm = bmesh.new()
    w, l, h = 0.14, 0.3, 0.14
    pts = [(-w, -l * 0.6, h), (w, -l * 0.6, h), (w, l, h), (-w, l, h), (0, -l * 1.15, h * 1.2)]
    top = [bm.verts.new(p) for p in pts]
    bot = [bm.verts.new((x * 0.6, y * 0.8, 0)) for x, y, _ in pts]
    order = [4, 1, 2, 3, 0]
    for k in range(5):
        a, b = order[k], order[(k + 1) % 5]
        bm.faces.new([top[a], bot[a], bot[b], top[b]])
    bm.faces.new([bot[i] for i in order])
    bm.faces.new([top[i] for i in reversed(order)])
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    _finish(_bm_object("hull", bm), "hull", "6b5a4e", (0, 0, 0), (0, 0, 0))
    box("detail_stripe", "e08f6c", (0.29, 0.62, 0.02), loc=(0, 0.05, 0.11))
    box("mast", "4a3b33", (0.03, 0.03, 0.5), loc=(0, 0, 0.14))
    box("sail", "tint1", (0.01, 0.28, 0.3), loc=(0.02, 0.0, 0.3))
