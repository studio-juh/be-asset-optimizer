import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { HDRLoader } from "three/examples/jsm/loaders/HDRLoader.js";
import { supportedImageFilter } from "./imageFiles";
import { localizedDetail, t } from "./i18n";

type MapName = "albedo" | "normal" | "roughness" | "metallic" | "ao";
const mapNames: MapName[] = ["albedo","normal","roughness","metallic","ao"];
const emptyMaps = () => ({albedo:"",normal:"",roughness:"",metallic:"",ao:""});
export default function MaterialPreview({active}:{active:boolean}) {
  const [maps,setMaps]=useState(emptyMaps);
  const [shape,setShape]=useState("sphere");
  const [lighting,setLighting]=useState("hdri");
  const [environmentIntensity,setEnvironmentIntensity]=useState(1);
  const [environmentRotation,setEnvironmentRotation]=useState(0);
  const [environmentNotice,setEnvironmentNotice]=useState("");
  const [normalStrength,setNormalStrength]=useState(1);
  const [roughness,setRoughness]=useState(0.5);
  const [metallic,setMetallic]=useState(0);
  const [flipGreen,setFlipGreen]=useState(false);
  const [notice,setNotice]=useState("");
  const [textures,setTextures]=useState<Partial<Record<MapName,THREE.Texture>>>({});
  const viewport=useRef<HTMLDivElement>(null);
  const materialRef=useRef<THREE.MeshStandardMaterial | undefined>(undefined);
  const sceneRef=useRef<THREE.Scene | undefined>(undefined);
  const environmentSettings=useRef({intensity:environmentIntensity,rotation:environmentRotation});
  environmentSettings.current={intensity:environmentIntensity,rotation:environmentRotation};
  const activeRef=useRef(active);activeRef.current=active;
  const target=useRef<MapName>("albedo");
  const choose=async(key:MapName)=>{const path=await open({multiple:false,filters:[supportedImageFilter]});if(typeof path==="string")setMaps(current=>({...current,[key]:path}));};
  useEffect(()=>{const locate=(event:Event)=>{const {x,y}=(event as CustomEvent<{x:number;y:number}>).detail;const key=document.elementFromPoint(x/devicePixelRatio,y/devicePixelRatio)?.closest<HTMLElement>("[data-material-map]")?.dataset.materialMap;target.current=mapNames.includes(key as MapName)?key as MapName:"albedo";};const drop=(event:Event)=>{const paths=(event as CustomEvent<string[]>).detail;const key=target.current;setMaps(current=>({...current,[key]:paths[0]??""}));};const menu=()=>{void choose("albedo");};window.addEventListener("smartpng-material-target",locate);window.addEventListener("smartpng-material-drop",drop);window.addEventListener("smartpng-material-menu-open",menu);return()=>{window.removeEventListener("smartpng-material-target",locate);window.removeEventListener("smartpng-material-drop",drop);window.removeEventListener("smartpng-material-menu-open",menu);};},[]);
  useEffect(()=>{
    let disposed=false;const created:THREE.Texture[]=[];setNotice("");setTextures({});
    void (async()=>{
      const loaded:Partial<Record<MapName,THREE.Texture>>={};
      for(const key of mapNames)if(maps[key])try{const url=await invoke<string>("create_preview",{path:maps[key],size:512});if(disposed)return;const texture=await new THREE.TextureLoader().loadAsync(url);created.push(texture);if(disposed){texture.dispose();return;}texture.colorSpace=key==="albedo"?THREE.SRGBColorSpace:THREE.NoColorSpace;texture.wrapS=texture.wrapT=THREE.RepeatWrapping;loaded[key]=texture;}catch(error){if(!disposed)setNotice(localizedDetail(error,"Could not load material texture"));}
      if(!disposed)setTextures(loaded);
    })();return()=>{disposed=true;created.forEach(texture=>texture.dispose());};
  },[maps]);
  useEffect(()=>{
    const host=viewport.current;if(!host)return;
    const renderer=new THREE.WebGLRenderer({antialias:true,alpha:true});renderer.setPixelRatio(Math.min(devicePixelRatio,2));renderer.outputColorSpace=THREE.SRGBColorSpace;renderer.toneMapping=THREE.ACESFilmicToneMapping;host.appendChild(renderer.domElement);
    const scene=new THREE.Scene();sceneRef.current=scene;scene.environmentIntensity=environmentSettings.current.intensity;scene.environmentRotation.y=THREE.MathUtils.degToRad(environmentSettings.current.rotation);const camera=new THREE.PerspectiveCamera(42,1,0.01,100);camera.position.set(3,2,3);const controls=new OrbitControls(camera,renderer.domElement);controls.enableDamping=true;
    let disposed=false;
    const environment=new RoomEnvironment();const pmrem=new THREE.PMREMGenerator(renderer);let env=pmrem.fromScene(environment);scene.environment=env.texture;environment.dispose();
    setEnvironmentNotice("");
    if(lighting==="hdri") {
      setEnvironmentNotice(t("HDRIを読み込み中…","Loading HDRI…"));
      void new HDRLoader().loadAsync(`${import.meta.env.BASE_URL}environments/monochrome_studio_02_1k.hdr`).then(hdr=>{
        if(disposed){hdr.dispose();return;}
        try {
          const loaded=pmrem.fromEquirectangular(hdr);
          const previous=env;env=loaded;scene.environment=loaded.texture;previous.dispose();
          setEnvironmentNotice("");
        } finally {hdr.dispose();}
      }).catch(error=>{if(!disposed)setEnvironmentNotice(localizedDetail(error,"Could not load HDRI; using studio fallback"));});
    }
    const geometry=shape==="cube"?new THREE.BoxGeometry(2,2,2):shape==="plane"?new THREE.PlaneGeometry(2.6,2.6):new THREE.SphereGeometry(1.2,64,48);geometry.setAttribute("uv1",geometry.getAttribute("uv").clone());
    const material=new THREE.MeshStandardMaterial({side:shape==="plane"?THREE.DoubleSide:THREE.FrontSide});materialRef.current=material;const mesh=new THREE.Mesh(geometry,material);scene.add(mesh);
    if(lighting!=="hdri") {const light=new THREE.DirectionalLight(0xffffff,lighting==="strong"?5:lighting==="soft"?1:3);light.position.set(lighting==="strong"?-3:4,3,4);scene.add(light);scene.add(new THREE.HemisphereLight(0xffffff,0x536078,0.5));}
    const resize=()=>{const width=Math.max(host.clientWidth,1),height=Math.max(host.clientHeight,1);camera.aspect=width/height;camera.updateProjectionMatrix();renderer.setSize(width,height,false);};const observer=new ResizeObserver(resize);observer.observe(host);resize();
    let frame=0;const render=()=>{frame=requestAnimationFrame(render);if(activeRef.current){controls.update();renderer.render(scene,camera);}};render();
    return()=>{disposed=true;cancelAnimationFrame(frame);observer.disconnect();controls.dispose();geometry.dispose();material.dispose();materialRef.current=undefined;sceneRef.current=undefined;env.dispose();pmrem.dispose();renderer.dispose();renderer.domElement.remove();};
  },[shape,lighting]);
  useEffect(()=>{const scene=sceneRef.current;if(scene){scene.environmentIntensity=environmentIntensity;scene.environmentRotation.y=THREE.MathUtils.degToRad(environmentRotation);}},[environmentIntensity,environmentRotation]);
  useEffect(()=>{const material=materialRef.current;if(!material)return;material.map=textures.albedo??null;material.normalMap=textures.normal??null;material.roughnessMap=textures.roughness??null;material.metalnessMap=textures.metallic??null;material.aoMap=textures.ao??null;material.normalScale.set(normalStrength,flipGreen?-normalStrength:normalStrength);material.roughness=textures.roughness?1:roughness;material.metalness=textures.metallic?1:metallic;material.needsUpdate=true;},[textures,normalStrength,roughness,metallic,flipGreen,shape,lighting]);
  const label=(key:MapName)=>({albedo:t("アルベド","Albedo"),normal:t("ノーマル","Normal"),roughness:t("ラフネス（G）","Roughness (G)"),metallic:t("メタリック（B）","Metallic (B)"),ao:"AO (R)"})[key];
  return <section className="workspace"><aside className="settings material-settings"><h2>{t("PBRプレビュー","PBR Preview")}</h2><div className="actions"><button onClick={()=>{setMaps(emptyMaps());setNotice("");}}>{t("クリア","Clear")}</button></div><fieldset><legend>{t("テクスチャ","Textures")}</legend>{mapNames.map(key=><label key={key} data-material-map={key} onMouseEnter={()=>target.current=key}>{label(key)}<div className="path-row"><input readOnly value={maps[key].split(/[\\/]/).pop()??""} title={maps[key]}/><button onClick={()=>void choose(key)}>{t("選択","Choose")}</button>{maps[key]&&<button className="map-clear" aria-label={t("削除", "Remove")+" "+label(key)} onClick={()=>setMaps(current=>({...current,[key]:""}))}>×</button>}</div></label>)}</fieldset><fieldset><legend>{t("表示","View")}</legend><label>{t("形状","Shape")}<select value={shape} onChange={e=>setShape(e.target.value)}><option value="sphere">{t("球","Sphere")}</option><option value="cube">{t("立方体","Cube")}</option><option value="plane">{t("平面","Plane")}</option></select></label><label>{t("照明","Lighting")}<select value={lighting} onChange={e=>setLighting(e.target.value)}><option value="hdri">Studio HDRI</option><option value="studio">Studio</option><option value="soft">Soft</option><option value="strong">Side</option></select></label><label>{t("環境光の明るさ","Environment Intensity")}<div className="range-row"><input type="range" min="0" max="3" step="0.05" value={environmentIntensity} onChange={e=>setEnvironmentIntensity(Number(e.target.value))}/><output>{environmentIntensity.toFixed(2)}</output></div></label><label>{t("環境光の回転","Environment Rotation")}<div className="range-row"><input type="range" min="0" max="360" step="5" value={environmentRotation} onChange={e=>setEnvironmentRotation(Number(e.target.value))}/><output>{environmentRotation}°</output></div></label><label>{t("ノーマル強度","Normal Strength")}<input type="range" min="0" max="4" step="0.05" value={normalStrength} onChange={e=>setNormalStrength(Number(e.target.value))}/></label><label className="checkbox"><input type="checkbox" checked={flipGreen} onChange={e=>setFlipGreen(e.target.checked)}/>{t("DirectX（G反転）","DirectX (flip G)")}</label><label>{t("ラフネス","Roughness")}<input disabled={!!maps.roughness} type="range" min="0" max="1" step="0.01" value={roughness} onChange={e=>setRoughness(Number(e.target.value))}/></label><label>{t("メタリック","Metallic")}<input disabled={!!maps.metallic} type="range" min="0" max="1" step="0.01" value={metallic} onChange={e=>setMetallic(Number(e.target.value))}/></label></fieldset></aside><section className="queue"><div className="queue-toolbar"><h2>{t("マテリアル確認","Material Preview")}</h2><p role="status">{notice || environmentNotice}</p></div><div className="model-stage material-stage" onMouseEnter={()=>target.current="albedo"}><div ref={viewport} className="model-viewport"/></div></section></section>;
}
