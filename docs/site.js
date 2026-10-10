const screens = {
  normal: ["ノーマルマップ生成の設定と画像一覧", "画像の明度や高さマップからノーマルマップを生成。"],
  atlas: ["3枚の画像を配置したテクスチャアトラス画面", "画像を任意の枠に配置。未使用の枠は黒で補完します。"],
  material: ["HDRI照明で球のマテリアルを確認するPBRプレビュー", "アルベドやノーマルを読み込んで、マテリアルの見え方を確認。"],
  channels: ["ORMプリセットで画像を配置したチャンネル合成画面", "入力チャンネルを選び、RGBA合成・分離やORM素材の準備に。"],
  adjust: ["元のノーマルと調整後を並べたノーマル調整画面", "強度調整や詳細ノーマルの合成結果を、並べて確認。"],
  preview: ["GLBモデルを表示しアセット診断した3Dプレビュー", "FBX・GLBを読み込んで、形状・テクスチャ・アセット情報を確認。"]
};
const image = document.querySelector("#gallery-image");
const caption = document.querySelector("#gallery-caption");
const buttons = document.querySelectorAll("[data-screen]");
for (const button of buttons) button.addEventListener("click", () => {
  const key = button.dataset.screen;
  if (!screens[key]) return;
  image.src = `assets/${key}.png`;
  image.alt = screens[key][0];
  caption.textContent = screens[key][1];
  for (const item of buttons) item.setAttribute("aria-pressed", String(item === button));
});
