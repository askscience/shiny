// Clouds and storm puffs: toon-lit like the rest of the planet, and
// stippled by their (instance) alpha with a 4x4 ordered dither, so a cloud
// shows what lies under it and fades in and out pixel by pixel. Works in the
// Compatibility renderer, where alpha hash does not.
shader_type spatial;
render_mode diffuse_toon, specular_disabled, cull_back;

varying vec4 tint;

float bayer4(vec2 p) {
	int x = int(mod(p.x, 4.0));
	int y = int(mod(p.y, 4.0));
	int index = x + y * 4;
	int m[16] = int[16](0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5);
	return (float(m[index]) + 0.5) / 16.0;
}

void vertex() {
	tint = COLOR;
}

void fragment() {
	if (tint.a < bayer4(floor(FRAGCOORD.xy))) {
		discard;
	}
	ALBEDO = tint.rgb;
	ROUGHNESS = 1.0;
}
