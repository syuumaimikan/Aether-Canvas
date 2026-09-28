//! Godot 4 extension for Aether Canvas models.
//!
//! Adds one node, `AetherModel2D`: point it at an exported `model.json` and
//! it plays the model — the same rig code as the editor, so every rig
//! feature moves as authored — drawing each part as a canvas item with its
//! texture, tint, blend mode and clipping (via Godot's canvas groups).
//! Parameters, motions, expressions, look-at, lip sync, face tracking and
//! hit testing are exposed to GDScript, and motion events arrive as a
//! signal.

use aether_player::tracking::FaceFrame;
use aether_player::{BlendKind, Player};
use godot::classes::rendering_server::{CanvasGroupMode, CanvasItemTextureFilter};
use godot::classes::{
    FileAccess, INode2D, Image, ImageTexture, Node2D, RenderingServer, ResourceLoader, Shader,
    ShaderMaterial, Texture2D,
};
use godot::prelude::*;

struct AetherExtension;

#[gdextension]
unsafe impl ExtensionLibrary for AetherExtension {}

/// The canvas shader for one blend mode. Textures are premultiplied on
/// load; tint follows the runtime's model (multiply, then screen, on
/// straight colour).
fn shader_code(blend: BlendKind) -> String {
    let (render_mode, output) = match blend {
        BlendKind::Normal => ("blend_premul_alpha", "COLOR = vec4(rgb, c.a) * opacity;"),
        // Godot multiplies the destination by the output. Writing
        // Cs + (1 - αs) makes that Cs·Cd + Cd·(1 - αs): premultiplied
        // multiply over opaque artwork.
        BlendKind::Multiply => (
            "blend_mul",
            "COLOR = vec4(rgb * opacity + (1.0 - c.a * opacity), 1.0);",
        ),
        // Additive: the destination plus the (premultiplied) source.
        BlendKind::Add => ("blend_add", "COLOR = vec4(rgb * opacity, 1.0);"),
        // Godot has no screen blend; source-over with the colour's own
        // brightness as coverage comes close for light-on-dark glows.
        BlendKind::Screen => (
            "blend_premul_alpha",
            "vec3 s = rgb * opacity; COLOR = vec4(s, max(s.r, max(s.g, s.b)));",
        ),
    };
    format!(
        "shader_type canvas_item;
render_mode {render_mode};
uniform float opacity = 1.0;
uniform vec3 multiply = vec3(1.0);
uniform vec3 screen = vec3(0.0);
void fragment() {{
    vec4 c = texture(TEXTURE, UV);
    vec3 rgb = c.rgb * multiply;
    rgb = rgb + screen * c.a - rgb * screen;
    {output}
}}
"
    )
}

/// One part's canvas item and its geometry in Godot's types.
struct PartItem {
    item: Rid,
    material: Gd<ShaderMaterial>,
    indices: PackedInt32Array,
    uvs: PackedVector2Array,
    texture: usize,
}

/// Plays an Aether Canvas model.
#[derive(GodotClass)]
#[class(base = Node2D)]
pub struct AetherModel2D {
    /// The exported `model.json`; texture pages load from next to it.
    #[export(file = "*.json")]
    model_path: GString,
    /// Motion to start when the model loads (empty for none).
    #[export]
    autoplay: GString,
    /// Playback speed.
    #[export]
    speed: f32,
    /// Advance time every frame.
    #[export]
    playing: bool,
    player: Option<Player>,
    textures: Vec<Gd<ImageTexture>>,
    shaders: Vec<(BlendKind, Gd<Shader>)>,
    items: Vec<PartItem>,
    base: Base<Node2D>,
}

#[godot_api]
impl INode2D for AetherModel2D {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            model_path: GString::new(),
            autoplay: GString::new(),
            speed: 1.0,
            playing: true,
            player: None,
            textures: Vec::new(),
            shaders: Vec::new(),
            items: Vec::new(),
            base,
        }
    }

    fn ready(&mut self) {
        if !self.model_path.is_empty() && self.player.is_none() {
            let path = self.model_path.clone();
            self.load(path);
        }
    }

    fn process(&mut self, delta: f64) {
        if !self.playing {
            return;
        }
        let dt = delta as f32 * self.speed;
        let events = match &mut self.player {
            Some(player) => {
                player.tick(dt);
                player
                    .events()
                    .iter()
                    .map(|e| {
                        let motion = player
                            .model()
                            .rig
                            .motions
                            .get(e.motion)
                            .map(|m| m.name.clone())
                            .unwrap_or_default();
                        (e.name.clone(), motion)
                    })
                    .collect::<Vec<_>>()
            }
            None => return,
        };
        self.redraw();
        for (name, motion) in events {
            self.base_mut().emit_signal(
                "motion_event",
                &[
                    GString::from(name.as_str()).to_variant(),
                    GString::from(motion.as_str()).to_variant(),
                ],
            );
        }
    }

    fn exit_tree(&mut self) {
        self.release();
    }
}

#[godot_api]
impl AetherModel2D {
    /// A motion's timeline event was passed: its name and the motion's.
    #[signal]
    fn motion_event(name: GString, motion: GString);

    /// The model finished loading.
    #[signal]
    fn model_loaded();

    /// Load a model from `model.json` (a `res://` or file path). Returns
    /// false, with the reason printed, when it cannot.
    #[func]
    pub fn load(&mut self, path: GString) -> bool {
        self.release();
        let text = FileAccess::get_file_as_string(&path);
        let player = match Player::from_json(&text.to_string()) {
            Ok(player) => player,
            Err(error) => {
                godot_error!("AetherModel2D: could not load {path}: {error}");
                return false;
            }
        };
        let path_string = path.to_string();
        let dir = path_string
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        let mut textures = Vec::new();
        for texture in &player.model().textures {
            let file = format!("{dir}/{}", texture.file);
            let Some(mut image) = load_image(&file) else {
                godot_error!("AetherModel2D: could not load texture {file}");
                return false;
            };
            image.premultiply_alpha();
            let Some(texture) = ImageTexture::create_from_image(&image) else {
                godot_error!("AetherModel2D: could not upload texture {file}");
                return false;
            };
            textures.push(texture);
        }
        self.textures = textures;
        self.model_path = path;

        let mut server = RenderingServer::singleton();
        let parent = self.base().get_canvas_item();
        let mut items = Vec::new();
        for part in &player.model().parts {
            let shader = self.shader(part.blend);
            let mut material = ShaderMaterial::new_gd();
            material.set_shader(&shader);
            let item = server.canvas_item_create();
            server.canvas_item_set_parent(item, parent);
            server.canvas_item_set_material(item, material.get_rid());
            server.canvas_item_set_default_texture_filter(item, CanvasItemTextureFilter::LINEAR);
            items.push(PartItem {
                item,
                material,
                indices: part.triangles.iter().flatten().map(|&i| i as i32).collect(),
                uvs: part.uvs.iter().map(|uv| Vector2::new(uv.x, uv.y)).collect(),
                texture: part.texture as usize,
            });
        }
        self.items = items;
        self.player = Some(player);
        if !self.autoplay.is_empty() {
            let motion = self.autoplay.clone();
            self.play_motion(motion, false);
        }
        self.redraw();
        self.base_mut().emit_signal("model_loaded", &[]);
        true
    }

    /// True once a model is loaded.
    #[func]
    pub fn is_loaded(&self) -> bool {
        self.player.is_some()
    }

    /// The model's canvas size in pixels (its local coordinates).
    #[func]
    pub fn get_canvas_size(&self) -> Vector2 {
        self.player
            .as_ref()
            .map(|p| Vector2::new(p.model().width as f32, p.model().height as f32))
            .unwrap_or(Vector2::ZERO)
    }

    /// Parameter names, in panel order.
    #[func]
    pub fn get_parameter_names(&self) -> PackedStringArray {
        self.player
            .as_ref()
            .map(|p| {
                p.parameters()
                    .iter()
                    .map(|q| GString::from(q.name.as_str()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Set a parameter's base value; motions and physics layer on top.
    #[func]
    pub fn set_parameter(&mut self, name: GString, value: f32) {
        if let Some(player) = &mut self.player {
            if let Some(i) = player.parameter_index(&name.to_string()) {
                player.set_parameter(i, value);
            }
        }
    }

    /// A parameter's value as last drawn (NAN for an unknown name).
    #[func]
    pub fn get_parameter(&self, name: GString) -> f32 {
        self.player
            .as_ref()
            .and_then(|p| p.parameter_index(&name.to_string()).map(|i| p.parameter_value(i)))
            .unwrap_or(f32::NAN)
    }

    /// Motion names.
    #[func]
    pub fn get_motion_names(&self) -> PackedStringArray {
        self.player
            .as_ref()
            .map(|p| {
                p.model()
                    .rig
                    .motions
                    .iter()
                    .map(|m| GString::from(m.name.as_str()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Start a motion (`additive` layers it on top). False for an unknown name.
    #[func]
    pub fn play_motion(&mut self, name: GString, additive: bool) -> bool {
        let Some(player) = &mut self.player else {
            return false;
        };
        match player.motion_index(&name.to_string()) {
            Some(i) => player.play_motion(i, additive),
            None => false,
        }
    }

    /// Fade every motion out.
    #[func]
    pub fn stop_motions(&mut self) {
        if let Some(player) = &mut self.player {
            player.stop_motions();
        }
    }

    /// True while a motion plays or fades.
    #[func]
    pub fn is_motion_playing(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.is_playing())
    }

    /// Fade to an expression by name; an empty name fades out of all.
    #[func]
    pub fn set_expression(&mut self, name: GString) {
        if let Some(player) = &mut self.player {
            let index = player.expression_index(&name.to_string());
            player.set_expression(index);
        }
    }

    /// Look toward `target` (-1..1 on each axis, y up), or ahead for (0, 0)
    /// with `ahead` true.
    #[func]
    pub fn look_toward(&mut self, target: Vector2) {
        if let Some(player) = &mut self.player {
            player.look_at(Some((target.x, target.y)));
        }
    }

    /// Look straight ahead.
    #[func]
    pub fn look_ahead(&mut self) {
        if let Some(player) = &mut self.player {
            player.look_at(None);
        }
    }

    /// Voice loudness (0..1) and brightness (-1..1) for lip sync.
    #[func]
    pub fn set_audio(&mut self, level: f32, brightness: f32) {
        if let Some(player) = &mut self.player {
            player.set_audio(level, brightness);
        }
    }

    /// Feed a face-tracker sample: head angles in degrees in the tracked
    /// person's frame (yaw toward their left, pitch up, roll toward their
    /// left shoulder) and a Dictionary of ARKit/MediaPipe blend shapes
    /// (`"jawOpen": 0.4`, …).
    #[func]
    pub fn track_face(&mut self, yaw: f32, pitch: f32, roll: f32, shapes: VarDictionary) {
        let Some(player) = &mut self.player else {
            return;
        };
        let mut frame = FaceFrame {
            yaw,
            pitch,
            roll,
            ..Default::default()
        };
        for (key, value) in shapes.iter_shared() {
            if let (Ok(name), Ok(weight)) = (key.try_to::<GString>(), value.try_to::<f32>()) {
                frame.set_shape(&name.to_string(), weight);
            }
        }
        player.track_face(&frame);
    }

    /// Make the latest tracked face the neutral one.
    #[func]
    pub fn calibrate_tracking(&mut self) {
        if let Some(player) = &mut self.player {
            player.calibrate_tracking();
        }
    }

    /// Stop following the face tracker.
    #[func]
    pub fn stop_tracking(&mut self) {
        if let Some(player) = &mut self.player {
            player.stop_tracking();
        }
    }

    /// The name of the topmost part under a point in the node's local
    /// coordinates (model pixels), or an empty string.
    #[func]
    pub fn hit_test(&self, point: Vector2) -> GString {
        self.player
            .as_ref()
            .and_then(|p| {
                p.hit_test(point.x, point.y)
                    .map(|i| p.model().parts[i].name.clone())
            })
            .map(|name| GString::from(name.as_str()))
            .unwrap_or_default()
    }

    /// Back to the default pose; motions stopped, simulations at rest.
    #[func]
    pub fn reset_pose(&mut self) {
        if let Some(player) = &mut self.player {
            player.reset();
        }
        self.redraw();
    }

    /// Advance `seconds` and redraw now, whatever `playing` says.
    #[func]
    pub fn advance(&mut self, seconds: f32) {
        if let Some(player) = &mut self.player {
            player.tick(seconds);
        }
        self.redraw();
    }

    /// Recompute the pose after parameter changes and redraw, without
    /// advancing time.
    #[func]
    pub fn refresh(&mut self) {
        if let Some(player) = &mut self.player {
            player.update();
        }
        self.redraw();
    }

    /// A shader for a blend mode, shared by every part using it.
    fn shader(&mut self, blend: BlendKind) -> Gd<Shader> {
        if let Some((_, shader)) = self.shaders.iter().find(|(b, _)| *b == blend) {
            return shader.clone();
        }
        let mut shader = Shader::new_gd();
        shader.set_code(&shader_code(blend));
        self.shaders.push((blend, shader.clone()));
        shader
    }

    /// Rebuild every canvas item from the player's current draw list.
    fn redraw(&mut self) {
        let Some(player) = &self.player else {
            return;
        };
        let mut server = RenderingServer::singleton();
        let parent = self.base().get_canvas_item();
        let white = PackedColorArray::new();
        let mut shown = vec![false; self.items.len()];
        let mut clip_bases = vec![false; self.items.len()];
        for draw in player.draw_list() {
            if let Some(base) = draw.mask_part() {
                clip_bases[base] = true;
            }
        }
        for (order, draw) in player.draw_list().iter().enumerate() {
            let index = draw.part as usize;
            if draw.opacity <= 0.0 {
                continue;
            }
            // Clipped parts draw inside their base's canvas group.
            let owner = match draw.mask_part() {
                Some(base) => self.items_rid(base).unwrap_or(parent),
                None => parent,
            };
            let Some(part) = self.items.get_mut(index) else {
                continue;
            };
            shown[index] = true;
            server.canvas_item_set_parent(part.item, owner);
            server.canvas_item_set_draw_index(part.item, order as i32);
            let points: PackedVector2Array = player
                .positions(index)
                .chunks_exact(2)
                .map(|c| Vector2::new(c[0], c[1]))
                .collect();
            server.canvas_item_clear(part.item);
            if let Some(texture) = self.textures.get(part.texture) {
                server
                    .canvas_item_add_triangle_array_ex(part.item, &part.indices, &points, &white)
                    .uvs(&part.uvs)
                    .texture(texture.get_rid())
                    .done();
            }
            let material = &mut part.material;
            material.set_shader_parameter("opacity", &draw.opacity.to_variant());
            material.set_shader_parameter(
                "multiply",
                &Vector3::new(draw.multiply[0], draw.multiply[1], draw.multiply[2]).to_variant(),
            );
            material.set_shader_parameter(
                "screen",
                &Vector3::new(draw.screen[0], draw.screen[1], draw.screen[2]).to_variant(),
            );
        }
        for (index, part) in self.items.iter().enumerate() {
            server.canvas_item_set_visible(part.item, shown[index]);
            let mode = if clip_bases[index] {
                CanvasGroupMode::CLIP_AND_DRAW
            } else {
                CanvasGroupMode::DISABLED
            };
            server.canvas_item_set_canvas_group_mode(part.item, mode);
        }
    }

    fn items_rid(&self, index: usize) -> Option<Rid> {
        self.items.get(index).map(|p| p.item)
    }

    /// Free every canvas item.
    fn release(&mut self) {
        let mut server = RenderingServer::singleton();
        for part in self.items.drain(..) {
            server.free_rid(part.item);
        }
        self.player = None;
    }
}

/// An image from a path: through the resource loader when the project has
/// imported it (so exported games work), straight from the file otherwise.
fn load_image(path: &str) -> Option<Gd<Image>> {
    let mut loader = ResourceLoader::singleton();
    if loader.exists(path) {
        if let Some(resource) = loader.load(path) {
            if let Ok(texture) = resource.try_cast::<Texture2D>() {
                if let Some(image) = texture.get_image() {
                    return Some(image);
                }
            }
        }
    }
    Image::load_from_file(path)
}
