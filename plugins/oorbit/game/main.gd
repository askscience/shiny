## Oorbit's window. The 3D planet renders at a low resolution (about 300
## pixels tall), upscaled with sharp pixels and reduced to a soft palette;
## the interface draws at full resolution above it. The game state lives in
## the plugin: this scene reads it (`/api/oorbit/state`), shows it, and sends
## the player's actions back.
extends Control

## Target height of the low-resolution view, in pixels.
const PIXEL_HEIGHT := 300.0
## How often the window asks for the planet again, in seconds.
const REFRESH_SECONDS := 60.0
## A press that moves less than this (screen pixels) is a click, not a drag.
const CLICK_SLOP := 6.0

var host := Host.new()
var container: SubViewportContainer
var viewport: SubViewport
var camera: Camera3D
var sky_material: ShaderMaterial
var pixel_material: ShaderMaterial
var planet_view: PlanetView
var ui: GameUI
var _refresh_left := REFRESH_SECONDS
var _press := Vector2.ZERO
var _pressing := false
var _dragged := false
var _busy := false


func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_STOP
	_build_view()
	_build_world()
	ui = GameUI.new()
	add_child(ui)
	ui.action_requested.connect(_send_action)
	ui.found_requested.connect(_found)
	ui.tile_closed.connect(func(): _select(-1))
	resized.connect(_fit)
	_fit()
	host.context_changed.connect(_apply_context)
	_apply_context(host.context())
	if not host.available:
		ui.show_offline()
		return
	ui.show_loading()
	host.get_json("catalog", _on_catalog)


func _process(delta: float) -> void:
	if not host.available or ui.catalog.is_empty():
		return
	_refresh_left -= delta
	if _refresh_left <= 0.0 and not _busy:
		_refresh()


func _refresh() -> void:
	_refresh_left = REFRESH_SECONDS
	_busy = true
	host.get_json("state", _on_view)


func _on_catalog(reply: Dictionary) -> void:
	if not reply.get("success", false):
		ui.toast(I18n.t("ui.error", {"error": reply.get("error", "?")}))
		return
	ui.catalog = reply.get("data", {})
	_refresh()


func _on_view(reply: Dictionary) -> void:
	_busy = false
	if not reply.get("success", false):
		ui.toast(I18n.t("ui.error", {"error": reply.get("error", "?")}))
		return
	var data: Dictionary = reply.get("data", {})
	var planet = data.get("planet")
	if planet != null:
		planet_view.visible = true
		planet_view.show_planet(planet)
	else:
		planet_view.visible = false
	ui.show_view(data)


func _send_action(action: Dictionary) -> void:
	_busy = true
	host.post_json("action", action, _on_view)


func _found(climate: String) -> void:
	_busy = true
	host.post_json("found", {"climate": climate}, _on_view)


func _select(index: int) -> void:
	planet_view.select(index)
	ui.select_tile(index)


# ---------------------------------------------------------------- input

func _gui_input(event: InputEvent) -> void:
	if not planet_view.visible:
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_pressing = true
			_dragged = false
			_press = event.position
		else:
			_pressing = false
			if not _dragged:
				var tile := planet_view.pick(camera, event.position / float(container.stretch_shrink))
				_select(tile if tile != planet_view.selected else -1)
	elif event is InputEventMouseMotion and _pressing:
		if _dragged or event.position.distance_to(_press) > CLICK_SLOP:
			_dragged = true
			planet_view.drag(event.relative / float(container.stretch_shrink))


# ---------------------------------------------------------------- scene

func _build_view() -> void:
	container = SubViewportContainer.new()
	container.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	container.stretch = true
	container.mouse_filter = Control.MOUSE_FILTER_IGNORE
	container.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	pixel_material = ShaderMaterial.new()
	pixel_material.shader = load("res://pixel.gdshader")
	var colors := PackedVector3Array()
	for c in Palette.STONE_AGE:
		colors.append(Vector3(c.r, c.g, c.b))
	pixel_material.set_shader_parameter("palette", colors)
	pixel_material.set_shader_parameter("palette_size", colors.size())
	container.material = pixel_material
	add_child(container)

	viewport = SubViewport.new()
	viewport.msaa_3d = Viewport.MSAA_DISABLED
	viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
	viewport.canvas_item_default_texture_filter = Viewport.DEFAULT_CANVAS_ITEM_TEXTURE_FILTER_NEAREST
	container.add_child(viewport)

	# The sky is a canvas layer drawn behind the 3D scene (BG_CANVAS).
	var back := CanvasLayer.new()
	back.layer = -1
	viewport.add_child(back)
	var sky := ColorRect.new()
	sky.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	sky_material = ShaderMaterial.new()
	sky_material.shader = load("res://sky.gdshader")
	sky.material = sky_material
	back.add_child(sky)


func _build_world() -> void:
	var world := Node3D.new()
	viewport.add_child(world)
	planet_view = PlanetView.new()
	planet_view.visible = false
	world.add_child(planet_view)

	var sun := DirectionalLight3D.new()
	sun.light_color = Palette.SUN
	sun.light_energy = 1.15
	sun.rotation = Vector3(deg_to_rad(-30.0), deg_to_rad(-50.0), 0.0)
	world.add_child(sun)

	camera = Camera3D.new()
	camera.fov = 32.0
	camera.position = Vector3(0.0, 0.0, 4.4)
	world.add_child(camera)

	var environment := Environment.new()
	environment.background_mode = Environment.BG_CANVAS
	environment.background_canvas_max_layer = -1
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Palette.AMBIENT
	environment.ambient_light_energy = 0.7
	var world_env := WorldEnvironment.new()
	world_env.environment = environment
	world.add_child(world_env)


## Keep the pixels square and the view near PIXEL_HEIGHT pixels tall.
func _fit() -> void:
	var shrink := maxi(1, roundi(size.y / PIXEL_HEIGHT))
	container.stretch_shrink = shrink
	var source := (size / shrink).floor()
	pixel_material.set_shader_parameter("source_size", source)
	sky_material.set_shader_parameter("size", source)
	# The camera's field of view is vertical: on a tall window (a phone) move
	# back so the planet still fits the width.
	var aspect := size.x / maxf(size.y, 1.0)
	camera.position.z = 4.4 / minf(1.0, aspect / 1.1)


func _apply_context(context: Dictionary) -> void:
	I18n.set_language(str(context.get("locale", "en")))
	var dark: bool = context.get("mode", "dark") != "light"
	# Night: deep indigo. Day (light mode): a soft dusk of lavender and peach.
	var top := Color("1e1b33") if dark else Color("8a7bb0")
	var bottom := Color("3d3361") if dark else Color("f2d6c2")
	sky_material.set_shader_parameter("top_color", Vector3(top.r, top.g, top.b))
	sky_material.set_shader_parameter("bottom_color", Vector3(bottom.r, bottom.g, bottom.b))
	if ui != null and not ui.view.is_empty():
		ui.show_view(ui.view)
