struct Globals {
    size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> globals: Globals;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) clip: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let normalized = vec2<f32>(
        input.position.x / globals.size.x * 2.0 - 1.0,
        1.0 - input.position.y / globals.size.y * 2.0,
    );
    var output: VertexOutput;
    output.position = vec4<f32>(normalized, 0.0, 1.0);
    output.color = input.color;
    output.clip = input.clip;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = input.position.xy;
    if pixel.x < input.clip.x || pixel.y < input.clip.y ||
       pixel.x > input.clip.z || pixel.y > input.clip.w {
        discard;
    }
    return input.color;
}
