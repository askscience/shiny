## The low-poly models made in Blender (`blender/models/*.py`, exported to
## `models/<name>.glb`, one file per model), turned into plain vertex
## arrays the planet merges into its meshes. See `blender/lib.py` for the
## conventions read here: tint materials, `detail` and `far` parts, and
## `anim_*` parts that swing.
class_name Models
extends RefCounted

## Blender metres to the game's tile units.
const SCALE := 0.1

## One level of detail: vertex arrays, plus per vertex the tint slot (-1
## for a baked colour), and for animated parts the pivot and swing.
class Part:
	var verts := PackedVector3Array()
	var normals := PackedVector3Array()
	var colors := PackedColorArray()
	var slots := PackedInt32Array()
	var indices := PackedInt32Array()
	## Pivot of the swing, in the two coordinates across its axis.
	var pivots := PackedVector2Array()
	## (amplitude in radians, axis: 0 x, 1 y, 2 z).
	var swings := PackedVector2Array()
	var animated := false

	func add(arrays: Array, xform: Transform3D, color: Color, slot: int, swing: Vector2) -> void:
		var start := verts.size()
		var vs: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var ns: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
		var rotation := xform.basis.orthonormalized()
		var origin := xform.origin
		var pivot := Vector2.ZERO
		match int(swing.y):
			0: pivot = Vector2(origin.y, origin.z)
			1: pivot = Vector2(origin.x, origin.z)
			2: pivot = Vector2(origin.x, origin.y)
		if swing.x != 0.0:
			animated = true
		for k in vs.size():
			verts.append(xform * vs[k])
			normals.append(rotation * ns[k])
			colors.append(color)
			slots.append(slot)
			pivots.append(pivot)
			swings.append(swing)
		var index = arrays[Mesh.ARRAY_INDEX]
		if index == null or (index as PackedInt32Array).is_empty():
			for k in vs.size():
				indices.append(start + k)
		else:
			for i in index:
				indices.append(start + i)

	## A mesh of this part for MultiMesh instances: tinted vertices carry
	## alpha 0 (the shader paints them with the instance's colour), and
	## animated parts their pivot (UV) and swing (UV2).
	func mesh() -> ArrayMesh:
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = verts
		arrays[Mesh.ARRAY_NORMAL] = normals
		var cs := PackedColorArray()
		cs.resize(colors.size())
		for k in colors.size():
			var c := colors[k]
			cs[k] = Color(c.r, c.g, c.b, 0.0 if slots[k] >= 0 else 1.0)
		arrays[Mesh.ARRAY_COLOR] = cs
		arrays[Mesh.ARRAY_TEX_UV] = pivots
		arrays[Mesh.ARRAY_TEX_UV2] = swings
		arrays[Mesh.ARRAY_INDEX] = indices
		var m := ArrayMesh.new()
		m.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		return m


## A model: `near` (everything), `far` (its far parts, or everything but
## the details), and its height.
class Model:
	var near := Part.new()
	var far := Part.new()
	var height := 0.0

static var _cache := {}


## The model `name` (models/<name>.glb), loaded once.
static func get_model(name: String) -> Model:
	if _cache.has(name):
		return _cache[name]
	var model := Model.new()
	_cache[name] = model
	var scene := load("res://models/%s.glb" % name) as PackedScene
	if scene == null:
		push_warning("no model %s" % name)
		return model
	var root := scene.instantiate()
	var has_far := false
	var parts := []
	_collect(root, Transform3D.IDENTITY.scaled(Vector3.ONE * SCALE), parts)
	for p in parts:
		if str(p[1]).begins_with("far"):
			has_far = true
	for p in parts:
		var mi: MeshInstance3D = p[0]
		var label: String = p[1]
		var xform: Transform3D = p[2]
		var swing := _swing(label)
		for s in mi.mesh.get_surface_count():
			var arrays := mi.mesh.surface_get_arrays(s)
			var material := mi.get_active_material(s)
			var color := Color.WHITE
			var slot := -1
			if material != null:
				var tint := material.resource_name.to_lower()
				if tint.begins_with("tint"):
					slot = maxi(0, tint.substr(4, 1).to_int() - 1)
				elif material is BaseMaterial3D:
					color = (material as BaseMaterial3D).albedo_color
			if label.begins_with("far"):
				model.far.add(arrays, xform, color, slot, swing)
				continue
			model.near.add(arrays, xform, color, slot, swing)
			if not has_far and not label.begins_with("detail"):
				model.far.add(arrays, xform, color, slot, swing)
	for v in model.near.verts:
		model.height = maxf(model.height, v.y)
	root.free()
	return model


## Every mesh under `node` with its name (the Blender object's) and its
## transform from the model's origin.
static func _collect(node: Node, xform: Transform3D, out: Array) -> void:
	var here := xform
	if node is Node3D:
		here = xform * (node as Node3D).transform
	if node is MeshInstance3D and (node as MeshInstance3D).mesh != null:
		var mi := node as MeshInstance3D
		# The node keeps the Blender object's name (the mesh's has the
		# file's name in front).
		var label := str(mi.name)
		out.append([mi, label, here])
	for child in node.get_children():
		_collect(child, here, out)


## `anim_<axis>_<p|m><hundredths>` (Blender axes) as (radians, game axis).
static func _swing(label: String) -> Vector2:
	if not label.begins_with("anim_"):
		return Vector2(0.0, 0.0)
	var bits := label.split("_")
	if bits.size() < 3:
		return Vector2.ZERO
	var axis := {"x": 0, "y": 2, "z": 1}.get(bits[1], 0) as int
	var amount := bits[2].get_slice(".", 0).substr(1).to_int() / 100.0
	if bits[2].begins_with("m"):
		amount = -amount
	return Vector2(amount, axis)
