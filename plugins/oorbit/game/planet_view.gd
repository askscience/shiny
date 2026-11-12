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
var _props: Node3D
var _highlight: MeshInstance3D
var _biomes: Array = []
var _signature := ""
var _idle := IDLE_BEFORE_SPIN
var selected := -1
var _materials := {}


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
	_props = Node3D.new()
	pivot.add_child(_props)
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
	var signature := JSON.stringify([biomes, buildings])
	if signature == _signature:
		return
	_signature = signature
	if biomes != _biomes:
		_biomes = biomes
		var mesh := builder.build(biomes, Palette.BIOMES)
		_land.mesh = mesh
		_outline.mesh = mesh
	for child in _props.get_children():
		child.queue_free()
	var rng := RandomNumberGenerator.new()
	rng.seed = 11
	for i in biomes.size():
		_place_tile(i, biomes[i], buildings[i], rng)
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


func _mat(color: Color) -> StandardMaterial3D:
	var key := color.to_html()
	if not _materials.has(key):
		_materials[key] = _toon(color)
	return _materials[key]


static func _cone(radius: float, height: float, sides: int) -> CylinderMesh:
	var mesh := CylinderMesh.new()
	mesh.top_radius = 0.0
	mesh.bottom_radius = radius
	mesh.height = height
	mesh.radial_segments = sides
	mesh.rings = 1
	return mesh


static func _cylinder(radius: float, height: float, sides: int) -> CylinderMesh:
	var mesh := CylinderMesh.new()
	mesh.top_radius = radius
	mesh.bottom_radius = radius
	mesh.height = height
	mesh.radial_segments = sides
	mesh.rings = 1
	return mesh


static func _box(x: float, y: float, z: float) -> BoxMesh:
	var mesh := BoxMesh.new()
	mesh.size = Vector3(x, y, z)
	return mesh


func _tangents(normal: Vector3) -> Array:
	var up := Vector3.UP if abs(normal.y) < 0.9 else Vector3.RIGHT
	var u := normal.cross(up).normalized()
	return [u, normal.cross(u)]


## Place `mesh` on the tile: `offset` is (along u, up, along w) in the tile's
## frame, `turn` rotates it around the tile's up.
func _put(mesh: Mesh, color: Color, normal: Vector3, base: Vector3, offset: Vector3, size := 1.0, turn := 0.0) -> void:
	var t := _tangents(normal)
	var u: Vector3 = t[0]
	var w: Vector3 = t[1]
	var y := normal.normalized()
	var basis := Basis(u, y, w).rotated(y, turn).scaled(Vector3.ONE * size * _scale)
	var node := MeshInstance3D.new()
	node.mesh = mesh
	node.material_override = _mat(color)
	offset *= _scale
	node.transform = Transform3D(basis, base + u * offset.x + y * offset.y + w * offset.z)
	_props.add_child(node)


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
const BUILDING_SCALE := 1.6


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
