## The game's strings, from res://i18n/<lang>.json, with English as the
## fallback for any missing language or key. LiteAura hands the language over
## in the host context.
class_name I18n
extends RefCounted

static var _strings := {}
static var _fallback := {}
static var language := "en"


static func _read(lang: String) -> Dictionary:
	var file := FileAccess.open("res://i18n/%s.json" % lang, FileAccess.READ)
	if file == null:
		return {}
	var value = JSON.parse_string(file.get_as_text())
	return value if value is Dictionary else {}


static func set_language(locale: String) -> void:
	if _fallback.is_empty():
		_fallback = _read("en")
	var lang := locale.split("-")[0].split("_")[0].to_lower()
	language = lang
	_strings = _read(lang) if lang != "en" else _fallback


## The string for `key`, with `{name}` placeholders filled from `params`.
static func t(key: String, params := {}) -> String:
	if _fallback.is_empty():
		set_language(language)
	var text: String = _strings.get(key, _fallback.get(key, key))
	for k in params:
		text = text.replace("{%s}" % k, str(params[k]))
	return text


## A catalog id's name: `kind` is resource, tech, building, era, climate,
## biome, event or category.
static func name_of(kind: String, id: String) -> String:
	return t("%s.%s" % [kind, id])


## "3h 20m" for a number of milliseconds.
static func duration(ms: float) -> String:
	var minutes := int(ceil(ms / 60000.0))
	if minutes >= 48 * 60:
		return t("ui.time.d", {"d": roundi(minutes / 1440.0)})
	if minutes < 60:
		return t("ui.time.m", {"m": minutes})
	return t("ui.time.hm", {"h": minutes / 60, "m": minutes % 60})
