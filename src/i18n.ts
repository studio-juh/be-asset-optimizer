import { storePreviewLanguage } from "./storePreview";
import { useState } from "react";
import catalog from "./locales.json";

export type AppLocale = "en" | "ja" | "zh-CN" | "de";
export const supportedLocales: AppLocale[] = ["en", "ja", "zh-CN", "de"];
export function detectLocale(language: string): AppLocale {
  if (/^ja(?:-|$)/i.test(language)) return "ja";
  if (/^zh(?:-|$)/i.test(language)) return "zh-CN";
  if (/^de(?:-|$)/i.test(language)) return "de";
  return "en";
}
function isLocale(value: unknown): value is AppLocale {
  return supportedLocales.includes(value as AppLocale);
}

const detectedLanguage = storePreviewLanguage ?? (typeof navigator === "undefined" ? "en" : navigator.language);

const savedLanguage = typeof localStorage === "undefined" ? null : localStorage.getItem("be-asset-optimizer-language-v1");
export let locale: AppLocale = isLocale(storePreviewLanguage)
  ? storePreviewLanguage
  : isLocale(savedLanguage) ? savedLanguage : detectLocale(detectedLanguage);
export let isJapanese = locale === "ja";

export function setLocale(value: AppLocale) {
  locale = value;
  isJapanese = value === "ja";
  localStorage.setItem("be-asset-optimizer-language-v1", value);
  document.documentElement.lang = value;
}

type Template = { key: string; values: unknown[] };
const translations: Record<string, string[]> = catalog;
export function translate(english: string, template?: Template) {
  const key = template?.key ?? english;
  const index = locale === "zh-CN" ? 0 : locale === "de" ? 1 : -1;
  const text = index < 0 ? undefined : translations[key]?.[index];
  if (!text) return english;
  return template ? text.replace(/\{(\d+)\}/g, (_, position: string) => String(template.values[Number(position)] ?? "")) : text;
}
export function t(japanese: string, english: string, template?: Template) {
  return isJapanese ? japanese : translate(english, template);
}

// Keep the message's resolver, not its translated snapshot. Language changes
// redraw existing notices without clearing inputs or restarting work.
export function useLocalizedState(initial: () => string = () => "") {
  const [resolve, setResolve] = useState<() => string>(() => initial);
  const update = (next: () => string) => setResolve(() => next);
  return [resolve(), update] as const;
}

const details: [string, string][] = [
  ["画像描画用のキャンバスを利用できません", "Canvas unavailable"],
  ["画像を指定してください", "Choose an image"],
  ["待機中", "Waiting"], ["変換中", "Converting"],
  ["ノーマルマップを生成中", "Generating normal map"],
  ["ノーマルマップを生成しました", "Normal map generated"],
  ["WebPに変換しました", "Converted to WebP"], ["JPEGに変換しました", "Converted to JPEG"],
  ["PNG-8では品質が保てないため、可逆PNGで保存しました", "Saved as lossless PNG because PNG-8 could not preserve sufficient quality"],
  ["PNGを最適化しました", "PNG optimized"],
  ["元ファイルより小さくなりませんでした", "The result was not smaller than the original"],
  ["画像を開けません", "Could not open image"], ["ファイル名を取得できません", "Could not get file name"],
  ["ファイルを確認できません", "Could not inspect file"],
  ["フォルダー階層が深いため、この先は読み込んでいません", "Folder nesting is too deep; deeper files were not loaded"],
  ["フォルダーを開けません", "Could not open folder"],
  ["プレビューを生成できませんでした", "Could not generate preview"],
  ["FBXまたはGLBファイルを選択してください", "Choose an FBX or GLB file"],
  ["3Dモデルを開けません", "Could not open 3D model"],
  ["300 MBを超える3Dモデルはプレビューできません", "3D models larger than 300 MB cannot be previewed"],
  ["3Dモデルを読み込めません", "Could not read 3D model"],
  ["3Dモデルの保存フォルダーを取得できません", "Could not locate the model folder"],
  ["テクスチャフォルダーを表示用に許可できません", "Could not allow access to the texture folder"],
  ["対応している画像ファイルを指定してください", "Choose a supported image file"],
  ["対応している画像を指定してください", "Choose a supported image"],
  ["対応している画像ファイルではありません", "Unsupported image format"],
  ["画像が指定されていません", "No image selected"], ["画像を1枚以上指定してください", "Choose at least one image"],
  ["入力元フォルダを取得できません", "Could not locate the source folder"],
  ["APNG は MVP では未対応です", "Animated PNG (APNG) is not supported"],
  ["画像を読み込めません", "Could not read image"],
  ["HEICのヘッダーを確認できません", "Could not inspect the HEIC header"],
  ["HEICの画像寸法を取得できません", "Could not get HEIC dimensions"],
  ["200 MBを超えるHEICは読み込めません", "HEIC files larger than 200 MB cannot be loaded"],
  ["HEICの画像寸法が大きすぎます", "HEIC dimensions are too large"],
  ["HEICの画素データが画像寸法と一致しません", "HEIC pixel data does not match its dimensions"],
  ["PNG-8 の色数は 2〜256 色で指定してください", "Choose 2–256 colors for PNG-8"],
  ["出力ファイル名を確保できません", "Could not allocate an output file name"],
  ["アトラスは最大16枠です", "An atlas supports up to 16 slots"],
  ["アトラスの枠配置が不正です", "Invalid atlas layout"], ["アトラスの枠が重なっています", "Atlas slots overlap"],
  ["アトラスの枠配置が不足しています", "Atlas layout has missing slots"],
  ["余白は0〜64 pxで指定してください", "Choose padding between 0 and 64 px"],
  ["1枠の解像度は16〜4096 pxで指定してください", "Choose a slot resolution between 16 and 4096 px"],
  ["空の画像は使用できません", "Empty images cannot be used"],
  ["入力画像が大きすぎます（最大64メガピクセル）", "Input image is too large (maximum 64 megapixels)"],
  ["アトラスが大きすぎます。1枠の解像度を下げてください（最大64メガピクセル）", "Atlas is too large; reduce the slot resolution (maximum 64 megapixels)"],
  ["分割枠の解像度が小さすぎます", "Split-slot resolution is too small"],
  ["枠に対して余白が大きすぎます", "Padding is too large for the slot"],
  ["入力画像の寸法が不正です", "Invalid input image dimensions"],
  ["画像が大きすぎます（最大64メガピクセル）", "Image is too large (maximum 64 megapixels)"],
  ["チャンネル指定が不正です", "Invalid source channel"],
  ["RGBAの4チャンネルを指定してください", "Specify all four RGBA channels"],
  ["解像度は16〜4096 pxです", "Resolution must be between 16 and 4096 px"],
  ["強度は0〜4で指定してください", "Strength must be between 0 and 4"],
  ["診断は最大1500枚です", "Diagnosis supports up to 1,500 images"],
];

export function localizedDetail(value: unknown, englishFallback: string) {
  const detail = value instanceof Error ? value.message : String(value);
  for (const [ja, en] of details) {
    for (const source of [ja, en, ...(translations[en] ?? [])]) {
      if (detail === source) return t(ja, en);
      if (detail.startsWith(`${source}:`)) return t(ja, en) + detail.slice(source.length);
    }
  }
  const converted = detail.match(/^(.+)をPNGに変換しました$/u);
  if (converted) return t(detail, `Converted ${converted[1]} to PNG`, { key: "Converted {0} to PNG", values: [converted[1]] });
  const readError = detail.match(/^([A-Z0-9]+)を読み込めません(:.*)?$/u);
  if (readError) return t(detail, `Could not read ${readError[1]}${readError[2] ?? ""}`, { key: "Could not read {0}{1}", values: [readError[1], readError[2] ?? ""] });
  if (isJapanese) return detail;
  if (!/[ぁ-んァ-ヶ一-龯]/u.test(detail)) return translate(detail);
  return translate(englishFallback);
}

if (typeof document !== "undefined") document.documentElement.lang = locale;
