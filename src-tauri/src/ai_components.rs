use std::{
  fs::{self, File},
  io::{Read, Write},
  path::{Path, PathBuf},
  sync::atomic::{AtomicBool, Ordering},
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use zip::ZipArchive;

const COMPONENT_BYTES: u64 = 73_425_210;
const DOWNLOAD_BYTES: u64 = 64_116_338;
const USER_AGENT: &str = "Be-Asset-Optimizer/0.5";

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
  drop(output);
  if let Some(expected) = archive.archive_sha256 {
    if !sha256_matches(destination, expected)? {
      return Err(format!("{}の安全性を確認できませんでした", archive.label));
    }
  }
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
  #[ignore = "downloads about 61 MB from the Be Asset Optimizer component release"]
  fn component_release_download_extract_and_verify() {
    INSTALL_CANCELLED.store(false, Ordering::SeqCst);
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("be-asset-optimizer-components-{id}"));
    let staging = root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let client = Client::builder().user_agent(USER_AGENT).build().unwrap();
    for (index, archive) in PRIMARY_ARCHIVES.iter().enumerate() {
      let archive_path = root.join(format!("archive-{index}.zip"));
      download_archive(None, &client, archive, index + 1, PRIMARY_ARCHIVES.len(), &archive_path).unwrap();
      extract_components(&archive_path, &staging, archive.files).unwrap();
    }
    assert!(component_root_verified(&staging).unwrap());
    fs::remove_dir_all(root).unwrap();
  }
}
