export type Texture = { path: string; name: string; originalBytes: number; width: number; height: number };
export type AtlasSlot = { id: string; x: number; y: number; size: number; texture?: Texture };
export type ResolutionWarning = { width: number; height: number; scale: number };

// Match atlas.rs, including auto sizing and integer boundaries of subdivided slots.
export function resolutionWarnings(slots: AtlasSlot[], square: boolean, resolution: number | null, padding = 0) {
  const warnings = new Map<string, ResolutionWarning>();
  if (square && resolution !== null && (!Number.isInteger(resolution) || resolution < 16 || resolution > 4096)) return warnings;
  let cellWidth = 1, cellHeight = 1;
  for (const slot of slots) if (slot.texture) {
    cellWidth = Math.max(cellWidth, (slot.texture.width + padding * 2) * 2 / slot.size);
    cellHeight = Math.max(cellHeight, (slot.texture.height + padding * 2) * 2 / slot.size);
  }
  if (square) cellWidth = cellHeight = resolution ?? Math.max(cellWidth, cellHeight);
  for (const slot of slots) if (slot.texture && slot.texture.width > 0 && slot.texture.height > 0) {
    const width = Math.floor((slot.x + slot.size) * cellWidth / 2) - Math.floor(slot.x * cellWidth / 2) - padding * 2;
    const height = Math.floor((slot.y + slot.size) * cellHeight / 2) - Math.floor(slot.y * cellHeight / 2) - padding * 2;
    // Aspect-ratio padding alone is not low resolution: warn only if fitting needs upscaling.
    const scale = Math.min(width / slot.texture.width, height / slot.texture.height);
    if (scale > 1) warnings.set(slot.id, { width, height, scale });
  }
  return warnings;
}
export const emptyAtlas = (): AtlasSlot[] => Array.from({ length: 4 }, (_, i) => ({ id: String(i), x: i % 2 * 2, y: Math.floor(i / 2) * 2, size: 2 }));
export function splitSlot(slots: AtlasSlot[], id: string): AtlasSlot[] {
  return slots.flatMap(slot => slot.id !== id || slot.size !== 2 ? [slot] : Array.from({ length: 4 }, (_, i) => ({
    id: `${id}.${i}`, x: slot.x + i % 2, y: slot.y + Math.floor(i / 2), size: 1,
    texture: i === 0 ? slot.texture : undefined,
  })));
}
export function addTextures(slots: AtlasSlot[], files: Texture[], target?: string) {
  let next = slots.map(slot => ({ ...slot }));
  let added = 0;
  for (const texture of files) {
    let index = added === 0 && target ? next.findIndex(slot => slot.id === target) : next.findIndex(slot => !slot.texture);
    if (index < 0) {
      const large = [...next].reverse().find(slot => slot.size === 2);
      if (!large) break;
      next = splitSlot(next, large.id);
      index = next.findIndex(slot => !slot.texture);
    }
    next[index] = { ...next[index], texture }; added++;
  }
  return { slots: next, added };
}
