import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import NormalMap from "./NormalMap";
import AiRestore from "./AiRestore";
import TextureAtlas from "./TextureAtlas";
import Thumbnail from "./Thumbnail";
import { InspectProgress, InspectResult, supportedImageFilter } from "./imageFiles";

const ModelPreview = lazy(() => import("./ModelPreview"));

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
  const [activeTab, setActiveTab] = useState<"optimize" | "restore" | "normal" | "atlas" | "preview">("optimize");
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
  const [notice, setNotice] = useState("PNG / HEIC をここへドロップしてください");

  useEffect(() => {
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
      setNotice(`${update.completed} / ${update.total} 枚を読み込んでいます…`);
    });

    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      if (activeTab === "atlas") {
        window.dispatchEvent(new CustomEvent("smartpng-atlas-drag", { detail: event.payload }));
        return;
      }
      if (event.payload.type !== "drop") return;
      if (activeTab === "preview") window.dispatchEvent(new CustomEvent("smartpng-model-drop", { detail: event.payload.paths }));
      else if (activeTab === "restore") window.dispatchEvent(new CustomEvent("smartpng-ai-restore-drop", { detail: event.payload.paths }));
      else if (activeTab === "normal") window.dispatchEvent(new CustomEvent("smartpng-normal-drop", { detail: event.payload.paths }));
      else void addPaths(event.payload.paths);
    });

    return () => { void unlistenProgress.then((off) => off()); void unlistenInspect.then((off) => off()); void unlistenDrop.then((off) => off()); };
  }, [activeTab]);

  useEffect(() => {
    void getCurrentWebview().setZoom(displayScale);
    localStorage.setItem("smartpng-display-scale-v2", String(displayScale));
  }, [displayScale]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("smartpng-theme-v1", theme);
  }, [theme]);

  const addPaths = async (paths: string[]) => {
    if (!paths.length) return;
    if (loadingRequestId.current) { setNotice("現在の画像を読み込み中です"); return; }
    const requestId = crypto.randomUUID();
    loadingRequestId.current = requestId;
    setIsLoading(true);
    setNotice("画像を確認しています…");
    try {
      const result = await invoke<InspectResult<Metadata>>("inspect_files", { paths, requestId, maxFiles: 500 });
      setJobs((current) => {
        const existing = new Set(current.map((job) => job.path.toLowerCase()));
        const added = result.files.filter((file) => !existing.has(file.path.toLowerCase()))
          .map((file) => ({ ...file, id: crypto.randomUUID(), status: "ready" as const }));
        if (result.files.length) setNotice(`${result.files.length} 枚を読み込みました${result.failures.length ? `（${result.failures.length} 枚は読込失敗）` : ""}`);
        else if (result.failures.length) setNotice(`読み込めませんでした: ${result.failures[0].message}`);
        else setNotice("対象画像がないか、重複したファイルです");
        return [...current, ...added];
      });
    } catch (error) { setNotice(`追加できませんでした: ${String(error)}`); }
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
    const selected = await open({ directory: true, multiple: false, title: "出力先フォルダを選択" });
    if (typeof selected === "string") setSettings((value) => ({ ...value, outputDir: selected }));
  };

  const run = async () => {
    const ready = jobs.filter((job) => job.status === "ready" || job.status === "failed" || job.status === "skipped");
    if (!ready.length || isProcessing) return;
    setIsProcessing(true);
    setNotice(`${ready.length} 枚を処理しています…`);
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id)
      ? { ...job, status: "processing", message: "待機中" } : job));
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
      setNotice("処理が完了しました");
    } catch (error) { setNotice(`処理を開始できませんでした: ${String(error)}`); }
    finally { setIsProcessing(false); }
  };

  const totals = useMemo(() => jobs.reduce((sum, job) => ({
    original: sum.original + job.originalBytes,
    output: sum.output + (job.outputBytes ?? 0),
  }), { original: 0, output: 0 }), [jobs]);
  const saved = totals.output ? totals.original - totals.output : 0;
  const sizeChange = saved >= 0 ? `${formatBytes(saved)} 削減` : `${formatBytes(-saved)} 増加`;

  return <main>
    <nav className="tabs" aria-label="機能">
      <button className={activeTab === "optimize" ? "active" : ""} onClick={() => setActiveTab("optimize")}>画像最適化</button>
      <button className={activeTab === "restore" ? "active" : ""} onClick={() => setActiveTab("restore")}>AI復元</button>
      <button className={activeTab === "normal" ? "active" : ""} onClick={() => setActiveTab("normal")}>ノーマルマップ</button>
      <button className={activeTab === "atlas" ? "active" : ""} onClick={() => setActiveTab("atlas")}>テクスチャアトラス</button>
      <button className={activeTab === "preview" ? "active" : ""} onClick={() => setActiveTab("preview")}>3Dプレビュー</button>
      <div className="view-controls"><label className="theme-control">テーマ <select value={theme} onChange={(event) => setTheme(event.target.value as "light" | "dark")}><option value="light">ライト（既定）</option><option value="dark">ダーク</option></select></label><label className="display-scale">表示 <select value={displayScale} onChange={(event) => setDisplayScale(Number(event.target.value))}><option value="0.8">80%</option><option value="0.9">90%</option><option value="1">100%（既定）</option><option value="1.1">110%</option><option value="1.25">125%</option></select></label></div>
    </nav>
    {activeTab === "optimize" ? <section className="workspace">
      <aside className="settings">
        <h2>変換設定</h2>
        <label>出力先 <div className="path-row"><input value={settings.outputDir} onChange={(e) => setSettings({ ...settings, outputDir: e.target.value })} placeholder="入力元 / optimized" /><button onClick={chooseOutput}>選択</button></div></label>
        <fieldset><legend>出力形式</legend><label>ファイル形式<select value={settings.outputFormat} onChange={(e) => setSettings({ ...settings, outputFormat: e.target.value as Settings["outputFormat"] })}><option value="png">PNG（既定）</option><option value="webp">WebP</option><option value="jpeg">JPEG</option></select></label>{settings.outputFormat !== "png" && <><label>品質<div className="range-row"><input type="range" min="1" max="100" value={settings.quality} onChange={(e) => setSettings({ ...settings, quality: e.target.value })} /><output>{settings.quality}</output></div></label>{settings.outputFormat === "jpeg" && <label>透明部分の背景<select value={settings.jpegBackground} onChange={(e) => setSettings({ ...settings, jpegBackground: e.target.value as Settings["jpegBackground"] })}><option value="white">白</option><option value="black">黒</option></select></label>}<p className="mode-note">{settings.outputFormat === "webp" ? "透過を保持できる非可逆WebPです。" : "JPEGは透過を保持できません。"}</p></>}</fieldset>
        <fieldset><legend>リサイズ</legend><div className="split"><label>最大幅<select value={settings.maxWidth} onChange={(e) => setSettings({ ...settings, maxWidth: e.target.value })}><option value="">指定なし</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{settings.maxWidth === "custom" && <input inputMode="numeric" value={settings.customMaxWidth} onChange={(e) => setSettings({ ...settings, customMaxWidth: e.target.value })} placeholder="px" />}</label><label>最大高<select value={settings.maxHeight} onChange={(e) => setSettings({ ...settings, maxHeight: e.target.value })}><option value="">指定なし</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{settings.maxHeight === "custom" && <input inputMode="numeric" value={settings.customMaxHeight} onChange={(e) => setSettings({ ...settings, customMaxHeight: e.target.value })} placeholder="px" />}</label></div><label>倍率 %<input inputMode="numeric" value={settings.scalePercent} onChange={(e) => setSettings({ ...settings, scalePercent: e.target.value })} placeholder="空欄なら寸法指定" /></label><div className="segmented"><button className={settings.resizeMode === "lanczos3" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "lanczos3" })}>高品質</button><button className={settings.resizeMode === "nearest" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "nearest" })}>ピクセルアート</button></div></fieldset>
        {settings.outputFormat === "png" && <><fieldset><legend>ピクセル形式</legend><label>PNG形式<select value={settings.colorMode} onChange={(e) => setSettings({ ...settings, colorMode: e.target.value as Settings["colorMode"] })}><option value="auto">自動・可逆（推奨）</option><option value="indexed">PNG-8 減色（非可逆）</option><option value="rgb24">PNG-24 RGB（透明なし）</option><option value="rgba32">PNG-32 RGBA（透明を保持）</option><option value="grayscale8">グレースケール 8-bit</option></select></label>{settings.colorMode === "indexed" && <><label>色数<select value={settings.colors} onChange={(e) => setSettings({ ...settings, colors: e.target.value })}><option value="256">256 色</option><option value="128">128 色</option><option value="64">64 色</option><option value="32">32 色</option><option value="16">16 色</option></select></label><label className="checkbox"><input type="checkbox" checked={settings.dithering} onChange={(e) => setSettings({ ...settings, dithering: e.target.checked })} />ディザリングを使う</label></>}<p className="mode-note">{settings.colorMode === "rgb24" ? "透明部分は黒背景へ合成します。" : settings.colorMode === "rgba32" ? "アルファを必ず保持します。" : settings.colorMode === "grayscale8" ? "明度のみを保持します。" : settings.colorMode === "indexed" ? "非可逆の減色です。品質が 90 未満になる画像は、可逆最適化へ自動で切り替えます。ノーマルマップにはこちらを推奨しません。" : "画素の値・色数・透過を変えない可逆圧縮です。"}</p></fieldset><fieldset><legend>最終最適化</legend><div className="segmented"><button className={settings.optimization === "fast" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "fast" })}>高速</button><button className={settings.optimization === "safe" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "safe" })}>標準</button><button className={settings.optimization === "max" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "max" })}>最大</button></div><p className="mode-note">高速は圧縮探索を抑えます。最大はより小さくなりますが時間がかかります。</p></fieldset></>}
        <p className="hint">PNG / HEICに対応します。HEICは表示向けsRGBへ変換します。元ファイルは変更しません。PNG最適化では結果が大きくなる画像を保存しません。</p>
      </aside>
      <section className="queue">
        <div className="queue-toolbar"><div><h2>処理リスト <span>{jobs.length}</span></h2><p>{isLoading ? notice : totals.output ? sizeChange : notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isProcessing || isLoading}>ファイルを追加</button><button className="primary" onClick={run} disabled={!jobs.length || isProcessing || isLoading}>{isLoading ? "読込中…" : isProcessing ? "処理中…" : "変換を開始"}</button></div></div>
        {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={isLoading}><strong>PNG / HEIC またはフォルダーをドロップ</strong><span>またはクリックしてファイルを選択</span></button> : <><div className="table-header"><span>ファイル</span><span>元の寸法</span><span>出力寸法</span><span>サイズ</span><span>状態</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row" key={job.id}><div className="file"><Thumbnail path={job.path} alt={`${job.name} のプレビュー`} fallbackLabel="画像" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{dimensions(job.width, job.height)}</span><span>{dimensions(job.outputWidth, job.outputHeight)}</span><span>{formatBytes(job.originalBytes)}{job.outputBytes !== undefined && <small> → {formatBytes(job.outputBytes)}</small>}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title="出力先を開く" onClick={() => openPath(job.outputPath!)}>開く</button>}<button title="リストから削除" onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={isProcessing || isLoading}>リストをクリア</button><button onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={isProcessing || isLoading}>変換ステータスをクリア</button></div><span>{jobs.filter((job) => job.status === "done").length} 件完了 / {jobs.filter((job) => job.status === "failed").length} 件失敗</span></div></>}
      </section>
    </section> : activeTab === "restore" ? <AiRestore /> : activeTab === "normal" ? <NormalMap /> : activeTab === "atlas" ? <TextureAtlas /> : <Suspense fallback={<section className="model-preview-empty"><div className="drop-zone"><strong>3D表示を準備中…</strong></div></section>}><ModelPreview /></Suspense>}
  </main>;
}

function numberOrNull(value: string) { const number = Number(value); return Number.isInteger(number) && number > 0 ? number : null; }
function label(status: Status) { return ({ ready: "準備完了", processing: "処理中", done: "完了", skipped: "スキップ", failed: "失敗" })[status]; }
