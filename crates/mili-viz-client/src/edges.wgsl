// Edge / wireframe shader: screen-space line quads with analytical AA.
//
// Why not `LineList`. Core WebGPU does not support `lineWidth > 1`, so
// the VB-003 `LineList` pass always rasterised a 1-px line — at native
// resolution the edges read as broken / partly invisible (MSAA only
// partly helped). This pass replaces it with **instanced screen-space
// quads**: each input edge is one instance, expanded in the vertex
// shader to a 2-triangle ribbon that is `line_width_px` wide along the
// screen-space normal, with an extra 1-px feather that the fragment
// shader uses to taper alpha analytically. See
// `planning/mili-viz/README.md` § "Edge rendering" for the rationale.
//
// Per-vertex layout:
//   @location(0) corner: vec2<f32>  // x ∈ {0,1} picks endpoint; y ∈ {-1,+1} picks side
// Per-instance layout:
//   @location(1) endpoint_a: vec3<f32>
//   @location(2) endpoint_b: vec3<f32>
//
// Depth-tested `LessEqual` + depth-write on so back edges are occluded
// by the filled hull in overlay mode and a bare wireframe still
// self-occludes (the original VB-003 semantics).

struct Uniforms {
    view_proj: mat4x4<f32>,
    // (viewport_px.x, viewport_px.y, line_width_px, _pad). The viewport
    // is the **scene** rect the host calls `set_viewport` with, not the
    // framebuffer, so the projected pixel widths land on the right
    // axis when the egui panels squeeze the scene off-centre.
    viewport_and_width: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    // Signed pixel distance from the line centre, ±(half_width + 1)
    // at the quad's outer edge. Fragment alpha is a smoothstep on
    // `|dist_px|` so the line edge antialiases without needing MSAA.
    @location(0) dist_px: f32,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) endpoint_a: vec3<f32>,
    @location(2) endpoint_b: vec3<f32>,
) -> VsOut {
    let viewport = u.viewport_and_width.xy;
    let line_w = max(u.viewport_and_width.z, 1.0);
    let half_px = line_w * 0.5;
    // +1 px buys the analytical-AA feather region the fragment shader
    // tapers across.
    let extend_px = half_px + 1.0;

    let a_clip = u.view_proj * vec4<f32>(endpoint_a, 1.0);
    let b_clip = u.view_proj * vec4<f32>(endpoint_b, 1.0);

    // Pick the active endpoint by `corner.x` ∈ {0,1}.
    let p_clip = mix(a_clip, b_clip, corner.x);

    // Screen-space direction in pixels, via NDC × half-viewport, so the
    // perpendicular has correct pixel units under perspective.
    let a_ndc = a_clip.xy / a_clip.w;
    let b_ndc = b_clip.xy / b_clip.w;
    let dir_px = (b_ndc - a_ndc) * viewport * 0.5;
    let dir = dir_px / max(length(dir_px), 1e-6);
    let normal = vec2<f32>(-dir.y, dir.x);

    // Pixel-space offset → NDC (×2/viewport) → clip (×w).
    let offset_px = normal * corner.y * extend_px;
    let offset_ndc = offset_px * 2.0 / viewport;
    let offset_clip = offset_ndc * p_clip.w;

    var out: VsOut;
    out.clip_position = vec4<f32>(p_clip.xy + offset_clip, p_clip.z, p_clip.w);
    out.dist_px = corner.y * extend_px;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half_px = max(u.viewport_and_width.z, 1.0) * 0.5;
    // alpha = 1 inside the core, linear taper across the 1-px feather.
    // At |dist| = half_px - 0.5 alpha is 1; at |dist| = half_px + 0.5
    // alpha is 0.
    let alpha = clamp(half_px + 0.5 - abs(in.dist_px), 0.0, 1.0);
    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
