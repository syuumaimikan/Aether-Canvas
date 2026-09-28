// Type declarations for aether-tracking.js.

import type { AetherModel, AetherPlayer } from './aether-player.js';

/** Where MediaPipe is fetched from unless told otherwise. */
export const DEFAULTS: { vision: string; wasm: string; model: string };

/** A face-mesh landmark in MediaPipe's normalised image coordinates. */
export interface Landmark {
  x: number;
  y: number;
  z?: number;
}

/** Head angles in degrees, in the tracked person's frame (yaw toward their left, pitch up, roll toward their left shoulder). */
export function headPose(landmarks: Landmark[], aspect?: number): { yaw: number; pitch: number; roll: number };

export interface FaceTrackingOptions {
  /** Video to track (default: ask for the front camera). */
  stream?: MediaStream;
  /** Element to play it in (default: a hidden one). */
  video?: HTMLVideoElement;
  /** Move like the person's mirror image (default true). */
  mirror?: boolean;
  /** Take the first face seen as neutral (default 'auto'). */
  calibrate?: 'auto' | 'manual';
  delegate?: 'GPU' | 'CPU';
  /** MediaPipe tasks-vision module, or its URL (relative URLs are relative to the page). */
  vision?: string | object;
  /** URL of MediaPipe's wasm directory. */
  wasm?: string;
  /** URL of face_landmarker.task. */
  model?: string;
}

export interface FaceTracking {
  readonly video: HTMLVideoElement;
  readonly stream: MediaStream;
  readonly landmarker: unknown;
  readonly state: { frames: number; faces: number; pose: { yaw: number; pitch: number; roll: number } | null };
  /** Take the current face as neutral. */
  calibrate(): void;
  /** Stop tracking; the model returns to its own animation. */
  stop(): void;
}

export function startFaceTracking(target: AetherPlayer | AetherModel, options?: FaceTrackingOptions): Promise<FaceTracking>;
