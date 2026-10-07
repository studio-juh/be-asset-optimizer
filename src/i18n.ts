import { storePreviewLanguage } from "./storePreview";

export type AppLocale = "en" | "ja";

const detectedLanguage = storePreviewLanguage ?? (typeof navigator === "undefined" ? "en" : navigator.language);

const savedLanguage = typeof localStorage === "undefined" ? null : localStorage.getItem("be-asset-optimizer-language-v1");
export let locale: AppLocale = storePreviewLanguage === "ja" || storePreviewLanguage === "en"
  ? storePreviewLanguage
  : savedLanguage === "ja" || savedLanguage === "en" ? savedLanguage
  : /^ja(?:-|$)/i.test(detectedLanguage) ? "ja" : "en";
export let isJapanese = locale === "ja";

export function setLocale(value: AppLocale) {
  locale = value;
  isJapanese = value === "ja";
  localStorage.setItem("be-asset-optimizer-language-v1", value);
  document.documentElement.lang = value;
}

export function t(japanese: string, english: string) {
  return isJapanese ? japanese : english;
}

export function localizedDetail(value: unknown, englishFallback: string) {
  const detail = String(value);
  if (isJapanese || !/[ぁ-んァ-ヶ一-龯]/u.test(detail)) return detail;
  return englishFallback;
}

if (typeof document !== "undefined") document.documentElement.lang = locale;
