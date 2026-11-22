// Palette lookup + nearest-neighbour scale. Drawn as one fullscreen
// triangle; the viewport rect places it in the window.

@group(0) @binding(0) var frame: texture_2d<u32>;
@group(0) @binding(1) var palette: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // (0,0) (2,0) (0,2) in uv space covers the unit square.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = textureDimensions(frame);
    let xy = min(vec2<u32>(in.uv * vec2<f32>(dims)), dims - vec2<u32>(1u));
    let index = textureLoad(frame, xy, 0).r;
    return vec4<f32>(textureLoad(palette, vec2<u32>(index, 0u), 0).rgb, 1.0);
}
