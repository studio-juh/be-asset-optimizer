import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import Thumbnail from "./Thumbnail";
import { InspectProgress, InspectResult, supportedImageFilter } from "./imageFiles";

type Status = "ready" | "processing" | "done" | "skipped" | "failed";
type Job = { id: string; path: string; name: string; originalBytes: number; width: number; height: number; status: Status; outputBytes?: number; outputWidth?: number; outputHeight?: number; message?: string; outputPath?: string };
type Metadata = Omit<Job, "id" | "status">;
type Progress = { path: string; status: Status; output_bytes?: number; output_width?: number; output_height?: number; message?: string; output_path?: string };

const formatBytes = (bytes?: number) => bytes === undefined ? "—" : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;
const label = (status: Status) => ({ ready: "準備完了", processing: "生成中", done: "完了", skipped: "スキップ", failed: "失敗" })[status];

export default function NormalMap() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [outputDir, setOutputDir] = useState("");
  const [maxLongEdge, setMaxLongEdge] = useState("");
  const [customMaxLongEdge, setCustomMaxLongEdge] = useState("");
  const [strength, setStrength] = useState(1);
  const [level, setLevel] = useState(1);
  const [convention, setConvention] = useState<"opengl" | "directx">("opengl");
  const [invertHeight, setInvertHeight] = useState(false);
  const [invertGreen, setInvertGreen] = useState(false);
  const [padToSquare, setPadToSquare] = useState(false);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const loadingRequestId = useRef("");
  const [notice, setNotice] = useState("高さマップ用 PNG / HEIC をここへドロップしてください");

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
        const added = result.files.filter((file) => !existing.has(file.path.toLowerCase())).map((file) => ({ ...file, id: crypto.randomUUID(), status: "ready" as const }));
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

  useEffect(() => {
    const offProgress = listen<Progress>("normal-job-progress", (event) => {
      const update = event.payload;
      setJobs((current) => current.map((job) => job.path === update.path ? { ...job, status: update.status, outputBytes: update.output_bytes, outputWidth: update.output_width, outputHeight: update.output_height, message: update.message, outputPath: update.output_path } : job));
    });
    const offInspect = listen<InspectProgress<Metadata>>("inspect-files-progress", (event) => {
      const update = event.payload;
      if (update.requestId !== loadingRequestId.current) return;
      if (update.file) setJobs((current) => current.some((job) => job.path.toLowerCase() === update.file!.path.toLowerCase())
        ? current : [...current, { ...update.file!, id: crypto.randomUUID(), status: "ready" as const }]);
      setNotice(`${update.completed} / ${update.total} 枚を読み込んでいます…`);
    });
    const onDrop = (event: Event) => { void addPaths((event as CustomEvent<string[]>).detail); };
    window.addEventListener("smartpng-normal-drop", onDrop);
    return () => { void offProgress.then((off) => off()); void offInspect.then((off) => off()); window.removeEventListener("smartpng-normal-drop", onDrop); };
  }, []);

  const chooseFiles = async () => { const selected = await open({ multiple: true, filters: [supportedImageFilter] }); if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]); };
  const chooseOutput = async () => { const selected = await open({ directory: true, multiple: false, title: "出力先フォルダを選択" }); if (typeof selected === "string") setOutputDir(selected); };
  const run = async () => {
    const ready = jobs.filter((job) => ["ready", "failed", "skipped"].includes(job.status));
    if (!ready.length || isProcessing) return;
    setIsProcessing(true); setNotice(`${ready.length} 枚を生成しています…`);
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id) ? { ...job, status: "processing", message: "待機中" } : job));
    try { await invoke("process_normal_batch", { paths: ready.map((job) => job.path), settings: { outputDir: outputDir || null, maxLongEdge: numberOrNull(maxLongEdge === "custom" ? customMaxLongEdge : maxLongEdge), strength, level, convention, invertHeight, invertGreen, padToSquare } }); setNotice("ノーマルマップの生成が完了しました"); }
    catch (error) { setNotice(`生成を開始できませんでした: ${String(error)}`); }
    finally { setIsProcessing(false); }
  };

  return <section className="workspace">
    <aside className="settings normal-settings">
      <h2>ノーマルマップ設定</h2>
      <label>出力先<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder="入力元 / normal_maps" /><button onClick={chooseOutput}>選択</button></div></label>
      <fieldset><legend>変換方法</legend><label>最大長辺<select value={maxLongEdge} onChange={(event) => setMaxLongEdge(event.target.value)}><option value="">指定なし</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{maxLongEdge === "custom" && <input inputMode="numeric" value={customMaxLongEdge} onChange={(event) => setCustomMaxLongEdge(event.target.value)} placeholder="px" />}</label><label>強さ <span className="range-row"><input type="range" min="0.1" max="4" step="0.1" value={strength} onChange={(event) => setStrength(Number(event.target.value))} /><output>{strength.toFixed(1)}</output></span></label><label>高さのレベル <span className="range-row"><input type="range" min="0.1" max="4" step="0.1" value={level} onChange={(event) => setLevel(Number(event.target.value))} /><output>{level.toFixed(1)}</output></span></label><label>Y 軸の向き<select value={convention} onChange={(event) => setConvention(event.target.value as "opengl" | "directx")}><option value="opengl">OpenGL (+Y) — Blender 向け</option><option value="directx">DirectX (-Y)</option></select></label><label className="checkbox"><input type="checkbox" checked={invertHeight} onChange={(event) => setInvertHeight(event.target.checked)} />高さを反転</label><label className="checkbox"><input type="checkbox" checked={invertGreen} onChange={(event) => setInvertGreen(event.target.checked)} />G チャンネルを反転</label><label className="checkbox"><input type="checkbox" checked={padToSquare} onChange={(event) => setPadToSquare(event.target.checked)} />正方形に補完（高さ 0・中央配置）</label></fieldset>
      <p className="hint">出力は Blender 向け RGB 24-bit（アルファなし）です。明るい部分を高く、暗い部分を低く扱います。Blender では Image Texture → Normal Map ノード経由で接続してください。</p>
    </aside>
    <section className="queue">
      <div className="queue-toolbar"><div><h2>ノーマルマップ一覧 <span>{jobs.length}</span></h2><p>{notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isProcessing || isLoading}>ファイルを追加</button><button className="primary" onClick={run} disabled={!jobs.length || isProcessing || isLoading}>{isLoading ? "読込中…" : isProcessing ? "生成中…" : "生成を開始"}</button></div></div>
      {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={isLoading}><strong>高さマップ PNG / HEIC またはフォルダーをドロップ</strong><span>明度から Blender 用ノーマルマップを生成</span></button> : <><div className="table-header normal-table"><span>ファイル</span><span>寸法</span><span>出力サイズ</span><span>状態</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row normal-table" key={job.id}><div className="file"><Thumbnail path={job.path} alt={`${job.name} のプレビュー`} fallbackLabel="N" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{job.width} × {job.height}</span><span>{formatBytes(job.outputBytes)}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title="出力先を開く" onClick={() => openPath(job.outputPath!)}>開く</button>}<button title="リストから削除" onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={isProcessing || isLoading}>リストをクリア</button><button onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={isProcessing || isLoading}>変換ステータスをクリア</button></div><span>{jobs.filter((job) => job.status === "done").length} 件完了 / {jobs.filter((job) => job.status === "failed").length} 件失敗</span></div></>}
    </section>
  </section>;
}

function numberOrNull(value: string) { const number = Number(value); return Number.isInteger(number) && number > 0 ? number : null; }
