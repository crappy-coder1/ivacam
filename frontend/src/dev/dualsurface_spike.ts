/// THROWAWAY SPIKE — dual-surface (two-sided / flip-stock) simulation.
/// bd ivac-rt1.11.1 (Phase 0 go/no-go gate). NOT wired into the app; a
/// manual visual smoke check. Delete once the go/no-go call is recorded.
///
/// QUESTION IT ANSWERS: if we machine the top with one heightfield sim and
/// the bottom with a SECOND sim fed a flipped toolpath, and render both as
/// the two faces of one box, does it read as ONE coherent solid or as two
/// floating sheets? (If stitching is ugly, a real voxel/SDF model is needed
/// — a much bigger feature, ivac-58nl.6.)
///
/// WHAT IT DOES:
///   * Builds a synthetic single-sided FRONT job (pockets milled into the
///     top of an W×H×T stock) and carves it with a real WASM `Simulator`.
///   * Derives a BACK job by the flip transform Phase 1 will formalize
///     (mirror X about the stock centre; the Z "invert" is applied at render
///     time as a reflection, see below) and carves it with a SECOND
///     `Simulator` in its own top-down local frame.
///   * Renders FRONT as the top slab [m, 0] and BACK as the bottom slab
///     [−T, m], the back mesh reflected about the mid-plane m = −T/2 so its
///     carved surface faces down as the stock underside. The two slabs meet
///     at m and should tile the box [0,W]×[0,H]×[−T,0].
///
/// MODEL NOTE (the crux the gate probes): a `HeightfieldMesh` has a CONSTANT
/// floor, so two of them meeting at a fixed mid-plane render the true
/// 2.5D-both-sides solid { z_bot(x,y) ≤ z ≤ z_top(x,y) } EXACTLY only where
/// neither side's cut crosses m. The scene deliberately includes both a
/// "stays in its half" feature (should look coherent) and a "crosses m"
/// feature (exposes the seam artifact) so the reviewer sees the boundary of
/// the naive approach.
///
/// TO RUN:
///   1. Create `frontend/dev-dualsurface.html` (sibling of index.html):
///        <!doctype html><meta charset="utf-8"><title>dual-surface spike</title>
///        <div id="dualsurface-spike" style="position:fixed;inset:0"></div>
///        <script type="module" src="/src/dev/dualsurface_spike.ts"></script>
///   2. `pnpm dev` and browse to /dev-dualsurface.html
///   Keys: 1 top · 2 bottom · 3 iso · 4 side(seam) · o opacity · space auto-orbit.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { HeightfieldMesh } from '../lib/sim/heightfield_mesh';

// ---- Stock geometry (mm) ----------------------------------------------------
const W = 80;
const H = 60;
const T = 6; // thickness
const M = -T / 2; // mid-plane Z
const CELL = 0.4; // ~200×150 grid → smooth, cheap
const TOOL_DIA = 4;

// ---- WASM wire shapes -------------------------------------------------------
type Kind = 'rapid' | 'cut' | 'plunge' | 'retract';
interface Seg {
  from: { x: number; y: number; z: number };
  to: { x: number; y: number; z: number };
  kind: Kind;
}
const seg = (
  x0: number,
  y0: number,
  z0: number,
  x1: number,
  y1: number,
  z1: number,
  kind: Kind,
): Seg => ({ from: { x: x0, y: y0, z: z0 }, to: { x: x1, y: y1, z: z1 }, kind });

/// Minimal ToolEntry the Rust deserializer accepts — every other field has a
/// serde default. `kind` is snake_case, `coolant` lowercase.
const TOOL = {
  id: 1,
  name: 'spike endmill',
  kind: 'endmill',
  diameter: TOOL_DIA,
  flutes: 2,
  speed: 18000,
  plunge_rate: 100,
  feed_rate: 800,
  coolant: 'off',
  pause: 1,
};

const SAFE_Z = 3; // rapid/plunge clearance above the top

/// Raster a rectangular pocket to depth `z` (world Z, negative = into stock)
/// as plunge → serpentine cuts → retract. `step` is the stepover.
function pocketRect(
  out: Seg[],
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  z: number,
  step = TOOL_DIA * 0.6,
): void {
  out.push(seg(x0, y0, SAFE_Z, x0, y0, z, 'plunge'));
  let dir = 1;
  let lastX = x0;
  let y = y0;
  for (; y <= y1 + 1e-9; y += step) {
    const xs = dir > 0 ? x0 : x1;
    const xe = dir > 0 ? x1 : x0;
    out.push(seg(xs, y, z, xe, y, z, 'cut'));
    lastX = xe;
    const yn = y + step;
    if (yn <= y1 + 1e-9) out.push(seg(xe, y, z, xe, yn, z, 'cut'));
    dir = -dir;
  }
  out.push(seg(lastX, Math.min(y, y1), z, lastX, Math.min(y, y1), SAFE_Z, 'retract'));
}

// ---- FRONT job (milled into the top, depth measured down from z=0) ----------
// Feature depths chosen relative to m = −3:
//   * shallow pockets (depth 1.5 → world −1.5) STAY above m  → coherent case
//   * a deep pocket   (depth 4.5 → world −4.5) CROSSES m     → seam artifact
const DEPTH_SHALLOW = 1.5;
const DEPTH_DEEP = 4.5;

function frontJob(): Seg[] {
  const s: Seg[] = [];
  // Shallow rectangular pocket, left third — should read as a clean pocket.
  pocketRect(s, 8, 10, 30, 50, -DEPTH_SHALLOW);
  // A shallow groove across the middle.
  pocketRect(s, 34, 27, 70, 33, -DEPTH_SHALLOW);
  // Deep square pocket, right side: carved to −4.5, so it CROSSES the
  // mid-plane m = −3. The front mesh's constant floor clamps it AT m, so
  // it bottoms out at the mid-plane (revealing the back slab's uncut top
  // there — the back's mirror-X deep pocket lands on the LEFT, not here)
  // instead of at its true −4.5. This is the naive model's boundary case.
  pocketRect(s, 50, 38, 66, 52, -DEPTH_DEEP);
  return s;
}

/// Flip transform (what Phase 1 formalises): mirror X about the stock centre.
/// Y is preserved; the Z sense is handled by reflecting the whole back mesh
/// about the mid-plane at render time, so back-local depths are authored the
/// same way as the front (down from the local top). Applying the mirror to
/// the front job proves front↔back register as one flipped part.
function flipX<T extends { x: number }>(p: T): T {
  return { ...p, x: W - p.x };
}
function backJobFromFront(front: Seg[]): Seg[] {
  return front.map((g) => ({ from: flipX(g.from), to: flipX(g.to), kind: g.kind }));
}

// ---- Carve one sim, return an owned copy of its dense top surface -----------
interface WasmSim {
  set_toolpath(v: unknown): number;
  advance(tool: unknown, from: number, to: number): Uint32Array;
  cols(): number;
  rows(): number;
  data_ptr(): number;
}
interface WasmMod {
  default: () => Promise<{ memory: WebAssembly.Memory }>;
  Simulator: new (
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    cell: number,
    topZ: number,
    stockBottomZ: number,
  ) => WasmSim;
}

async function loadWasm(): Promise<{
  Simulator: WasmMod['Simulator'];
  memory: WebAssembly.Memory;
}> {
  const mod = (await import('ivac-wasm')) as unknown as WasmMod;
  const init = await mod.default();
  return { Simulator: mod.Simulator, memory: init.memory };
}

/// Carve `job` into a fresh sim and return {cols, rows, view} where view is a
/// JS-owned copy (decoupled from WASM memory growth on the next sim).
function carve(
  Simulator: WasmMod['Simulator'],
  memory: WebAssembly.Memory,
  job: Seg[],
): { cols: number; rows: number; view: Float32Array } {
  const sim = new Simulator(0, 0, W, H, CELL, 0, -T);
  sim.set_toolpath(job);
  sim.advance(TOOL, 0, job.length);
  const cols = sim.cols();
  const rows = sim.rows();
  const live = new Float32Array(memory.buffer, sim.data_ptr(), cols * rows);
  return { cols, rows, view: live.slice() }; // copy off the live heap
}

// ---- Scene ------------------------------------------------------------------
export async function mountDualSurfaceSpike(host: HTMLElement): Promise<() => void> {
  const { Simulator, memory } = await loadWasm();

  const front = frontJob();
  const back = backJobFromFront(front);
  const fCarve = carve(Simulator, memory, front);
  const bCarve = carve(Simulator, memory, back);

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x0e1116);

  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 5000);
  camera.up.set(0, 0, 1); // Z-up, matches Scene3D + the heightfield mesh
  const target = new THREE.Vector3(W / 2, H / 2, M);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  host.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.target.copy(target);

  scene.add(new THREE.AmbientLight(0xffffff, 0.55));
  const dir = new THREE.DirectionalLight(0xffffff, 0.9);
  dir.position.set(60, -80, 160);
  scene.add(dir);
  const dir2 = new THREE.DirectionalLight(0xffffff, 0.5);
  dir2.position.set(-40, 60, -120); // light the underside too
  scene.add(dir2);

  // Reference wire box of the true stock extent [0,W]×[0,H]×[−T,0] so the eye
  // can judge whether the two slabs fill it.
  const boxGeom = new THREE.BoxGeometry(W, H, T);
  const boxEdges = new THREE.LineSegments(
    new THREE.EdgesGeometry(boxGeom),
    new THREE.LineBasicMaterial({ color: 0x3a4658 }),
  );
  boxEdges.position.set(W / 2, H / 2, M);
  scene.add(boxEdges);

  const OPACITY = 0.55;
  const commonStyle = {
    solidColor: '#c8b48a',
    solidOpacity: 1.0,
    edgeColor: '#20252c',
    edgeOpacity: 1.0,
  };

  // FRONT mesh: top slab [m, 0], carved top surface.
  const frontMesh = new HeightfieldMesh({
    cols: fCarve.cols,
    rows: fCarve.rows,
    cellSize: CELL,
    originX: 0,
    originY: 0,
    topZ: 0,
    floorZ: M,
    ...commonStyle,
  });
  frontMesh.updateHeights(fCarve.view);
  frontMesh.rebuildEdges();
  scene.add(frontMesh.group);

  // BACK mesh: built in its own top-down local frame [m, 0], then reflected
  // about the mid-plane m so it becomes the bottom slab [−T, m] with its
  // carved surface facing down (the stock underside). Reflection about z = m:
  // worldZ = 2m − localZ  ⇒  scale.z = −1, position.z = 2m = −T.
  const backMesh = new HeightfieldMesh({
    cols: bCarve.cols,
    rows: bCarve.rows,
    cellSize: CELL,
    originX: 0,
    originY: 0,
    topZ: 0,
    floorZ: M,
    ...commonStyle,
    solidColor: '#b7a488',
  });
  backMesh.updateHeights(bCarve.view);
  backMesh.rebuildEdges();
  const backReflect = new THREE.Group();
  backReflect.add(backMesh.group);
  backReflect.scale.z = -1;
  backReflect.position.z = -T;
  scene.add(backReflect);

  // Camera presets keyed for repeatable screenshots.
  const R = Math.max(W, H) * 1.15;
  function view(name: string): void {
    if (name === 'top') camera.position.set(W / 2, H / 2 - 1, R);
    else if (name === 'bottom') camera.position.set(W / 2, H / 2 + 1, -R);
    else if (name === 'side')
      camera.position.set(W / 2, -R, M); // look along +Y at the seam
    else camera.position.set(W / 2 + R * 0.7, H / 2 - R * 0.8, R * 0.7); // iso
    controls.target.copy(target);
    controls.update();
  }
  view('iso');

  let translucent = false;
  function setOpacity(t: boolean): void {
    translucent = t;
    const o = t ? OPACITY : 1.0;
    frontMesh.setStyle({ solidOpacity: o });
    backMesh.setStyle({ solidOpacity: o });
  }

  let autoOrbit = false;
  let raf = 0;
  function tick(): void {
    if (autoOrbit) {
      const a = performance.now() * 0.0003;
      camera.position.set(W / 2 + Math.cos(a) * R, H / 2 + Math.sin(a) * R, R * 0.6);
      controls.target.copy(target);
    }
    controls.update();
    renderer.render(scene, camera);
    raf = requestAnimationFrame(tick);
  }

  function fit(): void {
    const w = host.clientWidth || 1;
    const h = host.clientHeight || 1;
    renderer.setSize(w, h);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }
  const ro = new ResizeObserver(fit);
  ro.observe(host);
  fit();

  function onKey(e: KeyboardEvent): void {
    if (e.key === '1') view('top');
    else if (e.key === '2') view('bottom');
    else if (e.key === '3') view('iso');
    else if (e.key === '4') view('side');
    else if (e.key === 'o') setOpacity(!translucent);
    else if (e.key === ' ') autoOrbit = !autoOrbit;
  }
  window.addEventListener('keydown', onKey);
  raf = requestAnimationFrame(tick);

  // Expose a tiny handle so a headless driver can flip views/opacity via
  // evaluate_script without synthesising key events.
  (window as unknown as Record<string, unknown>).__spike = {
    view,
    setOpacity,
    toggleOrbit: () => (autoOrbit = !autoOrbit),
    stats: { front: fCarve, back: bCarve, W, H, T, M, CELL },
  };

  return () => {
    cancelAnimationFrame(raf);
    window.removeEventListener('keydown', onKey);
    ro.disconnect();
    controls.dispose();
    frontMesh.dispose();
    backMesh.dispose();
    renderer.dispose();
    if (host.contains(renderer.domElement)) host.removeChild(renderer.domElement);
  };
}

if (typeof document !== 'undefined') {
  const el = document.getElementById('dualsurface-spike');
  if (el) void mountDualSurfaceSpike(el);
}
