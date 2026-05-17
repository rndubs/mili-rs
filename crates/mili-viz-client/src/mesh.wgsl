// M2 mesh shader: a single fixed directional light + ambient term so
// the decoded server hull reads as a 3-D surface (phase-5-m2.md
// Decision 42). Scalar→color is Phase 5 M3; M2 is a uniform base
// color.

struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.clip_position = u.view_proj * vec4<f32>(position, 1.0);
    out.normal = normal;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = vec3<f32>(0.62, 0.68, 0.80);
    let light_dir = normalize(vec3<f32>(0.35, 0.55, 0.75));
    // Two-sided: a closed hull's outward winding is not guaranteed, so
    // light the face we actually see.
    let n = normalize(in.normal);
    let diffuse = abs(dot(n, light_dir));
    let ambient = 0.45;
    let shade = ambient + (1.0 - ambient) * diffuse;
    return vec4<f32>(base * shade, 1.0);
}
