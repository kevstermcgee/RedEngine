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

        // With a forced character the game starts at once; otherwise the launch menu (whose
        // 3-D backdrop is its own tiny scene) shows first.
        let menu = self.forced_character.is_none().then(|| LiveRenderer::new(&device, format, &self.menu_scene, config.width, config.height));
        self.gpu = Some(GpuState { surface, device, queue, config, live: None, menu });
        self.window = Some(window);
        self.last_frame = Instant::now();
        if let Some(who) = self.forced_character {
            self.start_game(who);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    for r in [gpu.live.as_mut(), gpu.menu.as_mut()].into_iter().flatten() {
                        r.resize(&gpu.device, gpu.config.width, gpu.config.height);
                    }
                }
                if self.paused {
                    self.repaint_pause();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.phase == Phase::Menu => {
                if let (PhysicalKey::Code(code), ElementState::Pressed) = (event.physical_key, event.state) {
                    self.menu_key(code, event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.phase == Phase::Menu => {
                self.cursor_x = position.x as f32;
                if let Some(gpu) = &self.gpu {
                    self.character = menu::character_at(gpu.config.width, self.cursor_x);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.phase == Phase::Menu => {
                if let Some(gpu) = &self.gpu {
                    let who = menu::character_at(gpu.config.width, self.cursor_x);
                    self.start_game(who);
                }
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
                    if code == KeyCode::KeyF && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_fullscreen();
                    }
                    if code == KeyCode::KeyQ && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_view_mode();
                    }
                    if code == KeyCode::KeyE && event.state == ElementState::Pressed && !self.keys.contains(&code) && self.grabbed {
                        self.interact();
                    }
                    if code == KeyCode::KeyR && event.state == ElementState::Pressed && !self.keys.contains(&code) && self.grabbed {
                        if self.net.is_some() {
                            self.net_pulse[3] = NET_PULSE_TICKS;
                        } else if self.weapon == Weapon::Revolver {
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
                        Some(PauseAction::Quit) => event_loop.exit(),
                        None => {}
                    }
                } else if !self.grabbed {
                    self.set_grab(true);
                } else {
                    // Acted on by the next simulation tick (`fixed_step_combat`).
                    self.attack_queued = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.phase == Phase::Playing && self.grabbed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.on_scroll(lines);
            }
            WindowEvent::Focused(false) => {
                // No key-release events arrive while unfocused: forget held keys so we do not walk on alone.
                self.keys.clear();
                self.sprint_held = false;
                self.set_grab(false);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let raw_dt = (now - self.last_frame).as_secs_f32();
                let dt = raw_dt.min(0.1);
                self.last_frame = now;
                if let Some(st) = &mut self.stats {
                    st.frames += 1;
                    st.worst_ms = st.worst_ms.max(raw_dt * 1000.0);
                    let elapsed = st.window_start.elapsed().as_secs_f32();
                    if elapsed >= 2.0 {
                        println!(
                            "[stats] {:.0} fps  (avg {:.2} ms, worst {:.1} ms)",
                            st.frames as f32 / elapsed,
                            elapsed * 1000.0 / st.frames as f32,
                            st.worst_ms
                        );
                        *st = FrameStats { window_start: Instant::now(), frames: 0, worst_ms: 0.0 };
                    }
                }
                match self.phase {
                    Phase::Menu => self.menu_frame(),
                    Phase::Playing => {
                        self.update(dt);
                        self.draw();
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
                self.camera.look(dx as f32 * MOUSE_SENSITIVITY, -dy as f32 * MOUSE_SENSITIVITY);
            }
        }
    }
}

impl App {
    /// Menu keyboard: 1 / 2 pick and start, arrows / A / D move the highlight, Enter or Space starts.
    pub(crate) fn menu_key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        match code {
            KeyCode::Digit1 | KeyCode::Numpad1 => self.start_game(Character::Human),
            KeyCode::Digit2 | KeyCode::Numpad2 => self.start_game(Character::Rat),
            KeyCode::ArrowLeft | KeyCode::KeyA => self.character = Character::Human,
            KeyCode::ArrowRight | KeyCode::KeyD => self.character = Character::Rat,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.start_game(self.character),
            KeyCode::Escape => event_loop.exit(),
            _ => {}
        }
    }
}
