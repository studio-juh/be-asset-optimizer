import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import NormalMap from "./NormalMap";
import TextureAtlas from "./TextureAtlas";
import Thumbnail from "./Thumbnail";
import TextureTools from "./TextureTools";
import { InspectProgress, InspectResult, supportedImageFilter, supportedImageShortLabel } from "./imageFiles";
import { locale, localizedDetail, setLocale, t, useLocalizedState } from "./i18n";
import { storePreview, storePreviewTab, StorePreviewTab } from "./storePreview";

const ModelPreview = lazy(() => import("./ModelPreview"));
const MaterialPreview = lazy(() => import("./MaterialPreview"));
const HelpPage = lazy(() => import("./HelpPage"));

type Status = "ready" | "processing" | "done" | "skipped" | "failed";
type ResizeMode = "lanczos3" | "nearest";
type Optimization = "fast" | "safe" | "max";

type ImageJob = {
  id: string;
  path: string;
  name: string;
  originalBytes: number;
  width: number;
  height: number;
  status: Status;
  outputBytes?: number;
  outputWidth?: number;
  outputHeight?: number;
  message?: string;
  outputPath?: string;
};

type Metadata = Omit<ImageJob, "id" | "status">;
type Progress = {
  path: string;
  status: Status;
  output_bytes?: number;
  output_width?: number;
  output_height?: number;
  message?: string;
  output_path?: string;
};

type Settings = {
  outputDir: string;
  outputFormat: "png" | "webp" | "jpeg";
  quality: string;
  jpegBackground: "white" | "black";
  maxWidth: string;
  maxHeight: string;
  customMaxWidth: string;
  customMaxHeight: string;
  scalePercent: string;
  resizeMode: ResizeMode;
  colorMode: "auto" | "indexed" | "rgb24" | "rgba32" | "grayscale8";
  colors: string;
  dithering: boolean;
  optimization: Optimization;
};

const initialSettings: Settings = {
  outputDir: "",
  outputFormat: "png",
  quality: "82",
  jpegBackground: "white",
  maxWidth: "",
  maxHeight: "",
  customMaxWidth: "",
  customMaxHeight: "",
  scalePercent: "",
  resizeMode: "lanczos3",
  colorMode: "auto",
  colors: "256",
  dithering: true,
  optimization: "fast",
};

const formatBytes = (value?: number) => {
  if (value === undefined) return "—";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(2)} MB`;
};

const dimensions = (width?: number, height?: number) =>
  width && height ? `${width} × ${height}` : "—";

export default function App() {
  const [appLocale, setAppLocale] = useState(locale);
  const [helpOpen, setHelpOpen] = useState(false);
  const [activeTab, setActiveTab] = useState<StorePreviewTab>(storePreviewTab);
  const [visitedTabs, setVisitedTabs] = useState<Set<StorePreviewTab>>(() => new Set([storePreviewTab]));
  const activeTabRef = useRef(activeTab);
  const [displayScale, setDisplayScale] = useState(() => {
    const saved = Number(localStorage.getItem("smartpng-display-scale-v2"));
    return [0.8, 0.9, 1, 1.1, 1.25].includes(saved) ? saved : 1;
  });
  const [theme, setTheme] = useState<"light" | "dark">(() => localStorage.getItem("smartpng-theme-v1") === "dark" ? "dark" : "light");
  const [jobs, setJobs] = useState<ImageJob[]>([]);
  const [settings, setSettings] = useState<Settings>(initialSettings);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const loadingRequestId = useRef("");
  const [notice, setNotice] = useLocalizedState(() => t(`${supportedImageShortLabel()}をここへドロップしてください`, `Drop ${supportedImageShortLabel()} here`, { key: "Drop {0} here", values: [supportedImageShortLabel()] }));

  useEffect(() => {
    if (storePreview) {
      document.title = t("Be Asset Optimizer — Blender向けアセット最適化", "Be Asset Optimizer — Blender Asset Optimization");
      return;
    }
    void invoke("set_ui_locale", { locale: appLocale });
    void getCurrentWindow().setTitle(t("Be Asset Optimizer — Blender向けアセット最適化", "Be Asset Optimizer — Blender Asset Optimization"));
  }, [appLocale]);

  useEffect(() => {
    if (storePreview) return;
    const unlistenProgress = listen<Progress>("job-progress", (event) => {
      const update = event.payload;
      setJobs((current) => current.map((job) => job.path === update.path ? {
        ...job,
        status: update.status,
        outputBytes: update.output_bytes,
        outputWidth: update.output_width,
        outputHeight: update.output_height,
        message: update.message,
        outputPath: update.output_path,
      } : job));
    });
    const unlistenInspect = listen<InspectProgress<Metadata>>("inspect-files-progress", (event) => {
      const update = event.payload;
      if (update.requestId !== loadingRequestId.current) return;
      if (update.file) setJobs((current) => current.some((job) => job.path.toLowerCase() === update.file!.path.toLowerCase())
        ? current : [...current, { ...update.file!, id: crypto.randomUUID(), status: "ready" as const }]);
      setNotice(() => (t(`${update.completed} / ${update.total} 枚を読み込んでいます…`, `Loading ${update.completed} / ${update.total}…`, { key: "Loading {0} / {1}…", values: [update.completed, update.total] })));
    });

    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      if (document.querySelector("dialog.help-dialog[open]")) return;
      if (activeTab === "atlas") {
        window.dispatchEvent(new CustomEvent("smartpng-atlas-drag", { detail: event.payload }));
        return;
      }
      if (event.payload.type !== "drop") return;
      if (activeTab === "preview") window.dispatchEvent(new CustomEvent("smartpng-model-drop", { detail: event.payload.paths }));
      else if (activeTab === "normal") window.dispatchEvent(new CustomEvent("smartpng-normal-drop", { detail: event.payload.paths }));
      else if (["material", "channels", "adjust"].includes(activeTab)) {
        window.dispatchEvent(new CustomEvent(`smartpng-${activeTab}-target`, { detail: event.payload.position }));
        window.dispatchEvent(new CustomEvent(`smartpng-${activeTab}-drop`, { detail: event.payload.paths }));
      }
      else void addPaths(event.payload.paths);
    });

    return () => { void unlistenProgress.then((off) => off()); void unlistenInspect.then((off) => off()); void unlistenDrop.then((off) => off()); };
  }, [activeTab]);

  useEffect(() => {
    if (!storePreview) void getCurrentWebview().setZoom(displayScale);
    localStorage.setItem("smartpng-display-scale-v2", String(displayScale));
  }, [displayScale]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("smartpng-theme-v1", theme);
  }, [theme]);

  useEffect(() => {
    activeTabRef.current = activeTab;
    setVisitedTabs(current => current.has(activeTab) ? current : new Set([...current, activeTab]));
  }, [activeTab]);

  const addPaths = async (paths: string[]) => {
    if (!paths.length) return;
    if (loadingRequestId.current) { setNotice(() => (t("現在の画像を読み込み中です", "Images are already being loaded"))); return; }
    const requestId = crypto.randomUUID();
    loadingRequestId.current = requestId;
    setIsLoading(true);
    setNotice(() => (t("画像を確認しています…", "Inspecting images…")));
    try {
      const result = await invoke<InspectResult<Metadata>>("inspect_files", { paths, requestId, maxFiles: 500 });
      setJobs((current) => {
        const existing = new Set(current.map((job) => job.path.toLowerCase()));
        const added = result.files.filter((file) => !existing.has(file.path.toLowerCase()))
          .map((file) => ({ ...file, id: crypto.randomUUID(), status: "ready" as const }));
        if (result.files.length) setNotice(() => (t(`${result.files.length} 枚を読み込みました${result.failures.length ? `（${result.failures.length} 枚は読込失敗）` : ""}`, `Loaded ${result.files.length} image${result.files.length === 1 ? "" : "s"}${result.failures.length ? t(`（${result.failures.length} 枚は読込失敗）`, ` (${result.failures.length} failed)`, { key: " ({0} failed)", values: [result.failures.length] }) : ""}`, { key: "Loaded {0} image{1}{2}", values: [result.files.length, result.files.length === 1 ? "" : "s", result.failures.length ? t(`（${result.failures.length} 枚は読込失敗）`, ` (${result.failures.length} failed)`, { key: " ({0} failed)", values: [result.failures.length] }) : ""] })));
        else if (result.failures.length) setNotice(() => (t(`読み込めませんでした: ${result.failures[0].message}`, `Could not load the selected image: ${localizedDetail(result.failures[0].message, "The file could not be inspected")}`, { key: "Could not load the selected image: {0}", values: [localizedDetail(result.failures[0].message, "The file could not be inspected")] })));
        else setNotice(() => (t("対象画像がないか、重複したファイルです", "No supported images were found, or the files are already listed")));
        return [...current, ...added];
      });
    } catch (error) { setNotice(() => (t(`追加できませんでした: ${String(error)}`, `Could not add images: ${localizedDetail(error, "The operation failed")}`, { key: "Could not add images: {0}", values: [localizedDetail(error, "The operation failed")] }))); }
    finally {
      if (loadingRequestId.current === requestId) loadingRequestId.current = "";
      setIsLoading(false);
    }
  };

  const chooseFiles = async () => {
    const selected = await open({ multiple: true, filters: [supportedImageFilter] });
    if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]);
  };

  const chooseOutput = async () => {
    const selected = await open({ directory: true, multiple: false, title: t("出力先フォルダを選択", "Choose output folder") });
    if (typeof selected === "string") setSettings((value) => ({ ...value, outputDir: selected }));
  };

  useEffect(() => {
    const openHelp = (event: KeyboardEvent) => { if (event.key === "F1") { event.preventDefault(); setHelpOpen(true); } };
    window.addEventListener("keydown", openHelp);
    return () => window.removeEventListener("keydown", openHelp);
  }, []);

  useEffect(() => {
    if (storePreview) return;
    const unlistenMenu = listen<string>("app-menu-action", (event) => {
      switch (event.payload) {
        case "help-guide": setHelpOpen(true); break;
        case "language-ja": setLocale("ja"); setAppLocale("ja"); break;
        case "language-en": setLocale("en"); setAppLocale("en"); break;
        case "language-zh-CN": setLocale("zh-CN"); setAppLocale("zh-CN"); break;
        case "language-de": setLocale("de"); setAppLocale("de"); break;
        case "file-open":
          if (activeTabRef.current === "optimize") void chooseFiles();
          else window.dispatchEvent(new Event(`smartpng-${activeTabRef.current}-menu-open`));
          break;
        case "tab-optimize": setActiveTab("optimize"); break;
        case "tab-normal": setActiveTab("normal"); break;
        case "tab-atlas": setActiveTab("atlas"); break;
        case "tab-preview": setActiveTab("preview"); break;
        case "tab-material": setActiveTab("material"); break;
        case "tab-channels": setActiveTab("channels"); break;
        case "tab-adjust": setActiveTab("adjust"); break;
        case "theme-light": setTheme("light"); break;
        case "theme-dark": setTheme("dark"); break;
        case "scale-80": setDisplayScale(0.8); break;
        case "scale-90": setDisplayScale(0.9); break;
        case "scale-100": setDisplayScale(1); break;
        case "scale-110": setDisplayScale(1.1); break;
        case "scale-125": setDisplayScale(1.25); break;
      }
    });
    return () => { void unlistenMenu.then((off) => off()); };
  }, []);

  const run = async () => {
    const ready = jobs.filter((job) => job.status === "ready" || job.status === "failed" || job.status === "skipped");
    if (!ready.length || isProcessing) return;
    setIsProcessing(true);
    setNotice(() => (t(`${ready.length} 枚を処理しています…`, `Processing ${ready.length} image${ready.length === 1 ? "" : "s"}…`, { key: "Processing {0} image{1}…", values: [ready.length, ready.length === 1 ? "" : "s"] })));
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id)
      ? { ...job, status: "processing", message: "Waiting" } : job));
    try {
      await invoke("process_batch", {
        paths: ready.map((job) => job.path),
        settings: {
          outputDir: settings.outputDir || null,
          outputFormat: settings.outputFormat,
          quality: Number(settings.quality),
          jpegBackground: settings.jpegBackground,
          maxWidth: numberOrNull(settings.maxWidth === "custom" ? settings.customMaxWidth : settings.maxWidth),
          maxHeight: numberOrNull(settings.maxHeight === "custom" ? settings.customMaxHeight : settings.maxHeight),
          scalePercent: numberOrNull(settings.scalePercent),
          resizeMode: settings.resizeMode,
          colorMode: settings.colorMode,
          colors: settings.colorMode === "indexed" ? Number(settings.colors) : null,
          dithering: settings.dithering,
          optimization: settings.optimization,
        },
      });
      setNotice(() => (t("処理が完了しました", "Processing completed")));
    } catch (error) { setNotice(() => (t(`処理を開始できませんでした: ${String(error)}`, `Could not start processing: ${localizedDetail(error, "The operation failed")}`, { key: "Could not start processing: {0}", values: [localizedDetail(error, "The operation failed")] }))); }
    finally { setIsProcessing(false); }
  };

  const totals = useMemo(() => jobs.reduce((sum, job) => ({
    original: sum.original + job.originalBytes,
    output: sum.output + (job.outputBytes ?? 0),
  }), { original: 0, output: 0 }), [jobs]);
  const saved = totals.output ? totals.original - totals.output : 0;
  const sizeChange = saved >= 0 ? t(`${formatBytes(saved)} 削減`, `${formatBytes(saved)} saved`, { key: "{0} saved", values: [formatBytes(saved)] }) : t(`${formatBytes(-saved)} 増加`, `${formatBytes(-saved)} larger`, { key: "{0} larger", values: [formatBytes(-saved)] });

  return <main>
    {helpOpen && <Suspense fallback={null}><HelpPage initialTopic={activeTab} onClose={() => setHelpOpen(false)} /></Suspense>}
    <nav className="tabs" aria-label={t("機能", "Tools")}>
      <button className={`primary-tool ${activeTab === "optimize" ? "active" : ""}`} onClick={() => setActiveTab("optimize")}>{t("画像最適化", "Image Optimization")}</button>
      <span className="tab-separator" aria-hidden="true" />
      <button className={activeTab === "normal" ? "active" : ""} onClick={() => setActiveTab("normal")}>{t("ノーマルマップ", "Normal Map")}</button>
      <button className={activeTab === "atlas" ? "active" : ""} onClick={() => setActiveTab("atlas")}>{t("テクスチャアトラス", "Texture Atlas")}</button>
      <button className={activeTab === "preview" ? "active" : ""} onClick={() => setActiveTab("preview")}>{t("3Dプレビュー", "3D Preview")}</button>
      <button className={activeTab === "material" ? "active" : ""} onClick={() => setActiveTab("material")}>{t("PBRプレビュー", "PBR Preview")}</button>
      <button className={activeTab === "channels" ? "active" : ""} onClick={() => setActiveTab("channels")}>{t("チャンネル", "Channels")}</button>
      <button className={activeTab === "adjust" ? "active" : ""} onClick={() => setActiveTab("adjust")}>{t("ノーマル調整", "Normal Adjust")}</button>
    </nav>
    <div className="tool-panel" hidden={activeTab !== "optimize"}><section className="workspace">
      <aside className="settings">
        <h2>{t("変換設定", "Conversion Settings")}</h2>
        <label>{t("出力先", "Output folder")} <div className="path-row"><input value={settings.outputDir} onChange={(e) => setSettings({ ...settings, outputDir: e.target.value })} placeholder={t("入力元 / optimized", "Source folder / optimized")} /><button onClick={chooseOutput}>{t("選択", "Choose")}</button></div></label>
        <fieldset><legend>{t("出力形式", "Output Format")}</legend><label>{t("ファイル形式", "File format")}<select value={settings.outputFormat} onChange={(e) => setSettings({ ...settings, outputFormat: e.target.value as Settings["outputFormat"] })}><option value="png">{t("PNG（既定）", "PNG (default)")}</option><option value="webp">WebP</option><option value="jpeg">JPEG</option></select></label>{settings.outputFormat !== "png" && <><label>{t("品質", "Quality")}<div className="range-row"><input type="range" min="1" max="100" value={settings.quality} onChange={(e) => setSettings({ ...settings, quality: e.target.value })} /><output>{settings.quality}</output></div></label>{settings.outputFormat === "jpeg" && <label>{t("透明部分の背景", "Transparent-area background")}<select value={settings.jpegBackground} onChange={(e) => setSettings({ ...settings, jpegBackground: e.target.value as Settings["jpegBackground"] })}><option value="white">{t("白", "White")}</option><option value="black">{t("黒", "Black")}</option></select></label>}</>}</fieldset>
        <fieldset><legend>{t("リサイズ", "Resize")}</legend><div className="split"><label>{t("最大幅", "Max width")}<select value={settings.maxWidth} onChange={(e) => setSettings({ ...settings, maxWidth: e.target.value })}><option value="">{t("指定なし", "No limit")}</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">{t("カスタム…", "Custom…")}</option></select>{settings.maxWidth === "custom" && <input inputMode="numeric" value={settings.customMaxWidth} onChange={(e) => setSettings({ ...settings, customMaxWidth: e.target.value })} placeholder="px" />}</label><label>{t("最大高", "Max height")}<select value={settings.maxHeight} onChange={(e) => setSettings({ ...settings, maxHeight: e.target.value })}><option value="">{t("指定なし", "No limit")}</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">{t("カスタム…", "Custom…")}</option></select>{settings.maxHeight === "custom" && <input inputMode="numeric" value={settings.customMaxHeight} onChange={(e) => setSettings({ ...settings, customMaxHeight: e.target.value })} placeholder="px" />}</label></div><label>{t("倍率 %", "Scale %")}<input inputMode="numeric" value={settings.scalePercent} onChange={(e) => setSettings({ ...settings, scalePercent: e.target.value })} placeholder={t("空欄なら寸法指定", "Leave blank to use dimensions")} /></label><div className="segmented"><button className={settings.resizeMode === "lanczos3" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "lanczos3" })}>{t("高品質", "High quality")}</button><button className={settings.resizeMode === "nearest" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "nearest" })}>{t("ピクセルアート", "Pixel art")}</button></div></fieldset>
        {settings.outputFormat === "png" && <><fieldset><legend>{t("ピクセル形式", "Pixel Format")}</legend><label>{t("PNG形式", "PNG format")}<select value={settings.colorMode} onChange={(e) => setSettings({ ...settings, colorMode: e.target.value as Settings["colorMode"] })}><option value="auto">{t("自動・可逆（推奨）", "Automatic, lossless (recommended)")}</option><option value="indexed">{t("PNG-8 減色（非可逆）", "PNG-8 indexed (lossy)")}</option><option value="rgb24">{t("PNG-24 RGB（透明なし）", "PNG-24 RGB (no transparency)")}</option><option value="rgba32">{t("PNG-32 RGBA（透明を保持）", "PNG-32 RGBA (keep transparency)")}</option><option value="grayscale8">{t("グレースケール 8-bit", "8-bit grayscale")}</option></select></label>{settings.colorMode === "indexed" && <><label>{t("色数", "Colors")}<select value={settings.colors} onChange={(e) => setSettings({ ...settings, colors: e.target.value })}><option value="256">{t("256 色", "256 colors")}</option><option value="128">{t("128 色", "128 colors")}</option><option value="64">{t("64 色", "64 colors")}</option><option value="32">{t("32 色", "32 colors")}</option><option value="16">{t("16 色", "16 colors")}</option></select></label><label className="checkbox"><input type="checkbox" checked={settings.dithering} onChange={(e) => setSettings({ ...settings, dithering: e.target.checked })} />{t("ディザリングを使う", "Use dithering")}</label></>}</fieldset><fieldset><legend>{t("最終最適化", "Final Optimization")}</legend><div className="segmented"><button className={settings.optimization === "fast" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "fast" })}>{t("高速", "Fast")}</button><button className={settings.optimization === "safe" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "safe" })}>{t("標準", "Standard")}</button><button className={settings.optimization === "max" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "max" })}>{t("最大", "Maximum")}</button></div></fieldset></>}
      </aside>
      <section className="queue">
        <div className="queue-toolbar"><div><h2>{t("処理リスト", "Processing Queue")} <span>{jobs.length}</span></h2><p>{isLoading ? notice : totals.output ? sizeChange : notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isProcessing || isLoading}>{t("ファイルを追加", "Add Files")}</button><button className="secondary" onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={!jobs.length || isProcessing || isLoading}>{t("変換ステータスをクリア", "Clear Conversion Status")}</button><button className="primary" onClick={run} disabled={!jobs.length || isProcessing || isLoading}>{isLoading ? t("読込中…", "Loading…") : isProcessing ? t("処理中…", "Processing…") : t("変換を開始", "Start Conversion")}</button></div></div>
        {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={isLoading}><strong>{t(`${supportedImageShortLabel()}またはフォルダーをドロップ`, `Drop ${supportedImageShortLabel()} or a folder`, { key: "Drop {0} or a folder", values: [supportedImageShortLabel()] })}</strong></button> : <><div className="table-header"><span>{t("ファイル", "File")}</span><span>{t("元の寸法", "Source Size")}</span><span>{t("出力寸法", "Output Size")}</span><span>{t("サイズ", "File Size")}</span><span>{t("状態", "Status")}</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row" key={job.id}><div className="file"><Thumbnail path={job.path} alt={t(`${job.name} のプレビュー`, `Preview of ${job.name}`, { key: "Preview of {0}", values: [job.name] })} fallbackLabel={t("画像", "Image")} /><div><strong title={job.path}>{job.name}</strong><small>{job.message ? localizedDetail(job.message, label(job.status)) : job.path}</small></div></div><span>{dimensions(job.width, job.height)}</span><span>{dimensions(job.outputWidth, job.outputHeight)}</span><span>{formatBytes(job.originalBytes)}{job.outputBytes !== undefined && <small> → {formatBytes(job.outputBytes)}</small>}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title={t("エクスプローラーで出力ファイルを表示", "Show output file in File Explorer")} onClick={() => revealItemInDir(job.outputPath!)}>{t("開く", "Open")}</button>}<button title={t("リストから削除", "Remove from list")} onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={isProcessing || isLoading}>{t("リストをクリア", "Clear List")}</button></div><span>{t(`${jobs.filter((job) => job.status === "done").length} 件完了 / ${jobs.filter((job) => job.status === "failed").length} 件失敗`, `${jobs.filter((job) => job.status === "done").length} completed / ${jobs.filter((job) => job.status === "failed").length} failed`, { key: "{0} completed / {1} failed", values: [jobs.filter((job) => job.status === "done").length, jobs.filter((job) => job.status === "failed").length] })}</span></div></>}
      </section>
    </section></div>
    <div className="tool-panel" hidden={activeTab !== "normal"}>{visitedTabs.has("normal") && <NormalMap />}</div>
    <div className="tool-panel" hidden={activeTab !== "atlas"}>{visitedTabs.has("atlas") && <TextureAtlas />}</div>
    <div className="tool-panel" hidden={activeTab !== "channels"}>{visitedTabs.has("channels") && <TextureTools mode="channels" />}</div>
    <div className="tool-panel" hidden={activeTab !== "adjust"}>{visitedTabs.has("adjust") && <TextureTools mode="adjust" />}</div>
    <div className="tool-panel" hidden={activeTab !== "material"}>{visitedTabs.has("material") && <Suspense fallback={<section className="queue" />}><MaterialPreview active={activeTab === "material"} /></Suspense>}</div>
    <div className="tool-panel" hidden={activeTab !== "preview"}>{visitedTabs.has("preview") && <Suspense fallback={<section className="model-preview-empty"><div className="drop-zone"><strong>{t("3D表示を準備中…", "Preparing 3D preview…")}</strong></div></section>}><ModelPreview active={activeTab === "preview"} /></Suspense>}</div>
  </main>;
}

function numberOrNull(value: string) { const number = Number(value); return Number.isInteger(number) && number > 0 ? number : null; }
function label(status: Status) { return ({ ready: t("準備完了", "Ready"), processing: t("処理中", "Processing"), done: t("完了", "Done"), skipped: t("スキップ", "Skipped"), failed: t("失敗", "Failed") })[status]; }
