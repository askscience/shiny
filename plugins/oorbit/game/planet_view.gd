## The 3D planet drawn from the simulation's state: continuous ground whose
## biomes fade into each other, a small model for each building
## (scaffolding while it is being built), nature on the free cells, and the
## selected cell's outline. Everything stands on the ground (`ground()`),
## never above it. Turns by itself while the whole planet is in view, or by
## dragging; a click picks a cell; `focus()` flies the camera over one.
class_name PlanetView
extends Node3D

signal tile_picked(index: int)
## A crew brought something home: where (world space) and what ("+3 wood").
signal delivered(where: Vector3, text: String, color: Color)

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

## Little people, all in one MultiMesh (one draw call): crews walking
## between the village and the cell they work and back with their load,
## builders on building sites, and a few people strolling at home.
const MAX_WALKERS := 48
## Walking speed (planet radii per second) and time spent working a cell.
const WALK_SPEED := 0.05
const WORK_SECONDS := 5.0
## The planet's time now (ms) and how fast it runs: set by the window.
var clock := func() -> float: return 0.0
var time_rate := 1.0
## Crews' routes: per order, ground points from home to the cell.
var _routes := []
## Building sites growing: [node, footing, normal, start_ms, done_ms].
var _sites := []
const RESOURCE_COLORS := {
	"food": Color("e08f6c"), "wood": Color("8f6f5a"), "stone": Color("b0a6ad"),
	"clay": Color("c07a54"), "marble": Color("eae2d6"),
}
const WALKER_COLORS := [Color("f2c078"), Color("e08f6c"), Color("eef3f2"), Color("e3c3c6"), Color("c2d69b")]
var _walkers: MultiMeshInstance3D
var _walk := []  # per walker: a Dictionary (kind, route or cell, time...)
var _highlight: MeshInstance3D
## Roads and sea routes, drawn on the ground and the water; carts and boats
## going back and forth on the finished ones; a name over every city.
var _roads: MeshInstance3D
var _vehicles: MultiMeshInstance3D
var _rides := []  # per vehicle: {points, length, offset, boat}
var _links := []
var _road_built := []
var _cities := []
var _labels := []
const ROAD_COLOR := Color("b58e6b")
const ROAD_PLAN := Color("e3cfa0")
const SEA_COLOR := Color("dfe9ec")
const CART_COLOR := Color("8f6f5a")
const BOAT_COLOR := Color("eef3f2")
const CITY_COLORS := [Color("e08f6c"), Color("4fa3c7"), Color("c2d69b"), Color("e3c3c6"), Color("f2c078"), Color("9b8ec4")]
## Finds to pick up and the cell of a waiting event: a model on the
## ground and a bobbing marker over it.
var _marks := []  # [{node, tile, kind, base}]
const FIND_COLOR := Color("f2c078")
const EVENT_COLOR := Color("e08f6c")
var _biomes: Array = []
var _signature := ""
var _idle := IDLE_BEFORE_SPIN
var selected := -1
var _heights: Array = []
## Turning on its own: only while the whole planet is in view.
var spin_allowed := true
## Flying to a cell: the pivot turns from `_fly_from` to `_fly_to`.
var _fly_from := Quaternion.IDENTITY
var _fly_to := Quaternion.IDENTITY
var _fly_t := 1.0
const FLY_SECONDS := 0.7
var _focused_home := false
var _home := -1


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
	_roads = MeshInstance3D.new()
	var road_material := _toon(Color.WHITE, true)
	road_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_roads.material_override = road_material
	pivot.add_child(_roads)
	_vehicles = MultiMeshInstance3D.new()
	var vehicle_mesh := BoxMesh.new()
	vehicle_mesh.size = Vector3(0.045, 0.03, 0.03)
	var vehicles := MultiMesh.new()
	vehicles.transform_format = MultiMesh.TRANSFORM_3D
	vehicles.use_colors = true
	vehicles.mesh = vehicle_mesh
	_vehicles.multimesh = vehicles
	_vehicles.material_override = _toon(Color.WHITE, true)
	pivot.add_child(_vehicles)
	_highlight = MeshInstance3D.new()
	var line_material := StandardMaterial3D.new()
	line_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	line_material.albedo_color = Color("f2c078")
	line_material.no_depth_test = true
	_highlight.material_override = line_material
	pivot.add_child(_highlight)


func _process(delta: float) -> void:
	_idle += delta
	if _fly_t < 1.0:
		_fly_t = minf(1.0, _fly_t + delta / FLY_SECONDS)
		var t := _fly_t * _fly_t * (3.0 - 2.0 * _fly_t)
		pivot.quaternion = _fly_from.slerp(_fly_to, t)
	elif _idle > IDLE_BEFORE_SPIN and selected < 0 and spin_allowed:
		pivot.rotate(Vector3.UP, SPIN_SPEED * delta)
	_move_walkers(delta)


## Seconds since the window started: people walk on this clock, so placing
## them again never sends them back home.
var _seconds := 0.0
var _warned := false


func _move_walkers(delta: float) -> void:
	_seconds += delta
	var multimesh := _walkers.multimesh
	var size := _tile_size() * 1.2
	for k in _walk.size():
		var w: Dictionary = _walk[k]
		w["t"] = _seconds + float(w["offset"])
		var at := Vector3.ZERO
		var forward := Vector3.FORWARD
		var normal := Vector3.UP
		match w["kind"]:
			"crew":
				var route: Dictionary = _routes[w["route"]]
				var points: PackedVector3Array = route["points"]
				var walk := float(route["walk"])
				var period := walk * 2.0 + WORK_SECONDS
				var t := fmod(float(w["t"]), period)
				var along := 0.0
				if t < walk:
					along = t / walk
				elif t < walk + WORK_SECONDS:
					along = 1.0
				else:
					along = 1.0 - (t - walk - WORK_SECONDS) / walk
				# Home with the load: show what it brought.
				if t < float(w["last"]) and w.get("leader", false):
					var where: Vector3 = pivot.to_global(points[0])
					delivered.emit(where, route["text"], route["color"])
				w["last"] = t
				var f := along * (points.size() - 1)
				var i := mini(int(f), points.size() - 2)
				at = points[i].lerp(points[i + 1], f - i)
				forward = (points[i + 1] - points[i]).normalized() * (1.0 if t < walk else -1.0)
				normal = at.normalized()
				if t >= walk and t < walk + WORK_SECONDS:
					# Working: a little bob on the spot.
					at += normal * absf(sin(t * 12.0)) * 0.003
					at += forward.cross(normal).normalized() * sin(float(w["phase"])) * 0.01
				multimesh.set_instance_color(k, route["color"] if t >= walk + WORK_SECONDS else WALKER_COLORS[k % WALKER_COLORS.size()])
			_:
				# Strolling or building: a slow loop around a cell.
				var cell: int = w["cell"]
				normal = builder.centers[cell]
				var tg := _tangents(normal)
				var angle := float(w["phase"]) + float(w["t"]) * float(w["speed"])
				var r := float(w["radius"])
				at = builder.ground(normal + (tg[0] * cos(angle) + tg[1] * sin(angle)) * r, cell)
				forward = (-tg[0] * sin(angle) + tg[1] * cos(angle)) * signf(float(w["speed"]))
				if w["kind"] == "builder":
					at += normal * absf(sin(float(w["t"]) * 10.0)) * 0.004
		at += normal * (0.026 * size + absf(sin(float(w["t"]) * 9.0)) * 0.002)
		var side := forward.cross(normal)
		if side.length_squared() < 1e-8:
			side = _tangents(normal)[0]
		side = side.normalized()
		var basis := Basis(side, normal, side.cross(normal)).scaled(Vector3.ONE * size)
		if not (at.length() < 1.3) or not basis.is_finite():
			# Never draw anyone off the planet.
			if not _warned:
				_warned = true
				push_warning("walker off the planet: %s %s at %s" % [w["kind"], w.get("cell", w.get("route", "")), at])
			basis = Basis().scaled(Vector3.ZERO)
			at = Vector3.ZERO
		multimesh.set_instance_transform(k, Transform3D(basis, at))
	_move_vehicles()
	_bob_marks()
	# Roads grow cell by cell.
	for k in mini(_links.size(), _road_built.size()):
		if _built_cells(_links[k]) != int(_road_built[k]):
			_build_roads()
			break
	# Building sites rise with their progress.
	var now: float = clock.call()
	for site in _sites:
		var node: MeshInstance3D = site[0]
		var p := clampf((now - float(site[3])) / maxf(1.0, float(site[4]) - float(site[3])), 0.05, 1.0)
		var n: Vector3 = site[2]
		var f: Vector3 = site[1]
		# Scale along the ground's up through the footing.
		var b := Basis.IDENTITY
		b.x += n * n.x * (p - 1.0)
		b.y += n * n.y * (p - 1.0)
		b.z += n * n.z * (p - 1.0)
		node.transform = Transform3D(b, f - b * f)


## Props shrink on denser planets (more, smaller cells); they were drawn for
## a 32-tile planet.
func _tile_size() -> float:
	return sqrt(32.0 / maxf(1.0, float(builder.centers.size())))


func set_rim(on: bool) -> void:
	_outline.visible = on


## Take the cell layout (once, before the first planet).
func set_layout(data: Dictionary) -> void:
	builder.load_layout(data)
	_ts = _tile_size()
	_signature = ""
	_biomes = []


var _ts := 1.0


## Ground points along the way from one cell to another (local space).
func _route(from: int, to: int) -> PackedVector3Array:
	var a: Vector3 = builder.centers[from]
	var b: Vector3 = builder.centers[to]
	var steps := maxi(4, int(a.angle_to(b) / 0.02))
	var points := PackedVector3Array()
	for i in steps + 1:
		var d := a.slerp(b, float(i) / steps)
		var g := builder.ground(d)
		points.append(g.normalized() * maxf(g.length(), PlanetMesh.SEA_LEVEL + 0.001))
	return points


## Place the people: crews on their routes, builders on sites, a few
## strolling at home.
func _place_walkers(planet: Dictionary, rates: Dictionary) -> void:
	_walk.clear()
	_routes.clear()
	var rng := RandomNumberGenerator.new()
	rng.seed = 23
	var orders: Array = planet.get("orders", [])
	var outputs: Array = rates.get("orders", [])
	var population := float(planet.get("population", 0))
	var crew_size := clampi(roundi(population / 16.0) + 1, 2, 4)
	for k in orders.size():
		var cell := int(orders[k]["tile"])
		if cell < 0 or cell >= builder.centers.size():
			continue
		var points := _route(_city_of(cell), cell)
		var length := 0.0
		for i in points.size() - 1:
			length += points[i].distance_to(points[i + 1])
		var walk := maxf(1.5, length / WALK_SPEED)
		var resource: String = orders[k]["resource"]
		# What one trip brings: the crew's planet-hour output over a trip.
		var trip_hours := (walk * 2.0 + WORK_SECONDS) * time_rate / 3600.0
		var amount := (float(outputs[k]) if k < outputs.size() else 0.0) * trip_hours
		var text := "+%s %s" % [_short(amount), I18n.name_of("resource", resource).to_lower()]
		_routes.append({"points": points, "walk": walk, "text": text, "resource": resource, "color": RESOURCE_COLORS.get(resource, Color("f2c078"))})
		for m in crew_size:
			var period := walk * 2.0 + WORK_SECONDS
			# The crew walks together, a step apart; the first one tells
			# what they brought.
			var offset := m * 0.35 + float(k) * 1.7
			_walk.append({"kind": "crew", "route": _routes.size() - 1, "offset": offset, "t": 0.0, "last": fmod(_seconds + offset, period), "phase": rng.randf() * TAU, "leader": m == 0})
	for c in planet.get("construction", []):
		for m in 2:
			_walk.append({"kind": "builder", "cell": int(c["tile"]), "offset": 0.0, "t": 0.0, "phase": m * PI + rng.randf(), "speed": 0.6, "radius": 0.035 * _tile_size()})
	var strollers := clampi(roundi(population / 10.0), 2, 8)
	var homes := [_home]
	for c in _cities:
		if int(c["tile"]) != _home:
			homes.append(int(c["tile"]))
	for m in strollers + homes.size() - 1:
		var speed := (0.12 + rng.randf() * 0.2) * (1.0 if rng.randf() < 0.5 else -1.0)
		# The capital gets most of the strollers, every colony one.
		var at: int = homes[m - strollers + 1] if m >= strollers else _home
		_walk.append({"kind": "stroll", "cell": at, "offset": 0.0, "t": 0.0, "phase": rng.randf() * TAU, "speed": speed, "radius": (0.1 + rng.randf() * 0.12) * _tile_size()})
	# Road builders at the end of the road being laid.
	for l in _links:
		if l["kind"] != "road" or _link_progress(l) >= 1.0:
			continue
		var cells: Array = l["cells"]
		var at := int(cells[mini(cells.size() - 1, int(_link_progress(l) * (cells.size() - 1)) + 1)])
		for m in 2:
			_walk.append({"kind": "builder", "cell": at, "offset": 0.0, "t": 0.0, "phase": m * PI + rng.randf(), "speed": 0.6, "radius": 0.05 * _tile_size()})
	if _walk.size() > MAX_WALKERS:
		_walk.resize(MAX_WALKERS)
	var multimesh := _walkers.multimesh
	multimesh.instance_count = _walk.size()
	for k in _walk.size():
		multimesh.set_instance_color(k, WALKER_COLORS[k % WALKER_COLORS.size()])
	_move_walkers(0.0)


static func _short(v: float) -> String:
	if v >= 10.0:
		return str(roundi(v))
	return "%.1f" % v


## Rebuild what changed in the planet (`planet` is the simulation's JSON).
func show_planet(planet: Dictionary, rates := {}) -> void:
	var tiles: Array = planet.get("tiles", [])
	if not builder.ready() or tiles.size() != builder.centers.size():
		return
	_home = int(planet.get("home", 0))
	_cities = planet.get("cities", [])
	_links = planet.get("links", [])
	var building_at := {}
	for c in planet.get("construction", []):
		building_at[int(c["tile"])] = "construction:" + str(c["building"])
	var biomes := []
	var heights := []
	var buildings := []
	for i in tiles.size():
		biomes.append(str(tiles[i]["biome"]))
		heights.append(float(tiles[i].get("height", 1.0)))
		var b = tiles[i].get("building")
		buildings.append(str(b) if b != null else building_at.get(i, ""))
	var stocks := []
	for t in tiles:
		stocks.append(roundi(float(t.get("stock", 1.0)) * 4.0))
	var dug := []
	for t in tiles:
		dug.append(float(t.get("deposit", 1.0)) == 0.0 and str(t["biome"]) in ["mountain", "plains", "desert", "coast"])
	# People move on every view; the props only when something they show
	# changed.
	_home = int(planet.get("home", 0))
	_place_walkers(planet, rates)
	_place_rides()
	var finds: Array = planet.get("finds", [])
	var event = planet.get("pending_event")
	var event_tile := int(event["tile"]) if event != null and event.get("tile") != null else -1
	_place_marks(finds, event_tile)
	_finds_at = {}
	for f in finds:
		_finds_at[int(f["tile"])] = str(f["kind"])
	var signature := JSON.stringify([biomes, buildings, planet.get("orders", []), stocks, _cities, dug, _finds_at])
	_build_roads()
	if signature == _signature:
		return
	_signature = signature
	if biomes != _biomes or heights != _heights:
		_biomes = biomes
		_heights = heights
		_land.mesh = builder.build(biomes, heights, Palette.BIOMES)
		# The rim only needs the shape: the coarse surface is enough.
		_outline.mesh = builder.build(biomes, heights, Palette.BIOMES, 1)
		_ts = _tile_size()
	if not _focused_home:
		_focused_home = true
		focus(_home, true)
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	var rng := RandomNumberGenerator.new()
	rng.seed = 11
	_stocks = stocks
	_dug = dug
	_ordered = {}
	for o in planet.get("orders", []):
		_ordered[int(o["tile"])] = str(o["resource"])
	for site in _sites:
		site[0].queue_free()
	_sites.clear()
	var step_ms := 600000.0
	for c in planet.get("construction", []):
		_site_times[int(c["tile"])] = [float(c.get("start_step", 0)) * step_ms, float(c["done_step"]) * step_ms]
	for i in biomes.size():
		_place_tile(i, biomes[i], buildings[i], rng)
	_site_times.clear()
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
	_place_labels()
	_update_highlight()


## Rotate by a drag of `relative` pixels (in the low-resolution view).
func drag(relative: Vector2) -> void:
	_idle = 0.0
	_fly_t = 1.0
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
	var to_local := pivot.global_transform.affine_inverse()
	var local := (to_local * hit).normalized()
	# Refine against the real ground: hills and peaks stand above the sphere.
	var local_origin := to_local * origin
	var local_dir := (to_local.basis * direction).normalized()
	for k in 3:
		var r := builder.ground(local).length()
		var lb := local_origin.dot(local_dir)
		var lc := local_origin.dot(local_origin) - r * r
		var ld := lb * lb - lc
		if ld < 0.0:
			break
		local = (local_origin + local_dir * (-lb - sqrt(ld))).normalized()
	return builder.cell_at(local)


## Turn the planet so `cell` faces the camera (at once with `instant`).
func focus(cell: int, instant := false) -> void:
	if cell < 0 or cell >= builder.centers.size():
		return
	var current := pivot.quaternion
	var world := current * builder.centers[cell]
	var turn := Quaternion(world.normalized(), Vector3.BACK)
	_fly_from = current
	_fly_to = (turn * current).normalized()
	_fly_t = 1.0 if instant else 0.0
	if instant:
		pivot.quaternion = _fly_to
	_idle = 0.0


func select(index: int) -> void:
	selected = index
	_idle = 0.0
	_update_highlight()


func _update_highlight() -> void:
	if selected < 0 or selected >= builder.corner_points.size() or not builder.ready() or builder.center_points.is_empty():
		_highlight.mesh = null
		return
	var ring := builder.outline(selected)
	var mesh := ImmediateMesh.new()
	mesh.surface_begin(Mesh.PRIMITIVE_LINE_STRIP)
	for k in ring.size() + 1:
		mesh.surface_add_vertex(ring[k % ring.size()])
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
	var at := base + u * offset.x + w * offset.z
	if _on_ground:
		# Nature stands wherever it is: on the ground at that very spot,
		# sunk a little so slopes never leave a gap underneath.
		at = builder.ground(at, _cell) - y * 0.004 * _ts
	var xform := Transform3D(basis, at + y * offset.y)
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


## Nature follows the ground prop by prop; buildings stand on one footing.
var _on_ground := false
var _cell := -1


func _place_tile(i: int, biome: String, building: String, rng: RandomNumberGenerator) -> void:
	var normal: Vector3 = builder.centers[i]
	_cell = i
	var base := builder.ground(normal, i)
	if biome == "ocean":
		return
	if building == "":
		_flag(i, normal, base)
		_on_ground = true
		# Worked land thins out: fewer trees and rocks while it is spent.
		var stock := float(_stocks[i]) / 4.0 if i < _stocks.size() else 1.0
		if i < _dug.size() and _dug[i]:
			_quarry(normal, base, rng)
		else:
			_nature(normal, base, biome, rng, maxi(1, roundi(6.0 * (0.25 + 0.75 * stock))))
		if biome == "forest" and stock < 0.75:
			# Stumps where the trees were cut.
			for k in roundi((1.0 - stock) * 7.0):
				var a := rng.randf() * TAU
				var r := 0.05 + rng.randf() * 0.18
				_put(_cylinder(0.014, 0.022, 5), Color("8f6f5a"), normal, base, Vector3(cos(a) * r, 0.008, sin(a) * r))
		if _finds_at.has(i):
			_find_model(_finds_at[i], normal, base, rng)
		if _ordered.has(i):
			# A stake with a little flag the colour of what is gathered.
			_put(_box(0.012, 0.16, 0.012), Palette.HUT_WALL, normal, base, Vector3(0, 0.08, 0))
			_put(_box(0.06, 0.035, 0.008), RESOURCE_COLORS.get(_ordered[i], Color("f2c078")), normal, base, Vector3(0.03, 0.14, 0))
		_on_ground = false
		return
	# The footing: the lowest ground under the building, with a plinth
	# below it so a slope never shows a gap.
	var footing := base
	var t := _tangents(normal)
	for k in 6:
		var a := k * TAU / 6.0
		var p := builder.ground(base + (t[0] * cos(a) + t[1] * sin(a)) * 0.05 * _ts, i)
		if p.length() < footing.length():
			footing = p
	footing = footing.normalized() * (footing.length() - 0.002)
	_put(_box(0.26, 0.08, 0.26), _plinth(biome), normal, footing, Vector3(0, -0.035, 0))
	if building.begins_with("construction:"):
		_scaffold(normal, footing)
		_grow_site(i, building.substr(13), normal, footing)
	else:
		_building(building, normal, footing)
	_flag(i, normal, footing)
	_on_ground = true
	_nature(normal, base, biome, rng, 2)
	_on_ground = false


## A worked-out quarry: a dark pit with spoil heaps around it.
func _quarry(normal: Vector3, base: Vector3, rng: RandomNumberGenerator) -> void:
	_put(_cylinder(0.11, 0.012, 7), Color("4a4152"), normal, base, Vector3(0, 0.002, 0))
	_put(_cylinder(0.075, 0.014, 7), Color("2f2a38"), normal, base, Vector3(0, 0.004, 0))
	for k in 4:
		var a := k * TAU / 4.0 + rng.randf()
		_put(_cone(0.04, 0.03, 5), Color("a39aa8"), normal, base, Vector3(cos(a) * 0.15, 0.012, sin(a) * 0.15), 0.8 + rng.randf() * 0.5)


## The plinth under a building: packed earth, sand or stone.
static func _plinth(biome: String) -> Color:
	match biome:
		"desert", "coast":
			return Color("b58e6b")
		"mountain", "ice":
			return Color("857c8a")
	return Color("8f6f5a")


## Buildings are drawn larger than life, so they read at the pixel scale.
const BUILDING_SCALE := 1.15


## Trees, peaks or ice around the tile's edge (`amount` trees on a forest).
func _nature(normal: Vector3, base: Vector3, biome: String, rng: RandomNumberGenerator, amount: int) -> void:
	match biome:
		"forest":
			for k in amount * 2:
				var angle := k * TAU / (amount * 2) + rng.randf() * 0.6
				var r := (0.04 + rng.randf() * 0.2) if amount > 2 else 0.2 + rng.randf() * 0.06
				var s := 0.7 + rng.randf() * 0.6
				_put(_cone(0.06, 0.18, 5), Palette.TREE, normal, base,
					Vector3(cos(angle) * r, 0.09 * s, sin(angle) * r), s)
		"mountain":
			# The peak is the ground itself; a few boulders on its flanks.
			for k in mini(amount, 3):
				var angle := k * TAU / 3.0 + rng.randf()
				var r := 0.12 + rng.randf() * 0.1
				_put(_box(0.05, 0.04, 0.045), Palette.PEAK, normal, base,
					Vector3(cos(angle) * r, 0.012, sin(angle) * r), 0.7 + rng.randf() * 0.6, rng.randf() * TAU)
		"plains":
			for k in mini(amount, 2):
				var angle := rng.randf() * TAU
				var r := 0.08 + rng.randf() * 0.14
				_put(_ball(0.035), Color("6f9163"), normal, base,
					Vector3(cos(angle) * r, 0.015, sin(angle) * r), 0.8 + rng.randf() * 0.5)
		"ice":
			for k in mini(amount, 3):
				var angle := k * TAU / 3.0 + rng.randf()
				_put(_cone(0.08, 0.06, 5), Palette.SNOW, normal, base,
					Vector3(cos(angle) * 0.14, 0.02, sin(angle) * 0.14), 0.8 + rng.randf() * 0.4)


var _stocks := []
var _finds_at := {}
var _dug := []
var _ordered := {}
var _site_times := {}


## The building going up on a site: its own small mesh, scaled up from the
## footing as the work goes on (see `_move_walkers`).
func _grow_site(cell: int, id: String, normal: Vector3, footing: Vector3) -> void:
	var saved := [_verts, _normals, _colors, _indices]
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	_building(id, normal, footing)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = _verts
	arrays[Mesh.ARRAY_NORMAL] = _normals
	arrays[Mesh.ARRAY_COLOR] = _colors
	arrays[Mesh.ARRAY_INDEX] = _indices
	_verts = saved[0]
	_normals = saved[1]
	_colors = saved[2]
	_indices = saved[3]
	if arrays[Mesh.ARRAY_VERTEX].is_empty():
		return
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var node := MeshInstance3D.new()
	node.mesh = mesh
	node.material_override = _props.material_override
	pivot.add_child(node)
	var times: Array = _site_times.get(cell, [0.0, 1.0])
	_sites.append([node, footing, normal.normalized(), times[0], times[1]])


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
		"colony":
			# A longhouse inside a palisade.
			_put(_box(0.16, 0.07, 0.09), Color("d9b48f"), n, b, Vector3(0, 0.035, 0))
			_put(_cone(0.12, 0.07, 4), roof, n, b, Vector3(0, 0.105, 0), 1.0, PI / 4.0)
			for k in 12:
				var a := k * TAU / 12.0
				if k == 3:
					continue
				_put(_box(0.016, 0.06, 0.016), Color("8f6f5a"), n, b, Vector3(cos(a) * 0.15, 0.03, sin(a) * 0.15))
		"jetty":
			_put(_box(0.26, 0.02, 0.06), Color("8f6f5a"), n, b, Vector3(0.04, 0.014, 0))
			for k in 3:
				_put(_box(0.014, 0.04, 0.014), Color("6b5a4e"), n, b, Vector3(-0.04 + k * 0.08, 0.0, 0.035))
			_put(_box(0.08, 0.025, 0.035), BOAT_COLOR, n, b, Vector3(0.1, 0.012, 0.075))
			_put(_cone(0.025, 0.06, 3), Color("e3cfa0"), n, b, Vector3(0.1, 0.055, 0.075))
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


# ---------------------------------------------------------------- cities

## The city a cell's crews come from: the nearest centre.
func _city_of(cell: int) -> int:
	var best := _home
	var best_dot := -2.0
	var at: Vector3 = builder.centers[cell]
	for c in _cities:
		var t := int(c["tile"])
		var d := at.dot(builder.centers[t])
		if d > best_dot:
			best_dot = d
			best = t
	return best


func _city_index(cell: int) -> int:
	for k in _cities.size():
		if int(_cities[k]["tile"]) == cell:
			return k
	return -1


## A city's banner: a pole with a flag in the city's colour.
func _flag(cell: int, normal: Vector3, base: Vector3) -> void:
	var k := _city_index(cell)
	if k < 0:
		return
	var saved := _on_ground
	_on_ground = false
	_put(_box(0.012, 0.24, 0.012), Palette.HUT_WALL, normal, base, Vector3(-0.12, 0.12, -0.06))
	_put(_box(0.08, 0.045, 0.008), CITY_COLORS[k % CITY_COLORS.size()], normal, base, Vector3(-0.08, 0.215, -0.06))
	_on_ground = saved


## How far along a way is now (0 to 1), on the planet's clock.
func _link_progress(l: Dictionary) -> float:
	var step_ms := 600000.0
	var start := float(l.get("start_step", 0)) * step_ms
	var done := float(l.get("done_step", 0)) * step_ms
	if done <= start:
		return 1.0
	return clampf((float(clock.call()) - start) / (done - start), 0.0, 1.0)


func _built_cells(l: Dictionary) -> int:
	var cells: Array = l["cells"]
	return int(floor(_link_progress(l) * (cells.size() - 1)))


## Ground points along a way's cells, one after the other.
func _way_points(l: Dictionary) -> PackedVector3Array:
	var cells: Array = l["cells"]
	var points := PackedVector3Array()
	for i in cells.size() - 1:
		var piece := _route(int(cells[i]), int(cells[i + 1]))
		if i > 0:
			piece.remove_at(0)
		points.append_array(piece)
	return points


## Roads (laid cells solid, the rest a pale dotted plan) and sea routes
## (dotted on the water), in one mesh.
func _build_roads() -> void:
	var saved := [_verts, _normals, _colors, _indices]
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	_road_built.clear()
	for l in _links:
		var cells: Array = l["cells"]
		var built := _built_cells(l)
		_road_built.append(built)
		var road: bool = l["kind"] == "road"
		var done := _link_progress(l) >= 1.0
		for i in cells.size() - 1:
			var piece := _route(int(cells[i]), int(cells[i + 1]))
			if road:
				if i < built:
					_strip(piece, 0.022 * _ts, ROAD_COLOR, 1)
				else:
					_strip(piece, 0.014 * _ts, ROAD_PLAN, 2)
			else:
				_strip(piece, 0.012 * _ts, SEA_COLOR if done else ROAD_PLAN, 2 if done else 3)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = _verts
	arrays[Mesh.ARRAY_NORMAL] = _normals
	arrays[Mesh.ARRAY_COLOR] = _colors
	arrays[Mesh.ARRAY_INDEX] = _indices
	var mesh := ArrayMesh.new()
	if _verts.size() > 0:
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	_roads.mesh = mesh
	_verts = saved[0]
	_normals = saved[1]
	_colors = saved[2]
	_indices = saved[3]


## A flat band along `points`, a little above the ground; with `every` > 1
## only one piece in `every` is drawn (dotted).
func _strip(points: PackedVector3Array, width: float, color: Color, every: int) -> void:
	for i in points.size() - 1:
		if i % every != 0:
			continue
		var a := points[i]
		var b := points[i + 1]
		var n := a.normalized()
		var side := (b - a).cross(n).normalized() * width * 0.5
		var lift := n * 0.0025
		var start := _verts.size()
		for p in [a - side, a + side, b + side, b - side]:
			_verts.append(p + lift)
			_normals.append(n)
			_colors.append(color)
		for k in [0, 2, 1, 0, 3, 2]:
			_indices.append(start + k)


## Carts on the finished roads, boats on the finished sea routes.
func _place_rides() -> void:
	_rides.clear()
	for k in _links.size():
		var l: Dictionary = _links[k]
		if _link_progress(l) < 1.0:
			continue
		var points := _way_points(l)
		if points.size() < 2:
			continue
		var length := 0.0
		for i in points.size() - 1:
			length += points[i].distance_to(points[i + 1])
		var boat: bool = l["kind"] == "sea"
		for m in (1 if boat else 2):
			_rides.append({"points": points, "length": length, "offset": m * 0.5 + k * 0.37, "boat": boat})
	var multimesh := _vehicles.multimesh
	multimesh.instance_count = _rides.size()
	for k in _rides.size():
		multimesh.set_instance_color(k, BOAT_COLOR if _rides[k]["boat"] else CART_COLOR)
	_move_vehicles()


func _move_vehicles() -> void:
	var multimesh := _vehicles.multimesh
	var size := _tile_size() * 1.2
	for k in _rides.size():
		var r: Dictionary = _rides[k]
		var points: PackedVector3Array = r["points"]
		var trip := float(r["length"]) / (WALK_SPEED * 1.4)
		var t := fmod(_seconds / trip + float(r["offset"]), 2.0)
		var along := t if t < 1.0 else 2.0 - t
		var f := along * (points.size() - 1)
		var i := mini(int(f), points.size() - 2)
		var at := points[i].lerp(points[i + 1], f - i)
		var normal := at.normalized()
		var forward := (points[i + 1] - points[i]).normalized()
		var side := forward.cross(normal)
		if side.length_squared() < 1e-8:
			side = _tangents(normal)[0]
		side = side.normalized()
		var basis := Basis(side.cross(normal), normal, -side).scaled(Vector3.ONE * size)
		at += normal * 0.018 * size
		multimesh.set_instance_transform(k, Transform3D(basis, at))


## City names over their centres.
func _place_labels() -> void:
	for label in _labels:
		label.queue_free()
	_labels.clear()
	for k in _cities.size():
		var c: Dictionary = _cities[k]
		var cell := int(c["tile"])
		if cell < 0 or cell >= builder.centers.size():
			continue
		var label := Label3D.new()
		label.text = str(c["name"])
		label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		label.fixed_size = true
		label.pixel_size = 0.0009
		label.font_size = 28
		label.outline_size = 8
		label.modulate = CITY_COLORS[k % CITY_COLORS.size()].lightened(0.35)
		label.outline_modulate = Palette.OUTLINE
		label.texture_filter = BaseMaterial3D.TEXTURE_FILTER_NEAREST
		var n: Vector3 = builder.centers[cell]
		label.position = builder.ground(n, cell) + n.normalized() * 0.2 * _ts
		pivot.add_child(label)
		_labels.append(label)


# ---------------------------------------------------------------- finds and marks

## What a find looks like on the ground.
func _find_model(kind: String, normal: Vector3, base: Vector3, rng: RandomNumberGenerator) -> void:
	match kind:
		"berries":
			for k in 3:
				var a := k * TAU / 3.0
				_put(_ball(0.04), Color("4f6b4a"), normal, base, Vector3(cos(a) * 0.05, 0.0, sin(a) * 0.05))
				_put(_ball(0.012), Color("b03a2e"), normal, base, Vector3(cos(a) * 0.05, 0.035, sin(a) * 0.05))
		"goats", "horses":
			var coat := Color("eef3f2") if kind == "goats" else Color("8f6f5a")
			for k in 3:
				var a := rng.randf() * TAU
				var at := Vector3(cos(a) * 0.08, 0.02, sin(a) * 0.08)
				_put(_box(0.05, 0.03, 0.025), coat, normal, base, at, 1.0, a)
				_put(_box(0.02, 0.02, 0.02), coat.darkened(0.2), normal, base, at + Vector3(cos(a) * 0.03, 0.015, -sin(a) * 0.03), 1.0, a)
		"crate":
			_put(_box(0.06, 0.05, 0.06), Color("b58e6b"), normal, base, Vector3(0, 0.025, 0), 1.0, 0.4)
			_put(_box(0.065, 0.01, 0.065), Color("6b5a4e"), normal, base, Vector3(0, 0.04, 0), 1.0, 0.4)
		"meteorite":
			_put(_ball(0.05), Color("4a4152"), normal, base, Vector3(0, -0.01, 0))
			_put(_ball(0.025), Color("ff9d3b"), normal, base, Vector3(0.02, 0.02, 0.01))
			_put(_cylinder(0.1, 0.006, 8), Color("5b5466"), normal, base, Vector3(0, 0.0, 0))
		"wreck":
			_put(_box(0.16, 0.035, 0.05), Color("6b5a4e"), normal, base, Vector3(0, 0.012, 0), 1.0, 0.5)
			_put(_box(0.01, 0.1, 0.01), Color("8f6f5a"), normal, base, Vector3(0.02, 0.05, 0.01))
		"ruins":
			for k in 4:
				var a := k * TAU / 4.0
				_put(_box(0.025, 0.04 + 0.03 * (k % 2), 0.025), Color("b0a6ad"), normal, base, Vector3(cos(a) * 0.07, 0.025, sin(a) * 0.07))
			_put(_box(0.1, 0.012, 0.03), Color("b0a6ad"), normal, base, Vector3(0, 0.002, 0), 1.0, 0.3)


func _marker_mesh(color: Color) -> MeshInstance3D:
	var mesh := SphereMesh.new()
	mesh.radius = 0.018
	mesh.height = 0.05
	mesh.radial_segments = 4
	mesh.rings = 2
	var node := MeshInstance3D.new()
	node.mesh = mesh
	var m := StandardMaterial3D.new()
	m.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	m.albedo_color = color
	node.material_override = m
	return node


## One bobbing marker per find, and one over the waiting event's cell.
func _place_marks(finds: Array, event_tile: int) -> void:
	for mark in _marks:
		mark["node"].queue_free()
	_marks.clear()
	var wanted := []
	for f in finds:
		wanted.append([int(f["tile"]), "find:" + str(f["kind"]), FIND_COLOR])
	if event_tile >= 0:
		wanted.append([event_tile, "event", EVENT_COLOR])
	for w in wanted:
		var cell: int = w[0]
		if cell < 0 or cell >= builder.centers.size():
			continue
		var node := _marker_mesh(w[2])
		node.scale = Vector3.ONE * _tile_size() * 5.0
		pivot.add_child(node)
		var n: Vector3 = builder.centers[cell]
		_marks.append({"node": node, "tile": cell, "kind": w[1], "base": builder.ground(n, cell) + n.normalized() * 0.09 * _tile_size()})
	_bob_marks()


func _bob_marks() -> void:
	for k in _marks.size():
		var mark: Dictionary = _marks[k]
		var base: Vector3 = mark["base"]
		var node: MeshInstance3D = mark["node"]
		node.position = base + base.normalized() * (0.012 * _tile_size() * sin(_seconds * 3.0 + k))
		node.rotation = Vector3(0.0, _seconds * 2.0, 0.0)


## Where the marks are (world space), for the window's edge markers.
func marks() -> Array:
	var out := []
	for mark in _marks:
		out.append({"tile": mark["tile"], "kind": mark["kind"], "at": pivot.to_global(mark["base"]), "normal": (pivot.global_transform.basis * (mark["base"] as Vector3)).normalized()})
	return out
