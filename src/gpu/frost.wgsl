struct Globals {
    size: vec2<f32>,
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;

struct RectInput {
    @location(0) corner: vec2<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) border: vec4<f32>,
    @location(5) params: vec4<f32>,
}

struct RectOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) border: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[vertex_index], 0.0, 1.0);
}

@vertex
fn vs_frost(input: RectInput) -> RectOutput {
    let pixel = input.rect.xy + input.corner * input.rect.zw;
    let normalized = vec2(pixel.x / globals.size.x * 2.0 - 1.0, 1.0 - pixel.y / globals.size.y * 2.0);
    var output: RectOutput;
    output.position = vec4(normalized, 0.0, 1.0);
    output.local = (input.corner - vec2(0.5)) * input.rect.zw;
    output.half_size = input.rect.zw * 0.5;
    output.clip = input.clip;
    output.fill = input.fill;
    output.border = input.border;
    output.params = input.params;
    return output;
}

fn rounded_distance(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let bounded_radius = min(radius, min(half_size.x, half_size.y));
    let delta = abs(point) - (half_size - vec2(bounded_radius));
    return length(max(delta, vec2(0.0))) + min(max(delta.x, delta.y), 0.0) - bounded_radius;
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, position.xy / globals.size);
}

@fragment
fn fs_downsample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    let uv = position.xy / globals.size;
    return (textureSample(source, source_sampler, uv + texel * vec2(-0.5, -0.5)) +
            textureSample(source, source_sampler, uv + texel * vec2(0.5, -0.5)) +
            textureSample(source, source_sampler, uv + texel * vec2(-0.5, 0.5)) +
            textureSample(source, source_sampler, uv + texel * vec2(0.5, 0.5))) * 0.25;
}

fn gaussian(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    var color = textureSample(source, source_sampler, uv) * 0.227027;
    color += textureSample(source, source_sampler, uv + direction * texel * 1.384615) * 0.316216;
    color += textureSample(source, source_sampler, uv - direction * texel * 1.384615) * 0.316216;
    color += textureSample(source, source_sampler, uv + direction * texel * 3.230769) * 0.070270;
    color += textureSample(source, source_sampler, uv - direction * texel * 3.230769) * 0.070270;
    return color;
}

@fragment
fn fs_blur_horizontal(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return gaussian(position.xy / globals.size, vec2(1.0, 0.0));
}

@fragment
fn fs_blur_vertical(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return gaussian(position.xy / globals.size, vec2(0.0, 1.0));
}

@fragment
fn fs_frost(input: RectOutput) -> @location(0) vec4<f32> {
    let pixel = input.position.xy;
    if pixel.x < input.clip.x || pixel.y < input.clip.y || pixel.x > input.clip.z || pixel.y > input.clip.w {
        discard;
    }
    let distance = rounded_distance(input.local, input.half_size, input.params.x);
    let coverage = 1.0 - smoothstep(-0.75, 0.75, distance);
    if coverage <= 0.0 { discard; }
    let border_mix = select(0.0, 1.0, input.params.y > 0.0 && distance > -input.params.y);
    let tint = mix(input.fill, input.border, border_mix);
    let tint_alpha = select(input.fill.a, 1.0, border_mix > 0.5);
    let backdrop = textureSample(source, source_sampler, pixel / globals.size);
    let color = mix(backdrop.rgb, tint.rgb, tint_alpha);
    return vec4(color, coverage);
}
