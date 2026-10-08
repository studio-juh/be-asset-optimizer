import { useEffect, useRef, useState } from "react";
import { locale, t } from "./i18n";
import type { StorePreviewTab } from "./storePreview";

type Topic = { id: StorePreviewTab; title: [string, string]; steps: [string[], string[]]; note: [string, string] };
const topics: Topic[] = [
  { id: "optimize", title: ["画像最適化", "Image Optimization"], steps: [
    ["画像やフォルダーをドロップします。Ctrl+Oでも選択できます。", "左側で出力先・形式・リサイズ・PNGの色形式を指定します。", "「変換を開始」を押し、完了後の「開く」で保存先を確認します。", "同じ画像を別の設定で処理する場合は「変換ステータスをクリア」を使います。"],
    ["Drop images or folders. Ctrl+O also opens the file picker.", "Choose the output folder, format, resize limits and PNG pixel format on the left.", "Start conversion, then use Open to reveal the saved file.", "Clear conversion status to process the same list with different settings."]], note: ["元画像は上書きしません。透過が必要な場合はPNG-32／WebPを選びます。PNG-8は減色するため、ノーマルの保存にはPNG-24を推奨します。", "Original files are not overwritten. Use PNG-32 or WebP for transparency. PNG-8 reduces colors; use PNG-24 for normal maps."] },
  { id: "normal", title: ["ノーマルマップ生成", "Normal Map Generation"], steps: [
    ["高さマップ、または明度から凹凸を推定したい画像を追加します。", "強さ・高さレベル・長辺の解像度を調整します。", "BlenderではOpenGL（+Y）を選びます。用途に応じてG反転を指定します。", "「生成を開始」で保存します。正方形補完は高さ0で余白を埋めます。"],
    ["Add a height map or an image whose luminance should represent surface height.", "Adjust strength, height level and the maximum long edge.", "Choose OpenGL (+Y) for Blender. Flip G when required by the target workflow.", "Start generation to save. Square padding fills extra height-map space with zero height."]], note: ["アルベドの明度には照明や模様も含まれます。生成結果は実際の形状を復元するものではないため、PBRプレビューで凹凸を確認してください。", "Albedo luminance includes lighting and painted patterns. This does not reconstruct actual geometry; inspect the result in PBR Preview."] },
  { id: "atlas", title: ["テクスチャアトラス", "Texture Atlas"], steps: [
    ["画像を任意の枠へドロップします。配置済み画像はドラッグで入れ替えられます。", "必要な枠を4分割します。最大16枚、7枚などの混在配置にも対応します。", "正方形セル・解像度・端の色を延長する余白を設定します。", "保存ボタンで結合画像を書き出します。未使用の枠は黒になります。"],
    ["Drop images into the desired cells. Drag filled cells to swap images.", "Split individual cells into four. Mixed layouts such as seven images, or up to sixteen, are supported.", "Set square cells, resolution and edge-extrusion padding.", "Save the atlas. Unused cells are filled with black."]], note: ["解像度不足バッジは枠より画像が小さい目安です。小さい画像は自動拡大しません。余白は境界の色にじみを軽減しますが、モデルのUV配置は別途調整が必要です。", "A low-resolution badge indicates an image smaller than its cell. Small images are not automatically upscaled. Padding reduces bleeding; model UVs still need to be adjusted separately."] },
  { id: "preview", title: ["3Dプレビュー・診断", "3D Preview / Diagnosis"], steps: [
    ["FBXまたはGLBをプレビュー領域にドロップします。", "マウス操作で回転・拡大縮小し、回転リセットで初期位置へ戻します。", "テクスチャの参照一覧を確認します。FBXの画像はモデルと同じフォルダー以下に置いてください。", "「アセット診断」で三角形数・UV不足・反転スケール・画像寸法などを確認します。"],
    ["Drop an FBX or GLB into the viewport.", "Orbit and zoom with the mouse; reset rotation to restore the initial view.", "Check texture references. Keep external FBX textures in the model folder or its subfolders.", "Diagnose Asset checks triangle counts, missing UVs, mirrored transforms and image dimensions."]], note: ["メモリ表示はRGBA8とミップマップによる概算です。ゲームエンジンの圧縮後容量とは異なります。外部参照画像と、埋め込みを含む読み込み済み画像は別表示です。", "Memory estimates assume RGBA8 with mipmaps, not engine-specific compression. External source images and loaded images including embedded textures are reported separately."] },
  { id: "material", title: ["PBRプレビュー", "PBR Preview"], steps: [
    ["アルベド・ノーマル・ラフネス・メタリック・AOの欄に画像を指定します。", "球・立方体・平面を選び、照明の明るさ・回転を調整します。既定は同梱のStudio HDRIです。", "ノーマル強度とG反転を調整します。マップ未指定のラフネス／メタリックはスライダーで設定できます。", "×で個別のマップを外し、「クリア」で全画像を解除します。"],
    ["Assign albedo, normal, roughness, metallic and AO images.", "Choose a sphere, cube or plane; adjust environment intensity and rotation. Bundled Studio HDRI is the default.", "Adjust normal strength and flip G. Roughness and metallic sliders apply when their maps are absent.", "Remove individual maps with ×, or clear all assigned images."]], note: ["確認用画像は長辺512pxです。ラフネスはG、メタリックはB、AOはRを読みます。ORM画像は3欄へ同じファイルを指定できます。HDRIはPoly HavenのMonochrome Studio 02（CC0）で、オフラインで利用できます。", "Preview images use a maximum long edge of 512px. Roughness reads G, metallic B and AO R. Assign the same ORM file to those three slots. The bundled Monochrome Studio 02 HDRI from Poly Haven (CC0) works offline."] },
  { id: "channels", title: ["チャンネル合成・分離", "Channel Packing / Split"], steps: [
    ["RGBA／ORM／Unity M/Sの配置プリセットを選びます。", "各チャンネルの領域へ画像をドロップ、または押して選択します。×で解除できます。", "入力チャンネル・反転・未指定時の固定値・出力解像度を指定して保存します。", "「RGBAを分離」は指定した最初の画像を4枚のグレースケールPNGへ分けます。"],
    ["Choose an RGBA, ORM or Unity M/S layout preset.", "Drop images into the channel areas or click to choose. Remove them with ×.", "Choose source channels, inversion, constants for unassigned channels and output resolution, then save.", "Split RGBA extracts the first assigned image into four grayscale PNG files."]], note: ["ORMはR=AO、G=Roughness、B=Metallicです。Unity M/SはR=Metallic、A=反転したRoughness（Smoothness）です。出力寸法が異なる入力は出力に合わせて伸縮します。", "ORM uses R=AO, G=Roughness, B=Metallic. Unity M/S uses R=Metallic and A=inverted Roughness (Smoothness). Inputs with different dimensions are stretched to the output size."] },
  { id: "adjust", title: ["ノーマル調整", "Normal Adjustment"], steps: [
    ["ベースのノーマル画像をドロップ、または領域を押して選択します。", "細かい凹凸を合成する場合は「詳細」に別のノーマルを指定します。", "ベース／詳細の強度とG反転を変更し、元画像と調整後を見比べます。", "「保存」で元のベース解像度のPNGを書き出します。×は個別解除、「クリア」は全入力解除です。"],
    ["Drop a base normal map or click its area to choose one.", "Optionally assign a detail normal map to blend smaller surface details.", "Change base/detail strength and flip G; compare Original and Adjusted.", "Save exports a PNG at the base image's original resolution. × removes one input; Clear removes all inputs."]], note: ["プレビューは長辺512pxで、保存前に自動更新されます。合成はRNM方式、出力は正規化したPNG-24です。画像サイズが違う詳細ノーマルはベースに合わせて伸縮します。", "The 512px preview updates before saving. Blending uses RNM; output is a normalized PNG-24. Detail maps with different dimensions are stretched to match the base."] },
];

export default function HelpPage({ initialTopic, onClose }: { initialTopic: StorePreviewTab; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const article = useRef<HTMLElement>(null);
  const [selected, setSelected] = useState(initialTopic);
  const [expanded, setExpanded] = useState(false);
  const topic = topics.find(item => item.id === selected)!;
  const language = locale === "ja" ? 0 : 1;
  useEffect(() => { dialog.current?.showModal(); }, []);
  useEffect(() => { setExpanded(false); article.current?.scrollTo(0, 0); }, [selected]);
  return <dialog ref={dialog} className="help-dialog" onCancel={onClose} onClose={onClose} aria-labelledby="help-title">
    <header className="help-header"><h2 id="help-title">{t("使い方 — Be Asset Optimizer", "Help — Be Asset Optimizer")}</h2><button autoFocus onClick={onClose} aria-label={t("ヘルプを閉じる", "Close help")}>×</button></header>
    <div className="help-layout"><nav className="help-nav" aria-label={t("ヘルプ目次", "Help topics")}>{topics.map(item => <button key={item.id} className={selected === item.id ? "active" : ""} aria-current={selected === item.id ? "page" : undefined} onClick={() => setSelected(item.id)}>{item.title[language]}</button>)}<p>{t("F1でヘルプを表示\nEscで閉じる", "F1 opens help\nEsc closes help")}</p></nav>
      <article ref={article} className="help-article"><h3>{topic.title[language]}</h3><p className="help-basics">{t("タブを切り替えても入力内容は保持されます。出力先未指定の場合は入力元の近くに保存され、元画像は上書きしません。", "Inputs remain when switching tabs. Without a specified output folder, files are saved near the source. Originals are not overwritten.")}</p>
        <button className={`help-screenshot ${expanded ? "expanded" : ""}`} onClick={() => setExpanded(value => !value)} aria-expanded={expanded} aria-label={t("スクリーンショットを拡大／縮小", "Expand / shrink screenshot")}><img key={`${locale}-${selected}`} src={`${import.meta.env.BASE_URL}help/${locale}/${selected}.png`} alt={t(`${topic.title[0]}の操作画面`, `${topic.title[1]} screenshot`)} /></button>
        <ol>{topic.steps[language].map(step => <li key={step}>{step}</li>)}</ol><p className="help-note">{topic.note[language]}</p>
      </article></div>
  </dialog>;
}
