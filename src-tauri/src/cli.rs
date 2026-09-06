//! Headless entry point sharing the desktop application's image processors.
use super::*;
use std::{ffi::OsString, io::{self, Write}};

const HELP: &str = r#"Be Asset Optimizer CLI

Usage:
  be-asset-optimizer-cli optimize [OPTIONS] <FILE_OR_FOLDER>...
  be-asset-optimizer-cli normal [OPTIONS] <FILE_OR_FOLDER>...

Common options:
  -o, --output-dir <DIR>   Output folder (default: next to source)
  --json                   Write one JSON result per line (JSONL)
  --                       Treat remaining arguments as input paths
  -h, --help               Show help
  -V, --version            Show version

Optimize options:
  --format <png|webp|jpeg>  Default: png
  --quality <1..100>        WebP/JPEG quality (default: 82)
  --max-width <PX>          Positive integer; preserves aspect ratio
  --max-height <PX>         Positive integer; preserves aspect ratio
  --scale-percent <1..100>  Combined with width/height limits (smallest wins)
  --resize-mode <lanczos3|nearest>  Default: lanczos3
  --color-mode <auto|indexed|rgb24|rgba32|grayscale8>  Default: auto
  --colors <16..256>        Indexed palette size (default: 256)
  --no-dithering
  --optimization <fast|safe|max>  Default: fast
  --jpeg-background <white|black>  Default: white

Normal options:
  --max-long-edge <PX>
  --strength <0.1..4>      Default: 1
  --level <0.1..4>         Default: 1
  --convention <opengl|directx>  Default: opengl
  --invert-height
  --invert-green
  --pad-to-square          Pad the height input with height 0

Folders are scanned recursively (up to 8 levels), without following symlinks.
Input list is collected before conversion. No wildcard expansion is performed;
pass a folder to process its images. Existing output names get a numeric suffix.
PNG optimization may skip files that would not become smaller, as in the GUI.
Exit codes: 0 = done/skipped, 1 = processing/input failure, 2 = invalid arguments.
"#;

struct Options {
    normal: bool,
    json: bool,
    inputs: Vec<PathBuf>,
    optimize: Settings,
    normal_settings: NormalSettings,
}

fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut args = args.into_iter();
    let command = args.next().ok_or("Missing command; use --help")?;
    let normal = match command.to_str() {
        Some("optimize") => false,
        Some("normal") => true,
        _ => return Err("Expected optimize or normal; use --help".into()),
    };
    let mut options = Options {
        normal, json: false, inputs: Vec::new(),
        optimize: Settings {
            output_dir: None, output_format: "png".into(), quality: 82,
            jpeg_background: "white".into(), max_width: None, max_height: None,
            scale_percent: None, resize_mode: "lanczos3".into(), color_mode: "auto".into(),
            colors: Some(256), dithering: true, optimization: "fast".into(),
        },
        normal_settings: NormalSettings {
            output_dir: None, max_long_edge: None, strength: 1.0, level: 1.0,
            convention: "opengl".into(), invert_height: false, invert_green: false,
            pad_to_square: false,
        },
    };
    while let Some(arg) = args.next() {
        let key = arg.to_str().ok_or("Arguments must be valid Unicode")?;
        if key == "--" { options.inputs.extend(args.map(PathBuf::from)); break; }
        if !key.starts_with('-') { options.inputs.push(PathBuf::from(arg)); continue; }
        match key {
            "--json" => options.json = true,
            "--no-dithering" if !normal => options.optimize.dithering = false,
            "--invert-height" if normal => options.normal_settings.invert_height = true,
            "--invert-green" if normal => options.normal_settings.invert_green = true,
            "--pad-to-square" if normal => options.normal_settings.pad_to_square = true,
            "-o" | "--output-dir" => {
                let value = value(&mut args, key)?;
                if value.trim().is_empty() { return Err("Output directory cannot be empty".into()); }
                options.optimize.output_dir = Some(value.clone());
                options.normal_settings.output_dir = Some(value);
            }
            "--format" if !normal => options.optimize.output_format = choice(&mut args, key, &["png", "webp", "jpeg"] )?,
            "--quality" if !normal => options.optimize.quality = integer(&mut args, key, 1, 100)? as u8,
            "--max-width" if !normal => options.optimize.max_width = Some(integer(&mut args, key, 1, u32::MAX)?),
            "--max-height" if !normal => options.optimize.max_height = Some(integer(&mut args, key, 1, u32::MAX)?),
            "--scale-percent" if !normal => options.optimize.scale_percent = Some(integer(&mut args, key, 1, 100)?),
            "--colors" if !normal => options.optimize.colors = Some(integer(&mut args, key, 16, 256)?),
            "--resize-mode" if !normal => options.optimize.resize_mode = choice(&mut args, key, &["lanczos3", "nearest"] )?,
            "--color-mode" if !normal => options.optimize.color_mode = choice(&mut args, key, &["auto", "indexed", "rgb24", "rgba32", "grayscale8"] )?,
            "--optimization" if !normal => options.optimize.optimization = choice(&mut args, key, &["fast", "safe", "max"] )?,
            "--jpeg-background" if !normal => options.optimize.jpeg_background = choice(&mut args, key, &["white", "black"] )?,
            "--max-long-edge" if normal => options.normal_settings.max_long_edge = Some(integer(&mut args, key, 1, u32::MAX)?),
            "--strength" if normal => options.normal_settings.strength = decimal(&mut args, key)?,
            "--level" if normal => options.normal_settings.level = decimal(&mut args, key)?,
            "--convention" if normal => options.normal_settings.convention = choice(&mut args, key, &["opengl", "directx"] )?,
            _ => return Err(format!("Unknown option for this command: {key}")),
        }
    }
    if options.inputs.is_empty() { return Err("At least one input file or folder is required".into()); }
    Ok(options)
}

fn value(args: &mut impl Iterator<Item = OsString>, key: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("Missing value for {key}"))?
        .into_string().map_err(|_| format!("Invalid Unicode for {key}"))
}
fn choice(args: &mut impl Iterator<Item = OsString>, key: &str, choices: &[&str]) -> Result<String, String> {
    let v = value(args, key)?;
    if choices.contains(&v.as_str()) { Ok(v) } else { Err(format!("{key}: expected {}", choices.join("|"))) }
}
fn integer(args: &mut impl Iterator<Item = OsString>, key: &str, min: u32, max: u32) -> Result<u32, String> {
    let v = value(args, key)?.parse::<u32>().ok().filter(|v| (min..=max).contains(v));
    v.ok_or_else(|| format!("{key}: expected integer {min}..{max}"))
}
fn decimal(args: &mut impl Iterator<Item = OsString>, key: &str) -> Result<f32, String> {
    value(args, key)?.parse::<f32>().ok().filter(|v| v.is_finite() && (0.1..=4.0).contains(v))
        .ok_or_else(|| format!("{key}: expected number 0.1..4"))
}

fn execute(options: Options, out: &mut impl Write) -> io::Result<i32> {
    let mut paths = Vec::new();
    let mut failures = Vec::new();
    let mut seen = HashSet::new();
    for input in options.inputs {
        match fs::canonicalize(&input) {
            Ok(path) if path.is_dir() || is_supported_image(&path) =>
                collect_image_inputs(&path, 0, usize::MAX, &mut paths, &mut failures, &mut seen),
            Ok(_) => failures.push(InspectFailure { path: input.to_string_lossy().into(), message: "Unsupported image format".into() }),
            Err(error) => failures.push(InspectFailure { path: input.to_string_lossy().into(), message: error.to_string() }),
        }
    }
    if paths.is_empty() && failures.is_empty() {
        failures.push(InspectFailure { path: String::new(), message: "No supported images found".into() });
    }
    let mut failed = failures.len();
    let mut done = 0;
    let mut skipped = 0;
    for failure in failures {
        report(out, options.json, &Progress { path: failure.path, status: Status::Failed,
            output_bytes: None, output_width: None, output_height: None,
            message: Some(failure.message), output_path: None })?;
    }
    // Sequential output creation keeps the GUI's collision-safe naming deterministic.
    for path in paths {
        let result = if options.normal { process_normal_file(&path, &options.normal_settings) }
            else { process_file(&path, &options.optimize) };
        let event = match result {
            Ok((status, bytes, width, height, message, output_path)) => {
                match status { Status::Done => done += 1, Status::Skipped => skipped += 1, _ => {} }
                Progress { path: path.to_string_lossy().into(), status, output_bytes: bytes,
                    output_width: Some(width), output_height: Some(height), message, output_path }
            }
            Err(error) => {
                failed += 1;
                Progress { path: path.to_string_lossy().into(), status: Status::Failed,
                    output_bytes: None, output_width: None, output_height: None,
                    message: Some(error), output_path: None }
            }
        };
        report(out, options.json, &event)?;
    }
    if options.json {
        writeln!(out, "{}", serde_json::json!({"type": "summary", "done": done, "skipped": skipped, "failed": failed}))?;
    } else { writeln!(out, "Done: {done}, skipped: {skipped}, failed: {failed}")?; }
    Ok(if failed == 0 { 0 } else { 1 })
}

fn report(out: &mut impl Write, json: bool, event: &Progress) -> io::Result<()> {
    if json {
        writeln!(out, "{}", serde_json::to_string(event).map_err(io::Error::other)?)?;
    } else {
        writeln!(out, "{:?}: {} -> {}{}", event.status, event.path,
            event.output_path.as_deref().unwrap_or("-"),
            event.message.as_ref().map(|m| format!(" ({m})")).unwrap_or_default())?;
    }
    out.flush()
}

/// Returns a conventional process exit code. Does not start Tauri or a webview.
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<_> = args.collect();
    let option_args: Vec<_> = args.iter().take_while(|arg| *arg != "--").collect();
    if args.is_empty() || option_args.iter().any(|arg| *arg == "--help" || *arg == "-h") {
        return if writeln!(io::stdout().lock(), "{HELP}").is_ok() { 0 } else { 1 };
    }
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        return if writeln!(io::stdout().lock(), "Be Asset Optimizer {}", env!("CARGO_PKG_VERSION")).is_ok() { 0 } else { 1 };
    }
    match parse(args) {
        Ok(options) => match execute(options, &mut io::stdout().lock()) {
            Ok(code) => code,
            Err(error) => { eprintln!("Output error: {error}"); 1 }
        },
        Err(error) => { eprintln!("{error}"); 2 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Result<Options, String> { parse(args.iter().map(OsString::from).collect()) }
    #[test]
    fn validates_options_before_processing() {
        for args in [vec!["optimize", "a.png", "--quality", "101"], vec!["normal", "a.png", "--strength", "NaN"],
            vec!["optimize", "a.png", "--max-width", "0"], vec!["normal", "a.png", "--format", "png"]] {
            assert!(options(&args).is_err());
        }
        let parsed = options(&["optimize", "--json", "--", "-画像.png"]).unwrap();
        assert_eq!(parsed.inputs, vec![PathBuf::from("-画像.png")]);
        assert!(parsed.json);
    }
    #[test]
    fn converts_real_files_and_reports_partial_failure() {
        let dir = std::env::temp_dir().join(format!("be-cli-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("画像 source.bmp");
        image::RgbImage::from_pixel(32, 16, image::Rgb([90, 120, 180])).save(&input).unwrap();
        let out_dir = dir.join("output");
        let parsed = options(&["optimize", "--json", "--format", "png", "--max-width", "8", "-o",
            out_dir.to_str().unwrap(), input.to_str().unwrap(), dir.join("missing.png").to_str().unwrap()]).unwrap();
        let mut output = Vec::new();
        assert_eq!(execute(parsed, &mut output).unwrap(), 1);
        let lines: Vec<serde_json::Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.last().unwrap()["done"], 1);
        assert_eq!(lines.last().unwrap()["failed"], 1);
        let generated = lines.iter().find_map(|l| l["output_path"].as_str()).unwrap();
        assert_eq!(image::image_dimensions(generated).unwrap(), (8, 4));
        assert_eq!(image::image_dimensions(&input).unwrap(), (32, 16));
        fs::remove_dir_all(&dir).unwrap();
    }
}
