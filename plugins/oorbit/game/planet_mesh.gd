## Builds the planet: 32 tiles on a truncated icosahedron (12 pentagons at the
## icosahedron's vertices, then 20 hexagons at its faces), the same 32 tiles
## and the same order as the simulation (`crates/oorbit-sim`, tuning.tiles).
## Each tile is a flat-shaded prism whose height and colour come from its
## biome, so the planet reads as low-poly land under the pixel filter.
class_name PlanetMesh
extends RefCounted

const BASE_RADIUS := 0.9

## Height of each biome's tile top, as a radius.
const HEIGHT := {
	"coast": 0.985,
	"plains": 1.0,
	"forest": 1.01,
	"desert": 1.0,
	"ice": 1.02,
	"mountain": 1.06,
}

## Tile centres (unit vectors) and corner rings, filled by `build`.
var centers: Array[Vector3] = []
var corners: Array[PackedVector3Array] = []


static func _icosahedron() -> Array:
	var t := (1.0 + sqrt(5.0)) / 2.0
	var v: Array[Vector3] = [
		Vector3(-1, t, 0), Vector3(1, t, 0), Vector3(-1, -t, 0), Vector3(1, -t, 0),
		Vector3(0, -1, t), Vector3(0, 1, t), Vector3(0, -1, -t), Vector3(0, 1, -t),
		Vector3(t, 0, -1), Vector3(t, 0, 1), Vector3(-t, 0, -1), Vector3(-t, 0, 1),
	]
	for i in v.size():
		v[i] = v[i].normalized()
	var f := [
		[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
		[1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
		[3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
		[4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
	]
	return [v, f]


## Compute the 32 tiles' centres and corners.
func _layout() -> void:
	var ico := _icosahedron()
	var verts: Array[Vector3] = ico[0]
	var faces: Array = ico[1]
	centers.clear()
	corners.clear()

	# Pentagons: the points a third of the way along each edge leaving a vertex.
	for i in verts.size():
		var ring := PackedVector3Array()
		var neighbours: Array[Vector3] = []
		for face in faces:
			if i in face:
				for j in face:
					if j != i and not verts[j] in neighbours:
						neighbours.append(verts[j])
		for n in neighbours:
			ring.append(verts[i].lerp(n, 1.0 / 3.0).normalized())
		centers.append(verts[i])
		corners.append(_sorted_ring(verts[i], ring))

	# Hexagons: two points on each of the face's three edges.
	for face in faces:
		var a: Vector3 = verts[face[0]]
		var b: Vector3 = verts[face[1]]
		var c: Vector3 = verts[face[2]]
		var ring := PackedVector3Array()
		for edge in [[a, b], [b, c], [c, a]]:
			ring.append(edge[0].lerp(edge[1], 1.0 / 3.0).normalized())
			ring.append(edge[0].lerp(edge[1], 2.0 / 3.0).normalized())
		var center := (a + b + c).normalized()
		centers.append(center)
		corners.append(_sorted_ring(center, ring))


## Order a ring of points counter-clockwise around `center`.
static func _sorted_ring(center: Vector3, ring: PackedVector3Array) -> PackedVector3Array:
	var up := Vector3.UP if abs(center.y) < 0.9 else Vector3.RIGHT
	var u := center.cross(up).normalized()
	var w := center.cross(u)
	var items := []
	for p in ring:
		var d := p - center
		items.append([atan2(d.dot(w), d.dot(u)), p])
	items.sort_custom(func(x, y): return x[0] < y[0])
	var out := PackedVector3Array()
	for item in items:
		out.append(item[1])
	return out


## The mesh for these biomes (one per tile, 32 of them), coloured by `colors`
## (biome -> Color). Vertex colours carry the biome; flat normals per face.
func build(biomes: Array, colors: Dictionary) -> ArrayMesh:
	_layout()
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	for i in centers.size():
		var biome: String = biomes[i] if i < biomes.size() else "plains"
		var h: float = HEIGHT.get(biome, 1.0)
		var top: Color = colors.get(biome, Color.MAGENTA)
		var side := top.darkened(0.25)
		var ring := corners[i]
		var n := ring.size()
		# Tile top: a fan around the centre, each triangle split in four so
		# the land reads as small facets rather than one flat plate.
		var c := centers[i] * surface(i, centers[i], h, biome)
		for k in n:
			var a := ring[k] * h
			var b := ring[(k + 1) % n] * h
			var ab := _bump((a + b) * 0.5, h, biome, 0.4)
			var ca := _bump((c + a) * 0.5, h, biome, 1.0)
			var bc := _bump((b + c) * 0.5, h, biome, 1.0)
			_tri(st, top, c, ca, bc)
			_tri(st, top, ca, a, ab)
			_tri(st, top, bc, ab, b)
			_tri(st, top, ca, ab, bc)
		# Tile sides, down to the base: they show wherever a neighbour is lower.
		for k in n:
			var p0 := ring[k]
			var p1 := ring[(k + 1) % n]
			_quad(st, side, p0 * h, p0 * BASE_RADIUS, p1 * BASE_RADIUS, p1 * h)
	return st.commit()


## How much a biome's ground undulates (as a share of the radius).
const ROUGHNESS := {
	"coast": 0.008,
	"plains": 0.02,
	"forest": 0.025,
	"desert": 0.028,
	"ice": 0.022,
	"mountain": 0.06,
}


## A repeatable pseudo-random value in -1..1 for a point.
static func _noise(p: Vector3) -> float:
	var v := sin(p.dot(Vector3(12.9898, 78.233, 37.719)) * 43758.5453)
	return (v - floor(v)) * 2.0 - 1.0


## A point on the tile top pushed in or out a little.
static func _bump(p: Vector3, h: float, biome: String, weight: float) -> Vector3:
	var rough: float = ROUGHNESS.get(biome, 0.01)
	return p.normalized() * h * (1.0 + _noise(p) * rough * weight)


## The radius of the tile's surface at its centre (props stand there).
static func surface(_tile: int, center: Vector3, h: float, biome: String) -> float:
	var rough: float = ROUGHNESS.get(biome, 0.01)
	return h * (1.0 + absf(_noise(center * h)) * rough)


static func _tri(st: SurfaceTool, color: Color, a: Vector3, b: Vector3, c: Vector3) -> void:
	var normal := (b - a).cross(c - a).normalized()
	if normal.dot(a) < 0.0:
		var t := b
		b = c
		c = t
		normal = -normal
	# Godot's front faces wind clockwise seen from outside.
	for p in [a, c, b]:
		st.set_color(color)
		st.set_normal(normal)
		st.add_vertex(p)


static func _quad(st: SurfaceTool, color: Color, a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> void:
	# Outward is away from the tile's own axis; orient both halves the same way.
	var mid := (a + b + c + d) / 4.0
	var axis := (a + d).normalized()
	var outward := mid - axis * mid.dot(axis)
	for tri in [[a, b, c], [a, c, d]]:
		var p: Vector3 = tri[0]
		var q: Vector3 = tri[1]
		var r: Vector3 = tri[2]
		var normal := (q - p).cross(r - p).normalized()
		if normal.dot(outward) < 0.0:
			var t := q
			q = r
			r = t
			normal = -normal
		for v in [p, r, q]:
			st.set_color(color)
			st.set_normal(normal)
			st.add_vertex(v)
