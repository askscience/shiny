## M0 prototype scene: a slowly turning low-poly planet and a status line that
## proves the framed export can reach the plugin's routes and read the host
## context (language and light/dark mode).
extends Node3D

const SPIN_SPEED := 0.12  # radians per second

var host := Host.new()
var planet: MeshInstance3D
var status: Label
var environment: Environment
var _status_parts := {}


func _ready() -> void:
	_build_world()
	_build_ui()
	host.context_changed.connect(_apply_context)
	_apply_context(host.context())
	if host.available:
		host.get_json("ping", _on_ping)
	else:
		_set_status("server", "offline (no LiteAura host)")


func _process(delta: float) -> void:
	planet.rotate_y(SPIN_SPEED * delta)


func _build_world() -> void:
	var mesh := SphereMesh.new()
	mesh.radius = 1.0
	mesh.height = 2.0
	mesh.radial_segments = 16
	mesh.rings = 8
	var material := StandardMaterial3D.new()
	material.albedo_color = Color("4f9d69")
	material.roughness = 0.9
	mesh.material = material

	planet = MeshInstance3D.new()
	planet.mesh = mesh
	planet.rotation.z = deg_to_rad(18.0)
	add_child(planet)

	var sun := DirectionalLight3D.new()
	sun.rotation = Vector3(deg_to_rad(-35.0), deg_to_rad(40.0), 0.0)
	add_child(sun)

	var camera := Camera3D.new()
	camera.position = Vector3(0.0, 0.0, 3.6)
	add_child(camera)

	environment = Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.35, 0.35, 0.4)
	var world_env := WorldEnvironment.new()
	world_env.environment = environment
	add_child(world_env)


func _build_ui() -> void:
	var layer := CanvasLayer.new()
	add_child(layer)
	status = Label.new()
	status.position = Vector2(16, 12)
	layer.add_child(status)


func _apply_context(context: Dictionary) -> void:
	var dark: bool = context.get("mode", "dark") != "light"
	environment.background_color = Color("10131c") if dark else Color("e9eef5")
	status.add_theme_color_override("font_color", Color("e6e9f0") if dark else Color("1d2330"))
	_set_status("locale", str(context.get("locale", "?")))


func _on_ping(reply: Dictionary) -> void:
	if reply.get("success", false):
		var data: Dictionary = reply.get("data", {})
		_set_status("server", "ok, v%s" % data.get("version", "?"))
	else:
		_set_status("server", "error: %s" % reply.get("error", "unknown"))


func _set_status(key: String, value: String) -> void:
	_status_parts[key] = value
	var lines := PackedStringArray()
	for k in _status_parts:
		lines.append("%s: %s" % [k, _status_parts[k]])
	status.text = "\n".join(lines)
