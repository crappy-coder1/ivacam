/// Translucent stock box + its wireframe. Always visible (not only in sim
/// mode) whenever an import is loaded and both `stock.visible` and
/// `settings.showStockBox` are on. The XY footprint comes from the shared
/// `computeFootprint` (auto = bbox + margin; manual = customX/Y centered on
/// the bbox); Z extents are `[offsetZ − thickness, offsetZ]`.
///
/// Extracted from Scene3D.svelte. Owns its THREE.Group.

import * as THREE from 'three';
import type { ImportResponse } from '../api/types';
import type { AxisLimits, StockConfig } from '../state/project.svelte';
import { computeFootprint } from '../sim/driver';
import { disposeGroup } from './dispose';
import { computeFlipGizmo } from './flip_gizmo';
import type { Builder, BuilderContext, CssColor } from './builder';

export interface StockBoxInput {
  stock: StockConfig;
  showStockBox: boolean;
  imported: ImportResponse | null;
  workArea: AxisLimits | undefined;
}

export class StockBoxBuilder implements Builder {
  readonly group = new THREE.Group();

  constructor(
    private ctx: BuilderContext,
    private cssColor: CssColor,
  ) {
    ctx.scene.add(this.group);
  }

  build(input: StockBoxInput) {
    disposeGroup(this.group);
    const cfg = input.stock;
    if (!cfg.visible || !input.showStockBox) return;
    // Stock-first: render the stock even without a drawing (falls back to
    // the machine work-area inside computeFootprint).
    const fp = computeFootprint(input.imported, cfg, input.workArea);
    const sizeX = fp.maxX - fp.minX;
    const sizeY = fp.maxY - fp.minY;
    const thickness = Math.max(0.1, cfg.thickness);
    if (sizeX <= 0.1 || sizeY <= 0.1) return;

    const cx = (fp.minX + fp.maxX) * 0.5;
    const cy = (fp.minY + fp.maxY) * 0.5;
    const topZ = cfg.offsetZ ?? 0;
    // Stock top sits at offsetZ (default 0); box centered half a
    // thickness below it, so it spans [offsetZ − thickness, offsetZ].
    const cz = topZ - thickness * 0.5;
    const box = new THREE.BoxGeometry(sizeX, sizeY, thickness);
    const fillMat = new THREE.MeshBasicMaterial({
      transparent: true,
      opacity: 0.05,
      // Theme-tracking neutral so the stock fill is visible against both
      // the dark and light backdrops. `--stock-edge` is the matching
      // outline token (used a few lines below).
      color: this.cssColor('--stock-edge', 0xcccccc),
      side: THREE.DoubleSide,
      depthWrite: false,
    });
    const fill = new THREE.Mesh(box, fillMat);
    fill.position.set(cx, cy, cz);
    this.group.add(fill);

    const edges = new THREE.EdgesGeometry(box);
    const lineMat = new THREE.LineBasicMaterial({
      color: this.cssColor('--stock-edge', 0x888888),
      transparent: true,
      opacity: 0.4,
    });
    const wire = new THREE.LineSegments(edges, lineMat);
    wire.position.set(cx, cy, cz);
    this.group.add(wire);

    // Two-sided flip gizmo: hinge line + roll arrow over the top face, so
    // the flip axis (the error-prone choice — wrong axis = mirrored scrap)
    // is unmistakable in the scene. Only when a flip is configured.
    if (cfg.flip) this.addFlipGizmo(cfg.flip.axis, fp, topZ);
  }

  /// Amber hinge line along the flip axis + a 180° roll arc arcing over the
  /// top and plunging an arrowhead down the far edge — the geometry is the
  /// pure `computeFlipGizmo`; this just skins it as meshes. `MeshBasicMaterial`
  /// (unlit) keeps it a flat, legible overlay like the stock wire.
  private addFlipGizmo(
    axis: 'x' | 'y',
    fp: { minX: number; minY: number; maxX: number; maxY: number },
    topZ: number,
  ) {
    const g = computeFlipGizmo({ ...fp, topZ, axis });
    const mat = new THREE.MeshBasicMaterial({
      color: this.cssColor('--warn', 0xffd23a),
      transparent: true,
      opacity: 0.9,
      side: THREE.DoubleSide,
    });
    // Tube radius scales with the gizmo so it reads on both a coaster and a
    // full sheet, with a floor so it never vanishes.
    const r = Math.max(0.4, g.spanHalf * 0.03);

    // Hinge: a cylinder from a→b (CylinderGeometry runs along +Y by default).
    const a = new THREE.Vector3(g.hinge.a.x, g.hinge.a.y, g.hinge.a.z);
    const b = new THREE.Vector3(g.hinge.b.x, g.hinge.b.y, g.hinge.b.z);
    const hingeLen = a.distanceTo(b);
    const hinge = new THREE.Mesh(new THREE.CylinderGeometry(r, r, hingeLen, 12), mat);
    hinge.position.copy(a).add(b).multiplyScalar(0.5);
    hinge.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), b.clone().sub(a).normalize());
    this.group.add(hinge);

    // Roll arc: a tube swept along the semicircle polyline.
    const pts = g.arc.map((p) => new THREE.Vector3(p.x, p.y, p.z));
    const curve = new THREE.CatmullRomCurve3(pts);
    const tube = new THREE.TubeGeometry(curve, g.arc.length, r, 10, false);
    this.group.add(new THREE.Mesh(tube, mat));

    // Arrowhead: a cone whose apex touches the far edge, pointing down along
    // the arc tangent — the head sits above the top plane (its body grows back
    // up the arc) so it reads as "pressing down onto the far edge" without
    // poking through thin stock. ConeGeometry's apex is +Y·(h/2) from centre.
    const dir = new THREE.Vector3(g.arrow.dir.x, g.arrow.dir.y, g.arrow.dir.z).normalize();
    const coneH = r * 7;
    const cone = new THREE.Mesh(new THREE.ConeGeometry(r * 3, coneH, 14), mat);
    cone.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
    cone.position
      .set(g.arrow.tip.x, g.arrow.tip.y, g.arrow.tip.z)
      .sub(dir.clone().multiplyScalar(coneH * 0.5));
    this.group.add(cone);
  }

  dispose() {
    disposeGroup(this.group);
    this.ctx.scene.remove(this.group);
  }
}
