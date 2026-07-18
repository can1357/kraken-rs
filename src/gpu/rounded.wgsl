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
    let pixel = input.rect.xy + input.corner * input.rect.zw;
    let normalized = vec2<f32>(
        pixel.x / globals.size.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(normalized, 0.0, 1.0);
    output.local = (input.corner - vec2<f32>(0.5)) * input.rect.zw;
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
    let coverage = 1.0 - smoothstep(-0.75, 0.75, distance);
    if coverage <= 0.0 {
        discard;
    }
    let border_mix = select(0.0, 1.0, input.params.y > 0.0 && distance > -input.params.y);
    let color = mix(input.fill, input.border, border_mix);
    var alpha = color.a * coverage;
    let mode = input.params.z;
    if mode > 0.5 {
        // Brand checker: 1px cells alternating inside a 2px tile, screen-anchored.
        let cell = floor(pixel);
        let on = (cell.x + cell.y) - 2.0 * floor((cell.x + cell.y) * 0.5);
        if on < 0.5 {
            discard;
        }
        if mode > 1.5 {
            // Field: rise from the bottom edge, fading out toward the top.
            let frac = (input.local.y + input.half_size.y) / max(input.half_size.y * 2.0, 1.0);
            alpha = alpha * clamp(frac * frac, 0.0, 1.0);
        }
    }
    return vec4<f32>(color.rgb, alpha);
}
