//! Window events and input: the `winit` application handler (resume, keyboard, mouse, focus, close) and menu key handling.

use super::*;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("Red Engine 2 — {}", self.scene_path.display()))
            // Maximized (not exclusive fullscreen) so it snaps to whatever monitor it opens on
            // at that monitor's native work area — centered and taskbar-aware, unlike a fixed
            // inner size that could land off-center on a different-resolution display. The
            // inner size below is only the fallback if the window is ever un-maximized.
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_maximized(true);
        // Debug: `RE2_WINDOW=x,y,w,h` places a plain window (to tile two clients side by side).
        let attrs = match std::env::var("RE2_WINDOW").ok().map(|v| v.split(',').filter_map(|n| n.trim().parse::<i32>().ok()).collect::<Vec<_>>()) {
            Some(v) if v.len() == 4 => attrs
                .with_maximized(false)
                .with_inner_size(winit::dpi::PhysicalSize::new(v[2].max(320) as u32, v[3].max(240) as u32))
                .with_position(winit::dpi::PhysicalPosition::new(v[0], v[1])),
            _ => attrs,
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("failed to create window"));

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("failed to create GPU surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter found (Red Engine 2 needs Vulkan, DX12, or Metal)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("red-engine-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("failed to create GPU device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // A scene policy, `--as` or `--connect` starts the game at once; otherwise the connect form (over its own tiny backdrop scene) shows first.
        let backdrop = self.start_character().is_none().then(|| LiveRenderer::new(&device, format, &self.backdrop_scene, config.width, config.height));
        self.gpu = Some(GpuState { surface: Some(surface), device, queue, config, live: None, backdrop });
        self.gpu_kind = "window".to_string();
        self.window = Some(window);
        self.last_frame = Instant::now();
        if self.start_fullscreen {
            self.set_fullscreen(true);
        }
        if let Some(who) = self.start_character() {
            self.start_game(who);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        // Fullscreen is a property of the window, not of a screen of the game: `F11` everywhere, and `F` everywhere the keyboard is not
        // typing (the connect form). It used to be handled only while playing, so the connect form, the pause menu and the lobby ignored it.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if let (PhysicalKey::Code(code), ElementState::Pressed, false) = (key.physical_key, key.state, key.repeat) {
                if code == KeyCode::F11 || (code == KeyCode::KeyF && self.phase != Phase::Connect) {
                    self.toggle_fullscreen();
                    return;
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                if let Some(net) = self.net.as_mut() {
                    net.client.disconnect(); // tell the server now instead of making it wait for the timeout
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    if let Some(surface) = &gpu.surface {
                        surface.configure(&gpu.device, &gpu.config);
                    }
                    for r in [gpu.live.as_mut(), gpu.backdrop.as_mut()].into_iter().flatten() {
                        r.resize(&gpu.device, gpu.config.width, gpu.config.height);
                    }
                }
                if self.paused {
                    self.repaint_pause();
                }
                self.online.painted = None;
                self.rule_hud_painted = None;
                self.repaint_maps();
                // A mode change (fullscreen on or off) released the mouse: take it back now that the window has settled.
                if std::mem::take(&mut self.regrab) && !self.paused && self.phase == Phase::Playing && !self.online.takeover {
                    self.set_grab(true);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.map_selection.is_some() => {
                if let (PhysicalKey::Code(code), ElementState::Pressed) = (event.physical_key, event.state) {
                    if !event.repeat {
                        self.map_key(code);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.map_selection.is_some() => self.cursor = (position.x as f32, position.y as f32),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.map_selection.is_some() => self.map_click(),
            // The connect form (typing) and the lobby / results screens (clicks): each owns the keyboard and the mouse while it shows.
            WindowEvent::KeyboardInput { event, .. } if self.phase == Phase::Connect => self.connect_key(&event, event_loop),
            WindowEvent::CursorMoved { position, .. } if self.phase == Phase::Connect => self.connect_hover(position.x as f32, position.y as f32),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.phase == Phase::Connect => self.connect_click(),
            WindowEvent::KeyboardInput { event, .. } if self.online.takeover && !self.paused => {
                if let (PhysicalKey::Code(code), ElementState::Pressed) = (event.physical_key, event.state) {
                    if !event.repeat {
                        self.online_key(code, event_loop);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.online.takeover && !self.paused => self.online_hover(position.x as f32, position.y as f32),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.online.takeover && !self.paused => {
                self.online_click(event_loop)
            }
            WindowEvent::CursorMoved { position, .. } if self.paused => {
                self.cursor = (position.x as f32, position.y as f32);
                let hover = self.gpu.as_ref().and_then(|g| menu::pause_action_at(g.config.width, g.config.height, self.cursor.0, self.cursor.1));
                if hover != self.pause_hover {
                    self.pause_hover = hover;
                    self.repaint_pause();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed && !event.repeat {
                        if code == KeyCode::KeyM {
                            self.open_maps();
                            return;
                        }
                        if code == KeyCode::Escape {
                            if self.paused {
                                self.leave_pause();
                            } else {
                                self.enter_pause();
                            }
                            return;
                        }
                        if self.paused && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
                            self.leave_pause();
                            return;
                        }
                    }
                    if self.paused {
                        return; // the menu owns the keyboard
                    }
                    if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                        self.sprint_held = event.state == ElementState::Pressed;
                    }
                    if code == KeyCode::Space && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.jump_queued = true;
                    }
                    if code == KeyCode::KeyQ && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_view_mode();
                    }
                    if code == KeyCode::KeyN && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_music();
                    }
                    if code == KeyCode::F3 && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.debug_hud = !self.debug_hud;
                        self.online.painted = None; // repaint the overlay with (or without) the debug lines
                        self.debug_text = self.debug_lines();
                    }
                    if code == KeyCode::F12 && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.shots.request("key", red_engine2::playscript::CameraSpec::First);
                    }
                    if code == KeyCode::KeyE && event.state == ElementState::Pressed && !self.keys.contains(&code) && self.grabbed {
                        self.interact();
                    }
                    if code == KeyCode::KeyR && event.state == ElementState::Pressed && !self.keys.contains(&code) && self.grabbed {
                        if self.net.is_some() {
                            self.net_pulse[3] = NET_PULSE_TICKS;
                        } else if self.weapon.is_firearm() {
                            let n = self.ammo.reload();
                            if n > 0 {
                                println!("Reloaded {n} round(s): {:?}", self.ammo);
                            }
                        }
                    }
                    match event.state {
                        ElementState::Pressed => {
                            self.keys.insert(code);
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.paused {
                    let hit = self.gpu.as_ref().and_then(|g| menu::pause_action_at(g.config.width, g.config.height, self.cursor.0, self.cursor.1));
                    match hit {
                        Some(PauseAction::Resume) => self.leave_pause(),
                        Some(PauseAction::Fullscreen) => self.toggle_fullscreen(),
                        Some(PauseAction::Quit) => event_loop.exit(),
                        None => {}
                    }
                } else if !self.grabbed {
                    self.set_grab(true);
                } else {
                    // Acted on by the next simulation tick (`fixed_step_combat`); in a peaceful scene it is an interaction instead.
                    self.press_primary();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.attack_held = false;
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if self.phase == Phase::Playing && !self.paused => {
                self.ads_held = state == ElementState::Pressed && self.grabbed;
            }
            WindowEvent::MouseWheel { delta, .. } if self.phase == Phase::Playing && self.grabbed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.on_scroll(lines);
            }
            WindowEvent::Focused(false) => {
                self.focused = false;
                // A match this game hosts waits for you: switching away opens the pause menu, which freezes it.
                if self.host_pause.is_some() && self.phase == Phase::Playing && !self.paused && !self.online.takeover {
                    self.enter_pause();
                }
                self.pad = Default::default();
                self.controller.reset();
                // No key-release events arrive while unfocused: forget held keys so we do not walk on alone.
                self.keys.clear();
                self.sprint_held = false;
                self.ads_held = false;
                self.set_grab(false);
            }
            WindowEvent::Focused(true) => self.focused = true,
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let raw_dt = (now - self.last_frame).as_secs_f32();
                let dt = raw_dt.min(0.1);
                self.last_frame = now;
                let pad_t0 = Instant::now();
                self.poll_controller(dt, event_loop);
                let pad_ms = pad_t0.elapsed().as_secs_f32() * 1000.0;
                // Online, the same line says how the other players fared: drawn, undrawn, stood in for, left out by interest management.
                let remote_summary = self
                    .net
                    .as_ref()
                    .map(|n| {
                        let s = n.stats();
                        format!(
                            "  | remote: {} drawn of {} in view, {} undrawn, {} stand-in, {} hidden by interest, {} unposed",
                            s.drawn, s.in_view, s.undrawn, s.standins, s.hidden_by_interest, s.unposed
                        )
                    })
                    .unwrap_or_default();
                if let Some(st) = &mut self.stats {
                    st.frames += 1;
                    st.worst_ms = st.worst_ms.max(raw_dt * 1000.0);
                    st.worst_pad_ms = st.worst_pad_ms.max(pad_ms);
                    let elapsed = st.window_start.elapsed().as_secs_f32();
                    if elapsed >= 2.0 {
                        println!(
                            "[stats] {:.0} fps  (avg {:.2} ms, worst {:.1} ms; slowest update {:.1} ms, draw {:.1} ms, gamepad poll {:.1} ms){}",
                            st.frames as f32 / elapsed,
                            elapsed * 1000.0 / st.frames as f32,
                            st.worst_ms,
                            st.worst_update_ms,
                            st.worst_draw_ms,
                            st.worst_pad_ms,
                            remote_summary
                        );
                        *st =
                            FrameStats { window_start: Instant::now(), frames: 0, worst_ms: 0.0, worst_update_ms: 0.0, worst_draw_ms: 0.0, worst_pad_ms: 0.0 };
                    }
                }
                match self.phase {
                    Phase::Connect => self.connect_frame(),
                    Phase::Playing => {
                        let t0 = Instant::now();
                        self.update(dt);
                        let t1 = Instant::now();
                        self.draw();
                        if let Some(st) = &mut self.stats {
                            st.worst_update_ms = st.worst_update_ms.max((t1 - t0).as_secs_f32() * 1000.0);
                            st.worst_draw_ms = st.worst_draw_ms.max(t1.elapsed().as_secs_f32() * 1000.0);
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if self.grabbed && self.grabbed_at.elapsed().as_secs_f32() > MOUSE_SETTLE_SECS {
                let sensitivity = MOUSE_SENSITIVITY * red_engine2::firearms::zoom_sensitivity(self.camera.fov_deg, self.scene.player.fov_deg);
                self.camera.look(dx as f32 * sensitivity, -dy as f32 * sensitivity);
            }
        }
    }
}
