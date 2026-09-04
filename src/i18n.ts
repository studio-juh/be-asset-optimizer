export type AppLocale = "en" | "ja";

const detectedLanguage = typeof navigator === "undefined" ? "en" : navigator.language;

export const locale: AppLocale = /^ja(?:-|$)/i.test(detectedLanguage) ? "ja" : "en";
export const isJapanese = locale === "ja";

export function t(japanese: string, english: string) {
  return isJapanese ? japanese : english;
}

export function localizedDetail(value: unknown, englishFallback: string) {
  const detail = String(value);
  if (isJapanese || !/[ぁ-んァ-ヶ一-龯]/u.test(detail)) return detail;
  return englishFallback;
}

if (typeof document !== "undefined") document.documentElement.lang = locale;
