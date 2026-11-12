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


## Props shrink on denser planets (more, smaller tiles); they were drawn for
## a 32-tile planet.
func _tile_size() -> float:
	return sqrt(32.0 / maxf(1.0, float(builder.centers.size())))


var _ts := 1.0


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
		_ts = _tile_size()
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
	var basis := Basis(u, y, w).rotated(y, turn).scaled(Vector3.ONE * size * _scale * _ts)
	offset *= _scale * _ts
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
			_styled(id, n, b)


## Later eras' buildings by style: [style, main colour, second colour, size].
const STYLES := {
	"villa": ["house", "eae2d6", "c07a54", 1.1],
	"forum": ["temple", "eae2d6", "cfc4b0", 1.0],
	"mint": ["hall", "cfc4b0", "d4a24c", 0.9],
	"academy": ["temple", "cfc4b0", "8e9a5b", 1.0],
	"aqueduct": ["arches", "cfc4b0", "eae2d6", 1.0],
	"amphitheatre": ["bowl", "eae2d6", "c07a54", 1.4],
	"iron_mine": ["mine", "4a4a4f", "8c8c8c", 1.0],
	"manor": ["house", "8c8c8c", "4f5d75", 1.2],
	"watermill": ["mill", "6b5a4e", "4f5d75", 1.0],
	"monastery": ["tower", "8c8c8c", "4f5d75", 1.0],
	"guild_hall": ["hall", "6b5a4e", "b03a2e", 1.0],
	"cathedral": ["cathedral", "8c8c8c", "4f5d75", 1.4],
	"townhouse": ["house", "c8b49a", "3f6e9e", 1.1],
	"glassworks": ["factory", "c8b49a", "7d9cc0", 0.9],
	"paper_mill": ["mill", "c8b49a", "d4a24c", 1.0],
	"printing_house": ["hall", "c8b49a", "3f6e9e", 1.0],
	"observatory": ["dome", "c8b49a", "3f6e9e", 1.0],
	"harbor": ["pier", "6b5a4e", "d4a24c", 1.0],
	"great_dome": ["dome", "d4a24c", "3f6e9e", 1.5],
	"coal_mine": ["mine", "4a4a4f", "1e1b33", 1.0],
	"steel_mill": ["factory", "a5532f", "d9822b", 1.1],
	"factory": ["factory", "a5532f", "4a4a4f", 1.1],
	"tenement": ["block", "a5532f", "4a4a4f", 1.0],
	"railway_station": ["hall", "707780", "a5532f", 1.1],
	"iron_tower": ["lattice", "4a4a4f", "707780", 1.5],
	"power_plant": ["factory", "596b7d", "f5d76e", 1.2],
	"apartment": ["block", "9fb3c8", "f5d76e", 1.3],
	"cinema_hall": ["hall", "596b7d", "f5d76e", 1.0],
	"hospital": ["block", "eef3f2", "b03a2e", 1.1],
	"radio_tower": ["mast", "596b7d", "b03a2e", 1.2],
	"skyscraper": ["skyscraper", "9fb3c8", "f5d76e", 1.6],
	"silicon_foundry": ["factory", "1b1530", "4fe3ff", 1.1],
	"data_center": ["hall", "1b1530", "4fe3ff", 1.1],
	"arcology": ["skyscraper", "1b1530", "ff4fd8", 1.5],
	"neon_strip": ["neon", "1b1530", "ff4fd8", 1.0],
	"robot_factory": ["factory", "2e3440", "9b5cff", 1.1],
	"neon_spire": ["spire", "1b1530", "4fe3ff", 1.8],
	"refinery": ["factory", "8fa3b8", "ff9d3b", 1.1],
	"alloy_works": ["factory", "e8edf2", "6fd3ff", 1.1],
	"hydroponics": ["dome", "e8edf2", "94b37e", 1.0],
	"mission_control": ["dome", "e8edf2", "6fd3ff", 1.1],
	"habitat": ["dome", "e8edf2", "6fd3ff", 1.2],
	"launch_pad": ["rocket", "8fa3b8", "e8edf2", 1.3],
	"orbital_station": ["station", "e8edf2", "6fd3ff", 1.6],
}


static func _ball(radius: float) -> Array:
	return _shape("ball %s" % radius, func():
		var mesh := SphereMesh.new()
		mesh.radius = radius
		mesh.height = radius * 2.0
		mesh.radial_segments = 8
		mesh.rings = 4
		mesh.is_hemisphere = true
		return mesh)


func _styled(id: String, n: Vector3, b: Vector3) -> void:
	var style: Array = STYLES.get(id, ["house", "b58e6b", "a35d5d", 1.0])
	var c1 := Color(style[1])
	var c2 := Color(style[2])
	var k: float = style[3]
	match style[0]:
		"house":
			_put(_box(0.12 * k, 0.08 * k, 0.1 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_cone(0.1 * k, 0.08 * k, 4), c2, n, b, Vector3(0, 0.12 * k, 0), 1.0, PI / 4.0)
		"hall":
			_put(_box(0.2 * k, 0.08 * k, 0.12 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_box(0.22 * k, 0.02 * k, 0.14 * k), c2, n, b, Vector3(0, 0.09 * k, 0))
		"temple":
			_put(_box(0.2 * k, 0.03 * k, 0.15 * k), c2, n, b, Vector3(0, 0.015 * k, 0))
			for i in 5:
				_put(_box(0.014 * k, 0.09 * k, 0.014 * k), c1, n, b, Vector3((-0.08 + i * 0.04) * k, 0.075 * k, 0.05 * k))
			_put(_cone(0.13 * k, 0.05 * k, 4), c1, n, b, Vector3(0, 0.145 * k, 0), 1.0, PI / 4.0)
		"arches":
			for i in 4:
				_put(_box(0.025 * k, 0.1 * k, 0.03 * k), c1, n, b, Vector3((-0.09 + i * 0.06) * k, 0.05 * k, 0))
			_put(_box(0.24 * k, 0.025 * k, 0.04 * k), c2, n, b, Vector3(0, 0.11 * k, 0))
		"bowl":
			_put(_cylinder(0.13 * k, 0.07 * k, 10), c1, n, b, Vector3(0, 0.035 * k, 0))
			_put(_cylinder(0.09 * k, 0.072 * k, 10), c2, n, b, Vector3(0, 0.04 * k, 0))
		"mine":
			_put(_box(0.13 * k, 0.08 * k, 0.08 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_box(0.06 * k, 0.05 * k, 0.02 * k), c2, n, b, Vector3(0, 0.025 * k, 0.045 * k))
			_put(_box(0.02 * k, 0.16 * k, 0.02 * k), c2, n, b, Vector3(0.07 * k, 0.08 * k, -0.03 * k))
		"mill":
			_put(_box(0.11 * k, 0.1 * k, 0.1 * k), c1, n, b, Vector3(0, 0.05 * k, 0))
			_put(_cone(0.09 * k, 0.07 * k, 4), c2, n, b, Vector3(0, 0.135 * k, 0), 1.0, PI / 4.0)
			_put(_box(0.2 * k, 0.015 * k, 0.015 * k), c1, n, b, Vector3(0, 0.1 * k, 0.06 * k), 1.0, 0.6)
			_put(_box(0.2 * k, 0.015 * k, 0.015 * k), c1, n, b, Vector3(0, 0.1 * k, 0.06 * k), 1.0, -0.6)
		"tower":
			_put(_box(0.14 * k, 0.07 * k, 0.12 * k), c1, n, b, Vector3(-0.03 * k, 0.035 * k, 0))
			_put(_box(0.06 * k, 0.18 * k, 0.06 * k), c1, n, b, Vector3(0.06 * k, 0.09 * k, 0))
			_put(_cone(0.05 * k, 0.07 * k, 4), c2, n, b, Vector3(0.06 * k, 0.215 * k, 0), 1.0, PI / 4.0)
		"cathedral":
			_put(_box(0.24 * k, 0.1 * k, 0.12 * k), c1, n, b, Vector3(0, 0.05 * k, 0))
			_put(_cone(0.12 * k, 0.06 * k, 4), c2, n, b, Vector3(0, 0.13 * k, 0), 1.0, PI / 4.0)
			for side in [-1, 1]:
				_put(_box(0.05 * k, 0.24 * k, 0.05 * k), c1, n, b, Vector3(side * 0.1 * k, 0.12 * k, 0.04 * k))
				_put(_cone(0.04 * k, 0.1 * k, 4), c2, n, b, Vector3(side * 0.1 * k, 0.29 * k, 0.04 * k), 1.0, PI / 4.0)
		"dome":
			_put(_cylinder(0.1 * k, 0.07 * k, 10), c1, n, b, Vector3(0, 0.035 * k, 0))
			_put(_ball(0.1 * k), c2, n, b, Vector3(0, 0.07 * k, 0))
		"pier":
			_put(_box(0.26 * k, 0.02 * k, 0.06 * k), c1, n, b, Vector3(0, 0.01 * k, 0))
			_put(_box(0.08 * k, 0.04 * k, 0.16 * k), c1, n, b, Vector3(0.06 * k, 0.03 * k, 0.04 * k))
			_put(_cone(0.05 * k, 0.12 * k, 3), c2, n, b, Vector3(0.06 * k, 0.11 * k, 0.04 * k))
		"factory":
			_put(_box(0.18 * k, 0.08 * k, 0.12 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_cone(0.07 * k, 0.04 * k, 4), c1, n, b, Vector3(-0.04 * k, 0.1 * k, 0), 1.0, PI / 4.0)
			for i in 2:
				_put(_cylinder(0.018 * k, 0.16 * k, 6), c2, n, b, Vector3((0.04 + i * 0.045) * k, 0.12 * k, -0.03 * k))
		"block":
			_put(_box(0.12 * k, 0.18 * k, 0.12 * k), c1, n, b, Vector3(0, 0.09 * k, 0))
			for i in 3:
				_put(_box(0.124 * k, 0.012 * k, 0.124 * k), c2, n, b, Vector3(0, (0.05 + i * 0.05) * k, 0))
		"lattice":
			for i in 4:
				var s := (0.16 - i * 0.035) * k
				_put(_box(s, 0.09 * k, s), c1 if i % 2 == 0 else c2, n, b, Vector3(0, (0.045 + i * 0.09) * k, 0))
			_put(_cone(0.03 * k, 0.12 * k, 4), c2, n, b, Vector3(0, 0.42 * k, 0))
		"mast":
			_put(_box(0.03 * k, 0.32 * k, 0.03 * k), c1, n, b, Vector3(0, 0.16 * k, 0))
			for i in 3:
				_put(_box(0.12 * k * (1.0 - i * 0.25), 0.012 * k, 0.012 * k), c2, n, b, Vector3(0, (0.1 + i * 0.08) * k, 0))
		"skyscraper":
			_put(_box(0.12 * k, 0.3 * k, 0.12 * k), c1, n, b, Vector3(0, 0.15 * k, 0))
			_put(_box(0.08 * k, 0.12 * k, 0.08 * k), c1, n, b, Vector3(0, 0.36 * k, 0))
			for i in 5:
				_put(_box(0.124 * k, 0.01 * k, 0.124 * k), c2, n, b, Vector3(0, (0.04 + i * 0.055) * k, 0))
		"neon":
			_put(_box(0.2 * k, 0.07 * k, 0.1 * k), c1, n, b, Vector3(0, 0.035 * k, 0))
			_put(_box(0.21 * k, 0.015 * k, 0.105 * k), c2, n, b, Vector3(0, 0.06 * k, 0))
			_put(_box(0.04 * k, 0.12 * k, 0.01 * k), Color("4fe3ff"), n, b, Vector3(0.06 * k, 0.13 * k, 0.04 * k))
		"spire":
			for i in 5:
				var s := (0.18 - i * 0.032) * k
				_put(_box(s, 0.08 * k, s), c1, n, b, Vector3(0, (0.04 + i * 0.08) * k, 0))
				_put(_box(s + 0.005, 0.01 * k, s + 0.005), c2 if i % 2 == 0 else Color("ff4fd8"), n, b, Vector3(0, (0.08 + i * 0.08) * k, 0))
			_put(_cone(0.02 * k, 0.14 * k, 4), c2, n, b, Vector3(0, 0.47 * k, 0))
		"rocket":
			_put(_box(0.22 * k, 0.02 * k, 0.22 * k), c1, n, b, Vector3(0, 0.01 * k, 0))
			_put(_cylinder(0.035 * k, 0.2 * k, 8), c2, n, b, Vector3(0, 0.12 * k, 0))
			_put(_cone(0.035 * k, 0.07 * k, 8), Color("ff9d3b"), n, b, Vector3(0, 0.255 * k, 0))
			_put(_box(0.015 * k, 0.24 * k, 0.015 * k), c1, n, b, Vector3(0.07 * k, 0.12 * k, 0))
		"station":
			_put(_cylinder(0.04 * k, 0.24 * k, 8), c1, n, b, Vector3(0, 0.12 * k, 0))
			_put(_cylinder(0.14 * k, 0.015 * k, 12), c2, n, b, Vector3(0, 0.2 * k, 0))
			_put(_ball(0.05 * k), c1, n, b, Vector3(0, 0.24 * k, 0))
		_:
			_put(_box(0.1 * k, 0.08 * k, 0.1 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
