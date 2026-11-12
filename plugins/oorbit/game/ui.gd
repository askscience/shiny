## Oorbit's interface, drawn at full resolution over the pixel-art planet:
## the resource bar, the Research / Society / Chronicle panels, the selected
## tile, the event dialog and the founding screen. It never changes the game
## itself: it asks through `action_requested` and `found_requested`, and
## redraws from each view the plugin returns.
class_name GameUI
extends Control

signal action_requested(action: Dictionary)
signal found_requested(climate: String)
signal tile_closed

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
	for id in ["research", "society", "chronicle"]:
		var button := _icon_button("res://icons/%s.svg" % id, I18n.t("ui." + id))
		button.toggle_mode = true
		button.pressed.connect(_toggle_panel.bind(id))
		_buttons[id] = button
		_rail.add_child(button)

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
	_founding.add_child(_label(text, 15))


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


## The current time on the server's clock, moving on since the view arrived.
func live_now_ms() -> float:
	return float(view.get("now_ms", 0)) + float(Time.get_ticks_msec() - _view_ticks)


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
	var resources: Dictionary = planet.get("resources", {})
	for r in catalog.get("resources", []):
		var id: String = r["id"]
		var amount := live_amount(id)
		var rate := _rate(id)
		if amount < 0.5 and absf(rate) < 0.01:
			continue
		var sign := "+" if rate >= 0.0 else ""
		var l := _label("%s %s (%s%s%s)" % [I18n.name_of("resource", id), _fmt(amount), sign, _fmt(rate), I18n.t("ui.per_hour")], 13, Color("eef3f2"), false)
		if id == "food" and planet.get("starving", false):
			l.add_theme_color_override("font_color", Color("e08f6c"))
		_top_row.add_child(l)


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
		"research":
			_fill_research()
		"society":
			_fill_society()
		"chronicle":
			_fill_chronicle()


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
	_side_body.add_child(_label(I18n.t("ui.priorities"), 15, Color("f2c078")))
	var priorities: Dictionary = planet.get("priorities", {})
	for category in ["food", "materials", "knowledge", "culture"]:
		var row := HBoxContainer.new()
		var name := _label(I18n.name_of("category", category))
		name.custom_minimum_size = Vector2(96, 0)
		row.add_child(name)
		var slider := HSlider.new()
		slider.min_value = 0
		slider.max_value = 4
		slider.step = 1
		slider.value = int(priorities.get(category, 0))
		slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		slider.custom_minimum_size = Vector2(0, 24)
		slider.drag_ended.connect(func(_changed: bool): _send_priorities())
		slider.set_meta("category", category)
		row.add_child(slider)
		_side_body.add_child(row)
		var workers := float(rates.get("workers", {}).get(category, 0.0))
		_side_body.add_child(_muted(I18n.t("ui.workers", {"n": _fmt(workers)})))
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


func _send_priorities() -> void:
	var priorities := {}
	for row in _side_body.get_children():
		for child in row.get_children():
			if child is HSlider:
				priorities[child.get_meta("category")] = int(child.value)
	if priorities.size() == 4:
		action_requested.emit({"type": "set_priorities", "priorities": priorities})


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

	var building = tile.get("building")
	if building != null:
		_tile_body.add_child(_label(I18n.name_of("building", building), 14))
		_tile_body.add_child(_muted(_effects_text(_building_def(building))))
		return
	for c in planet.get("construction", []):
		if int(c["tile"]) == selected_tile:
			_countdown_done_ms = float(c["done_step"]) * float(view.get("step_ms", 600000))
			_countdown_building = str(c["building"])
			_countdown = _label(_countdown_text())
			_tile_body.add_child(_countdown)
			return
	_tile_body.add_child(_muted(I18n.t("ui.tile.empty")))
	var any := false
	for b in view.get("buildings", []):
		# JSON numbers arrive as floats: compare tile indices as ints.
		var free := false
		for t in b.get("tiles", []):
			free = free or int(t) == selected_tile
		if not free and b["status"] != "not_affordable":
			continue
		if not _fits(b, tile["biome"]):
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
		button.text = "%s\n%s · %s" % [name, ", ".join(costs), I18n.t("ui.hours", {"n": _fmt(float(b["build_hours"]))})]
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


func _countdown_text() -> String:
	var left := maxf(0.0, _countdown_done_ms - live_now_ms())
	return I18n.t("ui.tile.building", {"building": I18n.name_of("building", _countdown_building), "time": I18n.duration(left)})


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
