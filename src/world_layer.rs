//! The seam a generated world draws through (ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library): both renderers hold a list of
//! [`WorldLayer`]s instead of naming each world, and draw them at the same points of their frame (prepared after the scene's uniforms, their solid
//! parts after the scene's opaque meshes, their shadows in every cascade, their see-through parts after the scene's, their glows after those).
//!
//! A scene's layers come from [`layers_for`]: the endless meadow (`procgen`, the engine's own [`crate::stream_gpu::StreamLayer`]) first, then a
//! layer from each [`LayerDef`] a crate defined ([`define_layers`]), in the order they were defined, as a scene block is
//! (`schema::define_blocks`). A layer the client feeds every frame (Frameless's museum) is reached by its type ([`find_layer_mut`]).

use crate::schema::Scene;
use crate::stream_gpu::ShadowDetail;
use glam::{Mat4, Vec3, Vec4};
use std::any::Any;

/// What a layer is built with.
pub struct LayerInit<'a> {
    pub device: &'a wgpu::Device,
    /// The bind group layout of every draw's object uniform (group 1).
    pub object_layout: &'a wgpu::BindGroupLayout,
    /// A live renderer (a window or a client's offscreen frames, fed by the client every frame) rather than a still picture's.
    pub live: bool,
}

/// One frame, as a layer prepares for it.
pub struct LayerFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// The camera (world space) and where it looks.
    pub eye: Vec3,
    pub forward: Vec3,
    /// Every viewer of this frame (split screen has several), world space; the camera's eye alone otherwise.
    pub eyes: &'a [Vec3],
    /// The renderer's origin: draws are placed relative to it.
    pub origin: Vec3,
    pub view_proj: Mat4,
    /// The scene's time, seconds.
    pub t: f32,
    pub clock: Option<&'a crate::daycycle::Clock>,
    /// A still picture: build everything in view now rather than over the coming frames.
    pub settle: bool,
}

/// The light a layer gives a still picture (a live client lights its scene from its game mode).
#[derive(Debug, Clone, Default)]
pub struct StillLight {
    /// Point lights: position (world), colour times intensity, range.
    pub lamps: Vec<(Vec3, Vec3, f32)>,
    /// What a game's own shaders read with `game_param(i)`, when the layer sets them.
    pub game_params: Option<[[f32; 4]; 8]>,
}

/// A generated world both renderers draw. Every method but [`WorldLayer::draw_main`] has a default that does nothing.
pub trait WorldLayer: Any {
    /// A still picture's light, standing at `eye` (called before the frame's uniforms are written; only by the offline renderer).
    fn still_light(&mut self, _eye: Vec3) -> Option<StillLight> {
        None
    }

    /// Brings the GPU side to this frame (after the scene's own uniforms are written).
    fn prepare(&mut self, _frame: &LayerFrame) {}

    /// Builds everything around `eyes` before returning (a picture is about to be taken).
    fn settle(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, _eyes: &[Vec3]) {}

    /// How far the world reaches, metres (split screen sees less far).
    fn set_view_distance(&mut self, _metres: f32) {}

    /// Its shadow casters in one cascade (`planes` that cascade's frustum, `instance` its index).
    fn draw_shadow(&self, _pass: &mut wgpu::RenderPass<'_>, _planes: &[Vec4; 6], _detail: ShadowDetail, _instance: std::ops::Range<u32>) {}

    /// Its solid surfaces, after the scene's opaque meshes (the pass has the main pipeline and bindings).
    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, view_proj: Mat4);

    /// Its see-through surfaces (water, glass), after the scene's own: `alpha` is the main pipeline with source-alpha blending, depth-tested, writing no
    /// depth and culling no faces; a draw's opacity is its uniform's `base_color.a`. The layer sets the pipeline and orders its draws far to near.
    fn draw_blended(&self, _pass: &mut wgpu::RenderPass<'_>, _alpha: &wgpu::RenderPipeline) {}

    /// What glows, after the see-through surfaces (`glow` is the blended pipeline that writes no depth).
    fn draw_glow(&self, _pass: &mut wgpu::RenderPass<'_>, _glow: &wgpu::RenderPipeline) {}
}

/// A layer a crate defines for its scene block: built when a scene has it.
pub struct LayerDef {
    /// Its name (`museum`): a name already defined keeps its first definition.
    pub name: &'static str,
    /// The layer for a scene, when the scene has what it draws.
    pub build: fn(&Scene, &LayerInit) -> Option<Box<dyn WorldLayer>>,
    /// The game's painted surfaces (a WGSL file with `fn surface`) the scene's pipelines compose, when the scene names one.
    pub art: fn(&Scene) -> Option<&str>,
}

static LAYERS: std::sync::RwLock<Vec<&'static LayerDef>> = std::sync::RwLock::new(Vec::new());

/// Adds the layers a crate defines; call it with its scene blocks (`scene_blocks::register` does for the engine's).
pub fn define_layers(defs: &'static [LayerDef]) {
    let mut all = LAYERS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    for d in defs {
        if !all.iter().any(|l| l.name == d.name) {
            all.push(d);
        }
    }
}

fn defined() -> Vec<&'static LayerDef> {
    LAYERS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The scene's layers: the meadow, when it has a `procgen` block, then each defined layer that builds for it.
pub fn layers_for(scene: &Scene, init: &LayerInit) -> Vec<Box<dyn WorldLayer>> {
    let mut layers: Vec<Box<dyn WorldLayer>> = Vec::new();
    if let Some(cfg) = scene.procgen.clone() {
        // A window streams with background workers; a still picture builds what it needs inside `fill`.
        let workers = if init.live { std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 3)) } else { 0 };
        layers.push(Box::new(crate::stream_gpu::StreamLayer::new(init.device, init.object_layout, cfg, crate::procgen::View::default(), workers)));
    }
    layers.extend(defined().iter().filter_map(|d| (d.build)(scene, init)));
    layers
}

/// The game's painted surfaces for this scene, from the first defined layer that names some.
pub fn art_of(scene: &Scene) -> Option<&str> {
    defined().into_iter().find_map(|d| (d.art)(scene))
}

/// The layer of type `T` among `layers`.
pub fn find_layer<T: WorldLayer>(layers: &[Box<dyn WorldLayer>]) -> Option<&T> {
    layers.iter().find_map(|l| (l.as_ref() as &dyn Any).downcast_ref::<T>())
}

/// The layer of type `T` among `layers`, to change.
pub fn find_layer_mut<T: WorldLayer>(layers: &mut [Box<dyn WorldLayer>]) -> Option<&mut T> {
    layers.iter_mut().find_map(|l| (l.as_mut() as &mut dyn Any).downcast_mut::<T>())
}

impl WorldLayer for crate::stream_gpu::StreamLayer {
    fn prepare(&mut self, f: &LayerFrame) {
        if f.settle {
            self.fill(f.device, f.queue, f.eyes);
        } else {
            self.update(f.device, f.queue, f.eyes);
        }
        self.set_origin(f.queue, f.origin);
        self.update_motes(f.queue, f.eye, f.forward, f.t, f.clock);
    }

    fn settle(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: &[Vec3]) {
        self.fill(device, queue, eyes);
    }

    fn set_view_distance(&mut self, metres: f32) {
        crate::stream_gpu::StreamLayer::set_view_distance(self, metres);
    }

    fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, planes: &[Vec4; 6], detail: ShadowDetail, instance: std::ops::Range<u32>) {
        crate::stream_gpu::StreamLayer::draw_shadow(self, pass, planes, detail, instance);
    }

    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, view_proj: Mat4) {
        crate::stream_gpu::StreamLayer::draw_main(self, pass, view_proj);
    }

    fn draw_glow(&self, pass: &mut wgpu::RenderPass<'_>, glow: &wgpu::RenderPipeline) {
        self.draw_motes(pass, glow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(text: &str) -> Scene {
        crate::scene_blocks::parse_scene(text).expect("parses")
    }

    #[test]
    fn a_layer_defined_twice_is_defined_once_and_a_plain_scene_has_no_art() {
        static TWICE: &[LayerDef] = &[LayerDef { name: "test-twice", build: |_, _| None, art: |_| None }];
        define_layers(TWICE);
        define_layers(TWICE);
        assert_eq!(defined().iter().filter(|d| d.name == "test-twice").count(), 1);
        let plain = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": []}"#);
        assert_eq!(art_of(&plain), None);
    }

    struct Flat;
    impl WorldLayer for Flat {
        fn draw_main(&self, _pass: &mut wgpu::RenderPass<'_>, _view_proj: Mat4) {}
    }
    struct Other;
    impl WorldLayer for Other {
        fn draw_main(&self, _pass: &mut wgpu::RenderPass<'_>, _view_proj: Mat4) {}
    }

    #[test]
    fn a_layer_is_found_by_its_type() {
        let mut layers: Vec<Box<dyn WorldLayer>> = vec![Box::new(Other), Box::new(Flat)];
        assert!(find_layer::<Flat>(&layers).is_some());
        assert!(find_layer_mut::<Other>(&mut layers).is_some());
        assert!(find_layer::<crate::stream_gpu::StreamLayer>(&layers).is_none());
        assert!(Flat.still_light(Vec3::ZERO).is_none(), "a layer gives a still picture no light unless it says so");
    }
}
