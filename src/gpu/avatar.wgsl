struct Globals { size: vec2<f32>, _padding: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
struct Input { @location(0) corner: vec2<f32>, @location(1) rect: vec4<f32>, @location(2) clip: vec4<f32>, @location(3) uv: vec4<f32> };
struct Output {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) @interpolate(flat) clip: vec4<f32>,
  @location(2) local: vec2<f32>,
  @location(3) @interpolate(flat) half_size: vec2<f32>,
};
@vertex fn vs_main(input: Input) -> Output {
  let position = input.rect.xy + input.corner * input.rect.zw;
  var output: Output;
  output.position = vec4<f32>(position.x / globals.size.x * 2.0 - 1.0, 1.0 - position.y / globals.size.y * 2.0, 0.0, 1.0);
  output.uv = input.uv.xy + input.corner * input.uv.zw;
  output.clip = input.clip;
  output.local = (input.corner - vec2<f32>(0.5)) * input.rect.zw;
  output.half_size = input.rect.zw * 0.5;
  return output;
}
@fragment fn fs_main(input: Output) -> @location(0) vec4<f32> {
  // Sample before any divergent control flow so implicit derivatives stay valid.
  let color = textureSample(atlas, atlas_sampler, input.uv);
  let pixel = input.position.xy;
  if pixel.x < input.clip.x || pixel.y < input.clip.y || pixel.x > input.clip.z || pixel.y > input.clip.w { discard; }
  // Avatars render as anti-aliased circles.
  let radius = min(input.half_size.x, input.half_size.y);
  let dist = length(input.local) - radius;
  let coverage = 1.0 - smoothstep(-0.75, 0.75, dist);
  return vec4<f32>(color.rgb, color.a * coverage);
}
