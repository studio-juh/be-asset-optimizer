export type StorePreviewTab = "optimize" | "normal" | "atlas" | "preview";

const params = typeof window === "undefined" ? null : new URLSearchParams(window.location.search);

export const storePreview = import.meta.env.DEV && params?.get("storePreview") === "1";

const requestedTab = params?.get("tab");
export const storePreviewTab: StorePreviewTab = storePreview && ["optimize", "normal", "atlas", "preview"].includes(requestedTab ?? "")
  ? requestedTab as StorePreviewTab
  : "optimize";

export const storePreviewLanguage = storePreview && ["en", "ja"].includes(params?.get("lang") ?? "")
  ? params?.get("lang")
  : null;
