pub mod cli;
mod atlas;
mod texture_tools;
use atlas::AtlasSlot;

use std::{
  collections::{HashMap, HashSet},
  fs,
  path::{Path, PathBuf},
  sync::{Condvar, Mutex, OnceLock},
  time::SystemTime,
};
#[cfg(test)]
use std::time::UNIX_EPOCH;
use bytemuck::cast_slice;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{imageops::FilterType, GenericImageView, ImageDecoder, ImageEncoder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{
  menu::{AboutMetadataBuilder, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
  AppHandle, Emitter, Manager,
};

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
struct AtlasSettings { output_dir: Option<String>, pad_to_square: bool, square_resolution: Option<u32>, #[serde(default)] padding: u32 }

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
static PREVIEW_SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
struct PreviewPermit;
impl PreviewPermit {
  fn acquire() -> Option<Self> {
    let mut active = PREVIEW_SLOTS.0.lock().ok()?;
    while *active >= 2 { active = PREVIEW_SLOTS.1.wait(active).ok()?; }
    *active += 1;
    Some(Self)
  }
}
impl Drop for PreviewPermit {
  fn drop(&mut self) {
    if let Ok(mut active) = PREVIEW_SLOTS.0.lock() {
      *active = active.saturating_sub(1);
      PREVIEW_SLOTS.1.notify_one();
    }
  }
}

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
async fn create_texture_atlas(slots: Vec<AtlasSlot>, settings: AtlasSettings) -> Result<AtlasResult, String> {
  tauri::async_runtime::spawn_blocking(move || {
    atlas::validate(&slots)?;
    let mut images = Vec::with_capacity(slots.len());
    for slot in &slots {
      images.push(if let Some(path) = &slot.path {
        let path = PathBuf::from(path);
        if !is_supported_image(&path) { return Err("対応している画像ファイルを指定してください".into()); }
        Some(load_input_image(&path)?.to_rgba8())
      } else { None });
    }
    let atlas = atlas::render_with_padding(&slots, &images, settings.pad_to_square, settings.square_resolution, settings.padding)?;
    let (width, height) = atlas.dimensions();
    let encoded = encode_rgba(&atlas, width, height)?;
    let optimized = oxipng::optimize_from_memory(&encoded, &lossless_options(1)).map_err(|error| error.to_string())?;
    let first_path = PathBuf::from(slots.iter().find_map(|slot| slot.path.as_ref()).ok_or("画像を1枚以上指定してください")?);
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

fn process_file(path: &Path, settings: &Settings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_supported_image(path) { return Err("対応している画像ファイルではありません".into()); }
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
    "webp" => (encode_webp(&rgba, width, height, quality)?, "webp", "WebPに変換しました".to_string(), false),
    "jpeg" | "jpg" => (encode_jpeg(&rgba, width, height, quality, &settings.jpeg_background)?, "jpg", "JPEGに変換しました".to_string(), false),
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
      let message = if fell_back_to_lossless {
        "PNG-8では品質が保てないため、可逆PNGで保存しました".to_string()
      } else if is_png(path) {
        "PNGを最適化しました".to_string()
      } else {
        format!("{}をPNGに変換しました", input_format_name(path))
      };
      let compression_only = !changed_size && (settings.color_mode == "auto" || fell_back_to_lossless);
      (optimized, "png", message, compression_only)
    }
  };
  if is_png(path) && skip_when_larger && optimized.len() as u64 >= original.len() as u64 {
    return Ok((Status::Skipped, None, source_width, source_height, Some("元ファイルより小さくなりませんでした".into()), None));
  }
  let output_dir = resolve_output_dir(path, settings.output_dir.as_deref())?;
  fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
  let output_path = unique_output_path(&output_dir, path.file_stem().and_then(|v| v.to_str()).unwrap_or("image"), extension);
  fs::write(&output_path, optimized.as_slice()).map_err(|error| error.to_string())?;
  Ok((Status::Done, Some(optimized.len() as u64), width, height, Some(message), Some(output_path.to_string_lossy().to_string())))
}

fn process_normal_file(path: &Path, settings: &NormalSettings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_supported_image(path) { return Err("対応している画像ファイルではありません".into()); }
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

#[cfg(test)]
fn build_texture_atlas(images: &[image::RgbaImage], pad_to_square: bool, square_resolution: Option<u32>) -> Result<image::RgbaImage, String> {
  let slots: Vec<_> = (0..4).map(|i| AtlasSlot { path: None, x: i % 2 * 2, y: i / 2 * 2, size: 2 }).collect();
  atlas::render(&slots, &images.iter().cloned().map(Some).collect::<Vec<_>>(), pad_to_square, square_resolution)
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
  if is_heic(path) {
    if let Some(cached) = cached_heic_preview(path) { return Ok((cached.width, cached.height)); }
    let bytes = fs::read(path).map_err(|error| format!("HEICを読み込めません: {error}"))?;
    heic_dimensions_from_bytes(&bytes)
  } else if standard_image_format(path).is_some() {
    let format = standard_image_format(path).unwrap();
    let mut reader = image::ImageReader::open(path).map_err(|error| format!("{}を読み込めません: {error}", input_format_name(path)))?;
    reader.set_format(format);
    let mut decoder = reader.into_decoder().map_err(|error| format!("{}を読み込めません: {error}", input_format_name(path)))?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    Ok(match orientation {
      image::metadata::Orientation::Rotate90 | image::metadata::Orientation::Rotate270 | image::metadata::Orientation::Rotate90FlipH | image::metadata::Orientation::Rotate270FlipH => (height, width),
      _ => (width, height),
    })
  } else {
    Err("対応している画像ファイルではありません".into())
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
  if let Some(format) = standard_image_format(path) {
    let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut decoder = reader.into_decoder().map_err(|error| format!("{}を読み込めません: {error}", input_format_name(path)))?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|error| format!("{}を読み込めません: {error}", input_format_name(path)))?;
    decoded.apply_orientation(orientation);
    return Ok(decoded);
  }
  if !is_heic(path) { return Err("対応している画像ファイルではありません".into()); }
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
  let _permit = PreviewPermit::acquire()?;
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
fn standard_image_format(path: &Path) -> Option<image::ImageFormat> {
  match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
    "png" => Some(image::ImageFormat::Png),
    "jpg" | "jpeg" | "jfif" => Some(image::ImageFormat::Jpeg),
    "webp" => Some(image::ImageFormat::WebP),
    "bmp" => Some(image::ImageFormat::Bmp),
    "tif" | "tiff" => Some(image::ImageFormat::Tiff),
    "gif" => Some(image::ImageFormat::Gif),
    "tga" => Some(image::ImageFormat::Tga),
    "dds" => Some(image::ImageFormat::Dds),
    _ => None,
  }
}
fn input_format_name(path: &Path) -> &'static str {
  if is_heic(path) { return "HEIC"; }
  match standard_image_format(path) {
    Some(image::ImageFormat::Png) => "PNG",
    Some(image::ImageFormat::Jpeg) => "JPEG",
    Some(image::ImageFormat::WebP) => "WebP",
    Some(image::ImageFormat::Bmp) => "BMP",
    Some(image::ImageFormat::Tiff) => "TIFF",
    Some(image::ImageFormat::Gif) => "GIF",
    Some(image::ImageFormat::Tga) => "TGA",
    Some(image::ImageFormat::Dds) => "DDS",
    _ => "画像",
  }
}
fn is_supported_image(path: &Path) -> bool { is_heic(path) || standard_image_format(path).is_some() }

fn build_app_menu<R: tauri::Runtime>(app: &AppHandle<R>, japanese: bool) -> tauri::Result<tauri::menu::Menu<R>> {
  let text = |ja: &'static str, en: &'static str| if japanese { ja } else { en };
  let open = MenuItemBuilder::with_id("file-open", text("ファイルを開く…", "Open Files…")).accelerator("Ctrl+O").build(app)?;
  let file = SubmenuBuilder::new(app, text("ファイル(&F)", "File (&F)"))
    .item(&open)
    .separator()
    .quit_with_text(text("終了", "Exit"))
    .build()?;
  let tools = SubmenuBuilder::new(app, text("機能(&T)", "Tools (&T)"))
    .text("tab-optimize", text("画像最適化", "Image Optimization"))
    .text("tab-normal", text("ノーマルマップ", "Normal Map"))
    .text("tab-atlas", text("テクスチャアトラス", "Texture Atlas"))
    .text("tab-preview", text("3Dプレビュー", "3D Preview"))
    .text("tab-material", text("PBRプレビュー", "PBR Preview"))
    .text("tab-channels", text("チャンネル合成・分離", "Channel Packing / Split"))
    .text("tab-adjust", text("ノーマル調整", "Normal Adjustment"))
    .build()?;
  let view = SubmenuBuilder::new(app, text("表示(&V)", "View (&V)"))
    .text("theme-light", text("ライトテーマ", "Light Theme"))
    .text("theme-dark", text("ダークテーマ", "Dark Theme"))
    .separator()
    .text("scale-80", text("表示倍率 80%", "Display Scale 80%"))
    .text("scale-90", text("表示倍率 90%", "Display Scale 90%"))
    .text("scale-100", text("表示倍率 100%", "Display Scale 100%"))
    .text("scale-110", text("表示倍率 110%", "Display Scale 110%"))
    .text("scale-125", text("表示倍率 125%", "Display Scale 125%"))
    .build()?;
  let japanese_item = CheckMenuItemBuilder::with_id("language-ja", "日本語")
    .checked(japanese).build(app)?;
  let english_item = CheckMenuItemBuilder::with_id("language-en", "English")
    .checked(!japanese).build(app)?;
  let language = SubmenuBuilder::new(app, text("言語(&L)", "Language (&L)"))
    .items(&[&japanese_item, &english_item]).build()?;
  let about = AboutMetadataBuilder::new()
    .name(Some("Be Asset Optimizer"))
    .version(Some(env!("CARGO_PKG_VERSION")))
    .comments(Some(text("Blender向けの画像・3Dアセット最適化ツール", "Image and 3D asset optimization for Blender")))
    .license(Some("GPL-3.0-or-later"))
    .website(Some("https://github.com/studio-juh/be-asset-optimizer"))
    .build();
  let help = SubmenuBuilder::new(app, text("ヘルプ(&H)", "Help (&H)"))
    .about_with_text(text("バージョン情報", "About Be Asset Optimizer"), Some(about))
    .build()?;
  MenuBuilder::new(app).items(&[&file, &tools, &view, &language, &help]).build()
}

#[tauri::command]
fn set_ui_locale(app: AppHandle, locale: String) -> Result<(), String> {
  let menu = build_app_menu(&app, locale.eq_ignore_ascii_case("ja")).map_err(|error| error.to_string())?;
  app.set_menu(menu).map_err(|error| error.to_string())?;
  Ok(())
}

pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .menu(|app| build_app_menu(app, false))
    .on_menu_event(|app, event| {
      let _ = app.emit("app-menu-action", event.id().as_ref());
    })
    .invoke_handler(tauri::generate_handler![set_ui_locale, inspect_files, create_preview, load_model_package, process_batch, process_normal_batch, create_texture_atlas, texture_tools::pack_channels, texture_tools::split_channels, texture_tools::adjust_normal, texture_tools::diagnose_textures])
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
  fn common_image_extensions_are_supported() {
    for extension in ["png", "jpg", "jpeg", "jfif", "webp", "heic", "heif", "bmp", "tif", "tiff", "gif", "tga", "dds"] {
      assert!(is_supported_image(Path::new(&format!("sample.{extension}"))), "{extension} should be supported");
    }
    assert!(!is_supported_image(Path::new("sample.svg")));
  }

  #[test]
  fn common_static_formats_decode_and_create_previews() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("be-asset-image-formats-{}-{id}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(12, 8, image::Rgba([40, 120, 220, 255])));
    let output_dir = root.join("output");
    let mut config = settings();
    config.output_dir = Some(output_dir.to_string_lossy().to_string());
    for (extension, format) in [
      ("png", image::ImageFormat::Png),
      ("jpg", image::ImageFormat::Jpeg),
      ("webp", image::ImageFormat::WebP),
      ("bmp", image::ImageFormat::Bmp),
      ("tiff", image::ImageFormat::Tiff),
      ("gif", image::ImageFormat::Gif),
      ("tga", image::ImageFormat::Tga),
    ] {
      let path = root.join(format!("sample.{extension}"));
      let mut encoded = std::io::Cursor::new(Vec::new());
      source.write_to(&mut encoded, format).unwrap();
      fs::write(&path, encoded.into_inner()).unwrap();
      assert_eq!(input_dimensions(&path).unwrap(), (12, 8), "{extension} dimensions");
      assert_eq!(load_input_image(&path).unwrap().dimensions(), (12, 8), "{extension} decode");
      assert!(make_preview(&path, 32).unwrap().starts_with("data:image/png;base64,"), "{extension} preview");
      let converted = process_file(&path, &config).unwrap();
      assert_eq!((converted.2, converted.3), (12, 8), "{extension} conversion dimensions");
      if extension != "png" {
        assert!(matches!(converted.0, Status::Done), "{extension} conversion status");
        assert!(Path::new(converted.5.as_deref().unwrap()).is_file(), "{extension} converted output");
      }
    }
    fs::remove_dir_all(root).unwrap();
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
  fn explicit_png_format_and_resize_are_saved_even_when_larger() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("be-png-transform-{}-{id}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("tiny.png");
    let source = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
    let original = oxipng::optimize_from_memory(&encode_rgb24(&source, 2, 1).unwrap(), &lossless_options(6)).unwrap();
    fs::write(&input, &original).unwrap();
    let mut config = settings();
    config.output_dir = Some(root.join("out").to_string_lossy().into());
    config.color_mode = "rgba32".into();
    let converted = process_file(&input, &config).unwrap();
    assert!(matches!(converted.0, Status::Done));
    let bytes = fs::read(converted.5.unwrap()).unwrap();
    let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
    config.color_mode = "auto".into();
    config.max_width = Some(1);
    let resized = process_file(&input, &config).unwrap();
    assert!(matches!(resized.0, Status::Done));
    assert_eq!(image::image_dimensions(resized.5.unwrap()).unwrap(), (1, 1));
    config.max_width = None;
    assert!(matches!(process_file(&input, &config).unwrap().0, Status::Skipped));
    assert_eq!(fs::read(&input).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn preview_decode_concurrency_is_bounded() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let active = AtomicUsize::new(0);
    let maximum = AtomicUsize::new(0);
    std::thread::scope(|scope| {
      for _ in 0..8 {
        scope.spawn(|| {
          let _permit = PreviewPermit::acquire().unwrap();
          let count = active.fetch_add(1, Ordering::SeqCst) + 1;
          maximum.fetch_max(count, Ordering::SeqCst);
          for _ in 0..100 { std::thread::yield_now(); }
          active.fetch_sub(1, Ordering::SeqCst);
        });
      }
    });
    assert!(maximum.load(Ordering::SeqCst) <= 2);
    assert_eq!(active.load(Ordering::SeqCst), 0);
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

}
