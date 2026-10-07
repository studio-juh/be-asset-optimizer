import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { DDSLoader } from "three/examples/jsm/loaders/DDSLoader.js";
import { FBXLoader } from "three/examples/jsm/loaders/FBXLoader.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { TGALoader } from "three/examples/jsm/loaders/TGALoader.js";
import { localizedDetail, t } from "./i18n";

type TextureFile = { path: string; relativePath: string; name: string; bytes: number };
type ModelPackage = { path: string; name: string; bytesBase64: string; fileBytes: number; textures: TextureFile[]; scanLimited: boolean };
type TextureResolution = { request: string; resolvedPath?: string; status: "found" | "missing" | "error" };
type ModelStats = { meshes: number; triangles: number; materials: number; animations: number };

const emptyStats: ModelStats = { meshes: 0, triangles: 0, materials: 0, animations: 0 };

export default function ModelPreview({ active = true }: { active?: boolean }) {
  const activeRef = useRef(active);
  activeRef.current = active;
  const viewportRef = useRef<HTMLDivElement>(null);
  const modelRef = useRef<THREE.Group | null>(null);
  const gridRef = useRef<THREE.GridHelper | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const initialViewDirectionRef = useRef<THREE.Vector3 | null>(null);
  const initialModelQuaternionRef = useRef<THREE.Quaternion | null>(null);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const actionRef = useRef<THREE.AnimationAction | null>(null);
  const autoRotateRef = useRef(false);
  const [model, setModel] = useState<ModelPackage>();
  const [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(false);
  const [wireframe, setWireframe] = useState(false);
  const [showGrid, setShowGrid] = useState(true);
  const [autoRotate, setAutoRotate] = useState(false);
  const [playing, setPlaying] = useState(true);
  const [resolutions, setResolutions] = useState<TextureResolution[]>([]);
  const [stats, setStats] = useState<ModelStats>(emptyStats);

  const loadModel = async (path: string) => {
    if (!isModelPath(path)) { setNotice(t("FBXまたはGLBファイルを選択してください", "Choose an FBX or GLB file")); return; }
    setLoading(true);
    setNotice(t("3Dモデルとテクスチャを確認しています…", "Inspecting the 3D model and textures…"));
    try {
      const loaded = await invoke<ModelPackage>("load_model_package", { path });
      setModel(loaded);
      setResolutions([]);
      setNotice(t("モデルを読み込みました", "Model loaded"));
    } catch (error) {
      setModel(undefined);
      setStats(emptyStats);
      setResolutions([]);
      setNotice(localizedDetail(error, "Could not load the 3D model"));
    } finally { setLoading(false); }
  };

  const chooseModel = async () => {
    const selected = await open({ multiple: false, filters: [{ name: t("3Dモデル", "3D Models"), extensions: ["fbx", "glb"] }] });
    if (typeof selected === "string") await loadModel(selected);
  };

  useEffect(() => {
    const onDrop = (event: Event) => {
      const paths = (event as CustomEvent<string[]>).detail;
      const modelPath = paths.find(isModelPath);
      if (modelPath) void loadModel(modelPath); else setNotice(t("FBXまたはGLBファイルをドロップしてください", "Drop an FBX or GLB file"));
    };
    const onMenuOpen = () => { void chooseModel(); };
    window.addEventListener("smartpng-model-drop", onDrop);
    window.addEventListener("smartpng-preview-menu-open", onMenuOpen);
    return () => { window.removeEventListener("smartpng-model-drop", onDrop); window.removeEventListener("smartpng-preview-menu-open", onMenuOpen); };
  }, []);

  useEffect(() => { autoRotateRef.current = autoRotate; }, [autoRotate]);
  useEffect(() => { if (gridRef.current) gridRef.current.visible = showGrid; }, [showGrid]);
  useEffect(() => {
    modelRef.current?.traverse((object) => {
      if (!(object instanceof THREE.Mesh)) return;
      for (const material of asMaterials(object.material)) setMaterialWireframe(material, wireframe);
    });
  }, [wireframe]);
  useEffect(() => {
    if (actionRef.current) actionRef.current.paused = !playing;
  }, [playing]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(42, 1, 0.01, 100000);
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    viewport.appendChild(renderer.domElement);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.screenSpacePanning = true;
    cameraRef.current = camera;
    controlsRef.current = controls;
    camera.position.set(5.4, 3.6, 5.4);
    controls.update();
    initialViewDirectionRef.current = camera.position.clone().normalize();
    let grid = new THREE.GridHelper(16, 20, 0x718096, 0xa7b0bd);
    grid.visible = showGrid;
    gridRef.current = grid;
    scene.add(grid);

    scene.add(new THREE.HemisphereLight(0xffffff, 0x536078, 2.2));
    const keyLight = new THREE.DirectionalLight(0xffffff, 3.2);
    keyLight.position.set(4, 7, 5);
    scene.add(keyLight);
    const fillLight = new THREE.DirectionalLight(0xb9d8ff, 1.3);
    fillLight.position.set(-5, 2, -4);
    scene.add(fillLight);

    const manager = new THREE.LoadingManager();
    manager.addHandler(/\.tga$/i, new TGALoader(manager));
    manager.addHandler(/\.dds$/i, new DDSLoader(manager));
    const resultMap = new Map<string, TextureResolution>();
    const assetRequests = new Map<string, string>();
    const flushResolutions = () => setResolutions(Array.from(resultMap.values()).sort((a, b) => a.request.localeCompare(b.request)));
    manager.setURLModifier((url) => {
      if (/^(data:|blob:)/i.test(url)) return url;
      const match = findTexture(url, model?.textures ?? []);
      if (!match) {
        resultMap.set(url, { request: readableRequest(url), status: "missing" });
        return url;
      }
      const assetUrl = convertFileSrc(match.path);
      resultMap.set(url, { request: readableRequest(url), resolvedPath: match.path, status: "found" });
      assetRequests.set(assetUrl, url);
      return assetUrl;
    });
    manager.onLoad = flushResolutions;
    manager.onError = (url) => {
      const request = assetRequests.get(url) ?? url;
      const current = resultMap.get(request);
      resultMap.set(request, { request: current?.request ?? readableRequest(request), resolvedPath: current?.resolvedPath, status: "error" });
      flushResolutions();
    };

    let disposed = false;
    let root: THREE.Group | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let frame = 0;

    const showModel = (loadedRoot: THREE.Group, animations: THREE.AnimationClip[]) => {
      if (disposed) { disposeObject(loadedRoot); return; }
      root = loadedRoot;
      root.animations = animations;
      modelRef.current = root;
      initialModelQuaternionRef.current = root.quaternion.clone();
      root.traverse((object) => {
        if (!(object instanceof THREE.Mesh)) return;
        for (const material of asMaterials(object.material)) setMaterialWireframe(material, wireframe);
      });

      const bounds = new THREE.Box3().setFromObject(root);
      const size = bounds.getSize(new THREE.Vector3());
      const center = bounds.getCenter(new THREE.Vector3());
      if (!bounds.isEmpty()) root.position.sub(center);
      scene.add(root);

      const diameter = Math.max(size.x, size.y, size.z, 1);
      camera.near = Math.max(diameter / 10000, 0.001);
      camera.far = diameter * 1000;
      camera.position.set(diameter * 1.35, diameter * 0.9, diameter * 1.35);
      camera.updateProjectionMatrix();
      controls.target.set(0, 0, 0);
      controls.update();
      initialViewDirectionRef.current = camera.position.clone().sub(controls.target).normalize();

      scene.remove(grid);
      grid.geometry.dispose();
      asMaterials(grid.material).forEach((material) => material.dispose());
      grid = new THREE.GridHelper(diameter * 4, 20, 0x718096, 0xa7b0bd);
      grid.position.y = -size.y / 2;
      grid.visible = showGrid;
      gridRef.current = grid;
      scene.add(grid);

      const modelStats = inspectModel(root);
      setStats({ ...modelStats, animations: animations.length });
      if (animations.length) {
        const mixer = new THREE.AnimationMixer(root);
        const action = mixer.clipAction(animations[0]);
        action.play();
        action.paused = !playing;
        mixerRef.current = mixer;
        actionRef.current = action;
      }
      window.setTimeout(flushResolutions, 0);
      setNotice("");
    };

    const resize = () => {
      const width = Math.max(viewport.clientWidth, 1);
      const height = Math.max(viewport.clientHeight, 1);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
    };
    resizeObserver = new ResizeObserver(resize);
    resizeObserver.observe(viewport);
    resize();

    const clock = new THREE.Clock();
    const render = () => {
      frame = requestAnimationFrame(render);
      const delta = Math.min(clock.getDelta(), 0.05);
      if (!activeRef.current) return;
      mixerRef.current?.update(delta);
      if (autoRotateRef.current && root) root.rotation.y += delta * 0.45;
      controls.update();
      renderer.render(scene, camera);
    };
    render();
    if (model) {
      const bytes = decodeBase64(model.bytesBase64);
      try {
        if (model.path.toLowerCase().endsWith(".glb")) {
          new GLTFLoader(manager).parse(bytes, "", (gltf) => showModel(gltf.scene, gltf.animations), (error) => setNotice(t(`GLBを表示できません: ${String(error)}`, `Could not display the GLB: ${localizedDetail(error, "The model could not be parsed")}`)));
        } else {
          const loadedRoot = new FBXLoader(manager).parse(bytes, "");
          showModel(loadedRoot, loadedRoot.animations);
        }
      } catch (error) { setNotice(t(`3Dモデルを表示できません: ${String(error)}`, `Could not display the 3D model: ${localizedDetail(error, "The model could not be parsed")}`)); }
    }

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
      controls.dispose();
      controlsRef.current = null;
      cameraRef.current = null;
      initialViewDirectionRef.current = null;
      initialModelQuaternionRef.current = null;
      mixerRef.current?.stopAllAction();
      mixerRef.current = null;
      actionRef.current = null;
      modelRef.current = null;
      gridRef.current = null;
      if (root) disposeObject(root);
      grid.geometry.dispose();
      asMaterials(grid.material).forEach((material) => material.dispose());
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, [model]);

  const resetRotation = () => {
    setAutoRotate(false);
    autoRotateRef.current = false;
    if (modelRef.current && initialModelQuaternionRef.current) modelRef.current.quaternion.copy(initialModelQuaternionRef.current);
    const camera = cameraRef.current;
    const controls = controlsRef.current;
    const direction = initialViewDirectionRef.current;
    if (!camera || !controls || !direction) return;
    const distance = Math.max(camera.position.distanceTo(controls.target), 0.001);
    camera.position.copy(controls.target).addScaledVector(direction, distance);
    camera.up.set(0, 1, 0);
    controls.update();
  };

  const clearModel = () => {
    setAutoRotate(false);
    autoRotateRef.current = false;
    setModel(undefined);
    setStats(emptyStats);
    setResolutions([]);
    setNotice("");
  };

  const missingCount = resolutions.filter((item) => item.status !== "found").length;
  return <section className="model-preview">
    <div className="model-toolbar">
      <div className="model-title"><strong title={model?.path}>{model?.name ?? t("3Dプレビュー", "3D Preview")}</strong>{model && <span>{t(`${formatBytes(model.fileBytes)} ・ メッシュ ${stats.meshes} ・ 三角形 ${stats.triangles.toLocaleString()}`, `${formatBytes(model.fileBytes)} · ${stats.meshes} meshes · ${stats.triangles.toLocaleString()} triangles`)}</span>}{notice && <span role="status">{notice}</span>}</div>
      <div className="model-actions"><button onClick={clearModel} disabled={!model || loading}>{t("クリア", "Clear")}</button><button onClick={chooseModel} disabled={loading}>{model ? t("別のモデル", "Choose Another") : t("モデルを開く", "Open Model")}</button><button disabled={!model} onClick={() => model && revealItemInDir(model.path)}>{t("保存場所", "Show Location")}</button></div>
    </div>
    <div className="model-content">
      <div className="model-stage">
        <div ref={viewportRef} className="model-viewport" />
        {(!model || loading) && <div className="model-drop-prompt">{loading ? t("読み込み中…", "Loading…") : t("FBX / GLBをドロップ", "Drop FBX / GLB")}</div>}
        <div className="model-view-options">
          <label><input type="checkbox" checked={showGrid} onChange={(event) => setShowGrid(event.target.checked)} />{t("グリッド", "Grid")}</label>
          <label><input type="checkbox" checked={wireframe} onChange={(event) => setWireframe(event.target.checked)} />{t("ワイヤー", "Wireframe")}</label>
          <label><input type="checkbox" checked={autoRotate} onChange={(event) => setAutoRotate(event.target.checked)} />{t("自動回転", "Auto Rotate")}</label>
          <button onClick={resetRotation}>{t("回転リセット", "Reset Rotation")}</button>
          {stats.animations > 0 && <button onClick={() => setPlaying((value) => !value)}>{playing ? t("停止", "Pause") : t("再生", "Play")}</button>}
        </div>
      </div>
      <aside className="model-inspector">
        <div className="model-summary"><div><span>{t("テクスチャ候補", "Texture Candidates")}</span><strong>{model?.textures.length ?? 0}</strong></div><div className={missingCount ? "warning" : "ok"}><span>{t("不足・読込失敗", "Missing / Failed")}</span><strong>{missingCount}</strong></div></div>
        <h3>{t("テクスチャ参照", "Texture References")}</h3>
        {resolutions.length ? <div className="texture-resolution-list">{resolutions.map((item, index) => <div className={`texture-resolution ${item.status}`} key={`${item.request}-${index}`}>
          <span className="resolution-mark">{item.status === "found" ? "✓" : "!"}</span>
          <div><strong title={item.request}>{fileName(item.request)}</strong><small title={item.resolvedPath ?? item.request}>{item.resolvedPath ? relativeForDisplay(item.resolvedPath, model!.path) : item.request}</small></div>
        </div>)}</div> : <p className="model-empty-note">{t("参照なし", "No references")}</p>}
        {model?.scanLimited && <p className="model-warning">{t("画像が多いため、探索は1,500件で打ち切りました。", "The texture scan stopped after 1,500 files.")}</p>}
        <div className="model-stats"><span>{t(`マテリアル ${stats.materials}`, `Materials ${stats.materials}`)}</span><span>{t(`アニメーション ${stats.animations}`, `Animations ${stats.animations}`)}</span></div>
      </aside>
    </div>
  </section>;
}

function decodeBase64(value: string) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes.buffer;
}

function isModelPath(path: string) { return /\.(fbx|glb)$/i.test(path); }

function normalizePath(value: string) {
  let decoded = value;
  try { decoded = decodeURIComponent(value); } catch { /* 元の文字列で照合する */ }
  return decoded.split(/[?#]/, 1)[0].replace(/^file:(\/\/)?/i, "").replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
}

function findTexture(request: string, textures: TextureFile[]) {
  const wanted = normalizePath(request);
  const exact = textures.find((texture) => {
    const relative = normalizePath(texture.relativePath);
    const absolute = normalizePath(texture.path);
    return wanted === relative || wanted === absolute || wanted.endsWith(`/${relative}`);
  });
  if (exact) return exact;
  const name = wanted.split("/").pop();
  const byName = textures.filter((texture) => texture.name.toLowerCase() === name);
  // FBXLoaderは外部画像のディレクトリ部分を捨ててファイル名だけを渡す。
  // 配布用distなどに同名画像が複製されていても、FBXと同じ場所に最も近い
  // （相対パスが短い）候補を選び、参照を未解決にしない。
  return byName.sort((left, right) => texturePathPriority(left.relativePath) - texturePathPriority(right.relativePath))[0];
}

function texturePathPriority(value: string) {
  const normalized = value.replace(/\\/g, "/");
  const depth = normalized.split("/").length;
  return depth * 10000 + normalized.length;
}

function readableRequest(value: string) {
  try { return decodeURIComponent(value).replace(/\\/g, "/"); } catch { return value.replace(/\\/g, "/"); }
}

function fileName(value: string) { return readableRequest(value).split("/").pop() || value; }
function relativeForDisplay(texturePath: string, modelPath: string) {
  const folder = modelPath.replace(/[\\/][^\\/]+$/, "");
  return texturePath.toLowerCase().startsWith(folder.toLowerCase()) ? texturePath.slice(folder.length + 1) : texturePath;
}

function asMaterials(material: THREE.Material | THREE.Material[]) { return Array.isArray(material) ? material : [material]; }
function setMaterialWireframe(material: THREE.Material, enabled: boolean) {
  if ("wireframe" in material) (material as THREE.Material & { wireframe: boolean }).wireframe = enabled;
}

function inspectModel(root: THREE.Object3D) {
  let meshes = 0;
  let triangles = 0;
  const materials = new Set<THREE.Material>();
  root.traverse((object) => {
    if (!(object instanceof THREE.Mesh)) return;
    meshes += 1;
    const geometry = object.geometry;
    triangles += geometry.index ? geometry.index.count / 3 : (geometry.attributes.position?.count ?? 0) / 3;
    asMaterials(object.material).forEach((material) => materials.add(material));
  });
  return { meshes, triangles: Math.round(triangles), materials: materials.size };
}

function disposeObject(root: THREE.Object3D) {
  root.traverse((object) => {
    if (!(object instanceof THREE.Mesh)) return;
    object.geometry.dispose();
    asMaterials(object.material).forEach((material) => {
      Object.values(material).forEach((value) => { if (value instanceof THREE.Texture) value.dispose(); });
      material.dispose();
    });
  });
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}
