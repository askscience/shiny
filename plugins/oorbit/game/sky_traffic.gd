## The sky around the planet: the three neighbours as small planets in the
## distance, and every ship under way between them and home, drawn as what
## its era sails with (seeds on the wind, birds, rafts, sailing ships,
## balloons, airships, planes, drones, rockets).
class_name SkyTraffic
extends Node3D

## Where the neighbours hang in the sky (world space, behind the planet).
const SPOTS := [Vector3(-3.1, 1.5, -3.5), Vector3(3.3, 1.1, -4.2), Vector3(2.6, -1.7, -3.8)]
const CLIMATE_COLORS := {
	"temperate": Color("6f9163"), "arid": Color("d9b48f"), "frozen": Color("c9dde8"), "oceanic": Color("4a7a8c"),
}
## Where ships touch home: the side of the planet facing the camera.
const HOME := Vector3(0.25, 0.35, 1.0)

var _neighbors := {}  # id -> Node3D
var _ships := {}      # ship id -> [Node3D, from, to, depart_ms, arrive_ms]
var _step_ms := 600000.0


func show_view(view: Dictionary) -> void:
	_step_ms = float(view.get("step_ms", 600000))
	var dock: Dictionary = view.get("dock", {})
	var list: Array = dock.get("neighbors", [])
	for i in list.size():
		var n: Dictionary = list[i]
		if not _neighbors.has(n["id"]):
			var body := _small_planet(CLIMATE_COLORS.get(n["climate"], Color("94b37e")))
			body.position = SPOTS[i % SPOTS.size()]
			add_child(body)
			_neighbors[n["id"]] = body
	var seen := {}
	for s in view.get("planet", {}).get("ships", []):
		var id := int(s["id"])
		seen[id] = true
		if _ships.has(id):
			continue
		var model := _ship_model(int(s["era"]), str(s["kind"]))
		add_child(model)
		_ships[id] = [model, str(s["from"]), str(s["to"]), float(s["depart_step"]) * _step_ms, float(s["arrive_step"]) * _step_ms]
	for id in _ships.keys():
		if not seen.has(id):
			_ships[id][0].queue_free()
			_ships.erase(id)


func _where(place: String) -> Vector3:
	if place == "home":
		return HOME.normalized() * 1.25
	var body: Node3D = _neighbors.get(place)
	return body.position if body != null else Vector3(0, 3, -4)


## Move every ship along its route at `now_ms` on the planet's clock.
func tick(now_ms: float) -> void:
	for id in _ships:
		var entry: Array = _ships[id]
		var model: Node3D = entry[0]
		var from := _where(entry[1])
		var to := _where(entry[2])
		var t := clampf((now_ms - float(entry[3])) / maxf(1.0, float(entry[4]) - float(entry[3])), 0.0, 1.0)
		# An arc rather than a straight line, so ships do not cross the planet.
		var mid := (from + to) * 0.5 + Vector3(0, 0.9, 0.6)
		var a := from.lerp(mid, t)
		var b := mid.lerp(to, t)
		var at := a.lerp(b, t)
		var ahead := a.lerp(b, minf(1.0, t + 0.02)).lerp(mid.lerp(to, minf(1.0, t + 0.02)), minf(1.0, t + 0.02))
		model.position = at
		if ahead.distance_to(at) > 0.0001:
			model.look_at(ahead, Vector3.UP)


func _toon(color: Color) -> StandardMaterial3D:
	var m := StandardMaterial3D.new()
	m.albedo_color = color
	m.diffuse_mode = BaseMaterial3D.DIFFUSE_TOON
	m.specular_mode = BaseMaterial3D.SPECULAR_DISABLED
	return m


func _part(parent: Node3D, mesh: Mesh, color: Color, offset := Vector3.ZERO, rotation_deg := Vector3.ZERO) -> void:
	var m := MeshInstance3D.new()
	m.mesh = mesh
	m.material_override = _toon(color)
	m.position = offset
	m.rotation_degrees = rotation_deg
	parent.add_child(m)


func _small_planet(color: Color) -> Node3D:
	var node := Node3D.new()
	var sphere := SphereMesh.new()
	sphere.radius = 0.32
	sphere.height = 0.64
	sphere.radial_segments = 10
	sphere.rings = 6
	_part(node, sphere, color)
	var outline := MeshInstance3D.new()
	outline.mesh = sphere
	outline.scale = Vector3.ONE * 1.06
	var m := StandardMaterial3D.new()
	m.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	m.cull_mode = BaseMaterial3D.CULL_FRONT
	m.albedo_color = Palette.OUTLINE
	outline.material_override = m
	node.add_child(outline)
	return node


static func _box(x: float, y: float, z: float) -> BoxMesh:
	var m := BoxMesh.new()
	m.size = Vector3(x, y, z)
	return m


static func _cone(r: float, h: float, sides := 6) -> CylinderMesh:
	var m := CylinderMesh.new()
	m.top_radius = 0.0
	m.bottom_radius = r
	m.height = h
	m.radial_segments = sides
	m.rings = 1
	return m


static func _ball(r: float) -> SphereMesh:
	var m := SphereMesh.new()
	m.radius = r
	m.height = r * 2.0
	m.radial_segments = 8
	m.rings = 4
	return m


## A ship as its era builds them; pranks fly a little darker.
func _ship_model(era: int, kind: String) -> Node3D:
	var node := Node3D.new()
	var hull := Color("8f6f5a") if kind != "prank" else Color("5b5466")
	match era:
		1:
			for k in 3:
				_part(node, _ball(0.025), Color("eef3f2"), Vector3((k - 1) * 0.05, sin(k) * 0.03, 0))
		2:
			_part(node, _box(0.12, 0.015, 0.03), Color("eef3f2"), Vector3(-0.05, 0, 0), Vector3(0, 0, 20))
			_part(node, _box(0.12, 0.015, 0.03), Color("eef3f2"), Vector3(0.05, 0, 0), Vector3(0, 0, -20))
		3:
			_part(node, _box(0.12, 0.025, 0.16), hull)
			_part(node, _box(0.01, 0.1, 0.01), Color("d9b48f"), Vector3(0, 0.05, 0))
		4, 5:
			_part(node, _box(0.1, 0.05, 0.22), hull)
			_part(node, _cone(0.07, 0.16, 3), Color("eef3f2"), Vector3(0, 0.11, 0))
		6:
			_part(node, _ball(0.09), Color("d4a24c"), Vector3(0, 0.12, 0))
			_part(node, _box(0.05, 0.04, 0.05), hull)
		7:
			var capsule := CapsuleMesh.new()
			capsule.radius = 0.06
			capsule.height = 0.3
			capsule.radial_segments = 8
			capsule.rings = 2
			_part(node, capsule, Color("707780"), Vector3.ZERO, Vector3(90, 0, 0))
			_part(node, _box(0.04, 0.03, 0.06), Color("4a4a4f"), Vector3(0, -0.07, 0))
		8:
			_part(node, _box(0.05, 0.05, 0.24), Color("9fb3c8"))
			_part(node, _box(0.3, 0.012, 0.06), Color("9fb3c8"))
		9:
			_part(node, _box(0.14, 0.03, 0.14), Color("1b1530"))
			_part(node, _box(0.16, 0.012, 0.02), Color("ff4fd8"))
			_part(node, _box(0.02, 0.012, 0.16), Color("4fe3ff"))
		_:
			var body := CylinderMesh.new()
			body.top_radius = 0.035
			body.bottom_radius = 0.035
			body.height = 0.2
			body.radial_segments = 8
			body.rings = 1
			_part(node, body, Color("e8edf2"), Vector3.ZERO, Vector3(90, 0, 0))
			_part(node, _cone(0.035, 0.08, 8), Color("e8edf2"), Vector3(0, 0, -0.14), Vector3(-90, 0, 0))
			_part(node, _ball(0.03), Color("ff9d3b"), Vector3(0, 0, 0.12))
	return node
