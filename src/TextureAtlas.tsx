import { DragEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { InspectProgress, InspectResult, supportedImageFilter, supportedImageShortLabel } from "./imageFiles";
import { localizedDetail, t } from "./i18n";

type Texture = { path: string; name: string; originalBytes: number; width: number; height: number };
type AtlasResult = { outputPath: string; outputBytes: number; width: number; height: number };
type NativeAtlasDrag = { type: "enter" | "drop"; paths: string[]; position: { x: number; y: number } } | { type: "over"; position: { x: number; y: number } } | { type: "leave" };

const slots = [t("左上", "Top Left"), t("右上", "Top Right"), t("左下", "Bottom Left"), t("右下", "Bottom Right")];
const emptyTextures = (): Array<Texture | undefined> => [undefined, undefined, undefined, undefined];
const formatBytes = (bytes: number) => bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;

function AtlasTile({ slot, texture, index, disabled, highlighted, onMove, onRemove, onAdd, onInternalDragStart, onInternalDragOver, onInternalDragEnd, onInternalDrop }: { slot: string; texture?: Texture; index: number; disabled: boolean; highlighted: boolean; onMove: (direction: -1 | 1) => void; onRemove: () => void; onAdd: () => void; onInternalDragStart: () => void; onInternalDragOver: () => void; onInternalDragEnd: () => void; onInternalDrop: (source: number) => void }) {
  const [preview, setPreview] = useState<string>();

  useEffect(() => {
    let active = true;
    setPreview(undefined);
    if (texture) void invoke<string>("create_preview", { path: texture.path, size: 512 }).then((data) => { if (active) setPreview(data); }).catch(() => {});
    return () => { active = false; };
  }, [texture?.path]);

  const acceptInternalDrop = (event: DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer.types.includes("application/x-smartpng-atlas-index")) return;
    event.preventDefault();
    onInternalDragOver();
  };
  const dropInternal = (event: DragEvent<HTMLDivElement>) => {
    const source = Number(event.dataTransfer.getData("application/x-smartpng-atlas-index"));
    if (Number.isInteger(source)) { event.preventDefault(); onInternalDrop(source); }
  };
  const startInternalDrag = (event: DragEvent<HTMLDivElement>) => {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("application/x-smartpng-atlas-index", String(index));
    onInternalDragStart();
  };

  return <div data-atlas-index={index} className={`atlas-slot ${texture ? "filled" : ""} ${highlighted ? "drop-target" : ""}`} onDragOver={acceptInternalDrop} onDrop={dropInternal}>
    <div className="atlas-tile-bar"><strong>{slot}</strong>{texture && <div className="atlas-actions"><button title={t("前の枠と入れ替え", "Swap with previous slot")} onClick={() => onMove(-1)} disabled={disabled || index === 0}>←</button><button title={t("次の枠と入れ替え", "Swap with next slot")} onClick={() => onMove(1)} disabled={disabled || index === 3}>→</button><button title={t("この画像を外す", "Remove this image")} onClick={onRemove} disabled={disabled}>×</button></div>}</div>
    {texture ? <><div className="atlas-tile-preview" draggable={!disabled} onDragStart={startInternalDrag} onDragEnd={onInternalDragEnd}>{preview ? <img src={preview} alt={`${slot}: ${texture.name}`} draggable={false} /> : <span>{t("読込中…", "Loading…")}</span>}</div><div className="atlas-tile-caption" title={texture.path}><strong>{texture.name}</strong><small>{texture.width} × {texture.height}</small></div></> : <button className="atlas-empty" onClick={onAdd} disabled={disabled}><strong>{t("ここへ画像をドロップ", "Drop an image here")}</strong><small>{t("主要な静止画形式", "Common still-image formats")}</small></button>}
  </div>;
}

export default function TextureAtlas() {
  const [textures, setTextures] = useState<Array<Texture | undefined>>(emptyTextures);
  const [outputDir, setOutputDir] = useState("");
  const [padToSquare, setPadToSquare] = useState(true);
  const [squareResolution, setSquareResolution] = useState("");
  const [customSquareResolution, setCustomSquareResolution] = useState("");
  const [result, setResult] = useState<AtlasResult>();
  const [isCreating, setIsCreating] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const loadingRequestId = useRef("");
  const [dropTarget, setDropTarget] = useState<number>();
  const [notice, setNotice] = useState(t(`${supportedImageShortLabel}を任意の枠へドロップしてください。`, `Drop ${supportedImageShortLabel} into any slot.`));
  const textureCount = useMemo(() => textures.filter(Boolean).length, [textures]);

  const addPaths = async (paths: string[], targetIndex?: number) => {
    if (!paths.length) return;
    if (loadingRequestId.current) { setNotice(t("現在の画像を読み込み中です", "Images are already being loaded")); return; }
    const requestId = crypto.randomUUID();
    loadingRequestId.current = requestId;
    setIsLoading(true);
    setNotice(t("画像を確認しています…", "Inspecting images…"));
    try {
      const result = await invoke<InspectResult<Texture>>("inspect_files", { paths, requestId, maxFiles: 4 });
      setTextures((current) => {
        const next = [...current];
        let added = 0;
        for (const file of result.files) {
          const duplicateIndex = next.findIndex((item) => item?.path.toLowerCase() === file.path.toLowerCase());
          if (duplicateIndex >= 0) next[duplicateIndex] = undefined;
          const destination = added === 0 && targetIndex !== undefined ? targetIndex : next.findIndex((item) => !item);
          if (destination < 0) continue;
          next[destination] = file;
          added += 1;
        }
        const count = next.filter(Boolean).length;
        if (added) setNotice(t(`${count} / 4 枚を登録しました${result.failures.length ? `（${result.failures.length} 枚は読込失敗）` : ""}`, `Added ${count} / 4 images${result.failures.length ? ` (${result.failures.length} failed)` : ""}`));
        else if (result.failures.length) setNotice(t(`読み込めませんでした: ${result.failures[0].message}`, `Could not load the selected image: ${localizedDetail(result.failures[0].message, "The file could not be inspected")}`));
        else setNotice(t("追加できる空き枠がないか、対象画像がありません", "No empty slot or supported image is available"));
        return next;
      });
      setResult(undefined);
    } catch (error) { setNotice(t(`追加できませんでした: ${String(error)}`, `Could not add images: ${localizedDetail(error, "The operation failed")}`)); }
    finally {
      if (loadingRequestId.current === requestId) loadingRequestId.current = "";
      setIsLoading(false);
    }
  };

  const atlasIndexAt = (position: { x: number; y: number }) => {
    const ratio = window.devicePixelRatio || 1;
    const element = document.elementFromPoint(position.x / ratio, position.y / ratio);
    const slot = element?.closest<HTMLElement>("[data-atlas-index]");
    const index = Number(slot?.dataset.atlasIndex);
    return Number.isInteger(index) && index >= 0 && index < 4 ? index : undefined;
  };

  useEffect(() => {
    const offInspect = listen<InspectProgress<Texture>>("inspect-files-progress", (event) => {
      const update = event.payload;
      if (update.requestId === loadingRequestId.current) setNotice(t(`${update.completed} / ${update.total} 枚を読み込んでいます…`, `Loading ${update.completed} / ${update.total}…`));
    });
    const onNativeDrag = (event: Event) => {
      const detail = (event as CustomEvent<NativeAtlasDrag>).detail;
      if (detail.type === "leave") { setDropTarget(undefined); return; }
      const target = atlasIndexAt(detail.position);
      setDropTarget(target);
      if (detail.type === "drop") { setDropTarget(undefined); void addPaths(detail.paths, target); }
    };
    const onMenuOpen = () => { void chooseFiles(); };
    window.addEventListener("smartpng-atlas-drag", onNativeDrag);
    window.addEventListener("smartpng-menu-open", onMenuOpen);
    return () => { void offInspect.then((off) => off()); window.removeEventListener("smartpng-atlas-drag", onNativeDrag); window.removeEventListener("smartpng-menu-open", onMenuOpen); };
  }, []);

  const chooseFiles = async () => {
    const selected = await open({ multiple: true, filters: [supportedImageFilter] });
    if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]);
  };
  const chooseOutput = async () => {
    const selected = await open({ directory: true, multiple: false, title: t("出力先フォルダを選択", "Choose output folder") });
    if (typeof selected === "string") setOutputDir(selected);
  };
  const swap = (source: number, target: number) => setTextures((items) => {
    if (source === target || target < 0 || target > 3) return items;
    const copy = [...items];
    [copy[source], copy[target]] = [copy[target], copy[source]];
    setResult(undefined);
    return copy;
  });
  const create = async () => {
    const complete = textures.every((texture): texture is Texture => Boolean(texture));
    if (!complete || isCreating) { setNotice(t("4つの枠すべてに画像を指定してください", "Add an image to all four slots")); return; }
    setIsCreating(true); setResult(undefined); setNotice(t("アトラスを作成しています…", "Creating texture atlas…"));
    try {
      const created = await invoke<AtlasResult>("create_texture_atlas", { paths: textures.map((item) => item.path), settings: { outputDir: outputDir || null, padToSquare, squareResolution: padToSquare ? numberOrNull(squareResolution === "custom" ? customSquareResolution : squareResolution) : null } });
      setResult(created); setNotice(t("テクスチャアトラスを作成しました", "Texture atlas created"));
    } catch (error) { setNotice(t(`作成できませんでした: ${String(error)}`, `Could not create the atlas: ${localizedDetail(error, "The operation failed")}`)); }
    finally { setIsCreating(false); }
  };

  return <section className="workspace">
    <aside className="settings atlas-settings">
      <h2>{t("アトラス設定", "Atlas Settings")}</h2>
      <label>{t("出力先", "Output folder")}<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder={t("1枚目の入力元 / atlas", "First source folder / atlas")} /><button onClick={chooseOutput}>{t("選択", "Choose")}</button></div></label>
      <fieldset><legend>{t("配置", "Layout")}</legend><label className="checkbox"><input type="checkbox" checked={padToSquare} onChange={(event) => { setPadToSquare(event.target.checked); setResult(undefined); }} />{t("各画像を正方形として補完", "Pad each image to a square")}</label>{padToSquare && <label>{t("1枠の解像度", "Resolution per slot")}<select value={squareResolution} onChange={(event) => { setSquareResolution(event.target.value); setResult(undefined); }}><option value="">{t("自動", "Automatic")}</option><option value="256">256 × 256</option><option value="512">512 × 512</option><option value="1024">1024 × 1024</option><option value="2048">2048 × 2048</option><option value="4096">4096 × 4096</option><option value="custom">Custom…</option></select>{squareResolution === "custom" && <input inputMode="numeric" value={customSquareResolution} onChange={(event) => { setCustomSquareResolution(event.target.value); setResult(undefined); }} placeholder="16–4096 px" />}</label>}<p className="mode-note">{padToSquare ? squareResolution ? t("最終アトラスは指定値の2倍です。大きい画像だけ縮小し、小さい画像は拡大せず透明余白で補完します。", "The final atlas is twice the selected resolution. Large images are reduced; small images are padded with transparency without upscaling.") : t("縦長・横長画像は、引き伸ばさず中央へ配置し、足りない部分を透明で埋めます。", "Portrait and landscape images are centered without stretching, with transparent padding.") : t("画像比率に合わせた長方形セルで配置します。", "Uses rectangular cells that follow the image aspect ratios.")}</p><p className="mode-note">{t("外部画像は好きな枠へ直接ドロップできます。登録後の画像をドラッグすると、枠同士を入れ替えられます。", "Drop external images directly into any slot. Drag added images to swap slots.")}</p></fieldset>
      <p className="hint">{t("4枚を2×2アトラスへ結合します。元の画像は変更しません。", "Combines four images into a 2×2 atlas. Source images are never modified.")}</p>
    </aside>
    <section className="queue">
      <div className="queue-toolbar"><div><h2>{t("テクスチャアトラス", "Texture Atlas")} <span>{textureCount} / 4</span></h2><p>{result ? `${result.width} × ${result.height} · ${formatBytes(result.outputBytes)}` : notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isCreating || isLoading || textureCount === 4}>{t("画像を追加", "Add Images")}</button><button className="primary" onClick={create} disabled={textureCount !== 4 || isCreating || isLoading}>{isLoading ? t("読込中…", "Loading…") : isCreating ? t("作成中…", "Creating…") : t("アトラスを作成", "Create Atlas")}</button></div></div>
      <div className="atlas-grid">{slots.map((slot, index) => <AtlasTile key={slot} slot={slot} texture={textures[index]} index={index} disabled={isCreating || isLoading} highlighted={dropTarget === index} onMove={(direction) => swap(index, index + direction)} onRemove={() => { setTextures((items) => items.map((item, itemIndex) => itemIndex === index ? undefined : item)); setResult(undefined); }} onAdd={chooseFiles} onInternalDragStart={() => setDropTarget(index)} onInternalDragOver={() => setDropTarget(index)} onInternalDragEnd={() => setDropTarget(undefined)} onInternalDrop={(source) => { swap(source, index); setDropTarget(undefined); }} />)}</div>
      <div className="queue-footer"><div className="footer-actions"><button onClick={() => { setTextures(emptyTextures()); setResult(undefined); setNotice(t(`${supportedImageShortLabel}を任意の枠へドロップしてください`, `Drop ${supportedImageShortLabel} into any slot`)); }} disabled={isCreating || textureCount === 0}>{t("リストをクリア", "Clear List")}</button>{result && <button onClick={() => revealItemInDir(result.outputPath)}>{t("出力を開く", "Open Output")}</button>}</div><span>{notice}</span></div>
    </section>
  </section>;
}

function numberOrNull(value: string) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : null;
}
