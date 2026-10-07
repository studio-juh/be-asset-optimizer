use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelInput { path: Option<String>, channel: String, invert: bool, value: u8 }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelSettings { channels: Vec<ChannelInput>, resolution: Option<u32>, output_dir: Option<String> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NormalAdjustSettings { base: String, detail: Option<String>, strength: f32, detail_strength: f32, flip_green: bool, output_dir: Option<String> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextureDiagnosis { path: String, width: u32, height: u32, file_bytes: u64, rgba_bytes: u64, power_of_two: bool, error: Option<String> }

fn checked_pixels(width: u32, height: u32) -> Result<(), String> {
  if width == 0 || height == 0 || width > 16384 || height > 16384 || width as u64 * height as u64 > 64 * 1024 * 1024 { return Err("画像が大きすぎます（最大64メガピクセル）".into()); }
  Ok(())
}
fn load_checked(path: &str) -> Result<image::RgbaImage, String> {
  let path = Path::new(path);
  if !is_supported_image(path) { return Err("対応している画像を指定してください".into()); }
  let (w, h) = input_dimensions(path)?;
  checked_pixels(w, h)?;
  let image = load_input_image(path)?.to_rgba8();
  checked_pixels(image.width(), image.height())?;
  Ok(image)
}
fn save(image: &image::RgbaImage, output: Option<&str>, source: &str, suffix: &str) -> Result<AtlasResult, String> {
  let dir = resolve_output_dir(Path::new(source), output)?;
  fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
  let path = unique_named_output_path(&dir, Path::new(source).file_stem().and_then(|s| s.to_str()).unwrap_or("texture"), suffix);
  let encoded = encode_rgba(image, image.width(), image.height())?;
  let encoded = oxipng::optimize_from_memory(&encoded, &lossless_options(0)).map_err(|e| e.to_string())?;
  fs::write(&path, &encoded).map_err(|e| e.to_string())?;
  Ok(AtlasResult { output_path: path.to_string_lossy().into(), output_bytes: encoded.len() as u64, width: image.width(), height: image.height() })
}
fn component(pixel: &image::Rgba<u8>, channel: &str) -> Result<u8, String> {
  Ok(match channel { "r" => pixel[0], "g" => pixel[1], "b" => pixel[2], "a" => pixel[3], "luma" => (pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722).round() as u8, _ => return Err("チャンネル指定が不正です".into()) })
}
fn pack(settings: &ChannelSettings) -> Result<(image::RgbaImage, String), String> {
  if settings.channels.len() != 4 { return Err("RGBAの4チャンネルを指定してください".into()); }
  let source = settings.channels.iter().find_map(|c| c.path.as_ref()).ok_or("画像を1枚以上指定してください")?.clone();
  if settings.resolution.is_some_and(|r| !(16..=4096).contains(&r)) { return Err("解像度は16〜4096 pxです".into()); }
  let first = load_checked(&source)?;
  let (width, height) = settings.resolution.map(|r| (r, r)).unwrap_or(first.dimensions());
  drop(first);
  checked_pixels(width, height)?;
  let mut output = image::RgbaImage::new(width, height);
  for (i, channel) in settings.channels.iter().enumerate() {
    component(&image::Rgba([0,0,0,0]), &channel.channel)?;
    let input = match &channel.path { Some(path) => { let input = load_checked(path)?; Some(if input.dimensions() == (width, height) { input } else { image::imageops::resize(&input, width, height, FilterType::Lanczos3) }) }, None => None };
    for (x, y, pixel) in output.enumerate_pixels_mut() {
      let value = match &input { Some(input) => component(input.get_pixel(x,y), &channel.channel)?, None => channel.value };
      pixel[i] = if channel.invert { 255 - value } else { value };
    }
  }
  Ok((output, source))
}
#[tauri::command]
pub(crate) async fn pack_channels(settings: ChannelSettings) -> Result<AtlasResult, String> {
  tauri::async_runtime::spawn_blocking(move || { let (image, source) = pack(&settings)?; save(&image, settings.output_dir.as_deref(), &source, "packed") }).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn split_channels(path: String, output_dir: Option<String>) -> Result<Vec<AtlasResult>, String> {
  tauri::async_runtime::spawn_blocking(move || {
    let source = load_checked(&path)?;
    let mut outputs = Vec::new();
    for (i, name) in ["r", "g", "b", "a"].iter().enumerate() {
      let image = image::RgbaImage::from_fn(source.width(), source.height(), |x,y| { let v = source.get_pixel(x,y)[i]; image::Rgba([v,v,v,255]) });
      outputs.push(save(&image, output_dir.as_deref(), &path, name)?);
    }
    Ok(outputs)
  }).await.map_err(|e| e.to_string())?
}
fn decode_normal(pixel: &image::Rgba<u8>, strength: f32, flip: bool) -> [f32; 3] {
  let x = (pixel[0] as f32 / 255.0 * 2.0 - 1.0) * strength;
  let y = (pixel[1] as f32 / 255.0 * 2.0 - 1.0) * strength * if flip { -1.0 } else { 1.0 };
  let z = (pixel[2] as f32 / 255.0 * 2.0 - 1.0).max(0.0001);
  normalize([x,y,z])
}
fn normalize(v: [f32; 3]) -> [f32; 3] { let length = (v[0]*v[0]+v[1]*v[1]+v[2]*v[2]).sqrt().max(0.0001); [v[0]/length,v[1]/length,v[2]/length] }
fn combine(base: [f32;3], detail: [f32;3]) -> [f32;3] {
  // Reoriented normal mapping: rotate detail into the base normal's frame.
  let t = [base[0],base[1],base[2]+1.0]; let u = [-detail[0],-detail[1],detail[2]];
  let d = (t[0]*u[0]+t[1]*u[1]+t[2]*u[2])/t[2].max(0.0001);
  normalize([t[0]*d-u[0],t[1]*d-u[1],t[2]*d-u[2]])
}
#[tauri::command]
pub(crate) async fn adjust_normal(settings: NormalAdjustSettings) -> Result<AtlasResult, String> {
  tauri::async_runtime::spawn_blocking(move || {
    if !settings.strength.is_finite() || !settings.detail_strength.is_finite() || !(0.0..=4.0).contains(&settings.strength) || !(0.0..=4.0).contains(&settings.detail_strength) { return Err("強度は0〜4で指定してください".into()); }
    let base = load_checked(&settings.base)?;
    let detail = if let Some(path) = &settings.detail { let image = load_checked(path)?; Some(image::imageops::resize(&image, base.width(), base.height(), FilterType::Lanczos3)) } else { None };
    let output = image::RgbaImage::from_fn(base.width(),base.height(), |x,y| {
      let normal = decode_normal(base.get_pixel(x,y),settings.strength,settings.flip_green);
      let normal = match &detail { Some(detail) => combine(normal,decode_normal(detail.get_pixel(x,y),settings.detail_strength,settings.flip_green)), None => normal };
      let encode = |v:f32| ((v*0.5+0.5)*255.0).round().clamp(0.0,255.0) as u8;
      image::Rgba([encode(normal[0]),encode(normal[1]),encode(normal[2]),255])
    });
    // Normal maps do not require an alpha channel.
    let dir = resolve_output_dir(Path::new(&settings.base),settings.output_dir.as_deref())?;
    fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
    let path = unique_named_output_path(&dir,Path::new(&settings.base).file_stem().and_then(|s|s.to_str()).unwrap_or("normal"),"adjusted");
    let encoded = encode_rgb24(&output,output.width(),output.height())?;
    let encoded = oxipng::optimize_from_memory(&encoded,&lossless_options(0)).map_err(|e|e.to_string())?;
    fs::write(&path,&encoded).map_err(|e|e.to_string())?;
    Ok(AtlasResult { output_path:path.to_string_lossy().into(), output_bytes:encoded.len() as u64,width:output.width(),height:output.height() })
  }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub(crate) async fn diagnose_textures(paths: Vec<String>) -> Result<Vec<TextureDiagnosis>, String> {
  if paths.len() > 1500 { return Err("診断は最大1500枚です".into()); }
  tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|path| {
    let result = input_dimensions(Path::new(&path));
    let (width,height,error) = match result { Ok((w,h)) => (w,h,None), Err(e) => (0,0,Some(e)) };
    TextureDiagnosis { file_bytes:fs::metadata(&path).map(|m|m.len()).unwrap_or(0),path,width,height,rgba_bytes:width as u64*height as u64*4,power_of_two:width.is_power_of_two()&&height.is_power_of_two(),error }
  }).collect::<Vec<_>>()).await.map_err(|e|e.to_string())
}
#[cfg(test)]
mod tests {
  use super::*;
  #[test] fn rnm_preserves_flat_and_unit_length() { let base=normalize([0.4,0.2,1.0]); let result=combine(base,[0.0,0.0,1.0]); for i in 0..3 { assert!((base[i]-result[i]).abs()<0.00001); } assert_eq!(normalize([0.0,0.0,1.0]),[0.0,0.0,1.0]); }
  #[test] fn flip_and_strength_are_applied() { let p=image::Rgba([200,180,240,255]); let a=decode_normal(&p,1.0,false);let b=decode_normal(&p,1.0,true);assert!((a[1]+b[1]).abs()<0.00001);let flat=decode_normal(&p,0.0,false);assert_eq!(flat,[0.0,0.0,1.0]); }
  #[test] fn channel_selection_and_limits() { let p=image::Rgba([10,20,30,40]);assert_eq!(component(&p,"a").unwrap(),40);assert!(component(&p,"bad").is_err());assert!(checked_pixels(20000,1).is_err()); }
  #[test] fn packing_splitting_adjustment_and_diagnosis_work_on_files() {
    let root=std::env::temp_dir().join(format!("be-texture-tools-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&root).unwrap();let path=root.join("source.png");
    let source=image::RgbaImage::from_pixel(16,8,image::Rgba([10,20,240,128]));source.save(&path).unwrap();
    let original=fs::read(&path).unwrap();let path=path.to_string_lossy().to_string();let out=root.join("out").to_string_lossy().to_string();
    let settings=ChannelSettings { channels:vec![ChannelInput{path:Some(path.clone()),channel:"r".into(),invert:false,value:0},ChannelInput{path:Some(path.clone()),channel:"g".into(),invert:true,value:0},ChannelInput{path:None,channel:"luma".into(),invert:false,value:33},ChannelInput{path:Some(path.clone()),channel:"a".into(),invert:false,value:255}],resolution:Some(32),output_dir:Some(out.clone()) };
    let (packed,_)=pack(&settings).unwrap();assert_eq!(packed.dimensions(),(32,32));assert_eq!(packed.get_pixel(0,0).0,[10,235,33,128]);
    let saved=tauri::async_runtime::block_on(pack_channels(settings)).unwrap();assert_eq!(image::image_dimensions(saved.output_path).unwrap(),(32,32));
    let split=tauri::async_runtime::block_on(split_channels(path.clone(),Some(out.clone()))).unwrap();
    assert_eq!(split.len(),4);
    for (i,item) in split.iter().enumerate(){
      let image=image::open(&item.output_path).unwrap().to_rgba8();
      let value=source.get_pixel(0,0)[i];
      assert_eq!(image.get_pixel(0,0).0,[value,value,value,255]);
    }
    let adjusted=tauri::async_runtime::block_on(adjust_normal(NormalAdjustSettings{base:path.clone(),detail:Some(path.clone()),strength:0.0,detail_strength:0.0,flip_green:true,output_dir:Some(out)})).unwrap();let image=image::open(adjusted.output_path).unwrap();assert!(!image.color().has_alpha());assert_eq!(image.to_rgb8().get_pixel(0,0).0,[128,128,255]);
    let diagnosis=tauri::async_runtime::block_on(diagnose_textures(vec![path.clone(),root.join("missing.png").to_string_lossy().into()])).unwrap();assert_eq!(diagnosis[0].rgba_bytes,16*8*4);assert!(diagnosis[0].power_of_two);assert!(diagnosis[1].error.is_some());assert_eq!(fs::read(path).unwrap(),original);fs::remove_dir_all(root).unwrap();
  }
}
