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
AI復元は同梱した Real-ESRGAN Vulkan 版で処理します。処理時の通信、Python、CUDA は不要です。
AI復元の既定は512px分割の画質優先です。GPUメモリ不足になる場合は分割品質を標準または低メモリへ下げられます。

GIFは先頭フレームを静止画として使用します。APNG、16-bit PNG、HEIC画像シーケンス、HDR HEICのトーンマッピングは現在の MVP の対象外です。

## 開発起動

```powershell
npm install
npm run tauri -- dev
```

## 検証とビルド

```powershell
npm run build
Set-Location src-tauri; cargo test
Set-Location ..; npm run tauri -- build
```

デバッグ用の Windows インストーラーは `src-tauri/target/debug/bundle/` 以下に生成されます。

## License

GPL-3.0-or-later。詳細は [LICENSE](LICENSE) と [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) を参照してください。
