# SmartPNG

ローカルで PNG をまとめて軽量化する Windows 向けデスクトップアプリです。画像は外部へ送信されません。

## 現在の機能

- 複数 PNG のドラッグ＆ドロップとファイル選択
- リスト形式の一括処理、個別の状態・寸法・サイズ表示
- 縦横比を維持したリサイズ（高品質 / ピクセルアート）
- 256〜16 色のインデックスカラー化とディザリング
- oxipng による最終最適化
- 安全な別フォルダ出力と、元より大きい結果の自動スキップ
- Blender 向けノーマルマップの一括生成（高さマップ PNG の明度を利用）
- OpenGL (+Y) / DirectX (-Y) 切替と強さの調整

APNG、16-bit PNG、ICC プロファイル付き PNG は現在の MVP の対象外です。

## 開発起動

```powershell
npm install
npm run tauri -- dev
```

## 検証とビルド

```powershell
npm run build
Set-Location src-tauri; cargo test
Set-Location ..; npm run tauri -- build --debug
```

デバッグ用の Windows インストーラーは `src-tauri/target/debug/bundle/` 以下に生成されます。

## License

GPL-3.0-or-later。詳細は [LICENSE](LICENSE) と [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) を参照してください。
