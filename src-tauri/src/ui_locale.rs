use std::{collections::HashMap, sync::OnceLock};

// Share menu translations with the frontend so names cannot drift between
// the native menu, tabs and help. The catalog is embedded; no network is used.
pub fn text(locale: &str, japanese: &str, english: &str) -> String {
    if locale == "ja" { return japanese.to_owned(); }
    let index = match locale { "zh-CN" => 0, "de" => 1, _ => return english.to_owned() };
    static CATALOG: OnceLock<HashMap<String, [String; 2]>> = OnceLock::new();
    let catalog = CATALOG.get_or_init(|| serde_json::from_str(include_str!("../../src/locales.json")).expect("Invalid embedded translation catalog"));
    catalog.get(english).map_or_else(|| english.to_owned(), |entry| entry[index].clone())
}

#[cfg(test)]
mod tests {
    use super::text;
    #[test]
    fn menu_translations_use_shared_catalog_and_safe_fallback() {
        assert_eq!(text("ja", "画像最適化", "Image Optimization"), "画像最適化");
        assert_eq!(text("en", "画像最適化", "Image Optimization"), "Image Optimization");
        assert_eq!(text("zh-CN", "画像最適化", "Image Optimization"), "图像优化");
        assert_eq!(text("de", "画像最適化", "Image Optimization"), "Bildoptimierung");
        assert_eq!(text("de", "不明", "Unknown"), "Unknown");
        assert_eq!(text("unknown", "不明", "Unknown"), "Unknown");
    }
}
