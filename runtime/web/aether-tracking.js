// Webcam face tracking for the Aether web player.
//
// Runs Google's MediaPipe Face Landmarker on a camera (or any video stream),
// turns each frame into head angles and the standard 52 blend shapes, and
// feeds them to a model. The mapping onto parameters happens in the player
// (aether_player::tracking), so every host moves a model the same way.
//
//   import { AetherPlayer } from './aether-player.js';
//   import { startFaceTracking } from './aether-tracking.js';
//   const player = await AetherPlayer.load(canvas, 'model/model.json');
//   player.start();
//   const tracking = await startFaceTracking(player);   // asks for the camera
//   …
//   tracking.calibrate();   // "this is my neutral face"
//   tracking.stop();
//
// MediaPipe is loaded on demand from a CDN by default; pass `vision`, `wasm`
// and `model` to self-host it. Video never leaves the browser.

const MEDIAPIPE = '0.10.14';

/** Where MediaPipe is fetched from unless told otherwise. */
export const DEFAULTS = {
  vision: `https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@${MEDIAPIPE}/vision_bundle.mjs`,
  wasm: `https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@${MEDIAPIPE}/wasm`,
  model: 'https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task',
};

// Points of MediaPipe's 478-point face mesh. "Left" and "right" are the
// person's own: their right eye and cheek appear on the left of the image.
const EYE_OUTER_RIGHT = 33;
const EYE_OUTER_LEFT = 263;
const CHEEK_RIGHT = 234;
const CHEEK_LEFT = 454;
const FOREHEAD = 10;
const CHIN = 152;

/**
 * Head angles in degrees, in the tracked person's frame (yaw toward their
 * left, pitch up, roll toward their left shoulder), from face-mesh landmarks
 * in MediaPipe's normalised image coordinates. `aspect` is the image's
 * width / height.
 *
 * MediaPipe gives x as a fraction of the width, y of the height, and z (depth,
 * smaller = nearer the camera) on the same scale as x, so everything is first
 * brought to one unit. Each angle then comes from a pair of points whose
 * relationship it changes, measured against their distance so the other
 * rotations barely matter:
 *   roll  — the eye line's slope (their left eye dropping = tip to the left)
 *   yaw   — the cheeks' depth difference (their left cheek receding = turn left)
 *   pitch — forehead and chin depth (forehead receding = looking up)
 */
export function headPose(landmarks, aspect = 1) {
  const point = (i) => {
    const l = landmarks[i];
    return [l.x * aspect, l.y, (l.z ?? 0) * aspect];
  };
  const deg = 180 / Math.PI;
  const across = (a, b) => Math.hypot(b[0] - a[0], b[1] - a[1]);
  const eyeR = point(EYE_OUTER_RIGHT);
  const eyeL = point(EYE_OUTER_LEFT);
  const roll = Math.atan2(eyeL[1] - eyeR[1], eyeL[0] - eyeR[0]) * deg;
  const cheekR = point(CHEEK_RIGHT);
  const cheekL = point(CHEEK_LEFT);
  const yaw = Math.atan2(cheekL[2] - cheekR[2], across(cheekR, cheekL)) * deg;
  const top = point(FOREHEAD);
  const chin = point(CHIN);
  const pitch = Math.atan2(top[2] - chin[2], across(top, chin)) * deg;
  return { yaw, pitch, roll };
}

/**
 * Track a face and drive a model with it.
 *
 * @param {object} target An AetherPlayer or AetherModel.
 * @param {object} [options]
 * @param {MediaStream} [options.stream] Video to track (default: ask for the front camera).
 * @param {HTMLVideoElement} [options.video] Element to play it in (default: a hidden one).
 * @param {boolean} [options.mirror=true] Move like the person's mirror image.
 * @param {'auto'|'manual'} [options.calibrate='auto'] Take the first face seen as neutral.
 * @param {'GPU'|'CPU'} [options.delegate='GPU'] Where MediaPipe runs its network.
 * @param {string|object} [options.vision] MediaPipe tasks-vision module, or its URL.
 * @param {string} [options.wasm] URL of MediaPipe's wasm directory.
 * @param {string} [options.model] URL of face_landmarker.task.
 */
export async function startFaceTracking(target, options = {}) {
  const model = target.model ?? target;
  const {
    vision = DEFAULTS.vision,
    wasm = DEFAULTS.wasm,
    model: modelAsset = DEFAULTS.model,
    delegate = 'GPU',
    mirror = true,
    calibrate = 'auto',
  } = options;

  // Relative URLs are relative to the page, as they are for the other two.
  const { FilesetResolver, FaceLandmarker } =
    typeof vision === 'string' ? await import(new URL(vision, document.baseURI).href) : vision;
  const fileset = await FilesetResolver.forVisionTasks(wasm);
  const landmarker = await FaceLandmarker.createFromOptions(fileset, {
    baseOptions: { modelAssetPath: modelAsset, delegate },
    runningMode: 'VIDEO',
    numFaces: 1,
    outputFaceBlendshapes: true,
  });

  const ownStream = !options.stream;
  const stream =
    options.stream ??
    (await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: { facingMode: 'user', width: { ideal: 640 }, height: { ideal: 480 } },
    }));
  const video = options.video ?? document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  await video.play();

  model.setTrackingOptions({ mirror });
  const state = { frames: 0, faces: 0, pose: null };
  let calibrated = calibrate !== 'auto';
  let running = true;
  let lastTime = -1;
  let pending = 0;
  const byVideoFrame = typeof video.requestVideoFrameCallback === 'function';

  const step = () => {
    if (!running) return;
    if (video.readyState >= 2 && video.currentTime !== lastTime) {
      lastTime = video.currentTime;
      const result = landmarker.detectForVideo(video, performance.now());
      state.frames++;
      const landmarks = result.faceLandmarks?.[0];
      if (landmarks) {
        state.faces++;
        state.pose = headPose(landmarks, video.videoWidth / video.videoHeight);
        model.trackFace({ ...state.pose, shapes: result.faceBlendshapes?.[0]?.categories ?? [] });
        if (!calibrated) {
          model.calibrateTracking();
          calibrated = true;
        }
      }
    }
    pending = byVideoFrame ? video.requestVideoFrameCallback(step) : requestAnimationFrame(step);
  };
  step();

  return {
    video,
    stream,
    landmarker,
    /** Frames looked at, faces found, and the latest head pose. */
    state,
    /** Take the current face as neutral. */
    calibrate() {
      model.calibrateTracking();
    },
    /** Stop tracking; the model returns to its own animation. */
    stop() {
      if (!running) return;
      running = false;
      if (byVideoFrame) video.cancelVideoFrameCallback(pending);
      else cancelAnimationFrame(pending);
      model.stopTracking();
      if (ownStream) stream.getTracks().forEach((t) => t.stop());
      landmarker.close();
    },
  };
}
