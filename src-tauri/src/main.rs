#![windows_subsystem = "windows"]

use std::{
  collections::{HashMap, HashSet},
  fs,
  path::{Path, PathBuf},
  process::{Command, Stdio},
  sync::{Mutex, OnceLock},
  time::{SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use bytemuck::cast_slice;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{imageops::FilterType, GenericImageView, ImageEncoder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileMetadata { path: String, name: String, original_bytes: u64, width: u32, height: u32 }

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InspectFailure { path: String, message: String }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectResult { files: Vec<FileMetadata>, failures: Vec<InspectFailure> }

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InspectProgress {
  request_id: String,
  completed: usize,
  total: usize,
  current_name: String,
  file: Option<FileMetadata>,
  failure: Option<InspectFailure>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum Status { Processing, Done, Skipped, Failed }

#[derive(Debug, Serialize, Clone)]
struct Progress { path: String, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings { output_dir: Option<String>, output_format: String, quality: u8, jpeg_background: String, max_width: Option<u32>, max_height: Option<u32>, scale_percent: Option<u32>, resize_mode: String, color_mode: String, colors: Option<u32>, dithering: bool, optimization: String }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NormalSettings { output_dir: Option<String>, max_long_edge: Option<u32>, strength: f32, level: f32, convention: String, invert_height: bool, invert_green: bool, pad_to_square: bool }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiRestoreSettings { output_dir: Option<String>, output_scale: u32, tile_size: Option<u32> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasSettings { output_dir: Option<String>, pad_to_square: bool, square_resolution: Option<u32> }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AtlasResult { output_path: String, output_bytes: u64, width: u32, height: u32 }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelTextureFile { path: String, relative_path: String, name: String, bytes: u64 }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelPackage { path: String, name: String, bytes_base64: String, file_bytes: u64, textures: Vec<ModelTextureFile>, scan_limited: bool }

#[derive(Clone)]
struct CachedHeicPreview { file_bytes: u64, modified: Option<SystemTime>, width: u32, height: u32, image: image::RgbaImage }

static HEIC_PREVIEW_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedHeicPreview>>> = OnceLock::new();

#[tauri::command]
async fn inspect_files(app: AppHandle, paths: Vec<String>, request_id: String, max_files: Option<usize>) -> Result<InspectResult, String> {
  tauri::async_runtime::spawn_blocking(move || {
    inspect_files_impl(paths, max_files.unwrap_or(500).clamp(1, 1000), |completed, total, path, file, failure| {
      let current_name = path.file_name().and_then(|value| value.to_str()).unwrap_or("画像").to_string();
      let _ = app.emit("inspect-files-progress", InspectProgress {
        request_id: request_id.clone(),
        completed,
        total,
        current_name,
        file: file.cloned(),
        failure: failure.cloned(),
      });
    })
  }).await.map_err(|error| error.to_string())?
}

fn inspect_files_impl<F>(paths: Vec<String>, max_files: usize, mut progress: F) -> Result<InspectResult, String>
where F: FnMut(usize, usize, &Path, Option<&FileMetadata>, Option<&InspectFailure>) {
  let mut candidates = Vec::new();
  let mut failures = Vec::new();
  let mut seen = HashSet::new();
  for path in paths {
    collect_image_inputs(&PathBuf::from(path), 0, max_files, &mut candidates, &mut failures, &mut seen);
    if candidates.len() >= max_files { break; }
  }

  let mut files = Vec::new();
  let total = candidates.len();
  for (index, path) in candidates.into_iter().enumerate() {
    let result = (|| {
      let metadata = fs::metadata(&path).map_err(|error| format!("画像を開けません: {error}"))?;
      let (width, height) = input_dimensions(&path)?;
      let name = path.file_name().ok_or("ファイル名を取得できません")?.to_string_lossy().to_string();
      Ok::<_, String>(FileMetadata { path: path.to_string_lossy().to_string(), name, original_bytes: metadata.len(), width, height })
    })();
    match result {
      Ok(file) => {
        progress(index + 1, total, &path, Some(&file), None);
        files.push(file);
      }
      Err(message) => {
        let failure = InspectFailure { path: path.to_string_lossy().to_string(), message };
        progress(index + 1, total, &path, None, Some(&failure));
        failures.push(failure);
      }
    }
  }
  Ok(InspectResult { files, failures })
}

fn collect_image_inputs(
  path: &Path,
  depth: usize,
  max_files: usize,
  output: &mut Vec<PathBuf>,
  failures: &mut Vec<InspectFailure>,
  seen: &mut HashSet<String>,
) {
  if output.len() >= max_files { return; }
  let metadata = match fs::symlink_metadata(path) {
    Ok(value) => value,
    Err(error) => {
      failures.push(InspectFailure { path: path.to_string_lossy().to_string(), message: format!("ファイルを確認できません: {error}") });
      return;
    }
  };
  if metadata.file_type().is_symlink() { return; }
  if metadata.is_file() {
    if !is_supported_image(path) { return; }
    let key = path.to_string_lossy().to_lowercase();
    if seen.insert(key) { output.push(path.to_path_buf()); }
    return;
  }
  if !metadata.is_dir() { return; }
  if depth >= 8 {
    failures.push(InspectFailure { path: path.to_string_lossy().to_string(), message: "フォルダー階層が深いため、この先は読み込んでいません".into() });
    return;
  }
  let entries = match fs::read_dir(path) {
    Ok(entries) => entries,
    Err(error) => {
      failures.push(InspectFailure { path: path.to_string_lossy().to_string(), message: format!("フォルダーを開けません: {error}") });
      return;
    }
  };
  let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
  entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
  for entry in entries {
    if output.len() >= max_files { break; }
    collect_image_inputs(&entry.path(), depth + 1, max_files, output, failures, seen);
  }
}

#[tauri::command]
async fn create_preview(path: String, size: u32) -> Result<String, String> {
  tauri::async_runtime::spawn_blocking(move || make_preview(&PathBuf::from(path), size.clamp(32, 512)).ok_or("プレビューを生成できませんでした".into()))
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn load_model_package(app: AppHandle, path: String) -> Result<ModelPackage, String> {
  tauri::async_runtime::spawn_blocking(move || {
    let model_path = PathBuf::from(&path);
    if !is_preview_model(&model_path) {
      return Err("FBXまたはGLBファイルを選択してください".into());
    }
    let metadata = fs::metadata(&model_path).map_err(|error| format!("3Dモデルを開けません: {error}"))?;
    if !metadata.is_file() { return Err("FBXまたはGLBファイルを選択してください".into()); }
    if metadata.len() > 300 * 1024 * 1024 { return Err("300 MBを超える3Dモデルはプレビューできません".into()); }
    let bytes = fs::read(&model_path).map_err(|error| format!("3Dモデルを読み込めません: {error}"))?;
    let root = model_path.parent().ok_or("3Dモデルの保存フォルダーを取得できません")?;
    app.asset_protocol_scope().allow_directory(root, true).map_err(|error| format!("テクスチャフォルダーを表示用に許可できません: {error}"))?;
    let mut textures = Vec::new();
    let mut scan_limited = false;
    collect_model_textures(root, root, 0, &mut textures, &mut scan_limited);
    Ok(ModelPackage {
      path: model_path.to_string_lossy().to_string(),
      name: model_path.file_name().and_then(|value| value.to_str()).unwrap_or("model.fbx").to_string(),
      bytes_base64: STANDARD.encode(bytes),
      file_bytes: metadata.len(),
      textures,
      scan_limited,
    })
  }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn process_batch(app: AppHandle, paths: Vec<String>, settings: Settings) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    for path in paths {
      let path_buf = PathBuf::from(&path);
      emit(&app, &path, Status::Processing, None, None, None, Some("変換中".into()), None);
      match process_file(&path_buf, &settings) {
        Ok(result) => emit(&app, &path, result.0, result.1, Some(result.2), Some(result.3), result.4, result.5),
        Err(error) => emit(&app, &path, Status::Failed, None, None, None, Some(error), None),
      }
    }
  }).await.map_err(|error| error.to_string())?;
  Ok(())
}

#[tauri::command]
async fn process_normal_batch(app: AppHandle, paths: Vec<String>, settings: NormalSettings) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    for path in paths {
      let path_buf = PathBuf::from(&path);
      emit_normal(&app, &path, Status::Processing, None, None, None, Some("ノーマルマップを生成中".into()), None);
      match process_normal_file(&path_buf, &settings) {
        Ok(result) => emit_normal(&app, &path, result.0, result.1, Some(result.2), Some(result.3), result.4, result.5),
        Err(error) => emit_normal(&app, &path, Status::Failed, None, None, None, Some(error), None),
      }
    }
  }).await.map_err(|error| error.to_string())?;
  Ok(())
}

#[tauri::command]
async fn process_ai_restore_batch(app: AppHandle, paths: Vec<String>, settings: AiRestoreSettings) -> Result<(), String> {
  tauri::async_runtime::spawn_blocking(move || {
    let runtime = match resolve_realesrgan_runtime(&app) {
      Ok(runtime) => runtime,
      Err(error) => {
        for path in paths {
          emit_ai_restore(&app, &path, Status::Failed, None, None, None, Some(error.clone()), None);
        }
        return;
      }
    };
    for path in paths {
      let path_buf = PathBuf::from(&path);
      emit_ai_restore(&app, &path, Status::Processing, None, None, None, Some("AIで復元中".into()), None);
      match process_ai_restore_file(&path_buf, &settings, &runtime) {
        Ok(result) => emit_ai_restore(&app, &path, result.0, result.1, Some(result.2), Some(result.3), result.4, result.5),
        Err(error) => emit_ai_restore(&app, &path, Status::Failed, None, None, None, Some(error), None),
      }
    }
  }).await.map_err(|error| error.to_string())?;
  Ok(())
}

#[tauri::command]
async fn create_texture_atlas(paths: Vec<String>, settings: AtlasSettings) -> Result<AtlasResult, String> {
  tauri::async_runtime::spawn_blocking(move || {
    if paths.len() != 4 { return Err("テクスチャアトラスには画像を4枚指定してください".into()); }
    let mut images = Vec::with_capacity(4);
    for path in &paths {
      let path = PathBuf::from(path);
      if !is_supported_image(&path) { return Err("PNGまたはHEICファイルを4枚指定してください".into()); }
      images.push(load_input_image(&path)?.to_rgba8());
    }
    let atlas = build_texture_atlas(&images, settings.pad_to_square, settings.square_resolution)?;
    let (width, height) = atlas.dimensions();
    let encoded = encode_rgba(&atlas, width, height)?;
    let optimized = oxipng::optimize_from_memory(&encoded, &lossless_options(1)).map_err(|error| error.to_string())?;
    let first_path = PathBuf::from(&paths[0]);
    let output_dir = if let Some(dir) = settings.output_dir.as_deref().filter(|value| !value.trim().is_empty()) { PathBuf::from(dir) } else { first_path.parent().ok_or("入力元フォルダを取得できません")?.join("atlas") };
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let output_path = unique_named_output_path(&output_dir, "texture", "atlas");
    fs::write(&output_path, optimized.as_slice()).map_err(|error| error.to_string())?;
    Ok(AtlasResult { output_path: output_path.to_string_lossy().to_string(), output_bytes: optimized.len() as u64, width, height })
  }).await.map_err(|error| error.to_string())?
}

fn emit(app: &AppHandle, path: &str, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String>) {
  let _ = app.emit("job-progress", Progress { path: path.into(), status, output_bytes, output_width, output_height, message, output_path });
}

fn emit_normal(app: &AppHandle, path: &str, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String>) {
  let _ = app.emit("normal-job-progress", Progress { path: path.into(), status, output_bytes, output_width, output_height, message, output_path });
}

fn emit_ai_restore(app: &AppHandle, path: &str, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String>) {
  let _ = app.emit("ai-restore-job-progress", Progress { path: path.into(), status, output_bytes, output_width, output_height, message, output_path });
}

struct RealEsrganRuntime { executable: PathBuf, working_dir: PathBuf }

fn resolve_realesrgan_runtime(app: &AppHandle) -> Result<RealEsrganRuntime, String> {
  let mut candidates = Vec::new();
  if let Ok(resource_dir) = app.path().resource_dir() {
    candidates.push(resource_dir.join("resources").join("realesrgan"));
    candidates.push(resource_dir.join("realesrgan"));
  }
  if let Ok(executable) = std::env::current_exe() {
    if let Some(parent) = executable.parent() {
      candidates.push(parent.join("resources").join("realesrgan"));
    }
  }
  #[cfg(debug_assertions)]
  candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("realesrgan"));

  for working_dir in candidates {
    let executable = working_dir.join("realesrgan-ncnn-vulkan.exe");
    let model = working_dir.join("models").join("realesrgan-x4plus.bin");
    let parameters = working_dir.join("models").join("realesrgan-x4plus.param");
    if executable.is_file() && model.is_file() && parameters.is_file() {
      return Ok(RealEsrganRuntime { executable, working_dir });
    }
  }
  Err("AI復元モデルが見つかりません。アプリを再インストールしてください".into())
}

fn process_ai_restore_file(path: &Path, settings: &AiRestoreSettings, runtime: &RealEsrganRuntime) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_supported_image(path) { return Err("PNGまたはHEICファイルではありません".into()); }
  if !path.is_file() { return Err("入力画像を開けません".into()); }
  if !matches!(settings.output_scale, 1 | 2 | 4) { return Err("出力倍率が正しくありません".into()); }
  if settings.tile_size.is_some_and(|value| value != 0 && value < 32) { return Err("タイルサイズは自動または32 px以上にしてください".into()); }

  // GPUへ渡す前はヘッダーだけを読み、元画像全体のCPUデコードを避ける。
  let (source_width, source_height) = input_dimensions(path)?;
  let id = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
  let temporary_dir = std::env::temp_dir().join(format!("smartpng-ai-{}-{id}", std::process::id()));
  fs::create_dir_all(&temporary_dir).map_err(|error| format!("一時フォルダーを作成できません: {error}"))?;
  let temporary_input = temporary_dir.join("input.png");
  let temporary_output = temporary_dir.join("output.png");
  if is_png(path) {
    fs::copy(path, &temporary_input).map_err(|error| format!("画像を処理用に準備できません: {error}"))?;
  } else {
    let source = load_input_image(path)?;
    let has_alpha = source.color().has_alpha();
    let source = source.to_rgba8();
    let encoded = if has_alpha { encode_rgba(&source, source.width(), source.height())? } else { encode_rgb24(&source, source.width(), source.height())? };
    fs::write(&temporary_input, encoded).map_err(|error| format!("HEICを処理用PNGへ変換できません: {error}"))?;
  }

  let model_scale = if settings.output_scale == 1 { 4 } else { settings.output_scale };
  let mut command = Command::new(&runtime.executable);
  command
    .current_dir(&runtime.working_dir)
    .arg("-i").arg(&temporary_input)
    .arg("-o").arg(&temporary_output)
    .arg("-s").arg(model_scale.to_string())
    .arg("-t").arg(settings.tile_size.unwrap_or(0).to_string())
    .arg("-m").arg("models")
    .arg("-n").arg("realesrgan-x4plus")
    .arg("-f").arg("png")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  #[cfg(windows)]
  command.creation_flags(0x0800_0000);

  let result = command.output().map_err(|error| format!("AI復元エンジンを起動できません: {error}"));
  let engine_result = match result {
    Ok(output) if output.status.success() => Ok(()),
    Ok(output) => {
      let details = String::from_utf8_lossy(&output.stderr);
      let message = if details.contains("vkCreate") || details.contains("gpu") || details.contains("GPU") {
        "AI復元を実行できません。GPUドライバーがVulkanに対応しているか確認してください".into()
      } else {
        let details = details.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("不明なエラー");
        format!("AI復元に失敗しました: {details}")
      };
      Err(message)
    }
    Err(error) => Err(error),
  };
  if let Err(error) = engine_result {
    let _ = fs::remove_dir_all(&temporary_dir);
    return Err(error);
  }

  // 2倍・4倍はNCNNが出力したPNGをそのまま保存する。従来行っていた
  // CPUでの全画素デコード、再エンコード、OxiPNG最適化は不要。
  let save_result = (|| {
    let output_dir = if let Some(dir) = settings.output_dir.as_deref().filter(|value| !value.trim().is_empty()) {
      PathBuf::from(dir)
    } else {
      path.parent().ok_or("入力元フォルダを取得できません")?.join("ai_restored")
    };
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let suffix = match settings.output_scale { 1 => "ai-restored", 2 => "ai-x2", _ => "ai-x4" };
    let output_path = unique_named_output_path(&output_dir, path.file_stem().and_then(|value| value.to_str()).unwrap_or("image"), suffix);

    if settings.output_scale != 1 {
      let (width, height) = image::image_dimensions(&temporary_output).map_err(|error| format!("AI復元結果を確認できません: {error}"))?;
      let output_bytes = fs::copy(&temporary_output, &output_path).map_err(|error| format!("AI復元結果を保存できません: {error}"))?;
      let message = if settings.output_scale == 2 { "AI復元して2倍に拡大しました" } else { "AI復元して4倍に拡大しました" };
      return Ok((Status::Done, Some(output_bytes), width, height, Some(message.into()), Some(output_path.to_string_lossy().to_string())));
    }

    // 元寸法モードだけは4倍のAI出力を縮小する必要がある。PNGは一度だけ
    // 高速エンコードし、重いOxiPNG処理は画像最適化タブへ任せる。
    let processed = image::open(&temporary_output).map_err(|error| format!("AI復元結果を読み込めません: {error}"))?;
    let has_alpha = processed.color().has_alpha();
    let output = processed.resize_exact(source_width, source_height, FilterType::Lanczos3).to_rgba8();
    let encoded = if has_alpha { encode_rgba(&output, source_width, source_height)? } else { encode_rgb24(&output, source_width, source_height)? };
    fs::write(&output_path, &encoded).map_err(|error| format!("AI復元結果を保存できません: {error}"))?;
    Ok((Status::Done, Some(encoded.len() as u64), source_width, source_height, Some("AI復元後、元の寸法に戻しました".into()), Some(output_path.to_string_lossy().to_string())))
  })();
  let _ = fs::remove_dir_all(&temporary_dir);
  save_result
}

fn process_file(path: &Path, settings: &Settings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_supported_image(path) { return Err("PNGまたはHEICファイルではありません".into()); }
  let original = fs::read(path).map_err(|error| error.to_string())?;
  if is_png(path) && original.windows(4).any(|chunk| chunk == b"acTL") { return Err("APNG は MVP では未対応です".into()); }
  let source = load_input_image_from_bytes(path, &original)?;
  let source_has_alpha = source.color().has_alpha();
  let (source_width, source_height) = source.dimensions();
  let (width, height) = resize_dimensions(source_width, source_height, settings);
  let changed_size = (width, height) != (source_width, source_height);
  let rgba = if changed_size { image::imageops::resize(&source.to_rgba8(), width, height, resize_filter(&settings.resize_mode)) } else { source.to_rgba8() };

  let quality = settings.quality.clamp(1, 100);
  let (optimized, extension, message, skip_when_larger) = match settings.output_format.as_str() {
    "webp" => (encode_webp(&rgba, width, height, quality)?, "webp", "WebPに変換しました", false),
    "jpeg" | "jpg" => (encode_jpeg(&rgba, width, height, quality, &settings.jpeg_background)?, "jpg", "JPEGに変換しました", false),
    _ => {
      let (encoded, fell_back_to_lossless) = match settings.color_mode.as_str() {
        "indexed" => match encode_indexed(&rgba, width, height, settings.colors.unwrap_or(256), settings.dithering) {
          Ok(encoded) => (encoded, false),
          // ノーマルマップや写真など、PNG-8 では十分な品質を保てない画像は
          // 失敗にせず、画素を変えない可逆最適化へ切り替える。
          Err(error) if is_quality_too_low(&error) => (if changed_size || !is_png(path) { encode_preserving_alpha(&rgba, width, height, source_has_alpha)? } else { original.clone() }, true),
          Err(error) => return Err(error),
        },
        "rgb24" => (encode_rgb24(&rgba, width, height)?, false),
        "rgba32" => (encode_rgba(&rgba, width, height)?, false),
        "grayscale8" => (encode_grayscale(&rgba, width, height)?, false),
        _ if changed_size || !is_png(path) => (encode_preserving_alpha(&rgba, width, height, source_has_alpha)?, false),
        _ => (original.clone(), false),
      };
      let preset = match settings.optimization.as_str() { "max" => 6, "safe" => 1, _ => 0 };
      let options = lossless_options(preset);
      let optimized = oxipng::optimize_from_memory(&encoded, &options).map_err(|error| error.to_string())?;
      let message = if fell_back_to_lossless { "PNG-8では品質が保てないため、可逆PNGで保存しました" } else if is_heic(path) { "HEICをPNGに変換しました" } else { "PNGを最適化しました" };
      (optimized, "png", message, true)
    }
  };
  if is_png(path) && skip_when_larger && optimized.len() as u64 >= original.len() as u64 {
    return Ok((Status::Skipped, None, source_width, source_height, Some("元ファイルより小さくなりませんでした".into()), None));
  }
  let output_dir = resolve_output_dir(path, settings.output_dir.as_deref())?;
  fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
  let output_path = unique_output_path(&output_dir, path.file_stem().and_then(|v| v.to_str()).unwrap_or("image"), extension);
  fs::write(&output_path, optimized.as_slice()).map_err(|error| error.to_string())?;
  Ok((Status::Done, Some(optimized.len() as u64), width, height, Some(message.into()), Some(output_path.to_string_lossy().to_string())))
}

fn process_normal_file(path: &Path, settings: &NormalSettings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_supported_image(path) { return Err("PNGまたはHEICファイルではありません".into()); }
  let original = fs::read(path).map_err(|error| error.to_string())?;
  if is_png(path) && original.windows(4).any(|chunk| chunk == b"acTL") { return Err("APNG は MVP では未対応です".into()); }
  let source = load_input_image_from_bytes(path, &original)?.to_rgba8();
  let source = resize_height_map_long_edge(&source, settings.max_long_edge);
  let source = if settings.pad_to_square { pad_height_map_to_square(&source) } else { source };
  let (width, height) = source.dimensions();
  let normal = generate_normal_map(&source, settings.strength.clamp(0.1, 4.0), settings.level.clamp(0.1, 4.0), settings.invert_height, settings.invert_green, &settings.convention);
  // ノーマルマップはアルファを使わない。Blender 向けの既定を RGB 24-bit にして
  // 不透明なアルファチャンネル分の容量を増やさない。
  let encoded = encode_rgb24(&normal, width, height)?;
  // ノーマルマップは画素変化が大きく、強い可逆圧縮を掛けても縮みにくい。
  // 生成速度を優先し、圧縮探索は最小限にする。
  let options = lossless_options(0);
  let optimized = oxipng::optimize_from_memory(&encoded, &options).map_err(|error| error.to_string())?;
  let output_dir = if let Some(dir) = settings.output_dir.as_deref().filter(|value| !value.trim().is_empty()) { PathBuf::from(dir) } else { path.parent().ok_or("入力元フォルダを取得できません")?.join("normal_maps") };
  fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
  let output_path = unique_named_output_path(&output_dir, path.file_stem().and_then(|value| value.to_str()).unwrap_or("image"), "normal");
  fs::write(&output_path, optimized.as_slice()).map_err(|error| error.to_string())?;
  Ok((Status::Done, Some(optimized.len() as u64), width, height, Some("ノーマルマップを生成しました".into()), Some(output_path.to_string_lossy().to_string())))
}

fn generate_normal_map(source: &image::RgbaImage, strength: f32, level: f32, invert_height: bool, invert_green: bool, convention: &str) -> image::RgbaImage {
  let (width, height) = source.dimensions();
  let mut output = image::RgbaImage::new(width, height);
  let height_values = source.as_raw().par_chunks_exact(4).map(|pixel| {
    let value = (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722) / 255.0;
    let value = if invert_height { 1.0 - value } else { value };
    ((value - 0.5) * level + 0.5).clamp(0.0, 1.0)
  }).collect::<Vec<_>>();
  let height_value = |x: i32, y: i32| -> f32 {
    let x = x.clamp(0, width.saturating_sub(1) as i32) as u32;
    let y = y.clamp(0, height.saturating_sub(1) as i32) as u32;
    height_values[(y * width + x) as usize]
  };
  output.as_mut().par_chunks_mut(width as usize * 4).enumerate().for_each(|(y, row)| {
    let y = y as i32;
    for x in 0..width as i32 {
      let tl = height_value(x - 1, y - 1); let tm = height_value(x, y - 1); let tr = height_value(x + 1, y - 1);
      let ml = height_value(x - 1, y); let mr = height_value(x + 1, y);
      let bl = height_value(x - 1, y + 1); let bm = height_value(x, y + 1); let br = height_value(x + 1, y + 1);
      let dx = (-tl + tr - 2.0 * ml + 2.0 * mr - bl + br) * strength;
      let dy = (-tl - 2.0 * tm - tr + bl + 2.0 * bm + br) * strength;
      let nx = -dx;
      let mut ny = if convention == "directx" { dy } else { -dy };
      if invert_green { ny = -ny; }
      let length = (nx * nx + ny * ny + 1.0).sqrt();
      let encode = |value: f32| ((value / length * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
      let offset = x as usize * 4;
      row[offset..offset + 4].copy_from_slice(&[encode(nx), encode(ny), encode(1.0), 255]);
    }
  });
  output
}

fn pad_height_map_to_square(source: &image::RgbaImage) -> image::RgbaImage {
  let (width, height) = source.dimensions();
  let size = width.max(height);
  if width == height { return source.clone(); }
  let mut padded = image::RgbaImage::from_pixel(size, size, image::Rgba([0, 0, 0, 255]));
  let offset_x = ((size - width) / 2) as i64;
  let offset_y = ((size - height) / 2) as i64;
  image::imageops::replace(&mut padded, source, offset_x, offset_y);
  padded
}

fn build_texture_atlas(images: &[image::RgbaImage], pad_to_square: bool, square_resolution: Option<u32>) -> Result<image::RgbaImage, String> {
  if images.len() != 4 { return Err("テクスチャアトラスには画像が4枚必要です".into()); }
  if square_resolution.is_some_and(|value| !(16..=4096).contains(&value)) { return Err("1枠の解像度は16〜4096 pxで指定してください".into()); }
  let max_width = images.iter().map(|image| image.width()).max().unwrap_or(1);
  let max_height = images.iter().map(|image| image.height()).max().unwrap_or(1);
  let (cell_width, cell_height) = if pad_to_square {
    let size = square_resolution.unwrap_or_else(|| max_width.max(max_height));
    (size, size)
  } else { (max_width, max_height) };
  let mut atlas = image::RgbaImage::from_pixel(cell_width * 2, cell_height * 2, image::Rgba([0, 0, 0, 0]));
  for (index, image) in images.iter().enumerate() {
    let fitted = fit_image_to_cell(image, cell_width, cell_height);
    let x = (index as u32 % 2) * cell_width + (cell_width - fitted.width()) / 2;
    let y = (index as u32 / 2) * cell_height + (cell_height - fitted.height()) / 2;
    image::imageops::replace(&mut atlas, &fitted, x as i64, y as i64);
  }
  Ok(atlas)
}

fn fit_image_to_cell(image: &image::RgbaImage, cell_width: u32, cell_height: u32) -> image::RgbaImage {
  if image.width() <= cell_width && image.height() <= cell_height { return image.clone(); }
  let scale = (cell_width as f64 / image.width() as f64).min(cell_height as f64 / image.height() as f64);
  let width = (image.width() as f64 * scale).round().max(1.0) as u32;
  let height = (image.height() as f64 * scale).round().max(1.0) as u32;
  image::imageops::resize(image, width, height, FilterType::Lanczos3)
}

fn resize_height_map_long_edge(source: &image::RgbaImage, max_long_edge: Option<u32>) -> image::RgbaImage {
  let (width, height) = source.dimensions();
  let Some(max_long_edge) = max_long_edge.filter(|value| *value > 0) else { return source.clone(); };
  let long_edge = width.max(height);
  if max_long_edge >= long_edge { return source.clone(); }
  let scale = max_long_edge as f64 / long_edge as f64;
  let output_width = (width as f64 * scale).round().max(1.0) as u32;
  let output_height = (height as f64 * scale).round().max(1.0) as u32;
  image::imageops::resize(source, output_width, output_height, FilterType::Lanczos3)
}

fn resize_dimensions(width: u32, height: u32, settings: &Settings) -> (u32, u32) {
  let mut scale = 1.0_f64;
  if let Some(percent) = settings.scale_percent.filter(|v| *v > 0 && *v < 100) { scale = percent as f64 / 100.0; }
  if let Some(max_width) = settings.max_width.filter(|v| *v > 0) { scale = scale.min(max_width as f64 / width as f64); }
  if let Some(max_height) = settings.max_height.filter(|v| *v > 0) { scale = scale.min(max_height as f64 / height as f64); }
  scale = scale.min(1.0);
  ((width as f64 * scale).round().max(1.0) as u32, (height as f64 * scale).round().max(1.0) as u32)
}

fn resize_filter(mode: &str) -> FilterType { if mode == "nearest" { FilterType::Nearest } else { FilterType::Lanczos3 } }

fn is_quality_too_low(error: &str) -> bool { error.contains("QUALITY_TOO_LOW") }

// "自動" は画素の値も形式も勝手に減らさない。OxiPNG の既定値には
// 可逆な形式縮小も含まれるため、ここでは圧縮し直しだけを許可する。
fn lossless_options(preset: u8) -> oxipng::Options {
  let mut options = oxipng::Options::from_preset(preset);
  options.optimize_alpha = false;
  options.bit_depth_reduction = false;
  options.color_type_reduction = false;
  options.palette_reduction = false;
  options.grayscale_reduction = false;
  options
}

fn encode_rgba(image: &image::RgbaImage, width: u32, height: u32) -> Result<Vec<u8>, String> {
  let mut output = Vec::new();
  let mut encoder = png::Encoder::new(&mut output, width, height);
  encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
  encoder.write_header().map_err(|error| error.to_string())?.write_image_data(image.as_raw()).map_err(|error| error.to_string())?;
  Ok(output)
}

fn encode_preserving_alpha(image: &image::RgbaImage, width: u32, height: u32, has_alpha: bool) -> Result<Vec<u8>, String> {
  if has_alpha { encode_rgba(image, width, height) } else { encode_rgb24(image, width, height) }
}

fn encode_rgb24(image: &image::RgbaImage, width: u32, height: u32) -> Result<Vec<u8>, String> {
  let pixels: Vec<u8> = image.pixels().flat_map(|pixel| {
    let alpha = pixel[3] as u16;
    [((pixel[0] as u16 * alpha) / 255) as u8, ((pixel[1] as u16 * alpha) / 255) as u8, ((pixel[2] as u16 * alpha) / 255) as u8]
  }).collect();
  encode_pixels(width, height, png::ColorType::Rgb, &pixels)
}

fn encode_grayscale(image: &image::RgbaImage, width: u32, height: u32) -> Result<Vec<u8>, String> {
  let pixels: Vec<u8> = image.pixels().map(|pixel| (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722).round() as u8).collect();
  encode_pixels(width, height, png::ColorType::Grayscale, &pixels)
}

fn encode_webp(image: &image::RgbaImage, width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
  let encoder = webp::Encoder::from_rgba(image.as_raw(), width, height);
  Ok(encoder.encode(quality.clamp(1, 100) as f32).to_vec())
}

fn encode_jpeg(image: &image::RgbaImage, width: u32, height: u32, quality: u8, background: &str) -> Result<Vec<u8>, String> {
  let background = if background == "black" { 0_u16 } else { 255_u16 };
  let pixels: Vec<u8> = image.pixels().flat_map(|pixel| {
    let alpha = pixel[3] as u16;
    [
      ((pixel[0] as u16 * alpha + background * (255 - alpha)) / 255) as u8,
      ((pixel[1] as u16 * alpha + background * (255 - alpha)) / 255) as u8,
      ((pixel[2] as u16 * alpha + background * (255 - alpha)) / 255) as u8,
    ]
  }).collect();
  let mut output = Vec::new();
  image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality.clamp(1, 100))
    .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
    .map_err(|error| error.to_string())?;
  Ok(output)
}

fn encode_pixels(width: u32, height: u32, color_type: png::ColorType, pixels: &[u8]) -> Result<Vec<u8>, String> {
  let mut output = Vec::new();
  let mut encoder = png::Encoder::new(&mut output, width, height);
  encoder.set_color(color_type); encoder.set_depth(png::BitDepth::Eight);
  encoder.write_header().map_err(|error| error.to_string())?.write_image_data(pixels).map_err(|error| error.to_string())?;
  Ok(output)
}

fn cached_heic_preview(path: &Path) -> Option<CachedHeicPreview> {
  let metadata = fs::metadata(path).ok()?;
  let cache = HEIC_PREVIEW_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
  cache.get(path).filter(|entry| entry.file_bytes == metadata.len() && entry.modified == metadata.modified().ok()).cloned()
}

fn remember_heic_preview(path: &Path, image: &image::DynamicImage) {
  let Ok(metadata) = fs::metadata(path) else { return; };
  let preview = image.thumbnail(512, 512).to_rgba8();
  let entry = CachedHeicPreview { file_bytes: metadata.len(), modified: metadata.modified().ok(), width: image.width(), height: image.height(), image: preview };
  let Ok(mut cache) = HEIC_PREVIEW_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() else { return; };
  if cache.len() >= 32 && !cache.contains_key(path) { cache.clear(); }
  cache.insert(path.to_path_buf(), entry);
}

fn input_dimensions(path: &Path) -> Result<(u32, u32), String> {
  if is_png(path) {
    image::image_dimensions(path).map_err(|error| format!("PNGを読み込めません: {error}"))
  } else if is_heic(path) {
    if let Some(cached) = cached_heic_preview(path) { return Ok((cached.width, cached.height)); }
    let bytes = fs::read(path).map_err(|error| format!("HEICを読み込めません: {error}"))?;
    heic_dimensions_from_bytes(&bytes)
  } else {
    Err("PNGまたはHEICファイルではありません".into())
  }
}

// 一覧追加では数千万画素を展開せず、HEIFコンテナの ispe（画像寸法）だけを読む。
// iPhoneのグリッドHEICには複数のispeがあるため、表示画像に当たる最大面積を採用する。
fn heic_dimensions_from_bytes(bytes: &[u8]) -> Result<(u32, u32), String> {
  if bytes.len() < 20 || !bytes.windows(4).take(64).any(|value| value == b"ftyp") {
    return Err("HEICのヘッダーを確認できません".into());
  }
  let mut best = None::<(u32, u32)>;
  for index in 4..bytes.len().saturating_sub(16) {
    if &bytes[index..index + 4] != b"ispe" { continue; }
    let box_size = u32::from_be_bytes(bytes[index - 4..index].try_into().unwrap_or([0; 4]));
    if box_size < 20 { continue; }
    let width = u32::from_be_bytes(bytes[index + 8..index + 12].try_into().unwrap_or([0; 4]));
    let height = u32::from_be_bytes(bytes[index + 12..index + 16].try_into().unwrap_or([0; 4]));
    let pixels = width as u64 * height as u64;
    if width == 0 || height == 0 || pixels > 256_000_000 { continue; }
    if best.is_none_or(|(best_width, best_height)| pixels > best_width as u64 * best_height as u64) {
      best = Some((width, height));
    }
  }
  best.ok_or_else(|| "HEICの画像寸法を取得できません".into())
}

fn load_input_image(path: &Path) -> Result<image::DynamicImage, String> {
  let bytes = fs::read(path).map_err(|error| format!("画像を読み込めません: {error}"))?;
  load_input_image_from_bytes(path, &bytes)
}

fn load_input_image_from_bytes(path: &Path, bytes: &[u8]) -> Result<image::DynamicImage, String> {
  if is_png(path) {
    return image::load_from_memory_with_format(bytes, image::ImageFormat::Png).map_err(|error| format!("PNGを読み込めません: {error}"));
  }
  if !is_heic(path) { return Err("PNGまたはHEICファイルではありません".into()); }
  if bytes.len() > 200 * 1024 * 1024 { return Err("200 MBを超えるHEICは読み込めません".into()); }

  let decoded = heif_oxide::decode_bytes(bytes).map_err(|error| format!("HEICを読み込めません: {error}"))?;
  let (width, height) = (decoded.width, decoded.height);
  let pixels = width as u64 * height as u64;
  if width == 0 || height == 0 || pixels > 256_000_000 { return Err("HEICの画像寸法が大きすぎます".into()); }
  let image = match decoded.pixels {
    heif_oxide::Pixels::Rgb8(data) => image::RgbImage::from_raw(width, height, data).map(image::DynamicImage::ImageRgb8),
    heif_oxide::Pixels::Rgba8(data) => image::RgbaImage::from_raw(width, height, data).map(image::DynamicImage::ImageRgba8),
    heif_oxide::Pixels::Rgb16(data) => image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(width, height, data).map(image::DynamicImage::ImageRgb16),
    heif_oxide::Pixels::Rgba16(data) => image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(width, height, data).map(image::DynamicImage::ImageRgba16),
  }.ok_or_else(|| "HEICの画素データが画像寸法と一致しません".to_string())?;
  remember_heic_preview(path, &image);
  Ok(image)
}

fn make_preview(path: &Path, size: u32) -> Option<String> {
  if !is_supported_image(path) { return None; }
  let source = if is_heic(path) {
    cached_heic_preview(path).map(|cached| image::DynamicImage::ImageRgba8(cached.image)).or_else(|| load_input_image(path).ok())?
  } else {
    load_input_image(path).ok()?
  };
  let image = source.thumbnail(size, size).to_rgba8();
  let (width, height) = image.dimensions();
  let encoded = encode_rgba(&image, width, height).ok()?;
  Some(format!("data:image/png;base64,{}", STANDARD.encode(encoded)))
}

fn collect_model_textures(root: &Path, folder: &Path, depth: usize, textures: &mut Vec<ModelTextureFile>, scan_limited: &mut bool) {
  const MAX_DEPTH: usize = 5;
  const MAX_FILES: usize = 1500;
  if depth > MAX_DEPTH || textures.len() >= MAX_FILES { *scan_limited = true; return; }
  let Ok(entries) = fs::read_dir(folder) else { return; };
  for entry in entries.flatten() {
    if textures.len() >= MAX_FILES { *scan_limited = true; return; }
    let path = entry.path();
    if path.is_dir() {
      collect_model_textures(root, &path, depth + 1, textures, scan_limited);
      continue;
    }
    if !is_model_texture(&path) { continue; }
    let Ok(metadata) = entry.metadata() else { continue; };
    let relative_path = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("texture").to_string();
    textures.push(ModelTextureFile { path: path.to_string_lossy().to_string(), relative_path, name, bytes: metadata.len() });
  }
}

fn is_model_texture(path: &Path) -> bool {
  path.extension().and_then(|value| value.to_str()).is_some_and(|value| {
    matches!(value.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" | "dds")
  })
}

fn is_preview_model(path: &Path) -> bool {
  path.extension().and_then(|value| value.to_str()).is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "fbx" | "glb"))
}

fn encode_indexed(image: &image::RgbaImage, width: u32, height: u32, colors: u32, dithering: bool) -> Result<Vec<u8>, String> {
  if !(2..=256).contains(&colors) { return Err("PNG-8 の色数は 2〜256 色で指定してください".into()); }
  let mut attributes = imagequant::new();
  attributes.set_max_colors(colors).map_err(|error| error.to_string())?;
  // PNG-8 は非可逆。細かい写真やテクスチャを不自然に減色した結果を
  // 保存しないよう、品質 90 未満の画像は処理を中止する。
  attributes.set_quality(90, 100).map_err(|error| error.to_string())?;
  attributes.set_speed(3).map_err(|error| error.to_string())?;
  let pixels: &[imagequant::RGBA] = cast_slice(image.as_raw());
  let mut input = attributes.new_image_borrowed(pixels, width as usize, height as usize, 0.0).map_err(|error| error.to_string())?;
  let mut result = attributes.quantize(&mut input).map_err(|error| error.to_string())?;
  result.set_dithering_level(if dithering { 0.8 } else { 0.0 }).map_err(|error| error.to_string())?;
  let (palette, indexes) = result.remapped(&mut input).map_err(|error| error.to_string())?;
  // PNG の PLTE は RGB 3 バイト単位。アルファは別の tRNS チャンクに記録する。
  // RGBA をそのまま PLTE に渡すと、一部の画像ビューアーで開けない PNG になる。
  let palette_bytes: Vec<u8> = palette.iter().flat_map(|color| [color.r, color.g, color.b]).collect();
  let transparency: Vec<u8> = palette.iter().map(|color| color.a).collect();
  let mut output = Vec::new();
  let mut encoder = png::Encoder::new(&mut output, width, height);
  encoder.set_color(png::ColorType::Indexed); encoder.set_depth(png::BitDepth::Eight); encoder.set_palette(palette_bytes);
  if transparency.iter().any(|alpha| *alpha < 255) { encoder.set_trns(transparency); }
  encoder.write_header().map_err(|error| error.to_string())?.write_image_data(&indexes).map_err(|error| error.to_string())?;
  Ok(output)
}

fn resolve_output_dir(path: &Path, configured: Option<&str>) -> Result<PathBuf, String> { if let Some(dir) = configured.filter(|value| !value.trim().is_empty()) { Ok(PathBuf::from(dir)) } else { Ok(path.parent().ok_or("入力元フォルダを取得できません")?.join("optimized")) } }
fn unique_output_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
  let base = dir.join(format!("{stem}-smart.{extension}"));
  if !base.exists() { return base; }
  for index in 2.. {
    let candidate = dir.join(format!("{stem}-smart-{index}.{extension}"));
    if !candidate.exists() { return candidate; }
  }
  unreachable!()
}
fn unique_named_output_path(dir: &Path, stem: &str, suffix: &str) -> PathBuf { let base = dir.join(format!("{stem}-{suffix}.png")); if !base.exists() { return base; } for index in 2.. { let candidate = dir.join(format!("{stem}-{suffix}-{index}.png")); if !candidate.exists() { return candidate; } } unreachable!() }
fn is_png(path: &Path) -> bool { path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("png")) }
fn is_heic(path: &Path) -> bool { path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "heic" | "heif")) }
fn is_supported_image(path: &Path) -> bool { is_png(path) || is_heic(path) }

fn main() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![inspect_files, create_preview, load_model_package, process_batch, process_normal_batch, process_ai_restore_batch, create_texture_atlas])
    .run(tauri::generate_context!())
    .expect("Be Asset Optimizer の起動に失敗しました");
}

#[cfg(test)]
mod tests {
  use super::*;

  fn settings() -> Settings {
    Settings { output_dir: None, output_format: "png".into(), quality: 82, jpeg_background: "white".into(), max_width: None, max_height: None, scale_percent: None, resize_mode: "lanczos3".into(), color_mode: "auto".into(), colors: None, dithering: true, optimization: "safe".into() }
  }

  #[test]
  fn resize_never_upscales() {
    let mut config = settings();
    config.max_width = Some(2000);
    config.max_height = Some(2000);
    assert_eq!(resize_dimensions(320, 200, &config), (320, 200));
    config.max_width = Some(160);
    assert_eq!(resize_dimensions(320, 200, &config), (160, 100));
  }

  #[test]
  fn indexed_encoder_writes_a_png() {
    let image = image::RgbaImage::from_raw(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0]).unwrap();
    let bytes = encode_indexed(&image, 2, 2, 16, false).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Indexed);
  }

  #[test]
  fn indexed_encoder_rejects_an_invalid_color_count() {
    let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    assert!(encode_indexed(&image, 1, 1, 0, false).is_err());
  }

  #[test]
  fn heic_fixture_decodes_with_orientation_ready_pixels() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("flat_red_64.heic");
    let image = load_input_image(&path).unwrap();
    assert_eq!(image.dimensions(), (64, 64));
    let pixel = image.to_rgb8().get_pixel(32, 32).0;
    assert!(pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80);
    assert!(make_preview(&path, 32).unwrap().starts_with("data:image/png;base64,"));
  }

  #[test]
  #[ignore = "requires SMARTPNG_HEIC_SAMPLE to point to a real camera HEIC"]
  fn real_camera_heic_decodes_and_creates_a_preview() {
    let path = PathBuf::from(std::env::var("SMARTPNG_HEIC_SAMPLE").expect("SMARTPNG_HEIC_SAMPLE is required"));
    let image = load_input_image(&path).unwrap();
    assert!(image.width() >= 512 && image.height() >= 512);
    assert!(make_preview(&path, 72).unwrap().starts_with("data:image/png;base64,"));
    if let Ok(output) = std::env::var("SMARTPNG_HEIC_PREVIEW_OUTPUT") {
      let preview = image.thumbnail(1024, 1024).to_rgba8();
      fs::write(output, encode_rgba(&preview, preview.width(), preview.height()).unwrap()).unwrap();
    }
  }

  #[test]
  #[ignore = "requires SMARTPNG_HEIC_SAMPLE to point to a real camera HEIC"]
  fn profile_real_heic_decode_and_png_optimization() {
    let path = PathBuf::from(std::env::var("SMARTPNG_HEIC_SAMPLE").expect("SMARTPNG_HEIC_SAMPLE is required"));
    let bytes = fs::read(&path).unwrap();
    let started = std::time::Instant::now();
    let source = load_input_image_from_bytes(&path, &bytes).unwrap();
    println!("decode {:?}", started.elapsed());
    let rgba = source.to_rgba8();
    let started = std::time::Instant::now();
    let encoded = encode_preserving_alpha(&rgba, rgba.width(), rgba.height(), source.color().has_alpha()).unwrap();
    println!("png encode {:?}, {} bytes", started.elapsed(), encoded.len());
    for preset in [0, 1, 3] {
      let started = std::time::Instant::now();
      let optimized = oxipng::optimize_from_memory(&encoded, &lossless_options(preset)).unwrap();
      println!("oxipng preset {preset}: {:?}, {} bytes", started.elapsed(), optimized.len());
    }
  }

  #[test]
  fn quality_limit_errors_are_detected_for_lossless_fallback() {
    assert!(is_quality_too_low("QUALITY_TOO_LOW"));
    assert!(!is_quality_too_low("VALUE_OUT_OF_RANGE"));
  }

  #[test]
  fn lossless_options_never_reduce_the_pixel_format() {
    let options = lossless_options(6);
    assert!(!options.optimize_alpha);
    assert!(!options.bit_depth_reduction);
    assert!(!options.color_type_reduction);
    assert!(!options.palette_reduction);
    assert!(!options.grayscale_reduction);
  }

  #[test]
  fn automatic_optimization_keeps_rgb_png_as_rgb() {
    let image = image::RgbaImage::from_raw(3, 1, vec![35, 20, 10, 255, 120, 86, 44, 255, 230, 220, 180, 255]).unwrap();
    let original = encode_rgb24(&image, 3, 1).unwrap();
    let optimized = oxipng::optimize_from_memory(&original, &lossless_options(6)).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(optimized));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(&buffer[..9], &[35, 20, 10, 120, 86, 44, 230, 220, 180]);
  }

  #[test]
  fn rgb24_encoder_removes_alpha_by_compositing_on_black() {
    let image = image::RgbaImage::from_raw(1, 1, vec![200, 100, 50, 128]).unwrap();
    let bytes = encode_rgb24(&image, 1, 1).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(&buffer[..3], &[100, 50, 25]);
  }

  #[test]
  fn webp_encoder_writes_a_webp_container() {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([40, 120, 220, 128]));
    let bytes = encode_webp(&image, 2, 2, 82).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
  }

  #[test]
  fn jpeg_encoder_writes_a_jpeg_and_composites_transparency() {
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
    let bytes = encode_jpeg(&image, 8, 8, 95, "white").unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    assert_eq!(&bytes[bytes.len() - 2..], &[0xff, 0xd9]);
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap().to_rgb8();
    assert!(decoded.get_pixel(4, 4).0.iter().all(|channel| *channel > 245));
  }

  #[test]
  fn a_flat_height_map_becomes_a_flat_normal_map() {
    let source = image::RgbaImage::from_pixel(3, 3, image::Rgba([120, 120, 120, 255]));
    let normal = generate_normal_map(&source, 1.0, 1.0, false, false, "opengl");
    assert_eq!(normal.get_pixel(1, 1).0, [128, 128, 255, 255]);
  }

  #[test]
  fn generated_normal_maps_use_rgb24_output() {
    let normal = image::RgbaImage::from_pixel(1, 1, image::Rgba([128, 128, 255, 255]));
    let bytes = encode_rgb24(&normal, 1, 1).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(&buffer[..3], &[128, 128, 255]);
  }

  #[test]
  fn square_padding_centers_the_height_map_on_a_black_canvas() {
    let source = image::RgbaImage::from_pixel(4, 2, image::Rgba([120, 80, 40, 255]));
    let padded = pad_height_map_to_square(&source);
    assert_eq!(padded.dimensions(), (4, 4));
    assert_eq!(padded.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(padded.get_pixel(0, 1).0, [120, 80, 40, 255]);
    assert_eq!(padded.get_pixel(3, 2).0, [120, 80, 40, 255]);
    assert_eq!(padded.get_pixel(0, 3).0, [0, 0, 0, 255]);
  }

  #[test]
  fn height_map_resize_limits_the_long_edge_without_upscaling() {
    let source = image::RgbaImage::from_pixel(1024, 800, image::Rgba([120, 80, 40, 255]));
    assert_eq!(resize_height_map_long_edge(&source, Some(512)).dimensions(), (512, 400));
    let small = image::RgbaImage::from_pixel(320, 200, image::Rgba([120, 80, 40, 255]));
    assert_eq!(resize_height_map_long_edge(&small, Some(512)).dimensions(), (320, 200));
  }

  #[test]
  fn texture_atlas_places_four_images_in_reading_order() {
    let images = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]]
      .into_iter()
      .map(|color| image::RgbaImage::from_pixel(2, 2, image::Rgba(color)))
      .collect::<Vec<_>>();
    let atlas = build_texture_atlas(&images, true, None).unwrap();
    assert_eq!(atlas.dimensions(), (4, 4));
    assert_eq!(atlas.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(atlas.get_pixel(3, 0).0, [0, 255, 0, 255]);
    assert_eq!(atlas.get_pixel(0, 3).0, [0, 0, 255, 255]);
    assert_eq!(atlas.get_pixel(3, 3).0, [255, 255, 0, 255]);
  }

  #[test]
  fn texture_atlas_can_pad_rectangular_images_to_square_cells() {
    let images = (0..4).map(|_| image::RgbaImage::from_pixel(4, 2, image::Rgba([120, 80, 40, 255]))).collect::<Vec<_>>();
    let padded = build_texture_atlas(&images, true, None).unwrap();
    assert_eq!(padded.dimensions(), (8, 8));
    assert_eq!(padded.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(padded.get_pixel(0, 1).0, [120, 80, 40, 255]);
    let rectangular = build_texture_atlas(&images, false, None).unwrap();
    assert_eq!(rectangular.dimensions(), (8, 4));
  }

  #[test]
  fn texture_atlas_uses_the_selected_square_cell_resolution() {
    let images = (0..4).map(|_| image::RgbaImage::from_pixel(32, 16, image::Rgba([120, 80, 40, 255]))).collect::<Vec<_>>();
    let atlas = build_texture_atlas(&images, true, Some(16)).unwrap();
    assert_eq!(atlas.dimensions(), (32, 32));
    assert_eq!(atlas.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(atlas.get_pixel(0, 4).0, [120, 80, 40, 255]);
    assert!(build_texture_atlas(&images, true, Some(8)).is_err());
  }

  #[test]
  fn processing_resizes_and_writes_to_the_selected_folder() {
    let root = std::env::temp_dir().join(format!("smartpng-test-{}", std::process::id()));
    let input_dir = root.join("input");
    let output_dir = root.join("output");
    fs::create_dir_all(&input_dir).unwrap();
    let input = input_dir.join("sample.png");
    let pixels = (0..(64 * 64)).flat_map(|index| [index as u8, (index / 3) as u8, 180, 255]).collect();
    let image = image::RgbaImage::from_raw(64, 64, pixels).unwrap();
    fs::write(&input, encode_rgba(&image, 64, 64).unwrap()).unwrap();
    let mut config = settings();
    config.max_width = Some(16);
    config.output_dir = Some(output_dir.to_string_lossy().to_string());
    let result = process_file(&input, &config).unwrap();
    assert!(matches!(result.0, Status::Done));
    assert_eq!((result.2, result.3), (16, 16));
    assert!(result.5.unwrap().ends_with("sample-smart.png"));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn processing_can_write_webp_and_jpeg_outputs() {
    let root = std::env::temp_dir().join(format!("smartpng-format-test-{}", std::process::id()));
    let input_dir = root.join("input");
    let output_dir = root.join("output");
    fs::create_dir_all(&input_dir).unwrap();
    let input = input_dir.join("sample.png");
    let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([40, 120, 220, 128]));
    fs::write(&input, encode_rgba(&image, 16, 16).unwrap()).unwrap();

    let mut webp_config = settings();
    webp_config.output_dir = Some(output_dir.to_string_lossy().to_string());
    webp_config.output_format = "webp".into();
    let webp_result = process_file(&input, &webp_config).unwrap();
    let webp_path = webp_result.5.unwrap();
    assert!(webp_path.ends_with("sample-smart.webp"));
    assert_eq!(&fs::read(webp_path).unwrap()[..4], b"RIFF");

    let mut jpeg_config = settings();
    jpeg_config.output_dir = Some(output_dir.to_string_lossy().to_string());
    jpeg_config.output_format = "jpeg".into();
    let jpeg_result = process_file(&input, &jpeg_config).unwrap();
    let jpeg_path = jpeg_result.5.unwrap();
    assert!(jpeg_path.ends_with("sample-smart.jpg"));
    assert_eq!(&fs::read(jpeg_path).unwrap()[..2], &[0xff, 0xd8]);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn processing_converts_heic_to_a_valid_png() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("smartpng-heic-test-{}-{id}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("flat_red_64.heic");
    let inspected = inspect_files_impl(vec![input.to_string_lossy().to_string()], 500, |_, _, _, _, _| {}).unwrap();
    assert_eq!((inspected.files[0].width, inspected.files[0].height), (64, 64));
    let mut config = settings();
    config.output_dir = Some(root.to_string_lossy().to_string());
    let result = process_file(&input, &config).unwrap();
    assert!(matches!(result.0, Status::Done));
    assert_eq!((result.2, result.3), (64, 64));
    let output = result.5.unwrap();
    assert!(output.ends_with("flat_red_64-smart.png"));
    assert_eq!(image::image_dimensions(output).unwrap(), (64, 64));

    let normal_settings = NormalSettings { output_dir: Some(root.to_string_lossy().to_string()), max_long_edge: None, strength: 1.0, level: 1.0, convention: "opengl".into(), invert_height: false, invert_green: false, pad_to_square: false };
    let normal = process_normal_file(&input, &normal_settings).unwrap();
    assert_eq!((normal.2, normal.3), (64, 64));
    assert_eq!(image::image_dimensions(normal.5.unwrap()).unwrap(), (64, 64));

    let texture = load_input_image(&input).unwrap().to_rgba8();
    let atlas = build_texture_atlas(&[texture.clone(), texture.clone(), texture.clone(), texture], true, None).unwrap();
    assert_eq!(atlas.dimensions(), (128, 128));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn inspection_reads_dropped_folders_and_keeps_valid_images_when_one_fails() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("smartpng-inspect-test-{}-{id}", std::process::id()));
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let png = root.join("sample.png");
    let image = image::RgbaImage::from_pixel(8, 6, image::Rgba([40, 120, 220, 255]));
    fs::write(&png, encode_rgba(&image, 8, 6).unwrap()).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("flat_red_64.heic");
    fs::copy(fixture, nested.join("sample.heic")).unwrap();
    fs::write(nested.join("broken.heic"), b"not a heic image").unwrap();
    fs::write(root.join("ignored.txt"), b"ignored").unwrap();

    let mut progress = Vec::new();
    let inspected = inspect_files_impl(vec![root.to_string_lossy().to_string()], 500, |completed, total, _, _, _| progress.push((completed, total))).unwrap();
    assert_eq!(inspected.files.len(), 2);
    assert_eq!(inspected.failures.len(), 1);
    assert_eq!(progress.last(), Some(&(3, 3)));
    assert!(inspected.files.iter().any(|file| (file.width, file.height) == (8, 6)));
    assert!(inspected.files.iter().any(|file| (file.width, file.height) == (64, 64)));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  #[ignore = "requires SMARTPNG_HEIC_DIR to point to a folder of real HEIC images"]
  fn real_heic_folder_can_be_inspected_as_a_batch() {
    let root = PathBuf::from(std::env::var("SMARTPNG_HEIC_DIR").expect("SMARTPNG_HEIC_DIR is required"));
    let inspect_started = std::time::Instant::now();
    let inspected = inspect_files_impl(vec![root.to_string_lossy().to_string()], 500, |completed, total, path, _, _| {
      println!("{completed}/{total}: {}", path.display());
    }).unwrap();
    println!("metadata inspection: {:?}", inspect_started.elapsed());
    for failure in &inspected.failures { println!("failed: {}: {}", failure.path, failure.message); }
    assert!(!inspected.files.is_empty());
    let decode_started = std::time::Instant::now();
    for file in &inspected.files {
      let decoded = load_input_image(Path::new(&file.path)).unwrap();
      assert_eq!((file.width, file.height), decoded.dimensions(), "metadata dimensions differ for {}", file.path);
    }
    println!("full decode validation: {:?}", decode_started.elapsed());
    println!("loaded={}, failed={}", inspected.files.len(), inspected.failures.len());
  }

  #[test]
  #[ignore = "requires a Vulkan-capable GPU and the bundled Real-ESRGAN runtime"]
  fn ai_restore_runtime_writes_valid_fast_and_original_size_pngs() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("smartpng-ai-smoke-{}-{id}", std::process::id()));
    let output_dir = root.join("output");
    fs::create_dir_all(&root).unwrap();
    let input = root.join("sample.png");
    let pixels = (0..(24 * 24)).flat_map(|index| [index as u8, (index / 2) as u8, 180, 255]).collect();
    let image = image::RgbaImage::from_raw(24, 24, pixels).unwrap();
    fs::write(&input, encode_rgba(&image, 24, 24).unwrap()).unwrap();
    let working_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("realesrgan");
    let runtime = RealEsrganRuntime { executable: working_dir.join("realesrgan-ncnn-vulkan.exe"), working_dir };
    let settings = AiRestoreSettings { output_dir: Some(output_dir.to_string_lossy().to_string()), output_scale: 2, tile_size: Some(128) };
    let result = process_ai_restore_file(&input, &settings, &runtime).unwrap();
    assert!(matches!(result.0, Status::Done));
    assert_eq!((result.2, result.3), (48, 48));
    assert_eq!(image::image_dimensions(result.5.unwrap()).unwrap(), (48, 48));

    let settings = AiRestoreSettings { output_dir: Some(output_dir.to_string_lossy().to_string()), output_scale: 1, tile_size: Some(128) };
    let result = process_ai_restore_file(&input, &settings, &runtime).unwrap();
    assert!(matches!(result.0, Status::Done));
    assert_eq!((result.2, result.3), (24, 24));
    assert_eq!(image::image_dimensions(result.5.unwrap()).unwrap(), (24, 24));

    let heic = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("flat_red_64.heic");
    let settings = AiRestoreSettings { output_dir: Some(output_dir.to_string_lossy().to_string()), output_scale: 2, tile_size: Some(128) };
    let result = process_ai_restore_file(&heic, &settings, &runtime).unwrap();
    assert!(matches!(result.0, Status::Done));
    assert_eq!((result.2, result.3), (128, 128));
    assert_eq!(image::image_dimensions(result.5.unwrap()).unwrap(), (128, 128));
    fs::remove_dir_all(root).unwrap();
  }
}
