// Aether Canvas web player.
//
// Plays runtime models exported from Aether Canvas (model.json + texture
// PNGs) in any browser with WebGL. Rigging, physics, motions and behaviours
// run in WebAssembly — the very same Rust code as the editor, so a model
// moves identically everywhere — and this file draws the result.
//
//   import { AetherPlayer } from './aether-player.js';
//   const player = await AetherPlayer.load(canvas, 'model/model.json');
//   player.playMotion('Idle');
//   player.followPointer();
//   player.start();
//
// Three layers, usable separately:
//   AetherRuntime  — the WebAssembly module (no DOM; works in Node too)
//   AetherModel    — one loaded model: parameters, motions, pose, draw list
//   WebGLRenderer  — draws an AetherModel with WebGL 1 or 2
//   AetherPlayer   — all of the above plus a frame loop and input helpers

const DRAW_ITEM_BYTES = 44;
const STAGES = { motions: 0, behaviours: 1, drivers: 2, physics: 3, jiggle: 4 };
const BLEND = { normal: 0, multiply: 1, screen: 2, add: 3 };

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function defaultWasmUrl() {
  return new URL('./aether_player.wasm', import.meta.url);
}

/** The WebAssembly module. One runtime can host any number of models. */
export class AetherRuntime {
  /**
   * @param {BufferSource | Response | Promise<Response> | URL | string} [source]
   *   The module bytes, a fetch response, or a URL (default: next to this file).
   */
  static async instantiate(source = defaultWasmUrl()) {
    let instance;
    if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
      ({ instance } = await WebAssembly.instantiate(source, {}));
    } else {
      const response = await (typeof source === 'string' || source instanceof URL ? fetch(source) : source);
      if (!response.ok) throw new Error(`could not load the player module (${response.status})`);
      if (WebAssembly.instantiateStreaming && response.headers.get('content-type') === 'application/wasm') {
        ({ instance } = await WebAssembly.instantiateStreaming(response, {}));
      } else {
        ({ instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {}));
      }
    }
    return new AetherRuntime(instance);
  }

  constructor(instance) {
    this.exports = instance.exports;
    if (this.exports.aether_api_version() !== 1) {
      throw new Error(`unsupported player module (API ${this.exports.aether_api_version()})`);
    }
    // Scratch space for out-parameters (string lengths, parameter ranges).
    this.scratch = this.exports.aether_alloc(16);
  }

  /** Load a model from the text of model.json. */
  createModel(json) {
    const bytes = encoder.encode(json);
    const e = this.exports;
    const ptr = e.aether_alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    const handle = e.aether_player_new(ptr, bytes.length);
    e.aether_dealloc(ptr, bytes.length);
    if (!handle) throw new Error(this.lastError() || 'could not load the model');
    return new AetherModel(this, handle);
  }

  lastError() {
    return this.readString(this.exports.aether_last_error(this.scratch));
  }

  /** Read a string returned together with its length in `scratch`. */
  readString(ptr) {
    if (!ptr) return '';
    const len = new Uint32Array(this.exports.memory.buffer, this.scratch, 1)[0];
    return decoder.decode(new Uint8Array(this.exports.memory.buffer, ptr, len));
  }

  f32(ptr, count) {
    return new Float32Array(this.exports.memory.buffer, ptr, count);
  }

  u32(ptr, count) {
    return new Uint32Array(this.exports.memory.buffer, ptr, count);
  }
}

/** One loaded model. */
export class AetherModel {
  constructor(runtime, handle) {
    this.runtime = runtime;
    this.handle = handle;
    const e = runtime.exports;
    const h = handle;
    const s = runtime.scratch;
    const list = (count, name) => Array.from({ length: count(h) }, (_, i) => runtime.readString(name(h, i, s)));

    this.width = e.aether_player_width(h);
    this.height = e.aether_player_height(h);
    this.textureFiles = list(e.aether_player_texture_count, e.aether_player_texture_file);
    this.parameters = list(e.aether_player_parameter_count, e.aether_player_parameter_name).map((name, index) => {
      e.aether_player_parameter_range(h, index, s);
      const [min, max, defaultValue] = runtime.f32(s, 3);
      return { index, name, min, max, default: defaultValue };
    });
    this.motions = list(e.aether_player_motion_count, e.aether_player_motion_name).map((name, index) => ({
      index,
      name,
      duration: e.aether_player_motion_duration(h, index),
    }));
    this.expressions = list(e.aether_player_expression_count, e.aether_player_expression_name);
    this.parts = list(e.aether_player_part_count, e.aether_player_part_name).map((name, index) => {
      const vertices = e.aether_player_part_vertex_count(h, index);
      const indices = e.aether_player_part_index_count(h, index);
      return {
        index,
        name,
        texture: e.aether_player_part_texture(h, index),
        vertexCount: vertices,
        // Copies: these never change, and GPU upload wants its own buffers.
        uvs: runtime.f32(e.aether_player_part_uvs(h, index), vertices * 2).slice(),
        indices: runtime.u32(e.aether_player_part_indices(h, index), indices).slice(),
      };
    });
    this.byName = new Map(this.parameters.map((p) => [p.name, p.index]));
  }

  /** Release the model's memory. */
  dispose() {
    if (this.handle) this.runtime.exports.aether_player_free(this.handle);
    this.handle = 0;
  }

  parameterIndex(nameOrIndex) {
    return typeof nameOrIndex === 'number' ? nameOrIndex : this.byName.get(nameOrIndex) ?? -1;
  }

  /** Set a parameter's base value (by name or index). Motions, behaviours and physics layer on top. */
  setParameter(nameOrIndex, value) {
    const i = this.parameterIndex(nameOrIndex);
    if (i >= 0) this.runtime.exports.aether_player_set_parameter(this.handle, i, value);
  }

  /** A parameter's value as last drawn. */
  parameter(nameOrIndex) {
    const i = this.parameterIndex(nameOrIndex);
    return i >= 0 ? this.runtime.exports.aether_player_parameter_value(this.handle, i) : NaN;
  }

  /** Start a motion by name or index. `additive` layers it on top of the current one. */
  playMotion(nameOrIndex, { additive = false } = {}) {
    const i = typeof nameOrIndex === 'number' ? nameOrIndex : this.motions.findIndex((m) => m.name === nameOrIndex);
    return i >= 0 && this.runtime.exports.aether_player_play_motion(this.handle, i, additive ? 1 : 0) === 1;
  }

  stopMotions() {
    this.runtime.exports.aether_player_stop_motions(this.handle);
  }

  get playing() {
    return this.runtime.exports.aether_player_is_playing(this.handle) === 1;
  }

  /** Fade to an expression by name or index, or out of all of them with null. */
  setExpression(nameOrIndex) {
    const i =
      nameOrIndex == null ? -1 : typeof nameOrIndex === 'number' ? nameOrIndex : this.expressions.indexOf(nameOrIndex);
    this.runtime.exports.aether_player_set_expression(this.handle, i);
  }

  /** Look towards (x, y) in -1..1, y up; null looks ahead. */
  lookAt(x, y) {
    const e = this.runtime.exports;
    if (x == null) e.aether_player_look_ahead(this.handle);
    else e.aether_player_look_at(this.handle, x, y);
  }

  /** Voice loudness 0..1 and brightness -1..1, for lip sync. */
  setAudio(level, brightness = 0) {
    this.runtime.exports.aether_player_set_audio(this.handle, level, brightness);
  }

  /** Switch a stage ('motions', 'behaviours', 'drivers', 'physics', 'jiggle') on or off. */
  setStage(stage, enabled) {
    this.runtime.exports.aether_player_set_stage(this.handle, STAGES[stage] ?? stage, enabled ? 1 : 0);
  }

  /** Advance `dt` seconds. Returns the motion events passed: [{ name, motion }]. */
  tick(dt) {
    const e = this.runtime.exports;
    e.aether_player_tick(this.handle, dt);
    const events = [];
    const count = e.aether_player_event_count(this.handle);
    for (let i = 0; i < count; i++) {
      const name = this.runtime.readString(e.aether_player_event_name(this.handle, i, this.runtime.scratch));
      const motion = e.aether_player_event_motion(this.handle, i);
      events.push({ name, motion: this.motions[motion]?.name ?? motion });
    }
    return events;
  }

  /** Recompute the pose without advancing time. */
  update() {
    this.runtime.exports.aether_player_update(this.handle);
  }

  /** Back to the default pose, motions stopped, simulations at rest. */
  reset() {
    this.runtime.exports.aether_player_reset(this.handle);
  }

  /** A part's posed positions (document pixels, x/y pairs). A view into module memory: copy to keep. */
  positions(part) {
    const e = this.runtime.exports;
    return this.runtime.f32(e.aether_player_part_positions(this.handle, part), this.parts[part].vertexCount * 2);
  }

  /** The draw calls for the current pose, back to front. */
  drawList() {
    const e = this.runtime.exports;
    const count = e.aether_player_draw_count(this.handle);
    const ptr = e.aether_player_draw_items(this.handle);
    const view = new DataView(e.memory.buffer, ptr, count * DRAW_ITEM_BYTES);
    const items = new Array(count);
    for (let i = 0; i < count; i++) {
      const o = i * DRAW_ITEM_BYTES;
      const f = (k) => view.getFloat32(o + k, true);
      items[i] = {
        part: view.getUint32(o, true),
        mask: view.getInt32(o + 4, true),
        blend: view.getUint32(o + 8, true),
        opacity: f(12),
        maskOpacity: f(16),
        multiply: [f(20), f(24), f(28)],
        screen: [f(32), f(36), f(40)],
      };
    }
    return items;
  }

  /** The topmost visible part under a document-space point, or null. */
  hitTest(x, y) {
    const i = this.runtime.exports.aether_player_hit_test(this.handle, x, y);
    return i >= 0 ? this.parts[i] : null;
  }
}

// ---------------------------------------------------------------- rendering

const VERTEX_SHADER = `
attribute vec2 a_position;
attribute vec2 a_uv;
uniform vec4 u_view; // scale x, scale y, offset x, offset y: document px -> clip
varying vec2 v_uv;
void main() {
  v_uv = a_uv;
  gl_Position = vec4(a_position * u_view.xy + u_view.zw, 0.0, 1.0);
}`;

const PART_SHADER = `
precision highp float;
uniform sampler2D u_texture;
uniform sampler2D u_mask;
uniform float u_use_mask;
uniform vec2 u_target;
uniform float u_opacity;
uniform vec3 u_multiply;
uniform vec3 u_screen;
varying vec2 v_uv;
void main() {
  vec4 c = texture2D(u_texture, v_uv); // premultiplied
  vec3 rgb = c.rgb * u_multiply;
  rgb = rgb + u_screen * c.a - rgb * u_screen;
  float k = u_opacity;
  if (u_use_mask > 0.5) k *= texture2D(u_mask, gl_FragCoord.xy / u_target).a;
  gl_FragColor = vec4(rgb, c.a) * k;
}`;

const MASK_SHADER = `
precision highp float;
uniform sampler2D u_texture;
uniform float u_opacity;
varying vec2 v_uv;
void main() {
  gl_FragColor = vec4(0.0, 0.0, 0.0, texture2D(u_texture, v_uv).a * u_opacity);
}`;

function compile(gl, vertex, fragment) {
  const shader = (type, source) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, source);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const program = gl.createProgram();
  gl.attachShader(program, shader(gl.VERTEX_SHADER, vertex));
  gl.attachShader(program, shader(gl.FRAGMENT_SHADER, fragment));
  gl.bindAttribLocation(program, 0, 'a_position');
  gl.bindAttribLocation(program, 1, 'a_uv');
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
  const uniforms = {};
  const count = gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS);
  for (let i = 0; i < count; i++) {
    const name = gl.getActiveUniform(program, i).name;
    uniforms[name] = gl.getUniformLocation(program, name);
  }
  return { program, uniforms };
}

/** Draws an AetherModel with WebGL. */
export class WebGLRenderer {
  /**
   * @param {WebGLRenderingContext | WebGL2RenderingContext} gl
   *   A context created with `premultipliedAlpha: true` (the default).
   * @param {AetherModel} model
   * @param {(TexImageSource)[]} images  Texture pages, in model.textureFiles order.
   */
  constructor(gl, model, images) {
    this.gl = gl;
    this.model = model;
    const webgl2 = typeof WebGL2RenderingContext !== 'undefined' && gl instanceof WebGL2RenderingContext;
    const wideIndices = webgl2 || gl.getExtension('OES_element_index_uint');
    this.part = compile(gl, VERTEX_SHADER, PART_SHADER);
    this.mask = compile(gl, VERTEX_SHADER, MASK_SHADER);

    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
    gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    this.textures = images.map((image) => {
      const t = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, t);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return t;
    });

    this.meshes = model.parts.map((part) => {
      const uv = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, uv);
      gl.bufferData(gl.ARRAY_BUFFER, part.uvs, gl.STATIC_DRAW);
      const position = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, position);
      gl.bufferData(gl.ARRAY_BUFFER, part.vertexCount * 8, gl.DYNAMIC_DRAW);
      const index = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, index);
      const small = part.vertexCount <= 65536;
      if (!small && !wideIndices) throw new Error(`part ${part.name} needs 32-bit indices`);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, small ? Uint16Array.from(part.indices) : part.indices, gl.STATIC_DRAW);
      return { uv, position, index, count: part.indices.length, type: small ? gl.UNSIGNED_SHORT : gl.UNSIGNED_INT };
    });

    this.maskTarget = null;
    /** Document-space rectangle shown, or null to fit the whole canvas. */
    this.camera = null;
    /** Clear colour [r, g, b, a] (straight), or null to keep transparency. */
    this.background = null;
  }

  /** How document pixels map to the drawing buffer: { scale, x, y } in buffer pixels. */
  view() {
    const gl = this.gl;
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const r = this.camera ?? { x: 0, y: 0, width: this.model.width, height: this.model.height };
    const scale = Math.min(w / r.width, h / r.height);
    return { scale, x: (w - r.width * scale) / 2 - r.x * scale, y: (h - r.height * scale) / 2 - r.y * scale };
  }

  /** Map a point in drawing-buffer pixels (y down) to document space. */
  toDocument(bx, by) {
    const v = this.view();
    return [(bx - v.x) / v.scale, (by - v.y) / v.scale];
  }

  ensureMaskTarget(w, h) {
    const gl = this.gl;
    if (this.maskTarget && this.maskTarget.w === w && this.maskTarget.h === h) return this.maskTarget;
    if (this.maskTarget) {
      gl.deleteTexture(this.maskTarget.texture);
      gl.deleteFramebuffer(this.maskTarget.fbo);
    }
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    const fbo = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.maskTarget = { texture, fbo, w, h };
    return this.maskTarget;
  }

  bindMesh(part) {
    const gl = this.gl;
    const mesh = this.meshes[part];
    gl.bindBuffer(gl.ARRAY_BUFFER, mesh.position);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, mesh.uv);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, mesh.index);
    return mesh;
  }

  /** Draw the model's current pose into the canvas (or the bound framebuffer). */
  render() {
    const gl = this.gl;
    const model = this.model;
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const v = this.view();
    // Document px -> clip space, y flipped.
    const view = [(2 * v.scale) / w, (-2 * v.scale) / h, (2 * v.x) / w - 1, 1 - (2 * v.y) / h];

    // Upload this frame's positions straight from module memory.
    for (let i = 0; i < model.parts.length; i++) {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.meshes[i].position);
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, model.positions(i));
    }

    gl.viewport(0, 0, w, h);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.CULL_FACE);
    gl.disable(gl.SCISSOR_TEST);
    gl.enable(gl.BLEND);
    gl.enableVertexAttribArray(0);
    gl.enableVertexAttribArray(1);
    const bg = this.background;
    gl.clearColor(bg ? bg[0] * bg[3] : 0, bg ? bg[1] * bg[3] : 0, bg ? bg[2] * bg[3] : 0, bg ? bg[3] : 0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    let maskPart = -1;
    let maskOpacity = -1;
    for (const item of model.drawList()) {
      if (item.opacity <= 0) continue;
      if (item.mask >= 0 && (item.mask !== maskPart || item.maskOpacity !== maskOpacity)) {
        this.renderMask(item.mask, item.maskOpacity, view, w, h);
        maskPart = item.mask;
        maskOpacity = item.maskOpacity;
      }
      this.renderItem(item, view, w, h);
    }
    gl.bindTexture(gl.TEXTURE_2D, null);
  }

  renderMask(part, opacity, view, w, h) {
    const gl = this.gl;
    const target = this.ensureMaskTarget(w, h);
    // Never sample the mask while drawing into it.
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, target.fbo);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(this.mask.program);
    gl.uniform4fv(this.mask.uniforms.u_view, view);
    gl.uniform1f(this.mask.uniforms.u_opacity, opacity);
    gl.uniform1i(this.mask.uniforms.u_texture, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.textures[this.model.parts[part].texture]);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    const mesh = this.bindMesh(part);
    gl.drawElements(gl.TRIANGLES, mesh.count, mesh.type, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  renderItem(item, view, w, h) {
    const gl = this.gl;
    const { program, uniforms } = this.part;
    gl.useProgram(program);
    gl.uniform4fv(uniforms.u_view, view);
    gl.uniform1f(uniforms.u_opacity, item.opacity);
    gl.uniform3fv(uniforms.u_multiply, item.multiply);
    gl.uniform3fv(uniforms.u_screen, item.screen);
    gl.uniform2f(uniforms.u_target, w, h);
    gl.uniform1f(uniforms.u_use_mask, item.mask >= 0 ? 1 : 0);
    gl.uniform1i(uniforms.u_texture, 0);
    gl.uniform1i(uniforms.u_mask, 1);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, item.mask >= 0 ? this.maskTarget.texture : null);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.textures[this.model.parts[item.part].texture]);
    const mesh = this.bindMesh(item.part);
    const draw = () => gl.drawElements(gl.TRIANGLES, mesh.count, mesh.type, 0);
    switch (item.blend) {
      case BLEND.multiply:
        // Premultiplied multiply is Cs·Cd + Cs·(1−αd) + Cd·(1−αs): two passes,
        // the first leaving alpha alone so the second still sees αd.
        gl.blendFuncSeparate(gl.DST_COLOR, gl.ONE_MINUS_SRC_ALPHA, gl.ZERO, gl.ONE);
        draw();
        gl.blendFuncSeparate(gl.ONE_MINUS_DST_ALPHA, gl.ONE, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
        draw();
        return;
      case BLEND.screen:
        gl.blendFuncSeparate(gl.ONE, gl.ONE_MINUS_SRC_COLOR, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
        break;
      case BLEND.add:
        gl.blendFuncSeparate(gl.ONE, gl.ONE, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
        break;
      default:
        gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    }
    draw();
  }

  /** Release GPU resources. */
  dispose() {
    const gl = this.gl;
    for (const m of this.meshes) [m.uv, m.position, m.index].forEach((b) => gl.deleteBuffer(b));
    this.textures.forEach((t) => gl.deleteTexture(t));
    if (this.maskTarget) {
      gl.deleteTexture(this.maskTarget.texture);
      gl.deleteFramebuffer(this.maskTarget.fbo);
    }
    gl.deleteProgram(this.part.program);
    gl.deleteProgram(this.mask.program);
  }
}

// ---------------------------------------------------------------- the player

async function loadImage(url) {
  const image = new Image();
  image.crossOrigin = 'anonymous';
  image.decoding = 'async';
  image.src = url;
  await image.decode();
  return image;
}

let sharedRuntime = null;

/** A model on a canvas, with a frame loop and input helpers. */
export class AetherPlayer {
  /**
   * Load a model onto a canvas.
   * @param {HTMLCanvasElement} canvas
   * @param {string | URL} modelUrl  URL of model.json; texture pages load from next to it.
   * @param {{ wasm?: string | URL | BufferSource, runtime?: AetherRuntime, contextAttributes?: object }} [options]
   */
  static async load(canvas, modelUrl, options = {}) {
    const runtime =
      options.runtime ??
      (options.wasm ? await AetherRuntime.instantiate(options.wasm) : (sharedRuntime ??= await AetherRuntime.instantiate()));
    const url = new URL(modelUrl, document.baseURI);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`could not load ${url} (${response.status})`);
    const model = runtime.createModel(await response.text());
    const images = await Promise.all(model.textureFiles.map((f) => loadImage(new URL(f, url))));
    return new AetherPlayer(canvas, model, images, options.contextAttributes);
  }

  /**
   * Load a model from local files (an `<input type="file" webkitdirectory>`
   * selection or a drop): model.json plus its texture pages.
   * @param {HTMLCanvasElement} canvas
   * @param {Iterable<File>} files
   */
  static async fromFiles(canvas, files, options = {}) {
    const byName = new Map([...files].map((f) => [f.name, f]));
    const json = byName.get('model.json');
    if (!json) throw new Error('model.json is missing');
    const runtime = options.runtime ?? (sharedRuntime ??= await AetherRuntime.instantiate(options.wasm));
    const model = runtime.createModel(await json.text());
    const images = await Promise.all(
      model.textureFiles.map(async (name) => {
        const file = byName.get(name.split('/').pop());
        if (!file) throw new Error(`${name} is missing`);
        const url = URL.createObjectURL(file);
        try {
          return await loadImage(url);
        } finally {
          URL.revokeObjectURL(url);
        }
      }),
    );
    return new AetherPlayer(canvas, model, images, options.contextAttributes);
  }

  constructor(canvas, model, images, contextAttributes = {}) {
    const attributes = { alpha: true, premultipliedAlpha: true, antialias: false, ...contextAttributes };
    const gl = canvas.getContext('webgl2', attributes) ?? canvas.getContext('webgl', attributes);
    if (!gl) throw new Error('WebGL is not available');
    this.canvas = canvas;
    this.model = model;
    this.renderer = new WebGLRenderer(gl, model, images);
    this.speed = 1;
    this.listeners = new Map();
    this.frame = 0;
    this.last = 0;
    this.cleanups = [];
  }

  // Model shortcuts.
  get parameters() { return this.model.parameters; }
  get motions() { return this.model.motions; }
  get expressions() { return this.model.expressions; }
  setParameter(name, value) { this.model.setParameter(name, value); }
  parameter(name) { return this.model.parameter(name); }
  playMotion(name, options) { return this.model.playMotion(name, options); }
  stopMotions() { this.model.stopMotions(); }
  setExpression(name) { this.model.setExpression(name); }
  lookAt(x, y) { this.model.lookAt(x, y); }
  setStage(stage, enabled) { this.model.setStage(stage, enabled); }
  reset() { this.model.reset(); }

  /** Subscribe: 'event' (motion events: { name, motion }), 'hit' ({ part, x, y }), 'frame' (dt). */
  on(type, callback) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(callback);
    return () => this.listeners.get(type)?.delete(callback);
  }

  emit(type, value) {
    for (const callback of this.listeners.get(type) ?? []) callback(value);
  }

  /** Match the drawing buffer to the canvas's CSS size and the display's pixel ratio. */
  resize() {
    const ratio = globalThis.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(this.canvas.clientWidth * ratio));
    const h = Math.max(1, Math.round(this.canvas.clientHeight * ratio));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
  }

  /** Advance `dt` seconds and draw. */
  step(dt) {
    for (const event of this.model.tick(dt * this.speed)) this.emit('event', event);
    this.emit('frame', dt);
    this.renderer.render();
  }

  /** Run the frame loop. */
  start() {
    if (this.frame) return;
    const loop = (now) => {
      const dt = this.last ? Math.min((now - this.last) / 1000, 0.1) : 0;
      this.last = now;
      if (this.canvas.clientWidth) this.resize();
      this.step(dt);
      this.frame = requestAnimationFrame(loop);
    };
    this.frame = requestAnimationFrame(loop);
  }

  /** Pause the frame loop. */
  stop() {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.last = 0;
  }

  /** Document-space point under a pointer event. */
  documentPoint(event) {
    const rect = this.canvas.getBoundingClientRect();
    const sx = this.canvas.width / rect.width;
    const sy = this.canvas.height / rect.height;
    return this.renderer.toDocument((event.clientX - rect.left) * sx, (event.clientY - rect.top) * sy);
  }

  /**
   * Make the model look at the pointer.
   * @param {{ element?: EventTarget, center?: [number, number], reach?: number }} [options]
   *   `center` is the document point the model looks out from (default: a
   *   third of the way down the canvas, where a face usually is); `reach` is
   *   how far from it, as a fraction of the canvas width, the pointer must be
   *   for a full turn.
   * @returns {() => void} Stops following and looks ahead again.
   */
  followPointer({ element = globalThis, center, reach = 0.5 } = {}) {
    const clamp = (v) => Math.max(-1, Math.min(1, v));
    const move = (event) => {
      const [x, y] = this.documentPoint(event);
      const [cx, cy] = center ?? [this.model.width / 2, this.model.height / 3];
      const span = reach * this.model.width;
      this.model.lookAt(clamp((x - cx) / span), clamp(-(y - cy) / span));
    };
    const leave = () => this.model.lookAt(null);
    const area = element === globalThis ? document.documentElement : element;
    element.addEventListener('pointermove', move);
    area.addEventListener('pointerleave', leave);
    return this.cleanup(() => {
      element.removeEventListener('pointermove', move);
      area.removeEventListener('pointerleave', leave);
      leave();
    });
  }

  /** Register teardown work; returns a function that runs it early. */
  cleanup(undo) {
    let done = false;
    const run = () => {
      if (done) return;
      done = true;
      this.cleanups = this.cleanups.filter((f) => f !== run);
      undo();
    };
    this.cleanups.push(run);
    return run;
  }

  /** Report taps and clicks on parts as 'hit' events. Returns a function that stops it. */
  enableHitTest() {
    const down = (event) => {
      const [x, y] = this.documentPoint(event);
      const part = this.model.hitTest(x, y);
      if (part) this.emit('hit', { part: part.name, index: part.index, x, y });
    };
    this.canvas.addEventListener('pointerdown', down);
    return this.cleanup(() => this.canvas.removeEventListener('pointerdown', down));
  }

  /**
   * Drive lip sync from live audio: an <audio>/<video> element or a
   * MediaStream (a microphone). Uses the same loudness and brightness
   * measures as the editor's WAV analysis. Returns a function that stops it.
   */
  lipSync(source, { gain = 1 } = {}) {
    const context = new AudioContext();
    const node =
      source instanceof MediaStream ? context.createMediaStreamSource(source) : context.createMediaElementSource(source);
    const analyser = context.createAnalyser();
    analyser.fftSize = 2048;
    node.connect(analyser);
    if (!(source instanceof MediaStream)) analyser.connect(context.destination);
    const samples = new Float32Array(analyser.fftSize);
    const span = Math.max(1, Math.round(context.sampleRate * 0.03));
    const off = this.on('frame', () => {
      analyser.getFloatTimeDomainData(samples);
      const slice = samples.subarray(samples.length - Math.min(span, samples.length));
      let sum = 0;
      let crossings = 0;
      for (let i = 0; i < slice.length; i++) {
        sum += slice[i] * slice[i];
        if (i && slice[i] >= 0 !== slice[i - 1] >= 0) crossings++;
      }
      const rms = Math.sqrt(sum / slice.length);
      const db = 20 * Math.log10(Math.max(rms, 1e-6));
      const level = Math.max(0, Math.min(1, ((db + 50) / 40) * gain));
      const zcr = (crossings * context.sampleRate) / (2 * slice.length);
      const bright = (Math.log2(Math.max(zcr, 1)) - Math.log2(300)) / (Math.log2(2500) - Math.log2(300)) * 2 - 1;
      this.model.setAudio(level, level > 0.05 ? Math.max(-1, Math.min(1, bright)) : 0);
    });
    return this.cleanup(() => {
      off();
      context.close();
      this.model.setAudio(0, 0);
    });
  }

  /** Stop, release the model and GPU resources, and remove listeners. */
  dispose() {
    this.stop();
    [...this.cleanups].forEach((f) => f());
    this.renderer.dispose();
    this.model.dispose();
  }
}
