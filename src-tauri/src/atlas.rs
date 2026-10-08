use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AtlasSlot {
    pub path: Option<String>,
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

pub(crate) fn validate(slots: &[AtlasSlot]) -> Result<(), String> {
    if slots.is_empty() || slots.len() > 16 { return Err("アトラスは最大16枠です".into()); }
    let mut occupied = [false; 16];
    for slot in slots {
        if !matches!(slot.size, 1 | 2) || slot.x > 3 || slot.y > 3
            || slot.x + slot.size > 4 || slot.y + slot.size > 4
            || (slot.size == 2 && (slot.x % 2 != 0 || slot.y % 2 != 0)) {
            return Err("アトラスの枠配置が不正です".into());
        }
        for y in slot.y..slot.y + slot.size {
            for x in slot.x..slot.x + slot.size {
                if std::mem::replace(&mut occupied[(y * 4 + x) as usize], true) {
                    return Err("アトラスの枠が重なっています".into());
                }
            }
        }
    }
    if occupied.iter().any(|filled| !filled) { return Err("アトラスの枠配置が不足しています".into()); }
    Ok(())
}

#[cfg(test)]
pub(crate) fn render(slots: &[AtlasSlot], images: &[Option<image::RgbaImage>], square: bool, resolution: Option<u32>) -> Result<image::RgbaImage, String> {
    render_with_padding(slots, images, square, resolution, 0)
}
#[cfg(test)]
pub(crate) fn render_with_padding(slots: &[AtlasSlot], images: &[Option<image::RgbaImage>], square: bool, resolution: Option<u32>, padding: u32) -> Result<image::RgbaImage, String> {
    let dimensions: Vec<_> = images.iter().map(|image| image.as_ref().map(image::RgbaImage::dimensions)).collect();
    render_streaming(slots, &dimensions, square, resolution, padding, |index| images[index].clone().ok_or("画像が指定されていません".into()))
}

// Plan using headers, then decode and release one source at a time.
pub(crate) fn render_streaming<F>(slots: &[AtlasSlot], dimensions: &[Option<(u32, u32)>], square: bool, resolution: Option<u32>, padding: u32, mut load: F) -> Result<image::RgbaImage, String>
where F: FnMut(usize) -> Result<image::RgbaImage, String> {
    validate(slots)?;
    if padding > 64 { return Err("余白は0〜64 pxで指定してください".into()); }
    if slots.len() != dimensions.len() || dimensions.iter().all(Option::is_none) { return Err("画像を1枚以上指定してください".into()); }
    if resolution.is_some_and(|v| !(16..=4096).contains(&v)) { return Err("1枠の解像度は16〜4096 pxで指定してください".into()); }
    let mut cell_width = 1u64;
    let mut cell_height = 1u64;
    for (slot, dimension) in slots.iter().zip(dimensions) {
        if let Some((width, height)) = *dimension {
            if width == 0 || height == 0 { return Err("空の画像は使用できません".into()); }
            if width > 16384 || height > 16384 || width as u64 * height as u64 > 64 * 1024 * 1024 { return Err("入力画像が大きすぎます（最大64メガピクセル）".into()); }
            cell_width = cell_width.max((width as u64 + padding as u64 * 2) * 2 / slot.size as u64);
            cell_height = cell_height.max((height as u64 + padding as u64 * 2) * 2 / slot.size as u64);
        }
    }
    if square { cell_width = resolution.map(u64::from).unwrap_or(cell_width.max(cell_height)); cell_height = cell_width; }
    let (width, height) = (cell_width * 2, cell_height * 2);
    if width > 16384 || height > 16384 || width * height > 64 * 1024 * 1024 {
        return Err("アトラスが大きすぎます。1枠の解像度を下げてください（最大64メガピクセル）".into());
    }
    let (width, height) = (width as u32, height as u32);
    let mut atlas = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for (index, slot) in slots.iter().enumerate() {
        if dimensions[index].is_none() { continue; }
        let (x, y) = (slot.x * width / 4, slot.y * height / 4);
        let (w, h) = ((slot.x + slot.size) * width / 4 - x, (slot.y + slot.size) * height / 4 - y);
        if w == 0 || h == 0 { return Err("分割枠の解像度が小さすぎます".into()); }
        if w <= padding * 2 || h <= padding * 2 { return Err("枠に対して余白が大きすぎます".into()); }
        let source = load(index)?;
        if source.width() == 0 || source.height() == 0 || source.width() as u64 * source.height() as u64 > 64 * 1024 * 1024 { return Err("入力画像の寸法が不正です".into()); }
        let fitted = super::fit_image_to_cell(&source, w - padding * 2, h - padding * 2);
        drop(source);
        // Keep existing transparent padding for occupied cells; empty cells are opaque black.
        for row in y..y + h { for col in x..x + w { atlas.put_pixel(col, row, image::Rgba([0, 0, 0, 0])); } }
        let left = x + (w - fitted.width()) / 2;
        let top = y + (h - fitted.height()) / 2;
        for row in top-padding..top+fitted.height()+padding {
            for col in left-padding..left+fitted.width()+padding {
                let sx = col.saturating_sub(left).min(fitted.width()-1);
                let sy = row.saturating_sub(top).min(fitted.height()-1);
                atlas.put_pixel(col,row,*fitted.get_pixel(sx,sy));
            }
        }
    }
    Ok(atlas)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Vec<AtlasSlot> { (0..4).map(|i| AtlasSlot { path: None, x: (i % 2) * 2, y: (i / 2) * 2, size: 2 }).collect() }
    fn pixel(color: u8) -> Option<image::RgbaImage> { Some(image::RgbaImage::from_pixel(2, 2, image::Rgba([color, 0, 0, 255]))) }
    #[test]
    fn streaming_loads_only_occupied_slots_in_order() {
        let dimensions = [Some((256, 256)), None, Some((256, 256)), None];
        let mut loaded = Vec::new();
        let result = render_streaming(&base(), &dimensions, true, Some(16), 0, |index| {
            loaded.push(index);
            Ok(image::RgbaImage::from_pixel(256, 256, image::Rgba([index as u8 + 1, 0, 0, 255])))
        }).unwrap();
        assert_eq!(loaded, [0, 2]);
        assert_eq!(result.dimensions(), (32, 32));
        assert_eq!(result.get_pixel(8, 8).0, [1, 0, 0, 255]);
        assert_eq!(result.get_pixel(8, 24).0, [3, 0, 0, 255]);
        assert_eq!(result.get_pixel(24, 8).0, [0, 0, 0, 255]);
    }
    #[test]
    fn oversized_input_and_output_are_rejected_before_decoding() {
        for (dimension, resolution) in [(Some((8193, 8192)), Some(16)), (Some((8192, 8192)), None)] {
            let mut calls = 0;
            let result = render_streaming(&base(), &[dimension, None, None, None], true, resolution, 0, |_| {
                calls += 1;
                Ok(image::RgbaImage::new(1, 1))
            });
            assert!(result.is_err());
            assert_eq!(calls, 0);
        }
    }
    #[test]
    fn padding_extrudes_edges_without_crossing_slots() {
        let images = vec![pixel(10),pixel(20),None,None];
        let result = render_with_padding(&base(),&images,true,Some(16),2).unwrap();
        assert_eq!(result.get_pixel(5,5).0,[10,0,0,255]);
        assert_eq!(result.get_pixel(21,5).0,[20,0,0,255]);
        assert_eq!(result.get_pixel(0,20).0,[0,0,0,255]);
        assert!(render_with_padding(&base(),&images,true,Some(16),8).is_err());
    }
    #[test]
    fn two_and_three_images_keep_empty_positions_black() {
        for images in [vec![pixel(10), None, pixel(30), None], vec![pixel(10), pixel(20), None, pixel(40)]] {
            let result = render(&base(), &images, true, None).unwrap();
            for (i, image) in images.iter().enumerate() { assert_eq!(result.get_pixel((i as u32 % 2) * 2, (i as u32 / 2) * 2).0, image.as_ref().map(|im| im.get_pixel(0,0).0).unwrap_or([0,0,0,255])); }
        }
    }
    #[test]
    fn seven_images_use_three_large_and_four_small_slots() {
        let mut slots = base(); slots.pop();
        slots.extend((0..4).map(|i| AtlasSlot { path: None, x: 2 + i % 2, y: 2 + i / 2, size: 1 }));
        let images: Vec<_> = (1..=7).map(pixel).collect();
        let result = render(&slots, &images, true, None).unwrap();
        assert_eq!(result.dimensions(), (8, 8));
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(result.get_pixel(slot.x * 2 + if slot.size == 2 {1} else {0}, slot.y * 2 + if slot.size == 2 {1} else {0}).0, [i as u8 + 1,0,0,255]);
        }
    }
    #[test]
    fn sixteen_slots_preserve_alpha_and_odd_resolution_has_no_gaps() {
        let slots: Vec<_> = (0..16).map(|i| AtlasSlot { path: None, x: i % 4, y: i / 4, size: 1 }).collect();
        let images: Vec<_> = (0..16).map(|i| Some(image::RgbaImage::from_pixel(40, 40, image::Rgba([i as u8, 20, 30, 128])))).collect();
        let result = render(&slots, &images, true, Some(17)).unwrap();
        assert_eq!(result.dimensions(), (34, 34));
        for y in 0..34 { for x in 0..34 {
            // Odd cell boundaries can make a cell 8x9, with transparent aspect padding.
            // Opaque black (255) would indicate an uncovered layout gap.
            assert!(matches!(result.get_pixel(x, y).0[3], 0 | 128));
        } }
        assert_eq!(result.get_pixel(33, 33).0, [15, 20, 30, 128]);
    }
    #[test]
    fn invalid_layout_empty_input_and_oversize_are_rejected() {
        let mut slots = base(); slots[1].x = 0;
        assert!(validate(&slots).is_err());
        assert!(validate(&base()[..3]).is_err());
        assert!(render(&base(), &[None,None,None,None], true, None).is_err());
        assert!(render(&base(), &[pixel(1)], true, None).is_err());
        let large = image::RgbaImage::new(9000, 1);
        assert!(render(&base(), &[Some(large),None,None,None], true, None).is_err());
    }
}
