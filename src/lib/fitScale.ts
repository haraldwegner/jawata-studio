export interface Size {
  width: number;
  height: number;
}

/**
 * How much to scale a chart of natural size `natural` so it fills the room `available` (D54 (c), his words: the Workflow view "doesn't use the
 * space here. It does not get bigger"). The smaller of the two ratios, so neither side overflows; never above `max` (a chart blown up to the
 * whole screen reads as a poster) and never below `min` (so a narrow window shrinks it only so far). A chart or room with no size scales by 1.
 */
export function fitScale(natural: Size, available: Size, limits: { min: number; max: number } = { min: 0.6, max: 1.8 }): number {
  if (!(natural.width > 0) || !(natural.height > 0) || !(available.width > 0) || !(available.height > 0)) {
    return 1;
  }
  const ratio = Math.min(available.width / natural.width, available.height / natural.height);
  return Math.min(limits.max, Math.max(limits.min, ratio));
}
