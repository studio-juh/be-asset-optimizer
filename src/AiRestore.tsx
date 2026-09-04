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
type AiComponentStatus = { installed: boolean; source: "downloaded" | "bundled" | "missing"; installDir: string; totalBytes: number; downloadBytes: number };
type AiComponentProgress = { phase: string; archiveIndex: number; archiveCount: number; downloadedBytes: number; totalBytes?: number; message: string };

const formatBytes = (bytes?: number) => bytes === undefined ? "—" : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;
const label = (status: Status) => ({ ready: t("準備完了", "Ready"), processing: t("復元中", "Restoring"), done: t("完了", "Done"), skipped: t("スキップ", "Skipped"), failed: t("失敗", "Failed") })[status];

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
  const [notice, setNotice] = useState(t(`復元する${supportedImageShortLabel}をここへドロップしてください`, `Drop ${supportedImageShortLabel} to restore here`));
  const aiAvailable = componentStatus?.installed === true;

  const addPaths = async (paths: string[]) => {
    if (!paths.length) return;
    if (!componentInstalledRef.current) { setNotice(t("先にAI機能を追加してください", "Add the AI feature first")); return; }
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
    const offProgress = listen<Progress>("ai-restore-job-progress", (event) => {
      const update = event.payload;
      setJobs((current) => current.map((job) => job.path === update.path ? { ...job, status: update.status, outputBytes: update.output_bytes, outputWidth: update.output_width, outputHeight: update.output_height, message: update.message ? localizedDetail(update.message, update.status === "done" ? "AI restoration completed" : update.status === "failed" ? "AI restoration failed" : "Restoring with AI") : undefined, outputPath: update.output_path } : job));
    });
    const offInspect = listen<InspectProgress<Metadata>>("inspect-files-progress", (event) => {
      const update = event.payload;
      if (update.requestId !== loadingRequestId.current) return;
      if (update.file) setJobs((current) => current.some((job) => job.path.toLowerCase() === update.file!.path.toLowerCase())
        ? current : [...current, { ...update.file!, id: crypto.randomUUID(), status: "ready" as const }]);
      setNotice(t(`${update.completed} / ${update.total} 枚を読み込んでいます…`, `Loading ${update.completed} / ${update.total}…`));
    });
    const offComponentProgress = listen<AiComponentProgress>("ai-component-progress", (event) => {
      const update = event.payload;
      setComponentProgress((current) => current?.phase === "downloading" && update.phase === "downloading" && current.downloadedBytes > update.downloadedBytes
        ? { ...update, downloadedBytes: current.downloadedBytes }
        : update);
    });
    void invoke<AiComponentStatus>("get_ai_component_status")
      .then((status) => { componentInstalledRef.current = status.installed; setComponentStatus(status); })
      .catch((error) => setNotice(t(`AI機能を確認できませんでした: ${String(error)}`, `Could not check the AI feature: ${localizedDetail(error, "The operation failed")}`)));
    const onDrop = (event: Event) => { void addPaths((event as CustomEvent<string[]>).detail); };
    const onMenuOpen = () => componentInstalledRef.current ? void chooseFiles() : setNotice(t("先にAI機能を追加してください", "Add the AI feature first"));
    window.addEventListener("smartpng-ai-restore-drop", onDrop);
    window.addEventListener("smartpng-menu-open", onMenuOpen);
    return () => { void offProgress.then((off) => off()); void offInspect.then((off) => off()); void offComponentProgress.then((off) => off()); window.removeEventListener("smartpng-ai-restore-drop", onDrop); window.removeEventListener("smartpng-menu-open", onMenuOpen); };
  }, []);

  const chooseFiles = async () => { const selected = await open({ multiple: true, filters: [supportedImageFilter] }); if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]); };
  const chooseOutput = async () => { const selected = await open({ directory: true, multiple: false, title: t("出力先フォルダを選択", "Choose output folder") }); if (typeof selected === "string") setOutputDir(selected); };
  const installComponents = async () => {
    if (isInstalling) return;
    setIsInstalling(true);
    setComponentProgress({ phase: "starting", archiveIndex: 0, archiveCount: 1, downloadedBytes: 0, message: t("ダウンロードを準備しています…", "Preparing download…") });
    try {
      const status = await invoke<AiComponentStatus>("install_ai_components");
      componentInstalledRef.current = status.installed;
      setComponentStatus(status);
      setComponentPanelOpen(false);
      setNotice(t("AI機能を追加しました", "AI feature added"));
    } catch (error) {
      setNotice(localizedDetail(error, "Could not add the AI feature"));
    } finally {
      setIsInstalling(false);
    }
  };
  const cancelComponentInstall = async () => { await invoke("cancel_ai_component_install"); };
  const removeComponents = async () => {
    if (!window.confirm(t("ダウンロード済みのAIコンポーネントを削除しますか？", "Remove the downloaded AI component?"))) return;
    try {
      const status = await invoke<AiComponentStatus>("remove_ai_components");
      componentInstalledRef.current = status.installed;
      setComponentStatus(status);
      setComponentPanelOpen(false);
      setComponentProgress(null);
      setNotice(t("AIコンポーネントを削除しました", "AI component removed"));
    } catch (error) { setNotice(t(`削除できませんでした: ${String(error)}`, `Could not remove the AI component: ${localizedDetail(error, "The operation failed")}`)); }
  };
  const run = async () => {
    const ready = jobs.filter((job) => ["ready", "failed", "skipped"].includes(job.status));
    if (!componentStatus?.installed) { setNotice(t("先にAI機能を追加してください", "Add the AI feature first")); return; }
    if (!ready.length || isProcessing) return;
    setIsProcessing(true); setNotice(t(`${ready.length} 枚をAI復元しています…`, `Restoring ${ready.length} image${ready.length === 1 ? "" : "s"} with AI…`));
    setJobs((current) => current.map((job) => ready.some((item) => item.id === job.id) ? { ...job, status: "processing", message: t("待機中", "Waiting") } : job));
    try {
      await invoke("process_ai_restore_batch", { paths: ready.map((job) => job.path), settings: { outputDir: outputDir || null, outputScale, model, restorationStrength, tileSize, seamlessTiles } });
      setNotice(t("AI復元が完了しました", "AI restoration completed"));
    } catch (error) { setNotice(t(`AI復元を開始できませんでした: ${String(error)}`, `Could not start AI restoration: ${localizedDetail(error, "The operation failed")}`)); }
    finally { setIsProcessing(false); }
  };

  return <section className="workspace">
    <aside className="settings ai-restore-settings">
      <h2>{t("AI復元設定", "AI Restoration Settings")}</h2>
      <div className={`ai-component-card ${componentStatus?.installed ? "installed" : "missing"}`}>
        {componentStatus?.installed ? <>
          <button className={`component-accordion-toggle ${componentPanelOpen ? "open" : ""}`} onClick={() => setComponentPanelOpen((open) => !open)} aria-expanded={componentPanelOpen}>
            <span className="component-status-text"><strong>{t("AI機能を使用できます", "AI feature is available")}</strong><span>{componentStatus.source === "downloaded" ? t(`追加済みコンポーネント（${formatBytes(componentStatus.totalBytes)}）`, `Downloaded component (${formatBytes(componentStatus.totalBytes)})`) : t("同梱コンポーネント", "Bundled component")}</span></span>
            <span className="component-chevron" aria-hidden="true">⌄</span>
          </button>
          {componentPanelOpen && <div className="component-accordion-content"><small>{componentStatus.source === "downloaded" ? t("不要になった場合のみ、追加データを削除できます。", "You can remove the downloaded data when it is no longer needed.") : t("このコンポーネントはアプリに同梱されています。", "This component is bundled with the app.")}</small>{componentStatus.source === "downloaded" && <button className="component-remove" onClick={removeComponents}>{t("AIコンポーネントを削除", "Remove AI Component")}</button>}</div>}
        </> : <>
          <div><strong>{componentStatus === null ? t("AI機能を確認中…", "Checking AI feature…") : t("AI機能は未追加です", "AI feature is not installed")}</strong><span>{componentStatus ? t(`${formatBytes(componentStatus.downloadBytes)}を取得します（保存後${formatBytes(componentStatus.totalBytes)}）`, `Downloads ${formatBytes(componentStatus.downloadBytes)} (${formatBytes(componentStatus.totalBytes)} installed)`) : t("必要なデータを確認しています", "Checking required data")}</span></div>
          {isInstalling ? <><progress value={componentProgress?.downloadedBytes ?? 0} max={componentProgress?.totalBytes ?? 1} /><small>{componentProgress ? localizedDetail(componentProgress.message, componentProgressLabel(componentProgress.phase)) : t("準備しています…", "Preparing…")}{componentProgress?.archiveIndex ? t(`（${componentProgress.archiveIndex}/${componentProgress.archiveCount}）`, ` (${componentProgress.archiveIndex}/${componentProgress.archiveCount})`) : ""}</small><button onClick={cancelComponentInstall}>{t("キャンセル", "Cancel")}</button></> : <button className="component-install" onClick={installComponents} disabled={componentStatus === null}>{t("AI機能を追加", "Add AI Feature")}</button>}
        </>}
      </div>
      <div className={!aiAvailable ? "ai-feature-disabled" : ""} aria-disabled={!aiAvailable}>
        <label>{t("出力先", "Output folder")}<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder={t("入力元 / ai_restored", "Source folder / ai_restored")} disabled={!aiAvailable} /><button onClick={chooseOutput} disabled={!aiAvailable}>{t("選択", "Choose")}</button></div></label>
        <fieldset disabled={!aiAvailable}><legend>{t("復元方法", "Restoration")}</legend><label>{t("復元モデル", "Restoration model")}<select value={model} onChange={(event) => setModel(event.target.value as "natural" | "detailed")}><option value="natural">{t("自然・忠実（既定）", "Natural and faithful (default)")}</option><option value="detailed">{t("高精細", "Detailed")}</option></select></label><label>{t("復元強度", "Restoration strength")} <span className="range-row"><input type="range" min="25" max="100" step="5" value={restorationStrength} onChange={(event) => setRestorationStrength(Number(event.target.value))} /><output>{restorationStrength}%</output></span></label><label>{t("出力倍率", "Output scale")}<select value={outputScale} onChange={(event) => setOutputScale(Number(event.target.value) as 1 | 2 | 4)}><option value={1}>{t("元の寸法（既定）", "Original size (default)")}</option><option value={2}>{t("2倍", "2×")}</option><option value={4}>{t("4倍", "4×")}</option></select></label><label>{t("分割品質", "Tile quality")}<select value={tileSize} onChange={(event) => setTileSize(Number(event.target.value))}><option value={512}>{t("高品質（既定）", "High quality (default)")}</option><option value={256}>{t("標準", "Standard")}</option><option value={128}>{t("低メモリ", "Low memory")}</option><option value={0}>{t("自動（互換性）", "Automatic (compatibility)")}</option></select></label><label className="checkbox"><input type="checkbox" checked={seamlessTiles} onChange={(event) => setSeamlessTiles(event.target.checked)} disabled={!aiAvailable || tileSize === 0} />{t("タイル境界をブレンド（推奨）", "Blend tile boundaries (recommended)")}</label><p className="mode-note">{t("「自然・忠実」は偽の細部を抑え、元画像を50%混ぜます。「高精細」は細部を強く作ります。境界ブレンドは四角いぼかしや継ぎ目を抑えます。", "Natural and faithful suppresses invented detail and blends in 50% of the source. Detailed creates stronger detail. Boundary blending reduces square blur patches and seams.")}</p></fieldset>
        <p className="hint">{t("アルベドや写真素材向けです。ノーマル、粗さ、金属、マスクには直接使わず、必要なら復元したアルベドから作り直してください。元ファイルは変更しません。", "Designed for albedo and photographic assets. Do not apply it directly to normal, roughness, metallic, or mask maps; regenerate those from the restored albedo if needed. Source files are never modified.")}</p>
      </div>
    </aside>
    <section className={`queue ${!aiAvailable ? "ai-feature-disabled" : ""}`} aria-disabled={!aiAvailable}>
      <div className="queue-toolbar"><div><h2>{t("AI復元リスト", "AI Restoration Queue")} <span>{jobs.length}</span></h2><p>{notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={!aiAvailable || isProcessing || isLoading}>{t("ファイルを追加", "Add Files")}</button><button className="secondary" onClick={() => setJobs((items) => items.map(({ outputBytes, outputWidth, outputHeight, message, outputPath, ...job }) => ({ ...job, status: "ready" })))} disabled={!aiAvailable || !jobs.length || isProcessing || isLoading}>{t("変換ステータスをクリア", "Clear Conversion Status")}</button><button className="primary" onClick={run} disabled={!aiAvailable || !jobs.length || isProcessing || isLoading}>{isLoading ? t("読込中…", "Loading…") : isProcessing ? t("復元中…", "Restoring…") : aiAvailable ? t("AI復元を開始", "Start AI Restoration") : t("AI機能を追加してください", "Add the AI Feature")}</button></div></div>
      {jobs.length === 0 ? <button className="drop-zone" onClick={chooseFiles} disabled={!aiAvailable || isLoading}><strong>{t(`${supportedImageShortLabel}またはフォルダーをドロップ`, `Drop ${supportedImageShortLabel} or a folder`)}</strong><span>{t("複数枚をまとめてAI復元・拡大", "Batch restore and upscale images with AI")}</span></button> : <><div className="table-header normal-table"><span>{t("ファイル", "File")}</span><span>{t("元の寸法", "Source Size")}</span><span>{t("出力寸法", "Output Size")}</span><span>{t("状態", "Status")}</span><span></span></div><div className="rows">{jobs.map((job) => <div className="job-row normal-table" key={job.id}><div className="file"><Thumbnail path={job.path} alt={t(`${job.name} のプレビュー`, `Preview of ${job.name}`)} fallbackLabel="AI" /><div><strong title={job.path}>{job.name}</strong><small>{job.message || job.path}</small></div></div><span>{job.width} × {job.height}</span><span>{job.outputWidth && job.outputHeight ? `${job.outputWidth} × ${job.outputHeight}` : formatBytes(job.outputBytes)}</span><span className={`badge ${job.status}`}>{label(job.status)}</span><div className="row-actions">{job.outputPath && <button title={t("エクスプローラーで出力ファイルを表示", "Show output file in File Explorer")} onClick={() => revealItemInDir(job.outputPath!)} disabled={!aiAvailable}>{t("開く", "Open")}</button>}<button title={t("リストから削除", "Remove from list")} onClick={() => setJobs((items) => items.filter((item) => item.id !== job.id))} disabled={!aiAvailable || job.status === "processing"}>×</button></div></div>)}</div><div className="queue-footer"><div className="footer-actions"><button onClick={() => setJobs([])} disabled={!aiAvailable || isProcessing || isLoading}>{t("リストをクリア", "Clear List")}</button></div><span>{t(`${jobs.filter((job) => job.status === "done").length} 件完了 / ${jobs.filter((job) => job.status === "failed").length} 件失敗`, `${jobs.filter((job) => job.status === "done").length} completed / ${jobs.filter((job) => job.status === "failed").length} failed`)}</span></div></>}
    </section>
  </section>;
}

function componentProgressLabel(phase: string) {
  return ({
    starting: "Preparing download…",
    downloading: "Downloading AI component…",
    retrying: "Switching to the backup source…",
    verifying: "Verifying AI component…",
    done: "AI feature added",
  } as Record<string, string>)[phase] ?? "Preparing AI component…";
}
