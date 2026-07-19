/// Pure frame relationship for the two-sided (flip-stock) dual-surface
/// preview (ivac-rt1.11.4). Kept THREE-free so the reflection math is
/// unit-testable without a WebGL context (same split as `./footprint`).
///
/// The FRONT carve renders as the TOP half of the stock, `[midPlane, topZ]`.
/// The BACK program is simulated in the SAME top-down frame as the front —
/// its toolpath Z is authored own-face-relative (0 at the face, negative into
/// material, identical numeric form to a front op; see
/// `two_sided_emit.rs`) — and the resulting mesh is REFLECTED about the stock
/// mid-plane so its carved surface becomes the part underside, filling
/// `[stockBottom, midPlane]`. The two meshes tile the full thickness and meet
/// at the mid-plane.
///
/// This is the "constant floor" v1 (spike parity): both surfaces floor at the
/// mid-plane, so it is EXACT everywhere except where a single cut crosses the
/// mid-plane — that column shows a flat seam at `midPlane` instead of its true
/// depth. Resolving it needs a per-column floor (a documented fast-follow).

/// Z of the stock mid-plane: halfway between the top face (`topZ`) and the
/// bottom (`topZ − thickness`). Both the front and back meshes floor here.
export function midPlaneZ(topZ: number, thickness: number): number {
  return topZ - thickness / 2;
}

/// The THREE group offset that reflects the back mesh — built in the front's
/// top-down frame with local Z in `[midPlane, topZ]` — about the mid-plane so
/// it becomes the bottom slab. The reflection `worldZ = 2·midPlane − localZ`
/// is realized by `group.scale.z = −1` plus `group.position.z =` this value
/// (`= 2·topZ − thickness`). It maps the back's top face to the stock bottom
/// and leaves the mid-plane fixed (the shared seam).
export function backReflectionOffsetZ(topZ: number, thickness: number): number {
  return 2 * midPlaneZ(topZ, thickness);
}
