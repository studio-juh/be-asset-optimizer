import { t } from "./i18n";

export const supportedImageExtensions = [
  "png", "jpg", "jpeg", "jfif", "webp", "heic", "heif",
  "bmp", "tif", "tiff", "gif", "tga", "dds",
];

export const supportedImageShortLabel = t("PNG / JPEG / WebP / HEIC などの画像", "PNG, JPEG, WebP, HEIC, and other images");

export const supportedImageFilter = {
  name: t("画像（主要な静止画形式）", "Images (common still-image formats)"),
  extensions: supportedImageExtensions,
};

export type InspectFailure = { path: string; message: string };
export type InspectResult<T> = { files: T[]; failures: InspectFailure[] };
export type InspectProgress<T> = { requestId: string; completed: number; total: number; currentName: string; file?: T; failure?: InspectFailure };

export function isSupportedImagePath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && supportedImageExtensions.includes(extension);
}
