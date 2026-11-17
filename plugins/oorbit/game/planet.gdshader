// The ground and the props: toon-lit vertex colours, plus the time of day
// and the season (set by PlanetView.set_sky every frame).
//
// - Night: the side of the planet away from the sun is lit only by the
//   ambient light, and lights (vertices with alpha below 0.75: windows,
//   fires, lanterns) glow there.
// - Seasons, on "leafy" colours (green with more red than blue: leaves,
//   grass, meadows, but not the bluish firs): blossom in spring, gold and
//   red in autumn, bare and brown in winter.
// - Snow on what faces up, from the poles down as far as `snow` reaches.
shader_type spatial;
render_mode diffuse_toon, specular_disabled, cull_back;

uniform vec3 sun_dir = vec3(0.0, 0.0, 1.0);
uniform vec3 axis = vec3(0.0, 1.0, 0.0);
uniform vec3 centre = vec3(0.0);
uniform float snow = 0.0;
uniform float autumn = 0.0;
uniform float bare = 0.0;
uniform float bloom = 0.0;
// How much the season shows on this mesh (props 1, the ground less).
uniform float leaf_strength = 1.0;
uniform float bloom_strength = 1.0;
// The ground: its meadows turn ochre in autumn rather than gold and red.
uniform bool ground = false;

varying vec3 radial;
varying vec3 world_normal;
varying vec3 world_pos;

float hash(vec3 p) {
	return fract(sin(dot(floor(p), vec3(12.9898, 78.233, 37.719))) * 43758.5453);
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	radial = normalize(world_pos - centre);
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

void fragment() {
	vec3 c = COLOR.rgb;
	float leafy = step(c.r * 1.05, c.g) * step(c.b, c.r) * leaf_strength;
	// A patch of land (about a tree) shares its season's colour.
	float h = ground ? 0.5 : hash(world_pos * 60.0);
	if (bloom > 0.0) {
		vec3 blossom = h > 0.5 ? vec3(0.89, 0.76, 0.78) : vec3(0.95, 0.94, 0.9);
		c = mix(c, blossom, leafy * bloom_strength * bloom * step(0.62, h));
	}
	if (autumn > 0.0) {
		vec3 gold = h < 0.4 ? vec3(0.88, 0.56, 0.42) : (h < 0.75 ? vec3(0.95, 0.75, 0.47) : vec3(0.64, 0.36, 0.36));
		if (ground) {
			gold = vec3(0.72, 0.69, 0.47);
		}
		c = mix(c, gold, leafy * autumn * 0.85);
	}
	if (bare > 0.0) {
		c = mix(c, ground ? vec3(0.62, 0.64, 0.5) : vec3(0.56, 0.5, 0.44), leafy * bare * 0.7);
	}
	if (snow > 0.0) {
		float lat = abs(dot(radial, axis));
		float up = dot(world_normal, radial);
		float line = 1.0 - snow + (h - 0.5) * 0.08;
		float cover = smoothstep(line, line + 0.06, lat) * smoothstep(0.3, 0.6, up);
		c = mix(c, vec3(0.93, 0.95, 0.95), cover);
	}
	ALBEDO = c;
	ROUGHNESS = 1.0;
	if (COLOR.a < 0.75) {
		float night = smoothstep(0.12, -0.18, dot(radial, sun_dir));
		EMISSION = c * night * 0.8;
	}
}
