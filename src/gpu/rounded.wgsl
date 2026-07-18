struct Globals {
    size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> globals: Globals;

struct VertexInput {
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) border: vec4<f32>,
    @location(5) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) @interpolate(flat) clip: vec4<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) border: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // Soft-edged rectangles (drop shadows) expand the quad by their falloff
    // half-width so the outer half of the smoothstep is not cut off.
    let softness = max(input.params.w, 0.0);
    let expanded = input.rect.zw + vec2<f32>(softness * 2.0);
    let pixel = input.rect.xy - vec2<f32>(softness) + input.corner * expanded;
    let normalized = vec2<f32>(
        pixel.x / globals.size.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(normalized, 0.0, 1.0);
    output.local = (input.corner - vec2<f32>(0.5)) * expanded;
    output.half_size = input.rect.zw * 0.5;
    output.clip = input.clip;
    output.fill = input.fill;
    output.border = input.border;
    output.params = input.params;
    return output;
}

fn rounded_distance(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let bounded_radius = min(radius, min(half_size.x, half_size.y));
    let delta = abs(point) - (half_size - vec2<f32>(bounded_radius));
    return length(max(delta, vec2<f32>(0.0))) + min(max(delta.x, delta.y), 0.0) - bounded_radius;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = input.position.xy;
    if pixel.x < input.clip.x || pixel.y < input.clip.y ||
       pixel.x > input.clip.z || pixel.y > input.clip.w {
        discard;
    }
    let distance = rounded_distance(input.local, input.half_size, input.params.x);
    let falloff = max(input.params.w, 0.75);
    let coverage = 1.0 - smoothstep(-falloff, falloff, distance);
    if coverage <= 0.0 {
        discard;
    }
    let border_mix = select(0.0, 1.0, input.params.y > 0.0 && distance > -input.params.y);
    let color = mix(input.fill, input.border, border_mix);
    let alpha = color.a * coverage;
    return vec4<f32>(color.rgb, alpha);
}
