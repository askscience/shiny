// The background: a soft vertical gradient with a few slow-twinkling stars,
// drawn at the low resolution so the stars are single pixels.
shader_type canvas_item;

uniform vec3 top_color = vec3(0.12, 0.11, 0.2);
uniform vec3 bottom_color = vec3(0.24, 0.2, 0.38);
uniform vec2 size = vec2(480.0, 300.0);
uniform float star_amount = 0.0025;

float hash(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void fragment() {
	vec3 c = mix(top_color, bottom_color, UV.y);
	vec2 cell = floor(UV * size);
	float h = hash(cell);
	if (h < star_amount) {
		float twinkle = 0.6 + 0.4 * sin(TIME * (0.5 + hash(cell + 3.0)) + h * 600.0);
		c = mix(c, vec3(0.93, 0.95, 0.95), twinkle);
	}
	COLOR = vec4(c, 1.0);
}
