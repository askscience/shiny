## Prototype scene in Oorbit's look: a low-poly planet of 32 tiles, rendered
## at a low resolution (about 480x300), upscaled with sharp pixels and reduced
## to a soft palette. The status line (drawn at full resolution) proves the
## links to the plugin and to the host context.
extends Control

## Target height of the low-resolution view, in pixels.
const PIXEL_HEIGHT := 300.0
## Planet spin, radians per second (continuous).
const SPIN_SPEED := 0.08

var host := Host.new()
var container: SubViewportContainer
var viewport: SubViewport
var sky: ColorRect
var sky_material: ShaderMaterial
var pixel_material: ShaderMaterial
var pivot: Node3D
var status: Label
var _status_parts := {}


func _ready() -> void:
	_build_view()
	_build_world()
	_build_ui()
	resized.connect(_fit)
	_fit()
	host.context_changed.connect(_apply_context)
	_apply_context(host.context())
	if host.available:
		host.get_json("ping", _on_ping)
	else:
		_set_status("server", "offline (no LiteAura host)")


func _process(delta: float) -> void:
	pivot.rotate_object_local(Vector3.UP, SPIN_SPEED * delta)


func _build_view() -> void:
	container = SubViewportContainer.new()
	container.set_anchors_preset(Control.PRESET_FULL_RECT)
	container.stretch = true
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
	sky = ColorRect.new()
	sky.set_anchors_preset(Control.PRESET_FULL_RECT)
	sky_material = ShaderMaterial.new()
	sky_material.shader = load("res://sky.gdshader")
	sky.material = sky_material
	back.add_child(sky)


func _build_world() -> void:
	var world := Node3D.new()
	viewport.add_child(world)

	pivot = Node3D.new()
	pivot.rotation = Vector3(deg_to_rad(14.0), 0.0, deg_to_rad(-12.0))
	world.add_child(pivot)

	var biomes := _sample_biomes(7)
	var builder := PlanetMesh.new()
	var mesh := builder.build(biomes, Palette.BIOMES)

	var land := MeshInstance3D.new()
	land.mesh = mesh
	land.material_override = _toon(Color.WHITE, true)
	pivot.add_child(land)

	# Outline: the same mesh, a little bigger, drawn from the inside.
	var outline := MeshInstance3D.new()
	outline.mesh = mesh
	outline.scale = Vector3.ONE * 1.025
	var outline_material := StandardMaterial3D.new()
	outline_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	outline_material.cull_mode = BaseMaterial3D.CULL_FRONT
	outline_material.albedo_color = Palette.OUTLINE
	outline.material_override = outline_material
	pivot.add_child(outline)

	_place_props(builder, biomes)

	var sun := DirectionalLight3D.new()
	sun.light_color = Palette.SUN
	sun.light_energy = 1.15
	sun.rotation = Vector3(deg_to_rad(-30.0), deg_to_rad(-50.0), 0.0)
	world.add_child(sun)

	var camera := Camera3D.new()
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


## A stand-in for the simulation's planet until the window reads it: the
## same biome mix as a temperate climate, from a fixed seed.
func _sample_biomes(seed_value: int) -> Array:
	var rng := RandomNumberGenerator.new()
	rng.seed = seed_value
	var pool := ["coast", "coast", "plains", "plains", "plains", "forest", "forest", "forest", "mountain", "mountain", "ice", "desert"]
	var out := []
	for i in 32:
		out.append(pool[rng.randi() % pool.size()])
	return out


func _toon(color: Color, vertex_colors := false) -> StandardMaterial3D:
	var m := StandardMaterial3D.new()
	m.albedo_color = color
	m.vertex_color_use_as_albedo = vertex_colors
	m.diffuse_mode = BaseMaterial3D.DIFFUSE_TOON
	m.specular_mode = BaseMaterial3D.SPECULAR_DISABLED
	m.roughness = 1.0
	return m


## Trees on forests, peaks on mountains, a couple of huts on the plains,
## bumps on the ice. Sizes vary per prop (from a fixed seed) so the land
## reads as natural, not stamped.
func _place_props(builder: PlanetMesh, biomes: Array) -> void:
	var rng := RandomNumberGenerator.new()
	rng.seed = 11
	var tree_mesh := _cone(0.06, 0.18, 5)
	var peak_mesh := _cone(0.16, 0.26, 4)
	var snow_mesh := _cone(0.065, 0.1, 4)
	var bump_mesh := _cone(0.08, 0.06, 5)
	var wall_mesh := BoxMesh.new()
	wall_mesh.size = Vector3(0.11, 0.07, 0.11)
	var roof_mesh := _cone(0.1, 0.09, 4)

	var tree_mat := _toon(Palette.TREE)
	var peak_mat := _toon(Palette.PEAK)
	var snow_mat := _toon(Palette.SNOW)
	var wall_mat := _toon(Palette.HUT_WALL)
	var roof_mat := _toon(Palette.HUT_ROOF)
	var huts := 0
	for i in builder.centers.size():
		var center: Vector3 = builder.centers[i]
		var top: Vector3 = center * float(PlanetMesh.HEIGHT.get(biomes[i], 1.0))
		match biomes[i]:
			"forest":
				for k in 6:
					var angle := k * TAU / 6.0 + rng.randf() * 0.6
					var at: Vector3 = top + _tangent_offset(center, angle, 0.05 + rng.randf() * 0.09)
					var s := 0.7 + rng.randf() * 0.6
					_prop(tree_mesh, tree_mat, center, at, 0.09 * s, s)
			"mountain":
				for k in 2:
					var at: Vector3 = top + _tangent_offset(center, k * PI + rng.randf(), 0.035 + k * 0.05)
					var s := 1.0 - k * 0.3
					_prop(peak_mesh, peak_mat, center, at, 0.13 * s, s)
					_prop(snow_mesh, snow_mat, center, at, (0.26 - 0.05) * s, s)
			"ice":
				for k in 3:
					var at: Vector3 = top + _tangent_offset(center, k * TAU / 3.0 + rng.randf(), 0.08)
					_prop(bump_mesh, snow_mat, center, at, 0.03, 0.8 + rng.randf() * 0.4)
			"plains":
				if huts < 3:
					huts += 1
					var at: Vector3 = top + _tangent_offset(center, rng.randf() * TAU, 0.04)
					_prop(wall_mesh, wall_mat, center, at, 0.035)
					_prop(roof_mesh, roof_mat, center, at, 0.115)


static func _cone(radius: float, height: float, sides: int) -> CylinderMesh:
	var mesh := CylinderMesh.new()
	mesh.top_radius = 0.0
	mesh.bottom_radius = radius
	mesh.height = height
	mesh.radial_segments = sides
	mesh.rings = 1
	return mesh


func _tangent_offset(normal: Vector3, angle: float, distance: float) -> Vector3:
	var up := Vector3.UP if abs(normal.y) < 0.9 else Vector3.RIGHT
	var u := normal.cross(up).normalized()
	var w := normal.cross(u)
	return (u * cos(angle) + w * sin(angle)) * distance


## A prop standing on the surface at `at`, its local up along `normal`,
## lifted by `lift` along it, scaled by `scale_by`.
func _prop(mesh: Mesh, material: Material, normal: Vector3, at: Vector3, lift: float, scale_by := 1.0) -> void:
	var node := MeshInstance3D.new()
	node.mesh = mesh
	node.material_override = material
	var y := normal.normalized()
	var x := y.cross(Vector3.FORWARD if abs(y.z) < 0.9 else Vector3.RIGHT).normalized()
	var z := x.cross(y)
	node.transform = Transform3D(Basis(x, y, z).scaled(Vector3.ONE * scale_by), at + y * lift)
	pivot.add_child(node)


func _build_ui() -> void:
	var layer := CanvasLayer.new()
	add_child(layer)
	status = Label.new()
	status.position = Vector2(12, 8)
	status.add_theme_font_size_override("font_size", 12)
	status.modulate = Color(1, 1, 1, 0.75)
	layer.add_child(status)


## Keep the pixels square and the view near PIXEL_HEIGHT pixels tall.
func _fit() -> void:
	var shrink := maxi(1, roundi(size.y / PIXEL_HEIGHT))
	container.stretch_shrink = shrink
	var source := (size / shrink).floor()
	pixel_material.set_shader_parameter("source_size", source)
	sky_material.set_shader_parameter("size", source)


func _apply_context(context: Dictionary) -> void:
	var dark: bool = context.get("mode", "dark") != "light"
	# Night: deep indigo. Day (light mode): a soft dusk of lavender and peach.
	var top := Color("1e1b33") if dark else Color("8a7bb0")
	var bottom := Color("3d3361") if dark else Color("f2d6c2")
	sky_material.set_shader_parameter("top_color", Vector3(top.r, top.g, top.b))
	sky_material.set_shader_parameter("bottom_color", Vector3(bottom.r, bottom.g, bottom.b))
	status.add_theme_color_override("font_color", Color("eef3f2") if dark else Color("1e1b33"))
	_set_status("locale", str(context.get("locale", "?")))


func _on_ping(reply: Dictionary) -> void:
	if reply.get("success", false):
		var data: Dictionary = reply.get("data", {})
		_set_status("server", "ok, v%s" % data.get("version", "?"))
	else:
		_set_status("server", "error: %s" % reply.get("error", "unknown"))


func _set_status(key: String, value: String) -> void:
	_status_parts[key] = value
	var lines := PackedStringArray()
	for k in _status_parts:
		lines.append("%s: %s" % [k, _status_parts[k]])
	status.text = "\n".join(lines)
