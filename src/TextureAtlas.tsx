import { DragEvent, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { InspectProgress, InspectResult, supportedImageFilter } from "./imageFiles";
import { localizedDetail, t } from "./i18n";
import { addTextures, AtlasSlot, emptyAtlas, splitSlot, Texture, ResolutionWarning, resolutionWarnings } from "./atlasLayout";

type AtlasResult = { outputPath: string; outputBytes: number; width: number; height: number };
type NativeDrag = { type: "enter" | "drop"; paths: string[]; position: { x: number; y: number } } | { type: "over"; position: { x: number; y: number } } | { type: "leave" };
const dragType = "application/x-smartpng-atlas-slot";

function Tile({ slot, warning, disabled, highlighted, onAdd, onRemove, onSplit, onSwap, onHighlight }: {
  slot: AtlasSlot; disabled: boolean; highlighted: boolean;
  warning?: ResolutionWarning;
  onAdd: () => void; onRemove: () => void; onSplit: () => void; onSwap: (source: string) => void; onHighlight: (id?: string) => void;
}) {
  const [preview, setPreview] = useState<string>();
  const warningText = warning && slot.texture ? t(
    `解像度不足：元画像 ${slot.texture.width} × ${slot.texture.height} px／配置枠 ${warning.width} × ${warning.height} px。縦横比を維持して枠に合わせるには約${warning.scale.toFixed(2)}倍の拡大が必要です。現在は拡大せず、原寸で中央配置します。`,
    `Low resolution: source ${slot.texture.width} × ${slot.texture.height} px / slot ${warning.width} × ${warning.height} px. Fitting with the original aspect ratio requires approximately ${warning.scale.toFixed(2)}× upscaling. Currently centered at original size without upscaling.`,
  ) : undefined;
  useEffect(() => {
    let active = true; setPreview(undefined);
    if (slot.texture) void invoke<string>("create_preview", { path: slot.texture.path, size: 256 }).then(data => { if (active) setPreview(data); }).catch(() => {});
    return () => { active = false; };
  }, [slot.texture?.path]);
  const start = (event: DragEvent) => {
    if (disabled) { event.preventDefault(); return; }
    event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData(dragType, slot.id); onHighlight(slot.id);
  };
  return <div data-atlas-slot={slot.id} className={`atlas-slot ${slot.size === 1 ? "atlas-small" : ""} ${slot.texture ? "filled" : ""} ${highlighted ? "drop-target" : ""}`}
    style={{ gridColumn: `${slot.x + 1} / span ${slot.size}`, gridRow: `${slot.y + 1} / span ${slot.size}` }}
    onDragOver={event => { if (!disabled && event.dataTransfer.types.includes(dragType)) { event.preventDefault(); onHighlight(slot.id); } }}
    onDrop={event => { if (!disabled && event.dataTransfer.types.includes(dragType)) { event.preventDefault(); onSwap(event.dataTransfer.getData(dragType)); onHighlight(); } }}>
    <div className="atlas-tile-bar"><strong>{slot.id.split(".").map(n => Number(n) + 1).join(".")}</strong><div className="atlas-actions">
      {slot.size === 2 && <button title={t("この枠を4分割（画像は左上へ移動）", "Split slot into four (image moves to top left)")} onClick={onSplit} disabled={disabled}>⊞</button>}
      {slot.texture && <button title={t("画像を外す", "Remove image")} onClick={onRemove} disabled={disabled}>×</button>}
    </div></div>
    {slot.texture ? <><div className="atlas-tile-preview" draggable={!disabled} onDragStart={start} onDragEnd={() => onHighlight()}>
      {preview ? <img src={preview} alt={slot.texture.name} draggable={false} /> : <span>{t("読込中…", "Loading…")}</span>}
      {warningText && <span className="atlas-resolution-warning" title={warningText} aria-label={warningText} tabIndex={0}>⚠ {t("解像度不足", "Low resolution")}</span>}
    </div><div className="atlas-tile-caption" title={`${slot.texture.path}\n${slot.texture.width} × ${slot.texture.height}`}><strong>{slot.texture.name}</strong></div></>
      : <button className="atlas-empty atlas-black" onClick={onAdd} disabled={disabled} title={t("画像を追加（空き枠は黒で出力）", "Add image (empty slots export as black)")}><strong>＋</strong><small>{t("黒", "Black")}</small></button>}
  </div>;
}

export default function TextureAtlas() {
  const [slots, setSlots] = useState(emptyAtlas);
  const slotsRef = useRef(slots);
  const [outputDir, setOutputDir] = useState("");
  const [square, setSquare] = useState(true);
  const [resolution, setResolution] = useState("");
  const [custom, setCustom] = useState("");
  const [padding, setPadding] = useState(2);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const requestRef = useRef("");
  const alive = useRef(true);
  const [result, setResult] = useState<AtlasResult>();
  const [target, setTarget] = useState<string>();
  const [notice, setNotice] = useState(t("画像を任意の枠へドロップしてください。空き枠は黒で埋めます。", "Drop images into any slot. Empty slots will be black."));
  const count = slots.filter(slot => slot.texture).length;
  const selectedResolution = resolution === "custom" ? (custom.trim() ? Number(custom) : NaN) : resolution ? Number(resolution) : null;
  const warnings = resolutionWarnings(slots, square, selectedResolution, padding);
  const commit = (next: AtlasSlot[]) => { slotsRef.current = next; setSlots(next); setResult(undefined); };
  const lock = (value: boolean) => { busyRef.current = value; if (alive.current) setBusy(value); };
  const addPaths = async (paths: string[], id?: string) => {
    if (!paths.length || busyRef.current) return;
    lock(true); const requestId = crypto.randomUUID(); requestRef.current = requestId;
    setNotice(t("画像を確認しています…", "Inspecting images…"));
    try {
      const inspected = await invoke<InspectResult<Texture>>("inspect_files", { paths, requestId, maxFiles: 17 });
      if (!alive.current) return;
      const added = addTextures(slotsRef.current, inspected.files, id);
      commit(added.slots);
      setNotice(t(`${added.added} 枚を追加。未追加 ${inspected.files.length - added.added} 枚、読込失敗 ${inspected.failures.length} 件（最大16枚）。`, `${added.added} added; ${inspected.files.length - added.added} not added, ${inspected.failures.length} failed (maximum 16 images).`));
    } catch (error) { if (alive.current) setNotice(localizedDetail(error, "Could not add images")); }
    finally { requestRef.current = ""; lock(false); }
  };
  const chooseFiles = async (id?: string) => {
    if (busyRef.current) return;
    const selected = await open({ multiple: true, filters: [supportedImageFilter] });
    if (selected && alive.current) await addPaths(Array.isArray(selected) ? selected : [selected], id);
  };
  const handlers = useRef({ addPaths, chooseFiles }); handlers.current = { addPaths, chooseFiles };
  useEffect(() => {
    alive.current = true;
    const off = listen<InspectProgress<Texture>>("inspect-files-progress", event => {
      if (alive.current && event.payload.requestId === requestRef.current) setNotice(t(`${event.payload.completed} / ${event.payload.total} 枚を読込中…`, `Loading ${event.payload.completed} / ${event.payload.total}…`));
    });
    const onDrag = (event: Event) => {
      const detail = (event as CustomEvent<NativeDrag>).detail;
      if (detail.type === "leave" || busyRef.current) { setTarget(undefined); return; }
      const ratio = window.devicePixelRatio || 1;
      const id = document.elementFromPoint(detail.position.x / ratio, detail.position.y / ratio)?.closest<HTMLElement>("[data-atlas-slot]")?.dataset.atlasSlot;
      setTarget(id);
      if (detail.type === "drop") { setTarget(undefined); void handlers.current.addPaths(detail.paths, id); }
    };
    const onOpen = () => { void handlers.current.chooseFiles(); };
    window.addEventListener("smartpng-atlas-drag", onDrag); window.addEventListener("smartpng-atlas-menu-open", onOpen);
    return () => { alive.current = false; void off.then(stop => stop()).catch(() => {}); window.removeEventListener("smartpng-atlas-drag", onDrag); window.removeEventListener("smartpng-atlas-menu-open", onOpen); };
  }, []);
  const swap = (source: string, destination: string) => {
    if (busyRef.current || source === destination) return;
    const current = slotsRef.current;
    const a = current.find(slot => slot.id === source), b = current.find(slot => slot.id === destination);
    if (!a || !b) return;
    commit(current.map(slot => slot.id === source ? { ...slot, texture: b.texture } : slot.id === destination ? { ...slot, texture: a.texture } : slot));
  };
  const create = async () => {
    if (busyRef.current || !count) return;
    const value = resolution === "custom" ? custom : resolution;
    const numeric = value === "" ? null : Number(value);
    if (square && ((resolution === "custom" && !value) || (numeric !== null && (!Number.isInteger(numeric) || numeric < 16 || numeric > 4096)))) {
      setNotice(t("1大枠の解像度を16〜4096 pxで指定してください", "Enter a large-slot resolution between 16 and 4096 px")); return;
    }
    lock(true); setResult(undefined); setNotice(t("アトラスを作成しています…", "Creating atlas…"));
    try {
      const created = await invoke<AtlasResult>("create_texture_atlas", { slots: slotsRef.current.map(slot => ({ x: slot.x, y: slot.y, size: slot.size, path: slot.texture?.path ?? null })), settings: { outputDir: outputDir || null, padToSquare: square, squareResolution: square ? numeric : null, padding } });
      if (alive.current) { setResult(created); setNotice(t("テクスチャアトラスを作成しました", "Texture atlas created")); }
    } catch (error) { if (alive.current) setNotice(localizedDetail(error, "Could not create atlas. Try a smaller slot resolution.")); }
    finally { lock(false); }
  };
  return <section className="workspace">
    <aside className="settings atlas-settings"><h2>{t("アトラス設定", "Atlas Settings")}</h2>
      <label>{t("出力先", "Output folder")}<div className="path-row"><input disabled={busy} value={outputDir} onChange={event => setOutputDir(event.target.value)} placeholder={t("最初の画像のフォルダー / atlas", "First image folder / atlas")} /><button disabled={busy} onClick={async () => { const dir = await open({ directory: true, multiple: false }); if (typeof dir === "string") setOutputDir(dir); }}>{t("選択", "Choose")}</button></div></label>
      <fieldset disabled={busy}><legend>{t("配置", "Layout")}</legend>
        <label>{t("端の色を延長する余白", "Edge Extrusion Padding")}<select value={padding} onChange={event => { setPadding(Number(event.target.value)); setResult(undefined); }}>{[0,1,2,4,8,16,32,64].map(value=><option key={value} value={value}>{value} px</option>)}</select></label>
        <div className="actions atlas-layout-actions"><button type="button" onClick={() => { let next = slotsRef.current; for (const slot of [...next]) if (slot.size === 2) next = splitSlot(next, slot.id); commit(next); }}>{t("全枠を4分割（4×4）", "Split all slots (4×4)")}</button></div>
        <label className="checkbox"><input type="checkbox" checked={square} onChange={event => { setSquare(event.target.checked); setResult(undefined); }} />{t("各画像を正方形として補完", "Pad images to square cells")}</label>
        {square && <label>{t("1大枠の解像度", "Resolution per large slot")}<select value={resolution} onChange={event => { setResolution(event.target.value); setResult(undefined); }}><option value="">{t("自動", "Automatic")}</option>{[256,512,1024,2048,4096].map(size => <option key={size} value={size}>{size} × {size}</option>)}<option value="custom">Custom…</option></select>{resolution === "custom" && <input inputMode="numeric" value={custom} onChange={event => { setCustom(event.target.value); setResult(undefined); }} placeholder="16–4096 px" />}</label>}
      </fieldset>
    </aside>
    <section className="queue"><div className="queue-toolbar"><div><h2>{t("テクスチャアトラス", "Texture Atlas")} <span>{count} / {slots.length}</span></h2><p>{result ? `${result.width} × ${result.height} · ${(result.outputBytes / 1024 / 1024).toFixed(2)} MB` : notice}</p></div><div className="actions"><button className="secondary" onClick={() => void chooseFiles()} disabled={busy || count === 16}>{t("画像を追加", "Add Images")}</button><button className="primary" onClick={create} disabled={busy || !count}>{busy ? t("処理中…", "Processing…") : t("アトラスを作成", "Create Atlas")}</button></div></div>
      <div className="atlas-grid atlas-flexible">{slots.map(slot => <Tile key={slot.id} slot={slot} warning={warnings.get(slot.id)} disabled={busy} highlighted={target === slot.id}
        onAdd={() => void chooseFiles(slot.id)} onRemove={() => { if (!busyRef.current) commit(slotsRef.current.map(item => item.id === slot.id ? { ...item, texture: undefined } : item)); }}
        onSplit={() => { if (!busyRef.current) commit(splitSlot(slotsRef.current, slot.id)); }} onSwap={source => swap(source, slot.id)} onHighlight={setTarget} />)}</div>
      <div className="queue-footer"><div className="footer-actions"><button disabled={busy} onClick={() => { commit(emptyAtlas()); setNotice(t("配置を2×2へ戻しました", "Reset to 2×2 layout")); }}>{t("クリアして2×2へ戻す", "Clear and reset to 2×2")}</button>{result && <button onClick={() => revealItemInDir(result.outputPath)}>{t("出力を開く", "Open Output")}</button>}</div><span>{t("空き枠は黒", "Empty slots are black")}</span></div>
    </section>
  </section>;
}
