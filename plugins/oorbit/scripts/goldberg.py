"""Cell layouts for Oorbit's planets: Goldberg polyhedra GP(m, n).

    python3 -I plugins/oorbit/scripts/goldberg.py crates/oorbit-sim/data

Writes cells_<N>.json with:
- `centers`: N cell centres (the vertices of a geodesic icosahedron, unit
  vectors). The 12 pentagons come first, then the hexagons from north to
  south.
- `vertices`: the cell corners, shared between the three cells that meet
  there (the circumcentres of the Delaunay triangles, unit vectors).
- `rings`: each cell's corners as indices into `vertices`,
  counter-clockwise seen from outside.
- `neighbors`: each cell's neighbours, in the same order as its ring edges
  (neighbour k shares the edge from corner k to corner k+1).

The simulation embeds the file (crates/oorbit-sim/src/cells.rs) and the
window gets it from the plugin's `layout` route, so both use the same cells.

GP(2,1) has 72 cells, GP(4,0) 162, GP(6,0) 362, GP(8,0) 642 (the one in use).
"""
import json, math, sys


def norm(v):
    l = math.sqrt(sum(x * x for x in v))
    return [x / l for x in v]


def sub(a, b): return [a[i] - b[i] for i in range(3)]
def dot(a, b): return sum(a[i] * b[i] for i in range(3))
def cross(a, b): return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


T = (1 + 5 ** .5) / 2
V = [norm(v) for v in [(-1, T, 0), (1, T, 0), (-1, -T, 0), (1, -T, 0), (0, -1, T), (0, 1, T),
                       (0, -1, -T), (0, 1, -T), (T, 0, -1), (T, 0, 1), (-T, 0, -1), (-T, 0, 1)]]
F = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2),
     (10, 7, 6), (7, 1, 8), (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5),
     (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]


def points(m, n):
    """The geodesic points: each icosahedron face sampled on the (m, n) lattice."""
    e1 = (1.0, 0.0)
    e2 = (0.5, math.sqrt(3) / 2)
    c = lambda i, j: (i * e1[0] + j * e2[0], i * e1[1] + j * e2[1])
    P0, P1, P2 = c(0, 0), c(m, n), c(-n, m + n)
    det = (P1[0] - P0[0]) * (P2[1] - P0[1]) - (P2[0] - P0[0]) * (P1[1] - P0[1])
    pts = []
    seen = {}
    R = m + n + 1
    for (a, b, cc) in F:
        A, B, C = V[a], V[b], V[cc]
        for i in range(-R, R + 1):
            for j in range(-R, R + 1):
                x, y = c(i, j)
                l1 = ((x - P0[0]) * (P2[1] - P0[1]) - (P2[0] - P0[0]) * (y - P0[1])) / det
                l2 = ((P1[0] - P0[0]) * (y - P0[1]) - (x - P0[0]) * (P1[1] - P0[1])) / det
                l0 = 1 - l1 - l2
                if min(l0, l1, l2) < -1e-9:
                    continue
                p = norm([l0 * A[k] + l1 * B[k] + l2 * C[k] for k in range(3)])
                key = tuple(round(x, 6) for x in p)
                if key not in seen:
                    seen[key] = len(pts)
                    pts.append(p)
    return pts


def layout(m, n):
    P = points(m, n)
    N = len(P)
    # Delaunay triangles of points on a sphere = faces of their convex hull.
    # Each point's triangles use its 6 or 7 nearest points.
    near = [sorted(range(N), key=lambda j: -dot(P[i], P[j]))[1:8] for i in range(N)]
    faces = {}
    for i in range(N):
        cand = near[i]
        for x in range(len(cand)):
            for y in range(x + 1, len(cand)):
                j, k = cand[x], cand[y]
                tri = tuple(sorted((i, j, k)))
                if tri in faces:
                    continue
                nrm = norm(cross(sub(P[j], P[i]), sub(P[k], P[i])))
                if dot(nrm, P[i]) < 0:
                    nrm = [-v for v in nrm]
                d = dot(nrm, P[i])
                # Only nearby points can poke above the plane of a small face.
                local = set(near[i]) | set(near[j]) | set(near[k])
                if all(dot(nrm, P[q]) <= d + 1e-9 for q in local):
                    faces[tri] = nrm
    assert len(faces) == 2 * N - 4, (len(faces), 2 * N - 4)
    face_list = list(faces.keys())
    of_point = [[] for _ in range(N)]
    for f, tri in enumerate(face_list):
        for v in tri:
            of_point[v].append(f)
    order = sorted(range(N), key=lambda i: (len(of_point[i]) != 5, -round(P[i][1], 6), math.atan2(P[i][2], P[i][0])))
    new_index = {old: new for new, old in enumerate(order)}
    # Vertices numbered in the order cells first use them.
    vertex_index = {}
    vertices = []
    centers, rings, neighbors = [], [], []
    for i in order:
        c = P[i]
        up = [0, 1, 0] if abs(c[1]) < 0.9 else [1, 0, 0]
        u = norm(cross(c, up))
        w = cross(c, u)
        fs = sorted(of_point[i], key=lambda f: math.atan2(dot(sub(faces[face_list[f]], c), w),
                                                         dot(sub(faces[face_list[f]], c), u)))
        # Counter-clockwise seen from outside: check with the first triangle.
        a, b = faces[face_list[fs[0]]], faces[face_list[fs[1]]]
        if dot(cross(sub(a, c), sub(b, c)), c) < 0:
            fs.reverse()
        ring = []
        for f in fs:
            if f not in vertex_index:
                vertex_index[f] = len(vertices)
                vertices.append(faces[face_list[f]])
            ring.append(vertex_index[f])
        # Neighbour k shares corners k and k+1: the other point of both faces.
        nb = []
        for k in range(len(fs)):
            shared = set(face_list[fs[k]]) & set(face_list[fs[(k + 1) % len(fs)]])
            shared.discard(i)
            assert len(shared) == 1
            nb.append(new_index[shared.pop()])
        centers.append(c)
        rings.append(ring)
        neighbors.append(nb)
    return {"centers": centers, "vertices": vertices, "rings": rings, "neighbors": neighbors}


def main():
    out = sys.argv[1]
    for (m, n) in [(8, 0)]:
        L = layout(m, n)
        sides = [len(r) for r in L["rings"]]
        print(m, n, len(L["centers"]), "cells,", len(L["vertices"]), "corners, pentagons", sides.count(5))
        r6 = lambda p: [round(x, 6) for x in p]
        data = {
            "centers": [r6(p) for p in L["centers"]],
            "vertices": [r6(p) for p in L["vertices"]],
            "rings": L["rings"],
            "neighbors": L["neighbors"],
        }
        with open(f"{out}/cells_{len(L['centers'])}.json", "w") as f:
            json.dump(data, f, separators=(",", ":"))


main()
