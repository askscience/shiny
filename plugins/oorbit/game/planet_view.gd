## The 3D planet drawn from the simulation's state: tiles by biome, a small
## model for each building (scaffolding while it is being built), nature on
## the free tiles, and the selected tile's outline. Turns by itself, or by
## dragging; a click picks a tile.
class_name PlanetView
extends Node3D

signal tile_picked(index: int)

const SPIN_SPEED := 0.08          # radians per second, while idle
const IDLE_BEFORE_SPIN := 6.0     # seconds after the last touch

var pivot: Node3D
var builder := PlanetMesh.new()
var _land: MeshInstance3D
var _outline: MeshInstance3D
## Every prop (trees, peaks, buildings) merged into one mesh with vertex
## colours: one draw call instead of hundreds of small ones.
var _props: MeshInstance3D
var _verts := PackedVector3Array()
var _normals := PackedVector3Array()
var _colors := PackedColorArray()
var _indices := PackedInt32Array()
static var _shapes := {}

## Little people walking around the inhabited tiles: one per two people, up
## to MAX_WALKERS, all in one MultiMesh (one draw call).
const MAX_WALKERS := 30
const WALKER_COLORS := [Color("f2c078"), Color("e08f6c"), Color("eef3f2"), Color("e3c3c6"), Color("c2d69b")]
var _walkers: MultiMeshInstance3D
var _walk := []  # per walker: [tile, angle, radius, speed, phase]
var _highlight: MeshInstance3D
var _biomes: Array = []
var _signature := ""
var _idle := IDLE_BEFORE_SPIN
var selected := -1


func _ready() -> void:
	pivot = Node3D.new()
	pivot.rotation = Vector3(deg_to_rad(14.0), 0.0, deg_to_rad(-12.0))
	add_child(pivot)
	_land = MeshInstance3D.new()
	_land.material_override = _toon(Color.WHITE, true)
	pivot.add_child(_land)
	_outline = MeshInstance3D.new()
	_outline.scale = Vector3.ONE * 1.025
	var outline_material := StandardMaterial3D.new()
	outline_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	outline_material.cull_mode = BaseMaterial3D.CULL_FRONT
	outline_material.albedo_color = Palette.OUTLINE
	_outline.material_override = outline_material
	pivot.add_child(_outline)
	_props = MeshInstance3D.new()
	_props.material_override = _toon(Color.WHITE, true)
	pivot.add_child(_props)
	_walkers = MultiMeshInstance3D.new()
	var walker_mesh := BoxMesh.new()
	walker_mesh.size = Vector3(0.03, 0.055, 0.03)
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_colors = true
	multimesh.mesh = walker_mesh
	_walkers.multimesh = multimesh
	_walkers.material_override = _toon(Color.WHITE, true)
	pivot.add_child(_walkers)
	_highlight = MeshInstance3D.new()
	var line_material := StandardMaterial3D.new()
	line_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	line_material.albedo_color = Color("f2c078")
	line_material.no_depth_test = true
	_highlight.material_override = line_material
	pivot.add_child(_highlight)


func _process(delta: float) -> void:
	_idle += delta
	if _idle > IDLE_BEFORE_SPIN and selected < 0:
		pivot.rotate(Vector3.UP, SPIN_SPEED * delta)
	_move_walkers(delta)


func _move_walkers(delta: float) -> void:
	var multimesh := _walkers.multimesh
	for k in _walk.size():
		var w: Array = _walk[k]
		w[1] = float(w[1]) + float(w[3]) * delta
		var tile: int = w[0]
		var normal: Vector3 = builder.centers[tile]
		var h: float = PlanetMesh.HEIGHT.get(_biomes[tile], 1.0)
		var base := normal * PlanetMesh.surface(tile, normal, h, _biomes[tile])
		var t := _tangents(normal)
		var u: Vector3 = t[0]
		var v: Vector3 = t[1]
		var angle: float = w[1]
		var r: float = w[2]
		# A slow loop around the tile, with a little bounce in the step.
		var bob := absf(sin(angle * 9.0 + float(w[4]))) * 0.008
		var at := base + (u * cos(angle) + v * sin(angle)) * r + normal * (0.02 + bob)
		var forward := (-u * sin(angle) + v * cos(angle)) * signf(float(w[3]))
		var basis := Basis(forward.cross(normal).normalized(), normal, -forward).scaled(Vector3.ONE * _tile_size())
		multimesh.set_instance_transform(k, Transform3D(basis, at))


## Props shrink on denser planets (more, smaller tiles).
func _tile_size() -> float:
	return sqrt(32.0 / maxf(1.0, float(builder.centers.size())))


## Place the walkers for this many people on the inhabited tiles.
func _place_walkers(population: float, buildings: Array) -> void:
	var homes := []
	for i in buildings.size():
		if buildings[i] != "" and not str(buildings[i]).begins_with("construction:"):
			homes.append(i)
	if homes.is_empty():
		for i in _biomes.size():
			if _biomes[i] == "plains" or _biomes[i] == "forest":
				homes.append(i)
		homes = homes.slice(0, 2)
	var count := mini(MAX_WALKERS, maxi(2, roundi(population / 2.0)))
	if homes.is_empty():
		count = 0
	var rng := RandomNumberGenerator.new()
	rng.seed = 23
	_walk.clear()
	var multimesh := _walkers.multimesh
	multimesh.instance_count = count
	for k in count:
		var speed := (0.25 + rng.randf() * 0.35) * (1.0 if rng.randf() < 0.5 else -1.0)
		_walk.append([homes[k % homes.size()], rng.randf() * TAU, (0.08 + rng.randf() * 0.1) * _tile_size(), speed, rng.randf() * TAU])
		multimesh.set_instance_color(k, WALKER_COLORS[k % WALKER_COLORS.size()])
	_move_walkers(0.0)


## Rebuild what changed in the planet (`planet` is the simulation's JSON).
func show_planet(planet: Dictionary) -> void:
	var tiles: Array = planet.get("tiles", [])
	var building_at := {}
	for c in planet.get("construction", []):
		building_at[int(c["tile"])] = "construction:" + str(c["building"])
	var biomes := []
	var buildings := []
	for i in tiles.size():
		biomes.append(str(tiles[i]["biome"]))
		var b = tiles[i].get("building")
		buildings.append(str(b) if b != null else building_at.get(i, ""))
	var signature := JSON.stringify([biomes, buildings, roundi(float(planet.get("population", 0)) / 2.0)])
	if signature == _signature:
		return
	_signature = signature
	if biomes != _biomes:
		_biomes = biomes
		var mesh := builder.build(biomes, Palette.BIOMES)
		_land.mesh = mesh
		_outline.mesh = mesh
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	var rng := RandomNumberGenerator.new()
	rng.seed = 11
	for i in biomes.size():
		_place_tile(i, biomes[i], buildings[i], rng)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = _verts
	arrays[Mesh.ARRAY_NORMAL] = _normals
	arrays[Mesh.ARRAY_COLOR] = _colors
	arrays[Mesh.ARRAY_INDEX] = _indices
	var mesh := ArrayMesh.new()
	if _verts.size() > 0:
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	_props.mesh = mesh
	_place_walkers(float(planet.get("population", 0)), buildings)
	_update_highlight()


## Rotate by a drag of `relative` pixels (in the low-resolution view).
func drag(relative: Vector2) -> void:
	_idle = 0.0
	pivot.rotate(Vector3.UP, relative.x * 0.012)
	pivot.rotate(Vector3.RIGHT, relative.y * 0.012)


## The tile under a point of the low-resolution view, or -1.
func pick(camera: Camera3D, point: Vector2) -> int:
	_idle = 0.0
	var origin := camera.project_ray_origin(point)
	var direction := camera.project_ray_normal(point)
	# Ray against the planet's sphere (radius ~1, centred on the pivot).
	var center := pivot.global_position
	var oc := origin - center
	var b := oc.dot(direction)
	var c := oc.dot(oc) - 1.03 * 1.03
	var disc := b * b - c
	if disc < 0.0:
		return -1
	var hit := origin + direction * (-b - sqrt(disc))
	var local := (pivot.global_transform.affine_inverse() * hit).normalized()
	var best := -1
	var best_dot := -2.0
	for i in builder.centers.size():
		var d := local.dot(builder.centers[i])
		if d > best_dot:
			best_dot = d
			best = i
	return best


func select(index: int) -> void:
	selected = index
	_idle = 0.0
	_update_highlight()


func _update_highlight() -> void:
	if selected < 0 or selected >= builder.corners.size():
		_highlight.mesh = null
		return
	var ring: PackedVector3Array = builder.corners[selected]
	var h: float = PlanetMesh.HEIGHT.get(_biomes[selected], 1.0) * 1.012
	var mesh := ImmediateMesh.new()
	mesh.surface_begin(Mesh.PRIMITIVE_LINE_STRIP)
	for k in ring.size() + 1:
		mesh.surface_add_vertex(ring[k % ring.size()] * h)
	mesh.surface_end()
	_highlight.mesh = mesh


# ---------------------------------------------------------------- models

func _toon(color: Color, vertex_colors := false) -> StandardMaterial3D:
	var m := StandardMaterial3D.new()
	m.albedo_color = color
	m.vertex_color_use_as_albedo = vertex_colors
	m.diffuse_mode = BaseMaterial3D.DIFFUSE_TOON
	m.specular_mode = BaseMaterial3D.SPECULAR_DISABLED
	m.roughness = 1.0
	return m


## A primitive's vertex arrays, built once per shape and reused.
static func _shape(key: String, make: Callable) -> Array:
	if not _shapes.has(key):
		var mesh: PrimitiveMesh = make.call()
		_shapes[key] = mesh.get_mesh_arrays()
	return _shapes[key]


static func _cone(radius: float, height: float, sides: int) -> Array:
	return _shape("cone %s %s %s" % [radius, height, sides], func():
		var mesh := CylinderMesh.new()
		mesh.top_radius = 0.0
		mesh.bottom_radius = radius
		mesh.height = height
		mesh.radial_segments = sides
		mesh.rings = 1
		return mesh)


static func _cylinder(radius: float, height: float, sides: int) -> Array:
	return _shape("cylinder %s %s %s" % [radius, height, sides], func():
		var mesh := CylinderMesh.new()
		mesh.top_radius = radius
		mesh.bottom_radius = radius
		mesh.height = height
		mesh.radial_segments = sides
		mesh.rings = 1
		return mesh)


static func _box(x: float, y: float, z: float) -> Array:
	return _shape("box %s %s %s" % [x, y, z], func():
		var mesh := BoxMesh.new()
		mesh.size = Vector3(x, y, z)
		return mesh)


## Two tangents (u, w) so that (u, normal, w) is a right-handed frame: baked
## into the merged mesh, a mirrored frame would turn every face inside out.
func _tangents(normal: Vector3) -> Array:
	var up := Vector3.UP if abs(normal.y) < 0.9 else Vector3.RIGHT
	var n := normal.normalized()
	var u := n.cross(up).normalized()
	return [u, u.cross(n)]


## Place `mesh` on the tile: `offset` is (along u, up, along w) in the tile's
## frame, `turn` rotates it around the tile's up.
func _put(shape: Array, color: Color, normal: Vector3, base: Vector3, offset: Vector3, size := 1.0, turn := 0.0) -> void:
	var t := _tangents(normal)
	var u: Vector3 = t[0]
	var w: Vector3 = t[1]
	var y := normal.normalized()
	var basis := Basis(u, y, w).rotated(y, turn).scaled(Vector3.ONE * size * _scale)
	offset *= _scale
	var xform := Transform3D(basis, base + u * offset.x + y * offset.y + w * offset.z)
	var rotation := basis.orthonormalized()
	var start := _verts.size()
	var verts: PackedVector3Array = shape[Mesh.ARRAY_VERTEX]
	var normals: PackedVector3Array = shape[Mesh.ARRAY_NORMAL]
	for k in verts.size():
		_verts.append(xform * verts[k])
		_normals.append(rotation * normals[k])
		_colors.append(color)
	var indices: PackedInt32Array = shape[Mesh.ARRAY_INDEX]
	for index in indices:
		_indices.append(start + index)


func _place_tile(i: int, biome: String, building: String, rng: RandomNumberGenerator) -> void:
	var normal: Vector3 = builder.centers[i]
	var h: float = PlanetMesh.HEIGHT.get(biome, 1.0)
	var base := normal * PlanetMesh.surface(i, normal, h, biome)
	if building.begins_with("construction:"):
		_scaffold(normal, base)
		_nature(normal, base, biome, rng, 2)
		return
	if building == "":
		_nature(normal, base, biome, rng, 6)
		return
	_building(building, normal, base)
	_nature(normal, base, biome, rng, 2)


## Buildings are drawn larger than life, so they read at the pixel scale.
const BUILDING_SCALE := 1.15


## Trees, peaks or ice around the tile's edge (`amount` trees on a forest).
func _nature(normal: Vector3, base: Vector3, biome: String, rng: RandomNumberGenerator, amount: int) -> void:
	match biome:
		"forest":
			for k in amount:
				var angle := k * TAU / amount + rng.randf() * 0.6
				var r := (0.05 + rng.randf() * 0.09) if amount > 2 else 0.13
				var s := 0.7 + rng.randf() * 0.6
				_put(_cone(0.06, 0.18, 5), Palette.TREE, normal, base,
					Vector3(cos(angle) * r, 0.09 * s, sin(angle) * r), s)
		"mountain":
			var peaks := 2 if amount > 2 else 1
			for k in peaks:
				var angle := k * PI + rng.randf()
				var r := (0.035 + k * 0.05) if amount > 2 else 0.12
				var s := 1.0 - k * 0.3 if amount > 2 else 0.6
				var at := Vector3(cos(angle) * r, 0.0, sin(angle) * r)
				_put(_cone(0.16, 0.26, 4), Palette.PEAK, normal, base, at + Vector3(0, 0.13 * s, 0), s)
				_put(_cone(0.065, 0.1, 4), Palette.SNOW, normal, base, at + Vector3(0, 0.21 * s, 0), s)
		"ice":
			for k in mini(amount, 3):
				var angle := k * TAU / 3.0 + rng.randf()
				_put(_cone(0.08, 0.06, 5), Palette.SNOW, normal, base,
					Vector3(cos(angle) * 0.1, 0.03, sin(angle) * 0.1), 0.8 + rng.randf() * 0.4)


func _scaffold(normal: Vector3, base: Vector3) -> void:
	var pole := _box(0.012, 0.12, 0.012)
	for k in 4:
		var angle := k * TAU / 4.0 + PI / 4.0
		_put(pole, Palette.HUT_WALL, normal, base, Vector3(cos(angle) * 0.05, 0.06, sin(angle) * 0.05))
	_put(_box(0.11, 0.012, 0.11), Palette.HUT_WALL, normal, base, Vector3(0, 0.12, 0))
	_put(_box(0.07, 0.03, 0.05), Palette.PEAK, normal, base, Vector3(0.02, 0.015, 0.0))


## One small model per building id.
func _building(id: String, n: Vector3, b: Vector3) -> void:
	_scale = BUILDING_SCALE
	_building_parts(id, n, b)
	_scale = 1.0


var _scale := 1.0


func _building_parts(id: String, n: Vector3, b: Vector3) -> void:
	var wall := Palette.HUT_WALL
	var roof := Palette.HUT_ROOF
	var stone := Palette.PEAK
	match id:
		"hut":
			_put(_box(0.11, 0.07, 0.11), wall, n, b, Vector3(0, 0.035, 0))
			_put(_cone(0.1, 0.09, 4), roof, n, b, Vector3(0, 0.115, 0))
		"house":
			_put(_box(0.15, 0.09, 0.11), Color("d9b48f"), n, b, Vector3(0, 0.045, 0))
			_put(_cone(0.12, 0.08, 4), roof, n, b, Vector3(0, 0.13, 0), 1.0, PI / 4.0)
		"hunting_camp":
			_put(_cone(0.08, 0.13, 6), Color("8f6f5a"), n, b, Vector3(-0.03, 0.065, 0))
			_put(_box(0.01, 0.16, 0.01), Color("7a4f4f"), n, b, Vector3(0.07, 0.08, 0.02))
		"fire_pit":
			for k in 6:
				var a := k * TAU / 6.0
				_put(_box(0.03, 0.02, 0.03), stone, n, b, Vector3(cos(a) * 0.06, 0.01, sin(a) * 0.06))
			_put(_cone(0.04, 0.08, 5), Color("f2c078"), n, b, Vector3(0, 0.04, 0))
			_put(_cone(0.025, 0.05, 5), Color("e08f6c"), n, b, Vector3(0, 0.07, 0))
		"stone_circle":
			for k in 7:
				var a := k * TAU / 7.0
				_put(_box(0.035, 0.11, 0.03), stone, n, b, Vector3(cos(a) * 0.12, 0.055, sin(a) * 0.12), 1.0, a)
			_put(_box(0.06, 0.04, 0.06), Color("d9d0cc"), n, b, Vector3(0, 0.02, 0))
		"field":
			for k in 3:
				_put(_box(0.24, 0.02, 0.05), Color("f2c078") if k != 1 else Color("c2d69b"), n, b, Vector3(0, 0.01, (k - 1) * 0.07))
		"pasture":
			for k in 4:
				var a := k * TAU / 4.0
				_put(_box(0.2, 0.025, 0.01), Color("8f6f5a"), n, b, Vector3(cos(a) * 0.1, 0.02, sin(a) * 0.1), 1.0, a + PI / 2.0)
			_put(_box(0.04, 0.03, 0.06), Palette.SNOW, n, b, Vector3(0.02, 0.025, 0.02))
			_put(_box(0.04, 0.03, 0.06), Palette.SNOW, n, b, Vector3(-0.04, 0.025, -0.03), 1.0, 1.0)
		"storehouse":
			_put(_box(0.2, 0.08, 0.12), Color("b58e6b"), n, b, Vector3(0, 0.04, 0))
			_put(_box(0.22, 0.02, 0.14), roof, n, b, Vector3(0, 0.09, 0))
		"great_granary":
			_put(_cylinder(0.09, 0.16, 8), Color("d9b48f"), n, b, Vector3(0, 0.08, 0))
			_put(_cone(0.11, 0.1, 8), roof, n, b, Vector3(0, 0.21, 0))
			_put(_cylinder(0.05, 0.1, 6), Color("d9b48f"), n, b, Vector3(0.13, 0.05, 0.04))
			_put(_cone(0.065, 0.06, 6), roof, n, b, Vector3(0.13, 0.13, 0.04))
		"mine":
			_put(_box(0.13, 0.08, 0.08), Color("5b5466"), n, b, Vector3(0, 0.04, 0))
			_put(_box(0.06, 0.05, 0.02), Palette.OUTLINE, n, b, Vector3(0, 0.025, 0.045))
			_put(_box(0.05, 0.03, 0.04), Color("b58e6b"), n, b, Vector3(0.09, 0.015, 0.06))
		"foundry":
			_put(_box(0.14, 0.08, 0.11), Color("857c8a"), n, b, Vector3(0, 0.04, 0))
			_put(_cylinder(0.02, 0.12, 6), Color("5b5466"), n, b, Vector3(0.04, 0.13, 0.02))
			_put(_box(0.04, 0.03, 0.01), Color("e08f6c"), n, b, Vector3(-0.03, 0.03, 0.056))
		"temple":
			_put(_box(0.18, 0.03, 0.14), stone, n, b, Vector3(0, 0.015, 0))
			for k in 4:
				_put(_box(0.015, 0.09, 0.015), Color("d9d0cc"), n, b, Vector3(-0.06 + k * 0.04, 0.075, 0.05))
			_put(_cone(0.12, 0.05, 4), Color("d9d0cc"), n, b, Vector3(0, 0.145, 0), 1.0, PI / 4.0)
		"scribe_house":
			_put(_box(0.13, 0.09, 0.11), Color("e3c3c6"), n, b, Vector3(0, 0.045, 0))
			_put(_box(0.15, 0.02, 0.13), Color("7a4f4f"), n, b, Vector3(0, 0.1, 0))
		"ziggurat":
			for k in 4:
				var s := 0.26 - k * 0.055
				_put(_box(s, 0.05, s), Color("d9b48f") if k % 2 == 0 else Color("b58e6b"), n, b, Vector3(0, 0.025 + k * 0.05, 0))
			_put(_box(0.04, 0.04, 0.04), Color("f2c078"), n, b, Vector3(0, 0.22, 0))
		_:
			_put(_box(0.1, 0.08, 0.1), wall, n, b, Vector3(0, 0.04, 0))
