// Same tangent-space adjustment and RNM blend as texture_tools.rs.
export function adjustNormalPixels(base: Uint8ClampedArray, detail: Uint8ClampedArray | null, strength: number, detailStrength: number, flip: boolean) {
  const output = new Uint8ClampedArray(base.length);
  const normalize = (v: number[]) => { const length = Math.max(Math.hypot(...v), 0.0001); return v.map(c => c / length); };
  const decode = (pixels: Uint8ClampedArray, i: number, scale: number) => normalize([
    (pixels[i] / 255 * 2 - 1) * scale,
    (pixels[i + 1] / 255 * 2 - 1) * scale * (flip ? -1 : 1),
    Math.max(pixels[i + 2] / 255 * 2 - 1, 0.0001),
  ]);
  for (let i = 0; i < base.length; i += 4) {
    let normal = decode(base, i, strength);
    if (detail) {
      const d = decode(detail, i, detailStrength);
      const v = [normal[0], normal[1], normal[2] + 1];
      const u = [-d[0], -d[1], d[2]];
      const dot = (v[0] * u[0] + v[1] * u[1] + v[2] * u[2]) / Math.max(v[2], 0.0001);
      normal = normalize(v.map((c, axis) => c * dot - u[axis]));
    }
    for (let axis = 0; axis < 3; axis++) output[i + axis] = Math.round((normal[axis] * 0.5 + 0.5) * 255);
    output[i + 3] = 255;
  }
  return output;
}
