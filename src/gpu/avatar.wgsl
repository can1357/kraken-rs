struct Globals { size: vec2<f32>, _padding: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
struct Input { @location(0) corner: vec2<f32>, @location(1) rect: vec4<f32>, @location(2) clip: vec4<f32>, @location(3) uv: vec4<f32> };
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) @interpolate(flat) clip: vec4<f32> };
@vertex fn vs_main(input: Input) -> Output {
  let position = input.rect.xy + input.corner * input.rect.zw;
  var output: Output;
  output.position = vec4<f32>(position.x / globals.size.x * 2.0 - 1.0, 1.0 - position.y / globals.size.y * 2.0, 0.0, 1.0);
  output.uv = input.uv.xy + input.corner * input.uv.zw;
  output.clip = input.clip;
  return output;
}
@fragment fn fs_main(input: Output) -> @location(0) vec4<f32> {
  let pixel = input.position.xy;
  if pixel.x < input.clip.x || pixel.y < input.clip.y || pixel.x > input.clip.z || pixel.y > input.clip.w { discard; }
  return textureSample(atlas, atlas_sampler, input.uv);
}
