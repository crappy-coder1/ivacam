/// Reactive pointer-hover feedback state for EntityCanvas2D (ivac-1hxn
/// slice): which imported object / text layer the cursor is over (drives
/// the hover halo + the grab-cursor affordance) and the cursor's world
/// position (the on-canvas coordinate HUD). Before this these were three
/// loose `$state` fields in the component; giving them an owning module
/// mirrors the ViewController precedent (view-controller.svelte.ts) and
/// gives future hover logic a home OUTSIDE the .svelte file, so the
/// component can't silently regrow.
///
/// A thin rune wrapper with no fragile math — so, like ViewController, it
/// needs no separate unit test.

/// Cursor world (data-space) coordinates, in mm.
export interface DataPoint {
  x: number;
  y: number;
}

export class HoverState {
  /// Hit-index slot of the imported object under the cursor, or null.
  objectIdx = $state<number | null>(null);
  /// Id of the text layer whose stroke the cursor is over, or null.
  textId = $state<number | null>(null);
  /// Cursor world coordinates for the HUD; null before the first import +
  /// move, and whenever the pointer is off-canvas.
  cursor = $state<DataPoint | null>(null);

  /// Update the hovered entity, assigning only on change so an unrelated
  /// redraw doesn't churn the reactive graph (matches the original guarded
  /// writes in the pointer-move hover branch).
  setEntity(objectIdx: number | null, textId: number | null): void {
    if (objectIdx !== this.objectIdx) this.objectIdx = objectIdx;
    if (textId !== this.textId) this.textId = textId;
  }

  /// Clear all hover feedback — the pointer left the canvas.
  clear(): void {
    this.objectIdx = null;
    this.textId = null;
    this.cursor = null;
  }
}
