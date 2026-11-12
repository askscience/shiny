## Oorbit's colours: one soft palette the whole frame is reduced to (see
## pixel.gdshader), and the biome colours drawn from it. Later eras will bring
## their own palettes; this is the Stone Age's.
class_name Palette
extends RefCounted

const STONE_AGE: Array[Color] = [
	# night sky and outline
	Color("15132a"), Color("1e1b33"), Color("2b2547"), Color("3d3361"), Color("564a7d"),
	# dusk sky (light mode)
	Color("8a7bb0"), Color("b9a6d6"), Color("e3c3c6"), Color("f2d6c2"),
	# plains and forest
	Color("2f4a3f"), Color("46695a"), Color("4f6b4a"), Color("6f9163"), Color("94b37e"), Color("c2d69b"),
	# stone
	Color("5b5466"), Color("857c8a"), Color("b0a6ad"), Color("d9d0cc"),
	# sand
	Color("8f6f5a"), Color("b58e6b"), Color("d9b48f"), Color("f0d9b5"),
	# water
	Color("3b5f73"), Color("4a7a8c"), Color("6fa3a8"), Color("9ccfc4"),
	# ice
	Color("a9c3d4"), Color("c9dde8"), Color("eef3f2"),
	# warm accents: huts, fire, light
	Color("7a4f4f"), Color("a35d5d"), Color("e08f6c"), Color("f2c078"),
]

const BIOMES := {
	"plains": Color("94b37e"),
	"forest": Color("46695a"),
	"mountain": Color("857c8a"),
	"desert": Color("d9b48f"),
	"coast": Color("6fa3a8"),
	"ice": Color("eef3f2"),
}

const OUTLINE := Color("15132a")
const TREE := Color("2f4a3f")
const PEAK := Color("b0a6ad")
const SNOW := Color("eef3f2")
const HUT_WALL := Color("b58e6b")
const HUT_ROOF := Color("a35d5d")
const SUN := Color("f2e1c8")
const AMBIENT := Color("6a5f8f")
