import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import NormalMap from "./NormalMap";
import Thumbnail from "./Thumbnail";

type Status = "ready" | "processing" | "done" | "skipped" | "failed";
type ResizeMode = "lanczos3" | "nearest";
type Optimization = "safe" | "max";

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
  maxWidth: "",
  maxHeight: "",
  customMaxWidth: "",
  customMaxHeight: "",
  scalePercent: "",
  resizeMode: "lanczos3",
  colorMode: "auto",
  colors: "256",
  dithering: true,
  optimization: "safe",
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
  const [activeTab, setActiveTab] = useState<"optimize" | "normal">("optimize");
  const [jobs, setJobs] = useState<ImageJob[]>([]);
  const [settings, setSettings] = useState<Settings>(initialSettings);
  const [isProcessing, setIsProcessing] = useState(false);
  const [notice, setNotice] = useState("PNG をここへドロップしてください");

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

    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type !== "drop") return;
      if (activeTab === "normal") window.dispatchEvent(new CustomEvent("smartpng-normal-drop", { detail: event.payload.paths }));
      else void addPaths(event.payload.paths);
    });

    return () => { void unlistenProgress.then((off) => off()); void unlistenDrop.then((off) => off()); };
  }, [activeTab]);

  const addPaths = async (paths: string[]) => {
    const pngs = paths.filter((path) => path.toLowerCase().endsWith(".png"));
    if (!pngs.length) { setNotice("PNG ファイルを選択してください"); return; }
    try {
      const files = await invoke<Metadata[]>("inspect_files", { paths: pngs });
      setJobs((current) => {
        const existing = new Set(current.map((job) => job.path.toLowerCase()));
        const added = files.filter((file) => !existing.has(file.path.toLowerCase()))
          .map((file) => ({ ...file, id: crypto.randomUUID(), status: "ready" as const }));
        setNotice(added.length ? `${added.length} 枚をキューに追加しました` : "重複したファイルは追加していません");
        return [...current, ...added];
      });
    } catch (error) { setNotice(`追加できませんでした: ${String(error)}`); }
  };

  const chooseFiles = async () => {
    const selected = await open({ multiple: true, filters: [{ name: "PNG", extensions: ["png"] }] });
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

  return <main>
    <nav className="tabs" aria-label="機能">
      <button className={activeTab === "optimize" ? "active" : ""} onClick={() => setActiveTab("optimize")}>PNG 最適化</button>
      <button className={activeTab === "normal" ? "active" : ""} onClick={() => setActiveTab("normal")}>ノーマルマップ</button>
    </nav>
    {activeTab === "optimize" ? <section className="workspace">
      <aside className="settings">
        <h2>変換設定</h2>
        <label>出力先 <div className="path-row"><input value={settings.outputDir} onChange={(e) => setSettings({ ...settings, outputDir: e.target.value })} placeholder="入力元 / optimized" /><button onClick={chooseOutput}>選択</button></div></label>
        <fieldset><legend>リサイズ</legend><div className="split"><label>最大幅<select value={settings.maxWidth} onChange={(e) => setSettings({ ...settings, maxWidth: e.target.value })}><option value="">指定なし</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{settings.maxWidth === "custom" && <input inputMode="numeric" value={settings.customMaxWidth} onChange={(e) => setSettings({ ...settings, customMaxWidth: e.target.value })} placeholder="px" />}</label><label>最大高<select value={settings.maxHeight} onChange={(e) => setSettings({ ...settings, maxHeight: e.target.value })}><option value="">指定なし</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{settings.maxHeight === "custom" && <input inputMode="numeric" value={settings.customMaxHeight} onChange={(e) => setSettings({ ...settings, customMaxHeight: e.target.value })} placeholder="px" />}</label></div><label>倍率 %<input inputMode="numeric" value={settings.scalePercent} onChange={(e) => setSettings({ ...settings, scalePercent: e.target.value })} placeholder="空欄なら寸法指定" /></label><div className="segmented"><button className={settings.resizeMode === "lanczos3" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "lanczos3" })}>高品質</button><button className={settings.resizeMode === "nearest" ? "active" : ""} onClick={() => setSettings({ ...settings, resizeMode: "nearest" })}>ピクセルアート</button></div></fieldset>
        <fieldset><legend>ピクセル形式</legend><label>出力形式<select value={settings.colorMode} onChange={(e) => setSettings({ ...settings, colorMode: e.target.value as Settings["colorMode"] })}><option value="auto">自動・可逆（推奨）</option><option value="indexed">PNG-8 減色（非可逆）</option><option value="rgb24">PNG-24 RGB（透明なし）</option><option value="rgba32">PNG-32 RGBA（透明を保持）</option><option value="grayscale8">グレースケール 8-bit</option></select></label>{settings.colorMode === "indexed" && <><label>色数<select value={settings.colors} onChange={(e) => setSettings({ ...settings, colors: e.target.value })}><option value="256">256 色</option><option value="128">128 色</option><option value="64">64 色</option><option value="32">32 色</option><option value="16">16 色</option></select></label><label className="checkbox"><input type="checkbox" checked={settings.dithering} onChange={(e) => setSettings({ ...settings, dithering: e.target.checked })} />ディザリングを使う</label></>}<p className="mode-note">{settings.colorMode === "rgb24" ? "透明部分は黒背景へ合成します。" : settings.colorMode === "rgba32" ? "アルファを必ず保持します。" : settings.colorMode === "grayscale8" ? "明度のみを保持します。" : settings.colorMode === "indexed" ? "非可逆の減色です。品質が 90 未満になる画像は保存せず失敗として表示します。" : "画素の値・色数・透過を変えない可逆圧縮です。"}</p></fieldset>
        <fieldset><legend>最終最適化</legend><div className="segmented"><button className={settings.optimization === "safe" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "safe" })}>安全</button><button className={settings.optimization === "max" ? "active" : ""} onClick={() => setSettings({ ...settings, optimization: "max" })}>最大</button></div></fieldset>
        <p className="hint">元ファイルは変更しません。結果が大きくなる画像は保存せず、スキップします。</p>
      </aside>
      <section className="queue">
        <div className="queue-toolbar"><div><h2>処理リスト <span>{jobs.length}</span></h2><p>{totals.output ? `${formatBytes(saved)} 削減` : notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isProcessing}>ファイルを追加</button><button className="primary" onClick={run} disabled={!jobs.length || isProcessing}>{isProcessing ? "処理中…" : "最適化を開始"}</button></div></div>
        {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles}><strong>PNG をドロップ</strong><span>またはクリックしてファイルを選択</span></button> : <><div className="table-header"><span>ファイル</span><span>元の寸法</span><span>出力寸法</span><span>サイズ</span><span>状態</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row" key={job.id}><div className="file"><Thumbnail path={job.path} alt={`${job.name} のプレビュー`} fallbackLabel="PNG" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{dimensions(job.width, job.height)}</span><span>{dimensions(job.outputWidth, job.outputHeight)}</span><span>{formatBytes(job.originalBytes)}{job.outputBytes !== undefined && <small> → {formatBytes(job.outputBytes)}</small>}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title="出力先を開く" onClick={() => openPath(job.outputPath!)}>開く</button>}<button title="リストから削除" onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><button onClick={() => setJobs([])} disabled={isProcessing}>リストをクリア</button><span>{jobs.filter((job) => job.status === "done").length} 件完了 / {jobs.filter((job) => job.status === "failed").length} 件失敗</span></div></>}
      </section>
    </section> : <NormalMap />}
  </main>;
}

function numberOrNull(value: string) { const number = Number(value); return Number.isInteger(number) && number > 0 ? number : null; }
function label(status: Status) { return ({ ready: "準備完了", processing: "処理中", done: "完了", skipped: "スキップ", failed: "失敗" })[status]; }
