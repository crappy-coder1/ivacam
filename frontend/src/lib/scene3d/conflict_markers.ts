/// Two-sided conflict markers — one red diamond per cluster of columns
/// where the finished front carve and the reflected back carve overlap (or
/// cut clean through). A deliberately DISTINCT shape (octahedron) + error
/// color from the sim-warning tetrahedra (`warning_markers.ts`), so a
/// static two-sided geometry conflict never reads as a runtime tool-path
/// warning. Rebuilt whenever a two-sided Generate changes.
///
/// Extracted from Scene3D.svelte and modelled on `WarningMarkersBuilder`:
/// owns its THREE.Group, rebuilds from plain `ConflictMarker` data the host
/// hands it, and sizes each marker off `sceneRadius` so it reads at any
/// zoom.

import * as THREE from 'three';
import type { ConflictMarker } from '../sim/two_sided_conflict';
import { disposeGroup } from './dispose';
import type { Builder, BuilderContext, CssColor } from './builder';

export interface ConflictMarkersInput {
  markers: ConflictMarker[];
  sceneRadius: number;
}

export class ConflictMarkersBuilder implements Builder {
  readonly group = new THREE.Group();

  constructor(
    private ctx: BuilderContext,
    private cssColor: CssColor,
  ) {
    ctx.scene.add(this.group);
  }

  build(input: ConflictMarkersInput) {
    disposeGroup(this.group);
    if (input.markers.length === 0) return;
    // Slightly larger than the sim-warning tetrahedra (×0.012) so a
    // conflict reads as the more serious, geometry-level problem.
    const radius = Math.max(0.6, input.sceneRadius * 0.016);
    const geom = new THREE.OctahedronGeometry(radius, 0);
    const color = this.cssColor('--error', 0xe54848);
    for (const m of input.markers) {
      const mat = new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: 0.92,
        // The collision sits inside the solid — draw the marker on top so
        // it's visible through the carved stock rather than buried.
        depthTest: false,
      });
      const mesh = new THREE.Mesh(geom, mat);
      mesh.position.set(m.x, m.y, m.z);
      mesh.renderOrder = 3;
      this.group.add(mesh);
    }
  }

  dispose() {
    disposeGroup(this.group);
    this.ctx.scene.remove(this.group);
  }
}
