## Oorbit's interface, drawn at full resolution over the pixel-art planet:
## the resource bar, the Research / Society / Chronicle panels, the selected
## tile, the event dialog and the founding screen. It never changes the game
## itself: it asks through `action_requested` and `found_requested`, and
## redraws from each view the plugin returns.
class_name GameUI
extends Control

signal action_requested(action: Dictionary)
signal found_requested(climate: String)
signal reset_requested
signal skip_requested(hours: float)
signal tile_closed
signal tile_focus_requested(index: int)
## An effect to play (see `PlanetAudio`), and the mute button.
signal sound_requested(name: String)
signal mute_toggled(on: bool)

const PANEL_WIDTH := 320.0
const NARROW := 640.0

var view := {}
var catalog := {}
var selected_tile := -1
var open_panel := ""
var dismissed_event := ""
var _was_founding := false
## Engine ticks (ms) when the current view arrived.
var _view_ticks := 0
var _tick_left := 1.0
var _countdown: Label
var _countdown_done_ms := 0.0
var _countdown_building := ""
var _research_bar: ProgressBar
var _research_label: Label
var _research_cost := 0.0

var _theme := Theme.new()
var _top: PanelContainer
var _top_row: HFlowContainer
var _rail: VBoxContainer
var _side: PanelContainer
var _side_body: VBoxContainer
var _tile: PanelContainer
var _tile_body: VBoxContainer
var _dialog: PanelContainer
var _dialog_body: VBoxContainer
var _founding: CenterContainer
var _toast: Label
var _toast_left := 0.0
var _buttons := {}


func _ready() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	_build_theme()
	theme = _theme

	_top = _panel()
	_top.set_anchors_and_offsets_preset(Control.PRESET_TOP_WIDE)
	_top.offset_left = 8
	_top.offset_right = -8
	_top.offset_top = 8
	add_child(_top)
	_top_row = HFlowContainer.new()
	_top_row.add_theme_constant_override("h_separation", 14)
	_top.add_child(_top_row)

	_rail = VBoxContainer.new()
	_rail.add_theme_constant_override("separation", 6)
	add_child(_rail)
	for id in ["goals", "research", "society", "chronicle", "dock", "dev"]:
		var button := _icon_button("res://icons/%s.svg" % id, I18n.t("ui." + id))
		button.toggle_mode = true
		button.pressed.connect(_toggle_panel.bind(id))
		_buttons[id] = button
		_rail.add_child(button)
	_mute = _icon_button("res://icons/sound.svg", I18n.t("ui.sound"))
	_mute.pressed.connect(func():
		set_muted(not _sound_off)
		mute_toggled.emit(_sound_off))
	_rail.add_child(_mute)

	_side = _panel()
	_side.visible = false
	add_child(_side)
	_side_body = _scroll_body(_side)

	_tile = _panel()
	_tile.visible = false
	add_child(_tile)
	_tile_body = _scroll_body(_tile)

	_dialog = _panel(true)
	_dialog.visible = false
	add_child(_dialog)
	_dialog_body = VBoxContainer.new()
	_dialog_body.add_theme_constant_override("separation", 8)
	_dialog.add_child(_dialog_body)

	_founding = CenterContainer.new()
	_founding.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_founding.visible = false
	add_child(_founding)

	_toast = Label.new()
	_toast.visible = false
	_toast.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_toast.add_theme_color_override("font_color", Color("f2c078"))
	add_child(_toast)

	resized.connect(_layout)
	_layout()


func _process(delta: float) -> void:
	# Between refreshes the numbers keep moving at their rates, and the
	# countdowns run, so the planet never looks frozen.
	_tick_left -= delta
	if _tick_left <= 0.0 and not view.is_empty() and view.get("planet") != null:
		_tick_left = 1.0
		_fill_top()
		if _countdown != null and is_instance_valid(_countdown):
			_countdown.text = _countdown_text()
		if _research_bar != null and is_instance_valid(_research_bar):
			_update_research_bar()
	if _toast.visible:
		_toast_left -= delta
		if _toast_left <= 0.0:
			_toast.visible = false


# ---------------------------------------------------------------- theme

func _build_theme() -> void:
	var panel := StyleBoxFlat.new()
	panel.bg_color = Color(0.118, 0.106, 0.2, 0.9)
	panel.border_color = Color("3d3361")
	panel.set_border_width_all(1)
	panel.set_corner_radius_all(6)
	panel.set_content_margin_all(10)
	_theme.set_stylebox("panel", "PanelContainer", panel)

	var normal := StyleBoxFlat.new()
	normal.bg_color = Color("2b2547")
	normal.set_corner_radius_all(5)
	normal.set_content_margin_all(6)
	var hover := normal.duplicate()
	hover.bg_color = Color("3d3361")
	var pressed := normal.duplicate()
	pressed.bg_color = Color("564a7d")
	var disabled := normal.duplicate()
	disabled.bg_color = Color(0.17, 0.15, 0.28, 0.6)
	for kind in ["Button"]:
		_theme.set_stylebox("normal", kind, normal)
		_theme.set_stylebox("hover", kind, hover)
		_theme.set_stylebox("pressed", kind, pressed)
		_theme.set_stylebox("hover_pressed", kind, pressed)
		_theme.set_stylebox("disabled", kind, disabled)
		_theme.set_stylebox("focus", kind, StyleBoxEmpty.new())
		_theme.set_color("font_color", kind, Color("eef3f2"))
		_theme.set_color("font_disabled_color", kind, Color(0.93, 0.95, 0.95, 0.4))
	_theme.set_color("font_color", "Label", Color("eef3f2"))
	_theme.set_font_size("font_size", "Label", 13)
	_theme.set_font_size("font_size", "Button", 13)

	var track := StyleBoxFlat.new()
	track.bg_color = Color("2b2547")
	track.set_corner_radius_all(3)
	track.content_margin_top = 3
	track.content_margin_bottom = 3
	_theme.set_stylebox("slider", "HSlider", track)
	var fill := StyleBoxFlat.new()
	fill.bg_color = Color("94b37e")
	fill.set_corner_radius_all(3)
	_theme.set_stylebox("grabber_area", "HSlider", fill)
	_theme.set_stylebox("grabber_area_highlight", "HSlider", fill)

	var bar_bg := StyleBoxFlat.new()
	bar_bg.bg_color = Color("2b2547")
	bar_bg.set_corner_radius_all(3)
	var bar_fill := StyleBoxFlat.new()
	bar_fill.bg_color = Color("f2c078")
	bar_fill.set_corner_radius_all(3)
	_theme.set_stylebox("background", "ProgressBar", bar_bg)
	_theme.set_stylebox("fill", "ProgressBar", bar_fill)


func _panel(strong := false) -> PanelContainer:
	var p := PanelContainer.new()
	p.mouse_filter = Control.MOUSE_FILTER_STOP
	if strong:
		var box: StyleBoxFlat = _theme.get_stylebox("panel", "PanelContainer").duplicate()
		box.bg_color = Color(0.118, 0.106, 0.2, 0.97)
		box.border_color = Color("f2c078")
		box.set_content_margin_all(16)
		p.add_theme_stylebox_override("panel", box)
	return p


func _scroll_body(parent: PanelContainer) -> VBoxContainer:
	var scroll := ScrollContainer.new()
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	parent.add_child(scroll)
	var margin := MarginContainer.new()
	margin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	margin.add_theme_constant_override("margin_right", 10)
	scroll.add_child(margin)
	var body := VBoxContainer.new()
	body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	body.add_theme_constant_override("separation", 6)
	margin.add_child(body)
	return body


func _icon_button(path: String, tip: String) -> Button:
	var b := Button.new()
	b.icon = load(path)
	b.expand_icon = true
	b.custom_minimum_size = Vector2(40, 40)
	b.tooltip_text = tip
	b.mouse_filter = Control.MOUSE_FILTER_STOP
	return b


func _label(text: String, size := 13, color := Color("eef3f2"), wrap := true) -> Label:
	var l := Label.new()
	l.text = text
	if wrap:
		l.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	l.add_theme_font_size_override("font_size", size)
	l.add_theme_color_override("font_color", color)
	return l


func _muted(text: String) -> Label:
	return _label(text, 12, Color(0.93, 0.95, 0.95, 0.65))


func _clear(box: Container) -> void:
	for child in box.get_children():
		box.remove_child(child)
		child.queue_free()


# ---------------------------------------------------------------- layout

func _narrow() -> bool:
	return size.x < NARROW


func _layout() -> void:
	var top_h := _top.get_combined_minimum_size().y + 8
	_rail.position = Vector2(8, top_h + 8)
	var narrow := _narrow()
	var width: float = size.x - 16 if narrow else PANEL_WIDTH
	if narrow:
		# Phone: one panel at a time, across the bottom half.
		for pair in [[_side, _side_body], [_tile, _tile_body]]:
			var h := _fit_height(pair[1], size.y * 0.55)
			pair[0].size = Vector2(width, h)
			pair[0].position = Vector2(8, size.y - h - 8)
	else:
		_side.position = Vector2(56, top_h + 8)
		_side.size = Vector2(width, _fit_height(_side_body, size.y - top_h - 16))
		_tile.position = Vector2(size.x - width - 8, top_h + 8)
		_tile.size = Vector2(width, _fit_height(_tile_body, size.y - top_h - 16))
	var dialog_w := minf(420.0, size.x - 32)
	_dialog.custom_minimum_size = Vector2(dialog_w, 0)
	_dialog.size = Vector2(dialog_w, _dialog.get_combined_minimum_size().y)
	_dialog.position = Vector2((size.x - dialog_w) / 2.0, maxf(top_h + 16, (size.y - _dialog.size.y) / 2.0))
	_toast.position = Vector2(16, size.y - 40)
	_toast.size = Vector2(size.x - 32, 24)


## A panel as tall as its content, up to `available`.
func _fit_height(body: Control, available: float) -> float:
	return minf(available, body.get_combined_minimum_size().y + 24.0)


## A short text rising and fading over a spot of the screen ("+3 wood").
func float_text(at: Vector2, text: String, color := Color("f2e1c8")) -> void:
	if get_child_count() > 80:
		return
	var l := _label(text, 14, color.lightened(0.35), false)
	l.add_theme_color_override("font_outline_color", Color("15132a"))
	l.add_theme_constant_override("outline_size", 4)
	l.mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_child(l)
	move_child(l, 0)
	l.position = at - Vector2(20, 24)
	var tween := l.create_tween()
	tween.set_parallel(true)
	tween.tween_property(l, "position:y", l.position.y - 28.0, 1.6)
	tween.tween_property(l, "modulate:a", 0.0, 1.6).set_ease(Tween.EASE_IN)
	tween.chain().tween_callback(l.queue_free)


func toast(text: String) -> void:
	_toast.text = text
	_toast.visible = true
	_toast_left = 5.0


# ---------------------------------------------------------------- views

func show_offline() -> void:
	_show_founding_message(I18n.t("ui.offline"))


func show_loading() -> void:
	_show_founding_message(I18n.t("ui.loading"))


func _show_founding_message(text: String) -> void:
	_top.visible = false
	_rail.visible = false
	_founding.visible = true
	_clear(_founding)
	# No wrapping: inside a CenterContainer a wrapped label shrinks to one
	# letter per line.
	_founding.add_child(_label(text, 15, Color("eef3f2"), false))


## Redraw everything from a view (the plugin's reply).
func show_view(new_view: Dictionary) -> void:
	view = new_view
	_view_ticks = Time.get_ticks_msec()
	var planet = view.get("planet")
	if planet == null:
		_show_founding()
		return
	_founding.visible = false
	_top.visible = true
	_rail.visible = true
	if _was_founding:
		# A planet was just founded: start by choosing what to study.
		_was_founding = false
		open_panel = "research"
	_fill_top()
	_fill_side()
	_fill_tile()
	_fill_event()
	_announce(planet)
	_layout.call_deferred()


func _show_founding() -> void:
	_was_founding = true
	_top.visible = false
	_rail.visible = false
	_side.visible = false
	_tile.visible = false
	_dialog.visible = false
	_founding.visible = true
	_clear(_founding)
	var card := _panel(true)
	card.custom_minimum_size = Vector2(minf(460.0, size.x - 32), 0)
	_founding.add_child(card)
	var body := VBoxContainer.new()
	body.add_theme_constant_override("separation", 10)
	card.add_child(body)
	body.add_child(_label(I18n.t("ui.title"), 22, Color("f2c078")))
	body.add_child(_label(I18n.t("ui.found.subtitle")))
	for climate in catalog.get("climates", []):
		var id: String = climate["id"]
		var b := Button.new()
		b.text = "%s\n%s" % [I18n.name_of("climate", id), I18n.t("climate.%s.desc" % id)]
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		b.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		b.pressed.connect(func(): found_requested.emit(id))
		body.add_child(b)


func _planet() -> Dictionary:
	return view.get("planet", {})


## The planet's time now, moving on since the view arrived: fast while
## someone plays (until the presence the view reported runs out), slow after.
func live_now_ms() -> float:
	var elapsed := float(Time.get_ticks_msec() - _view_ticks)
	var clock: Dictionary = view.get("clock", {})
	if clock.is_empty():
		return float(view.get("now_ms", 0)) + elapsed
	var fast := minf(elapsed, float(clock.get("present_left_ms", 0))) if clock.get("present", false) else 0.0
	return float(view.get("now_ms", 0)) + fast * float(clock.get("active_rate", 60)) + (elapsed - fast) * float(clock.get("away_rate", 0.5))


## Planet time per wall time right now.
func clock_rate() -> float:
	return maxf(0.01, float(view.get("clock", {}).get("rate", 1.0)))


## How long `planet_ms` of planet time takes on the wall clock at the
## current speed.
func wall_ms(planet_ms: float) -> float:
	return planet_ms / clock_rate()


## A rate per planet hour as the player feels it: per real minute while
## playing (a planet hour is a minute then), per real hour otherwise.
func _felt_rate(per_planet_hour: float) -> Array:
	if view.get("clock", {}).get("present", false):
		return [per_planet_hour * clock_rate() / 60.0, I18n.t("ui.per_minute")]
	return [per_planet_hour * clock_rate(), I18n.t("ui.per_hour")]


## Hours since the state's last simulation step.
func _hours_since_state() -> float:
	var step_ms := float(view.get("step_ms", 600000))
	return maxf(0.0, (live_now_ms() - float(_planet().get("step", 0)) * step_ms) / 3600000.0)


## A resource as it is right now: the saved amount moved on at its rate,
## within the storage cap for capped resources.
func live_amount(id: String) -> float:
	var value := float(_planet().get("resources", {}).get(id, 0.0)) + _rate(id) * _hours_since_state()
	value = maxf(0.0, value)
	var capped := true
	for r in catalog.get("resources", []):
		if r["id"] == id:
			capped = not r.get("uncapped", false)
	if capped:
		value = minf(value, maxf(float(view.get("rates", {}).get("storage", value)), float(_planet().get("resources", {}).get(id, 0.0))))
	return value


## Milliseconds until the next thing finishes (a building, the research), or
## -1 when nothing is under way. The window refreshes right after it.
func next_change_ms() -> float:
	var planet := _planet()
	var step_ms := float(view.get("step_ms", 600000))
	var best := -1.0
	for c in planet.get("construction", []):
		var left := float(c["done_step"]) * step_ms - live_now_ms()
		if best < 0.0 or left < best:
			best = left
	var research = planet.get("research")
	if research != null:
		for t in view.get("techs", []):
			if t["id"] == research:
				var rate := _rate("knowledge")
				var missing := float(t["cost"]) - live_amount("knowledge")
				if rate > 0.0:
					# Discoveries complete on a step boundary.
					var left := maxf(0.0, missing / rate * 3600000.0)
					left = ceil((live_now_ms() + left) / step_ms) * step_ms - live_now_ms()
					if best < 0.0 or left < best:
						best = left
	return best


func _rate(id: String) -> float:
	return float(view.get("rates", {}).get("per_hour", {}).get(id, 0.0))


func _fmt(value: float) -> String:
	if absf(value) >= 100.0:
		return str(roundi(value))
	return "%.1f" % value if absf(value - roundf(value)) > 0.05 else str(roundi(value))


func _fill_top() -> void:
	_clear(_top_row)
	var planet := _planet()
	var era = view.get("era")
	if era != null:
		_top_row.add_child(_label(I18n.name_of("era", era["id"]), 14, Color("f2c078"), false))
	var rates: Dictionary = view.get("rates", {})
	_top_row.add_child(_label("%s %s/%s" % [I18n.t("ui.population"), roundi(planet.get("population", 0)), roundi(rates.get("housing", 0))], 13, Color("eef3f2"), false))
	_top_row.add_child(_label("%s %d%%" % [I18n.t("ui.mood"), roundi(float(planet.get("mood", 0)) * 100.0)], 13, Color("eef3f2"), false))
	# Food, knowledge and culture always; materials while they matter: the
	# ones this era and the last build with.
	var era_n := int(view.get("era", {}).get("number", 1)) if view.get("era") != null else 1
	var relevant := {"food": true, "knowledge": true, "culture": true}
	for b in catalog.get("buildings", []):
		if int(b["era"]) >= era_n - 1 and int(b["era"]) <= era_n:
			for r in b["cost"]:
				relevant[r] = true
	for r in catalog.get("resources", []):
		var id: String = r["id"]
		if not relevant.has(id):
			continue
		var amount := live_amount(id)
		var rate := _rate(id)
		if amount < 0.5 and absf(rate) < 0.01:
			continue
		var sign := "+" if rate >= 0.0 else ""
		var felt := _felt_rate(rate)
		var l := _label("%s %s (%s%s%s)" % [I18n.name_of("resource", id), _fmt(amount), sign, _fmt(felt[0]), felt[1]], 13, Color("eef3f2"), false)
		if id == "food" and planet.get("starving", false):
			l.add_theme_color_override("font_color", Color("e08f6c"))
		_top_row.add_child(l)
	# How fast the planet's time runs: quick while you play, slow while away.
	var present: bool = view.get("clock", {}).get("present", false)
	_top_row.add_child(_label(I18n.t("ui.speed.fast" if present else "ui.speed.slow"), 13, Color("f2c078") if present else Color("9fb3c8"), false))


func _toggle_panel(id: String) -> void:
	open_panel = "" if open_panel == id else id
	if _narrow() and open_panel != "":
		tile_closed.emit()
	_fill_side()
	_fill_tile()
	_layout.call_deferred()


func _fill_side() -> void:
	for id in _buttons:
		_buttons[id].button_pressed = id == open_panel
	# The research icon glows while nothing is being studied but could be.
	var idle: bool = _planet().get("research") == null and view.get("techs", []).any(func(t): return t["status"] == "available")
	_buttons["research"].modulate = Color("f2c078") if idle else Color.WHITE
	_side.visible = open_panel != "" and not view.is_empty()
	if not _side.visible:
		return
	_clear(_side_body)
	match open_panel:
		"goals":
			_fill_goals()
		"research":
			_fill_research()
		"society":
			_fill_society()
		"chronicle":
			_fill_chronicle()
		"dock":
			_fill_dock()
		"dev":
			_fill_dev()


func _fill_research() -> void:
	var planet := _planet()
	_side_body.add_child(_label(I18n.t("ui.research"), 15, Color("f2c078")))
	var era = view.get("era")
	if planet.get("eras_done", false):
		_side_body.add_child(_muted(I18n.t("ui.eras.done")))
	elif era != null:
		var names := PackedStringArray()
		for t in era["requires"]:
			if not t in planet.get("known", []):
				names.append(I18n.name_of("tech", t))
		var monument := I18n.name_of("building", era["monument"])
		if names.is_empty():
			_side_body.add_child(_muted(I18n.t("ui.era.monument_only", {"monument": monument})))
		else:
			_side_body.add_child(_muted(I18n.t("ui.era.next", {"techs": ", ".join(names), "monument": monument})))
	var order := {"researching": 0, "available": 1, "not_affordable": 2, "locked": 3, "known": 4}
	var techs: Array = view.get("techs", []).duplicate()
	techs.sort_custom(func(a, b): return order.get(a["status"], 5) < order.get(b["status"], 5))
	var known := PackedStringArray()
	for tech in techs:
		var id: String = tech["id"]
		var status: String = tech["status"]
		if status == "known":
			known.append(I18n.name_of("tech", id))
			continue
		var row := VBoxContainer.new()
		var b := Button.new()
		b.text = "%s  ·  %s" % [I18n.name_of("tech", id), _fmt(float(tech["cost"]))]
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		b.disabled = status != "available"
		if status == "available":
			b.pressed.connect(func(): action_requested.emit({"type": "research", "tech": id}))
		row.add_child(b)
		if status == "researching":
			_research_cost = float(tech["cost"])
			_research_bar = ProgressBar.new()
			_research_bar.max_value = _research_cost
			_research_bar.show_percentage = false
			_research_bar.custom_minimum_size = Vector2(0, 6)
			row.add_child(_research_bar)
			_research_label = _muted("")
			row.add_child(_research_label)
			_update_research_bar()
		elif status != "available":
			row.add_child(_muted(I18n.t("ui.status." + status)))
		_side_body.add_child(row)
	if known.size() > 0:
		_side_body.add_child(_muted(I18n.t("ui.known", {"list": ", ".join(known)})))


func _fill_society() -> void:
	var planet := _planet()
	var rates: Dictionary = view.get("rates", {})
	_side_body.add_child(_label(I18n.t("ui.crews"), 15, Color("f2c078")))
	var orders: Array = planet.get("orders", [])
	var outputs: Array = rates.get("orders", [])
	_side_body.add_child(_muted(I18n.t("ui.crews.help", {"n": orders.size(), "max": int(view.get("slots", 9)), "people": _fmt(float(view.get("crew", 0)))})))
	for k in orders.size():
		var o: Dictionary = orders[k]
		var tile := int(o["tile"])
		var stock := float(planet.get("tiles", [])[tile].get("stock", 1.0))
		var out := float(outputs[k]) if k < outputs.size() else 0.0
		var felt := _felt_rate(out)
		var row := HBoxContainer.new()
		var row_text := I18n.t("ui.crew.row", {"resource": I18n.name_of("resource", o["resource"]), "rate": "+%s%s" % [_fmt(felt[0]), felt[1]], "stock": roundi(stock * 100.0)})
		if _is_deposit(str(o["resource"])):
			row_text = I18n.t("ui.crew.row_deposit", {"resource": I18n.name_of("resource", o["resource"]), "rate": "+%s%s" % [_fmt(felt[0]), felt[1]], "n": roundi(float(planet.get("tiles", [])[tile].get("deposit", 0.0)))})
		var text := _label(row_text)
		text.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		row.add_child(text)
		var look := Button.new()
		look.text = I18n.t("ui.crew.look")
		look.pressed.connect(func(): tile_focus_requested.emit(tile))
		row.add_child(look)
		var stop := Button.new()
		stop.text = I18n.t("ui.crew.stop")
		stop.pressed.connect(func(): action_requested.emit({"type": "stop_harvest", "tile": tile}))
		row.add_child(stop)
		_side_body.add_child(row)
	var thinkers := float(rates.get("workers", {}).get("knowledge", 0.0)) + float(rates.get("workers", {}).get("culture", 0.0))
	_side_body.add_child(_muted(I18n.t("ui.thinkers", {"n": _fmt(thinkers)})))
	var cities: Array = view.get("cities", [])
	if int(planet.get("era", 1)) >= 2 or cities.size() > 1:
		_side_body.add_child(_label(I18n.t("ui.cities"), 15, Color("f2c078")))
		_side_body.add_child(_muted(I18n.t("ui.cities.help", {"r": 4, "n": 5})))
		for k in cities.size():
			var c: Dictionary = cities[k]
			var row := HBoxContainer.new()
			var status := I18n.t("ui.city.capital") if k == 0 else (I18n.t("ui.city.linked") if c.get("connected", false) else I18n.t("ui.city.unlinked", {"n": roundi(float(view.get("unlinked_pace", 0.6)) * 100.0)}))
			var text := _label("%s · %s" % [c["name"], status])
			text.size_flags_horizontal = Control.SIZE_EXPAND_FILL
			row.add_child(text)
			var look := Button.new()
			look.text = I18n.t("ui.crew.look")
			var tile := int(c["tile"])
			look.pressed.connect(func(): tile_focus_requested.emit(tile))
			row.add_child(look)
			_side_body.add_child(row)
		var bonus := float(view.get("link_bonus", 0.0))
		if bonus > 0.0:
			_side_body.add_child(_muted(I18n.t("ui.city.bonus", {"n": roundi(bonus * 100.0)})))
	var notables: Array = planet.get("notables", [])
	if notables.size() > 0:
		_side_body.add_child(_label(I18n.t("ui.notables"), 14, Color("f2c078")))
		for n in notables:
			_side_body.add_child(_label("%s · %s" % [n["name"], I18n.name_of("category", n["category"])]))
	var traits: Dictionary = planet.get("traits", {})
	if traits.size() > 0:
		_side_body.add_child(_label(I18n.t("ui.traits"), 14, Color("f2c078")))
		for id in traits:
			_side_body.add_child(_label("%s %+d" % [I18n.t("ui.trait." + id), int(traits[id])]))
	# Starting over takes two presses, the second within a few seconds.
	_side_body.add_child(_confirm_button(I18n.t("ui.reset"), I18n.t("ui.reset.confirm"), func(): reset_requested.emit()))


func _fill_chronicle() -> void:
	var planet := _planet()
	_side_body.add_child(_label(I18n.t("ui.chronicle"), 15, Color("f2c078")))
	var entries: Array = planet.get("chronicle", [])
	var step := int(planet.get("step", 0))
	var step_ms := float(view.get("step_ms", 600000))
	var shown := 0
	for k in range(entries.size() - 1, -1, -1):
		var entry: Dictionary = entries[k]
		if entry["key"] in ["research_started", "building_started", "event"]:
			continue
		var ago := (step - int(entry["step"])) * step_ms
		_side_body.add_child(_label(describe(entry)))
		_side_body.add_child(_muted(I18n.t("ui.ago", {"time": I18n.duration(ago)}) if ago > 0 else "·"))
		shown += 1
		if shown >= 40:
			break


## A button that needs two presses within five seconds.
func _confirm_button(text: String, confirm: String, action: Callable) -> Button:
	var b := Button.new()
	b.text = text
	b.flat = true
	var quiet := Color(0.93, 0.95, 0.95, 0.55)
	b.add_theme_color_override("font_color", quiet)
	b.pressed.connect(func():
		if b.has_meta("armed"):
			action.call()
			return
		b.set_meta("armed", true)
		b.text = confirm
		b.add_theme_color_override("font_color", Color("e08f6c"))
		get_tree().create_timer(5.0).timeout.connect(func():
			if is_instance_valid(b):
				b.remove_meta("armed")
				b.text = text
				b.add_theme_color_override("font_color", quiet)))
	return b


func _neighbor(id: String) -> Dictionary:
	for n in view.get("dock", {}).get("neighbors", []):
		if n["id"] == id:
			return n
	return {}


func _era_name(number: int) -> String:
	var eras: Array = catalog.get("eras", [])
	if number >= 1 and number <= eras.size():
		return I18n.name_of("era", eras[number - 1]["id"])
	return str(number)


## The dock: neighbours, what can be sent to each, ships under way, offers.
func _fill_dock() -> void:
	var planet := _planet()
	var dock: Dictionary = view.get("dock", {})
	_side_body.add_child(_label(I18n.t("ui.dock"), 15, Color("f2c078")))
	_side_body.add_child(_muted(I18n.t("ui.dock.travel", {"time": I18n.duration(wall_ms(float(dock.get("travel_hours", 0)) * 3600000.0)), "capacity": _fmt(float(dock.get("capacity", 0)))})))

	var offers: Array = planet.get("offers", [])
	if offers.size() > 0:
		_side_body.add_child(_label(I18n.t("ui.dock.offers"), 14, Color("f2c078")))
		for o in offers:
			var give := "%s %s" % [_fmt(float(o["give_amount"])), I18n.name_of("resource", o["give_resource"]).to_lower()]
			var want := "%s %s" % [_fmt(float(o["want_amount"])), I18n.name_of("resource", o["want_resource"]).to_lower()]
			_side_body.add_child(_label(I18n.t("ui.dock.offer", {"name": _neighbor(o["from"]).get("name", "?"), "give": give, "want": want})))
			var row := HBoxContainer.new()
			var id := int(o["id"])
			var yes := Button.new()
			yes.text = I18n.t("ui.dock.accept")
			yes.disabled = live_amount(str(o["want_resource"])) < float(o["want_amount"])
			yes.pressed.connect(func(): action_requested.emit({"type": "accept_offer", "id": id}))
			row.add_child(yes)
			var no := Button.new()
			no.text = I18n.t("ui.dock.decline")
			no.flat = true
			no.pressed.connect(func(): action_requested.emit({"type": "decline_offer", "id": id}))
			row.add_child(no)
			_side_body.add_child(row)

	_side_body.add_child(_label(I18n.t("ui.dock.in_flight"), 14, Color("f2c078")))
	var ships: Array = planet.get("ships", [])
	if ships.is_empty():
		_side_body.add_child(_muted(I18n.t("ui.dock.none")))
	var step_ms := float(view.get("step_ms", 600000))
	for s in ships:
		var left := maxf(0.0, float(s["arrive_step"]) * step_ms - live_now_ms())
		var incoming: bool = s["to"] == "home"
		var other: String = s["from"] if incoming else s["to"]
		var key := "ui.dock.incoming" if incoming else "ui.dock.outgoing"
		_side_body.add_child(_muted(I18n.t(key, {"kind": I18n.t("kind." + str(s["kind"])), "name": _neighbor(other).get("name", "?"), "time": I18n.duration(wall_ms(left))})))

	var cap := float(dock.get("capacity", 0))
	for n in dock.get("neighbors", []):
		_side_body.add_child(_label("%s · %s · %s" % [n["name"], I18n.name_of("climate", n["climate"]), _era_name(int(n["era"]))], 14, Color("f2c078")))
		_side_body.add_child(_muted("%s · %s" % [I18n.t("persona." + str(n["persona"])), I18n.t("ui.dock.friendship", {"n": int(n["friendship"])})]))
		var to: String = n["id"]
		var flow := HFlowContainer.new()
		for k in dock.get("kinds", []):
			var kind: String = k["kind"]
			if not k["unlocked"]:
				continue
			if kind == "gift":
				continue
			var b := Button.new()
			b.text = I18n.t("kind." + kind)
			if kind == "idea":
				b.tooltip_text = I18n.t("ui.dock.idea_cost", {"n": _fmt(float(dock.get("idea_cost", 0)))})
			var send := {"type": "send", "to": to, "kind": kind}
			if kind == "prank":
				send["resource"] = "food"
			b.pressed.connect(func(): action_requested.emit(send))
			flow.add_child(b)
		_side_body.add_child(flow)
		# Gifts: a fair share of each resource there is plenty of.
		var gifts := HFlowContainer.new()
		var by_amount: Array = dock.get("resources", []).duplicate()
		by_amount.sort_custom(func(a, b): return live_amount(str(a)) > live_amount(str(b)))
		for r in by_amount.slice(0, 6):
			var have := live_amount(str(r))
			var amount := floorf(minf(cap, have * 0.25))
			if amount < 1.0:
				continue
			var g := Button.new()
			g.text = "%s %s" % [_fmt(amount), I18n.name_of("resource", r).to_lower()]
			var gift := {"type": "send", "to": to, "kind": "gift", "resource": r, "amount": amount}
			g.pressed.connect(func(): action_requested.emit(gift))
			gifts.add_child(g)
		if gifts.get_child_count() > 0:
			_side_body.add_child(_muted(I18n.t("kind.gift")))
			_side_body.add_child(gifts)
	var locked := PackedStringArray()
	for k in dock.get("kinds", []):
		if not k["unlocked"]:
			locked.append(I18n.t("ui.dock.locked", {"kind": I18n.t("kind." + str(k["kind"])), "n": int(k["first_era"])}))
	if locked.size() > 0:
		_side_body.add_child(_muted(", ".join(locked)))


## Testing only (temporary): move the planet's clock forward.
func _fill_dev() -> void:
	_side_body.add_child(_label(I18n.t("ui.dev"), 15, Color("f2c078")))
	_side_body.add_child(_muted(I18n.t("ui.dev.note")))
	var row := HFlowContainer.new()
	for pair in [["1h", 1.0], ["6h", 6.0], ["1g" if I18n.language == "it" else "1d", 24.0], ["3g" if I18n.language == "it" else "3d", 72.0]]:
		var b := Button.new()
		b.text = I18n.t("ui.dev.skip", {"n": pair[0]})
		var hours: float = pair[1]
		b.pressed.connect(func(): skip_requested.emit(hours))
		row.add_child(b)
	_side_body.add_child(row)


## A Chronicle entry in words.
func describe(entry: Dictionary) -> String:
	var params := {}
	var raw: Dictionary = entry.get("params", {})
	for k in raw:
		var v := str(raw[k])
		match k:
			"tech":
				params[k] = I18n.name_of("tech", v)
			"building":
				params[k] = I18n.name_of("building", v)
			"era":
				params[k] = I18n.name_of("era", v)
			"event":
				params[k] = I18n.name_of("event", v)
			"category":
				params[k] = I18n.name_of("category", v).to_lower()
			"climate":
				params[k] = I18n.name_of("climate", v)
			"resource":
				params[k] = I18n.name_of("resource", v).to_lower()
			"kind":
				params[k] = I18n.t("kind." + v).to_lower()
			"prank":
				params[k] = I18n.t("prank." + v)
			"find":
				params[k] = I18n.name_of("find", v).to_lower()
			"goal":
				params[k] = v
				for g in catalog.get("goals", []):
					if g["id"] == v:
						params[k] = _goal_text({"goal": g})
			"persona":
				params["message"] = I18n.t("message." + v)
			_:
				params[k] = v
	if raw.has("choice") and raw.has("event"):
		params["choice"] = I18n.t("choice.%s.%s" % [raw["event"], raw["choice"]]).to_lower()
	return I18n.t("log." + str(entry["key"]), params)


func select_tile(index: int) -> void:
	selected_tile = index
	if _narrow() and index >= 0:
		open_panel = ""
		_fill_side()
	_fill_tile()
	_layout.call_deferred()


func _fill_tile() -> void:
	var planet := _planet()
	var tiles: Array = planet.get("tiles", [])
	_tile.visible = selected_tile >= 0 and selected_tile < tiles.size() and not (_narrow() and open_panel != "")
	if not _tile.visible:
		return
	_clear(_tile_body)
	var tile: Dictionary = tiles[selected_tile]
	var head := HBoxContainer.new()
	var title := _label(I18n.name_of("biome", tile["biome"]), 15, Color("f2c078"))
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	head.add_child(title)
	var close := _icon_button("res://icons/close.svg", "")
	close.custom_minimum_size = Vector2(28, 28)
	close.pressed.connect(func(): tile_closed.emit())
	head.add_child(close)
	_tile_body.add_child(head)

	var city := _city_at(selected_tile)
	if city >= 0:
		_fill_city(city)
	var building = tile.get("building")
	if building != null:
		_tile_body.add_child(_label(I18n.name_of("building", building), 14))
		_tile_body.add_child(_muted(_effects_text(_building_def(building))))
		var def := _building_def(building)
		if not def.get("monument", false) and not def.get("city", false):
			var tile_index := selected_tile
			_tile_body.add_child(_confirm_button(I18n.t("ui.demolish"), I18n.t("ui.demolish.confirm"),
				func(): action_requested.emit({"type": "demolish", "tile": tile_index})))
		return
	_fill_find_and_event()
	for c in planet.get("construction", []):
		if int(c["tile"]) == selected_tile:
			_countdown_done_ms = float(c["done_step"]) * float(view.get("step_ms", 600000))
			_countdown_building = str(c["building"])
			_countdown = _label(_countdown_text())
			_tile_body.add_child(_countdown)
			var help: Dictionary = view.get("help", {})
			if help.has(str(selected_tile)):
				var saved := float(help[str(selected_tile)])
				var b := Button.new()
				b.text = I18n.t("ui.help", {"time": I18n.duration(wall_ms(saved))})
				b.alignment = HORIZONTAL_ALIGNMENT_LEFT
				var tile_index := selected_tile
				b.pressed.connect(func(): action_requested.emit({"type": "help", "tile": tile_index}))
				_tile_body.add_child(b)
				_tile_body.add_child(_muted(I18n.t("ui.help.cost", {"time": I18n.duration(wall_ms(saved))})))
			elif float(view.get("helping_until_ms", 0)) > live_now_ms():
				_tile_body.add_child(_muted(I18n.t("ui.help.busy")))
			return
	_fill_crew(tile)
	_fill_ground(tile)
	if tile["biome"] == "ocean":
		if not view.get("harvest", {}).has(str(selected_tile)):
			_tile_body.add_child(_muted(I18n.t("ui.tile.sea")))
		return
	# JSON numbers arrive as floats: compare cell indices as ints.
	var reachable := false
	for t in view.get("buildable", []):
		reachable = reachable or int(t) == selected_tile
	var site := false
	for t in view.get("city_sites", []):
		site = site or int(t) == selected_tile
	if not reachable and not site:
		_tile_body.add_child(_muted(I18n.t("ui.tile.far")))
		return
	_tile_body.add_child(_muted(I18n.t("ui.tile.empty") if reachable else I18n.t("ui.tile.site")))
	for o in planet.get("orders", []):
		if int(o["tile"]) == selected_tile:
			_tile_body.add_child(_label(I18n.t("ui.build.moves_crew"), 12, Color("e3cfa0")))
	var any := false
	for b in view.get("buildings", []):
		if not _fits(b, tile["biome"]):
			continue
		var founds: bool = b.get("city", false)
		if (founds and not site) or (not founds and not reachable):
			continue
		if b["status"] == "locked" or b["status"] == "monument_done":
			continue
		any = true
		var id: String = b["id"]
		var button := Button.new()
		var costs := PackedStringArray()
		for r in b["cost"]:
			costs.append("%s %s" % [_fmt(float(b["cost"][r])), I18n.name_of("resource", r).to_lower()])
		var name := I18n.name_of("building", id)
		if b.get("monument", false):
			name = "%s · %s" % [name, I18n.t("ui.monument")]
		button.text = "%s\n%s · %s" % [name, ", ".join(costs), I18n.duration(wall_ms(float(b["build_hours"]) * 3600000.0))]
		button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		button.icon = load("res://icons/build.svg")
		button.expand_icon = false
		button.add_theme_constant_override("icon_max_width", 18)
		button.disabled = b["status"] != "available"
		var tile_index := selected_tile
		button.pressed.connect(func(): action_requested.emit({"type": "build", "building": id, "tile": tile_index}))
		_tile_body.add_child(button)
		var effects := _effects_text(_building_def(id))
		if effects != "":
			_tile_body.add_child(_muted(effects))
	if not any:
		_tile_body.add_child(_muted(I18n.t("ui.tile.none")))


## The crew working this cell, or buttons to send one.
func _fill_crew(tile: Dictionary) -> void:
	var planet := _planet()
	var orders: Array = planet.get("orders", [])
	var outputs: Array = view.get("rates", {}).get("orders", [])
	for k in orders.size():
		if int(orders[k]["tile"]) != selected_tile:
			continue
		var out := float(outputs[k]) if k < outputs.size() else 0.0
		var felt := _felt_rate(out)
		_tile_body.add_child(_label(I18n.t("ui.crew.here", {"resource": I18n.name_of("resource", orders[k]["resource"]).to_lower(), "rate": "+%s%s" % [_fmt(felt[0]), felt[1]]}), 14, Color("c2d69b")))
		if _is_deposit(str(orders[k]["resource"])):
			_tile_body.add_child(_muted(I18n.t("ui.crew.deposit", {"n": roundi(float(tile.get("deposit", 0.0)))})))
		else:
			_tile_body.add_child(_muted(I18n.t("ui.crew.stock", {"stock": roundi(float(tile.get("stock", 1.0)) * 100.0)})))
		var stop := Button.new()
		stop.text = I18n.t("ui.crew.stop")
		var tile_index := selected_tile
		stop.pressed.connect(func(): action_requested.emit({"type": "stop_harvest", "tile": tile_index}))
		_tile_body.add_child(stop)
		return
	var options: Array = view.get("harvest", {}).get(str(selected_tile), [])
	if options.is_empty():
		return
	var reachable := false
	for t in view.get("work_reach", []):
		reachable = reachable or int(t) == selected_tile
	if not reachable:
		_tile_body.add_child(_muted(I18n.t("ui.crew.far")))
		return
	if orders.size() >= int(view.get("slots", 9)):
		_tile_body.add_child(_muted(I18n.t("ui.crew.full")))
		return
	if float(tile.get("stock", 1.0)) < 0.99 and tile["biome"] != "forest":
		_tile_body.add_child(_muted(I18n.t("ui.crew.stock", {"stock": roundi(float(tile.get("stock", 1.0)) * 100.0)})))
	# What one more crew costs the thinkers.
	var rates: Dictionary = view.get("rates", {})
	var thinkers := float(rates.get("workers", {}).get("knowledge", 0.0)) + float(rates.get("workers", {}).get("culture", 0.0))
	var know := float(rates.get("per_hour", {}).get("knowledge", 0.0))
	if thinkers > 0.0 and know > 0.0:
		var after := know * maxf(0.0, thinkers - float(view.get("crew", 0.0))) / thinkers
		var a := _felt_rate(know)
		var b := _felt_rate(after)
		_tile_body.add_child(_muted(I18n.t("ui.crew.cost", {"from": "+%s%s" % [_fmt(a[0]), a[1]], "to": "+%s%s" % [_fmt(b[0]), b[1]]})))
	for resource in options:
		var b := Button.new()
		b.text = I18n.t("ui.crew.send", {"resource": I18n.name_of("resource", resource).to_lower()})
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		var tile_index := selected_tile
		var res: String = resource
		b.pressed.connect(func(): action_requested.emit({"type": "harvest", "tile": tile_index, "resource": res}))
		_tile_body.add_child(b)


func _countdown_text() -> String:
	var left := maxf(0.0, _countdown_done_ms - live_now_ms())
	return I18n.t("ui.tile.building", {"building": I18n.name_of("building", _countdown_building), "time": I18n.duration(wall_ms(left))})


func _update_research_bar() -> void:
	var have := minf(live_amount("knowledge"), _research_cost)
	_research_bar.value = have
	_research_label.text = I18n.t("ui.research.progress", {"have": _fmt(have), "cost": _fmt(_research_cost)})


func _fits(b: Dictionary, biome: String) -> bool:
	var biomes: Array = b.get("biomes", [])
	return biomes.is_empty() or biome in biomes


func _building_def(id: String) -> Dictionary:
	for b in catalog.get("buildings", []):
		if b["id"] == id:
			return b
	return {}


func _effects_text(def: Dictionary) -> String:
	var parts := PackedStringArray()
	for e in def.get("effects", []):
		match str(e["type"]):
			"housing":
				parts.append(I18n.t("ui.effect.housing", {"n": _fmt(float(e["amount"]))}))
			"bonus":
				parts.append(I18n.t("ui.effect.bonus", {"n": roundi(float(e["amount"]) * 100.0), "category": I18n.name_of("category", e["category"]).to_lower()}))
			"storage":
				parts.append(I18n.t("ui.effect.storage", {"n": _fmt(float(e["amount"]))}))
			"mood":
				parts.append(I18n.t("ui.effect.mood", {"n": roundi(float(e["amount"]) * 100.0)}))
			"convert":
				parts.append(I18n.t("ui.effect.convert", {"output": I18n.name_of("resource", e["output"]).to_lower()}))
	if def.get("city", false):
		parts.insert(0, I18n.t("ui.effect.city"))
	return ", ".join(parts)


func _fill_event() -> void:
	var event = view.get("event")
	_dialog.visible = event != null and event["id"] != dismissed_event
	if not _dialog.visible:
		return
	_clear(_dialog_body)
	var id: String = event["id"]
	_dialog_body.add_child(_label(I18n.name_of("event", id), 17, Color("f2c078")))
	_dialog_body.add_child(_label(I18n.t("event.%s.text" % id)))
	for choice in event["choices"]:
		var cid: String = choice["id"]
		var b := Button.new()
		var outcome := _outcomes_text(choice.get("outcomes", []))
		b.text = I18n.t("choice.%s.%s" % [id, cid]) + ("\n" + outcome if outcome != "" else "")
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		b.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		b.pressed.connect(func():
			dismissed_event = id
			_dialog.visible = false
			action_requested.emit({"type": "resolve_event", "choice": cid}))
		_dialog_body.add_child(b)


func _outcomes_text(outcomes: Array) -> String:
	var parts := PackedStringArray()
	for o in outcomes:
		match str(o["type"]):
			"resource":
				parts.append("%+d %s" % [roundi(float(o["amount"])), I18n.name_of("resource", o["id"]).to_lower()])
			"population":
				parts.append("%+d %s" % [roundi(float(o["amount"])), I18n.t("ui.population").to_lower()])
			"mood":
				parts.append("%+d %s" % [roundi(float(o["amount"]) * 100.0), I18n.t("ui.mood").to_lower()])
			"grant":
				parts.append("+ " + I18n.name_of("tech", o["tech"]))
			"trait":
				parts.append("%+d %s" % [int(o["amount"]), I18n.t("ui.trait." + str(o["id"])).to_lower()])
	return ", ".join(parts)


func _city_at(tile: int) -> int:
	var cities: Array = view.get("cities", [])
	for k in cities.size():
		if int(cities[k]["tile"]) == tile:
			return k
	return -1


func _city_name(k: int) -> String:
	var cities: Array = view.get("cities", [])
	return str(cities[k]["name"]) if k >= 0 and k < cities.size() else "?"


## A city: its name, whether it reaches the capital, its roads and sea
## routes (being laid or open), and the ways that could be opened.
func _fill_city(k: int) -> void:
	var cities: Array = view.get("cities", [])
	var c: Dictionary = cities[k]
	_tile_body.add_child(_label(str(c["name"]), 16, Color("f2e1c8")))
	if k == 0:
		_tile_body.add_child(_muted(I18n.t("ui.city.capital")))
	elif c.get("connected", false):
		_tile_body.add_child(_muted(I18n.t("ui.city.linked")))
	else:
		_tile_body.add_child(_label(I18n.t("ui.city.unlinked", {"n": roundi(float(view.get("unlinked_pace", 0.6)) * 100.0)}), 13, Color("e08f6c")))
	var way_name := func(kind: String, other: int) -> String:
		return I18n.t("ui.city.road_to" if kind == "road" else "ui.city.sea_to", {"city": _city_name(other)})
	for l in view.get("links", []):
		var a := int(l["from"])
		var b := int(l["to"])
		if a != k and b != k:
			continue
		var other := b if a == k else a
		var name: String = way_name.call(str(l["kind"]), other)
		if l.get("done", false):
			_tile_body.add_child(_label(name, 13, Color("c2d69b")))
		else:
			var start := float(l["start_ms"])
			var done := float(l["done_ms"])
			var p := clampf((live_now_ms() - start) / maxf(1.0, done - start), 0.0, 1.0)
			_tile_body.add_child(_label(I18n.t("ui.city.way_building", {"way": name, "n": roundi(p * 100.0)})))
	var any_sea := false
	for r in view.get("routes", []):
		var a := int(r["from"])
		var b := int(r["to"])
		if a != k and b != k:
			continue
		var other := b if a == k else a
		var kind := str(r["kind"])
		any_sea = any_sea or kind == "sea"
		var costs := PackedStringArray()
		var affordable := true
		for id in r["cost"]:
			var need := float(r["cost"][id])
			costs.append("%s %s" % [_fmt(need), I18n.name_of("resource", id).to_lower()])
			affordable = affordable and live_amount(id) >= need
		var button := Button.new()
		button.text = "%s\n%s · %s" % [way_name.call(kind, other), ", ".join(costs), I18n.duration(wall_ms(float(r["hours"]) * 3600000.0))]
		button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		button.disabled = not affordable
		button.pressed.connect(func(): action_requested.emit({"type": "link", "kind": kind, "from": a, "to": b}))
		_tile_body.add_child(button)
	if cities.size() > 1 and not any_sea and not c.get("shore", false):
		_tile_body.add_child(_muted(I18n.t("ui.city.port_hint")))


func _is_deposit(resource: String) -> bool:
	return resource in view.get("deposits", [])


## What the cell holds: trees (and planting them), or a deposit to dig.
func _fill_ground(tile: Dictionary) -> void:
	var biome := str(tile["biome"])
	var deposit := float(tile.get("deposit", 0.0))
	var digs := false
	for r in view.get("harvest", {}).get(str(selected_tile), []):
		digs = digs or _is_deposit(str(r))
	if biome == "forest":
		_tile_body.add_child(_muted(I18n.t("ui.tile.trees", {"stock": roundi(float(tile.get("stock", 1.0)) * 100.0)})))
		var planet := _planet()
		if float(tile.get("planted_until", 0)) > float(planet.get("step", 0)):
			_tile_body.add_child(_label(I18n.t("ui.planted"), 13, Color("c2d69b")))
		var plantable := false
		for t in view.get("plantable", []):
			plantable = plantable or int(t) == selected_tile
		if plantable:
			var food := float(view.get("plant_food", 6.0))
			var b := Button.new()
			b.text = I18n.t("ui.plant", {"food": _fmt(food)})
			b.alignment = HORIZONTAL_ALIGNMENT_LEFT
			b.disabled = live_amount("food") < food
			var tile_index := selected_tile
			b.pressed.connect(func(): action_requested.emit({"type": "plant", "tile": tile_index}))
			_tile_body.add_child(b)
			_tile_body.add_child(_muted(I18n.t("ui.plant.help")))
	elif deposit > 0.0 and digs:
		_tile_body.add_child(_muted(I18n.t("ui.tile.deposit", {"n": roundi(deposit)})))
	elif deposit == 0.0 and biome == "mountain":
		_tile_body.add_child(_muted(I18n.t("ui.tile.exhausted")))


# ---------------------------------------------------------------- edge marks

var _mark_buttons := []


## Finds and events on the hidden side of the planet (or off screen): a
## small button on the edge, in their direction; a press flies there.
func set_marks(list: Array) -> void:
	var hidden := list.filter(func(m): return m["hidden"])
	while _mark_buttons.size() < hidden.size():
		var b := Button.new()
		b.custom_minimum_size = Vector2(30, 30)
		b.focus_mode = Control.FOCUS_NONE
		b.add_theme_font_size_override("font_size", 15)
		add_child(b)
		move_child(b, 0)
		_mark_buttons.append(b)
	var centre := size / 2.0
	for k in _mark_buttons.size():
		var b: Button = _mark_buttons[k]
		b.visible = k < hidden.size()
		if not b.visible:
			continue
		var m: Dictionary = hidden[k]
		var event: bool = m["kind"] == "event"
		b.text = "!" if event else "*"
		b.add_theme_color_override("font_color", Color("e08f6c") if event else Color("f2c078"))
		b.tooltip_text = I18n.t("ui.mark.event") if event else I18n.name_of("find", str(m["kind"]).substr(5))
		# Towards the mark from the centre, clamped to the edge.
		var dir: Vector2 = (m["screen"] as Vector2) - centre
		if dir.length() < 1.0:
			dir = Vector2(0, 1)
		# Inside the margins: clear of the top bar and the left rail.
		var area := Rect2(Vector2(72, _top.size.y + 20), size - Vector2(72 + 24, _top.size.y + 20 + 24))
		var mid := area.get_center()
		dir = (m["screen"] as Vector2) - mid
		if dir.length() < 1.0:
			dir = Vector2(0, 1)
		var half := area.size / 2.0
		var t := minf(half.x / maxf(0.001, absf(dir.x)), half.y / maxf(0.001, absf(dir.y)))
		b.position = mid + dir * minf(t, 1.0) - b.custom_minimum_size / 2.0
		if b.pressed.is_connected(_on_mark):
			b.pressed.disconnect(_on_mark)
		b.pressed.connect(_on_mark.bind(int(m["tile"])))


func _on_mark(tile: int) -> void:
	tile_focus_requested.emit(tile)


func _reward_text(reward: Dictionary) -> String:
	var parts := PackedStringArray()
	for r in reward:
		parts.append("+%s %s" % [_fmt(float(reward[r])), I18n.name_of("resource", r).to_lower()])
	return ", ".join(parts)


## A find on this cell (with a button to bring it home), or the event
## happening here (with a button to answer it).
func _fill_find_and_event() -> void:
	for f in view.get("finds", []):
		if int(f["tile"]) != selected_tile:
			continue
		var kind := str(f["kind"])
		_tile_body.add_child(_label(I18n.name_of("find", kind), 15, Color("f2c078")))
		_tile_body.add_child(_muted(I18n.t("find.%s.text" % kind)))
		var b := Button.new()
		b.text = I18n.t("ui.find.collect", {"reward": _reward_text(f.get("reward", {}))})
		b.alignment = HORIZONTAL_ALIGNMENT_LEFT
		var id := int(f["id"])
		b.pressed.connect(func(): action_requested.emit({"type": "collect", "id": id}))
		_tile_body.add_child(b)
		_tile_body.add_child(_muted(I18n.t("ui.find.left", {"time": I18n.duration(wall_ms(maxf(0.0, float(f["until_ms"]) - live_now_ms())))})))
	var event = _planet().get("pending_event")
	if event != null and event.get("tile") != null and int(event["tile"]) == selected_tile:
		_tile_body.add_child(_label(I18n.name_of("event", str(event["id"])), 15, Color("e08f6c")))
		var b := Button.new()
		b.text = I18n.t("ui.event.answer")
		b.pressed.connect(func():
			dismissed_event = ""
			_fill_event())
		_tile_body.add_child(b)


func _goal_text(g: Dictionary) -> String:
	var goal: Dictionary = g["goal"]
	var kind := str(goal["type"])
	var params := {}
	for k in goal:
		var v = goal[k]
		match k:
			"tech":
				params[k] = I18n.name_of("tech", str(v))
			"building":
				params[k] = I18n.name_of("building", str(v))
			"resource":
				params[k] = I18n.name_of("resource", str(v)).to_lower()
			"count", "amount":
				params[k] = _fmt(float(v))
	return I18n.t("goal." + kind, params)


func _fill_goals() -> void:
	_side_body.add_child(_label(I18n.t("ui.goals"), 15, Color("f2c078")))
	var goals: Array = view.get("goals", [])
	if goals.is_empty():
		_side_body.add_child(_muted(I18n.t("ui.goals.none")))
	for g in goals:
		_side_body.add_child(_label(_goal_text(g), 14))
		var bar := ProgressBar.new()
		bar.max_value = maxf(1.0, float(g["need"]))
		bar.value = float(g["have"])
		bar.show_percentage = false
		bar.custom_minimum_size = Vector2(0, 6)
		_side_body.add_child(bar)
		_side_body.add_child(_muted("%s / %s · %s" % [_fmt(float(g["have"])), _fmt(float(g["need"])), _reward_text(g.get("reward", {}))]))
	_side_body.add_child(_muted(I18n.t("ui.goals.done", {"n": int(view.get("goals_done", 0))})))


var _seen_entries := -1


## Tell what was just reached: a goal, a find brought home.
func _announce(planet: Dictionary) -> void:
	var chronicle: Array = planet.get("chronicle", [])
	var last := 0
	if not chronicle.is_empty():
		last = int(chronicle[-1]["step"]) * 1000 + chronicle.size() % 1000
	if _seen_entries < 0:
		_seen_entries = last
		return
	if last == _seen_entries:
		return
	var said := false
	for k in range(chronicle.size() - 1, maxi(-1, chronicle.size() - 8), -1):
		var e: Dictionary = chronicle[k]
		if int(e["step"]) * 1000 + (k + 1) % 1000 <= _seen_entries:
			break
		var key := str(e["key"])
		if not said and key in ["goal_done", "find_collected", "city_founded", "era_reached", "deposit_exhausted"]:
			toast(describe(e))
			said = true
		var sound: String = ANNOUNCE_SOUNDS.get(key, "")
		if sound != "":
			sound_requested.emit(sound)
	_seen_entries = last


const ANNOUNCE_SOUNDS := {
	"building_done": "done", "goal_done": "done", "road_done": "done", "tech_done": "discovery",
	"era_reached": "era", "city_founded": "era", "find_collected": "find", "sea_done": "boat",
	"sea_started": "boat", "building_started": "build", "road_started": "build", "site_helped": "build",
	"trees_planted": "build",
}
var _mute: Button
var _sound_off := false


func set_muted(on: bool) -> void:
	_sound_off = on
	if _mute != null:
		_mute.icon = load("res://icons/mute.svg" if on else "res://icons/sound.svg")
		_mute.tooltip_text = I18n.t("ui.sound.off" if on else "ui.sound")
