#![windows_subsystem = "windows"]

use std::{fs, path::{Path, PathBuf}};
use bytemuck::cast_slice;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{imageops::FilterType, GenericImageView};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileMetadata { path: String, name: String, original_bytes: u64, width: u32, height: u32 }

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum Status { Processing, Done, Skipped, Failed }

#[derive(Debug, Serialize, Clone)]
struct Progress { path: String, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings { output_dir: Option<String>, max_width: Option<u32>, max_height: Option<u32>, scale_percent: Option<u32>, resize_mode: String, color_mode: String, colors: Option<u32>, dithering: bool, optimization: String }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NormalSettings { output_dir: Option<String>, strength: f32, level: f32, convention: String, invert_height: bool, invert_green: bool }

#[tauri::command]
fn inspect_files(paths: Vec<String>) -> Result<Vec<FileMetadata>, String> {
  paths.into_iter().filter_map(|path| {
    let path = PathBuf::from(path);
    if !is_png(&path) { return None; }
    let metadata = fs::metadata(&path).ok()?;
    let (width, height) = image::image_dimensions(&path).ok()?;
    Some(FileMetadata { path: path.to_string_lossy().to_string(), name: path.file_name()?.to_string_lossy().to_string(), original_bytes: metadata.len(), width, height })
  }).collect::<Vec<_>>().pipe(Ok)
}

#[tauri::command]
async fn create_preview(path: String, size: u32) -> Result<String, String> {
  tauri::async_runtime::spawn_blocking(move || make_preview(&PathBuf::from(path), size.clamp(32, 512)).ok_or("プレビューを生成できませんでした".into()))
    .await
    .map_err(|error| error.to_string())?
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

fn emit(app: &AppHandle, path: &str, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String>) {
  let _ = app.emit("job-progress", Progress { path: path.into(), status, output_bytes, output_width, output_height, message, output_path });
}

fn emit_normal(app: &AppHandle, path: &str, status: Status, output_bytes: Option<u64>, output_width: Option<u32>, output_height: Option<u32>, message: Option<String>, output_path: Option<String>) {
  let _ = app.emit("normal-job-progress", Progress { path: path.into(), status, output_bytes, output_width, output_height, message, output_path });
}

fn process_file(path: &Path, settings: &Settings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_png(path) { return Err("PNG ファイルではありません".into()); }
  let original = fs::read(path).map_err(|error| error.to_string())?;
  if original.windows(4).any(|chunk| chunk == b"acTL") { return Err("APNG は MVP では未対応です".into()); }
  let source = image::load_from_memory_with_format(&original, image::ImageFormat::Png).map_err(|error| error.to_string())?;
  let (source_width, source_height) = source.dimensions();
  let (width, height) = resize_dimensions(source_width, source_height, settings);
  let changed_size = (width, height) != (source_width, source_height);
  let rgba = if changed_size { image::imageops::resize(&source.to_rgba8(), width, height, resize_filter(&settings.resize_mode)) } else { source.to_rgba8() };

  let encoded = match settings.color_mode.as_str() {
    "indexed" => encode_indexed(&rgba, width, height, settings.colors.unwrap_or(256), settings.dithering)?,
    "rgb24" => encode_rgb24(&rgba, width, height)?,
    "rgba32" => encode_rgba(&rgba, width, height)?,
    "grayscale8" => encode_grayscale(&rgba, width, height)?,
    _ if changed_size => encode_rgba(&rgba, width, height)?,
    _ => original.clone(),
  };

  let options = lossless_options(if settings.optimization == "max" { 6 } else { 3 });
  let optimized = oxipng::optimize_from_memory(&encoded, &options).map_err(|error| error.to_string())?;
  if optimized.len() as u64 >= original.len() as u64 {
    return Ok((Status::Skipped, None, source_width, source_height, Some("元ファイルより小さくなりませんでした".into()), None));
  }
  let output_dir = resolve_output_dir(path, settings.output_dir.as_deref())?;
  fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
  let output_path = unique_output_path(&output_dir, path.file_stem().and_then(|v| v.to_str()).unwrap_or("image"));
  fs::write(&output_path, optimized.as_slice()).map_err(|error| error.to_string())?;
  Ok((Status::Done, Some(optimized.len() as u64), width, height, Some("最適化しました".into()), Some(output_path.to_string_lossy().to_string())))
}

fn process_normal_file(path: &Path, settings: &NormalSettings) -> Result<(Status, Option<u64>, u32, u32, Option<String>, Option<String>), String> {
  if !is_png(path) { return Err("PNG ファイルではありません".into()); }
  let original = fs::read(path).map_err(|error| error.to_string())?;
  if original.windows(4).any(|chunk| chunk == b"acTL") { return Err("APNG は MVP では未対応です".into()); }
  let source = image::load_from_memory_with_format(&original, image::ImageFormat::Png).map_err(|error| error.to_string())?.to_rgba8();
  let (width, height) = source.dimensions();
  let normal = generate_normal_map(&source, settings.strength.clamp(0.1, 4.0), settings.level.clamp(0.1, 4.0), settings.invert_height, settings.invert_green, &settings.convention);
  let encoded = encode_rgba(&normal, width, height)?;
  let options = lossless_options(3);
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
  let height_value = |x: i32, y: i32| -> f32 {
    let x = x.clamp(0, width.saturating_sub(1) as i32) as u32;
    let y = y.clamp(0, height.saturating_sub(1) as i32) as u32;
    let pixel = source.get_pixel(x, y).0;
    let value = (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722) / 255.0;
    let value = if invert_height { 1.0 - value } else { value };
    ((value - 0.5) * level + 0.5).clamp(0.0, 1.0)
  };
  for y in 0..height as i32 {
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
      output.put_pixel(x as u32, y as u32, image::Rgba([encode(nx), encode(ny), encode(1.0), 255]));
    }
  }
  output
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

fn encode_pixels(width: u32, height: u32, color_type: png::ColorType, pixels: &[u8]) -> Result<Vec<u8>, String> {
  let mut output = Vec::new();
  let mut encoder = png::Encoder::new(&mut output, width, height);
  encoder.set_color(color_type); encoder.set_depth(png::BitDepth::Eight);
  encoder.write_header().map_err(|error| error.to_string())?.write_image_data(pixels).map_err(|error| error.to_string())?;
  Ok(output)
}

fn make_preview(path: &Path, size: u32) -> Option<String> {
  if !is_png(path) { return None; }
  let image = image::open(path).ok()?.thumbnail(size, size).to_rgba8();
  let (width, height) = image.dimensions();
  let encoded = encode_rgba(&image, width, height).ok()?;
  Some(format!("data:image/png;base64,{}", STANDARD.encode(encoded)))
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
fn unique_output_path(dir: &Path, stem: &str) -> PathBuf { unique_named_output_path(dir, stem, "smart") }
fn unique_named_output_path(dir: &Path, stem: &str, suffix: &str) -> PathBuf { let base = dir.join(format!("{stem}-{suffix}.png")); if !base.exists() { return base; } for index in 2.. { let candidate = dir.join(format!("{stem}-{suffix}-{index}.png")); if !candidate.exists() { return candidate; } } unreachable!() }
fn is_png(path: &Path) -> bool { path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("png")) }

trait Pipe: Sized { fn pipe<T, F: FnOnce(Self) -> T>(self, f: F) -> T { f(self) } }
impl<T> Pipe for T {}

fn main() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![inspect_files, create_preview, process_batch, process_normal_batch])
    .run(tauri::generate_context!())
    .expect("SmartPNG の起動に失敗しました");
}

#[cfg(test)]
mod tests {
  use super::*;

  fn settings() -> Settings {
    Settings { output_dir: None, max_width: None, max_height: None, scale_percent: None, resize_mode: "lanczos3".into(), color_mode: "auto".into(), colors: None, dithering: true, optimization: "safe".into() }
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
  fn a_flat_height_map_becomes_a_flat_normal_map() {
    let source = image::RgbaImage::from_pixel(3, 3, image::Rgba([120, 120, 120, 255]));
    let normal = generate_normal_map(&source, 1.0, 1.0, false, false, "opengl");
    assert_eq!(normal.get_pixel(1, 1).0, [128, 128, 255, 255]);
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
}
