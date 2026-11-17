"""Helpers shared by Oorbit's model scripts (run inside Blender, see build.py).

Conventions every model follows, read back by `game/models.gd`:

- Scale: one Blender metre is 0.1 of the game's tile units (a hut is about
  1.2 m wide here). The game shrinks models again on denser planets.
- Axes: Blender's Z is up and the front faces -Y (glTF turns these into the
  game's +Y up and +Z front).
- Colour: flat-shaded faces, one material per colour. A material named
  `tint1`, `tint2`... takes a colour the game picks (a tree's leaves, a
  building's walls in a later era); any other material's base colour is
  baked into the game's vertex colours.
- Lights: a material named `glow_<colour>` (`glow_f2e1a0`, a lit window)
  glows on the night side of the planet.
- Levels of detail: objects whose name starts with `detail` are small parts
  the far view drops (a window, a fence post); objects whose name starts with
  `far` replace everything else in the far view (a tree as a single cone).
  A model with neither is drawn the same near and far.
- Animation: an object named `anim_<axis>_<p|m><amplitude>` (for example
  `anim_x_p60`, `anim_x_m60`) swings around Blender's axis through its own
  origin, by up to the amplitude in hundredths of a radian, in phase (p) or
  against it (m); the game's vertex shader drives the swing (walking legs
  and arms, a swishing tail). Names carry no dots: Blender adds `.001` to
  repeated names, which the game ignores.
"""
import math
import random

import bpy
import bmesh

UNIT = 1.0
_materials = {}


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def material(color):
    """A flat material: a palette colour ("6b5a4e") or a tint slot ("tint1")."""
    if color in _materials:
        return _materials[color]
    m = bpy.data.materials.new(color)
    if color.startswith("tint"):
        rgb = (1.0, 1.0, 1.0)
    else:
        hexa = color[5:] if color.startswith("glow_") else color
        rgb = tuple(srgb_to_linear(int(hexa[i:i + 2], 16) / 255.0) for i in (0, 2, 4))
    m.diffuse_color = (*rgb, 1.0)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    if bsdf:
        bsdf.inputs["Base Color"].default_value = (*rgb, 1.0)
        bsdf.inputs["Roughness"].default_value = 1.0
    _materials[color] = m
    return m


def reset():
    """An empty scene."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    _materials.clear()


def _finish(obj, name, color, loc, rot, flat=True):
    obj.name = name
    obj.data.name = name
    obj.data.materials.clear()
    obj.data.materials.append(material(color))
    obj.location = loc
    obj.rotation_euler = rot
    if flat:
        for p in obj.data.polygons:
            p.use_smooth = False
    return obj


def _bm_object(name, bm):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def box(name, color, size, loc=(0, 0, 0), rot=(0, 0, 0), bevel=0.0):
    """A box `size` (x, y, z) standing on its base at `loc`."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co.x *= size[0]
        v.co.y *= size[1]
        v.co.z = (v.co.z + 0.5) * size[2]
    if bevel > 0:
        bmesh.ops.bevel(bm, geom=list(bm.edges), offset=bevel, segments=1, affect='EDGES')
    return _finish(_bm_object(name, bm), name, color, loc, rot)


def cylinder(name, color, r, h, sides=6, loc=(0, 0, 0), rot=(0, 0, 0), r_top=None, cap=True):
    """A prism of `sides` (a cone if r_top is 0) standing on its base."""
    bm = bmesh.new()
    top = r if r_top is None else r_top
    bmesh.ops.create_cone(bm, cap_ends=cap, cap_tris=False, segments=sides,
                          radius1=r, radius2=max(top, 0.0), depth=h)
    for v in bm.verts:
        v.co.z += h / 2.0
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-5)
    return _finish(_bm_object(name, bm), name, color, loc, rot)


def cone(name, color, r, h, sides=6, loc=(0, 0, 0), rot=(0, 0, 0)):
    return cylinder(name, color, r, h, sides, loc, rot, r_top=0.0)


def blob(name, color, r, loc=(0, 0, 0), squash=1.0, subdiv=1, seed=0, rough=0.12, rot=(0, 0, 0)):
    """A rough low-poly ball (a bush, a crown of leaves, a boulder)."""
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=subdiv, radius=r)
    rng = random.Random(seed)
    for v in bm.verts:
        k = 1.0 + rng.uniform(-rough, rough)
        v.co.x *= k
        v.co.y *= k
        v.co.z *= k * squash
    return _finish(_bm_object(name, bm), name, color, loc, rot)


def rock(name, color, r, loc=(0, 0, 0), squash=0.7, seed=0, rot=(0, 0, 0)):
    """A boulder: a rough ball flattened, sitting on its base."""
    o = blob(name, color, r, loc, squash, subdiv=1, seed=seed, rough=0.25, rot=rot)
    lowest = min(v.co.z for v in o.data.vertices)
    for v in o.data.vertices:
        v.co.z -= lowest * 0.75
    return o


def prism(name, color, w, h, d, loc=(0, 0, 0), rot=(0, 0, 0)):
    """A gable roof: `w` across (X), ridge along Y (`d` long), `h` high."""
    bm = bmesh.new()
    a = [bm.verts.new((-w / 2, -d / 2, 0)), bm.verts.new((w / 2, -d / 2, 0)), bm.verts.new((0, -d / 2, h))]
    b = [bm.verts.new((-w / 2, d / 2, 0)), bm.verts.new((w / 2, d / 2, 0)), bm.verts.new((0, d / 2, h))]
    bm.faces.new([a[0], a[2], a[1]])
    bm.faces.new([b[0], b[1], b[2]])
    bm.faces.new([a[0], a[1], b[1], b[0]])
    bm.faces.new([a[1], a[2], b[2], b[1]])
    bm.faces.new([a[2], a[0], b[0], b[2]])
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    return _finish(_bm_object(name, bm), name, color, loc, rot)


def jitter(obj, amount, seed=0, keep_base=True):
    """Move each vertex a little, so shapes look hand-cut."""
    rng = random.Random(seed)
    for v in obj.data.vertices:
        if keep_base and v.co.z < 1e-4:
            v.co.x += rng.uniform(-amount, amount)
            v.co.y += rng.uniform(-amount, amount)
            continue
        v.co.x += rng.uniform(-amount, amount)
        v.co.y += rng.uniform(-amount, amount)
        v.co.z += rng.uniform(-amount, amount)
    return obj


def ring(n, r, start=0.0):
    """`n` points around a circle of radius `r`: [(x, y, angle)]."""
    return [(math.cos(start + k * math.tau / n) * r, math.sin(start + k * math.tau / n) * r,
             start + k * math.tau / n) for k in range(n)]


def triangles():
    """How many triangles the model draws near and far."""
    near = far = detail = 0
    has_far = False
    for o in bpy.context.scene.objects:
        if o.type != 'MESH':
            continue
        n = sum(len(p.vertices) - 2 for p in o.data.polygons)
        if o.name.startswith("far"):
            has_far = True
            far += n
        else:
            near += n
            if o.name.startswith("detail"):
                detail += n
    return near, far if has_far else near - detail


def export(path):
    """Write the scene as a GLB: meshes with flat normals and materials,
    modifiers applied, no cameras or lights."""
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format='GLB',
        export_apply=True,
        export_yup=True,
        export_normals=True,
        export_texcoords=False,
        export_materials='EXPORT',
        export_cameras=False,
        export_lights=False,
        export_animations=False,
    )
