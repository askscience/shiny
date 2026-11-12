// Reduces the low-resolution frame to the palette, with a light ordered
// dither so soft gradients survive as pixel art. Applied to the
// SubViewportContainer that upscales the 3D view with nearest filtering.
shader_type canvas_item;

const int MAX_COLORS = 40;
uniform vec3 palette[MAX_COLORS];
uniform int palette_size = 1;
uniform vec2 source_size = vec2(480.0, 300.0);
uniform float dither = 0.035;

float bayer4(vec2 p) {
	int x = int(mod(p.x, 4.0));
	int y = int(mod(p.y, 4.0));
	int index = x + y * 4;
	int m[16] = int[16](0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5);
	return float(m[index]) / 16.0 - 0.5;
}

void fragment() {
	vec4 source = texture(TEXTURE, UV);
	vec2 pixel = floor(UV * source_size);
	vec3 c = source.rgb + vec3(bayer4(pixel) * dither);
	vec3 best = palette[0];
	float best_d = 1e9;
	for (int i = 0; i < MAX_COLORS; i++) {
		if (i >= palette_size) {
			break;
		}
		vec3 d = c - palette[i];
		// Weighted distance: the eye is most sensitive to green.
		float dist = dot(d * d, vec3(0.3, 0.59, 0.11));
		if (dist < best_d) {
			best_d = dist;
			best = palette[i];
		}
	}
	COLOR = vec4(best, 1.0);
}
