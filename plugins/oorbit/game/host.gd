## The game's side of the page bridge in `web/index.html` (`window.oorbitHost`).
## Every call to LiteAura (plugin routes, host context) goes through here, so
## the rest of the game never touches JavaScript. Outside a browser (the
## editor, a desktop run) the bridge is absent and `available` is false.
class_name Host
extends RefCounted

signal context_changed(context: Dictionary)

var available := false
var _host: JavaScriptObject
var _callbacks: Array[JavaScriptObject] = []


func _init() -> void:
	if not OS.has_feature("web"):
		return
	_host = JavaScriptBridge.get_interface("oorbitHost")
	available = _host != null
	if available:
		var on_context := JavaScriptBridge.create_callback(_on_context)
		_callbacks.append(on_context)
		_host.onContext(on_context)


## The host context LiteAura handed to the frame: `locale`, `theme`, `mode`.
func context() -> Dictionary:
	if not available:
		return {}
	return _parse(_host.contextJson())


## GET a plugin route (`path` relative to `/api/oorbit/`). `done` receives the
## parsed JSON envelope, or `{ "success": false, "error": ... }`.
func get_json(path: String, done: Callable) -> void:
	if not available:
		done.call({"success": false, "error": "no host bridge"})
		return
	var holder: Array[JavaScriptObject] = []
	var on_reply := JavaScriptBridge.create_callback(func(args: Array) -> void:
		_callbacks.erase(holder[0])
		done.call(_parse(str(args[0])))
	)
	holder.append(on_reply)
	_callbacks.append(on_reply)
	_host.getJson(path, on_reply)


func _on_context(args: Array) -> void:
	context_changed.emit(_parse(str(args[0])))


func _parse(text: String) -> Dictionary:
	var value = JSON.parse_string(text)
	return value if value is Dictionary else {}
