import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import Thumbnail from "./Thumbnail";
import { InspectProgress, InspectResult, supportedImageFilter, supportedImageShortLabel } from "./imageFiles";
import { localizedDetail, t } from "./i18n";

type Status = "ready" | "processing" | "done" | "skipped" | "failed";
type Job = { id: string; path: string; name: string; originalBytes: number; width: number; height: number; status: Status; outputBytes?: number; outputWidth?: number; outputHeight?: number; message?: string; outputPath?: string };
type Metadata = Omit<Job, "id" | "status">;
type Progress = { path: string; status: Status; output_bytes?: number; output_width?: number; output_height?: number; message?: string; output_path?: string };

const formatBytes = (bytes?: number) => bytes === undefined ? "—" : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;
const label = (status: Status) => ({ ready: t("準備完了", "Ready"), processing: t("生成中", "Generating"), done: t("完了", "Done"), skipped: t("スキップ", "Skipped"), failed: t("失敗", "Failed") })[status];

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
  const [notice, setNotice] = useState(t(`高さマップ用の${supportedImageShortLabel}をここへドロップしてください`, `Drop ${supportedImageShortLabel} for height input here`));

  const addPaths = async (paths: string[]) => {
    if (!paths.length) return;
    if (loadingRequestId.current) { setNotice(t("現在の画像を読み込み中です", "Images are already being loaded")); return; }
    const requestId = crypto.randomUUID();
    loadingRequestId.current = requestId;
    setIsLoading(true);
    setNotice(t("画像を確認しています…", "Inspecting images…"));
    try {
      const result = await invoke<InspectResult<Metadata>>("inspect_files", { paths, requestId, maxFiles: 500 });
      setJobs((current) => {
        const existing = new Set(current.map((job) => job.path.toLowerCase()));
        const added = result.files.filter((file) => !existing.has(file.path.toLowerCase())).map((file) => ({ ...file, id: crypto.randomUUID(), status: "ready" as const }));
        if (result.files.length) setNotice(t(`${result.files.length} 枚を読み込みました${result.failures.length ? `（${result.failures.length} 枚は読込失敗）` : ""}`, `Loaded ${result.files.length} image${result.files.length === 1 ? "" : "s"}${result.failures.length ? ` (${result.failures.length} failed)` : ""}`));
        else if (result.failures.length) setNotice(t(`読み込めませんでした: ${result.failures[0].message}`, `Could not load the selected image: ${localizedDetail(result.failures[0].message, "The file could not be inspected")}`));
        else setNotice(t("対象画像がないか、重複したファイルです", "No supported images were found, or the files are already listed"));
        return [...current, ...added];
      });
    } catch (error) { setNotice(t(`追加できませんでした: ${String(error)}`, `Could not add images: ${localizedDetail(error, "The operation failed")}`)); }
    finally {
      if (loadingRequestId.current === requestId) loadingRequestId.current = "";
      setIsLoading(false);
    }
  };

  useEffect(() => {
    const offProgress = listen<Progress>("normal-job-progress", (event) => {
      const update = event.payload;
      setJobs((current) => current.map((job) => job.path === update.path ? { ...job, status: update.status, outputBytes: update.output_bytes, outputWidth: update.output_width, outputHeight: update.output_height, message: update.message ? localizedDetail(update.message, update.status === "done" ? "Normal map generated" : update.status === "failed" ? "Generation failed" : "Generating normal map") : undefined, outputPath: update.output_path } : job));
    });
    const offInspect = listen<InspectProgress<Metadata>>("inspect-files-progress", (event) => {
      const update = event.payload;
      if (update.requestId !== loadingRequestId.current) return;
      if (update.file) setJobs((current) => current.some((job) => job.path.toLowerCase() === update.file!.path.toLowerCase())
        ? current : [...current, { ...update.file!, id: crypto.randomUUID(), status: "ready" as const }]);
      setNotice(t(`${update.completed} / ${update.total} 枚を読み込んでいます…`, `Loading ${update.completed} / ${update.total}…`));
    });
    const onDrop = (event: Event) => { void addPaths((event as CustomEvent<string[]>).detail); };
    const onMenuOpen = () => { void chooseFiles(); };
    window.addEventListener("smartpng-normal-drop", onDrop);
    window.addEventListener("smartpng-menu-open", onMenuOpen);
    return () => { void offProgress.then((off) => off()); void offInspect.then((off) => off()); window.removeEventListener("smartpng-normal-drop", onDrop); window.removeEventListener("smartpng-menu-open", onMenuOpen); };
  }, []);

  const chooseFiles = async () => { const selected = await open({ multiple: true, filters: [supportedImageFilter] }); if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]); };
  const chooseOutput = async () => { const selected = await open({ directory: true, multiple: false, title: t("出力先フォルダを選択", "Choose output folder") }); if (typeof selected === "string") setOutputDir(selected); };
  const run = async () => {
    const ready = jobs.filter((job) => ["ready", "failed", "skipped"].includes(job.status));
    if (!ready.length || isProcessing) return;
    setIsProcessing(true); setNotice(t(`${ready.length} 枚を生成しています…`, `Generating ${ready.length} normal map${ready.length === 1 ? "" : "s"}…`));
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id) ? { ...job, status: "processing", message: t("待機中", "Waiting") } : job));
    try { await invoke("process_normal_batch", { paths: ready.map((job) => job.path), settings: { outputDir: outputDir || null, maxLongEdge: numberOrNull(maxLongEdge === "custom" ? customMaxLongEdge : maxLongEdge), strength, level, convention, invertHeight, invertGreen, padToSquare } }); setNotice(t("ノーマルマップの生成が完了しました", "Normal-map generation completed")); }
    catch (error) { setNotice(t(`生成を開始できませんでした: ${String(error)}`, `Could not start generation: ${localizedDetail(error, "The operation failed")}`)); }
    finally { setIsProcessing(false); }
  };

  return <section className="workspace">
    <aside className="settings normal-settings">
      <h2>{t("ノーマルマップ設定", "Normal Map Settings")}</h2>
      <label>{t("出力先", "Output folder")}<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder={t("入力元 / normal_maps", "Source folder / normal_maps")} /><button onClick={chooseOutput}>{t("選択", "Choose")}</button></div></label>
      <fieldset><legend>{t("変換方法", "Generation")}</legend><label>{t("最大長辺", "Max long edge")}<select value={maxLongEdge} onChange={(event) => setMaxLongEdge(event.target.value)}><option value="">{t("指定なし", "No limit")}</option><option value="256">256 px</option><option value="512">512 px</option><option value="1024">1024 px</option><option value="2048">2048 px</option><option value="4096">4096 px</option><option value="custom">Custom…</option></select>{maxLongEdge === "custom" && <input inputMode="numeric" value={customMaxLongEdge} onChange={(event) => setCustomMaxLongEdge(event.target.value)} placeholder="px" />}</label><label>{t("強さ", "Strength")} <span className="range-row"><input type="range" min="0.1" max="4" step="0.1" value={strength} onChange={(event) => setStrength(Number(event.target.value))} /><output>{strength.toFixed(1)}</output></span></label><label>{t("高さのレベル", "Height level")} <span className="range-row"><input type="range" min="0.1" max="4" step="0.1" value={level} onChange={(event) => setLevel(Number(event.target.value))} /><output>{level.toFixed(1)}</output></span></label><label>{t("Y 軸の向き", "Y-axis convention")}<select value={convention} onChange={(event) => setConvention(event.target.value as "opengl" | "directx")}><option value="opengl">{t("OpenGL (+Y) — Blender 向け", "OpenGL (+Y) — for Blender")}</option><option value="directx">DirectX (-Y)</option></select></label><label className="checkbox"><input type="checkbox" checked={invertHeight} onChange={(event) => setInvertHeight(event.target.checked)} />{t("高さを反転", "Invert height")}</label><label className="checkbox"><input type="checkbox" checked={invertGreen} onChange={(event) => setInvertGreen(event.target.checked)} />{t("G チャンネルを反転", "Invert green channel")}</label><label className="checkbox"><input type="checkbox" checked={padToSquare} onChange={(event) => setPadToSquare(event.target.checked)} />{t("正方形に補完（高さ 0・中央配置）", "Pad to square (height 0, centered)")}</label></fieldset>
      <p className="hint">{t("出力は Blender 向け RGB 24-bit（アルファなし）です。明るい部分を高く、暗い部分を低く扱います。Blender では Image Texture → Normal Map ノード経由で接続してください。", "Output is 24-bit RGB without alpha for Blender. Bright areas are treated as high and dark areas as low. In Blender, connect it through Image Texture → Normal Map.")}</p>
    </aside>
    <section className="queue">
      <div className="queue-toolbar"><div><h2>{t("ノーマルマップ一覧", "Normal Map Queue")} <span>{jobs.length}</span></h2><p>{notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isProcessing || isLoading}>{t("ファイルを追加", "Add Files")}</button><button className="secondary" onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={!jobs.length || isProcessing || isLoading}>{t("変換ステータスをクリア", "Clear Conversion Status")}</button><button className="primary" onClick={run} disabled={!jobs.length || isProcessing || isLoading}>{isLoading ? t("読込中…", "Loading…") : isProcessing ? t("生成中…", "Generating…") : t("生成を開始", "Start Generation")}</button></div></div>
      {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={isLoading}><strong>{t("高さマップ画像またはフォルダーをドロップ", "Drop height-map images or a folder")}</strong><span>{t("明度から Blender 用ノーマルマップを生成", "Generate Blender normal maps from luminance")}</span></button> : <><div className="table-header normal-table"><span>{t("ファイル", "File")}</span><span>{t("寸法", "Dimensions")}</span><span>{t("出力サイズ", "Output Size")}</span><span>{t("状態", "Status")}</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row normal-table" key={job.id}><div className="file"><Thumbnail path={job.path} alt={t(`${job.name} のプレビュー`, `Preview of ${job.name}`)} fallbackLabel="N" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{job.width} × {job.height}</span><span>{formatBytes(job.outputBytes)}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title={t("エクスプローラーで出力ファイルを表示", "Show output file in File Explorer")} onClick={() => revealItemInDir(job.outputPath!)}>{t("開く", "Open")}</button>}<button title={t("リストから削除", "Remove from list")} onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={isProcessing || isLoading}>{t("リストをクリア", "Clear List")}</button></div><span>{t(`${jobs.filter((job) => job.status === "done").length} 件完了 / ${jobs.filter((job) => job.status === "failed").length} 件失敗`, `${jobs.filter((job) => job.status === "done").length} completed / ${jobs.filter((job) => job.status === "failed").length} failed`)}</span></div></>}
    </section>
  </section>;
}

function numberOrNull(value: string) { const number = Number(value); return Number.isInteger(number) && number > 0 ? number : null; }
