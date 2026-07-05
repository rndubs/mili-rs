// Mesh shader: camera-relative headlight + Blinn-Phong specular over
// the per-vertex base colour (the M3 colormap of the MVG2 scalar, or
// the uniform M2 base when no scalar — phase-5-m3.md Decision 47).
//
// Lighting model (rendering-quality pass, 2026-07):
//  * All math is in **linear** RGB. `upload_mesh` converts the
//    sRGB-authored base/colormap colours to linear before upload; the
//    fragment end re-encodes to sRGB manually when the target format
//    is non-sRGB (`viewport_and_width.w == 0`, the headless RGBA8
//    path) and leaves the hardware to encode when it is sRGB (the
//    windowed surface). This makes the window and the snapshot PNG
//    agree instead of being platform-dependent.
//  * The light rides the camera (an over-the-shoulder headlight
//    computed per frame in `render_in`), so the model is always lit
//    no matter how it is orbited — the old fixed world-space light
//    left whole orientations in flat ambient darkness.
//  * Two-sided: a closed hull's outward winding is not guaranteed, so
//    lighting uses |cos| terms — sign-symmetric and, unlike flipping
//    the normal toward the viewer, continuous at grazing incidence
//    (a hard flip speckles faces with `dot(n, v) ≈ 0`).
//  * Crease-aware normals: the CPU vertex normals are area-averaged
//    across *all* incident triangles, which melts 90° FEA corners
//    into soft bevels. Where the smoothed normal deviates from the
//    true face normal (screen-space derivatives) by more than ~30°,
//    the face normal wins — crisp creases, still-smooth curved
//    surfaces, no vertex splitting (which would break the node-array
//    order picking and the MVG3 edge indices rely on).

struct Uniforms {
    view_proj: mat4x4<f32>,
    // (viewport_px.x, viewport_px.y, line_width_px, srgb_target_flag).
    viewport_and_width: vec4<f32>,
    // World-space unit vector surface→light, camera-relative headlight
    // tilted up/right; .w unused.
    light_dir: vec4<f32>,
    // World-space unit vector surface→eye; .w unused.
    view_dir: vec4<f32>,
    // Edge-pass parameters (linear rgb + density-fade floor); unused
    // here, present so the mesh + edge pipelines share one uniform
    // buffer layout.
    edge_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) world_pos: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.clip_position = u.view_proj * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    out.world_pos = position;
    return out;
}

// Piecewise sRGB encode (linear → display). Matches the hardware
// encode an sRGB target performs, so both target kinds look the same.
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// cos(30°) — the crease threshold, matching FEATURE_EDGE_ANGLE_DEG so
// shading creases land exactly where the FeatureEdges lines draw.
const CREASE_COS: f32 = 0.86602540;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let v = normalize(u.view_dir.xyz);
    var n = normalize(in.normal);
    // Crease fix: prefer the true face normal where the smoothed
    // vertex normal has been bent > ~30° away from it. Signs of both
    // normals are arbitrary (winding / two-sided), so the comparison
    // is on |cos| and the chosen face normal is oriented into the
    // vertex normal's hemisphere.
    let nf = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    let nf_dot = dot(nf, n);
    if (abs(nf_dot) < CREASE_COS) {
        n = nf * sign(nf_dot);
    }

    // Two-sided lighting via |cos| terms rather than flipping `n`
    // toward the viewer: a hard sign flip flip-flops per fragment on
    // grazing faces (`dot(n, v) ≈ 0`) and speckles them; abs() is the
    // continuous equivalent.
    let l = normalize(u.light_dir.xyz);
    let ndl = abs(dot(n, l));
    let h = normalize(l + v);
    let spec = 0.18 * pow(abs(dot(n, h)), 32.0);
    let shade = 0.22 + 0.78 * ndl;
    var rgb = in.color * shade + vec3<f32>(spec);
    if (u.viewport_and_width.w < 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
