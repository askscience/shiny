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
## Zoom: 1 shows the whole planet; closer, the camera moves in and the
## low-resolution view grows with it (`PIXEL_HEIGHT * zoom` pixels tall), so
## a close look is sharper, down to single screen pixels.
const ZOOM_MIN := 1.0
const ZOOM_MAX := 1.7
const ZOOM_STEP := 1.1
## Frames per second while the window is on screen: the planet turns slowly
## and the art is low-resolution, so more would only spend battery.
const FPS_VISIBLE := 30
## While the window is hidden (another workspace, minimised, tab hidden).
const FPS_HIDDEN := 1

var host := Host.new()
var container: SubViewportContainer
var viewport: SubViewport
var camera: Camera3D
var sky_material: ShaderMaterial
var pixel_material: ShaderMaterial
var planet_view: PlanetView
var traffic: SkyTraffic
var ui: GameUI
var _refresh_left := REFRESH_SECONDS
var _press := Vector2.ZERO
var _pressing := false
var _dragged := false
var _busy := false
var _visible := true
var _zoom := 1.0
var _zoom_target := 1.0
var _touches := {}
var _pinch_distance := 0.0


func _ready() -> void:
	Engine.max_fps = FPS_VISIBLE
	mouse_filter = Control.MOUSE_FILTER_STOP
	_build_view()
	_build_world()
	ui = GameUI.new()
	add_child(ui)
	ui.action_requested.connect(_send_action)
	ui.found_requested.connect(_found)
	ui.reset_requested.connect(_reset)
	ui.skip_requested.connect(func(hours: float):
		_busy = true
		host.post_json("dev/skip", {"hours": hours}, _on_view))
	ui.tile_closed.connect(func(): _select(-1))
	resized.connect(_fit)
	_fit()
	host.context_changed.connect(_apply_context)
	host.visibility_changed.connect(_on_visibility)
	_apply_context(host.context())
	if not host.available:
		ui.show_offline()
		return
	ui.show_loading()
	host.get_json("catalog", _on_catalog)


func _process(delta: float) -> void:
	if absf(_zoom - _zoom_target) > 0.001:
		_zoom = lerpf(_zoom, _zoom_target, minf(1.0, delta * 10.0))
		if absf(_zoom - _zoom_target) < 0.002:
			_zoom = _zoom_target
		_fit()
	if traffic.visible:
		traffic.tick(ui.live_now_ms())
	if not host.available or ui.catalog.is_empty():
		return
	if not _visible:
		return
	_refresh_left -= delta
	if _refresh_left <= 0.0 and not _busy:
		_refresh()


## The window went on or off screen: almost stop drawing while nobody can
## see it, and catch up on the planet as soon as it is back.
func _on_visibility(on_screen: bool) -> void:
	_visible = on_screen
	Engine.max_fps = FPS_VISIBLE if on_screen else FPS_HIDDEN
	if on_screen and not ui.catalog.is_empty() and not _busy:
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
		var era = data.get("era")
		_set_palette(int(era["number"]) if era != null else 1)
		traffic.visible = true
		traffic.show_view(data)
	else:
		planet_view.visible = false
		traffic.visible = false
	ui.show_view(data)
	# Look again right after the next thing finishes, or in a minute.
	var next := ui.next_change_ms()
	_refresh_left = REFRESH_SECONDS if next < 0.0 else clampf(next / 1000.0 + 2.0, 5.0, REFRESH_SECONDS)


var _palette_era := 0


## Reduce the frame to this era's palette (each era adds its colours).
func _set_palette(era: int) -> void:
	if era == _palette_era:
		return
	_palette_era = era
	var colors := PackedVector3Array()
	for c in Palette.for_era(era):
		colors.append(Vector3(c.r, c.g, c.b))
	pixel_material.set_shader_parameter("palette", colors)
	pixel_material.set_shader_parameter("palette_size", colors.size())


func _send_action(action: Dictionary) -> void:
	_busy = true
	host.post_json("action", action, _on_view)


func _found(climate: String) -> void:
	_busy = true
	host.post_json("found", {"climate": climate}, _on_view)


func _reset() -> void:
	_busy = true
	_select(-1)
	ui.open_panel = ""
	host.post_json("reset", {}, _on_view)


func _select(index: int) -> void:
	planet_view.select(index)
	ui.select_tile(index)


# ---------------------------------------------------------------- input

func _zoom_by(factor: float) -> void:
	_zoom_target = clampf(_zoom_target * factor, ZOOM_MIN, ZOOM_MAX)


func _gui_input(event: InputEvent) -> void:
	if not planet_view.visible:
		return
	# Zoom: the wheel, a trackpad pinch, or two fingers on a touch screen.
	if event is InputEventMouseButton and event.pressed and event.button_index in [MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN]:
		_zoom_by(ZOOM_STEP if event.button_index == MOUSE_BUTTON_WHEEL_UP else 1.0 / ZOOM_STEP)
		return
	if event is InputEventMagnifyGesture:
		_zoom_by(event.factor)
		return
	if event is InputEventScreenTouch:
		if event.pressed:
			_touches[event.index] = event.position
		else:
			_touches.erase(event.index)
		_pinch_distance = _touch_spread()
		if _touches.size() >= 2:
			# A pinch is not a click or a drag of the first finger.
			_pressing = false
			_dragged = true
		return
	if event is InputEventScreenDrag:
		_touches[event.index] = event.position
		if _touches.size() >= 2:
			var spread := _touch_spread()
			if _pinch_distance > 0.0 and spread > 0.0:
				_zoom_by(spread / _pinch_distance)
			_pinch_distance = spread
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
			if _touches.size() < 2:
				# The same turn per screen pixel at any zoom: slower up close.
				planet_view.drag(event.relative * (PIXEL_HEIGHT / maxf(size.y, 1.0)) / _zoom)


func _touch_spread() -> float:
	if _touches.size() < 2:
		return 0.0
	var points: Array = _touches.values()
	return (points[0] as Vector2).distance_to(points[1])


# ---------------------------------------------------------------- scene

func _build_view() -> void:
	container = SubViewportContainer.new()
	container.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	container.stretch = true
	container.mouse_filter = Control.MOUSE_FILTER_IGNORE
	container.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	pixel_material = ShaderMaterial.new()
	pixel_material.shader = load("res://pixel.gdshader")
	_set_palette(1)
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

	# The palette pass, over everything drawn in the low-resolution view.
	var front := CanvasLayer.new()
	front.layer = 1
	viewport.add_child(front)
	var quantize := ColorRect.new()
	quantize.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	quantize.mouse_filter = Control.MOUSE_FILTER_IGNORE
	quantize.material = pixel_material
	front.add_child(quantize)


func _build_world() -> void:
	var world := Node3D.new()
	viewport.add_child(world)
	planet_view = PlanetView.new()
	planet_view.visible = false
	world.add_child(planet_view)
	traffic = SkyTraffic.new()
	traffic.visible = false
	world.add_child(traffic)

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
	var shrink := maxi(1, roundi(size.y / (PIXEL_HEIGHT * _zoom)))
	container.stretch_shrink = shrink
	var source := (size / shrink).floor()
	sky_material.set_shader_parameter("size", source)
	# The camera's field of view is vertical: on a tall window (a phone) move
	# back so the planet still fits the width.
	var aspect := size.x / maxf(size.y, 1.0)
	camera.position.z = 4.4 / minf(1.0, aspect / 1.1) / _zoom


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
