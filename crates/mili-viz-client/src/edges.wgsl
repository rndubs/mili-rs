// Edge/wireframe shader: a flat constant-colour line pass over the
// unique mesh edges, sharing the mesh pass's camera uniform and vertex
// buffer (only the position attribute is read). Depth-tested so back
// edges are occluded by the filled hull in the overlay mode and a
// bare wireframe still self-occludes (VB-003).

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
    return vec4<f32>(0.86, 0.90, 1.0, 1.0);
}
