"""Build Oorbit's models: each script in blender/models/ makes one model,
exported to game/models/<name>.glb (and, with --blend, saved as
blender/blend/<name>.blend to open and refine in Blender).

    blender -b --factory-startup --python plugins/oorbit/blender/build.py -- [--blend] [NAME...]

A model script defines `build()`, which adds its objects to an empty scene
with the helpers in lib.py (see its conventions). Writes nothing else.
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "models"))
import lib  # noqa: E402

import bpy  # noqa: E402

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
save_blend = "--blend" in args
names = [a for a in args if not a.startswith("--")]
models = os.path.join(HERE, "models")
out = os.path.join(HERE, "..", "game", "models")
os.makedirs(out, exist_ok=True)
if not names:
    names = sorted(f[:-3] for f in os.listdir(models) if f.endswith(".py") and not f.startswith("_"))

for name in names:
    spec = importlib.util.spec_from_file_location("model_" + name, os.path.join(models, name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    lib.reset()
    module.build()
    near, far = lib.triangles()
    lib.export(os.path.join(out, name + ".glb"))
    if save_blend:
        os.makedirs(os.path.join(HERE, "blend"), exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=os.path.join(HERE, "blend", name + ".blend"), compress=True)
    print("model %-16s %4d triangles near, %4d far" % (name, near, far))
