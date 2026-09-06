# コマンドライン変換

`be-asset-optimizer-cli.exe` はGUIと同じ変換処理を使用します。
ウィンドウを開かず、CMD / PowerShell / BlenderのPythonなどから呼び出せます。
現在の対象は画像最適化（`optimize`）とノーマルマップ生成（`normal`）です。
AI復元・アトラス・HTTP APIは対象外です。

## ビルド

開発環境で `npm run build:cli` を実行すると、
`src-tauri/target/release/be-asset-optimizer-cli.exe` が生成されます。
このCLIは別実行ファイルです。既存のStoreパッケージには含まれません。
WindowsへPATH登録はしないため、実行ファイルのあるフォルダーか絶対パスで呼び出してください。

## CMDの例

```bat
be-asset-optimizer-cli.exe --help
be-asset-optimizer-cli.exe optimize "C:\Textures\albedo.png" --format webp --quality 82 -o "C:\Converted"
be-asset-optimizer-cli.exe optimize "C:\Textures" --max-width 1024 --max-height 1024 -o "C:\Converted"
be-asset-optimizer-cli.exe optimize "C:\Textures\a.png" "C:\Textures\b.jpg" --color-mode indexed --colors 256
be-asset-optimizer-cli.exe normal "C:\Textures\height.png" --max-long-edge 1024 --pad-to-square -o "C:\Normals"
be-asset-optimizer-cli.exe optimize "C:\Textures" --json > results.jsonl
echo %ERRORLEVEL%
```

PowerShellでは、現在のフォルダーにあるEXEは `./be-asset-optimizer-cli.exe` で実行します。
空白を含む絶対パスなら `& "C:\My Tools\be-asset-optimizer-cli.exe" optimize ...` とします。
終了コードは `$LASTEXITCODE` で取得できます。

## 外部プログラムとの連携

`--json` は標準出力に1行1件のJSONを出力します。最後の行は集計です。
ファイル別結果の主な項目は `path`、`status`（`done` / `skipped` / `failed`）、
`output_path`、`output_bytes`、`output_width`、`output_height`、`message` です。
エラーやスキップの場合、一部の項目は `null` になります。
`message` はGUIと共有しているため日本語を含みます。判定には `status` を使用してください。

```json
{"type":"summary","done":2,"skipped":1,"failed":0}
```

BlenderのPythonからも、GUIの起動なしに同期実行できます。

```python
import json
import subprocess

result = subprocess.run(
    [r"C:\Tools\be-asset-optimizer-cli.exe", "optimize",
     r"C:\Textures\albedo.png", "--format", "webp", "--json"],
    capture_output=True, text=True, encoding="utf-8",
)
records = [json.loads(line) for line in result.stdout.splitlines()]
if result.returncode != 0:
    print(result.stderr, records)
```

## 入出力の仕様

- 終了コード: `0` 成功またはスキップのみ、`1` 入力・変換失敗あり、`2` 引数不正。
- 複数入力のうち1件が失敗しても、残りの変換を続行します。
- フォルダーは8階層まで再帰探索します。ワイルドカードは展開しません。
- 変換開始前に対象を一覧化します。既存の出力フォルダーが入力内にあれば、その画像も対象になります。反復処理では出力先を入力フォルダーの外へ指定してください。
- 同じ入力の重複は除外します。出力は逐次処理し、既存の出力名と重なると連番を追加します。
- 出力先省略時はGUI同様、画像最適化は入力元の `optimized`、ノーマル生成は `normal_maps` に保存します。
- PNG最適化では、出力が元PNG以上の容量になるとGUI同様にスキップします。
- リサイズは縦横比を保持し、拡大は行いません。幅・高さ・倍率のうち最も小さくなる条件を適用します。
- PNG-8で品質を保てない場合はGUI同様に可逆PNGへ切り替えます。
- `--` 以降はすべて入力パスとして扱います。

署名されていないCLIは、GUI版と同様にWindowsのアプリ実行制御の対象になります。
