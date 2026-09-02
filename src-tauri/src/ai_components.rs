use std::{
  fs::{self, File},
  io::{Read, Write},
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
  },
  thread,
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use reqwest::{
  header::{CONTENT_RANGE, RANGE},
  StatusCode,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use zip::ZipArchive;

const COMPONENT_BYTES: u64 = 73_425_210;
const DOWNLOAD_BYTES: u64 = 64_116_338;
const USER_AGENT: &str = "Be-Asset-Optimizer/0.5";
const PARALLEL_DOWNLOADS: usize = 4;
const PARALLEL_DOWNLOAD_MIN_BYTES: u64 = 8 * 1024 * 1024;
const DOWNLOAD_PROGRESS_STEP: u64 = 512 * 1024;

struct ComponentFile {
  archive_name: &'static str,
  relative_path: &'static str,
  sha256: &'static str,
}

struct ComponentArchive {
  label: &'static str,
  url: &'static str,
  archive_sha256: Option<&'static str>,
  files: &'static [ComponentFile],
}

const RUNTIME_FILES: &[ComponentFile] = &[
  ComponentFile {
    archive_name: "realesrgan-ncnn-vulkan.exe",
    relative_path: "realesrgan-ncnn-vulkan.exe",
    sha256: "07E49F7CBB4EDE01AE4DD4C399D3A7E5846E3D2085C3128EFF881E55CB7B1A0C",
  },
  ComponentFile {
    archive_name: "vcomp140.dll",
    relative_path: "vcomp140.dll",
    sha256: "8F72EF2E483465444B2059FC6744D6CB22CD8D8A27F6FA56BEFD2A42DCD0F78B",
  },
  ComponentFile {
    archive_name: "realesrgan-x4plus.bin",
    relative_path: "models/realesrgan-x4plus.bin",
    sha256: "713EE713B0353AFAA27976F0563A64A5043BD70B9BD8936C2E26E25EBCDBCDDF",
  },
  ComponentFile {
    archive_name: "realesrgan-x4plus.param",
    relative_path: "models/realesrgan-x4plus.param",
    sha256: "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86",
  },
];

const NATURAL_MODEL_FILES: &[ComponentFile] = &[
  ComponentFile {
    archive_name: "realesrnet-x4plus.bin",
    relative_path: "models/realesrnet-x4plus.bin",
    sha256: "26BCCFCC82D9E8260C0C6B0DFFB34AB297982740882D1F33C6D423F70B562C40",
  },
  ComponentFile {
    archive_name: "realesrnet-x4plus.param",
    relative_path: "models/realesrnet-x4plus.param",
    sha256: "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86",
  },
];

const RELEASE_FILES: &[ComponentFile] = &[
  ComponentFile {
    archive_name: "realesrgan-ncnn-vulkan.exe",
    relative_path: "realesrgan-ncnn-vulkan.exe",
    sha256: "07E49F7CBB4EDE01AE4DD4C399D3A7E5846E3D2085C3128EFF881E55CB7B1A0C",
  },
  ComponentFile {
    archive_name: "vcomp140.dll",
    relative_path: "vcomp140.dll",
    sha256: "8F72EF2E483465444B2059FC6744D6CB22CD8D8A27F6FA56BEFD2A42DCD0F78B",
  },
  ComponentFile {
    archive_name: "realesrgan-x4plus.bin",
    relative_path: "models/realesrgan-x4plus.bin",
    sha256: "713EE713B0353AFAA27976F0563A64A5043BD70B9BD8936C2E26E25EBCDBCDDF",
  },
  ComponentFile {
    archive_name: "realesrgan-x4plus.param",
    relative_path: "models/realesrgan-x4plus.param",
    sha256: "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86",
  },
  ComponentFile {
    archive_name: "realesrnet-x4plus.bin",
    relative_path: "models/realesrnet-x4plus.bin",
    sha256: "26BCCFCC82D9E8260C0C6B0DFFB34AB297982740882D1F33C6D423F70B562C40",
  },
  ComponentFile {
    archive_name: "realesrnet-x4plus.param",
    relative_path: "models/realesrnet-x4plus.param",
    sha256: "35330ECECCEA33B6C397A72548E788D5D53BECEE4734C50B7FADA36E89F10A86",
  },
];

const PRIMARY_ARCHIVES: &[ComponentArchive] = &[ComponentArchive {
  label: "AIコンポーネント",
  url: "https://github.com/studio-juh/be-asset-optimizer/releases/download/ai-components-v1.0.0/be-asset-optimizer-ai-components-1.0.0-windows-x64.zip",
  archive_sha256: Some("A5F54E81994B194DD5DEAD69DF6835035001D6A583AEC04EF8C18AC25DEAE5C7"),
  files: RELEASE_FILES,
}];

const FALLBACK_ARCHIVES: &[ComponentArchive] = &[
  ComponentArchive {
    label: "AI実行環境",
    url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip",
    archive_sha256: None,
    files: RUNTIME_FILES,
  },
  ComponentArchive {
    label: "自然・忠実モデル",
    url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.3.0/realesrgan-ncnn-vulkan-20211212-windows.zip",
    archive_sha256: None,
    files: NATURAL_MODEL_FILES,
  },
];

static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);
static INSTALL_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
pub struct RealEsrganRuntime {
  pub executable: PathBuf,
  pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiComponentStatus {
  installed: bool,
  source: String,
  install_dir: String,
  total_bytes: u64,
  download_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiComponentProgress {
  phase: String,
  archive_index: usize,
  archive_count: usize,
  downloaded_bytes: u64,
  total_bytes: Option<u64>,
  message: String,
}

fn emit_progress(
  app: &AppHandle,
  phase: &str,
  archive_index: usize,
  archive_count: usize,
  downloaded_bytes: u64,
  total_bytes: Option<u64>,
  message: String,
) {
  let _ = app.emit(
    "ai-component-progress",
    AiComponentProgress {
      phase: phase.into(),
      archive_index,
      archive_count,
      downloaded_bytes,
      total_bytes,
      message,
    },
  );
}

fn component_files() -> impl Iterator<Item = &'static ComponentFile> {
  RUNTIME_FILES.iter().chain(NATURAL_MODEL_FILES.iter())
}

fn component_root_ready(root: &Path) -> bool {
  component_files().all(|component| root.join(component.relative_path).is_file())
}

fn component_root_verified(root: &Path) -> Result<bool, String> {
  for component in component_files() {
    let path = root.join(component.relative_path);
    if !path.is_file() || !sha256_matches(&path, component.sha256)? {
      return Ok(false);
    }
  }
  Ok(true)
}

fn portable_component_root() -> Option<PathBuf> {
  let executable = std::env::current_exe().ok()?;
  let directory = executable.parent()?;
  directory
    .join("portable.marker")
    .is_file()
    .then(|| directory.join("components").join("realesrgan"))
}

fn downloaded_component_root(app: &AppHandle) -> Result<PathBuf, String> {
  if let Some(root) = portable_component_root() {
    return Ok(root);
  }
  app
    .path()
    .app_local_data_dir()
    .map(|path| path.join("components").join("realesrgan"))
    .map_err(|error| format!("AIコンポーネントの保存先を取得できません: {error}"))
}

fn runtime_candidates(app: &AppHandle) -> Vec<PathBuf> {
  let mut candidates = Vec::new();
  if let Ok(root) = downloaded_component_root(app) {
    candidates.push(root);
  }
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
  candidates
}

pub fn resolve_realesrgan_runtime(app: &AppHandle) -> Result<RealEsrganRuntime, String> {
  for working_dir in runtime_candidates(app) {
    if component_root_ready(&working_dir) {
      return Ok(RealEsrganRuntime {
        executable: working_dir.join("realesrgan-ncnn-vulkan.exe"),
        working_dir,
      });
    }
  }
  Err("AIコンポーネントがありません。AI復元設定から追加してください".into())
}

fn component_status(app: &AppHandle) -> Result<AiComponentStatus, String> {
  let install_root = downloaded_component_root(app)?;
  let runtime = resolve_realesrgan_runtime(app).ok();
  let source = match runtime.as_ref() {
    Some(runtime) if runtime.working_dir == install_root => "downloaded",
    Some(_) => "bundled",
    None => "missing",
  };
  Ok(AiComponentStatus {
    installed: runtime.is_some(),
    source: source.into(),
    install_dir: install_root.to_string_lossy().to_string(),
    total_bytes: COMPONENT_BYTES,
    download_bytes: DOWNLOAD_BYTES,
  })
}

#[tauri::command]
pub fn get_ai_component_status(app: AppHandle) -> Result<AiComponentStatus, String> {
  component_status(&app)
}

#[tauri::command]
pub async fn install_ai_components(app: AppHandle) -> Result<AiComponentStatus, String> {
  if INSTALL_RUNNING
    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
    .is_err()
  {
    return Err("AIコンポーネントを追加中です".into());
  }
  INSTALL_CANCELLED.store(false, Ordering::SeqCst);
  let worker_app = app.clone();
  let result = tauri::async_runtime::spawn_blocking(move || install_components_sync(&worker_app))
    .await
    .map_err(|error| format!("AIコンポーネント追加処理が停止しました: {error}"));
  INSTALL_RUNNING.store(false, Ordering::SeqCst);
  result??;
  component_status(&app)
}

#[tauri::command]
pub fn cancel_ai_component_install() -> bool {
  if INSTALL_RUNNING.load(Ordering::SeqCst) {
    INSTALL_CANCELLED.store(true, Ordering::SeqCst);
    true
  } else {
    false
  }
}

#[tauri::command]
pub async fn remove_ai_components(app: AppHandle) -> Result<AiComponentStatus, String> {
  if INSTALL_RUNNING.load(Ordering::SeqCst) {
    return Err("追加処理が終わってから削除してください".into());
  }
  let root = downloaded_component_root(&app)?;
  tauri::async_runtime::spawn_blocking(move || {
    if root.exists() {
      fs::remove_dir_all(&root).map_err(|error| format!("AIコンポーネントを削除できません: {error}"))?;
    }
    Ok::<_, String>(())
  })
  .await
  .map_err(|error| error.to_string())??;
  component_status(&app)
}

fn install_components_sync(app: &AppHandle) -> Result<(), String> {
  let install_root = downloaded_component_root(app)?;
  if component_root_verified(&install_root)? {
    return Ok(());
  }

  let stamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let install_parent = install_root.parent().ok_or("AIコンポーネントの保存先が正しくありません")?;
  fs::create_dir_all(install_parent).map_err(|error| format!("AIコンポーネントの保存先を作成できません: {error}"))?;
  let staging_root = install_parent.join(format!(".realesrgan-install-{stamp}"));
  let cache_root = app
    .path()
    .app_cache_dir()
    .map_err(|error| format!("一時保存先を取得できません: {error}"))?
    .join(format!("ai-components-{stamp}"));
  fs::create_dir_all(&staging_root).map_err(|error| format!("AIコンポーネントの準備先を作成できません: {error}"))?;
  fs::create_dir_all(&cache_root).map_err(|error| format!("ダウンロード先を作成できません: {error}"))?;

  let result = (|| {
    let client = Client::builder()
      .user_agent(USER_AGENT)
      .connect_timeout(Duration::from_secs(30))
      .timeout(Duration::from_secs(20 * 60))
      .build()
      .map_err(|error| format!("ダウンロード機能を準備できません: {error}"))?;

    let archive_count = match download_and_extract_archives(app, &client, PRIMARY_ARCHIVES, &cache_root, &staging_root) {
      Ok(()) => PRIMARY_ARCHIVES.len(),
      Err(primary_error) => {
        check_cancelled()?;
        let _ = fs::remove_dir_all(&staging_root);
        fs::create_dir_all(&staging_root).map_err(|error| format!("予備の取得先を準備できません: {error}"))?;
        emit_progress(app, "retrying", 0, FALLBACK_ARCHIVES.len(), 0, None, "予備の公式配布元へ切り替えています…".into());
        download_and_extract_archives(app, &client, FALLBACK_ARCHIVES, &cache_root, &staging_root)
          .map_err(|fallback_error| format!("AIコンポーネントを取得できません。専用配布: {primary_error} / 公式配布: {fallback_error}"))?;
        FALLBACK_ARCHIVES.len()
      }
    };

    emit_progress(app, "verifying", archive_count, archive_count, 0, None, "AIコンポーネントを確認しています…".into());
    if !component_root_verified(&staging_root)? {
      return Err("AIコンポーネントの検証に失敗しました".into());
    }
    if install_root.exists() {
      fs::remove_dir_all(&install_root).map_err(|error| format!("古いAIコンポーネントを置き換えられません: {error}"))?;
    }
    fs::rename(&staging_root, &install_root).map_err(|error| format!("AIコンポーネントを保存できません: {error}"))?;
    emit_progress(app, "done", archive_count, archive_count, COMPONENT_BYTES, Some(COMPONENT_BYTES), "AI機能を追加しました".into());
    Ok(())
  })();

  let _ = fs::remove_dir_all(&cache_root);
  if result.is_err() {
    let _ = fs::remove_dir_all(&staging_root);
  }
  result
}

fn download_and_extract_archives(
  app: &AppHandle,
  client: &Client,
  archives: &[ComponentArchive],
  cache_root: &Path,
  staging_root: &Path,
) -> Result<(), String> {
  for (index, archive) in archives.iter().enumerate() {
      check_cancelled()?;
      let archive_path = cache_root.join(format!("archive-{index}.zip"));
      download_archive(Some(app), client, archive, index + 1, archives.len(), &archive_path)?;
      emit_progress(
        app,
        "extracting",
        index + 1,
        archives.len(),
        0,
        None,
        format!("{}を検証しています…", archive.label),
      );
      extract_components(&archive_path, staging_root, archive.files)?;
    }
  Ok(())
}

fn download_archive(
  app: Option<&AppHandle>,
  client: &Client,
  archive: &ComponentArchive,
  archive_index: usize,
  archive_count: usize,
  destination: &Path,
) -> Result<(), String> {
  if let Some(app) = app {
    emit_progress(
      app,
      "downloading",
      archive_index,
      archive_count,
      0,
      None,
      format!("{}をダウンロードしています…", archive.label),
    );
  }

  match download_archive_parallel(app, client, archive, archive_index, archive_count, destination) {
    Ok(true) => {}
    Ok(false) => {
      download_archive_sequential(app, client, archive, archive_index, archive_count, destination)?;
    }
    Err(error) => {
      check_cancelled()?;
      cleanup_download_parts(destination);
      let _ = fs::remove_file(destination);
      if let Some(app) = app {
        emit_progress(
          app,
          "retrying",
          archive_index,
          archive_count,
          0,
          None,
          format!("{}の接続を切り替えています…", archive.label),
        );
      }
      download_archive_sequential(app, client, archive, archive_index, archive_count, destination)
        .map_err(|fallback_error| format!("並列ダウンロード: {error} / 通常ダウンロード: {fallback_error}"))?;
    }
  }

  cleanup_download_parts(destination);
  if let Some(expected) = archive.archive_sha256 {
    if !sha256_matches(destination, expected)? {
      return Err(format!("{}の安全性を確認できませんでした", archive.label));
    }
  }
  Ok(())
}

fn download_archive_parallel(
  app: Option<&AppHandle>,
  client: &Client,
  archive: &ComponentArchive,
  archive_index: usize,
  archive_count: usize,
  destination: &Path,
) -> Result<bool, String> {
  let Some(total_bytes) = probe_range_size(client, archive.url)? else {
    return Ok(false);
  };
  if total_bytes < PARALLEL_DOWNLOAD_MIN_BYTES {
    return Ok(false);
  }

  let ranges = parallel_ranges(total_bytes, PARALLEL_DOWNLOADS);
  let progress = Arc::new(AtomicU64::new(0));
  let result = thread::scope(|scope| {
    let workers = ranges
      .iter()
      .enumerate()
      .map(|(part_index, &(start, end))| {
        let progress = Arc::clone(&progress);
        let part_path = download_part_path(destination, part_index);
        let worker_app = app.cloned();
        scope.spawn(move || {
          download_range(
            worker_app.as_ref(),
            client,
            archive,
            archive_index,
            archive_count,
            total_bytes,
            start,
            end,
            &part_path,
            &progress,
          )
        })
      })
      .collect::<Vec<_>>();

    let mut first_error = None;
    for worker in workers {
      match worker.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
        Err(_) if first_error.is_none() => first_error = Some("並列ダウンロード処理が停止しました".into()),
        _ => {}
      }
    }
    first_error.map_or(Ok(()), Err)
  });

  if let Err(error) = result {
    return Err(error);
  }
  combine_download_parts(destination, ranges.len())?;
  if let Some(app) = app {
    emit_progress(
      app,
      "downloading",
      archive_index,
      archive_count,
      total_bytes,
      Some(total_bytes),
      format!("{}を並列ダウンロードしています…", archive.label),
    );
  }
  Ok(true)
}

fn probe_range_size(client: &Client, url: &str) -> Result<Option<u64>, String> {
  let response = client
    .get(url)
    .header(RANGE, "bytes=0-0")
    .send()
    .and_then(|response| response.error_for_status())
    .map_err(|error| format!("分割ダウンロードを確認できません: {error}"))?;
  if response.status() != StatusCode::PARTIAL_CONTENT {
    return Ok(None);
  }
  Ok(
    response
      .headers()
      .get(CONTENT_RANGE)
      .and_then(|value| value.to_str().ok())
      .and_then(parse_content_range_total),
  )
}

fn parse_content_range_total(value: &str) -> Option<u64> {
  let (range, total) = value.rsplit_once('/')?;
  range.starts_with("bytes ").then_some(())?;
  total.parse().ok().filter(|total| *total > 0)
}

fn parallel_ranges(total_bytes: u64, part_count: usize) -> Vec<(u64, u64)> {
  if total_bytes == 0 {
    return Vec::new();
  }
  let part_count = part_count.max(1).min(total_bytes as usize);
  let part_size = (total_bytes + part_count as u64 - 1) / part_count as u64;
  (0..part_count)
    .filter_map(|index| {
      let start = index as u64 * part_size;
      (start < total_bytes).then(|| (start, (start + part_size - 1).min(total_bytes - 1)))
    })
    .collect()
}

fn download_part_path(destination: &Path, part_index: usize) -> PathBuf {
  let name = destination.file_name().and_then(|name| name.to_str()).unwrap_or("download.zip");
  destination.with_file_name(format!("{name}.part-{part_index}"))
}

fn cleanup_download_parts(destination: &Path) {
  for part_index in 0..PARALLEL_DOWNLOADS {
    let _ = fs::remove_file(download_part_path(destination, part_index));
  }
}

#[allow(clippy::too_many_arguments)]
fn download_range(
  app: Option<&AppHandle>,
  client: &Client,
  archive: &ComponentArchive,
  archive_index: usize,
  archive_count: usize,
  total_bytes: u64,
  start: u64,
  end: u64,
  destination: &Path,
  progress: &AtomicU64,
) -> Result<(), String> {
  check_cancelled()?;
  let expected_bytes = end - start + 1;
  let mut response = client
    .get(archive.url)
    .header(RANGE, format!("bytes={start}-{end}"))
    .send()
    .and_then(|response| response.error_for_status())
    .map_err(|error| format!("{}の一部を取得できません: {error}", archive.label))?;
  if response.status() != StatusCode::PARTIAL_CONTENT {
    return Err(format!("{}の配布元が分割取得に対応していません", archive.label));
  }
  let expected_range = format!("bytes {start}-{end}/");
  let actual_range = response.headers().get(CONTENT_RANGE).and_then(|value| value.to_str().ok()).unwrap_or_default();
  if !actual_range.starts_with(&expected_range) {
    return Err(format!("{}の分割範囲を確認できません", archive.label));
  }

  let mut output = File::create(destination).map_err(|error| format!("分割ダウンロードを保存できません: {error}"))?;
  let mut part_bytes = 0_u64;
  let mut last_reported = 0_u64;
  let mut buffer = vec![0_u8; 128 * 1024];
  loop {
    check_cancelled()?;
    let count = response.read(&mut buffer).map_err(|error| format!("分割ダウンロード中に通信が切れました: {error}"))?;
    if count == 0 {
      break;
    }
    part_bytes += count as u64;
    if part_bytes > expected_bytes {
      return Err(format!("{}の分割データが指定サイズを超えています", archive.label));
    }
    output.write_all(&buffer[..count]).map_err(|error| format!("分割ダウンロードを保存できません: {error}"))?;
    let downloaded_bytes = progress.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
    if part_bytes - last_reported >= DOWNLOAD_PROGRESS_STEP {
      last_reported = part_bytes;
      if let Some(app) = app {
        emit_progress(
          app,
          "downloading",
          archive_index,
          archive_count,
          downloaded_bytes,
          Some(total_bytes),
          format!("{}を並列ダウンロードしています…", archive.label),
        );
      }
    }
  }
  output.flush().map_err(|error| format!("分割ダウンロードを保存できません: {error}"))?;
  if part_bytes != expected_bytes {
    return Err(format!("{}の分割データが不足しています", archive.label));
  }
  Ok(())
}

fn combine_download_parts(destination: &Path, part_count: usize) -> Result<(), String> {
  let mut output = File::create(destination).map_err(|error| format!("ダウンロードファイルを保存できません: {error}"))?;
  let mut buffer = vec![0_u8; 256 * 1024];
  for part_index in 0..part_count {
    check_cancelled()?;
    let path = download_part_path(destination, part_index);
    let mut part = File::open(&path).map_err(|error| format!("分割ダウンロードを読み込めません: {error}"))?;
    loop {
      check_cancelled()?;
      let count = part.read(&mut buffer).map_err(|error| format!("分割ダウンロードを読み込めません: {error}"))?;
      if count == 0 {
        break;
      }
      output.write_all(&buffer[..count]).map_err(|error| format!("ダウンロードファイルを保存できません: {error}"))?;
    }
  }
  output.flush().map_err(|error| format!("ダウンロードファイルを保存できません: {error}"))
}

fn download_archive_sequential(
  app: Option<&AppHandle>,
  client: &Client,
  archive: &ComponentArchive,
  archive_index: usize,
  archive_count: usize,
  destination: &Path,
) -> Result<(), String> {
  let mut response = client
    .get(archive.url)
    .send()
    .and_then(|response| response.error_for_status())
    .map_err(|error| format!("{}をダウンロードできません: {error}", archive.label))?;
  let total_bytes = response.content_length();
  let mut output = File::create(destination).map_err(|error| format!("ダウンロードファイルを保存できません: {error}"))?;
  let mut downloaded_bytes = 0_u64;
  let mut buffer = vec![0_u8; 128 * 1024];
  loop {
    check_cancelled()?;
    let count = response.read(&mut buffer).map_err(|error| format!("ダウンロード中に通信が切れました: {error}"))?;
    if count == 0 {
      break;
    }
    output.write_all(&buffer[..count]).map_err(|error| format!("ダウンロードデータを保存できません: {error}"))?;
    downloaded_bytes += count as u64;
    if let Some(app) = app {
      emit_progress(
        app,
        "downloading",
        archive_index,
        archive_count,
        downloaded_bytes,
        total_bytes,
        format!("{}をダウンロードしています…", archive.label),
      );
    }
  }
  output.flush().map_err(|error| format!("ダウンロードデータを保存できません: {error}"))?;
  Ok(())
}

fn extract_components(archive_path: &Path, staging_root: &Path, components: &[ComponentFile]) -> Result<(), String> {
  let file = File::open(archive_path).map_err(|error| format!("ダウンロードファイルを開けません: {error}"))?;
  let mut archive = ZipArchive::new(file).map_err(|error| format!("ダウンロードファイルが壊れています: {error}"))?;
  for component in components {
    check_cancelled()?;
    let entry_index = (0..archive.len())
      .find(|index| {
        archive
          .by_index(*index)
          .ok()
          .and_then(|entry| Path::new(entry.name()).file_name().map(|name| name == component.archive_name))
          .unwrap_or(false)
      })
      .ok_or_else(|| format!("{}が公式配布ファイルに見つかりません", component.archive_name))?;
    let mut entry = archive.by_index(entry_index).map_err(|error| format!("{}を展開できません: {error}", component.archive_name))?;
    let destination = staging_root.join(component.relative_path);
    if let Some(parent) = destination.parent() {
      fs::create_dir_all(parent).map_err(|error| format!("AIコンポーネントの保存先を作成できません: {error}"))?;
    }
    let mut output = File::create(&destination).map_err(|error| format!("{}を保存できません: {error}", component.archive_name))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
      check_cancelled()?;
      let count = entry.read(&mut buffer).map_err(|error| format!("{}を展開できません: {error}", component.archive_name))?;
      if count == 0 {
        break;
      }
      output.write_all(&buffer[..count]).map_err(|error| format!("{}を保存できません: {error}", component.archive_name))?;
      hasher.update(&buffer[..count]);
    }
    let actual = format!("{:X}", hasher.finalize());
    if actual != component.sha256 {
      return Err(format!("{}の安全性を確認できませんでした", component.archive_name));
    }
  }
  Ok(())
}

fn sha256_matches(path: &Path, expected: &str) -> Result<bool, String> {
  let mut file = File::open(path).map_err(|error| format!("AIコンポーネントを確認できません: {error}"))?;
  let mut hasher = Sha256::new();
  let mut buffer = vec![0_u8; 128 * 1024];
  loop {
    let count = file.read(&mut buffer).map_err(|error| format!("AIコンポーネントを確認できません: {error}"))?;
    if count == 0 {
      break;
    }
    hasher.update(&buffer[..count]);
  }
  Ok(format!("{:X}", hasher.finalize()) == expected)
}

fn check_cancelled() -> Result<(), String> {
  if INSTALL_CANCELLED.load(Ordering::SeqCst) {
    Err("AIコンポーネントの追加をキャンセルしました".into())
  } else {
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn component_destinations_stay_relative_and_unique() {
    let mut paths = std::collections::HashSet::new();
    for component in component_files() {
      let path = Path::new(component.relative_path);
      assert!(!path.is_absolute());
      assert!(!path.components().any(|part| matches!(part, std::path::Component::ParentDir)));
      assert!(paths.insert(component.relative_path));
    }
    assert_eq!(paths.len(), 6);
  }

  #[test]
  fn sha256_verification_rejects_changed_content() {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("be-asset-optimizer-sha-{id}.txt"));
    fs::write(&path, b"abc").unwrap();
    assert!(sha256_matches(&path, "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD").unwrap());
    assert!(!sha256_matches(&path, "0000000000000000000000000000000000000000000000000000000000000000").unwrap());
    fs::remove_file(path).unwrap();
  }

  #[test]
  fn content_range_total_is_parsed_safely() {
    assert_eq!(parse_content_range_total("bytes 0-0/64116338"), Some(64_116_338));
    assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    assert_eq!(parse_content_range_total("invalid/100"), None);
    assert_eq!(parse_content_range_total("invalid"), None);
  }

  #[test]
  fn parallel_ranges_cover_the_file_once() {
    assert_eq!(parallel_ranges(10, 4), vec![(0, 2), (3, 5), (6, 8), (9, 9)]);
    assert_eq!(parallel_ranges(3, 4), vec![(0, 0), (1, 1), (2, 2)]);
    assert!(parallel_ranges(0, 4).is_empty());
    let ranges = parallel_ranges(DOWNLOAD_BYTES, PARALLEL_DOWNLOADS);
    assert_eq!(ranges.first().unwrap().0, 0);
    assert_eq!(ranges.last().unwrap().1, DOWNLOAD_BYTES - 1);
    assert_eq!(ranges.iter().map(|(start, end)| end - start + 1).sum::<u64>(), DOWNLOAD_BYTES);
  }

  #[test]
  #[ignore = "downloads about 61 MB from the component release using four connections"]
  fn component_release_download_extract_and_verify() {
    INSTALL_CANCELLED.store(false, Ordering::SeqCst);
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("be-asset-optimizer-components-{id}"));
    let staging = root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let client = Client::builder().user_agent(USER_AGENT).build().unwrap();
    for (index, archive) in PRIMARY_ARCHIVES.iter().enumerate() {
      let archive_path = root.join(format!("archive-{index}.zip"));
      assert!(download_archive_parallel(None, &client, archive, index + 1, PRIMARY_ARCHIVES.len(), &archive_path).unwrap());
      assert!(sha256_matches(&archive_path, archive.archive_sha256.unwrap()).unwrap());
      extract_components(&archive_path, &staging, archive.files).unwrap();
    }
    assert!(component_root_verified(&staging).unwrap());
    fs::remove_dir_all(root).unwrap();
  }
}
