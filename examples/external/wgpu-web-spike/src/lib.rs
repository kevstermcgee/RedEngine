use wasm_bindgen::prelude::*;
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct U { mvp: mat4x4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VO { @builtin(position) p: vec4<f32>, @location(0) n: vec3<f32>, @location(1) c: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) n: vec3<f32>, @location(2) c: vec3<f32>) -> VO {
    var o: VO; o.p = u.mvp * vec4<f32>(pos, 1.0); o.n = n; o.c = c; return o;
}
@fragment fn fs(i: VO) -> @location(0) vec4<f32> {
    let d = max(dot(normalize(i.n), normalize(-u.light.xyz)), 0.0);
    return vec4<f32>(i.c * (0.35 + 0.65 * d), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct V { p: [f32; 3], n: [f32; 3], c: [f32; 3] }

fn cube() -> (Vec<V>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0.9, 0.3, 0.3]),
        ([-1., 0., 0.], [0., 1., 0.], [0., 0., -1.], [0.3, 0.9, 0.3]),
        ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.], [0.3, 0.3, 0.9]),
        ([0., -1., 0.], [0., 0., -1.], [1., 0., 0.], [0.9, 0.9, 0.3]),
        ([0., 0., 1.], [0., 1., 0.], [-1., 0., 0.], [0.3, 0.9, 0.9]),
        ([0., 0., -1.], [0., 1., 0.], [1., 0., 0.], [0.9, 0.3, 0.9]),
    ];
    let (mut v, mut i) = (Vec::new(), Vec::new());
    for (n, up, right, c) in faces {
        let base = v.len() as u16;
        for (a, b) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            let p = [n[0] + up[0] * b + right[0] * a, n[1] + up[1] * b + right[1] * a, n[2] + up[2] * b + right[2] * a];
            v.push(V { p, n, c });
        }
        i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (v, i)
}

/// Starts the spike on the canvas `#c`; reports through `window.__spike`.
#[wasm_bindgen]
pub async fn start(force_gl: bool) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let win = web_sys::window().unwrap();
    let canvas: web_sys::HtmlCanvasElement = win.document().unwrap().get_element_by_id("c").unwrap().dyn_into()?;
    let (w, h) = (canvas.width(), canvas.height());
    let backends = if force_gl { wgpu::Backends::GL } else { wgpu::Backends::BROWSER_WEBGPU };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| JsValue::from_str(&format!("surface: {e}")))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })
        .await
        .map_err(|e| JsValue::from_str(&format!("adapter: {e}")))?;
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: if force_gl { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::default() },
            ..Default::default()
        })
        .await
        .map_err(|e| JsValue::from_str(&format!("device: {e}")))?;
    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats[0];
    surface.configure(&device, &wgpu::SurfaceConfiguration { usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format, width: w, height: h, present_mode: wgpu::PresentMode::Fifo, alpha_mode: caps.alpha_modes[0], view_formats: vec![], desired_maximum_frame_latency: 2, color_space: wgpu::SurfaceColorSpace::Auto });
    let (verts, idx) = cube();
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
    let ub = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }] });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }] });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
    let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 36, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(format.into())] }),
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
    let dview = depth.create_view(&Default::default());


    // Offscreen proof that does not depend on presentation: draw one frame into a texture, copy it to a buffer, map it, count colours.
    {
        let tex = device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let tview = tex.create_view(&Default::default());
        let p = glam::Mat4::perspective_rh(0.9, w as f32 / h as f32, 0.1, 50.0) * glam::Mat4::look_at_rh(glam::Vec3::new(4., 3., 5.), glam::Vec3::ZERO, glam::Vec3::Y) * glam::Mat4::from_rotation_y(0.6);
        let mut data = [0f32; 20];
        data[..16].copy_from_slice(&p.to_cols_array());
        data[16..19].copy_from_slice(&[-0.4, -0.8, -0.45]);
        queue.write_buffer(&ub, 0, bytemuck::cast_slice(&data));
        let out = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (w * h * 4) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &tview, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.07, b: 0.12, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dview, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &out, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        out.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r.is_ok()); });
        // The browser resolves the map between tasks: yield until it has.
        let mut ok = None;
        for _ in 0..200 {
            let _ = device.poll(wgpu::PollType::Poll);
            if let Ok(v) = rx.try_recv() { ok = Some(v); break; }
            wasm_bindgen_futures::JsFuture::from(web_sys::js_sys::Promise::new(&mut |r, _| { let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&r, 10); })).await?;
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut centre = [0u8; 4];
        if ok == Some(true) {
            let view = out.slice(..).get_mapped_range().map_err(|e| JsValue::from_str(&format!("range: {e}")))?;
            for (k, px) in view.chunks_exact(4).enumerate() {
                seen.insert([px[0], px[1], px[2]]);
                if k == (h as usize / 2) * w as usize + w as usize / 2 { centre.copy_from_slice(px); }
            }
        }
        let o = web_sys::js_sys::Object::new();
        web_sys::js_sys::Reflect::set(&o, &"mapped".into(), &format!("{ok:?}").into())?;
        web_sys::js_sys::Reflect::set(&o, &"distinct".into(), &(seen.len() as u32).into())?;
        web_sys::js_sys::Reflect::set(&o, &"centre".into(), &format!("{centre:?}").into())?;
        web_sys::js_sys::Reflect::set(&win, &"__probe".into(), &o)?;
    }
    let frames = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let t0 = win.performance().unwrap().now();
    let f = std::rc::Rc::new(std::cell::RefCell::new(None::<Closure<dyn FnMut()>>));
    let g = f.clone();
    let frames2 = frames.clone();
    let win2 = win.clone();
    *g.borrow_mut() = Some(Closure::new(move || {
        let t = frames2.get() as f32 * 0.02;
        let p = glam::Mat4::perspective_rh(0.9, w as f32 / h as f32, 0.1, 50.0) * glam::Mat4::look_at_rh(glam::Vec3::new(4., 3., 5.), glam::Vec3::ZERO, glam::Vec3::Y) * glam::Mat4::from_rotation_y(t) * glam::Mat4::from_rotation_x(t * 0.5);
        let mut data = [0f32; 20];
        data[..16].copy_from_slice(&p.to_cols_array());
        data[16..19].copy_from_slice(&[-0.4, -0.8, -0.45]);
        queue.write_buffer(&ub, 0, bytemuck::cast_slice(&data));
        if let wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) = surface.get_current_texture() {
            let view = frame.texture.create_view(&Default::default());
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.07, b: 0.12, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dview, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
            }
            queue.submit([enc.finish()]);
            queue.present(frame);
        }
        frames2.set(frames2.get() + 1);
        let _ = js_sys_set(&win2, frames2.get());
        win2.request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref()).unwrap();
    }));
    win.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;
    let _ = t0;
    Ok(JsValue::from_str(&format!("{:?} | {} | {:?}", info.backend, info.name, info.device_type)))
}

fn js_sys_set(win: &web_sys::Window, frames: u32) -> Result<(), JsValue> {
    let o = web_sys::js_sys::Object::new();
    web_sys::js_sys::Reflect::set(&o, &"frames".into(), &frames.into())?;
    web_sys::js_sys::Reflect::set(win, &"__spike".into(), &o)?;
    Ok(())
}
