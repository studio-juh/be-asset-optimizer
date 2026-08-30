import { DragEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { InspectProgress, InspectResult, supportedImageFilter, supportedImageShortLabel } from "./imageFiles";

type Texture = { path: string; name: string; originalBytes: number; width: number; height: number };
type AtlasResult = { outputPath: string; outputBytes: number; width: number; height: number };
type NativeAtlasDrag = { type: "enter" | "drop"; paths: string[]; position: { x: number; y: number } } | { type: "over"; position: { x: number; y: number } } | { type: "leave" };

const slots = ["左上", "右上", "左下", "右下"];
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
    <div className="atlas-tile-bar"><strong>{slot}</strong>{texture && <div className="atlas-actions"><button title="前の枠と入れ替え" onClick={() => onMove(-1)} disabled={disabled || index === 0}>←</button><button title="次の枠と入れ替え" onClick={() => onMove(1)} disabled={disabled || index === 3}>→</button><button title="この画像を外す" onClick={onRemove} disabled={disabled}>×</button></div>}</div>
    {texture ? <><div className="atlas-tile-preview" draggable={!disabled} onDragStart={startInternalDrag} onDragEnd={onInternalDragEnd}>{preview ? <img src={preview} alt={`${slot}: ${texture.name}`} draggable={false} /> : <span>読込中…</span>}</div><div className="atlas-tile-caption" title={texture.path}><strong>{texture.name}</strong><small>{texture.width} × {texture.height}</small></div></> : <button className="atlas-empty" onClick={onAdd} disabled={disabled}><strong>ここへ画像をドロップ</strong><small>主要な静止画形式</small></button>}
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
  const [notice, setNotice] = useState(`${supportedImageShortLabel}を任意の枠へドロップしてください。`);
  const textureCount = useMemo(() => textures.filter(Boolean).length, [textures]);

  const addPaths = async (paths: string[], targetIndex?: number) => {
    if (!paths.length) return;
    if (loadingRequestId.current) { setNotice("現在の画像を読み込み中です"); return; }
    const requestId = crypto.randomUUID();
    loadingRequestId.current = requestId;
    setIsLoading(true);
    setNotice("画像を確認しています…");
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
        if (added) setNotice(`${count} / 4 枚を登録しました${result.failures.length ? `（${result.failures.length} 枚は読込失敗）` : ""}`);
        else if (result.failures.length) setNotice(`読み込めませんでした: ${result.failures[0].message}`);
        else setNotice("追加できる空き枠がないか、対象画像がありません");
        return next;
      });
      setResult(undefined);
    } catch (error) { setNotice(`追加できませんでした: ${String(error)}`); }
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
      if (update.requestId === loadingRequestId.current) setNotice(`${update.completed} / ${update.total} 枚を読み込んでいます…`);
    });
    const onNativeDrag = (event: Event) => {
      const detail = (event as CustomEvent<NativeAtlasDrag>).detail;
      if (detail.type === "leave") { setDropTarget(undefined); return; }
      const target = atlasIndexAt(detail.position);
      setDropTarget(target);
      if (detail.type === "drop") { setDropTarget(undefined); void addPaths(detail.paths, target); }
    };
    window.addEventListener("smartpng-atlas-drag", onNativeDrag);
    return () => { void offInspect.then((off) => off()); window.removeEventListener("smartpng-atlas-drag", onNativeDrag); };
  }, []);

  const chooseFiles = async () => {
    const selected = await open({ multiple: true, filters: [supportedImageFilter] });
    if (selected) await addPaths(Array.isArray(selected) ? selected : [selected]);
  };
  const chooseOutput = async () => {
    const selected = await open({ directory: true, multiple: false, title: "出力先フォルダを選択" });
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
    if (!complete || isCreating) { setNotice("4つの枠すべてに画像を指定してください"); return; }
    setIsCreating(true); setResult(undefined); setNotice("アトラスを作成しています…");
    try {
      const created = await invoke<AtlasResult>("create_texture_atlas", { paths: textures.map((item) => item.path), settings: { outputDir: outputDir || null, padToSquare, squareResolution: padToSquare ? numberOrNull(squareResolution === "custom" ? customSquareResolution : squareResolution) : null } });
      setResult(created); setNotice("テクスチャアトラスを作成しました");
    } catch (error) { setNotice(`作成できませんでした: ${String(error)}`); }
    finally { setIsCreating(false); }
  };

  return <section className="workspace">
    <aside className="settings atlas-settings">
      <h2>アトラス設定</h2>
      <label>出力先<div className="path-row"><input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder="1枚目の入力元 / atlas" /><button onClick={chooseOutput}>選択</button></div></label>
      <fieldset><legend>配置</legend><label className="checkbox"><input type="checkbox" checked={padToSquare} onChange={(event) => { setPadToSquare(event.target.checked); setResult(undefined); }} />各画像を正方形として補完</label>{padToSquare && <label>1枠の解像度<select value={squareResolution} onChange={(event) => { setSquareResolution(event.target.value); setResult(undefined); }}><option value="">自動</option><option value="256">256 × 256</option><option value="512">512 × 512</option><option value="1024">1024 × 1024</option><option value="2048">2048 × 2048</option><option value="4096">4096 × 4096</option><option value="custom">Custom…</option></select>{squareResolution === "custom" && <input inputMode="numeric" value={customSquareResolution} onChange={(event) => { setCustomSquareResolution(event.target.value); setResult(undefined); }} placeholder="16〜4096 px" />}</label>}<p className="mode-note">{padToSquare ? squareResolution ? "最終アトラスは指定値の2倍です。大きい画像だけ縮小し、小さい画像は拡大せず透明余白で補完します。" : "縦長・横長画像は、引き伸ばさず中央へ配置し、足りない部分を透明で埋めます。" : "画像比率に合わせた長方形セルで配置します。"}</p><p className="mode-note">外部画像は好きな枠へ直接ドロップできます。登録後の画像をドラッグすると、枠同士を入れ替えられます。</p></fieldset>
      <p className="hint">4枚を2×2アトラスへ結合します。元の画像は変更しません。</p>
    </aside>
    <section className="queue">
      <div className="queue-toolbar"><div><h2>テクスチャアトラス <span>{textureCount} / 4</span></h2><p>{result ? `${result.width} × ${result.height} · ${formatBytes(result.outputBytes)}` : notice}</p></div><div className="actions"><button className="secondary" onClick={chooseFiles} disabled={isCreating || isLoading || textureCount === 4}>画像を追加</button><button className="primary" onClick={create} disabled={textureCount !== 4 || isCreating || isLoading}>{isLoading ? "読込中…" : isCreating ? "作成中…" : "アトラスを作成"}</button></div></div>
      <div className="atlas-grid">{slots.map((slot, index) => <AtlasTile key={slot} slot={slot} texture={textures[index]} index={index} disabled={isCreating || isLoading} highlighted={dropTarget === index} onMove={(direction) => swap(index, index + direction)} onRemove={() => { setTextures((items) => items.map((item, itemIndex) => itemIndex === index ? undefined : item)); setResult(undefined); }} onAdd={chooseFiles} onInternalDragStart={() => setDropTarget(index)} onInternalDragOver={() => setDropTarget(index)} onInternalDragEnd={() => setDropTarget(undefined)} onInternalDrop={(source) => { swap(source, index); setDropTarget(undefined); }} />)}</div>
      <div className="queue-footer"><div className="footer-actions"><button onClick={() => { setTextures(emptyTextures()); setResult(undefined); setNotice(`${supportedImageShortLabel}を任意の枠へドロップしてください`); }} disabled={isCreating || textureCount === 0}>リストをクリア</button>{result && <button onClick={() => revealItemInDir(result.outputPath)}>出力を開く</button>}</div><span>{notice}</span></div>
    </section>
  </section>;
}

function numberOrNull(value: string) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : null;
}
