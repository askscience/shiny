## The planet's sound: ambient loops mixed by what the camera looks at, and
## short effects placed where they happen.
##
## From far away only the hum of space and a breath of wind; coming down to
## the ground, the sea where the view is on water, birds over forests,
## voices over towns, crickets on the meadows. Turning the planet changes the
## mix slowly. Effects fade with their distance from the middle of the view
## and are silent on the far side; only a few play at once, and the same one
## never stacks up. The sounds come from `scripts/synth-sounds.py`.
class_name PlanetAudio
extends Node

const AMBIENTS := ["space", "wind", "waves", "birds", "village", "crickets"]
const EFFECTS := ["tap", "build", "hammer", "done", "discovery", "era", "deliver", "boat", "find", "error"]
## Loudness of each effect (linear), before distance.
const EFFECT_GAIN := {
	"tap": 0.35, "build": 0.6, "hammer": 0.3, "done": 0.6, "discovery": 0.7, "era": 0.8,
	"deliver": 0.18, "boat": 0.45, "find": 0.55, "error": 0.4,
}
## The same effect never plays twice within this many seconds.
const MIN_GAP := {"deliver": 0.35, "hammer": 0.5, "tap": 0.06}
const VOICES := 4
const AMBIENT_LEVEL := 0.55

var muted := false
var _ambient := {}     # name -> AudioStreamPlayer
var _gain := {}        # name -> current linear gain
var _target := {}      # name -> wanted linear gain
var _effects := {}     # name -> AudioStream
var _voices: Array[AudioStreamPlayer] = []
var _last := {}        # name -> seconds when it last played
var _clock := 0.0


func _ready() -> void:
	for name in AMBIENTS:
		var stream: AudioStreamWAV = load("res://sounds/amb_%s.wav" % name)
		if stream == null:
			continue
		stream.loop_mode = AudioStreamWAV.LOOP_FORWARD
		stream.loop_begin = 0
		stream.loop_end = int(stream.get_length() * stream.mix_rate)
		var player := AudioStreamPlayer.new()
		player.stream = stream
		player.volume_db = -80.0
		add_child(player)
		player.play()
		_ambient[name] = player
		_gain[name] = 0.0
		_target[name] = 0.0
	for name in EFFECTS:
		var stream = load("res://sounds/sfx_%s.wav" % name)
		if stream != null:
			_effects[name] = stream
	for k in VOICES:
		var voice := AudioStreamPlayer.new()
		add_child(voice)
		_voices.append(voice)


## What the camera sees: `closeness` 0 (whole planet) to 1 (at the ground),
## and the share of each kind of land around the middle of the view
## (`ocean`, `forest`, `open` for meadows and beaches, `rock` for mountains,
## desert and ice, `town` for built cells).
func listen(closeness: float, mix: Dictionary) -> void:
	var z := clampf(closeness, 0.0, 1.0)
	var ocean := float(mix.get("ocean", 0.0))
	var forest := float(mix.get("forest", 0.0))
	var open := float(mix.get("open", 0.0))
	var rock := float(mix.get("rock", 0.0))
	var town := float(mix.get("town", 0.0))
	_target["space"] = 0.08 + 0.55 * (1.0 - z)
	_target["wind"] = 0.12 + z * (0.25 * open + 0.55 * rock + 0.1 * ocean)
	_target["waves"] = z * 0.75 * ocean + 0.05 * ocean
	_target["birds"] = z * (0.7 * forest + 0.2 * open)
	_target["village"] = z * 0.8 * town
	_target["crickets"] = z * 0.25 * open


func _process(delta: float) -> void:
	_clock += delta
	for name in _ambient:
		var g: float = _gain[name]
		g += (float(_target[name]) - g) * minf(1.0, delta * 1.5)
		_gain[name] = g
		var level := 0.0 if muted else g * AMBIENT_LEVEL
		(_ambient[name] as AudioStreamPlayer).volume_db = linear_to_db(maxf(level, 0.0001))


## Play an effect at `weight` (0 to 1: how near the middle of the view and
## how close the camera is; 1 for the window's own sounds).
func play(name: String, weight := 1.0) -> void:
	if muted or not _effects.has(name) or weight <= 0.02:
		return
	if _clock - float(_last.get(name, -10.0)) < float(MIN_GAP.get(name, 0.12)):
		return
	_last[name] = _clock
	var voice: AudioStreamPlayer = null
	for v in _voices:
		if not v.playing:
			voice = v
			break
	if voice == null:
		return
	voice.stream = _effects[name]
	voice.volume_db = linear_to_db(clampf(float(EFFECT_GAIN.get(name, 0.5)) * weight, 0.0001, 1.0))
	voice.pitch_scale = randf_range(0.96, 1.04)
	voice.play()
