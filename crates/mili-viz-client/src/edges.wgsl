// Edge/wireframe shader: a flat constant-colour line pass over the
// unique mesh edges, sharing the mesh pass's camera uniform and vertex
// buffer (only the position attribute is read). Depth-tested so back
// edges are occluded by the filled hull in the overlay mode and a
// bare wireframe still self-occludes (VB-003).
//
// Black on the lit hull for contrast; the windowed renderer pairs this
// with 4× MSAA so the 1-px LineList lines don't alias into a broken
// "dashed" look. See `planning/mili-viz/README.md` § "Edge rendering"
// for the upgrade path to screen-space line quads.

struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
