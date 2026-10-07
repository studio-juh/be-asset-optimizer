import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { localizedDetail, t } from "./i18n";
import { adjustNormalPixels } from "./normalAdjustment";

type Input = { source: string; base: ImageData; detail: ImageData | null };
async function loadImage(path: string) {
  const source = await invoke<string>("create_preview", { path, size: 512 });
  const image = new Image();
  image.src = source;
  await image.decode();
  return { source, image };
}
function pixels(image: HTMLImageElement, width: number, height: number) {
  const canvas = document.createElement("canvas");
  canvas.width = width; canvas.height = height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw Error("Canvas unavailable");
  context.drawImage(image, 0, 0, width, height);
  return context.getImageData(0, 0, width, height);
}
export default function NormalAdjustPreview({ base, detail, strength, detailStrength, flip }: { base: string; detail: string; strength: number; detailStrength: number; flip: boolean }) {
  const [input, setInput] = useState<Input | null>(null);
  const [adjusted, setAdjusted] = useState("");
  const [notice, setNotice] = useState("");
  useEffect(() => {
    let cancelled = false;
    setInput(null); setAdjusted(""); setNotice(base ? t("読み込み中…", "Loading…") : "");
    if (base) void (async () => {
      try {
        const loaded = await loadImage(base);
        if (cancelled) return;
        const extra = detail ? await loadImage(detail) : null;
        if (cancelled) return;
        const { naturalWidth: width, naturalHeight: height } = loaded.image;
        setInput({ source: loaded.source, base: pixels(loaded.image, width, height), detail: extra ? pixels(extra.image, width, height) : null });
        setNotice("");
      } catch (error) { if (!cancelled) setNotice(localizedDetail(error, "Could not load preview")); }
    })();
    return () => { cancelled = true; };
  }, [base, detail]);
  useEffect(() => {
    if (!input) return;
    // Coalesce slider changes without re-reading image files.
    const timer = window.setTimeout(() => {
      const canvas = document.createElement("canvas");
      canvas.width = input.base.width; canvas.height = input.base.height;
      const context = canvas.getContext("2d");
      if (!context) return;
      context.putImageData(new ImageData(adjustNormalPixels(input.base.data, input.detail?.data ?? null, strength, detailStrength, flip), canvas.width, canvas.height), 0, 0);
      setAdjusted(canvas.toDataURL("image/png"));
    }, 60);
    return () => window.clearTimeout(timer);
  }, [input, strength, detailStrength, flip]);
  return <div className="normal-adjust-preview">
    <div className="normal-preview-pane"><strong>{t("元画像", "Original")}</strong>{input ? <img src={input.source} alt={t("元のノーマル", "Original normal map")} /> : <span role="status">{notice || t("ベース画像を選択", "Choose a base image")}</span>}</div>
    <div className="normal-preview-pane"><strong>{t("調整後", "Adjusted")}</strong>{adjusted ? <img src={adjusted} alt={t("調整後のノーマル", "Adjusted normal map")} /> : <span>{notice}</span>}</div>
  </div>;
}
