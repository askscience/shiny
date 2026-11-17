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
const PIXEL_ZOOM_MAX := 1.7
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
var audio: PlanetAudio
var _listen_left := 0.0
var _hammer_left := 2.0
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
	audio = PlanetAudio.new()
	add_child(audio)
	audio.muted = _load_muted()
	ui.set_muted(audio.muted)
	ui.sound_requested.connect(func(name: String): audio.play(name))
	ui.laura_persona = _load_pref("oorbit.laura", "historian")
	ui.laura_requested.connect(func(persona: String):
		_save_pref("oorbit.laura", persona)
		_laura_tries = 0
		_ask_laura())
	ui.mute_toggled.connect(func(on: bool):
		audio.muted = on
		_save_muted(on))
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
	planet_view.sfx.connect(func(name: String, where: Vector3): audio.play(name, _weight_at(where)))
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


## `?stats` in the window's address (the preview server's `/?stats`): print
## what the low-resolution view draws every few seconds, to measure costs.
var _stats := OS.has_feature("web") and str(JavaScriptBridge.eval("location.search")).contains("stats")
var _stats_left := 3.0
var _stats_frames := 0
var _stats_process := 0.0


func _print_stats(delta: float, spent_usec: int) -> void:
	_stats_left -= delta
	_stats_frames += 1
	_stats_process += spent_usec / 1000.0
	if _stats_left > 0.0:
		return
	var vp := viewport.get_viewport_rid()
	print("[stats] zoom %.2f  primitives %d  draw calls %d  objects %d  scripts %.2f ms/frame  fps %d" % [
		_zoom,
		RenderingServer.viewport_get_render_info(vp, RenderingServer.VIEWPORT_RENDER_INFO_TYPE_VISIBLE, RenderingServer.VIEWPORT_RENDER_INFO_PRIMITIVES_IN_FRAME),
		RenderingServer.viewport_get_render_info(vp, RenderingServer.VIEWPORT_RENDER_INFO_TYPE_VISIBLE, RenderingServer.VIEWPORT_RENDER_INFO_DRAW_CALLS_IN_FRAME),
		RenderingServer.viewport_get_render_info(vp, RenderingServer.VIEWPORT_RENDER_INFO_TYPE_VISIBLE, RenderingServer.VIEWPORT_RENDER_INFO_OBJECTS_IN_FRAME),
		_stats_process / maxf(1.0, _stats_frames),
		Engine.get_frames_per_second()])
	_stats_left = 3.0
	_stats_frames = 0
	_stats_process = 0.0


func _process(delta: float) -> void:
	_step(delta)
	if _stats:
		_print_stats(delta, roundi(Performance.get_monitor(Performance.TIME_PROCESS) * 1e6))


func _step(delta: float) -> void:
	if absf(_zoom - _zoom_target) > 0.001:
		_zoom = lerpf(_zoom, _zoom_target, minf(1.0, delta * 10.0))
		if absf(_zoom - _zoom_target) < 0.002:
			_zoom = _zoom_target
		_fit()
	# Ships between the planets sail past the camera: only in the far view.
	traffic.visible = planet_view.visible and _zoom < 1.35 and not ui.catalog.is_empty()
	if traffic.visible:
		traffic.tick(ui.live_now_ms())
	_update_marks()
	planet_view.set_camera(camera, clampf((_zoom - 1.0) / (ZOOM_MAX - 1.0), 0.0, 1.0), float(viewport.size.y))
	_update_sky()
	_update_sound(delta)
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
		audio.play("error")
		return
	var data: Dictionary = reply.get("data", {})
	var planet = data.get("planet")
	if planet != null:
		planet_view.visible = true
		planet_view.time_rate = float(data.get("clock", {}).get("rate", 1.0))
		_load_wonders(str(planet.get("seed", "")))
		planet_view.show_planet(planet, data.get("rates", {}))
		ui.wonders = planet_view.wonders
		_look_once()
		var era = data.get("era")
		_set_palette(int(era["number"]) if era != null else 1)
		traffic.show_view(data)
	else:
		planet_view.visible = false
	ui.show_view(data)
	# At the opening, Laura gets to work on her account of what happened.
	if planet != null and not _laura_asked:
		_laura_asked = true
		_ask_laura()
	# An event that happens somewhere: fly there once.
	if planet != null and planet.get("pending_event") != null:
		var e: Dictionary = planet["pending_event"]
		var key := "%s@%s" % [e.get("id"), e.get("step")]
		if e.get("tile") != null and key != _event_focused and planet_view.builder.ready():
			_event_focused = key
			planet_view.focus(int(e["tile"]))
	# Look again right after the next thing finishes, or in a minute.
	if _playing() and planet != null:
		_refresh_left = PLAY_REFRESH_SECONDS
	else:
		var next := ui.next_change_ms()
		_refresh_left = REFRESH_SECONDS if next < 0.0 else clampf(ui.wall_ms(next) / 1000.0 + 2.0, 5.0, REFRESH_SECONDS)


var _event_focused := ""


## How loud something at `where` (world space) should sound: full in the
## middle of the view and close up, fading towards the edges, silent on the
## far side of the planet.
func _weight_at(where: Vector3) -> float:
	if camera.is_position_behind(where):
		return 0.0
	var normal := (where - planet_view.pivot.global_position).normalized()
	if normal.dot((camera.global_position - where).normalized()) <= 0.0:
		return 0.0
	var screen := camera.unproject_position(where) * float(container.stretch_shrink)
	var size := get_viewport_rect().size
	var d := clampf((screen - size / 2.0).length() / (size.length() / 2.0), 0.0, 1.0)
	var closeness := clampf((_zoom - 1.0) / (ZOOM_MAX - 1.0), 0.0, 1.0)
	return pow(1.0 - d, 2.0) * (0.35 + 0.65 * closeness)


## The ambient mix follows what the camera looks at; now and then a hammer
## rings on a building site in view.
func _update_sound(delta: float) -> void:
	if not planet_view.visible:
		audio.listen(0.0, {})
		return
	_listen_left -= delta
	if _listen_left <= 0.0:
		_listen_left = 0.25
		var closeness := clampf((_zoom - 1.0) / (ZOOM_MAX - 1.0), 0.0, 1.0)
		var mix := planet_view.view_mix(camera.global_position, closeness)
		var storm: Array = planet_view.weather.storm()
		if not storm.is_empty():
			var where := planet_view.pivot.to_global(storm[0])
			mix["storm"] = float(storm[1]) * clampf(0.25 + _weight_at(where) * 1.5, 0.0, 1.0) if _weight_at(where) > 0.0 else 0.0
		audio.listen(closeness, mix)
	_hammer_left -= delta
	if _hammer_left <= 0.0:
		_hammer_left = randf_range(1.6, 3.2)
		var sites := planet_view.site_points()
		if not sites.is_empty():
			audio.play("hammer", _weight_at(sites[randi() % sites.size()]))


var _laura_tries := 0
var _laura_asked := false


## Ask Laura for her account; while she is writing, ask again a little
## later (a few times at most).
func _ask_laura() -> void:
	host.post_json("chronicle", {"lang": I18n.language, "persona": ui.laura_persona}, func(reply: Dictionary):
		if not reply.get("success", false):
			return
		var data: Dictionary = reply.get("data", {})
		ui.set_laura(data)
		if str(data.get("status", "")) == "writing" and _laura_tries < 12:
			_laura_tries += 1
			get_tree().create_timer(6.0).timeout.connect(_ask_laura))


func _load_pref(key: String, fallback: String) -> String:
	if not OS.has_feature("web"):
		return fallback
	var v = JavaScriptBridge.eval("(function(){try{return localStorage.getItem('%s')||''}catch(e){return ''}})()" % key)
	return fallback if str(v) == "" else str(v)


func _save_pref(key: String, value: String) -> void:
	if OS.has_feature("web"):
		JavaScriptBridge.eval("(function(){try{localStorage.setItem('%s','%s')}catch(e){}})()" % [key, value.replace("'", "")])


func _load_muted() -> bool:
	if not OS.has_feature("web"):
		return false
	var v = JavaScriptBridge.eval("(function(){try{return localStorage.getItem('oorbit.muted')||''}catch(e){return ''}})()")
	return str(v) == "1"


func _save_muted(on: bool) -> void:
	if OS.has_feature("web"):
		JavaScriptBridge.eval("(function(){try{localStorage.setItem('oorbit.muted','%s')}catch(e){}})()" % ("1" if on else "0"))


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
	audio.play("deliver", _weight_at(where))


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
	var before: Dictionary = ui.view.get("planet", {}) if ui.view.get("planet") != null else {}
	host.post_json("action", action, func(reply: Dictionary):
		_on_view(reply)
		if reply.get("success", false):
			_react(action, before, reply.get("data", {}).get("planet", {})))


## The people around show what they think of the player's choice: a
## coloured dot from the heads on screen (see `PlanetView.feel`).
func _react(action: Dictionary, before: Dictionary, after) -> void:
	if after == null or before.is_empty():
		return
	var moved := float(after.get("mood", 0.0)) - float(before.get("mood", 0.0))
	var tile := int(action.get("tile", -1))
	match str(action.get("type", "")):
		"resolve_event":
			var e = before.get("pending_event")
			var cell := int(e["tile"]) if e != null and e.get("tile") != null else -1
			if moved > 0.004:
				planet_view.feel("joy", cell, 6)
			elif moved < -0.004:
				planet_view.feel("anger", cell, 3)
				planet_view.feel("sad", cell, 3)
			else:
				planet_view.feel("wonder", cell, 4)
		"build":
			planet_view.feel("joy", tile, 3)
		"demolish":
			planet_view.feel("sad", tile, 4)
		"help":
			planet_view.feel("pride", tile, 4)
		"plant":
			planet_view.feel("love", tile, 3)
		"harvest":
			planet_view.feel("pride", tile, 2)
		"stop_harvest":
			planet_view.feel("wonder", tile, 1)
		"collect":
			for f in before.get("finds", []):
				if int(f["id"]) == int(action.get("id", -1)):
					planet_view.feel("wonder", int(f["tile"]), 2)
					planet_view.feel("joy", int(f["tile"]), 3)
		"research":
			planet_view.feel("wonder", -1, 2)
		"link", "accept_offer":
			planet_view.feel("joy", -1, 4)
		"send":
			planet_view.feel("love", -1, 3)
	if int(after.get("era", 0)) > int(before.get("era", 0)):
		planet_view.feel("joy", -1, 10)


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
	_discover(index)
	ui.select_tile(index)


# ---------------------------------------------------------------- day, seasons, festivals

var _sun: DirectionalLight3D
## Preview overrides from the address (`?hour=21&season=2.5&fest=sun`,
## `&lapse=30` for a whole day in 30 seconds).
var _sky_debug := {}
var _sky_debug_read := false
## How far the snow comes down in deep winter, by climate (1 = to the
## equator), and how bare the trees get.
const SNOW_BY_CLIMATE := {"temperate": 0.38, "frozen": 0.75, "arid": 0.1, "oceanic": 0.25}


## Planet hour (0-24) and season position (0 spring .. 4) right now.
func _calendar_now() -> Array:
	var cal: Dictionary = ui.view.get("calendar", {})
	var now := ui.live_now_ms()
	var day := float(cal.get("day_ms", 86400000.0))
	var hour := fmod(now, day) / day * 24.0
	var season := 0.0
	if cal.has("season_ms"):
		var into := clampf((now - float(cal["season_start_ms"])) / maxf(1.0, float(cal["season_ms"])), 0.0, 0.999)
		season = float(cal.get("season", 0)) + into
	if _sky_debug.has("hour"):
		hour = float(_sky_debug["hour"])
	if _sky_debug.has("lapse"):
		# A whole day in `lapse` seconds, from the hour given (or noon).
		hour = fmod(float(_sky_debug.get("hour", 12.0)) + Time.get_ticks_msec() / 1000.0 * 24.0 / maxf(1.0, float(_sky_debug["lapse"])), 24.0)
	if _sky_debug.has("season"):
		season = float(_sky_debug["season"])
	return [hour, season]


## The sun goes round the planet once a planet day (noon over the home),
## higher in summer; the season shows on the land; a festival under way is
## drawn.
func _update_sky() -> void:
	if planet_view == null or not planet_view.builder.ready() or ui.view.is_empty():
		return
	if not _sky_debug_read:
		_sky_debug_read = true
		if OS.has_feature("web"):
			var query := str(JavaScriptBridge.eval("location.search")).trim_prefix("?")
			for pair in query.split("&", false):
				var kv := pair.split("=")
				if kv.size() == 2 and kv[0] in ["hour", "season", "fest", "lapse"]:
					_sky_debug[kv[0]] = kv[1]
	var when := _calendar_now()
	var hour: float = when[0]
	var season: float = when[1]
	var w := []
	for k in 4:
		# Each season peaks in its middle and fades into the next.
		var d := absf(fposmod(season - (k + 0.5) + 2.0, 4.0) - 2.0)
		w.append(clampf(1.0 - d, 0.0, 1.0))
	var planet: Dictionary = ui.view.get("planet", {})
	var home: Vector3 = planet_view.builder.centers[clampi(int(planet.get("home", 0)), 0, planet_view.builder.centers.size() - 1)]
	var noon := atan2(home.x, home.z)
	var a := noon + (hour - 12.0) / 24.0 * TAU
	var tilt := 0.35 * (float(w[1]) - float(w[3]))
	var sun := Vector3(cos(tilt) * sin(a), sin(tilt), cos(tilt) * cos(a))
	var reach: float = SNOW_BY_CLIMATE.get(str(planet.get("climate", "temperate")), 0.5)
	planet_view.set_sky(sun, {
		"snow": reach * float(w[3]), "autumn": float(w[2]), "bare": float(w[3]) * (0.4 if reach < 0.2 else 0.9),
		"bloom": float(w[0]), "summer": float(w[1]),
	})
	var world_sun := planet_view.pivot.global_basis * sun
	_sun.global_basis = Basis.looking_at(-world_sun, Vector3.UP if absf(world_sun.y) < 0.99 else Vector3.RIGHT)
	# The festival under way, if any.
	var fest := ""
	var c = planet.get("celebration")
	if c is Dictionary:
		var step := ui.live_now_ms() / float(ui.view.get("step_ms", 600000))
		if step >= float(c.get("from_step", 0)) and step < float(c.get("until_step", 0)):
			fest = str(c.get("id", ""))
	if _sky_debug.has("fest"):
		fest = str(_sky_debug["fest"])
	planet_view.set_festival(fest)
	# Preview: `?fest=` also flies over the village green once.
	if _sky_debug.has("fest") and not _sky_debug.has("flown") and planet_view.fest_cell() >= 0:
		_sky_debug["flown"] = true
		planet_view.focus(planet_view.fest_cell(), true)
		_zoom_target = ZOOM_MAX


# ---------------------------------------------------------------- wonders

## `?look=<wonder>` in the window's address (the preview server's
## `/?look=whale`): fly close over that wonder once, for screenshots.
var _look_done := false


func _look_once() -> void:
	if _look_done or not OS.has_feature("web"):
		return
	_look_done = true
	var query := str(JavaScriptBridge.eval("location.search"))
	var at := query.find("look=")
	if at < 0:
		return
	var kind := query.substr(at + 5).get_slice("&", 0)
	for cell in planet_view.wonders:
		if planet_view.wonders[cell] == kind:
			planet_view.focus(cell, true)
			_zoom_target = ZOOM_MAX

var _wonders_seed := ""


## The wonders found on this planet, kept in the browser per planet.
func _load_wonders(seed: String) -> void:
	if seed == _wonders_seed:
		return
	_wonders_seed = seed
	var found := {}
	for kind in _load_pref("oorbit.wonders." + seed, "").split(",", false):
		found[kind] = true
	planet_view.found_wonders = found
	ui.found_wonders = found


## A click on a hidden wonder finds it: a toast, a chime, and it stops
## glinting.
func _discover(cell: int) -> void:
	var kind := planet_view.wonder_at(cell)
	if kind == "" or planet_view.found_wonders.has(kind) or ui.building_at(cell):
		return
	planet_view.found_wonders[kind] = true
	_save_pref("oorbit.wonders." + _wonders_seed, ",".join(planet_view.found_wonders.keys()))
	ui.found_wonders = planet_view.found_wonders
	audio.play("discovery")
	ui.toast(I18n.t("ui.wonder.found", {"name": I18n.name_of("wonder", kind),
		"count": planet_view.found_wonders.size(), "total": planet_view.wonders.size()}))


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
				if tile >= 0:
					audio.play("tap")
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

	_sun = DirectionalLight3D.new()
	_sun.light_color = Palette.SUN
	_sun.light_energy = 1.15
	_sun.rotation = Vector3(deg_to_rad(-30.0), deg_to_rad(-50.0), 0.0)
	world.add_child(_sun)

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
