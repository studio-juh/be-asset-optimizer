import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import Thumbnail from "./Thumbnail";
import NormalAdjustPreview from "./NormalAdjustPreview";
import { supportedImageFilter } from "./imageFiles";
import { localizedDetail, t } from "./i18n";

type Channel = { path: string | null; channel: string; invert: boolean; value: number };
type Result = { outputPath: string; outputBytes: number; width: number; height: number };
const initialChannels = (): Channel[] => [0,0,0,255].map(value => ({ path:null, channel:"luma", invert:false, value }));
const name = (path: string) => path.split(/[\\/]/).pop() ?? path;

export default function TextureTools({ mode }: { mode: "channels" | "adjust" }) {
  const [channels,setChannels] = useState(initialChannels);
  const [layout,setLayout] = useState("custom");
  const [base,setBase] = useState("");
  const [detail,setDetail] = useState("");
  const [strength,setStrength] = useState(1);
  const [detailStrength,setDetailStrength] = useState(1);
  const [flip,setFlip] = useState(false);
  const [resolution,setResolution] = useState("");
  const [output,setOutput] = useState("");
  const [busy,setBusy] = useState(false);
  const [results,setResults] = useState<Result[]>([]);
  const [notice,setNotice] = useState("");
  const target = useRef(0);
  const busyRef = useRef(false);
  const receiveRef = useRef<(paths:string[])=>void>(()=>{});
  const update = (i:number, patch: Partial<Channel>) => { setChannels(current=>current.map((c,index)=>index===i?{...c,...patch}:c));setResults([]); };
  receiveRef.current = paths => {
    if (busyRef.current) return;
    const index = target.current;
    if (mode === "channels") { setChannels(current=> { const next=current.map(c=>({...c})); paths.slice(0,4-index).forEach((path,i)=>next[index+i].path=path); return next; }); }
    else { if (target.current === 1) setDetail(paths[0] ?? ""); else {setBase(paths[0]??"");if(paths[1])setDetail(paths[1]);} }
    setResults([]);
  };
  const choose = async (index:number) => { const files=await open({multiple:mode==="channels",filters:[supportedImageFilter]}); if(files){target.current=index;receiveRef.current(Array.isArray(files)?files:[files]);} };
  useEffect(()=>{
    const locate=(event:Event)=>{const {x,y}=(event as CustomEvent<{x:number;y:number}>).detail;const index=document.elementFromPoint(x/devicePixelRatio,y/devicePixelRatio)?.closest<HTMLElement>("[data-texture-input]")?.dataset.textureInput;target.current=index===undefined?0:Number(index);};
    const drop=(event:Event)=>receiveRef.current((event as CustomEvent<string[]>).detail);
    const menu=()=>{void choose(0);};
    window.addEventListener(`smartpng-${mode}-drop`,drop);window.addEventListener(`smartpng-${mode}-menu-open`,menu);
    window.addEventListener(`smartpng-${mode}-target`,locate);
    return()=>{window.removeEventListener(`smartpng-${mode}-drop`,drop);window.removeEventListener(`smartpng-${mode}-menu-open`,menu);window.removeEventListener(`smartpng-${mode}-target`,locate);};
  },[mode]);
  const run = async (split=false) => {
    if(busyRef.current)return;busyRef.current=true;setBusy(true);setNotice("");setResults([]);
    try {
      if(mode==="channels"&&split) { const path=channels.find(c=>c.path)?.path;if(!path)throw Error(t("画像を指定してください","Choose an image"));setResults(await invoke<Result[]>("split_channels",{path,outputDir:output||null})); }
      else if(mode==="channels") setResults([await invoke<Result>("pack_channels",{settings:{channels,resolution:resolution?Number(resolution):null,outputDir:output||null}})]);
      else setResults([await invoke<Result>("adjust_normal",{settings:{base,detail:detail||null,strength,detailStrength,flipGreen:flip,outputDir:output||null}})]);
      setNotice(t("保存しました","Saved"));
    }catch(error){setNotice(localizedDetail(error,"Processing failed"));}finally{busyRef.current=false;setBusy(false);}
  };
  const preset=(kind:string)=>{setLayout(kind);const next=initialChannels();if(kind==="orm"){next[0].value=255;next[1].value=255;}if(kind==="unity"){next[3].invert=true;next[3].value=255;}setChannels(current=>next.map((c,i)=>({...c,path:current[i].path})));setResults([]);};
  const clear=()=>{setChannels(initialChannels());setLayout("custom");setBase("");setDetail("");setResults([]);setNotice("");target.current=0;};
  const channelLabels=layout==="orm"?["R · AO","G · Roughness","B · Metallic","A"]:layout==="unity"?["R · Metallic","G","B","A · Roughness → Smoothness"]:["R","G","B","A"];
  return <section className="workspace">
    <aside className="settings"><h2>{mode==="channels"?t("チャンネル設定","Channel Settings"):t("ノーマル調整","Normal Adjustment")}</h2>
      <label>{t("出力先","Output folder")}<div className="path-row"><input disabled={busy} value={output} onChange={e=>setOutput(e.target.value)} placeholder="optimized"/><button disabled={busy} onClick={async()=>{const dir=await open({directory:true,multiple:false});if(typeof dir==="string")setOutput(dir);}}>{t("選択","Choose")}</button></div></label>
      {mode==="channels"?<fieldset disabled={busy}><legend>{t("配置プリセット","Layout Preset")}</legend><div className="segmented"><button onClick={()=>preset("custom")}>RGBA</button><button onClick={()=>preset("orm")}>ORM</button><button onClick={()=>preset("unity")}>Unity M/S</button></div><label>{t("出力解像度","Output Resolution")}<select value={resolution} onChange={e=>setResolution(e.target.value)}><option value="">{t("最初の画像に合わせる","Match first image")}</option>{[256,512,1024,2048,4096].map(v=><option key={v} value={v}>{v} × {v}</option>)}</select></label></fieldset>:<fieldset disabled={busy}><legend>{t("補正・合成","Adjust / Blend")}</legend><label>{t("ベース強度","Base Strength")}<input type="range" min="0" max="4" step="0.05" value={strength} onChange={e=>setStrength(Number(e.target.value))}/><output>{strength.toFixed(2)}</output></label><label>{t("詳細強度","Detail Strength")}<input type="range" min="0" max="4" step="0.05" value={detailStrength} onChange={e=>setDetailStrength(Number(e.target.value))}/><output>{detailStrength.toFixed(2)}</output></label><label className="checkbox"><input type="checkbox" checked={flip} onChange={e=>setFlip(e.target.checked)}/>{t("G反転（OpenGL ↔ DirectX）","Flip G (OpenGL ↔ DirectX)")}</label></fieldset>}
    </aside>
    <section className="queue"><div className="queue-toolbar"><div><h2>{mode==="channels"?t("チャンネル合成・分離","Channel Packing / Split"):t("既存ノーマルの調整","Normal Map Adjustment")}</h2><p role="status">{notice}</p></div><div className="actions"><button disabled={busy} onClick={clear}>{t("クリア","Clear")}</button>{mode==="channels"&&<button disabled={busy||!channels.some(c=>c.path)} onClick={()=>void run(true)}>{t("RGBAを分離","Split RGBA")}</button>}<button className="primary" disabled={busy||(mode==="channels"?!channels.some(c=>c.path):!base)} onClick={()=>void run()}>{busy?t("処理中…","Processing…"):t("保存","Save")}</button></div></div>
      <div className={`texture-tool-inputs ${mode === "adjust" ? "normal-adjust-inputs" : ""}`}>{(mode==="channels"?channelLabels:[t("ベース","Base"),t("詳細（任意）","Detail (optional)")]).map((label,i)=>{const path=mode==="channels"?channels[i].path:(i===0?base:detail);return <div className="texture-tool-card" data-texture-input={i} key={i} onMouseEnter={()=>target.current=i}><strong>{label}</strong><div className="texture-input-row"><button className="texture-input" disabled={busy} onClick={()=>void choose(i)}>{path?<><Thumbnail key={path} path={path} alt={name(path)} fallbackLabel={label}/><span title={path}>{name(path)}</span></>:t("ドロップまたは選択","Drop or choose")}</button><button className="texture-remove" disabled={busy||!path} aria-label={t("削除","Remove")+" "+label} onClick={()=>{if(mode==="channels")update(i,{path:null});else {if(i===0)setBase("");else setDetail("");setResults([]);}}}>×</button></div>{mode==="channels"&&<><label>{t("入力チャンネル","Source Channel")}<select disabled={busy} value={channels[i].channel} onChange={e=>update(i,{channel:e.target.value})}>{["luma","r","g","b","a"].map(c=><option key={c} value={c}>{c==="luma"?t("明度","Luminance"):c.toUpperCase()}</option>)}</select></label><label className="checkbox"><input disabled={busy} type="checkbox" checked={channels[i].invert} onChange={e=>update(i,{invert:e.target.checked})}/>{t("反転","Invert")}</label>{!path&&<label>{t("固定値","Constant")}<input disabled={busy} type="number" min="0" max="255" value={channels[i].value} onChange={e=>update(i,{value:Math.max(0,Math.min(255,Number(e.target.value)))})}/></label>}</>}</div>;})}</div>
      {mode === "adjust" && <NormalAdjustPreview base={base} detail={detail} strength={strength} detailStrength={detailStrength} flip={flip} />}
      <div className="tool-results">{results.map(result=><div key={result.outputPath}><span>{name(result.outputPath)} · {result.width} × {result.height}</span><button onClick={()=>revealItemInDir(result.outputPath)}>{t("開く","Open")}</button></div>)}</div>
    </section>
  </section>;
}
