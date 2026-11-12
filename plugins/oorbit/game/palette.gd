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

## Colours each era adds to the palette (cumulative: a later era keeps the
## earlier ones, so old buildings keep their look).
const ERA_ACCENTS := {
	4: [Color("eae2d6"), Color("8e9a5b"), Color("c07a54"), Color("cfc4b0")],
	5: [Color("6b5a4e"), Color("8c8c8c"), Color("4f5d75"), Color("b03a2e")],
	6: [Color("d4a24c"), Color("3f6e9e"), Color("c8b49a"), Color("7d9cc0")],
	7: [Color("a5532f"), Color("4a4a4f"), Color("707780"), Color("d9822b")],
	8: [Color("f5d76e"), Color("596b7d"), Color("9fb3c8"), Color("2e3440")],
	9: [Color("ff4fd8"), Color("4fe3ff"), Color("9b5cff"), Color("1b1530")],
	10: [Color("e8edf2"), Color("8fa3b8"), Color("ff9d3b"), Color("6fd3ff")],
}


## The palette the frame is reduced to in `era`.
static func for_era(era: int) -> Array[Color]:
	var colors: Array[Color] = STONE_AGE.duplicate()
	for e in range(4, era + 1):
		for c in ERA_ACCENTS.get(e, []):
			colors.append(c)
	return colors

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
