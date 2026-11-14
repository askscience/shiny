## Oorbit's window. The 3D planet renders at a low resolution (about 300
## pixels tall), upscaled with sharp pixels and reduced to a soft palette;
## the interface draws at full resolution above it. The game state lives in
## the plugin: this scene reads it (`/api/oorbit/state`), shows it, and sends
## the player's actions back.
extends Control

## Target height of the low-resolution view, in pixels.
const PIXEL_HEIGHT := 300.0
## How often the window asks for the planet again, in seconds: while you
## play (and tells the plugin you are playing, so the planet runs fast), and
## while the window only sits there.
const PLAY_REFRESH_SECONDS := 5.0
const REFRESH_SECONDS := 60.0
## You count as playing until this long after your last touch.
const PLAYING_AFTER_INPUT_MS := 5 * 60 * 1000
## A press that moves less than this (screen pixels) is a click, not a drag.
const CLICK_SLOP := 6.0
## Zoom: 1 shows the whole planet; closer, the camera comes down towards the
## ground and tilts to the horizon, and the low-resolution view grows with
## it (up to `PIXEL_HEIGHT * PIXEL_ZOOM_MAX` pixels tall), so a close look
## is sharper.
const ZOOM_MIN := 1.0
const ZOOM_MAX := 5.0
const ZOOM_STEP := 1.15
const PIXEL_ZOOM_MAX := 2.4
## Distance of the camera from the planet's centre at zoom 1.
const FAR := 4.4
## A double click flies to the cell and comes at least this close.
const FOCUS_ZOOM := 3.5
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
var _double_click := false
var _last_input_ms := 0
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
	ui.tile_focus_requested.connect(func(cell: int):
		planet_view.focus(cell)
		_zoom_target = maxf(_zoom_target, FOCUS_ZOOM)
		_select(cell))
	planet_view.clock = ui.live_now_ms
	planet_view.delivered.connect(_on_delivered)
	_last_input_ms = Time.get_ticks_msec()
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
	_update_marks()
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


func _playing() -> bool:
	return Time.get_ticks_msec() - _last_input_ms < PLAYING_AFTER_INPUT_MS


func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton or event is InputEventScreenTouch or event is InputEventKey or event is InputEventMagnifyGesture:
		var was_playing := _playing()
		_last_input_ms = Time.get_ticks_msec()
		# Coming back after a pause: speed the planet up at once.
		if not was_playing and not _busy and not ui.catalog.is_empty() and ui.view.get("planet") != null:
			_refresh()


func _refresh() -> void:
	_refresh_left = REFRESH_SECONDS
	_busy = true
	if _playing() and ui.view.get("planet") != null:
		host.post_json("presence", {}, _on_view)
	else:
		host.get_json("state", _on_view)


func _on_catalog(reply: Dictionary) -> void:
	if not reply.get("success", false):
		ui.toast(I18n.t("ui.error", {"error": reply.get("error", "?")}))
		return
	ui.catalog = reply.get("data", {})
	host.get_json("layout", _on_layout)


func _on_layout(reply: Dictionary) -> void:
	if not reply.get("success", false):
		ui.toast(I18n.t("ui.error", {"error": reply.get("error", "?")}))
		return
	planet_view.set_layout(reply.get("data", {}))
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
		planet_view.time_rate = float(data.get("clock", {}).get("rate", 1.0))
		planet_view.show_planet(planet, data.get("rates", {}))
		var era = data.get("era")
		_set_palette(int(era["number"]) if era != null else 1)
		traffic.visible = true
		traffic.show_view(data)
	else:
		planet_view.visible = false
		traffic.visible = false
	ui.show_view(data)
	# An event that happens somewhere: fly there once.
	if planet != null and planet.get("pending_event") != null:
		var e: Dictionary = planet["pending_event"]
		var key := "%s@%s" % [e.get("id"), e.get("step")]
		if e.get("tile") != null and key != _event_focused:
			_event_focused = key
			planet_view.focus(int(e["tile"]))
	# Look again right after the next thing finishes, or in a minute.
	if _playing() and planet != null:
		_refresh_left = PLAY_REFRESH_SECONDS
	else:
		var next := ui.next_change_ms()
		_refresh_left = REFRESH_SECONDS if next < 0.0 else clampf(ui.wall_ms(next) / 1000.0 + 2.0, 5.0, REFRESH_SECONDS)


var _event_focused := ""


## Finds and events on the planet: where they are on screen, and whether
## they face the camera (the window puts a marker on the edge for the
## hidden ones).
func _update_marks() -> void:
	if not planet_view.visible:
		ui.set_marks([])
		return
	var list := []
	var shrink := float(container.stretch_shrink)
	var size := get_viewport_rect().size
	for m in planet_view.marks():
		var at: Vector3 = m["at"]
		var facing := (m["normal"] as Vector3).dot((camera.global_position - at).normalized()) > 0.05
		var screen := camera.unproject_position(at) * shrink
		var inside := Rect2(Vector2.ZERO, size).grow(-24.0).has_point(screen)
		list.append({"tile": m["tile"], "kind": m["kind"], "screen": screen, "hidden": not facing or not inside or camera.is_position_behind(at)})
	ui.set_marks(list)


## A crew brought something home: a little "+3 wood" over the spot.
func _on_delivered(where: Vector3, text: String, color: Color) -> void:
	# Only on the side of the planet facing the camera.
	if camera.is_position_behind(where) or where.dot(camera.global_position - where) <= 0.0:
		return
	var screen := camera.unproject_position(where) * float(container.stretch_shrink)
	ui.float_text(screen, text, color)


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
			_double_click = event.double_click
		else:
			_pressing = false
			if not _dragged:
				var tile := planet_view.pick(camera, event.position / float(container.stretch_shrink))
				if _double_click and tile >= 0:
					# A double click (or tap) flies over the cell.
					planet_view.focus(tile)
					_zoom_target = maxf(_zoom_target, FOCUS_ZOOM)
					_select(tile)
				else:
					_select(tile if tile != planet_view.selected else -1)
			_double_click = false
	elif event is InputEventMouseMotion and _pressing:
		if _dragged or event.position.distance_to(_press) > CLICK_SLOP:
			_dragged = true
			if _touches.size() < 2:
				# The ground follows the finger: slower the closer the camera.
				var closeness := (_distance() - 0.98) / (FAR - 1.0)
				planet_view.drag(event.relative * (PIXEL_HEIGHT / maxf(size.y, 1.0)) * closeness)


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
	var shrink := maxi(1, roundi(size.y / (PIXEL_HEIGHT * minf(_zoom, PIXEL_ZOOM_MAX))))
	container.stretch_shrink = shrink
	var source := (size / shrink).floor()
	sky_material.set_shader_parameter("size", source)
	# Close up, the camera sinks below the planet's equator and looks along
	# the ground towards the horizon.
	var d := _distance()
	var k := clampf((_zoom - 1.8) / (ZOOM_MAX - 1.8), 0.0, 1.0)
	var tilt := deg_to_rad(24.0) * k
	camera.position = Vector3(0.0, -d * sin(tilt), d * cos(tilt))
	camera.look_at(Vector3(0.0, 0.0, 1.0 * k), Vector3.UP)
	if planet_view != null:
		planet_view.spin_allowed = _zoom < 1.3
		# The dark rim reads well around the whole planet, but up close it
		# would stand above the horizon.
		planet_view.set_rim(_zoom < 1.4)


## The camera's distance from the planet's centre. The field of view is
## vertical: on a tall window (a phone) the far view moves back so the
## planet still fits the width.
func _distance() -> float:
	var aspect := size.x / maxf(size.y, 1.0)
	var far := (FAR - 1.0) / minf(1.0, aspect / 1.1)
	return 1.0 + far / pow(_zoom, 1.3)


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
