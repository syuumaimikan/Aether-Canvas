// Type declarations for aether-player.js.

export interface ParameterInfo {
  index: number;
  name: string;
  min: number;
  max: number;
  default: number;
}

export interface MotionInfo {
  index: number;
  name: string;
  /** Seconds. */
  duration: number;
}

export interface PartInfo {
  index: number;
  name: string;
  /** Texture page. */
  texture: number;
  vertexCount: number;
  /** Texture coordinates, 0..1, x/y pairs. */
  uvs: Float32Array;
  /** Triangle vertex indices. */
  indices: Uint32Array;
}

export interface DrawItem {
  part: number;
  /** Part whose coverage clips this one, or -1. */
  mask: number;
  /** 0 normal, 1 multiply, 2 screen, 3 add. */
  blend: number;
  opacity: number;
  maskOpacity: number;
  multiply: [number, number, number];
  screen: [number, number, number];
}

export interface MotionEvent {
  name: string;
  /** Name of the motion that fired it. */
  motion: string | number;
}

export type Stage = 'motions' | 'behaviours' | 'drivers' | 'physics' | 'jiggle';

/** A key that plays a motion or switches an expression, set up in the editor. */
export interface HotkeyInfo {
  index: number;
  /** Its name, or the motion or expression it plays. */
  name: string;
  /** The keys, written like "Shift+1". */
  keys: string;
}

export interface KeyModifiers {
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
}

/**
 * One face-tracker sample. Angles are degrees in the tracked person's frame:
 * yaw toward their left, pitch up, roll toward their left shoulder.
 */
export interface FaceSample {
  yaw?: number;
  pitch?: number;
  roll?: number;
  /** MediaPipe categories, { name: weight }, or weights in runtime.blendshapes order. */
  shapes?: { categoryName: string; score: number }[] | Record<string, number> | ArrayLike<number>;
}

export interface TrackingOptions {
  /** Move like a mirror image of the person (default true). */
  mirror?: boolean;
  /** Smoothing time constant, seconds. */
  smoothing?: number;
  headGain?: number;
  bodyFollow?: number;
  mouthGain?: number;
}

/** The WebAssembly module. One runtime can host any number of models. */
export class AetherRuntime {
  /** Blend shape names, in the order face samples carry them (ARKit / MediaPipe naming). */
  readonly blendshapes: string[];
  static instantiate(
    source?: BufferSource | Response | Promise<Response> | URL | string,
  ): Promise<AetherRuntime>;
  /** Load a model from the text of model.json. Throws with the reason on failure. */
  createModel(json: string): AetherModel;
  lastError(): string;
}

/** One loaded model. */
export class AetherModel {
  readonly runtime: AetherRuntime;
  /** Canvas size in document pixels. */
  readonly width: number;
  readonly height: number;
  readonly textureFiles: string[];
  readonly parameters: ParameterInfo[];
  readonly motions: MotionInfo[];
  readonly expressions: string[];
  readonly hotkeys: HotkeyInfo[];
  readonly parts: PartInfo[];
  readonly playing: boolean;

  dispose(): void;
  parameterIndex(nameOrIndex: string | number): number;
  /** Set a parameter's base value; motions, behaviours and physics layer on top. */
  setParameter(nameOrIndex: string | number, value: number): void;
  /** A parameter's value as last drawn (NaN for an unknown name). */
  parameter(nameOrIndex: string | number): number;
  playMotion(nameOrIndex: string | number, options?: { additive?: boolean }): boolean;
  stopMotions(): void;
  /** Fade to an expression, or out of all of them with null. */
  setExpression(nameOrIndex: string | number | null): void;
  /** Switch an expression on or off, leaving the others. */
  toggleExpression(nameOrIndex: string | number): void;
  expressionActive(nameOrIndex: string | number): boolean;
  /** Carry out a hotkey (by index, name or keys) as if its key were pressed. */
  triggerHotkey(which: number | string): boolean;
  /** A key was pressed ("1", "F5", or a KeyboardEvent code like "Digit1"); returns the hotkey that fired. */
  pressKey(key: string, modifiers?: KeyModifiers): HotkeyInfo | null;
  /** Look towards (x, y) in -1..1, y up; null looks ahead. */
  lookAt(x: number | null, y?: number): void;
  /** Voice loudness 0..1 and brightness -1..1, for lip sync. */
  setAudio(level: number, brightness?: number): void;
  setStage(stage: Stage | number, enabled: boolean): void;
  /** Feed one face-tracker sample; auto-blink pauses while tracking. */
  trackFace(sample: FaceSample): void;
  /** Make the latest tracked face the neutral one. */
  calibrateTracking(): void;
  stopTracking(): void;
  readonly tracking: boolean;
  setTrackingOptions(options: TrackingOptions): void;
  /** Advance `dt` seconds. Returns the motion events passed. */
  tick(dt: number): MotionEvent[];
  /** Recompute the pose without advancing time. */
  update(): void;
  reset(): void;
  /** Posed positions in document pixels; a view into module memory, valid until the next tick. */
  positions(part: number): Float32Array;
  drawList(): DrawItem[];
  hitTest(x: number, y: number): PartInfo | null;
}

/** Draws an AetherModel with WebGL 1 or 2. */
export class WebGLRenderer {
  constructor(gl: WebGLRenderingContext | WebGL2RenderingContext, model: AetherModel, images: TexImageSource[]);
  readonly gl: WebGLRenderingContext | WebGL2RenderingContext;
  /** Document rectangle to show, or null to fit the whole canvas. */
  camera: { x: number; y: number; width: number; height: number } | null;
  /** Clear colour [r, g, b, a] (straight alpha, 0..1), or null for transparent. */
  background: [number, number, number, number] | null;
  view(): { scale: number; x: number; y: number };
  /** Map drawing-buffer pixels (y down) to document space. */
  toDocument(x: number, y: number): [number, number];
  render(): void;
  dispose(): void;
}

export interface LoadOptions {
  wasm?: BufferSource | URL | string;
  runtime?: AetherRuntime;
  contextAttributes?: WebGLContextAttributes;
}

/** A model on a canvas, with a frame loop and input helpers. */
export class AetherPlayer {
  static load(canvas: HTMLCanvasElement, modelUrl: string | URL, options?: LoadOptions): Promise<AetherPlayer>;
  static fromFiles(canvas: HTMLCanvasElement, files: Iterable<File>, options?: LoadOptions): Promise<AetherPlayer>;

  readonly canvas: HTMLCanvasElement;
  readonly model: AetherModel;
  readonly renderer: WebGLRenderer;
  /** Playback speed multiplier. */
  speed: number;
  readonly parameters: ParameterInfo[];
  readonly motions: MotionInfo[];
  readonly expressions: string[];
  readonly hotkeys: HotkeyInfo[];

  setParameter(nameOrIndex: string | number, value: number): void;
  parameter(nameOrIndex: string | number): number;
  playMotion(nameOrIndex: string | number, options?: { additive?: boolean }): boolean;
  stopMotions(): void;
  setExpression(nameOrIndex: string | number | null): void;
  toggleExpression(nameOrIndex: string | number): void;
  expressionActive(nameOrIndex: string | number): boolean;
  triggerHotkey(which: number | string): boolean;
  pressKey(key: string, modifiers?: KeyModifiers): HotkeyInfo | null;
  lookAt(x: number | null, y?: number): void;
  setStage(stage: Stage | number, enabled: boolean): void;
  reset(): void;

  on(type: 'hotkey', callback: (hotkey: HotkeyInfo) => void): () => void;
  on(type: 'event', callback: (event: MotionEvent) => void): () => void;
  on(type: 'hit', callback: (hit: { part: string; index: number; x: number; y: number }) => void): () => void;
  on(type: 'frame', callback: (dt: number) => void): () => void;

  /** Match the drawing buffer to the canvas's CSS size and pixel ratio. */
  resize(): void;
  /** Advance `dt` seconds and draw. */
  step(dt: number): void;
  start(): void;
  stop(): void;
  documentPoint(event: { clientX: number; clientY: number }): [number, number];
  /** Look at the pointer. Returns a function that stops. */
  followPointer(options?: { element?: EventTarget; center?: [number, number]; reach?: number }): () => void;
  /** Play motions and switch expressions with the model's hotkeys. Returns a function that stops. */
  listenForHotkeys(options?: { element?: EventTarget }): () => void;
  /** Emit 'hit' for taps on parts. Returns a function that stops. */
  enableHitTest(): () => void;
  /** Record the canvas as video (WebM where supported); `stop()` resolves to the file. */
  record(options?: { fps?: number; mimeType?: string; bitsPerSecond?: number }): {
    stop(): Promise<Blob>;
    mimeType: string;
  };
  /** Lip sync from an <audio>/<video> element or a MediaStream. Returns a function that stops. */
  lipSync(source: HTMLMediaElement | MediaStream, options?: { gain?: number }): () => void;
  dispose(): void;
}
