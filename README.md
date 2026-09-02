# Be Asset Optimizer

Blenderで使うテクスチャと3Dアセットをまとめて整える、Windows向けデスクトップアプリです。画像は外部へ送信されません。

## 現在の機能

- PNG / JPEG / WebP / HEIC / BMP / TIFF / GIF / TGA / DDS、または画像フォルダーのドラッグ＆ドロップとファイル選択
- PNG（既定）/ WebP / JPEG 出力と、WebP・JPEGの品質指定
- 対応画像のローカルAI復元と 2倍 / 4倍アップスケール
- リスト形式の一括処理、個別の状態・寸法・サイズ表示
- 縦横比を維持したリサイズ（高品質 / ピクセルアート）
- 256〜16 色のインデックスカラー化とディザリング
- oxipng による高速 / 標準 / 最大の最終最適化
- 安全な別フォルダ出力と、元より大きい結果の自動スキップ
- Blender 向けノーマルマップの一括生成（対応画像の明度を利用）
- OpenGL (+Y) / DirectX (-Y) 切替と強さの調整

HEICは一覧追加時にヘッダーだけを高速解析し、変換時に純Rustデコーダーで向き補正と表示向けsRGB変換を適用します。
AI復元は追加ダウンロードした Real-ESRGAN Vulkan 版で処理します。追加完了後の処理時には通信、Python、CUDAは不要です。
既定の「自然・忠実」は RealESRNet と元画像の50%合成で偽の細部を抑えます。「高精細」では RealESRGAN を使用し、復元強度を25〜100%で調整できます。
AI復元の既定は512px分割の画質優先です。GPUメモリ不足になる場合は分割品質を標準または低メモリへ下げられます。

AI実行環境とモデル本体はGitや通常の配布物に含めません。AI復元タブの「AI機能を追加」から専用Releaseを約61 MB取得し、ZIPと各ファイルのSHA-256検証後に約70 MBを保存します。専用Releaseを取得できない場合は公式Real-ESRGAN Releaseへ自動的に切り替えます。通常版はユーザーデータ領域、`portable.marker` があるポータブル版はEXE横の `components` フォルダーを使用します。

GIFは先頭フレームを静止画として使用します。APNG、16-bit PNG、HEIC画像シーケンス、HDR HEICのトーンマッピングは現在の MVP の対象外です。

## 開発起動

```powershell
npm install
npm run tauri -- dev
```

開発環境でAIコンポーネントを事前取得する場合は `npm run prepare:ai` を実行します。通常のビルドには同梱されません。

## 検証とビルド

```powershell
npm run build
Set-Location src-tauri; cargo test
Set-Location ..; npm run tauri -- build
```

デバッグ用の Windows インストーラーは `src-tauri/target/debug/bundle/` 以下に生成されます。

## License

GPL-3.0-or-later。詳細は [LICENSE](LICENSE) と [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) を参照してください。
