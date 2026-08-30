export const supportedImageExtensions = ["png", "heic", "heif"];

export const supportedImageFilter = {
  name: "画像（PNG / HEIC）",
  extensions: supportedImageExtensions,
};

export type InspectFailure = { path: string; message: string };
export type InspectResult<T> = { files: T[]; failures: InspectFailure[] };
export type InspectProgress<T> = { requestId: string; completed: number; total: number; currentName: string; file?: T; failure?: InspectFailure };

export function isSupportedImagePath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && supportedImageExtensions.includes(extension);
}
