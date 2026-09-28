//! Live2D physics (`physics3.json`), simulated as the Cubism Framework does.
//!
//! Each setting is a strand of particles hanging from a root that the input
//! parameters move (sideways, up and down) and turn. The strand swings under
//! gravity with delay, acceleration and mobility, keeps each particle at its
//! radius from the one above, and its outputs write parameters from the
//! swing angle of chosen particles. Steps run at the file's `Fps` (or once
//! per frame without one), and outputs are interpolated between the last
//! two steps.
//!
//! This follows the Framework's arithmetic, quirks included (a rotation that
//! reuses its freshly rotated x, and X/Y outputs that the Framework never
//! scales, so they write 0), so a model moves as it does in Live2D's own
//! apps; `tests/cubism_physics.rs` holds it to the Framework's output.

use serde_json::Value;

const AIR_RESISTANCE: f64 = 5.0;
const MAXIMUM_WEIGHT: f64 = 100.0;
const MOVEMENT_THRESHOLD: f64 = 0.001;
const MAX_DELTA_TIME: f64 = 5.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct V2 {
    x: f64,
    y: f64,
}

impl V2 {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
    fn scale(self, s: f64) -> V2 {
        V2::new(self.x * s, self.y * s)
    }
    fn div(self, s: f64) -> V2 {
        V2::new(self.x / s, self.y / s)
    }
    fn normalized(self) -> V2 {
        let length = (self.x * self.x + self.y * self.y).sqrt();
        V2::new(self.x / length, self.y / length)
    }
}

fn direction_to_radian(from: V2, to: V2) -> f64 {
    let q1 = to.y.atan2(to.x);
    let q2 = from.y.atan2(from.x);
    let mut r = q1 - q2;
    while r < -std::f64::consts::PI {
        r += std::f64::consts::PI * 2.0;
    }
    while r > std::f64::consts::PI {
        r -= std::f64::consts::PI * 2.0;
    }
    r
}

fn degrees_to_radian(d: f64) -> f64 {
    d / 180.0 * std::f64::consts::PI
}

fn radian_to_direction(r: f64) -> V2 {
    V2::new(r.sin(), r.cos())
}

/// What an input moves or an output reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsKind {
    /// Sideways.
    X,
    /// Up and down.
    Y,
    /// Rotation.
    Angle,
}

impl PhysicsKind {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("X") => PhysicsKind::X,
            Some("Y") => PhysicsKind::Y,
            _ => PhysicsKind::Angle,
        }
    }
}

#[derive(Clone, Debug)]
struct Input {
    id: String,
    index: Option<usize>,
    weight: f64,
    kind: PhysicsKind,
    reflect: bool,
}

#[derive(Clone, Debug)]
struct Output {
    id: String,
    index: Option<usize>,
    vertex: i64,
    scale: f64,
    weight: f64,
    kind: PhysicsKind,
    reflect: bool,
}

#[derive(Clone, Debug, Default)]
struct Particle {
    mobility: f64,
    delay: f64,
    acceleration: f64,
    radius: f64,
    initial: V2,
    position: V2,
    last_position: V2,
    last_gravity: V2,
    velocity: V2,
    force: V2,
}

#[derive(Clone, Copy, Debug, Default)]
struct Normalization {
    minimum: f64,
    maximum: f64,
    default: f64,
}

#[derive(Clone, Debug)]
struct Setting {
    name: String,
    inputs: Vec<Input>,
    outputs: Vec<Output>,
    particles: Vec<Particle>,
    position: Normalization,
    angle: Normalization,
    current: Vec<f64>,
    previous: Vec<f64>,
}

/// The parameters physics reads and writes, index-aligned with the model.
pub struct Parameters<'a> {
    /// Current values; outputs are written here.
    pub values: &'a mut [f32],
    /// Minima.
    pub min: &'a [f32],
    /// Maxima.
    pub max: &'a [f32],
    /// Defaults.
    pub default: &'a [f32],
}

/// A model's physics, with its simulation state.
#[derive(Clone, Debug)]
pub struct Physics {
    settings: Vec<Setting>,
    fps: f64,
    /// Gravity direction (the Framework's option; (0, -1)).
    pub gravity: [f64; 2],
    /// Wind (the Framework's option; none).
    pub wind: [f64; 2],
    remain: f64,
    caches: Vec<f32>,
    input_caches: Vec<f32>,
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

impl Physics {
    /// Parse a `physics3.json`.
    pub fn from_json(json: &Value) -> Option<Physics> {
        let meta = json.get("Meta")?;
        let fps = num(meta.get("Fps"));
        let names: Vec<(String, String)> = meta
            .get("PhysicsDictionary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some((
                    e.get("Id")?.as_str()?.to_string(),
                    e.get("Name")?.as_str()?.to_string(),
                ))
            })
            .collect();
        let mut settings = Vec::new();
        for s in json.get("PhysicsSettings")?.as_array()? {
            let id = s.get("Id").and_then(Value::as_str).unwrap_or("");
            let name = names
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| id.to_string());
            let inputs = s
                .get("Input")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|i| Input {
                    id: i
                        .pointer("/Source/Id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    index: None,
                    weight: num(i.get("Weight")),
                    kind: PhysicsKind::parse(i.get("Type").and_then(Value::as_str)),
                    reflect: i.get("Reflect").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect();
            let outputs: Vec<Output> = s
                .get("Output")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|o| {
                    let kind = PhysicsKind::parse(o.get("Type").and_then(Value::as_str));
                    Output {
                        id: o
                            .pointer("/Destination/Id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        index: None,
                        vertex: o.get("VertexIndex").and_then(Value::as_i64).unwrap_or(0),
                        // The Framework reads Scale only as an angle scale;
                        // translation outputs keep a scale of zero.
                        scale: if kind == PhysicsKind::Angle {
                            num(o.get("Scale"))
                        } else {
                            0.0
                        },
                        weight: num(o.get("Weight")),
                        kind,
                        reflect: o.get("Reflect").and_then(Value::as_bool).unwrap_or(false),
                    }
                })
                .collect();
            let particles = s
                .get("Vertices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|v| Particle {
                    mobility: num(v.get("Mobility")),
                    delay: num(v.get("Delay")),
                    acceleration: num(v.get("Acceleration")),
                    radius: num(v.get("Radius")),
                    position: V2::new(num(v.pointer("/Position/X")), num(v.pointer("/Position/Y"))),
                    ..Default::default()
                })
                .collect();
            let norm = |key: &str| Normalization {
                minimum: num(s.pointer(&format!("/Normalization/{key}/Minimum"))),
                maximum: num(s.pointer(&format!("/Normalization/{key}/Maximum"))),
                default: num(s.pointer(&format!("/Normalization/{key}/Default"))),
            };
            let n = outputs.len();
            settings.push(Setting {
                name,
                inputs,
                outputs,
                particles,
                position: norm("Position"),
                angle: norm("Angle"),
                current: vec![0.0; n],
                previous: vec![0.0; n],
            });
        }
        let mut physics = Physics {
            settings,
            fps,
            gravity: [0.0, -1.0],
            wind: [0.0, 0.0],
            remain: 0.0,
            caches: Vec::new(),
            input_caches: Vec::new(),
        };
        physics.initialize();
        Some(physics)
    }

    fn initialize(&mut self) {
        for s in &mut self.settings {
            let Some(first) = s.particles.first_mut() else {
                continue;
            };
            first.initial = V2::default();
            first.last_position = first.initial;
            first.last_gravity = V2::new(0.0, 1.0);
            first.velocity = V2::default();
            first.force = V2::default();
            for i in 1..s.particles.len() {
                let initial = s.particles[i - 1]
                    .initial
                    .add(V2::new(0.0, s.particles[i].radius));
                let p = &mut s.particles[i];
                p.initial = initial;
                p.position = initial;
                p.last_position = initial;
                p.last_gravity = V2::new(0.0, 1.0);
                p.velocity = V2::default();
                p.force = V2::default();
            }
        }
    }

    /// Setting names (from the file's dictionary), for display.
    pub fn setting_names(&self) -> Vec<String> {
        self.settings.iter().map(|s| s.name.clone()).collect()
    }

    /// Every parameter id the physics reads or writes.
    pub fn parameter_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .settings
            .iter()
            .flat_map(|s| {
                s.inputs
                    .iter()
                    .map(|i| i.id.clone())
                    .chain(s.outputs.iter().map(|o| o.id.clone()))
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Resolve parameter ids against a model's parameter list; unknown ids
    /// are skipped.
    pub fn bind(&mut self, ids: &[String]) {
        let find = |id: &str| ids.iter().position(|p| p == id);
        for s in &mut self.settings {
            for i in &mut s.inputs {
                i.index = find(&i.id);
            }
            for o in &mut s.outputs {
                o.index = find(&o.id);
            }
        }
        self.caches.clear();
        self.input_caches.clear();
    }

    /// Back to the rest state (call [`Physics::stabilize`] next).
    pub fn reset(&mut self) {
        self.remain = 0.0;
        self.caches.clear();
        self.input_caches.clear();
        for s in &mut self.settings {
            s.current.iter_mut().for_each(|v| *v = 0.0);
            s.previous.iter_mut().for_each(|v| *v = 0.0);
        }
        self.initialize();
    }

    /// Settle every strand for the current parameters at once (what the
    /// Framework does when a model loads) and write the outputs.
    pub fn stabilize(&mut self, p: &mut Parameters) {
        let n = p.values.len();
        self.caches = p.values.to_vec();
        self.input_caches = p.values.to_vec();
        let (gravity, wind) = (
            V2::new(self.gravity[0], self.gravity[1]),
            V2::new(self.wind[0], self.wind[1]),
        );
        for s in &mut self.settings {
            let (translation, angle) = total_input(s, |i| p.values[i], p);
            stabilize_particles(
                &mut s.particles,
                translation,
                angle,
                wind,
                MOVEMENT_THRESHOLD * s.position.maximum,
            );
            for (k, o) in s.outputs.iter().enumerate() {
                let Some(value) = output_value(o, &s.particles, gravity) else {
                    continue;
                };
                s.current[k] = value;
                s.previous[k] = value;
                if let Some(dest) = o.index.filter(|&d| d < n) {
                    write_output(&mut p.values[dest], p.min[dest], p.max[dest], value, o);
                    for j in dest..n {
                        self.caches[j] = p.values[j];
                    }
                }
            }
        }
    }

    /// Advance `dt` seconds with the current parameter values as input and
    /// write the outputs.
    pub fn evaluate(&mut self, p: &mut Parameters, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let n = p.values.len();
        self.remain += dt;
        if self.remain > MAX_DELTA_TIME {
            self.remain = 0.0;
        }
        if self.caches.len() < n {
            self.caches = vec![0.0; n];
        }
        if self.input_caches.len() < n {
            self.input_caches = p.values.to_vec();
        }
        let step = if self.fps > 0.0 { 1.0 / self.fps } else { dt };
        let (gravity, wind) = (
            V2::new(self.gravity[0], self.gravity[1]),
            V2::new(self.wind[0], self.wind[1]),
        );
        while self.remain >= step {
            for s in &mut self.settings {
                s.previous.clone_from(&s.current);
            }
            let input_weight = step / self.remain;
            for j in 0..n {
                let v =
                    self.input_caches[j] as f64 * (1.0 - input_weight) + p.values[j] as f64 * input_weight;
                self.caches[j] = v as f32;
                self.input_caches[j] = self.caches[j];
            }
            for s in &mut self.settings {
                let caches = &self.caches;
                let (translation, angle) = total_input(s, |i| caches[i], p);
                update_particles(
                    &mut s.particles,
                    translation,
                    angle,
                    wind,
                    MOVEMENT_THRESHOLD * s.position.maximum,
                    step,
                );
                for (k, o) in s.outputs.iter().enumerate() {
                    let Some(value) = output_value(o, &s.particles, gravity) else {
                        continue;
                    };
                    s.current[k] = value;
                    if let Some(dest) = o.index.filter(|&d| d < n) {
                        write_output(&mut self.caches[dest], p.min[dest], p.max[dest], value, o);
                    }
                }
            }
            self.remain -= step;
        }
        let alpha = self.remain / step;
        for s in &self.settings {
            for (k, o) in s.outputs.iter().enumerate() {
                let Some(dest) = o.index.filter(|&d| d < n) else {
                    continue;
                };
                let value = s.previous[k] * (1.0 - alpha) + s.current[k] * alpha;
                write_output(&mut p.values[dest], p.min[dest], p.max[dest], value, o);
            }
        }
    }
}

/// The strand root's translation and angle from a setting's inputs.
fn total_input(s: &Setting, value: impl Fn(usize) -> f32, p: &Parameters) -> (V2, f64) {
    let mut translation = V2::default();
    let mut angle = 0.0f64;
    for input in &s.inputs {
        let Some(i) = input.index.filter(|&i| i < p.min.len()) else {
            continue;
        };
        let weight = input.weight / MAXIMUM_WEIGHT;
        let norm = match input.kind {
            PhysicsKind::Angle => s.angle,
            _ => s.position,
        };
        let v = normalize(
            value(i) as f64,
            p.min[i] as f64,
            p.max[i] as f64,
            norm.minimum,
            norm.maximum,
            norm.default,
            input.reflect,
        ) * weight;
        match input.kind {
            PhysicsKind::X => translation.x += v,
            PhysicsKind::Y => translation.y += v,
            PhysicsKind::Angle => angle += v,
        }
    }
    let rad = degrees_to_radian(-angle);
    translation.x = translation.x * rad.cos() - translation.y * rad.sin();
    // The Framework rotates y with the x it has just rotated.
    translation.y = translation.x * rad.sin() + translation.y * rad.cos();
    (translation, angle)
}

fn normalize(value: f64, pmin: f64, pmax: f64, nmin: f64, nmax: f64, ndefault: f64, inverted: bool) -> f64 {
    let max_value = pmax.max(pmin);
    let min_value = pmax.min(pmin);
    let value = value.min(max_value).max(min_value);
    let min_norm = nmin.min(nmax);
    let max_norm = nmin.max(nmax);
    let middle_norm = ndefault;
    let middle = min_value + (max_value - min_value).abs() / 2.0;
    let param = value - middle;
    let mut result = 0.0;
    if param > 0.0 {
        let n = max_norm - middle_norm;
        let pl = max_value - middle;
        if pl != 0.0 {
            result = param * (n / pl) + middle_norm;
        }
    } else if param < 0.0 {
        let n = min_norm - middle_norm;
        let pl = min_value - middle;
        if pl != 0.0 {
            result = param * (n / pl) + middle_norm;
        }
    } else {
        result = middle_norm;
    }
    if inverted {
        result
    } else {
        -result
    }
}

fn update_particles(strand: &mut [Particle], translation: V2, angle: f64, wind: V2, threshold: f64, dt: f64) {
    let Some(first) = strand.first_mut() else { return };
    first.position = translation;
    let gravity = radian_to_direction(degrees_to_radian(angle)).normalized();
    for i in 1..strand.len() {
        let previous = strand[i - 1].position;
        let p = &mut strand[i];
        p.force = gravity.scale(p.acceleration).add(wind);
        p.last_position = p.position;
        let delay = p.delay * dt * 30.0;
        let mut direction = p.position.sub(previous);
        let radian = direction_to_radian(p.last_gravity, gravity) / AIR_RESISTANCE;
        direction.x = radian.cos() * direction.x - direction.y * radian.sin();
        direction.y = radian.sin() * direction.x + direction.y * radian.cos();
        p.position = previous.add(direction);
        let velocity = p.velocity.scale(delay);
        let force = p.force.scale(delay).scale(delay);
        p.position = p.position.add(velocity).add(force);
        let new_direction = p.position.sub(previous).normalized();
        p.position = previous.add(new_direction.scale(p.radius));
        if p.position.x.abs() < threshold {
            p.position.x = 0.0;
        }
        if delay != 0.0 {
            p.velocity = p.position.sub(p.last_position).div(delay).scale(p.mobility);
        }
        p.force = V2::default();
        p.last_gravity = gravity;
    }
}

fn stabilize_particles(strand: &mut [Particle], translation: V2, angle: f64, wind: V2, threshold: f64) {
    let Some(first) = strand.first_mut() else { return };
    first.position = translation;
    let gravity = radian_to_direction(degrees_to_radian(angle)).normalized();
    for i in 1..strand.len() {
        let previous = strand[i - 1].position;
        let p = &mut strand[i];
        p.force = gravity.scale(p.acceleration).add(wind);
        p.last_position = p.position;
        p.velocity = V2::default();
        let force = p.force.normalized().scale(p.radius);
        p.position = previous.add(force);
        if p.position.x.abs() < threshold {
            p.position.x = 0.0;
        }
        p.force = V2::default();
        p.last_gravity = gravity;
    }
}

fn output_value(o: &Output, particles: &[Particle], gravity: V2) -> Option<f64> {
    let i = o.vertex;
    if i < 1 || i as usize >= particles.len() {
        return None;
    }
    let i = i as usize;
    let translation = particles[i].position.sub(particles[i - 1].position);
    let value = match o.kind {
        PhysicsKind::X => translation.x,
        PhysicsKind::Y => translation.y,
        PhysicsKind::Angle => {
            let parent = if i >= 2 {
                particles[i - 1].position.sub(particles[i - 2].position)
            } else {
                gravity.scale(-1.0)
            };
            direction_to_radian(parent, translation)
        }
    };
    Some(if o.reflect { -value } else { value })
}

fn write_output(param: &mut f32, min: f32, max: f32, translation: f64, o: &Output) {
    let mut value = translation * o.scale;
    if value < min as f64 {
        value = min as f64;
    } else if value > max as f64 {
        value = max as f64;
    }
    let weight = o.weight / MAXIMUM_WEIGHT;
    if weight >= 1.0 {
        *param = value as f32;
    } else {
        *param = (*param as f64 * (1.0 - weight) + value * weight) as f32;
    }
}
