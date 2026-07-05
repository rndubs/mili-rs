//! Temporary visual-debug harness: render corpus meshes headlessly in
//! each RenderMode, at MSAA 1 and 4, and write PNGs for inspection.
//! Not part of the test suite — run with:
//!   cargo run -p mili-viz-client --example render_debug -- <out_dir>

use std::path::{Path, PathBuf};

use mili_viz_client::{
    fetch_server_mesh, headless_device, write_snapshot_png, Camera, RenderMode, Renderer,
    OFFSCREEN_FORMAT,
};

fn corpus_path(rel: &[&str]) -> PathBuf {
    let mut p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data");
    for c in rel {
        p = p.join(c);
    }
    p
}

fn render_with_samples(
    mesh: &mili_viz_client::Mesh,
    range: Option<(f32, f32)>,
    mode: RenderMode,
    samples: u32,
    w: u32,
    h: u32,
    camera: &Camera,
) -> Vec<u8> {
    let (device, queue) = headless_device().expect("wgpu adapter (lavapipe)");
    let mut renderer = Renderer::new_with_samples(device, queue, OFFSCREEN_FORMAT, samples);
    renderer.set_mode(mode);
    renderer.upload_mesh(mesh, range, "cool");

    let device = renderer.device();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("debug target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(&view, w, h, camera);

    // Read back.
    let unpadded = w * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    renderer
        .device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((unpadded * h) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    out
}

#[tokio::main]
async fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp".into());
    std::fs::create_dir_all(&out_dir).unwrap();
    let (w, h) = (800u32, 600u32);

    for (name, rel, result) in [
        ("basic1", vec!["serial", "basic1", "basic1.pltA"], ""),
        ("basic1-sx", vec!["serial", "basic1", "basic1.pltA"], "sx"),
        ("d3samp4", vec!["serial", "d3samp4", "d3samp4.pltA"], ""),
        ("tet", vec!["serial", "tet", "tet.pltA"], ""),
    ] {
        let path = corpus_path(&rel.iter().map(|s| *s).collect::<Vec<_>>());
        if !path.exists() {
            eprintln!("skip {name}: fixture absent");
            continue;
        }
        let mesh = match fetch_server_mesh(&path.to_string_lossy(), result).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("skip {name}: {e}");
                continue;
            }
        };
        let range = mesh.scalars.as_ref().map(|s| {
            let lo = s.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = s.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            (lo, hi)
        });
        let (center, radius) = mesh.bounds();
        let camera = Camera::looking_at(center, radius);
        for (mode, mname) in [
            (RenderMode::Shaded, "shaded"),
            (RenderMode::Edges, "edges"),
            (RenderMode::FeatureEdges, "featureedges"),
            (RenderMode::Wireframe, "wireframe"),
        ] {
            for samples in [1u32, 4] {
                let px = render_with_samples(&mesh, range, mode, samples, w, h, &camera);
                let out = Path::new(&out_dir).join(format!("{name}-{mname}-msaa{samples}.png"));
                write_snapshot_png(&out, w, h, &px).unwrap();
                println!("{}", out.display());
            }
        }
    }
}
