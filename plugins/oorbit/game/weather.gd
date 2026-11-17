## The planet's weather, drawn with particles: banks of cloud puffs that
## drift around the globe, form, thicken and melt away (a few grey ones
## drizzle), and, when the planet has a storm or a lightning event, a dark
## thunderhead that rolls in towards the threatened cell, churning, raining
## and throwing lightning, and moves on once the event is over.
##
## Everything stays low over the ground and shrinks away near the planet's
## rim, so no cloud ever sticks out of the planet's outline. Fair-weather
## clouds belong to the far view and thin out as the camera comes down; the
## storm stays. All the particles live in MultiMeshes of fixed size
## (see `PlanetView._fixed`).
class_name Weather
extends Node3D

signal thunder(where: Vector3, weight: float)

const BANKS := 16
const PUFFS_PER_BANK := 18
const STORM_PUFFS := 280
const MAX_RAIN := 180
const BOLT_SEGMENTS := 32
## Height of the cloud base over the ground (or the sea), world units.
const LIFT := 0.035
const STORM_LIFT := 0.05
## How far the storm forms from its target (radians on the globe), and how
## fast it rolls in.
const STORM_START := 0.5
const STORM_SPEED := 0.014
## The storm's size (radius of its base, world units).
const STORM_RADIUS := 0.13

const WHITE := Color("eef3f2")
const GREY := Color("b9b4c4")
const STORM_LOW := Color("3b3548")
const STORM_HIGH := Color("6f6680")
const FLASH := Color("e8e2ff")
const RAIN := Color("a8bccf")
const BOLT := Color("fff6c8")

var builder: PlanetMesh
var _clouds: MultiMeshInstance3D
var _thunderhead: MultiMeshInstance3D
var _rain: MultiMeshInstance3D
var _bolt: MultiMeshInstance3D
var _banks := []
var _storm := {}       # dir, target, key, power, leaving, puffs, next_bolt, bolt
var _drops := []
var _thunders := []    # [seconds left, where, weight]
var _camera := Vector3(0, 0, 4.4)
var _closeness := 0.0
var _time := 0.0
var _rng := RandomNumberGenerator.new()


func setup(planet_builder: PlanetMesh) -> void:
	builder = planet_builder
	_rng.seed = 7
	var puff := SphereMesh.new()
	puff.radius = 1.0
	puff.height = 2.0
	puff.radial_segments = 7
	puff.rings = 4
	# Clouds are stippled rather than solid (see cloud.gdshader), so what
	# lies under them still shows through.
	var stipple := ShaderMaterial.new()
	stipple.shader = load("res://cloud.gdshader")
	_clouds = _multi(puff, BANKS * PUFFS_PER_BANK, stipple)
	_thunderhead = _multi(puff, STORM_PUFFS, stipple)
	var cube := BoxMesh.new()
	cube.size = Vector3.ONE
	var glow := StandardMaterial3D.new()
	glow.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	glow.vertex_color_use_as_albedo = true
	_rain = _multi(cube, MAX_RAIN, glow)
	_bolt = _multi(cube, BOLT_SEGMENTS, glow)
	for k in BANKS:
		_banks.append(_new_bank(true))
	for k in MAX_RAIN:
		_drops.append({"source": -2, "p": Vector3.ZERO, "v": Vector3.ZERO, "ground": 0.0, "splash": 0.0})


func _multi(mesh: Mesh, count: int, material: Material) -> MultiMeshInstance3D:
	var node := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.mesh = mesh
	PlanetView._fixed(mm, count)
	node.multimesh = mm
	node.material_override = material
	add_child(node)
	return node


## Where the camera is (in the planet's frame) and how close it has come
## (0 whole planet, 1 at the ground).
func look(camera_local: Vector3, closeness: float) -> void:
	_camera = camera_local
	_closeness = closeness


## The planet's storm: the direction of the threatened cell, or
## `Vector3.ZERO` for none. `key` tells one storm from the next.
func set_storm(target: Vector3, key: String) -> void:
	if target == Vector3.ZERO:
		if not _storm.is_empty():
			_storm["leaving"] = true
		return
	if not _storm.is_empty() and _storm["key"] == key:
		return
	var t := target.normalized()
	var side := t.cross(Vector3(_rng.randf_range(-1, 1), _rng.randf_range(-1, 1), _rng.randf_range(-1, 1))).normalized()
	if side.length_squared() < 0.5:
		side = PlanetView._jitter(1.0).cross(t).normalized()
	var puffs := []
	# Lobes: the cloud's edge bulges out here and there, never a disc.
	var lobes := [_rng.randf() * TAU, _rng.randf() * TAU, _rng.randf_range(0.15, 0.3)]
	for k in STORM_PUFFS:
		# A wide dark base, a tower rising in the middle and an anvil
		# spreading out on top.
		var layer := _rng.randf()
		var r: float
		var h: float
		if layer < 0.5:
			r = sqrt(_rng.randf()) * STORM_RADIUS
			h = _rng.randf() * 0.012
		elif layer < 0.8:
			r = sqrt(_rng.randf()) * STORM_RADIUS * 0.45
			h = 0.01 + _rng.randf() * 0.045
		else:
			r = sqrt(_rng.randf()) * STORM_RADIUS * 0.85
			h = 0.05 + _rng.randf() * 0.012
		var a := _rng.randf() * TAU
		r *= 1.0 - float(lobes[2]) + float(lobes[2]) * (0.6 * sin(3.0 * a + float(lobes[0])) + 0.4 * sin(5.0 * a + float(lobes[1])) + 1.0) * 0.5 * 2.0
		puffs.append({"a": a, "r": r, "h": h, "s": _rng.randf_range(0.007, 0.015) * (1.2 if layer < 0.5 else 1.0), "phase": _rng.randf() * TAU})
	_storm = {
		"key": key, "target": t, "dir": (t * cos(STORM_START) + side * sin(STORM_START)).normalized(),
		"power": 0.0, "leaving": false, "puffs": puffs, "next_bolt": 4.0, "bolt": [], "bolt_t": 9.0,
		"flash": 0.0, "height": 1.0, "height_t": 0.0, "wind": Vector3.ZERO,
	}


## Where the storm is (planet frame) and how strong (0 to 1), or an empty
## array when the sky is calm.
func storm() -> Array:
	if _storm.is_empty() or float(_storm["power"]) < 0.05:
		return []
	return [_storm["dir"] * float(_storm["height"]), float(_storm["power"])]


func _process(delta: float) -> void:
	if builder == null or builder.centers.is_empty():
		return
	_time += delta
	_clouds.multimesh.visible_instance_count = _draw_banks(delta)
	_thunderhead.multimesh.visible_instance_count = _draw_storm(delta, 0)
	_draw_rain(delta)
	_draw_bolt(delta)
	var k := 0
	while k < _thunders.size():
		_thunders[k][0] -= delta
		if _thunders[k][0] <= 0.0:
			thunder.emit(to_global(_thunders[k][1]), _thunders[k][2])
			_thunders.remove_at(k)
		else:
			k += 1


## The height of the cloud base over `dir`: the ground or the sea, and a
## little more.
func _base_height(dir: Vector3, lift: float) -> float:
	return maxf(builder.ground(dir).length(), PlanetMesh.SEA_LEVEL) + lift


## How much of a thing at `p` (planet frame) shows: 1 facing the camera,
## falling to 0 before the planet's rim in the far view, so nothing ever
## sticks out of the outline.
func _facing(p: Vector3) -> float:
	var horizon := 1.0 / maxf(_camera.length(), 1.01)
	var fade := smoothstep(horizon + 0.04, horizon + 0.22, p.normalized().dot(_camera.normalized()))
	# Up close the rim is the horizon, and a storm may tower over it.
	return lerpf(fade, 1.0, smoothstep(0.35, 0.7, _closeness))


func _new_bank(anywhere: bool) -> Dictionary:
	var dir := PlanetView._jitter(1.0)
	while dir.length_squared() < 0.05:
		dir = PlanetView._jitter(1.0)
	dir = dir.normalized()
	var axis := dir.cross(PlanetView._jitter(1.0)).normalized()
	var wet := _rng.randf() < 0.25
	var puffs := []
	var long := _rng.randf_range(0.08, 0.16)
	var wide := long * _rng.randf_range(0.4, 0.7)
	for k in PUFFS_PER_BANK:
		var u := _rng.randf_range(-1.0, 1.0)
		var w := _rng.randf_range(-1.0, 1.0) * sqrt(1.0 - u * u * 0.8)
		var middle := 1.0 - absf(u) * 0.6
		puffs.append({"u": u * long, "w": w * wide, "h": _rng.randf() * 0.018 * middle,
			"s": _rng.randf_range(0.012, 0.024) * (0.6 + 0.6 * middle), "phase": _rng.randf() * TAU})
	return {
		"dir": dir, "axis": axis, "speed": _rng.randf_range(0.006, 0.016),
		"age": _rng.randf_range(0.0, 80.0) if anywhere else 0.0, "life": _rng.randf_range(70.0, 150.0),
		"wet": wet, "puffs": puffs, "height": 1.04, "height_t": _rng.randf() * 0.5,
	}


func _draw_banks(delta: float) -> int:
	var mm := _clouds.multimesh
	var i := 0
	# Fair weather is a far view: it melts away as the camera comes down.
	var near := 1.0 - smoothstep(0.2, 0.55, _closeness)
	# The sky's cover comes and goes slowly.
	var cover := 0.55 + 0.45 * sin(_time * 0.013 + 1.0) * sin(_time * 0.007)
	for b in _banks.size():
		var bank: Dictionary = _banks[b]
		bank["age"] = float(bank["age"]) + delta
		if float(bank["age"]) > float(bank["life"]):
			bank = _new_bank(false)
			_banks[b] = bank
		var dir: Vector3 = bank["dir"]
		dir = dir.rotated(bank["axis"], float(bank["speed"]) * delta).normalized()
		bank["dir"] = dir
		bank["height_t"] = float(bank["height_t"]) - delta
		if float(bank["height_t"]) <= 0.0:
			bank["height_t"] = 0.5
			bank["height"] = _base_height(dir, LIFT)
		var f := float(bank["age"]) / float(bank["life"])
		var grow := smoothstep(0.0, 0.2, f) * (1.0 - smoothstep(0.75, 1.0, f))
		# Banks beyond the sky's cover stay away for now.
		if float(b) / BANKS > cover:
			grow = 0.0
		var amount := grow * near
		if amount < 0.02:
			continue
		var t := PlanetView._tangents_of(dir)
		var along: Vector3 = (bank["axis"] as Vector3).cross(dir).normalized()
		var across := dir.cross(along)
		var centre := dir * float(bank["height"])
		var tone := GREY if bank["wet"] else WHITE
		for puff in bank["puffs"]:
			var p: Vector3 = centre + along * float(puff["u"]) + across * float(puff["w"]) + dir * float(puff["h"])
			var facing := _facing(p)
			var show := amount * facing
			if show < 0.03:
				continue
			var s := float(puff["s"]) * grow * sqrt(facing) * (1.0 + 0.08 * sin(_time * 0.7 + float(puff["phase"])))
			var basis := Basis(t[0], dir, t[1]).scaled(Vector3(s * 1.3, s * 0.75, s))
			mm.set_instance_transform(i, _sane(Transform3D(basis, p), "242"))
			var c := tone.darkened(0.12 * (1.0 - float(puff["h"]) / 0.018))
			var edge := Vector2(float(puff["u"]), float(puff["w"])).length() / 0.16
			c.a = show * (0.95 - 0.5 * edge * edge)
			mm.set_instance_color(i, c)
			i += 1
	return i


func _draw_storm(delta: float, i: int) -> int:
	if _storm.is_empty():
		return i
	var leaving: bool = _storm["leaving"]
	var power := float(_storm["power"])
	power = maxf(0.0, power - delta / 12.0) if leaving else minf(1.0, power + delta / 6.0)
	_storm["power"] = power
	if leaving and power <= 0.0:
		_storm = {}
		return i
	# Roll in towards the target, slowing down as it arrives, then hang
	# over it and wander a little; once over, roll on and away.
	var dir: Vector3 = _storm["dir"]
	var target: Vector3 = _storm["target"]
	var gap := dir.angle_to(target)
	var before := dir
	if leaving:
		var axis := (_storm["wind"] as Vector3)
		if axis.length_squared() > 0.5:
			dir = dir.rotated(axis, STORM_SPEED * 1.5 * delta)
	elif gap > 0.04:
		var axis := dir.cross(target).normalized()
		dir = dir.rotated(axis, minf(gap - 0.03, STORM_SPEED * clampf(gap / 0.15, 0.35, 1.0) * delta))
		_storm["wind"] = axis
	else:
		var t := PlanetView._tangents_of(target)
		var wander: Vector3 = target + (t[0] * sin(_time * 0.05) + t[1] * sin(_time * 0.037)) * 0.03
		dir = dir.slerp(wander.normalized(), minf(1.0, delta * 0.2))
	dir = dir.normalized()
	_storm["dir"] = dir
	var motion := (dir - before) / maxf(delta, 1e-4)
	_storm["height_t"] = float(_storm["height_t"]) - delta
	if float(_storm["height_t"]) <= 0.0:
		_storm["height_t"] = 0.4
		_storm["height"] = _base_height(dir, STORM_LIFT)
	var base := float(_storm["height"])
	_storm["motion"] = motion
	# Lightning: a flash in the cloud, a bolt to the ground, thunder a
	# moment later.
	_storm["next_bolt"] = float(_storm["next_bolt"]) - delta
	if power > 0.5 and float(_storm["next_bolt"]) <= 0.0:
		_storm["next_bolt"] = _rng.randf_range(1.8, 5.0) / power
		_strike(dir, target, base)
	_storm["flash"] = maxf(0.0, float(_storm["flash"]) - delta)
	var bolt_t := float(_storm["bolt_t"])
	var lit := bolt_t < 0.06 or (bolt_t > 0.1 and bolt_t < 0.22) or float(_storm["flash"]) > 0.0
	var mm := _thunderhead.multimesh
	var t := PlanetView._tangents_of(dir)
	var spin := 0.04
	for puff in _storm["puffs"]:
		# The cloud churns: puffs swirl around its middle, faster inside.
		puff["a"] = float(puff["a"]) + delta * spin / (float(puff["r"]) * 8.0 + 0.4)
		var a := float(puff["a"])
		var r := float(puff["r"]) * (0.7 + 0.3 * power)
		var p: Vector3 = dir * (base + float(puff["h"]) * power) + (t[0] * cos(a) + t[1] * sin(a)) * r
		var facing := _facing(p)
		var show := power * facing
		var s := float(puff["s"]) * (0.5 + 0.5 * power) * sqrt(facing) * (1.0 + 0.12 * sin(_time * 1.3 + float(puff["phase"])))
		var up := p.normalized()
		var tp := PlanetView._tangents_of(up)
		mm.set_instance_transform(i, _sane(Transform3D(Basis(tp[0], up, tp[1]).scaled(Vector3(s * 1.2, s * 0.85, s)), p), "311"))
		var height := clampf(float(puff["h"]) / 0.06, 0.0, 1.0)
		var c := STORM_LOW.lerp(STORM_HIGH, height)
		if lit:
			c = c.lerp(FLASH, 0.6 - 0.4 * height)
		# Thick and dark at the base, thinner up in the anvil.
		# Soft, ragged edges.
		var edge := clampf(float(puff["r"]) / STORM_RADIUS, 0.0, 1.0)
		c.a = show * (0.95 - 0.3 * height) * (1.0 - 0.55 * edge * edge)
		mm.set_instance_color(i, c)
		i += 1
	return i


## A bolt from the cloud's base to the ground near the target: a jagged
## line with a branch, lit for a fraction of a second, then thunder.
func _strike(dir: Vector3, target: Vector3, base: float) -> void:
	var t := PlanetView._tangents_of(dir)
	var a := _rng.randf() * TAU
	var top: Vector3 = dir * base + (t[0] * cos(a) + t[1] * sin(a)) * _rng.randf() * STORM_RADIUS * 0.5
	var spot := target
	if dir.angle_to(target) > 0.12:
		spot = dir
	var tt := PlanetView._tangents_of(spot)
	var b := _rng.randf() * TAU
	var ground := builder.ground((spot + (tt[0] * cos(b) + tt[1] * sin(b)) * _rng.randf() * 0.08).normalized())
	var points := [top]
	var n := 12
	for k in range(1, n):
		var f := float(k) / n
		points.append(top.lerp(ground, f) + PlanetView._jitter(0.012) * (1.0 - f * 0.5))
	points.append(ground)
	var segments := []
	for k in n:
		segments.append([points[k], points[k + 1], 0.0035])
	# A branch from somewhere in the upper half.
	var from: Vector3 = points[_rng.randi_range(2, 5)]
	var go := (ground - top).normalized().slerp(PlanetView._jitter(1.0).normalized(), 0.5) * 0.018
	for k in 5:
		var to := from + go + PlanetView._jitter(0.006)
		segments.append([from, to, 0.002])
		from = to
	_storm["bolt"] = segments
	_storm["bolt_t"] = 0.0
	_storm["flash"] = 0.0
	var weight := float(_storm["power"])
	_thunders.append([_rng.randf_range(0.25, 0.9), ground, weight])


func _draw_bolt(delta: float) -> void:
	var mm := _bolt.multimesh
	if _storm.is_empty():
		mm.visible_instance_count = 0
		return
	_storm["bolt_t"] = float(_storm["bolt_t"]) + delta
	var t := float(_storm["bolt_t"])
	var on := t < 0.06 or (t > 0.1 and t < 0.24)
	if not on:
		mm.visible_instance_count = 0
		return
	var i := 0
	for seg in _storm["bolt"]:
		if i >= BOLT_SEGMENTS:
			break
		mm.set_instance_transform(i, _sane(_stretched((seg[0] + seg[1]) / 2.0, seg[1] - seg[0], seg[2]), "371"))
		mm.set_instance_color(i, BOLT)
		i += 1
	mm.visible_instance_count = i


## Rain: drops fall from the storm's base (and now and then from a grey
## bank) to the ground and leave a tiny splash.
func _draw_rain(delta: float) -> void:
	var mm := _rain.multimesh
	var storm_on := not _storm.is_empty() and float(_storm["power"]) > 0.2
	var wet := []
	if _closeness > 0.15:
		# Only banks on this side of the planet rain: drops nobody can see
		# are not worth a thought.
		var toward := _camera.normalized()
		var horizon := 1.0 / maxf(_camera.length(), 1.01) - 0.1
		for b in _banks.size():
			var bank: Dictionary = _banks[b]
			var f := float(bank["age"]) / float(bank["life"])
			if bank["wet"] and f > 0.25 and f < 0.75 and (bank["dir"] as Vector3).dot(toward) > horizon:
				wet.append(b)
	var i := 0
	for drop in _drops:
		var source := int(drop["source"])
		var p: Vector3 = drop["p"]
		if float(drop["splash"]) > 0.0:
			drop["splash"] = float(drop["splash"]) - delta
			if i < MAX_RAIN:
				var up := p.normalized()
				var tg := PlanetView._tangents_of(up)
				var s := 0.006 * (1.0 - float(drop["splash"]) / 0.12)
				mm.set_instance_transform(i, _sane(Transform3D(Basis(tg[0], up, tg[1]).scaled(Vector3(s, 0.001, s)), p), "399"))
				mm.set_instance_color(i, RAIN.lightened(0.3))
				i += 1
			if float(drop["splash"]) <= 0.0:
				drop["source"] = -2
			continue
		if source == -2:
			# A new drop: from the storm mostly, else from a wet bank.
			if storm_on and _rng.randf() < 0.85:
				drop["source"] = -1
			elif not wet.is_empty() and _rng.randf() < 0.3:
				drop["source"] = wet[_rng.randi() % wet.size()]
			else:
				continue
			source = int(drop["source"])
			var dir: Vector3
			var spread: float
			var height: float
			if source == -1:
				dir = _storm["dir"]
				spread = STORM_RADIUS * 0.85 * float(_storm["power"])
				height = float(_storm["height"])
			else:
				dir = _banks[source]["dir"]
				spread = 0.07
				height = float(_banks[source]["height"])
			var tg := PlanetView._tangents_of(dir)
			var a := _rng.randf() * TAU
			var start: Vector3 = (dir * height + (tg[0] * cos(a) + tg[1] * sin(a)) * sqrt(_rng.randf()) * spread)
			drop["p"] = start - start.normalized() * _rng.randf() * 0.02
			drop["ground"] = maxf(builder.ground(start).length(), PlanetMesh.SEA_LEVEL)
			var wind: Vector3 = _storm.get("motion", Vector3.ZERO) if source == -1 else Vector3.ZERO
			drop["v"] = -start.normalized() * _rng.randf_range(0.28, 0.36) + wind * 2.0
			p = drop["p"]
		var v: Vector3 = drop["v"]
		p += v * delta
		drop["p"] = p
		if p.length() <= float(drop["ground"]):
			drop["splash"] = 0.12
			drop["p"] = p.normalized() * float(drop["ground"])
			continue
		if i >= MAX_RAIN:
			continue
		var show := _facing(p)
		if show < 0.05:
			continue
		mm.set_instance_transform(i, _sane(_stretched(p, v.normalized() * 0.016, 0.0018), "445"))
		mm.set_instance_color(i, RAIN)
		i += 1
	mm.visible_instance_count = i


## A cube stretched along `dir` (its length), `width` across.
static func _stretched(centre: Vector3, dir: Vector3, width: float) -> Transform3D:
	var length := dir.length()
	var z := dir / maxf(length, 1e-6)
	var x := centre.normalized().cross(z)
	if x.length_squared() < 1e-8:
		x = PlanetView._tangents_of(z)[0]
	x = x.normalized()
	var y := z.cross(x)
	return Transform3D(Basis(x * width, y * width, z * maxf(length, 1e-6)), centre)


var _warned := {}


## Never draw a particle off the planet or blown up (warn once per place).
func _sane(t: Transform3D, where: String) -> Transform3D:
	var sc := t.basis.get_scale()
	if t.origin.is_finite() and t.basis.is_finite() and t.origin.length() < 1.3 and sc.x < 0.15 and sc.y < 0.15 and sc.z < 0.15:
		return t
	if not _warned.has(where):
		_warned[where] = true
		push_warning("oorbit weather: particle out of shape (line %s): %s" % [where, t])
	return Transform3D(Basis().scaled(Vector3.ZERO), Vector3.ZERO)
