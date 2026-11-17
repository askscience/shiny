// People, animals, birds and vehicles (MultiMesh instances of the Blender
// models, see models.gd). Toon-lit like the rest of the planet.
//
// Each instance's custom data is (tint r, g, b, motion): vertices with alpha
// 0 (a model's tint materials) take the tint, the others keep their colour.
// Parts named anim_* in Blender carry their pivot (UV) and swing (UV2:
// amplitude, axis 0 x, 1 y, 2 z) and swing back and forth as fast as
// `speed`, scaled by the instance's motion (0 standing, 1 walking): legs,
// arms and tails move on the GPU, at no cost to the scripts.
shader_type spatial;
render_mode diffuse_toon, specular_disabled, cull_back;

uniform float speed = 9.0;
varying vec3 tint;

vec2 turn(vec2 p, vec2 pivot, float c, float s) {
	vec2 q = p - pivot;
	return pivot + vec2(q.x * c - q.y * s, q.x * s + q.y * c);
}

void vertex() {
	float motion = INSTANCE_CUSTOM.a;
	float id = float(INSTANCE_ID);
	float phase = TIME * speed * (0.85 + 0.3 * fract(id * 0.618)) + id * 1.7;
	float a = UV2.x * sin(phase) * motion;
	if (a != 0.0) {
		float c = cos(a);
		float s = sin(a);
		int axis = int(UV2.y + 0.5);
		if (axis == 0) {
			VERTEX.yz = turn(VERTEX.yz, UV, c, s);
			NORMAL.yz = turn(NORMAL.yz, vec2(0.0), c, s);
		} else if (axis == 1) {
			VERTEX.xz = turn(VERTEX.xz, UV, c, s);
			NORMAL.xz = turn(NORMAL.xz, vec2(0.0), c, s);
		} else {
			VERTEX.xy = turn(VERTEX.xy, UV, c, s);
			NORMAL.xy = turn(NORMAL.xy, vec2(0.0), c, s);
		}
	}
	tint = COLOR.a < 0.5 ? INSTANCE_CUSTOM.rgb : COLOR.rgb;
}

void fragment() {
	ALBEDO = tint;
	ROUGHNESS = 1.0;
}
