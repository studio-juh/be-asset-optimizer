import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import Thumbnail from "./Thumbnail";
import { InspectProgress, InspectResult, supportedImageFilter, supportedImageShortLabel } from "./imageFiles";

type Status = "ready" | "processing" | "done" | "skipped" | "failed";
type Job = { id: string; path: string; name: string; originalBytes: number; width: number; height: number; status: Status; outputBytes?: number; outputWidth?: number; outputHeight?: number; message?: string; outputPath?: string };
type Metadata = Omit<Job, "id" | "status">;
type Progress = { path: string; status: Status; output_bytes?: number; output_width?: number; output_height?: number; message?: string; output_path?: string };
type AiComponentStatus = { installed: boolean; source: "downloaded" | "bundled" | "missing"; installDir: string; totalBytes: number; downloadBytes: number };
type AiComponentProgress = { phase: string; archiveIndex: number; archiveCount: number; downloadedBytes: number; totalBytes?: number; message: string };

const formatBytes = (bytes?: number) => bytes === undefined ? "—" : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;
const label = (status: Status) => ({ ready: "準備完了", processing: "復元中", done: "完了", skipped: "スキップ", failed: "失敗" })[status];

export default function AiRestore() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [outputDir, setOutputDir] = useState("");
  const [outputScale, setOutputScale] = useState<1 | 2 | 4>(1);
  const [model, setModel] = useState<"natural" | "detailed">("natural");
  const [restorationStrength, setRestorationStrength] = useState(50);
  const [tileSize, setTileSize] = useState(512);
  const [seamlessTiles, setSeamlessTiles] = useState(true);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [componentStatus, setComponentStatus] = useState<AiComponentStatus | null>(null);
  const [isInstalling, setIsInstalling] = useState(false);
  const [componentPanelOpen, setComponentPanelOpen] = useState(false);
  const [componentProgress, setComponentProgress] = useState<AiComponentProgress | null>(null);
  const loadingRequestId = useRef("");
  const componentInstalledRef = useRef(false);
  const [notice, setNotice] = useState(`復元する${supportedImageShortLabel}をここへドロップしてください`);
  const aiAvailable = componentStatus?.installed === true;

  const addPaths = async (paths: string[]) => {
    if (!paths.length) return;
    if (!componentInstalledRef.current) { setNotice("先にAI機能を追加してください"); return; }
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
    const offProgress = listen<Progress>("ai-restore-job-progress", (event) => {
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
    const offComponentProgress = listen<AiComponentProgress>("ai-component-progress", (event) => {
      const update = event.payload;
      setComponentProgress((current) => current?.phase === "downloading" && update.phase === "downloading" && current.downloadedBytes > update.downloadedBytes
        ? { ...update, downloadedBytes: current.downloadedBytes }
        : update);
    });
    void invoke<AiComponentStatus>("get_ai_component_status")
      .then((status) => { componentInstalledRef.current = status.installed; setComponentStatus(status); })
      .catch((error) => setNotice(`AI機能を確認できませんでした: ${String(error)}`));
    const onDrop = (event: Event) => { void addPaths((event as CustomEvent<string[]>).detail); };
    window.addEventListener("smartpng-ai-restore-drop", onDrop);
    return () => { void offProgress.then((off) => off()); void offInspect.then((off) => off()); void offComponentProgress.then((off) => off()); window.removeEventListener("smartpng-ai-restore-drop", onDrop); };
  }, []);

  const chooseFiles = async () => { const selected = await open({ multiple: true, filters: [supportedImageFilter] }); if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]); };
  const chooseOutput = async () => { const selected = await open({ directory: true, multiple: false, title: "出力先フォルダを選択" }); if (typeof selected === "string") setOutputDir(selected); };
  const installComponents = async () => {
    if (isInstalling) return;
    setIsInstalling(true);
    setComponentProgress({ phase: "starting", archiveIndex: 0, archiveCount: 1, downloadedBytes: 0, message: "ダウンロードを準備しています…" });
    try {
      const status = await invoke<AiComponentStatus>("install_ai_components");
      componentInstalledRef.current = status.installed;
      setComponentStatus(status);
      setComponentPanelOpen(false);
      setNotice("AI機能を追加しました");
    } catch (error) {
      setNotice(String(error));
    } finally {
      setIsInstalling(false);
    }
  };
  const cancelComponentInstall = async () => { await invoke("cancel_ai_component_install"); };
  const removeComponents = async () => {
    if (!window.confirm("ダウンロード済みのAIコンポーネントを削除しますか？")) return;
    try {
      const status = await invoke<AiComponentStatus>("remove_ai_components");
      componentInstalledRef.current = status.installed;
      setComponentStatus(status);
      setComponentPanelOpen(false);
      setComponentProgress(null);
      setNotice("AIコンポーネントを削除しました");
    } catch (error) { setNotice(`削除できませんでした: ${String(error)}`); }
  };
  const run = async () => {
    const ready = jobs.filter((job) => ["ready", "failed", "skipped"].includes(job.status));
    if (!componentStatus?.installed) { setNotice("先にAI機能を追加してください"); return; }
    if (!ready.length || isProcessing) return;
    setIsProcessing(true); setNotice(`${ready.length} 枚をAI復元しています…`);
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id) ? { ...job, status: "processing", message: "待機中" } : job));
    try {
      await invoke("process_ai_restore_batch", { paths: ready.map((job) => job.path), settings: { outputDir: outputDir || null, outputScale, model, restorationStrength, tileSize, seamlessTiles } });
      setNotice("AI復元が完了しました");
    } catch (error) { setNotice(`AI復元を開始できませんでした: ${String(error)}`); }
    finally { setIsProcessing(false); }
  };

  return <section className="workspace">
    <aside className="settings ai-restore-settings">
      <h2>AI復元設定</h2>
      <div className={`ai-component-card ${componentStatus?.installed ? "installed" : "missing"}`}>
        {componentStatus?.installed ? <>
          <button className={`component-accordion-toggle ${componentPanelOpen ? "open" : ""}`} onClick={() => setComponentPanelOpen((open) => !open)} aria-expanded={componentPanelOpen}>
            <span className="component-status-text"><strong>AI機能を使用できます</strong><span>{componentStatus.source === "downloaded" ? `追加済みコンポーネント（${formatBytes(componentStatus.totalBytes)}）` : "同梱コンポーネント"}</span></span>
            <span className="component-chevron" aria-hidden="true">⌄</span>
          </button>
          {componentPanelOpen && <div className="component-accordion-content"><small>{componentStatus.source === "downloaded" ? "不要になった場合のみ、追加データを削除できます。" : "このコンポーネントはアプリに同梱されています。"}</small>{componentStatus.source === "downloaded" && <button className="component-remove" onClick={removeComponents}>AIコンポーネントを削除</button>}</div>}
        </> : <>
          <div><strong>{componentStatus === null ? "AI機能を確認中…" : "AI機能は未追加です"}</strong><span>{componentStatus ? `${formatBytes(componentStatus.downloadBytes)}を取得します（保存後${formatBytes(componentStatus.totalBytes)}）` : "必要なデータを確認しています"}</span></div>
          {isInstalling ? <><progress value={componentProgress?.downloadedBytes ?? 0} max={componentProgress?.totalBytes ?? 1} /><small>{componentProgress?.message ?? "準備しています…"}{componentProgress?.archiveIndex ? `（${componentProgress.archiveIndex}/${componentProgress.archiveCount}）` : ""}</small><button onClick={cancelComponentInstall}>キャンセル</button></> : <button className="component-install" onClick={installComponents} disabled={componentStatus === null}>AI機能を追加</button>}
        </>}
      </div>
      <div className={!aiAvailable ? "ai-feature-disabled" : ""} aria-disabled={!aiAvailable}>
        <label>出力先<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder="入力元 / ai_restored" disabled={!aiAvailable} /><button onClick={chooseOutput} disabled={!aiAvailable}>選択</button></div></label>
        <fieldset disabled={!aiAvailable}><legend>復元方法</legend><label>復元モデル<select value={model} onChange={(event) => setModel(event.target.value as "natural" | "detailed")}><option value="natural">自然・忠実（既定）</option><option value="detailed">高精細</option></select></label><label>復元強度 <span className="range-row"><input type="range" min="25" max="100" step="5" value={restorationStrength} onChange={(event) => setRestorationStrength(Number(event.target.value))} /><output>{restorationStrength}%</output></span></label><label>出力倍率<select value={outputScale} onChange={(event) => setOutputScale(Number(event.target.value) as 1 | 2 | 4)}><option value={1}>元の寸法（既定）</option><option value={2}>2倍</option><option value={4}>4倍</option></select></label><label>分割品質<select value={tileSize} onChange={(event) => setTileSize(Number(event.target.value))}><option value={512}>高品質（既定）</option><option value={256}>標準</option><option value={128}>低メモリ</option><option value={0}>自動（互換性）</option></select></label><label className="checkbox"><input type="checkbox" checked={seamlessTiles} onChange={(event) => setSeamlessTiles(event.target.checked)} disabled={!aiAvailable || tileSize === 0} />タイル境界をブレンド（推奨）</label><p className="mode-note">「自然・忠実」は偽の細部を抑え、元画像を50%混ぜます。「高精細」は細部を強く作ります。境界ブレンドは四角いぼかしや継ぎ目を抑えます。</p></fieldset>
        <p className="hint">アルベドや写真素材向けです。ノーマル、粗さ、金属、マスクには直接使わず、必要なら復元したアルベドから作り直してください。元ファイルは変更しません。</p>
      </div>
    </aside>
    <section className={`queue ${!aiAvailable ? "ai-feature-disabled" : ""}`} aria-disabled={!aiAvailable}>
      <div className="queue-toolbar"><div><h2>AI復元リスト <span>{jobs.length}</span></h2><p>{notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={!aiAvailable || isProcessing || isLoading}>ファイルを追加</button><button className="secondary" onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={!aiAvailable || !jobs.length || isProcessing || isLoading}>変換ステータスをクリア</button><button className="primary" onClick={run} disabled={!aiAvailable || !jobs.length || isProcessing || isLoading}>{isLoading ? "読込中…" : isProcessing ? "復元中…" : aiAvailable ? "AI復元を開始" : "AI機能を追加してください"}</button></div></div>
      {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={!aiAvailable || isLoading}><strong>{supportedImageShortLabel}またはフォルダーをドロップ</strong><span>複数枚をまとめてAI復元・拡大</span></button> : <><div className="table-header normal-table"><span>ファイル</span><span>元の寸法</span><span>出力寸法</span><span>状態</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row normal-table" key={job.id}><div className="file"><Thumbnail path={job.path} alt={`${job.name} のプレビュー`} fallbackLabel="AI" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{job.width} × {job.height}</span><span>{job.outputWidth && job.outputHeight ? `${job.outputWidth} × ${job.outputHeight}` : formatBytes(job.outputBytes)}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title="エクスプローラーで出力ファイルを表示" onClick={() => revealItemInDir(job.outputPath!)} disabled={!aiAvailable}>開く</button>}<button title="リストから削除" onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={!aiAvailable || job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={!aiAvailable || isProcessing || isLoading}>リストをクリア</button></div><span>{jobs.filter((job) => job.status === "done").length} 件完了 / {jobs.filter((job) => job.status === "failed").length} 件失敗</span></div></>}
    </section>
  </section>;
}
