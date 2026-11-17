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
## A small sound where something happens (world space): an axe, a splash,
## birds taking off.
signal sfx(name: String, where: Vector3)

const SPIN_SPEED := 0.08          # radians per second, while idle
const IDLE_BEFORE_SPIN := 6.0     # seconds after the last touch

var pivot: Node3D
var builder := PlanetMesh.new()
var _land: MeshInstance3D
var _outline: MeshInstance3D
## Every prop (trees, peaks, buildings) merged into a few meshes with
## vertex colours: one patch of the planet per mesh (see `_chunks`), in two
## levels of detail, so the far side of the planet and the details too
## small to see cost nothing.
var _props_material: ShaderMaterial
var _land_material: ShaderMaterial
var _verts := PackedVector3Array()
var _normals := PackedVector3Array()
var _colors := PackedColorArray()
var _indices := PackedInt32Array()
var _far_v := PackedVector3Array()
var _far_n := PackedVector3Array()
var _far_c := PackedColorArray()
var _far_i := PackedInt32Array()
## Building the planet's props: also fill the far arrays.
var _lod := false
## The highest prop point (squared distance from the centre) in the patch.
var _top := 0.0
static var _shapes := {}

## Patches of the planet (`CHUNKS` of them, neighbouring cells together),
## each with its props near and far: {cells, dir, reach (radians), top,
## near, far}. `_cull` shows one level of each patch on this side of the
## horizon and hides the rest. Not many more: with about fifty meshes the
## Compatibility renderer in the browser (SwiftShader at least) drew the
## clouds' and particles' MultiMesh instances with garbage transforms.
const CHUNKS := 12
var _chunks := []
var _chunk_of := PackedInt32Array()
## A shape smaller than this (planet radii) is left out of the far view.
const FAR_MIN := 0.006
## The near level shows once a detail this big (planet radii) covers this
## many pixels of the low-resolution view.
const DETAIL_SIZE := 0.005
const DETAIL_PIXELS := 1.2
## Lowest ground (the sea), for the horizon.
const GROUND_LOW := 0.985
var view_height := 300.0
var _culled_from := Vector3.INF

## Little people, all in one MultiMesh (one draw call): crews walking
## between the village and the cell they work and back with their load,
## builders on building sites, and a few people strolling at home.
const MAX_WALKERS := 64
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
var _walk := []  # per walker: a Dictionary (kind, route or cell, time...)
var _highlight: MeshInstance3D
## Roads and sea routes, drawn on the ground and the water; carts and boats
## going back and forth on the finished ones; a name over every city.
var _roads: MeshInstance3D
var _vehicles: MultiMeshInstance3D
var _boats: MultiMeshInstance3D
var _fishers := []  # per fishing crew: the ocean cell its boat bobs around
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
var weather: Weather
## Events that come as a storm rolling in (drawn by `Weather`).
const STORMY := ["storm", "lightning"]
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
	_land_material = _planet_material(0.6, 0.0)
	_land_material.set_shader_parameter("ground", true)
	_land.material_override = _land_material
	pivot.add_child(_land)
	_outline = MeshInstance3D.new()
	_outline.scale = Vector3.ONE * 1.025
	var outline_material := StandardMaterial3D.new()
	outline_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	outline_material.cull_mode = BaseMaterial3D.CULL_FRONT
	outline_material.albedo_color = Palette.OUTLINE
	_outline.material_override = outline_material
	pivot.add_child(_outline)
	_props_material = _planet_material(1.0, 1.0)
	_roads = MeshInstance3D.new()
	var road_material := _toon(Color.WHITE, true)
	road_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_roads.material_override = road_material
	pivot.add_child(_roads)
	# Carts, boats, people, animals and birds: the Blender models as
	# MultiMeshes drawn by creature.gdshader (tints and walking legs).
	_vehicles = _creature_set(_creature_mesh("cart"), MAX_RIDES)
	_boats = _creature_set(_creature_mesh("boat"), MAX_RIDES)
	_build_walkers()
	for species in SPECIES_TINTS:
		_animal_sets[species] = _creature_set(_creature_mesh(species), MAX_ANIMALS + (MAX_QUARRIES if species == QUARRY else 0))
	_build_birds_and_fx()
	weather = Weather.new()
	pivot.add_child(weather)
	weather.setup(builder)
	weather.thunder.connect(func(where: Vector3, power: float): sfx.emit("thunder", where))
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


var _tick := 0


## Someone beyond the horizon: not drawn.
func _hide_walker(mm: MultiMesh, index: int) -> void:
	for label in _name_labels.values():
		if label.get_meta("index", -1) == index and label.get_meta("mm") == mm:
			label.visible = false
	if index < MAX_WALKERS:
		mm.set_instance_transform(index, Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO))


## Seconds since the window started: people walk on this clock, so placing
## them again never sends them back home.
var _seconds := 0.0
var _warned := false


func _move_walkers(delta: float) -> void:
	_seconds += delta
	_tick += 1
	_people.clear()
	_fx_static.clear()
	var size := _tile_size() * 1.2
	# From afar people are a few pixels tall: each moves every other frame.
	var halves := _closeness < 0.25
	for k in _walk.size():
		var w: Dictionary = _walk[k]
		if halves and (k + _tick) % 2 == 1 and w.has("at"):
			if w["at"] != Vector3.ZERO:
				_people.append(w["at"])
			continue
		var multimesh := _walker_mm(w)
		var index := int(w.get("index", 0))
		w["t"] = _seconds + float(w["offset"])
		var at := Vector3.ZERO
		var forward := Vector3.FORWARD
		var normal := Vector3.UP
		var tilt := 0.0
		var crouch := 1.0
		var hide := false
		var tint: Color = WALKER_COLORS[k % WALKER_COLORS.size()]
		var motion := 0.0
		match w["kind"]:
			"crew":
				var route: Dictionary = _routes[w["route"]]
				var points: PackedVector3Array = route["points"]
				var walk := float(route["walk"])
				var work := float(route["work"])
				var period := walk * 2.0 + work
				var t := fmod(float(w["t"]), period)
				var along := 0.0
				if t < walk:
					along = t / walk
				elif t < walk + work:
					along = 1.0
				else:
					along = 1.0 - (t - walk - work) / walk
				# Home with the load: show what it brought.
				if t < float(w["last"]) and w.get("leader", false):
					var where: Vector3 = pivot.to_global(points[0])
					delivered.emit(where, route["text"], route["color"])
					if randf() < 0.35 and _closeness >= 0.15:
						_pop(points[0] + points[0].normalized() * 0.03 * size, "pride" if randf() < 0.6 else "joy")
				w["last"] = t
				var f := along * (points.size() - 1)
				var i := mini(int(f), points.size() - 2)
				at = points[i].lerp(points[i + 1], f - i)
				if not _seen(at):
					w["at"] = Vector3.ZERO
					_hide_walker(multimesh, index)
					continue
				forward = (points[i + 1] - points[i]).normalized() * (1.0 if t < walk else -1.0)
				normal = at.normalized()
				motion = 1.0
				if t >= walk and t < walk + work:
					motion = 0.0
					var pose := _work_pose(route, w, t - walk)
					at = pose["at"]
					forward = pose["forward"]
					normal = at.normalized()
					tilt = pose.get("tilt", 0.0)
					crouch = pose.get("crouch", 1.0)
				if t >= walk + work:
					tint = route["color"]
				if w.get("hidden", false):
					hide = true
			_:
				# Strolling or building: a slow loop around a cell.
				var cell: int = w["cell"]
				normal = builder.centers[cell]
				if not _seen(normal):
					w["at"] = Vector3.ZERO
					_hide_walker(multimesh, index)
					continue
				motion = 0.6 if w["kind"] in ["stroll", "villager"] else 0.0
				if w.has("tunic"):
					tint = w["tunic"]
				if w["kind"] == "dance":
					tint = FEST_COLORS[k % FEST_COLORS.size()]
					motion = 1.0
				var tg := _tangents(normal)
				var angle := float(w["phase"]) + float(w["t"]) * float(w["speed"])
				var r := float(w["radius"])
				at = builder.ground(normal + (tg[0] * cos(angle) + tg[1] * sin(angle)) * r, cell)
				forward = (-tg[0] * sin(angle) + tg[1] * cos(angle)) * signf(float(w["speed"]))
				if w["kind"] == "builder":
					at += normal * absf(sin(float(w["t"]) * 10.0)) * 0.004
				elif w["kind"] == "groom":
					# Under the arch, facing each other.
					var c: Vector3 = _fest_at
					var cn := c.normalized()
					var ct := _tangents(cn)
					var side := 1.0 if float(w["phase"]) == 0.0 else -1.0
					at = builder.ground(c + ct[0] * side * 0.018 * _ts, cell)
					forward = -ct[0] * side
				elif w["kind"] == "dance":
					# Round the bonfire, hopping.
					var c: Vector3 = w["centre"]
					var cn := c.normalized()
					var ct := _tangents(cn)
					at = builder.ground(c + (ct[0] * cos(angle) + ct[1] * sin(angle)) * r, cell)
					forward = (-ct[0] * sin(angle) + ct[1] * cos(angle))
					at += cn * absf(sin(float(w["t"]) * 7.0 + float(w["phase"]))) * 0.006 * _ts
		at += normal * (0.026 * size + absf(sin(float(w["t"]) * 9.0)) * 0.002)
		var basis := _ride_basis(forward, normal, 1.0)
		var side := basis.x
		if tilt != 0.0:
			basis = basis.rotated(side, tilt)
		var grow := 0.7 if w.get("small", false) else 1.0
		basis = basis.scaled(Vector3(size, size * crouch, size) * grow)
		if not hide:
			_people.append(at)
			w["at"] = at
			if w.has("pid"):
				_place_name(w, at, size)
		else:
			basis = Basis().scaled(Vector3.ZERO)
			w["at"] = Vector3.ZERO
		if not (at.length() < 1.3) or not basis.is_finite():
			# Never draw anyone off the planet.
			if not _warned:
				_warned = true
				push_warning("walker off the planet: %s %s at %s" % [w["kind"], w.get("cell", w.get("route", "")), at])
			basis = Basis().scaled(Vector3.ZERO)
			at = Vector3.ZERO
		if index < MAX_WALKERS:
			multimesh.set_instance_transform(index, _safe(Transform3D(basis, at), "walker"))
			_look(multimesh, index, tint, motion)
	_move_vehicles()
	_move_animals()
	_move_birds(delta)
	_move_wonders(delta)
	_festival_fx(delta)
	_launch_bursts()
	_feel_now_and_then(delta)
	_update_fx(delta)
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
	_fishers.clear()
	for k in orders.size():
		var cell := int(orders[k]["tile"])
		if cell < 0 or cell >= builder.centers.size():
			continue
		if cell < _biomes.size() and str(_biomes[cell]) == "ocean":
			_fishers.append(cell)
		var points := _route(_city_of(cell), cell)
		var length := 0.0
		for i in points.size() - 1:
			length += points[i].distance_to(points[i + 1])
		var walk := maxf(1.5, length / WALK_SPEED)
		var resource: String = orders[k]["resource"]
		var tool := _tool_for(resource, str(_biomes[cell]) if cell < _biomes.size() else "", int(planet.get("era", 1)))
		var work := HUNT_SECONDS if tool in HUNTING else WORK_SECONDS
		# What one trip brings: the crew's planet-hour output over a trip.
		var trip_hours := (walk * 2.0 + work) * time_rate / 3600.0
		var amount := (float(outputs[k]) if k < outputs.size() else 0.0) * trip_hours
		var text := "+%s %s" % [_short(amount), I18n.name_of("resource", resource).to_lower()]
		_routes.append({"points": points, "walk": walk, "work": work, "text": text, "resource": resource,
			"color": RESOURCE_COLORS.get(resource, Color("f2c078")), "tool": tool, "cell": cell,
			"a0": rng.randf() * TAU, "offset0": float(k) * 1.7, "sea_dir": _sea_dir(cell)})
		# At sea the boat does the work: one unseen walker still brings the
		# catch home.
		var at_sea := tool == "net"
		for m in (1 if at_sea else crew_size):
			var period := walk * 2.0 + work
			# The crew walks together, a step apart; the first one tells
			# what they brought.
			var offset := m * 0.35 + float(k) * 1.7
			_walk.append({"kind": "crew", "route": _routes.size() - 1, "offset": offset, "t": 0.0, "last": fmod(_seconds + offset, period), "phase": rng.randf() * TAU, "leader": m == 0, "tool": tool, "member": m, "hidden": at_sea})
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
	# The villagers with a name stroll round the home, couples side by
	# side, children smaller; at their wedding the couple stands under the
	# arch.
	var step_now := int(planet.get("step", 0))
	# Their haunts: the home and the built cells around it.
	var haunts := [_home]
	if _home >= 0 and _home < builder.neighbors.size():
		for n in builder.neighbors[_home]:
			if n < _town.size() and _town[n]:
				haunts.append(n)
	var tunics := [Color("e3c3c6"), Color("c2d69b"), Color("eef3f2"), Color("f2c078"), Color("9fb3c8"), Color("e08f6c")]
	for p in planet.get("people", []):
		var pid := int(p["id"])
		var pair := mini(pid, int(p["partner"])) if p.get("partner") != null else pid
		var seed := float(pair) * 2.39
		var role := str(p.get("role", "food"))
		var w := {"kind": "villager", "cell": haunts[pair % haunts.size()], "offset": 0.0, "t": 0.0, "phase": seed + (0.35 if pid != pair else 0.0),
			"speed": (0.1 + fmod(seed * 0.37, 0.12)) * (1.0 if pair % 2 == 0 else -1.0),
			"radius": (0.08 + fmod(seed * 0.53, 0.16)) * _tile_size(), "tool": "hat_" + role,
			"pid": pid, "name": str(p["name"]), "famous": bool(p.get("famous", false)), "tunic": tunics[pid % tunics.size()],
			"small": step_now - int(p.get("born_step", 0)) < CHILD_STEPS}
		if festival == "wedding" and str(p["name"]) in _wedding_names and _fest_cell >= 0:
			w["kind"] = "groom"
			w["cell"] = _fest_cell
			w["phase"] = 0.0 if str(p["name"]) == _wedding_names[0] else PI
		_walk.push_front(w)
	_name_labels_for(_walk)
	# A festival: people dance around the bonfire.
	if _fest_cell >= 0:
		var dancers := clampi(roundi(population / 6.0), 6, 12)
		for m in dancers:
			_walk.push_front({"kind": "dance", "cell": _fest_cell, "offset": 0.0, "t": 0.0, "phase": m * TAU / dancers,
				"speed": 0.7, "radius": 0.14 * _tile_size(), "centre": _fest_at})
	if _walk.size() > MAX_WALKERS:
		_walk.resize(MAX_WALKERS)
	# One MultiMesh per tool (and one for empty hands): each walker is an
	# instance of the set of what it carries.
	var counts := {}
	for k in _walk.size():
		var w: Dictionary = _walk[k]
		var tool: String = w.get("tool", "")
		if not _walker_sets.has(tool):
			tool = ""
			w["tool"] = ""
		w["index"] = counts.get(tool, 0)
		counts[tool] = counts.get(tool, 0) + 1
	for tool in _walker_sets:
		_count(_walker_sets[tool], counts.get(tool, 0))
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
	mood = float(planet.get("mood", 0.6))
	_cities = planet.get("cities", [])
	_links = planet.get("links", [])
	var building_at := {}
	for c in planet.get("construction", []):
		building_at[int(c["tile"])] = "construction:" + str(c["building"])
	var biomes := []
	var heights := []
	var buildings := []
	_site_cells.clear()
	for c in planet.get("construction", []):
		_site_cells.append(int(c["tile"]))
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
	_last_view = [planet, rates]
	_wedding_names = []
	var chronicle: Array = planet.get("chronicle", [])
	for k in range(chronicle.size() - 1, maxi(-1, chronicle.size() - 40), -1):
		if str(chronicle[k]["key"]) == "wedding":
			var wp: Dictionary = chronicle[k].get("params", {})
			_wedding_names = [str(wp.get("name", "")), str(wp.get("other", ""))]
			break
	_place_walkers(planet, rates)
	if _wildlife_dirty or buildings != _wild_buildings:
		_wildlife_dirty = false
		_wild_buildings = buildings
		_place_animals(biomes, buildings)
	var finds: Array = planet.get("finds", [])
	var event = planet.get("pending_event")
	var event_tile := int(event["tile"]) if event != null and event.get("tile") != null else -1
	_place_marks(finds, event_tile, str(event["id"]) if event != null else "")
	if event_tile >= 0 and str(event["id"]) in STORMY:
		weather.set_storm(builder.centers[event_tile], "%s@%s" % [event["id"], event.get("step", 0)])
	else:
		weather.set_storm(Vector3.ZERO, "")
	_town.clear()
	for i in tiles.size():
		_town.append(buildings[i] != "")
	for c in _cities:
		var t := int(c["tile"])
		if t < _town.size():
			_town[t] = true
	_finds_at = {}
	for f in finds:
		_finds_at[int(f["tile"])] = str(f["kind"])
	_memorials_at = {}
	var memorials: Array = planet.get("memorials", [])
	for k in memorials.size():
		var m: Dictionary = memorials[k]
		var list: Array = _memorials_at.get(int(m["tile"]), [])
		list.append([k * 1.9 + 0.7, bool(m.get("famous", false))])
		_memorials_at[int(m["tile"])] = list
	var signature := JSON.stringify([biomes, buildings, planet.get("orders", []), stocks, _cities, dug, _finds_at, memorials.size()])
	_build_roads()
	if signature == _signature:
		return
	_signature = signature
	var biomes_changed := biomes != _biomes or heights != _heights
	if biomes_changed:
		_biomes = biomes
		_heights = heights
		_land.mesh = builder.build(biomes, heights, Palette.BIOMES)
		_wildlife_dirty = true
		# The rim only needs the shape: the coarse surface is enough.
		_outline.mesh = builder.build(biomes, heights, Palette.BIOMES, 1)
		_ts = _tile_size()
	if biomes_changed:
		_place_wonders(biomes)
	if not _focused_home:
		_focused_home = true
		focus(_home, true)
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
	_smoke.clear()
	if _chunks.is_empty():
		_make_chunks()
	# Each patch of the planet in its own meshes; every cell draws its
	# props from its own seed, so a change on one cell never reshuffles the
	# trees of another.
	var rng := RandomNumberGenerator.new()
	_lod = true
	for chunk in _chunks:
		_verts = PackedVector3Array()
		_normals = PackedVector3Array()
		_colors = PackedColorArray()
		_indices = PackedInt32Array()
		_far_v = PackedVector3Array()
		_far_n = PackedVector3Array()
		_far_c = PackedColorArray()
		_far_i = PackedInt32Array()
		_top = 1.0
		for i in chunk["cells"]:
			rng.seed = 11 + i * 7919
			_place_tile(i, biomes[i], buildings[i], rng)
		chunk["top"] = sqrt(_top)
		(chunk["near"] as MeshInstance3D).mesh = _mesh_of(_verts, _normals, _colors, _indices)
		(chunk["far"] as MeshInstance3D).mesh = _mesh_of(_far_v, _far_n, _far_c, _far_i)
	_lod = false
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	_far_v = PackedVector3Array()
	_far_n = PackedVector3Array()
	_far_c = PackedColorArray()
	_far_i = PackedInt32Array()
	_site_times.clear()
	_culled_from = Vector3.INF
	_place_labels()
	_update_highlight()


static func _mesh_of(verts: PackedVector3Array, normals: PackedVector3Array, colors: PackedColorArray, indices: PackedInt32Array) -> ArrayMesh:
	var mesh := ArrayMesh.new()
	if verts.is_empty():
		return mesh
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = verts
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_INDEX] = indices
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh


## Split the cells into `CHUNKS` patches around evenly spread directions
## (a Fibonacci sphere), each with a near and a far mesh.
func _make_chunks() -> void:
	var dirs := []
	for k in CHUNKS:
		var y := 1.0 - (k + 0.5) * 2.0 / CHUNKS
		var r := sqrt(1.0 - y * y)
		var a := k * PI * (3.0 - sqrt(5.0))
		dirs.append(Vector3(cos(a) * r, y, sin(a) * r))
		_chunks.append({"cells": [], "dir": dirs[k], "reach": 0.0, "top": 1.0})
	_chunk_of.resize(builder.centers.size())
	for i in builder.centers.size():
		var c: Vector3 = builder.centers[i].normalized()
		var best := 0
		for k in CHUNKS:
			if c.dot(dirs[k]) > c.dot(dirs[best]):
				best = k
		_chunk_of[i] = best
		_chunks[best]["cells"].append(i)
	var cell_reach := _tile_size() * 0.75
	for chunk in _chunks:
		var reach := 0.0
		for i in chunk["cells"]:
			reach = maxf(reach, (chunk["dir"] as Vector3).angle_to(builder.centers[i]))
		chunk["reach"] = reach + cell_reach
		for level in ["near", "far"]:
			var node := MeshInstance3D.new()
			node.material_override = _props_material
			node.visible = false
			pivot.add_child(node)
			chunk[level] = node


## Show each patch at the level the camera needs: none behind the
## horizon, the far meshes where details would be smaller than
## `DETAIL_PIXELS`, the near ones up close. Runs only when the camera moved
## in the planet's frame.
func _cull() -> void:
	if _camera == null or _chunks.is_empty():
		return
	var eye := pivot.global_transform.affine_inverse() * _camera.global_position
	if eye.distance_squared_to(_culled_from) < 1e-8:
		return
	_culled_from = eye
	var d := eye.length()
	var toward := eye / d
	var horizon := acos(clampf(GROUND_LOW / d, -1.0, 1.0))
	# Moving things: the horizon, plus a cell and the height of a bird.
	_eye_dir = toward
	_see_cos = cos(minf(PI, horizon + acos(GROUND_LOW / 1.08) + _tile_size() * 0.5))
	# Pixels per planet radius at distance 1 from the camera.
	var focal := view_height / (2.0 * tan(deg_to_rad(_camera.fov) * 0.5))
	for chunk in _chunks:
		var dir: Vector3 = chunk["dir"]
		var reach: float = chunk["reach"]
		# A prop as tall as the patch's tallest still peeks over the horizon.
		var over := acos(clampf(GROUND_LOW / maxf(float(chunk["top"]), GROUND_LOW), -1.0, 1.0))
		var seen := toward.angle_to(dir) <= horizon + over + reach
		var near := false
		if seen:
			# The patch's nearest point to the camera.
			var gap := maxf(0.02, eye.distance_to(dir) - reach)
			near = DETAIL_SIZE * focal / gap >= DETAIL_PIXELS
		(chunk["near"] as MeshInstance3D).visible = seen and near
		(chunk["far"] as MeshInstance3D).visible = seen and not near


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
		var arrays := mesh.get_mesh_arrays()
		var box := mesh.get_aabb()
		arrays.append(maxf(box.size.x, maxf(box.size.y, box.size.z)))
		_shapes[key] = arrays
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
	return _tangents_of(normal)


static func _tangents_of(normal: Vector3) -> Array:
	var up := Vector3.UP if abs(normal.y) < 0.9 else Vector3.RIGHT
	var n := normal.normalized()
	var u := n.cross(up).normalized()
	return [u, u.cross(n)]


## Place `mesh` on the tile: `offset` is (along u, up, along w) in the tile's
## frame, `turn` rotates it around the tile's up. While the planet's props
## are built (`_lod`), a shape big enough to show from afar also goes into
## the far mesh; smaller ones (a window, a flower) only into the near one.
func _put(shape: Array, color: Color, normal: Vector3, base: Vector3, offset: Vector3, size := 1.0, turn := 0.0, roll := 0.0) -> void:
	if color in GLOWING:
		color.a = 0.5
	var xform := _place(normal, base, offset, size, turn, roll)
	var rotation := xform.basis.orthonormalized()
	var verts: PackedVector3Array = shape[Mesh.ARRAY_VERTEX]
	var normals: PackedVector3Array = shape[Mesh.ARRAY_NORMAL]
	var indices: PackedInt32Array = shape[Mesh.ARRAY_INDEX]
	var start := _verts.size()
	for k in verts.size():
		var v := xform * verts[k]
		_verts.append(v)
		_normals.append(rotation * normals[k])
		_colors.append(color)
		_top = maxf(_top, v.length_squared())
	for index in indices:
		_indices.append(start + index)
	if not _lod or _extent(shape) * xform.basis.get_scale().x < FAR_MIN:
		return
	start = _far_v.size()
	for k in verts.size():
		_far_v.append(xform * verts[k])
		_far_n.append(rotation * normals[k])
		_far_c.append(color)
	for index in indices:
		_far_i.append(start + index)


## Place a Blender model (`models/<name>.glb`) like `_put` places a shape:
## its near parts in the near mesh and its far parts in the far one.
## `tints` colour its tint1, tint2... materials.
func _put_model(name: String, normal: Vector3, base: Vector3, offset: Vector3, size := 1.0, turn := 0.0, tints := []) -> void:
	var model := Models.get_model(name)
	var xform := _place(normal, base, offset, size, turn, 0.0)
	var rotation := xform.basis.orthonormalized()
	var part := model.near
	var start := _verts.size()
	for k in part.verts.size():
		var v := xform * part.verts[k]
		_verts.append(v)
		_normals.append(rotation * part.normals[k])
		var slot := part.slots[k]
		_colors.append(part.colors[k] if slot < 0 else (tints[slot] if slot < tints.size() else Color.WHITE))
		_top = maxf(_top, v.length_squared())
	for index in part.indices:
		_indices.append(start + index)
	if not _lod:
		return
	part = model.far
	start = _far_v.size()
	for k in part.verts.size():
		_far_v.append(xform * part.verts[k])
		_far_n.append(rotation * part.normals[k])
		var slot := part.slots[k]
		_far_c.append(part.colors[k] if slot < 0 else (tints[slot] if slot < tints.size() else Color.WHITE))
	for index in part.indices:
		_far_i.append(start + index)


## Colours drawn as lights (lit windows, neon): they glow at night.
const GLOWING := [Color("f2e1a0"), Color("4fe3ff"), Color("ff4fd8"), Color("f5d76e")]


## Where `_put` puts a shape: the transform from the shape's frame to the
## planet's.
func _place(normal: Vector3, base: Vector3, offset: Vector3, size: float, turn: float, roll: float) -> Transform3D:
	var t := _tangents(normal)
	var u: Vector3 = t[0]
	var w: Vector3 = t[1]
	var y := normal.normalized()
	# `roll` tips the shape over around its own first axis (a wheel, a
	# log lying down), then `turn` spins it around the ground's up.
	var basis := (Basis(u, y, w) * Basis(Vector3.RIGHT, roll)).rotated(y, turn).scaled(Vector3.ONE * size * _scale * _ts)
	offset *= _scale * _ts
	var at := base + u * offset.x + w * offset.z
	if _on_ground:
		# Nature stands wherever it is: on the ground at that very spot,
		# sunk a little so slopes never leave a gap underneath.
		at = builder.ground(at, _cell) - y * 0.004 * _ts
	return Transform3D(basis, at + y * offset.y)


## A shape's largest side (`_shape` keeps it after the mesh arrays).
static func _extent(shape: Array) -> float:
	return float(shape[Mesh.ARRAY_MAX]) if shape.size() > Mesh.ARRAY_MAX else 1.0


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
			_nature(normal, base, biome, rng, 1 if wonders.has(i) else maxi(1, roundi(6.0 * (0.25 + 0.75 * stock))))
		if biome == "forest" and stock < 0.75:
			# Stumps where the trees were cut.
			for k in roundi((1.0 - stock) * 7.0):
				var a := rng.randf() * TAU
				var r := 0.05 + rng.randf() * 0.18
				_put_model("stump", normal, base, Vector3(cos(a) * r, 0.0, sin(a) * r), 0.9 + rng.randf() * 0.3, rng.randf() * TAU)
		if _finds_at.has(i):
			_find_model(_finds_at[i], normal, base, rng)
		for m in _memorials_at.get(i, []):
			_put_model("statue" if m[1] else "memorial_stone", normal, base, Vector3(cos(m[0]) * 0.12, 0.0, sin(m[0]) * 0.12), 1.0, -m[0] + PI / 2.0)
		if wonders.has(i) and wonders[i] != "whale":
			_put_model(wonders[i], normal, base, Vector3(0.03, 0.0, -0.02), 1.4, rng.randf() * TAU, [LEAVES[0]])
		if _ordered.has(i):
			# A stake with a little flag the colour of what is gathered.
			_put(_box(0.012, 0.16, 0.012), Palette.HUT_WALL, normal, base, Vector3(0, 0.08, 0))
			_put(_box(0.06, 0.035, 0.008), RESOURCE_COLORS.get(_ordered[i], Color("f2c078")), normal, base, Vector3(0.03, 0.14, 0))
		_on_ground = false
		return
	# The footing: the plane that best fits the ground under the building,
	# so it sits on the slope like the land around it, sunk just enough
	# that no corner floats.
	var fit := _ground_plane(i, normal, base)
	var up: Vector3 = fit[0]
	var footing: Vector3 = fit[1]
	if building.begins_with("construction:"):
		_scaffold(up, footing)
		_grow_site(i, building.substr(13), up, footing)
	else:
		_building(building, up, footing)
		if SMOKY.has(building) and _smoke.size() < 40:
			_smoke.append([footing + up * float(SMOKY[building]) * _ts * BUILDING_SCALE, randf() * 2.0])
	_flag(i, normal, base)
	_on_ground = true
	_nature(normal, base, biome, rng, 2)
	# A memorial stays when a building goes up: at the cell's edge.
	for m in _memorials_at.get(i, []):
		_put_model("statue" if m[1] else "memorial_stone", normal, base, Vector3(cos(m[0]) * 0.22, 0.0, sin(m[0]) * 0.22), 0.9, -m[0] + PI / 2.0)
	_on_ground = false


## A tree standing on the ground at `at` (offset in the cell's frame): a
## fir or a tall spruce, or a broadleaf (an oak or a round lime) with leaves
## of its own green.
func _tree(normal: Vector3, base: Vector3, at: Vector3, s: float, conifer: bool, rng: RandomNumberGenerator) -> void:
	var turn := rng.randf() * TAU
	if conifer:
		_put_model("tree_conifer_tall" if rng.randf() < 0.35 else "tree_conifer", normal, base, at, s, turn)
	else:
		var leaf: Color = LEAVES[rng.randi() % LEAVES.size()]
		_put_model("tree_round" if rng.randf() < 0.4 else "tree_broadleaf", normal, base, at, s, turn, [leaf])

const LEAVES := [Color("6f9163"), Color("7fa070"), Color("8fae6b"), Color("5e7d62"), Color("94b37e")]


## A palm: a leaning trunk and a crown of fronds.
func _palm(normal: Vector3, base: Vector3, at: Vector3, s: float, rng: RandomNumberGenerator) -> void:
	_put_model("tree_palm", normal, base, at, s, rng.randf() * TAU)


## A worked-out quarry: a dark pit with spoil heaps around it.
func _quarry(normal: Vector3, base: Vector3, rng: RandomNumberGenerator) -> void:
	_put(_cylinder(0.11, 0.012, 7), Color("4a4152"), normal, base, Vector3(0, 0.002, 0))
	_put(_cylinder(0.075, 0.014, 7), Color("2f2a38"), normal, base, Vector3(0, 0.004, 0))
	for k in 4:
		var a := k * TAU / 4.0 + rng.randf()
		_put(_cone(0.04, 0.03, 5), Color("a39aa8"), normal, base, Vector3(cos(a) * 0.15, 0.012, sin(a) * 0.15), 0.8 + rng.randf() * 0.5)


## Most a building leans with the slope, in radians: steeper ground would
## tip it over.
const MAX_LEAN := 0.42


## The ground under a building on cell `cell`: [up, footing]. `up` is the
## normal of the plane fitted through two rings of ground points around the
## centre (no more than `MAX_LEAN` off the sphere's up), and `footing` the
## plane's centre, lowered so the lowest point of the rings is covered.
func _ground_plane(cell: int, normal: Vector3, base: Vector3) -> Array:
	var radial := normal.normalized()
	var t := _tangents(normal)
	var ring := []
	for r: float in [0.05, 0.1]:
		for k in 8:
			var a: float = k * TAU / 8.0 + r * 3.0
			ring.append(builder.ground(base + (t[0] * cos(a) + t[1] * sin(a)) * r * _ts * BUILDING_SCALE, cell))
	var centre := Vector3.ZERO
	for p in ring:
		centre += p
	centre /= ring.size()
	var up := Vector3.ZERO
	for k in ring.size():
		var a: Vector3 = ring[k] - centre
		var b: Vector3 = ring[(k + 1) % 8 + (8 if k >= 8 else 0)] - centre
		up += b.cross(a)
	if up.dot(radial) < 0.0:
		up = -up
	up = up.normalized() if up.length_squared() > 1e-12 else radial
	var lean := up.angle_to(radial)
	if lean > MAX_LEAN:
		up = radial.slerp(up, MAX_LEAN / lean).normalized()
	var low := 0.0
	for p in ring:
		low = minf(low, (p - centre).dot(up))
	return [up, centre + up * (low - 0.003 * _ts)]


## Buildings are drawn larger than life, so they read at the pixel scale.
const BUILDING_SCALE := 1.15


## Trees, peaks or ice around the tile's edge (`amount` trees on a forest),
## and the small things only seen up close: grass, flowers, mushrooms,
## mossy stones.
func _nature(normal: Vector3, base: Vector3, biome: String, rng: RandomNumberGenerator, amount: int) -> void:
	match biome:
		"forest":
			for k in amount * 2:
				var angle := k * TAU / (amount * 2) + rng.randf() * 0.6
				var r := (0.04 + rng.randf() * 0.2) if amount > 2 else 0.2 + rng.randf() * 0.06
				var s := 0.7 + rng.randf() * 0.6
				_tree(normal, base, Vector3(cos(angle) * r, 0.0, sin(angle) * r), s, rng.randf() < 0.6, rng)
			if rng.randf() < 0.5:
				_scatter("rock_mossy", normal, base, rng, 1, 0.5, 0.8)
			if rng.randf() < 0.3:
				_scatter("bush", normal, base, rng, 1, 0.5, 0.7, [LEAVES[3]])
			_scatter("grass", normal, base, rng, 2, 0.6, 1.0)
		"mountain":
			# The peak is the ground itself; a few boulders on its flanks.
			for k in mini(amount, 3):
				var angle := k * TAU / 3.0 + rng.randf()
				var r := 0.12 + rng.randf() * 0.1
				_put_model("rock", normal, base, Vector3(cos(angle) * r, 0.0, sin(angle) * r), 0.7 + rng.randf() * 0.6, rng.randf() * TAU)
			if rng.randf() < 0.3:
				_scatter("tree_conifer", normal, base, rng, 1, 0.5, 0.7)
		"plains":
			for k in mini(amount, 2):
				_scatter("bush", normal, base, rng, 1, 0.8, 1.2, [LEAVES[rng.randi() % LEAVES.size()]])
			# Now and then a lone broadleaf tree, flowers and grass.
			if amount > 3 and rng.randf() < 0.45:
				var a := rng.randf() * TAU
				_tree(normal, base, Vector3(cos(a) * 0.17, 0.0, sin(a) * 0.17), 0.8, false, rng)
			_scatter("flowers", normal, base, rng, 2, 0.8, 1.1)
			_scatter("grass", normal, base, rng, 4, 0.7, 1.1)
			if rng.randf() < 0.15:
				_scatter("rock", normal, base, rng, 1, 0.3, 0.5)
		"coast":
			if amount > 3 and rng.randf() < 0.5:
				var a := rng.randf() * TAU
				_palm(normal, base, Vector3(cos(a) * 0.15, 0.0, sin(a) * 0.15), 0.9 + rng.randf() * 0.3, rng)
			_scatter("grass", normal, base, rng, 1, 0.6, 0.9)
		"desert":
			if amount > 3 and rng.randf() < 0.35:
				var a := rng.randf() * TAU
				_palm(normal, base, Vector3(cos(a) * 0.15, 0.0, sin(a) * 0.15), 0.9 + rng.randf() * 0.3, rng)
			elif rng.randf() < 0.5:
				_scatter("cactus", normal, base, rng, 1 + rng.randi() % 2, 0.8, 1.2)
			if rng.randf() < 0.2:
				_scatter("tree_dead", normal, base, rng, 1, 0.8, 1.0)
			if rng.randf() < 0.4:
				_scatter("rock", normal, base, rng, 1, 0.3, 0.6)
		"ice":
			for k in mini(amount, 3):
				var angle := k * TAU / 3.0 + rng.randf()
				_put_model("ice_shard", normal, base, Vector3(cos(angle) * 0.14, 0.0, sin(angle) * 0.14), 0.8 + rng.randf() * 0.4, rng.randf() * TAU)


## `count` copies of a model at random spots of the cell, sized between
## `low` and `high`.
func _scatter(name: String, normal: Vector3, base: Vector3, rng: RandomNumberGenerator, count: int, low: float, high: float, tints := []) -> void:
	for k in count:
		var a := rng.randf() * TAU
		var r := 0.04 + rng.randf() * 0.2
		_put_model(name, normal, base, Vector3(cos(a) * r, 0.0, sin(a) * r), lerpf(low, high, rng.randf()), rng.randf() * TAU, tints)


var _stocks := []
var _finds_at := {}
## cell -> [[angle, famous]] of the memorials there.
var _memorials_at := {}
var _dug := []
var _ordered := {}
var _site_times := {}


## The building going up on a site: its own small mesh, scaled up from the
## footing as the work goes on (see `_move_walkers`).
func _grow_site(cell: int, id: String, normal: Vector3, footing: Vector3) -> void:
	var saved := [_verts, _normals, _colors, _indices, _lod, _top]
	_lod = false
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
	_lod = saved[4]
	_top = saved[5]
	if arrays[Mesh.ARRAY_VERTEX].is_empty():
		return
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var node := MeshInstance3D.new()
	node.mesh = mesh
	node.material_override = _props_material
	pivot.add_child(node)
	var times: Array = _site_times.get(cell, [0.0, 1.0])
	_sites.append([node, footing, normal.normalized(), times[0], times[1]])


func _scaffold(normal: Vector3, base: Vector3) -> void:
	_put_model("scaffold", normal, base, Vector3.ZERO)


## One small model per building id.
func _building(id: String, n: Vector3, b: Vector3) -> void:
	_scale = BUILDING_SCALE
	_building_parts(id, n, b)
	_scale = 1.0


var _scale := 1.0


## Buildings with a Blender model of their own (models/<id>.glb); the
## later eras' ones are still drawn from shapes (`_styled`).
const MODELLED := ["hut", "house", "hunting_camp", "fire_pit", "stone_circle", "field", "pasture",
	"storehouse", "great_granary", "mine", "foundry", "temple", "scribe_house", "ziggurat", "colony"]


func _building_parts(id: String, n: Vector3, b: Vector3) -> void:
	if id in MODELLED:
		_put_model(id, n, b, Vector3.ZERO)
		return
	var wall := Palette.HUT_WALL
	var roof := Palette.HUT_ROOF
	var stone := Palette.PEAK
	match id:
		"hut":
			# Round walls of wattle, a door, a thatched cone with an overhang.
			_put(_cylinder(0.06, 0.06, 8), wall, n, b, Vector3(0, 0.03, 0))
			_put(_box(0.02, 0.035, 0.012), Color("4a3b33"), n, b, Vector3(0.0, 0.018, 0.058))
			_put(_cone(0.085, 0.08, 8), roof, n, b, Vector3(0, 0.1, 0))
			_put(_cylinder(0.008, 0.02, 5), Color("6b5a4e"), n, b, Vector3(0, 0.145, 0))
		"house":
			# Walls of brick, a gable roof, door, windows and a chimney.
			_put(_box(0.15, 0.08, 0.1), Color("d9b48f"), n, b, Vector3(0, 0.04, 0))
			_put(_prism(0.17, 0.06, 0.115), roof, n, b, Vector3(0, 0.11, 0), 1.0, PI / 2.0)
			_put(_box(0.024, 0.042, 0.006), Color("6b4a3a"), n, b, Vector3(0.0, 0.021, 0.051))
			for side in [-1, 1]:
				_put(_box(0.02, 0.018, 0.006), Color("f2e1a0"), n, b, Vector3(side * 0.045, 0.05, 0.051))
			_put(_box(0.018, 0.04, 0.018), Color("a35d5d"), n, b, Vector3(0.045, 0.13, -0.02))
		"hunting_camp":
			_put(_cone(0.07, 0.12, 6), Color("b58e6b"), n, b, Vector3(-0.04, 0.06, 0))
			_put(_box(0.02, 0.03, 0.006), Color("4a3b33"), n, b, Vector3(-0.04, 0.015, 0.05))
			# A drying rack with hides, and a small fire.
			for side in [-1, 1]:
				_put(_box(0.008, 0.08, 0.008), Color("6b5a4e"), n, b, Vector3(0.06, 0.04, side * 0.04))
			_put(_box(0.008, 0.008, 0.09), Color("6b5a4e"), n, b, Vector3(0.06, 0.078, 0))
			_put(_box(0.006, 0.04, 0.05), Color("d9b48f"), n, b, Vector3(0.06, 0.055, 0))
			_put(_cone(0.015, 0.03, 5), Color("f2c078"), n, b, Vector3(0.02, 0.015, 0.07))
		"fire_pit":
			for k in 8:
				var a := k * TAU / 8.0
				_put(_box(0.026, 0.02, 0.026), stone, n, b, Vector3(cos(a) * 0.06, 0.01, sin(a) * 0.06), 1.0, a)
			for k in 3:
				_put(_box(0.07, 0.012, 0.012), Color("6b5a4e"), n, b, Vector3(0, 0.008, 0), 1.0, k * PI / 3.0)
			_put(_cone(0.04, 0.08, 5), Color("f2c078"), n, b, Vector3(0, 0.045, 0))
			_put(_cone(0.025, 0.06, 5), Color("e08f6c"), n, b, Vector3(0.006, 0.07, 0))
			# Logs to sit on.
			for k in 3:
				var a := k * TAU / 3.0 + 0.5
				_put(_cylinder(0.012, 0.07, 6), Color("8f6f5a"), n, b, Vector3(cos(a) * 0.12, 0.01, sin(a) * 0.12), 1.0, -a, PI / 2.0)
		"stone_circle":
			# Trilithons: pairs of standing stones with a lintel.
			for k in 5:
				var a := k * TAU / 5.0
				var c := Vector3(cos(a) * 0.12, 0.0, sin(a) * 0.12)
				var side := Vector3(-sin(a), 0.0, cos(a)) * 0.025
				_put(_box(0.024, 0.1, 0.02), stone, n, b, c + side + Vector3(0, 0.05, 0), 1.0, -a)
				_put(_box(0.024, 0.1, 0.02), stone, n, b, c - side + Vector3(0, 0.05, 0), 1.0, -a)
				_put(_box(0.026, 0.018, 0.075), Color("a39aa8"), n, b, c + Vector3(0, 0.108, 0), 1.0, -a)
			_put(_box(0.05, 0.03, 0.05), Color("d9d0cc"), n, b, Vector3(0, 0.015, 0))
		"field":
			# Furrows of two crops and a low fence.
			for k in 5:
				_put(_box(0.24, 0.016, 0.03), Color("f2c078") if k % 2 == 0 else Color("c2d69b"), n, b, Vector3(0, 0.008, (k - 2) * 0.045))
			for k in 6:
				_put(_box(0.008, 0.03, 0.008), Color("8f6f5a"), n, b, Vector3(-0.13 + k * 0.052, 0.015, 0.125))
			_put(_box(0.27, 0.006, 0.006), Color("8f6f5a"), n, b, Vector3(0, 0.024, 0.125))
		"pasture":
			for k in 8:
				var a := k * TAU / 8.0
				_put(_box(0.008, 0.035, 0.008), Color("8f6f5a"), n, b, Vector3(cos(a) * 0.13, 0.017, sin(a) * 0.13))
				_put(_box(0.1, 0.006, 0.006), Color("8f6f5a"), n, b, Vector3(cos(a + PI / 8.0) * 0.12, 0.027, sin(a + PI / 8.0) * 0.12), 1.0, -(a + PI / 8.0) + PI / 2.0)
			for k in 3:
				var a := k * 2.1 + 0.4
				var at := Vector3(cos(a) * 0.06, 0.0, sin(a) * 0.06)
				_put(_box(0.045, 0.028, 0.03), Palette.SNOW, n, b, at + Vector3(0, 0.024, 0), 1.0, a)
				_put(_box(0.016, 0.016, 0.016), Color("4a3b33"), n, b, at + Vector3(cos(a) * 0.028, 0.032, -sin(a) * 0.028), 1.0, a)
		"storehouse":
			_put(_box(0.2, 0.08, 0.12), Color("b58e6b"), n, b, Vector3(0, 0.04, 0))
			_put(_prism(0.22, 0.06, 0.135), roof, n, b, Vector3(0, 0.11, 0), 1.0, PI / 2.0)
			_put(_box(0.05, 0.06, 0.006), Color("6b4a3a"), n, b, Vector3(0, 0.03, 0.061))
			_put(_box(0.03, 0.025, 0.04), Color("d9b48f"), n, b, Vector3(0.12, 0.0125, 0.05))
		"great_granary":
			_put(_cylinder(0.09, 0.16, 10), Color("d9b48f"), n, b, Vector3(0, 0.08, 0))
			_put(_cone(0.11, 0.1, 10), roof, n, b, Vector3(0, 0.21, 0))
			_put(_cylinder(0.05, 0.1, 8), Color("d9b48f"), n, b, Vector3(0.13, 0.05, 0.04))
			_put(_cone(0.065, 0.06, 8), roof, n, b, Vector3(0.13, 0.13, 0.04))
			_put(_box(0.03, 0.05, 0.006), Color("6b4a3a"), n, b, Vector3(0, 0.025, 0.09))
			for k in 4:
				_put(_box(0.012, 0.012, 0.03), Color("f2c078"), n, b, Vector3(-0.08 + k * 0.02, 0.006, 0.1))
		"mine":
			_put(_box(0.13, 0.08, 0.08), Color("5b5466"), n, b, Vector3(0, 0.04, 0))
			_put(_box(0.06, 0.05, 0.02), Palette.OUTLINE, n, b, Vector3(0, 0.025, 0.045))
			for side in [-1, 1]:
				_put(_box(0.01, 0.06, 0.01), Color("8f6f5a"), n, b, Vector3(side * 0.035, 0.03, 0.056))
			_put(_box(0.08, 0.01, 0.01), Color("8f6f5a"), n, b, Vector3(0, 0.06, 0.056))
			for side in [-1, 1]:
				_put(_box(0.004, 0.004, 0.09), Color("4a4a4f"), n, b, Vector3(side * 0.012, 0.002, 0.1))
			_put(_box(0.03, 0.02, 0.025), Color("6b5a4e"), n, b, Vector3(0, 0.012, 0.11))
			_put(_cone(0.04, 0.03, 6), Color("857c8a"), n, b, Vector3(0.1, 0.015, 0.05))
		"foundry":
			_put(_box(0.14, 0.08, 0.11), Color("857c8a"), n, b, Vector3(0, 0.04, 0))
			_put(_prism(0.15, 0.04, 0.12), Color("5b5466"), n, b, Vector3(0, 0.1, 0), 1.0, PI / 2.0)
			_put(_cylinder(0.022, 0.13, 6), Color("5b5466"), n, b, Vector3(0.04, 0.14, 0.02))
			_put(_box(0.04, 0.03, 0.006), Color("e08f6c"), n, b, Vector3(-0.03, 0.025, 0.056))
			_put(_ball(0.018), Color("b0a6ad"), n, b, Vector3(0.04, 0.21, 0.02))
		"temple":
			_put(_box(0.2, 0.03, 0.15), stone, n, b, Vector3(0, 0.015, 0))
			_put(_box(0.16, 0.012, 0.12), Color("d9d0cc"), n, b, Vector3(0, 0.036, 0))
			for k in 5:
				for row in [-1, 1]:
					_put(_cylinder(0.009, 0.09, 6), Color("eae2d6"), n, b, Vector3(-0.07 + k * 0.035, 0.087, row * 0.05))
			_put(_box(0.18, 0.015, 0.13), Color("d9d0cc"), n, b, Vector3(0, 0.138, 0))
			_put(_prism(0.19, 0.04, 0.14), Color("c07a54"), n, b, Vector3(0, 0.165, 0))
		"scribe_house":
			_put(_box(0.13, 0.09, 0.11), Color("e3c3c6"), n, b, Vector3(0, 0.045, 0))
			_put(_box(0.15, 0.02, 0.13), Color("7a4f4f"), n, b, Vector3(0, 0.1, 0))
			_put(_box(0.024, 0.045, 0.006), Color("6b4a3a"), n, b, Vector3(0, 0.022, 0.056))
			for side in [-1, 1]:
				_put(_box(0.018, 0.018, 0.006), Color("f2e1a0"), n, b, Vector3(side * 0.04, 0.06, 0.056))
			_put(_box(0.03, 0.004, 0.04), Color("eae2d6"), n, b, Vector3(0.08, 0.002, 0.06))
		"ziggurat":
			for k in 4:
				var s := 0.26 - k * 0.055
				_put(_box(s, 0.05, s), Color("d9b48f") if k % 2 == 0 else Color("b58e6b"), n, b, Vector3(0, 0.025 + k * 0.05, 0))
			_put(_box(0.03, 0.2, 0.02), Color("c07a54"), n, b, Vector3(0, 0.09, 0.12))
			_put(_box(0.04, 0.04, 0.04), Color("f2c078"), n, b, Vector3(0, 0.22, 0))
		"colony":
			# A longhouse with a gable roof inside a pointed palisade.
			_put(_box(0.16, 0.07, 0.09), Color("d9b48f"), n, b, Vector3(0, 0.035, 0))
			_put(_prism(0.18, 0.06, 0.1), roof, n, b, Vector3(0, 0.1, 0), 1.0, PI / 2.0)
			_put(_box(0.024, 0.04, 0.006), Color("6b4a3a"), n, b, Vector3(0, 0.02, 0.046))
			for k in 16:
				var a := k * TAU / 16.0
				if k == 4 or k == 5:
					continue
				var at := Vector3(cos(a) * 0.15, 0.0, sin(a) * 0.15)
				_put(_cylinder(0.009, 0.055, 5), Color("8f6f5a"), n, b, at + Vector3(0, 0.027, 0))
				_put(_cone(0.009, 0.016, 5), Color("6b5a4e"), n, b, at + Vector3(0, 0.062, 0))
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
			_put(_box(0.13 * k, 0.08 * k, 0.1 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_prism(0.15 * k, 0.06 * k, 0.115 * k), c2, n, b, Vector3(0, 0.11 * k, 0), 1.0, PI / 2.0)
			_facade(n, b, k, 0.051, 1)
			_put(_box(0.016 * k, 0.04 * k, 0.016 * k), c2.darkened(0.2), n, b, Vector3(0.04 * k, 0.13 * k, -0.02 * k))
		"hall":
			_put(_box(0.2 * k, 0.08 * k, 0.12 * k), c1, n, b, Vector3(0, 0.04 * k, 0))
			_put(_box(0.22 * k, 0.02 * k, 0.14 * k), c2, n, b, Vector3(0, 0.09 * k, 0))
			_facade(n, b, k, 0.061, 2)
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
			for i in 3:
				for j in 3:
					_put(_box(0.018 * k, 0.018 * k, 0.004 * k), Color("f2e1a0"), n, b, Vector3((j - 1) * 0.035 * k, (0.03 + i * 0.05) * k, 0.061 * k))
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
	# The pole stands upright on the ground where it is.
	var t := _tangents(normal)
	var foot := builder.ground(base + (t[0] * -0.12 + t[1] * -0.06) * _ts, cell) - normal.normalized() * 0.004 * _ts
	var saved := _on_ground
	_on_ground = false
	_put(_box(0.012, 0.24, 0.012), Palette.HUT_WALL, normal, foot, Vector3(0, 0.12, 0))
	_put(_box(0.08, 0.045, 0.008), CITY_COLORS[k % CITY_COLORS.size()], normal, foot, Vector3(0.04, 0.215, 0))
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


## Carts on the finished roads, boats on the finished sea routes, and a
## fishing boat on every cell of sea a crew works.
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
	var carts := _rides.filter(func(r): return not r["boat"])
	var sails := _rides.filter(func(r): return r["boat"])
	_rides = carts + sails
	_count(_vehicles, mini(carts.size(), MAX_RIDES))
	for k in mini(carts.size(), MAX_RIDES):
		_look(_vehicles.multimesh, k, CART_LOADS[k % CART_LOADS.size()], 0.0)
	_count(_boats, mini(sails.size() + _fishers.size(), MAX_RIDES))
	for k in _boats.multimesh.visible_instance_count:
		_look(_boats.multimesh, k, Color("eef3f2") if k < sails.size() else Color("e3c3c6"), 0.0)
	_move_vehicles()


const CART_LOADS := [Color("d9b48f"), Color("c2d69b"), Color("b0a6ad"), Color("f2c078")]


## A model's frame on the ground: up along `normal`, its front (+Z) along
## `forward`.
static func _ride_basis(forward: Vector3, normal: Vector3, size: float) -> Basis:
	var side := normal.cross(forward)
	if side.length_squared() < 1e-8:
		side = _tangents_of(normal)[0]
	side = side.normalized()
	return Basis(side, normal, side.cross(normal)).scaled(Vector3.ONE * size)


func _move_vehicles() -> void:
	var size := _tile_size() * 1.2
	var carts := 0
	var boats := 0
	for r in _rides:
		var points: PackedVector3Array = r["points"]
		var trip := float(r["length"]) / (WALK_SPEED * 1.4)
		var t := fmod(_seconds / trip + float(r["offset"]), 2.0)
		var along := t if t < 1.0 else 2.0 - t
		var f := along * (points.size() - 1)
		var i := mini(int(f), points.size() - 2)
		var at := points[i].lerp(points[i + 1], f - i)
		var normal := at.normalized()
		var forward := (points[i + 1] - points[i]).normalized() * (1.0 if t < 1.0 else -1.0)
		if r["boat"] and boats >= MAX_RIDES:
			continue
		if r["boat"]:
			at += normal * (0.002 * sin(_seconds * 2.0 + boats) - 0.002 * size)
			_boats.multimesh.set_instance_transform(boats, _safe(Transform3D(_ride_basis(forward, normal, size), at), "boat"))
			boats += 1
		elif carts < MAX_RIDES:
			at += normal * (0.004 * size + 0.002)
			_vehicles.multimesh.set_instance_transform(carts, _safe(Transform3D(_ride_basis(forward, normal, size), at), "cart"))
			carts += 1
	# Fishing boats circle slowly on their cell.
	for k in _fishers.size():
		var cell: int = _fishers[k]
		if boats >= _boats.multimesh.visible_instance_count or cell >= builder.centers.size():
			break
		var n: Vector3 = builder.centers[cell]
		var tg := _tangents(n)
		var a := _seconds * 0.25 + k * 2.1
		var dir: Vector3 = tg[0] * cos(a) + tg[1] * sin(a)
		var at := n * PlanetMesh.SEA_LEVEL + dir * 0.03 * _ts
		at = at.normalized() * (PlanetMesh.SEA_LEVEL - 0.002 * size + 0.002 * sin(_seconds * 2.0 + k))
		var forward: Vector3 = -tg[0] * sin(a) + tg[1] * cos(a)
		_boats.multimesh.set_instance_transform(boats, _safe(Transform3D(_ride_basis(forward, at.normalized(), size), at), "fisher"))
		boats += 1


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


## A marker over every find (a bobbing gem on a pin) and, over the cell of
## an event that happens somewhere, the event itself: a burning sun, a traders' cart, an explorers' boat, a
## falling meteor, or a sign.
func _place_marks(finds: Array, event_tile: int, event_id: String) -> void:
	for mark in _marks:
		mark["node"].queue_free()
	_marks.clear()
	var wanted := []
	for f in finds:
		wanted.append([int(f["tile"]), "find:" + str(f["kind"])])
	if event_tile >= 0:
		wanted.append([event_tile, "event:" + event_id])
	for w in wanted:
		var cell: int = w[0]
		if cell < 0 or cell >= builder.centers.size():
			continue
		var kind: String = w[1]
		var n: Vector3 = builder.centers[cell]
		var ground := builder.ground(n, cell)
		var t := _tangents(n)
		var node := Node3D.new()
		node.transform = Transform3D(Basis(t[0], n.normalized(), t[1]).scaled(Vector3.ONE * _tile_size()), ground)
		pivot.add_child(node)
		var parts := _mark_parts(kind, node)
		_marks.append({"node": node, "tile": cell, "kind": "event" if kind.begins_with("event:") else kind, "event": kind.substr(6) if kind.begins_with("event:") else "", "base": ground + n.normalized() * 0.09 * _tile_size(), "parts": parts})
	_bob_marks()


func _mark_mesh(build: Callable, unshaded := false) -> MeshInstance3D:
	var node := MeshInstance3D.new()
	node.mesh = _compose(build)
	if unshaded:
		var m := StandardMaterial3D.new()
		m.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		m.vertex_color_use_as_albedo = true
		node.material_override = m
	else:
		node.material_override = _props_material
	return node


## The pieces of a mark, as children of `node` (local frame: +Y up, one
## unit is a cell's size); returns what `_bob_marks` animates.
func _mark_parts(kind: String, node: Node3D) -> Dictionary:
	var up := Vector3.UP
	var o := Vector3.ZERO
	var parts := {}
	var event := kind.substr(6) if kind.begins_with("event:") else ""
	if not kind.begins_with("event:"):
		var gem := _mark_mesh(func():
			_put(_cone(0.06, 0.08, 4), FIND_COLOR, up, o, Vector3(0, 0.04, 0))
			_put(_cone(0.06, 0.08, 4), FIND_COLOR.darkened(0.2), up, o, Vector3(0, -0.04, 0), 1.0, 0.0, PI), true)
		gem.position = Vector3(0, 0.42, 0)
		node.add_child(gem)
		var pin := _mark_mesh(func(): _put(_box(0.006, 0.3, 0.006), Color("eef3f2"), up, o, Vector3(0, 0.15, 0)), true)
		node.add_child(pin)
		parts["spin"] = gem
		return parts
	match event:
		"storm", "lightning":
			# The storm itself is weather (see `Weather`): no model here.
			pass
		"drought":
			var sun := _mark_mesh(func():
				_put(_ball(0.07), Color("f2c078"), up, o, Vector3.ZERO)
				_put(_ball(0.07), Color("f2c078"), up, o, Vector3.ZERO, 1.0, 0.0, PI)
				for k in 8:
					_put(_box(0.03, 0.012, 0.012), Color("e08f6c"), up, o, Vector3(cos(k * TAU / 8.0) * 0.11, 0.0, sin(k * TAU / 8.0) * 0.11), 1.0, -k * TAU / 8.0), true)
			sun.position = Vector3(0, 0.55, 0)
			node.add_child(sun)
			parts["spin"] = sun
		"traders":
			var cart := _mark_mesh(func():
				_put(_box(0.1, 0.03, 0.06), Color("8f6f5a"), up, o, Vector3(0, 0.04, 0))
				_put(_box(0.08, 0.04, 0.05), Color("e3c3c6"), up, o, Vector3(0, 0.075, 0))
				for side in [-1.0, 1.0]:
					_put(_cylinder(0.025, 0.01, 8), Color("4a3b33"), up, o, Vector3(0, 0.025, side * 0.035), 1.0, PI / 2.0, PI / 2.0)
				_put(_box(0.006, 0.16, 0.006), Color("6b5a4e"), up, o, Vector3(-0.04, 0.12, 0))
				_put(_box(0.05, 0.03, 0.004), Color("4fa3c7"), up, o, Vector3(-0.015, 0.18, 0)))
			node.add_child(cart)
			parts["bob"] = cart
		"explorers":
			var boat := _mark_mesh(func():
				_put(_box(0.14, 0.03, 0.05), Color("6b5a4e"), up, o, Vector3(0, 0.0, 0))
				_put(_box(0.008, 0.14, 0.008), Color("4a3b33"), up, o, Vector3(0, 0.08, 0))
				_put(_prism(0.008, 0.11, 0.08), Color("eef3f2"), up, o, Vector3(0.025, 0.09, 0), 1.0, PI / 2.0))
			node.add_child(boat)
			parts["bob"] = boat
		"meteor":
			var rock := _mark_mesh(func():
				_put(_ball(0.04), Color("ff9d3b"), up, o, Vector3.ZERO)
				_put(_ball(0.04), Color("e08f6c"), up, o, Vector3.ZERO, 1.0, 0.0, PI)
				for k in 3:
					_put(_cone(0.03 - k * 0.008, 0.06, 5), Color("f2c078"), up, o, Vector3(0, 0.05 + k * 0.05, 0)), true)
			node.add_child(rock)
			parts["fall"] = rock
		_:
			var sign := _mark_mesh(func():
				_put(_box(0.012, 0.25, 0.012), Color("6b5a4e"), up, o, Vector3(0, 0.125, 0))
				_put(_box(0.03, 0.09, 0.03), EVENT_COLOR, up, o, Vector3(0, 0.36, 0))
				_put(_box(0.03, 0.03, 0.03), EVENT_COLOR, up, o, Vector3(0, 0.28, 0)), true)
			node.add_child(sign)
			parts["bob"] = sign
	return parts


func _bob_marks() -> void:
	for k in _marks.size():
		var parts: Dictionary = _marks[k]["parts"]
		var t := _seconds + k * 1.3
		if parts.has("spin"):
			var gem: Node3D = parts["spin"]
			gem.rotation = Vector3(0.0, t * 1.5, 0.0)
			if not _marks[k]["kind"].begins_with("event"):
				gem.position.y = 0.42 + 0.03 * sin(t * 3.0)
		if parts.has("bob"):
			(parts["bob"] as Node3D).position.y = 0.02 + 0.015 * absf(sin(t * 2.0))
		if parts.has("fall"):
			var f := fmod(t * 0.5, 1.0)
			var rock: Node3D = parts["fall"]
			rock.position = Vector3(0.25 * (1.0 - f), 0.05 + 0.6 * (1.0 - f), 0.0)
			rock.rotation = Vector3(0.0, 0.0, 0.6)


## Where the marks are (world space), for the window's edge markers.
func marks() -> Array:
	var out := []
	for mark in _marks:
		out.append({"tile": mark["tile"], "kind": mark["kind"], "at": pivot.to_global(mark["base"]), "normal": (pivot.global_transform.basis * (mark["base"] as Vector3)).normalized()})
	return out


# ---------------------------------------------------------------- sound

var _town := []
var _site_cells := []


## The kinds of land around the middle of the view (shares adding to 1):
## ocean, forest, open (meadows, beaches), rock (mountains, desert, ice),
## town (built cells). The closer the camera, the smaller the patch.
func view_mix(camera_position: Vector3, closeness: float) -> Dictionary:
	var mix := {"ocean": 0.0, "forest": 0.0, "open": 0.0, "rock": 0.0, "town": 0.0}
	if _biomes.is_empty() or not builder.ready():
		return mix
	var centre := (pivot.global_transform.affine_inverse() * camera_position).normalized()
	var radius := lerpf(0.7, 0.12, closeness)
	var t := _tangents(centre)
	var count := 0.0
	for ring in 3:
		var r := radius * ring / 2.0
		var n := 1 if ring == 0 else 6 * ring
		for k in n:
			var a := k * TAU / n
			var d: Vector3 = (centre + (t[0] * cos(a) + t[1] * sin(a)) * r).normalized()
			var cell := builder.cell_at(d)
			count += 1.0
			if cell < _town.size() and _town[cell]:
				mix["town"] += 1.0
				continue
			match str(_biomes[cell]):
				"ocean":
					mix["ocean"] += 1.0
				"forest":
					mix["forest"] += 1.0
				"plains", "coast":
					mix["open"] += 1.0
				_:
					mix["rock"] += 1.0
	for k in mix:
		mix[k] /= maxf(1.0, count)
	return mix


## Building sites, in world space (for their hammers).
func site_points() -> Array:
	var out := []
	for cell in _site_cells:
		if cell >= 0 and cell < builder.centers.size():
			out.append(pivot.to_global(builder.ground(builder.centers[cell], cell)))
	return out


## A small mesh built from shapes in a local frame (up is +Y, the front is
## +X), for MultiMesh instances.
func _compose(build: Callable) -> ArrayMesh:
	var saved := [_verts, _normals, _colors, _indices, _ts, _scale, _on_ground]
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	_ts = 1.0
	_scale = 1.0
	_on_ground = false
	build.call()
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = _verts
	arrays[Mesh.ARRAY_NORMAL] = _normals
	arrays[Mesh.ARRAY_COLOR] = _colors
	arrays[Mesh.ARRAY_INDEX] = _indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	_verts = saved[0]
	_normals = saved[1]
	_colors = saved[2]
	_indices = saved[3]
	_ts = saved[4]
	_scale = saved[5]
	_on_ground = saved[6]
	return mesh


## A triangular prism (a gable roof): `x` wide, `y` high, `z` long.
static func _prism(x: float, y: float, z: float) -> Array:
	return _shape("prism %s %s %s" % [x, y, z], func():
		var mesh := PrismMesh.new()
		mesh.size = Vector3(x, y, z)
		return mesh)


## A front with a door and lit windows (`z` is the wall's depth from the
## middle, `rows` of windows on each side of the door).
func _facade(n: Vector3, b: Vector3, k: float, z: float, rows: int) -> void:
	_put(_box(0.024 * k, 0.042 * k, 0.005 * k), Color("6b4a3a"), n, b, Vector3(0, 0.021 * k, z * k))
	for side in [-1, 1]:
		for r in rows:
			_put(_box(0.018 * k, 0.016 * k, 0.005 * k), Color("f2e1a0"), n, b, Vector3(side * (0.04 + r * 0.035) * k, 0.052 * k, z * k))


# ---------------------------------------------------------------- tools and wildlife

var _creature_material: ShaderMaterial


## A Blender model (its near level) as one mesh for MultiMesh instances,
## with the shapes `extra` puts (as in `_compose`) added in.
func _creature_mesh(name: String, extra := Callable()) -> ArrayMesh:
	var mesh := Models.get_model(name).near.mesh()
	if not extra.is_valid():
		return mesh
	var more := _compose(extra)
	if more.get_surface_count() == 0:
		return mesh
	var a := mesh.surface_get_arrays(0)
	var b := more.surface_get_arrays(0)
	a[Mesh.ARRAY_TANGENT] = null
	var start: int = (a[Mesh.ARRAY_VERTEX] as PackedVector3Array).size()
	var added: int = (b[Mesh.ARRAY_VERTEX] as PackedVector3Array).size()
	for k in [Mesh.ARRAY_VERTEX, Mesh.ARRAY_NORMAL, Mesh.ARRAY_COLOR]:
		var list = a[k]
		list.append_array(b[k])
		a[k] = list
	var uv: PackedVector2Array = a[Mesh.ARRAY_TEX_UV]
	var uv2: PackedVector2Array = a[Mesh.ARRAY_TEX_UV2]
	for k in added:
		uv.append(Vector2.ZERO)
		uv2.append(Vector2.ZERO)
	a[Mesh.ARRAY_TEX_UV] = uv
	a[Mesh.ARRAY_TEX_UV2] = uv2
	var indices: PackedInt32Array = a[Mesh.ARRAY_INDEX]
	for i in (b[Mesh.ARRAY_INDEX] as PackedInt32Array):
		indices.append(start + i)
	a[Mesh.ARRAY_INDEX] = indices
	var out := ArrayMesh.new()
	out.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, a)
	return out


## A MultiMesh of `count` hidden instances of `mesh`, drawn by
## creature.gdshader; `_look` gives each its tint and motion.
func _creature_set(mesh: Mesh, count: int) -> MultiMeshInstance3D:
	if _creature_material == null:
		_creature_material = ShaderMaterial.new()
		_creature_material.shader = load("res://creature.gdshader")
	var node := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_custom_data = true
	mm.mesh = mesh
	_fixed(mm, count)
	node.multimesh = mm
	node.material_override = _creature_material
	node.visible = false
	pivot.add_child(node)
	return node


## An instance's tint (its model's tint materials) and motion (0 standing
## still, 1 walking: how far legs, arms and tails swing).
static func _look(mm: MultiMesh, i: int, tint: Color, motion: float) -> void:
	mm.set_instance_custom_data(i, Color(tint.r, tint.g, tint.b, motion))


## Show only the MultiMeshes with something to draw.
static func _count(node: MultiMeshInstance3D, count: int) -> void:
	node.multimesh.visible_instance_count = count
	node.visible = count > 0


## Whether a point (planet frame) is on this side of the horizon, with a
## margin: things beyond it are neither moved nor drawn.
func _seen(p: Vector3) -> bool:
	return p.dot(_eye_dir) > _see_cos * p.length()


var _eye_dir := Vector3.BACK
var _see_cos := -1.0


var _walker_sets := {} # tool ("" for empty hands) -> MultiMeshInstance3D
## Wild animals, one MultiMesh per species (`_animal_sets`).
var _animal_sets := {}
var _birds: MultiMeshInstance3D
var _herd := []        # per animal: {cell, phase, speed, radius}
var _flocks := []      # per flock: see `_place_birds`
var _wildlife_dirty := true
var _wild_buildings := []
## Which animals live where, and their coats (a fox on the ice is white).
const SPECIES := {
	"forest": ["deer", "deer", "boar"], "plains": ["deer", "fox", "sheep", "sheep"], "mountain": ["goat"],
	"ice": ["fox", "penguin", "penguin"], "desert": ["camel"], "coast": ["crab"],
}
const SPECIES_TINTS := {
	"deer": [Color("8f6f5a"), Color("b58e6b")], "boar": [Color("5b4a42"), Color("6b5a4e")],
	"goat": [Color("e8e2d6"), Color("b0a6ad")], "sheep": [Color("eef3f2"), Color("e3d9c8")],
	"fox": [Color("e08f6c"), Color("c07a54")], "camel": [Color("d9b48f"), Color("c8a46e")],
	"crab": [Color.WHITE], "penguin": [Color.WHITE],
}
## The hunters' quarry.
const QUARRY := "deer"


## What a crew carries: spears, then bows, then muskets to hunt in the
## woods; baskets on the meadows; rods on the beach and nets at sea; axes
## for wood and picks for stone.
static func _tool_for(resource: String, biome: String, era: int) -> String:
	if resource == "food":
		match biome:
			"forest":
				return "spear" if era <= 2 else ("bow" if era <= 5 else "musket")
			"coast":
				return "rod"
			"ocean":
				return "net"
			_:
				return "basket"
	if resource == "wood":
		return "axe"
	return "pick"


func _build_walkers() -> void:
	var up := Vector3.UP
	var o := Vector3.ZERO
	var wood := Color("8f6f5a")
	var shapes := {
		"": Callable(),
		"spear": func():
			_put(_box(0.004, 0.075, 0.004), wood, up, o, Vector3(0.01, 0.02, 0.018), 1.0, 0.0, 0.25)
			_put(_cone(0.006, 0.014, 4), Color("b0a6ad"), up, o, Vector3(0.01, 0.062, 0.028)),
		"bow": func():
			for k in 3:
				_put(_box(0.003, 0.024, 0.004), wood, up, o, Vector3(0.012 - absf(k - 1) * 0.005, 0.0 + k * 0.022, 0.018))
			_put(_box(0.001, 0.06, 0.001), Color("eef3f2"), up, o, Vector3(0.002, 0.022, 0.018)),
		"musket": func():
			_put(_box(0.004, 0.065, 0.004), Color("4a3b33"), up, o, Vector3(0.006, 0.02, 0.018), 1.0, 0.0, 0.3),
		"rod": func():
			_put(_box(0.003, 0.09, 0.003), wood, up, o, Vector3(0.025, 0.04, 0.012), 1.0, PI / 2.0, 0.7)
			_put(_box(0.001, 0.05, 0.001), Color("eef3f2"), up, o, Vector3(0.06, 0.02, 0.012)),
		"net": func():
			_put(_box(0.03, 0.022, 0.006), Color("d9c8b0"), up, o, Vector3(-0.01, 0.02, -0.014))
			_put(_box(0.032, 0.002, 0.007), Color("8f6f5a"), up, o, Vector3(-0.01, 0.031, -0.014)),
		"basket": func():
			_put(_cylinder(0.01, 0.014, 6), Color("c8a46e"), up, o, Vector3(0.0, -0.004, 0.02)),
		# The named villagers' hats, by trade.
		"hat_food": func():
			_put(_cylinder(0.021, 0.003, 8), Color("f2c078"), up, o, Vector3(0, 0.037, 0))
			_put(_cone(0.011, 0.012, 6), Color("d9b48f"), up, o, Vector3(0, 0.044, 0)),
		"hat_materials": func():
			_put(_box(0.021, 0.008, 0.021), Color("a35d5d"), up, o, Vector3(0, 0.038, 0))
			_put(_box(0.014, 0.002, 0.012), Color("7a4f4f"), up, o, Vector3(0, 0.035, 0.014)),
		"hat_knowledge": func():
			_put(_cone(0.013, 0.03, 6), Color("3f6e9e"), up, o, Vector3(0, 0.046, 0))
			_put(_ball(0.003), Color("f5d76e"), up, o, Vector3(0, 0.061, 0)),
		"hat_culture": func():
			_put(_cylinder(0.012, 0.007, 7), Color("e08f6c"), up, o, Vector3(0, 0.039, 0))
			_put(_box(0.003, 0.022, 0.007), Color("eef3f2"), up, o, Vector3(0.008, 0.05, -0.004), 1.0, 0.0, -0.4),
		"axe": func():
			_put(_box(0.003, 0.045, 0.003), wood, up, o, Vector3(0.008, 0.01, 0.018), 1.0, 0.0, 0.3)
			_put(_box(0.012, 0.01, 0.003), Color("b0a6ad"), up, o, Vector3(0.012, 0.032, 0.024)),
		"pick": func():
			_put(_box(0.003, 0.045, 0.003), wood, up, o, Vector3(0.008, 0.01, 0.018), 1.0, 0.0, 0.3)
			_put(_box(0.024, 0.004, 0.004), Color("857c8a"), up, o, Vector3(0.012, 0.032, 0.024)),
	}
	# A person (the Blender model: walking legs and arms, a tunic in the
	# instance's colour) with what it carries.
	for tool in shapes:
		_walker_sets[tool] = _creature_set(_creature_mesh("person", shapes[tool]), MAX_WALKERS)


func _walker_mm(w: Dictionary) -> MultiMesh:
	return (_walker_sets[w.get("tool", "")] as MultiMeshInstance3D).multimesh


## Wild animals on the free land (deer and boars in the woods, goats on the
## mountains, foxes on the ice...), wandering slowly around their cell and
## running off when people come close, and the birds (see `_place_birds`).
func _place_animals(biomes: Array, buildings: Array) -> void:
	_herd.clear()
	var rng := RandomNumberGenerator.new()
	rng.seed = 77
	var chance := {"forest": 0.35, "plains": 0.16, "mountain": 0.22, "ice": 0.12, "desert": 0.08, "coast": 0.1}
	var counts := {}
	for i in biomes.size():
		var biome := str(biomes[i])
		var roll := rng.randf()
		var pick := rng.randi()
		if not chance.has(biome) or str(buildings[i]) != "" or roll >= float(chance[biome]):
			continue
		if _herd.size() >= MAX_ANIMALS:
			break
		var kinds: Array = SPECIES[biome]
		var species: String = kinds[pick % kinds.size()]
		var coats: Array = SPECIES_TINTS[species]
		var coat: Color = Color("f4f6f8") if species == "fox" and biome == "ice" else coats[(pick >> 8) % coats.size()]
		var index: int = counts.get(species, 0)
		counts[species] = index + 1
		var slow := 0.5 if species in ["crab", "penguin"] else 1.0
		_herd.append({"cell": i, "set": species, "i": index, "coat": coat, "phase": rng.randf() * TAU,
			"speed": (0.05 + rng.randf() * 0.08) * slow * (1.0 if rng.randf() < 0.5 else -1.0),
			"radius": (0.06 + rng.randf() * 0.1), "run": 0.0, "shown": true})
	_herd_counts = counts
	for a in _herd:
		_look((_animal_sets[a["set"]] as MultiMeshInstance3D).multimesh, a["i"], a["coat"], 0.4)
	_place_birds(biomes, rng)
	_move_animals()


var _herd_counts := {}


## The herd, plus one quarry per hunting crew (it grazes, bolts when the
## hunters come, falls, and is carried home). Animals beyond the horizon are
## hidden and not moved.
func _move_animals() -> void:
	var size := _tile_size() * 1.3
	var delta := get_process_delta_time()
	var hidden := Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO)
	# From afar each animal moves every other frame.
	var halves := _closeness < 0.25
	for k in _herd.size():
		var a: Dictionary = _herd[k]
		if halves and (k + _tick) % 2 == 1:
			continue
		var mm: MultiMesh = (_animal_sets[a["set"]] as MultiMeshInstance3D).multimesh
		var cell: int = a["cell"]
		var n: Vector3 = builder.centers[cell]
		if not _seen(n):
			if a["shown"]:
				a["shown"] = false
				mm.set_instance_transform(a["i"], hidden)
			continue
		a["shown"] = true
		var tg := _tangents(n)
		# Someone close: run off for a while.
		var run := float(a["run"])
		if run <= 0.0:
			var here: Vector3 = builder.center_points[cell] if cell < builder.center_points.size() else n
			for p in _people:
				if (p as Vector3).distance_squared_to(here) < 0.0016:
					a["run"] = 2.5
					run = 2.5
					break
		if run > 0.0:
			var step := delta * (2.0 if halves else 1.0)
			a["run"] = run - step
			a["phase"] = float(a["phase"]) + step * 3.5 * signf(float(a["speed"]))
		# Graze a while, then amble on.
		var t := _seconds * float(a["speed"])
		var angle := float(a["phase"]) + t + 0.6 * sin(t * 3.0)
		var r := float(a["radius"]) * _ts
		var at := builder.ground(n + (tg[0] * cos(angle) + tg[1] * sin(angle)) * r, cell)
		var forward: Vector3 = (-tg[0] * sin(angle) + tg[1] * cos(angle)) * signf(float(a["speed"]))
		if run > 0.0:
			at += at.normalized() * absf(sin(_seconds * 14.0 + k)) * 0.004 * _ts
		mm.set_instance_transform(a["i"], _safe(_upright(at, forward, size), "animal"))
		_look(mm, a["i"], a["coat"], 1.0 if run > 0.0 else 0.35 + 0.3 * absf(cos(t * 3.0)))
	# Quarries of the hunting crews.
	var quarry: MultiMesh = (_animal_sets[QUARRY] as MultiMeshInstance3D).multimesh
	var first: int = _herd_counts.get(QUARRY, 0)
	var q := 0
	for route in _routes:
		if not route["tool"] in HUNTING or q >= MAX_QUARRIES:
			continue
		var index := first + q
		q += 1
		var walk := float(route["walk"])
		var work := float(route["work"])
		var t := fmod(_seconds + float(route["offset0"]), walk * 2.0 + work)
		if t >= walk + work or not _seen(builder.centers[int(route["cell"])]):
			quarry.set_instance_transform(index, hidden)
			continue
		var wt := clampf(t - walk, 0.0, work - CATCH_SECONDS)
		var at := _quarry_pos(route, wt)
		var ahead := _quarry_pos(route, wt + 0.1)
		var xform := _upright(at, ahead - at, size * 1.1)
		var motion := 1.0
		if t < walk:
			# Grazing, unaware.
			xform = _upright(at, (ahead - at).rotated(at.normalized(), sin(_seconds) * 1.5), size * 1.1)
			motion = 0.3
		elif t - walk >= work - CATCH_SECONDS:
			# Caught: it lies on its side.
			var b := xform.basis.orthonormalized()
			xform = Transform3D(b.rotated(b.z.normalized(), PI / 2.0).scaled(Vector3.ONE * size * 1.1), at + at.normalized() * 0.004)
			motion = 0.0
		else:
			xform.origin += at.normalized() * absf(sin(_seconds * 16.0)) * 0.005 * _ts
		quarry.set_instance_transform(index, _safe(xform, "quarry"))
		_look(quarry, index, Color("8f6f5a"), motion)
	for species in _animal_sets:
		_count(_animal_sets[species], _herd_counts.get(species, 0) + (q if species == QUARRY else 0))


## A model standing on the ground at `at`, facing `forward`.
func _upright(at: Vector3, forward: Vector3, size: float) -> Transform3D:
	return Transform3D(_ride_basis(forward, at.normalized(), size), at)


# ---------------------------------------------------------------- work

const HUNTING := ["spear", "bow", "musket"]
const HUNT_SECONDS := 8.0
const CATCH_SECONDS := 1.6
const MAX_QUARRIES := 12


## Where the quarry is `s` seconds into the hunt: it bolts in a widening,
## swerving arc across the cell.
func _quarry_pos(route: Dictionary, s: float) -> Vector3:
	var cell: int = route["cell"]
	var n: Vector3 = builder.centers[cell]
	var tg := _tangents(n)
	var angle := float(route["a0"]) + s * 0.8 + 0.25 * sin(s * 3.0)
	var r := (0.04 + 0.022 * s) * _ts
	return builder.ground(n + (tg[0] * cos(angle) + tg[1] * sin(angle)) * r, cell)


## How far from a cell's centre (along `sea`, a tangent) the beach meets
## the water: where a fisher stands.
func _shore_reach(cell: int, sea: Vector3) -> float:
	var n: Vector3 = builder.centers[cell]
	var d := 0.0
	for k in 24:
		var next := d + 0.006
		if builder.ground(n + sea * next).length() <= PlanetMesh.SEA_LEVEL + 0.0015:
			return d
		d = next
	return d


## The way to the open sea from a cell (along the ground), or zero.
func _sea_dir(cell: int) -> Vector3:
	if cell >= builder.neighbors.size() or _biomes.is_empty():
		return Vector3.ZERO
	var n: Vector3 = builder.centers[cell]
	var dir := Vector3.ZERO
	for m in builder.neighbors[cell]:
		if m < _biomes.size() and str(_biomes[m]) == "ocean":
			dir += builder.centers[m] - n
	if dir.length_squared() < 1e-10:
		return Vector3.ZERO
	return (dir - n * dir.dot(n)).normalized()


## How a crew member works `wt` seconds after reaching the cell: hunters
## chase the quarry (arrows fly, muskets smoke), fishers cast from the
## shore and fish jump, woodcutters and quarrymen swing at a tree or a rock
## and chips fly, gatherers stoop over the bushes.
func _work_pose(route: Dictionary, w: Dictionary, wt: float) -> Dictionary:
	var points: PackedVector3Array = route["points"]
	var end := points[points.size() - 1]
	var cell: int = route["cell"]
	var n: Vector3 = builder.centers[cell]
	var tg := _tangents(n)
	var m := int(w.get("member", 0))
	var tool: String = route["tool"]
	var leader: bool = w.get("leader", false)
	var last := float(w.get("wt", -1.0))
	w["wt"] = wt
	if tool in HUNTING:
		var work := float(route["work"])
		var q := _quarry_pos(route, minf(wt, work - CATCH_SECONDS))
		var at: Vector3
		if wt < work - CATCH_SECONDS:
			at = _quarry_pos(route, maxf(0.0, wt - 0.7 - 0.3 * m))
			at = at + (tg[0] * cos(m * 2.0) + tg[1] * sin(m * 2.0)) * 0.012 * _ts
		else:
			# Gathered round the catch.
			var a := float(m) * TAU / 4.0
			at = builder.ground(q + (tg[0] * cos(a) + tg[1] * sin(a)) * 0.025 * _ts, cell)
		at = builder.ground(at, cell)
		var hunting := wt < work - CATCH_SECONDS
		if leader and hunting:
			if tool == "bow" and floor(wt / 1.1) != floor(last / 1.1) and wt > 0.6:
				var from := at + at.normalized() * 0.012 * _ts
				var dir := (q - from)
				_spawn(from, dir.normalized() * 0.12 * _ts, Color("6b5a4e"), dir.length() / (0.12 * _ts), 0.004 * _ts, 0.004 * _ts, 0.0, true)
			elif tool == "musket" and floor(wt / 2.2) != floor(last / 2.2) and wt > 0.6:
				var muzzle := at + at.normalized() * 0.015 * _ts + (q - at).normalized() * 0.01 * _ts
				for k in 4:
					_spawn(muzzle, (q - at).normalized() * 0.01 * _ts + at.normalized() * 0.01 * _ts + _jitter(0.006 * _ts), Color("eef3f2"), 1.2, 0.004 * _ts, 0.012 * _ts)
			elif tool == "spear" and wt < work - CATCH_SECONDS and wt + get_process_delta_time() >= work - CATCH_SECONDS - 0.4:
				var from := at + at.normalized() * 0.015 * _ts
				var dir := (q - from)
				_spawn(from, dir.normalized() * 0.1 * _ts, Color("8f6f5a"), dir.length() / (0.1 * _ts), 0.005 * _ts, 0.005 * _ts, 0.0, true)
		return {"at": at, "forward": (q - at) if (q - at).length_squared() > 1e-12 else tg[0], "tilt": -0.25 if hunting else 0.0}
	if tool == "rod":
		var sea: Vector3 = route["sea_dir"]
		if sea == Vector3.ZERO:
			sea = tg[0]
		var side := sea.cross(n).normalized()
		if not route.has("reach"):
			route["reach"] = _shore_reach(cell, sea)
		var reach := float(route["reach"])
		var at := builder.ground(n + sea * reach + side * (m - 1) * 0.03 * _ts)
		var bob := (n + sea * (reach + 0.07 * _ts)).normalized() * PlanetMesh.SEA_LEVEL
		bob += bob.normalized() * (0.001 + 0.0015 * sin(_seconds * 3.0 + m))
		var tip := at + at.normalized() * 0.03 * _ts + sea * 0.025 * _ts
		_fx_line(tip, bob, Color("eef3f2"), 0.0008 * _ts)
		_fx_static.append({"p": bob, "s": 0.005 * _ts, "c": Color("e08f6c")})
		# Now and then a fish jumps at the float.
		var period := 2.4 + m * 0.7
		if floor((wt + m) / period) != floor((last + m) / period) and last >= 0.0:
			_spawn(bob, bob.normalized() * 0.07 * _ts - sea * 0.02 * _ts, Color("c8d8e0"), 0.9, 0.006 * _ts, 0.006 * _ts, 0.16 * _ts, true)
			_splash(bob, 0.4)
		return {"at": at, "forward": sea, "tilt": 0.1 * sin(_seconds * 0.8 + m)}
	if tool == "axe" or tool == "pick":
		var a := float(route["a0"]) + m * 2.1
		var spot := builder.ground(n + (tg[0] * cos(a) + tg[1] * sin(a)) * 0.07 * _ts, cell)
		var dir := (spot - end)
		dir = (dir - spot.normalized() * dir.dot(spot.normalized())).normalized()
		var at := builder.ground(spot - dir * 0.025 * _ts, cell)
		var swing := sin(wt * 6.0 + m)
		var prev := sin(last * 6.0 + m)
		if prev > 0.0 and swing <= 0.0 and last >= 0.0:
			var color := Color("c8a46e") if tool == "axe" else Color("b0a6ad")
			for k in 3:
				_spawn(spot + spot.normalized() * 0.012 * _ts, (spot.normalized() * 0.03 + _jitter(0.03)) * _ts, color, 0.6, 0.003 * _ts, 0.002 * _ts, 0.12 * _ts)
			sfx.emit("chop" if tool == "axe" else "hammer", pivot.to_global(spot))
			if tool == "axe" and randf() < 0.3:
				_spawn(spot + spot.normalized() * 0.11 * _ts + _jitter(0.02 * _ts), _jitter(0.004) * _ts, Color("6f9163"), 2.5, 0.004 * _ts, 0.003 * _ts, 0.012 * _ts)
		return {"at": at, "forward": dir, "tilt": 0.45 * swing}
	if tool == "basket":
		var a := float(route["a0"]) + m * 1.9 + wt * 0.25
		var at := builder.ground(n + (tg[0] * cos(a) + tg[1] * sin(a)) * 0.06 * _ts, cell)
		var stoop := 0.7 + 0.3 * absf(cos(wt * 1.6 + m))
		if stoop < 0.75 and randf() < 0.03:
			_spawn(at + at.normalized() * 0.006 * _ts, (at.normalized() * 0.02 + _jitter(0.01)) * _ts, Color("b03a2e"), 0.5, 0.003 * _ts, 0.003 * _ts, 0.1 * _ts)
		return {"at": at, "forward": -tg[0] * sin(a) + tg[1] * cos(a), "crouch": stoop}
	return {"at": end, "forward": tg[0]}


# ---------------------------------------------------------------- little effects

## Small things that fly, fall, rise or splash: one MultiMesh of tinted
## cubes. `_fx_static` holds what is drawn this frame only (lines, floats);
## `_parts` are particles with a life.
const MAX_FX := 500
var _fx: MultiMeshInstance3D
var _parts := []
var _fx_static := []
var _people := []
var _smoke := []       # [position, next puff time]
const SMOKY := {"hut": 0.15, "house": 0.17, "fire_pit": 0.09, "hunting_camp": 0.06, "colony": 0.14, "foundry": 0.25, "villa": 0.16, "manor": 0.16, "townhouse": 0.17}


func _spawn(p: Vector3, v: Vector3, c: Color, life: float, s0: float, s1: float, gravity := 0.0, stretch := false) -> void:
	if _parts.size() >= MAX_FX - 60:
		return
	_parts.append({"p": p, "v": v, "c": c, "life": 0.0, "max": life, "s0": s0, "s1": s1, "g": gravity, "stretch": stretch})


static func _jitter(r: float) -> Vector3:
	return Vector3(randf_range(-r, r), randf_range(-r, r), randf_range(-r, r))


## A ring of drops on the water.
func _splash(at: Vector3, weight := 1.0) -> void:
	var n := at.normalized()
	var tg := _tangents(n)
	for k in 6:
		var a := k * TAU / 6.0
		var out: Vector3 = (tg[0] * cos(a) + tg[1] * sin(a))
		_spawn(at, (out * 0.02 + n * 0.03) * _ts, Color("eef3f2"), 0.5, 0.003 * _ts, 0.002 * _ts, 0.12 * _ts)
	if weight > 0.0:
		sfx.emit("splash", pivot.to_global(at))


## A thin straight line for this frame (a fishing line).
func _fx_line(a: Vector3, b: Vector3, c: Color, width: float) -> void:
	_fx_static.append({"p": (a + b) / 2.0, "dir": b - a, "s": width, "c": c})


func _update_fx(delta: float) -> void:
	if _fx == null:
		return
	# Smoke from hearths and chimneys.
	for k in _smoke.size():
		var sm: Array = _smoke[k]
		if _seconds >= float(sm[1]):
			sm[1] = _seconds + randf_range(0.9, 1.6)
			var at: Vector3 = sm[0]
			_spawn(at, at.normalized() * 0.03 * _ts + _jitter(0.004 * _ts), Color("d9d0cc"), 3.0, 0.004 * _ts, 0.014 * _ts, -0.004 * _ts)
	var mm := _fx.multimesh
	var i := 0
	for st in _fx_static:
		if i >= MAX_FX:
			break
		var p: Vector3 = st["p"]
		if st.has("dir"):
			mm.set_instance_transform(i, _safe(_stretched(p, st["dir"], float(st["s"])), "fx line"))
		else:
			mm.set_instance_transform(i, _safe(_upright(p, _tangents(p.normalized())[0], float(st["s"])), "fx dot"))
		mm.set_instance_color(i, st["c"])
		i += 1
	var k := 0
	while k < _parts.size():
		var part: Dictionary = _parts[k]
		part["life"] = float(part["life"]) + delta
		if float(part["life"]) >= float(part["max"]):
			_parts.remove_at(k)
			continue
		var p: Vector3 = part["p"]
		var v: Vector3 = part["v"]
		v -= p.normalized() * float(part["g"]) * delta
		p += v * delta
		part["p"] = p
		part["v"] = v
		k += 1
		if i >= MAX_FX:
			continue
		var f := float(part["life"]) / float(part["max"])
		var size := lerpf(float(part["s0"]), float(part["s1"]), f)
		if part["stretch"]:
			mm.set_instance_transform(i, _safe(_stretched(p, v.normalized() * size * 3.0, size), "fx streak"))
		else:
			mm.set_instance_transform(i, _safe(_upright(p, _tangents(p.normalized())[0], size), "fx puff"))
		var c: Color = part["c"]
		mm.set_instance_color(i, c)
		i += 1
	mm.visible_instance_count = i


## A cube stretched along `dir` (its length), `width` across.
func _stretched(centre: Vector3, dir: Vector3, width: float) -> Transform3D:
	var length := dir.length()
	if length < 1e-6:
		return _upright(centre, _tangents(centre.normalized())[0], width)
	var z := dir / length
	var up := centre.normalized()
	var x := up.cross(z)
	if x.length_squared() < 1e-8:
		x = _tangents(z)[0]
	x = x.normalized()
	var y := z.cross(x)
	return Transform3D(Basis(x * width, y * width, z * length), centre)


# ---------------------------------------------------------------- birds

## Flocks: songbirds over the forests and gulls along the coasts. A flock
## flies to a cell, settles (songbirds on the treetops, gulls on the water
## and the shore), rests a while, and takes off again when its time is up
## or when people come close, to settle somewhere else nearby.
const MAX_BIRDS := 32
var _wings: MultiMeshInstance3D
var _forest_cells := []
var _gull_cells := []


func _build_birds_and_fx() -> void:
	var up := Vector3.UP
	var o := Vector3.ZERO
	# Bodies and wings (the right wing, mirrored for the left one) from
	# the Blender models, tinted per bird.
	_birds = _creature_set(_creature_mesh("bird"), MAX_BIRDS)
	_wings = _creature_set(_creature_mesh("bird_wing"), MAX_BIRDS * 2)
	_fx = MultiMeshInstance3D.new()
	var fx := MultiMesh.new()
	fx.transform_format = MultiMesh.TRANSFORM_3D
	fx.use_colors = true
	var cube := BoxMesh.new()
	cube.size = Vector3.ONE
	fx.mesh = cube
	_fixed(fx, MAX_FX)
	_fx.multimesh = fx
	var fx_material := StandardMaterial3D.new()
	fx_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	fx_material.vertex_color_use_as_albedo = true
	_fx.material_override = fx_material
	pivot.add_child(_fx)


func _place_birds(biomes: Array, rng: RandomNumberGenerator) -> void:
	_flocks.clear()
	_forest_cells.clear()
	_gull_cells.clear()
	for i in biomes.size():
		var b := str(biomes[i])
		if b == "forest":
			_forest_cells.append(i)
		elif b == "coast":
			_gull_cells.append(i)
	var used := 0
	for f in 4:
		var gulls := f == 3
		var cells: Array = _gull_cells if gulls else _forest_cells
		if cells.is_empty():
			continue
		var count := 5 if gulls else 7
		if used + count > MAX_BIRDS:
			break
		var cell: int = cells[rng.randi() % cells.size()]
		var flock := {"gulls": gulls, "cell": cell, "state": "perch", "timer": rng.randf_range(2.0, 12.0), "pos": Vector3.ZERO, "birds": [], "first": used}
		var color: Color = Color("eef3f2") if gulls else [Color("5b4a42"), Color("4a4152"), Color("8f6f5a")][f % 3]
		for b in count:
			flock["birds"].append({"off": Vector3(rng.randf_range(-1, 1), 0, rng.randf_range(-1, 1)), "phase": rng.randf() * TAU, "p": Vector3.ZERO, "h": Vector3.ZERO, "perch": Vector3.ZERO, "from": Vector3.ZERO, "c": color.lightened(rng.randf() * 0.15)})
		_settle(flock, rng)
		for bird in flock["birds"]:
			bird["p"] = bird["perch"]
		flock["pos"] = builder.ground(builder.centers[cell], cell)
		_flocks.append(flock)
		used += count
	_count(_birds, used)
	_count(_wings, used * 2)
	var k := 0
	for flock in _flocks:
		for bird in flock["birds"]:
			_look(_birds.multimesh, k, bird["c"], 0.0)
			_look(_wings.multimesh, k * 2, bird["c"], 0.0)
			_look(_wings.multimesh, k * 2 + 1, bird["c"], 0.0)
			k += 1


## Pick each bird's resting spot on the flock's cell: treetops, or the
## water and the beach for gulls.
func _settle(flock: Dictionary, rng: RandomNumberGenerator) -> void:
	var cell: int = flock["cell"]
	var n: Vector3 = builder.centers[cell]
	var tg := _tangents(n)
	for bird in flock["birds"]:
		var a := rng.randf() * TAU
		var r := rng.randf_range(0.03, 0.17) * _ts
		var d: Vector3 = n + (tg[0] * cos(a) + tg[1] * sin(a)) * r
		var g := builder.ground(d, cell)
		if flock["gulls"]:
			bird["perch"] = g.normalized() * maxf(g.length(), PlanetMesh.SEA_LEVEL) + g.normalized() * 0.003 * _ts
		else:
			bird["perch"] = g + g.normalized() * rng.randf_range(0.13, 0.19) * _ts


func _move_birds(delta: float) -> void:
	if _flocks.is_empty():
		return
	var size := _tile_size() * 0.8
	var rng := RandomNumberGenerator.new()
	rng.seed = int(_seconds * 1000.0)
	var k := 0
	for flock in _flocks:
		var cell: int = flock["cell"]
		var n: Vector3 = builder.centers[cell]
		var home := builder.ground(n, cell)
		var state: String = flock["state"]
		flock["timer"] = float(flock["timer"]) - delta
		match state:
			"perch":
				var scared := false
				for p in _people:
					if (p as Vector3).distance_squared_to(home) < 0.0036:
						scared = true
						break
				if scared or float(flock["timer"]) <= 0.0:
					# Off to another spot nearby.
					var cells: Array = _gull_cells if flock["gulls"] else _forest_cells
					var best := cell
					for tries in 12:
						var c: int = cells[rng.randi() % cells.size()]
						var d := n.angle_to(builder.centers[c])
						if c != cell and d > 0.08 and d < 0.45:
							best = c
							break
					flock["cell"] = best
					flock["state"] = "fly"
					flock["timer"] = 0.0
					flock["start"] = flock["pos"]
					flock["leg"] = 0.0
					sfx.emit("flutter", pivot.to_global(home))
					for bird in flock["birds"]:
						bird["from"] = bird["p"]
			"fly":
				var target: Vector3 = builder.centers[cell]
				var start: Vector3 = flock["start"]
				var span := maxf(0.05, start.normalized().angle_to(target))
				var leg := minf(1.0, float(flock["leg"]) + delta * 0.06 * _ts * 10.0 / span)
				flock["leg"] = leg
				var dir := start.normalized().slerp(target, leg)
				var ground := builder.ground(dir).length()
				flock["pos"] = dir * (ground + (0.08 + 0.25 * sin(PI * leg)) * _ts)
				if leg >= 1.0:
					flock["state"] = "land"
					flock["timer"] = 1.6
					_settle(flock, rng)
					for bird in flock["birds"]:
						bird["from"] = bird["p"]
			"land":
				if float(flock["timer"]) <= 0.0:
					flock["state"] = "perch"
					flock["timer"] = rng.randf_range(6.0, 16.0)
		var pos: Vector3 = flock["pos"]
		if not _seen(pos):
			# Beyond the horizon: the flock keeps its plans, unseen.
			if flock.get("shown", true):
				flock["shown"] = false
				var gone := Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO)
				for m in flock["birds"].size():
					_birds.multimesh.set_instance_transform(k + m, gone)
					_wings.multimesh.set_instance_transform((k + m) * 2, gone)
					_wings.multimesh.set_instance_transform((k + m) * 2 + 1, gone)
			k += flock["birds"].size()
			continue
		flock["shown"] = true
		var up := pos.normalized()
		var tg := _tangents(up)
		for bird in flock["birds"]:
			var p: Vector3 = bird["p"]
			var target: Vector3
			var flapping := true
			match flock["state"]:
				"fly":
					var off: Vector3 = bird["off"]
					var phase := float(bird["phase"])
					target = pos + (tg[0] * off.x + tg[1] * off.z) * 0.035 * _ts + up * sin(_seconds * 2.0 + phase) * 0.008 * _ts
					p = p.lerp(target, minf(1.0, delta * 3.0))
				"land":
					var f := 1.0 - clampf(float(flock["timer"]) / 1.6, 0.0, 1.0)
					f = f * f * (3.0 - 2.0 * f)
					p = (bird["from"] as Vector3).lerp(bird["perch"], f)
				_:
					flapping = false
					p = bird["perch"]
					# A little hop now and then.
					p += (bird["perch"] as Vector3).normalized() * maxf(0.0, sin(_seconds * 1.3 + float(bird["phase"]) * 3.0) - 0.9) * 0.03 * _ts
			var heading: Vector3 = p - (bird["p"] as Vector3)
			if heading.length_squared() > 1e-12 and flapping:
				bird["h"] = heading.normalized()
			elif (bird["h"] as Vector3) == Vector3.ZERO or not flapping:
				if not flapping and fmod(_seconds + float(bird["phase"]), 4.0) < delta:
					bird["h"] = tg[0].rotated(up, rng.randf() * TAU)
				elif (bird["h"] as Vector3) == Vector3.ZERO:
					bird["h"] = tg[0]
			bird["p"] = p
			var body := _upright(p, bird["h"], size)
			_birds.multimesh.set_instance_transform(k, _safe(body, "bird"))
			# Wings: beating in flight (gulls glide between beats),
			# folded flat along the body at rest.
			var flap := -0.15
			if flapping:
				var beat := sin(_seconds * (9.0 if flock["gulls"] else 20.0) + float(bird["phase"]))
				flap = (0.25 + 0.55 * beat) if flock["gulls"] else 0.9 * beat
			var b := body.basis
			var fwd := b.z.normalized()
			var right := Basis(fwd, flap) * b
			var left := Basis(fwd, -flap) * b.rotated(b.y.normalized(), PI)
			if not flapping:
				right = right * Basis.from_scale(Vector3(0.45, 1.0, 1.0))
				left = left * Basis.from_scale(Vector3(0.45, 1.0, 1.0))
			_wings.multimesh.set_instance_transform(k * 2, _safe(Transform3D(right, p), "wing"))
			_wings.multimesh.set_instance_transform(k * 2 + 1, _safe(Transform3D(left, p), "wing"))
			k += 1


const MAX_RIDES := 32
const MAX_ANIMALS := 140


## Give a MultiMesh a fixed number of instances, all hidden: only
## `visible_instance_count` changes afterwards (see `_build_walkers`).
static func _fixed(mm: MultiMesh, count: int) -> void:
	mm.instance_count = count
	for k in count:
		mm.set_instance_transform(k, Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO))
	mm.visible_instance_count = 0


var _unsafe_warned := {}


## Never draw anything far off the planet or bent out of shape (warn once
## per kind, for the console).
func _safe(t: Transform3D, kind: String) -> Transform3D:
	var scale := t.basis.get_scale()
	if t.origin.length() < 1.3 and t.basis.is_finite() and t.origin.is_finite() and scale.x < 0.6 and scale.y < 0.6 and scale.z < 0.6:
		return t
	if not _unsafe_warned.has(kind):
		_unsafe_warned[kind] = true
		push_warning("oorbit: %s off the planet or out of shape: %s" % [kind, t])
	return Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO)


# ---------------------------------------------------------------- sky and festivals

## The planet's material (planet.gdshader): `leaf` and `bloom` say how much
## the season shows on it.
static func _planet_material(leaf: float, bloom: float) -> ShaderMaterial:
	var m := ShaderMaterial.new()
	m.shader = load("res://planet.gdshader")
	m.set_shader_parameter("leaf_strength", leaf)
	m.set_shader_parameter("bloom_strength", bloom)
	return m


## The sun in the planet's frame, and night where it is down.
var sun_local := Vector3(0, 0, 1)
var _night_at_fest := 0.0


## Time of day and season: `sun` points at the sun in the planet's frame,
## `season` holds the amounts the shader mixes in (snow, autumn, bare,
## bloom).
func set_sky(sun: Vector3, season: Dictionary) -> void:
	sun_local = sun.normalized()
	var sun_world := pivot.global_basis * sun_local
	for m in [_props_material, _land_material]:
		if m == null:
			continue
		m.set_shader_parameter("sun_dir", sun_world)
		m.set_shader_parameter("axis", pivot.global_basis.y)
		m.set_shader_parameter("centre", pivot.global_position)
		for k in season:
			m.set_shader_parameter(k, season[k])
	_summer = float(season.get("summer", 0.0))


var _summer := 0.0


## How dark it is at a point of the planet (0 day, 1 night).
func night_at(p: Vector3) -> float:
	return clampf((0.12 - p.normalized().dot(sun_local)) / 0.3, 0.0, 1.0)


const FEST_COLORS := [Color("e08f6c"), Color("f2c078"), Color("4a7a8c"), Color("e3c3c6"), Color("c2d69b"), Color("9b8ec4")]
## What each festival puts up around the bonfire.
const FEST_PROPS := {
	"flowers": ["maypole", "bunting", "feast_table"],
	"sun": ["bunting", "feast_table", "lantern_pole"],
	"lanterns": ["lantern_pole", "lantern_pole", "feast_table"],
	"stars": ["lantern_pole", "bunting", "lantern_pole"],
	"anniversary": ["bunting", "feast_table", "maypole"],
	"harvest": ["feast_table", "bunting"],
	"harvest_grand": ["feast_table", "feast_table", "bunting", "lantern_pole"],
	"games": ["bunting", "bunting"],
	"games_grand": ["bunting", "bunting", "lantern_pole", "lantern_pole"],
	"wedding": ["bunting", "feast_table", "bunting", "lantern_pole"],
}
## Festivals with fireworks once it is dark.
const FIREWORKS := ["sun", "stars", "anniversary", "harvest_grand", "games_grand"]

## The festival being drawn ("" for none), where, and its node.
var festival := ""
var _fest_cell := -1
var _fest_at := Vector3.ZERO
var _fest_node: MeshInstance3D
var _fest_next := 0.0
var _last_view := []
var _firefly_next := 0.0


## The cell of the festival under way, or -1.
func fest_cell() -> int:
	return _fest_cell


## Start or end the festival `id` ("" for none): the bonfire, the
## decorations and the dancers go up on a free cell next to the home.
func set_festival(id: String) -> void:
	if id == festival:
		return
	festival = id
	if _fest_node != null:
		_fest_node.queue_free()
		_fest_node = null
	_fest_cell = -1
	if id == "" or _home < 0 or _home >= builder.centers.size():
		_dance()
		return
	# The village green: the first free meadow (or any free land) one or
	# two cells from the home.
	_fest_cell = _home
	var ring := Array(builder.neighbors[_home])
	for n in builder.neighbors[_home]:
		for m in builder.neighbors[n]:
			if m != _home and not m in ring:
				ring.append(m)
	var best := 99
	for n in ring:
		if n >= _biomes.size() or n >= _town.size() or _town[n] or str(_biomes[n]) in ["ocean", "mountain"]:
			continue
		var rank := 0 if str(_biomes[n]) == "plains" else 1
		if rank < best:
			best = rank
			_fest_cell = n
	var normal: Vector3 = builder.centers[_fest_cell]
	var fit := _ground_plane(_fest_cell, normal, builder.ground(normal, _fest_cell))
	var up: Vector3 = fit[0]
	var footing: Vector3 = fit[1]
	_fest_at = footing
	var saved := [_verts, _normals, _colors, _indices, _lod, _top]
	_verts = PackedVector3Array()
	_normals = PackedVector3Array()
	_colors = PackedColorArray()
	_indices = PackedInt32Array()
	_lod = false
	_put_model("wedding_arch" if id == "wedding" else "bonfire", up, footing, Vector3.ZERO, 1.2)
	var props: Array = FEST_PROPS.get(id, ["bunting"])
	for k in props.size():
		var a := k * TAU / props.size() + 0.4
		_put_model(props[k], up, footing, Vector3(cos(a) * 0.24, 0.0, sin(a) * 0.24), 1.0, -a + PI / 2.0)
	_fest_node = MeshInstance3D.new()
	_fest_node.mesh = _mesh_of(_verts, _normals, _colors, _indices)
	_fest_node.material_override = _props_material
	pivot.add_child(_fest_node)
	_verts = saved[0]
	_normals = saved[1]
	_colors = saved[2]
	_indices = saved[3]
	_lod = saved[4]
	_top = saved[5]
	_dance()


## The dancers join (or leave) the walkers.
func _dance() -> void:
	if _last_view.size() == 2:
		_place_walkers(_last_view[0], _last_view[1])


## Sparks from the bonfire; after dark fireworks, lanterns rising or
## petals in the air; fireflies over the meadows on summer nights.
func _festival_fx(delta: float) -> void:
	if _fest_cell >= 0 and _seen(_fest_at):
		var n := _fest_at.normalized()
		var top := _fest_at + n * 0.03 * _ts
		if randf() < 0.5 and festival != "wedding":
			_spawn(top + _jitter(0.006 * _ts), n * 0.05 * _ts + _jitter(0.01 * _ts), Color("f5d76e") if randf() < 0.5 else Color("e08f6c"), 1.2, 0.004 * _ts, 0.0)
		_fest_next -= delta
		var dark := night_at(_fest_at)
		if _fest_next <= 0.0:
			_fest_next = randf_range(0.6, 1.4)
			if festival in FIREWORKS and dark > 0.5:
				_firework(n)
			elif festival in ["lanterns", "stars"]:
				for k in 2:
					var t := _tangents(n)
					var at: Vector3 = _fest_at + (t[0] * randf_range(-1, 1) + t[1] * randf_range(-1, 1)) * 0.1 * _ts
					_spawn(at + n * 0.02 * _ts, n * 0.03 * _ts + _jitter(0.004 * _ts), Color("f2c078"), 6.0, 0.006 * _ts, 0.004 * _ts)
			elif festival in ["flowers", "wedding"]:
				for k in 6:
					var t := _tangents(n)
					var at: Vector3 = _fest_at + (t[0] * randf_range(-1, 1) + t[1] * randf_range(-1, 1)) * 0.15 * _ts + n * 0.12 * _ts
					_spawn(at, _jitter(0.01 * _ts) - n * 0.012 * _ts, Color("e3c3c6") if randf() < 0.6 else Color("eef3f2"), 4.0, 0.004 * _ts, 0.003 * _ts)
	# Fireflies: summer nights, close up, over the green land in view.
	_firefly_next -= delta
	if _firefly_next > 0.0 or _closeness < 0.35 or _summer < 0.3:
		return
	_firefly_next = 0.15
	var look := _eye_dir
	var cell := builder.cell_at(look)
	if cell < 0 or cell >= _biomes.size() or not str(_biomes[cell]) in ["plains", "forest"]:
		return
	var t := _tangents(look)
	var at: Vector3 = builder.ground(look + (t[0] * randf_range(-1, 1) + t[1] * randf_range(-1, 1)) * 0.06)
	if night_at(at) < 0.6:
		return
	_spawn(at + at.normalized() * randf_range(0.004, 0.02), _jitter(0.004), Color("e8f59a"), 1.6, 0.0025, 0.0)


## One rocket: a streak up, then a burst of colour falling back.
func _firework(n: Vector3) -> void:
	var t := _tangents(n)
	var at: Vector3 = _fest_at + (t[0] * randf_range(-1, 1) + t[1] * randf_range(-1, 1)) * 0.12 * _ts + n * randf_range(0.2, 0.3) * _ts
	var c: Color = FEST_COLORS[randi() % FEST_COLORS.size()].lightened(0.25)
	_spawn(_fest_at + n * 0.03 * _ts, (at - _fest_at) * 1.4, Color("f5d76e"), 0.6, 0.003 * _ts, 0.002 * _ts, 0.0, true)
	for k in 22:
		var d := _jitter(1.0).normalized()
		var v := d * randf_range(0.07, 0.11) * _ts
		_parts_later.append([_seconds + 0.6, at, v, c])
	sfx.emit("firework", pivot.to_global(at))


## Bursts waiting for their rocket to arrive: [time, at, velocity, colour].
var _parts_later := []


func _launch_bursts() -> void:
	var k := 0
	while k < _parts_later.size():
		var b: Array = _parts_later[k]
		if _seconds >= float(b[0]):
			_spawn(b[1], b[2], b[3], 1.4, 0.005 * _ts, 0.001 * _ts, 0.05 * _ts)
			_parts_later.remove_at(k)
		else:
			k += 1


# ---------------------------------------------------------------- villagers

## A villager is a child this many steps (three planet days).
const CHILD_STEPS := 3 * 144
## The couple being married (from the Chronicle), while a wedding is on.
var _wedding_names := []
## Name labels over the villagers, by person id.
var _name_labels := {}


## One label per named villager (made or dropped as the cast changes).
func _name_labels_for(walk: Array) -> void:
	var wanted := {}
	for k in walk.size():
		var w: Dictionary = walk[k]
		if not w.has("pid"):
			continue
		wanted[w["pid"]] = true
		var label: Label3D = _name_labels.get(w["pid"])
		if label == null:
			label = Label3D.new()
			label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
			label.fixed_size = true
			label.pixel_size = 0.0007
			label.font_size = 22
			label.outline_size = 6
			label.outline_modulate = Palette.OUTLINE
			label.texture_filter = BaseMaterial3D.TEXTURE_FILTER_NEAREST
			label.no_depth_test = false
			label.visible = false
			pivot.add_child(label)
			_name_labels[w["pid"]] = label
		label.text = str(w["name"])
		label.modulate = Color("f5d76e") if w["famous"] else Color("eef3f2")
	for pid in _name_labels.keys():
		if not wanted.has(pid):
			_name_labels[pid].queue_free()
			_name_labels.erase(pid)


## Keep a villager's name over their head, only up close.
func _place_name(w: Dictionary, at: Vector3, size: float) -> void:
	var label: Label3D = _name_labels.get(w["pid"])
	if label == null:
		return
	# Only up close, and at three heights so neighbours' names do not sit
	# on top of each other.
	label.visible = _closeness > 0.7
	label.position = at + at.normalized() * (0.07 + 0.035 * (int(w["pid"]) % 3)) * size
	label.set_meta("index", int(w.get("index", -1)))
	label.set_meta("mm", _walker_mm(w))


## The named villager under a click (screen point of the low-resolution
## view), or -1.
func pick_person(camera: Camera3D, point: Vector2) -> int:
	var best := -1
	var nearest := 9.0
	for w in _walk:
		if not w.has("pid") or not w.has("at") or w["at"] == Vector3.ZERO:
			continue
		var at: Vector3 = w["at"]
		var head := pivot.to_global(at + at.normalized() * 0.02 * _ts)
		if camera.is_position_behind(head):
			continue
		var d := camera.unproject_position(head).distance_to(point)
		if d < nearest:
			nearest = d
			best = int(w["pid"])
	return best


# ---------------------------------------------------------------- wonders

## Small wonders hidden on the planet, one of each kind where its land
## exists (a cell drawn from the planet's own land, so it never moves):
## found by clicking their cell, they glint now and then until then.
const WONDERS := [
	["mushroom_ring", ["forest"]], ["old_oak", ["forest", "plains"]], ["shipwreck", ["coast"]],
	["cave", ["mountain"]], ["totem", ["plains"]], ["whale", ["ocean"]], ["stone_head", ["desert", "coast"]],
	["igloo", ["ice"]], ["bones", ["desert", "plains"]], ["crystal", ["mountain", "plains", "desert", "ice"]],
]
## cell -> kind
var wonders := {}
## Kinds already found (main.gd keeps them per planet).
var found_wonders := {}
var _whale: MeshInstance3D
var _whale_cell := -1
var _glint_left := 1.0


## Pick a cell for each wonder: land of its kind away from the home, drawn
## from the land itself so the same planet always hides them in the same
## places (a building put up there covers it).
func _place_wonders(biomes: Array) -> void:
	wonders.clear()
	var rng := RandomNumberGenerator.new()
	rng.seed = hash(biomes)
	var home: Vector3 = builder.centers[_home] if _home >= 0 and _home < builder.centers.size() else Vector3.UP
	for w in WONDERS:
		var kind: String = w[0]
		var cells := []
		for i in biomes.size():
			if str(biomes[i]) in w[1] and not wonders.has(i) and home.angle_to(builder.centers[i]) > 0.25:
				cells.append(i)
		if cells.is_empty():
			continue
		wonders[cells[rng.randi() % cells.size()]] = kind
	_whale_cell = -1
	for cell in wonders:
		if wonders[cell] == "whale":
			_whale_cell = cell
	if _whale == null:
		_whale = MeshInstance3D.new()
		_whale.mesh = _mesh_of(Models.get_model("whale").near.verts, Models.get_model("whale").near.normals, Models.get_model("whale").near.colors, Models.get_model("whale").near.indices)
		_whale.material_override = _props_material
		pivot.add_child(_whale)
	_whale.visible = _whale_cell >= 0


## The wonder on a cell, or "".
func wonder_at(cell: int) -> String:
	return str(wonders.get(cell, ""))


## The whale surfaces, blows and dives again; undiscovered wonders glint
## when the camera is close enough to see them.
func _move_wonders(delta: float) -> void:
	if _whale != null and _whale_cell >= 0:
		var n: Vector3 = builder.centers[_whale_cell]
		var cycle := fmod(_seconds, 14.0) / 14.0
		var up := sin(cycle * TAU)
		var tg := _tangents(n)
		var heading: Vector3 = tg[0].rotated(n, _seconds * 0.05)
		var at: Vector3 = n * (PlanetMesh.SEA_LEVEL + (up - 0.3) * 0.012 * _ts) + heading * sin(_seconds * 0.05) * 0.06 * _ts
		_whale.visible = up > -0.4 and _seen(at)
		_whale.transform = Transform3D(_ride_basis(heading.cross(n).normalized(), n, _ts * 1.4), at)
	_glint_left -= delta
	if _glint_left > 0.0 or _closeness < 0.2:
		return
	_glint_left = 1.2
	for cell in wonders:
		if found_wonders.has(wonders[cell]):
			continue
		var p := builder.ground(builder.centers[cell], cell)
		if not _seen(p):
			continue
		var n := p.normalized()
		for k in 3:
			_spawn(p + n * (0.03 + randf() * 0.04) * _ts + _jitter(0.025 * _ts), n * 0.01 * _ts, Color("f5d76e"), 0.9, 0.006 * _ts, 0.0)


# ---------------------------------------------------------------- feelings

## How people feel, shown as a coloured dot that pops out of a head: only
## for people on screen. Now and then someone shows the planet's mood (and
## fear under a storm); the player's choices make the people around show
## what they think of it (see `feel`).
const FEELINGS := {
	"joy": Color("f2c94c"), "love": Color("f28bb0"), "pride": Color("8fd16a"),
	"sad": Color("5b8fd9"), "anger": Color("e0524a"), "fear": Color("a77bd9"), "wonder": Color("6fd6d0"),
}
var mood := 0.6
var _camera: Camera3D
var _closeness := 0.0
var _feel_left := 2.0
var _feel_queue := []   # [seconds left, position, kind]


## Where the camera is and how close it has come (0 whole planet, 1 at the
## ground): the weather and the feelings follow it.
func set_camera(camera: Camera3D, closeness: float, pixels_tall := 300.0) -> void:
	_camera = camera
	_closeness = closeness
	if pixels_tall != view_height:
		view_height = pixels_tall
		_culled_from = Vector3.INF
	_cull()
	if weather != null:
		weather.look(pivot.to_local(camera.global_position), closeness)


## The heads on screen right now (planet frame), the ones nearest `near`
## first when it is given.
func _heads(near := Vector3.ZERO) -> Array:
	var out := []
	if _camera == null:
		return out
	var eye := pivot.to_local(_camera.global_position)
	var lift := 0.03 * _tile_size() * 1.2
	for p in _people:
		var head: Vector3 = p + p.normalized() * lift
		if head.normalized().dot((eye - head).normalized()) < 0.05:
			continue
		if not _camera.is_position_in_frustum(pivot.to_global(head)):
			continue
		out.append(head)
	if near != Vector3.ZERO:
		out.sort_custom(func(a: Vector3, b: Vector3): return a.distance_squared_to(near) < b.distance_squared_to(near))
	else:
		out.shuffle()
	return out


## A dot of feeling rising from a head.
func _pop(head: Vector3, kind: String) -> void:
	var up := head.normalized()
	var c: Color = FEELINGS.get(kind, Color.WHITE)
	_spawn(head + up * 0.006 * _ts, up * 0.07 * _ts + _jitter(0.005 * _ts), c, 1.6, 0.012 * _ts, 0.02 * _ts, 0.06 * _ts)


## The people around `cell` (or anyone on screen, with -1) feel `kind`:
## up to `count` of them, one after the other.
func feel(kind: String, cell := -1, count := 4) -> void:
	var near := Vector3.ZERO
	if cell >= 0 and cell < builder.centers.size():
		near = builder.ground(builder.centers[cell], cell)
	var heads := _heads(near)
	if near != Vector3.ZERO:
		# Only people close by take part.
		heads = heads.filter(func(h: Vector3): return h.distance_to(near) < 0.35)
	for k in mini(count, heads.size()):
		_feel_queue.append([k * randf_range(0.12, 0.3), heads[k], kind])


func _feel_now_and_then(delta: float) -> void:
	var k := 0
	while k < _feel_queue.size():
		_feel_queue[k][0] -= delta
		if _feel_queue[k][0] <= 0.0:
			_pop(_feel_queue[k][1], _feel_queue[k][2])
			_feel_queue.remove_at(k)
		else:
			k += 1
	_feel_left -= delta
	if _feel_left > 0.0:
		return
	_feel_left = randf_range(0.7, 1.6)
	# From far away nobody could see a face.
	if _closeness < 0.15:
		return
	var heads := _heads()
	if heads.is_empty():
		return
	var head: Vector3 = heads[0]
	var storm: Array = weather.storm() if weather != null else []
	var kind := ""
	if not storm.is_empty() and head.normalized().angle_to(storm[0].normalized()) < 0.35 and randf() < 0.8 * float(storm[1]):
		kind = "fear"
	elif randf() < mood:
		kind = ["joy", "joy", "joy", "love", "pride", "wonder"][randi() % 6]
	elif randf() < 0.8:
		kind = "sad" if randf() < 0.6 else "anger"
	if kind != "":
		_pop(head, kind)
