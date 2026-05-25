use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

const FONT_DIRS: &[&str] = &[
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "~/.local/share/fonts",
    "~/.fonts",
];

const PREFERRED_CJK_FONTS: &[&str] = &[
    "ipaexg.ttf",
    "ipag.ttf",
    "TakaoGothic.ttf",
    "VL-Gothic-Regular.ttf",
    "NotoSansJP-Regular.otf",
    "NotoSansCJK-Regular.ttc",
    "NotoSansCJKjp-Regular.otf",
    "SourceHanSansJP-Regular.otf",
];

#[derive(Debug, Clone)]
pub struct FontCheck {
    pub font_path: Option<PathBuf>,
}

impl FontCheck {
    pub fn status_line(&self) -> String {
        match &self.font_path {
            Some(path) => format!("日本語 UI フォント: {}", path.display()),
            None => {
                "日本語 UI フォントを自動検出できませんでした。システム既定フォントを使います。"
                    .to_string()
            }
        }
    }
}

pub fn configure(ctx: &egui::Context) -> FontCheck {
    let font_check = detect_system_cjk_font();
    if let Some(path) = &font_check.font_path {
        install_system_cjk_font(ctx, path);
    }
    font_check
}

pub fn detect_system_cjk_font() -> FontCheck {
    FontCheck {
        font_path: find_system_cjk_font(),
    }
}

fn install_system_cjk_font(ctx: &egui::Context, font_path: &Path) {
    let bytes = match fs::read(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "[window-jump] failed to read UI font {}: {error}",
                font_path.display()
            );
            return;
        }
    };

    let font_name = "system-cjk".to_string();
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert(font_name.clone(), FontData::from_owned(bytes).into());

    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, font_name.clone());
    }

    if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
        family.push(font_name);
    }

    ctx.set_fonts(fonts);
    eprintln!("[window-jump] loaded UI font {}", font_path.display());
}

fn find_system_cjk_font() -> Option<PathBuf> {
    let candidates = collect_font_candidates();
    select_cjk_font(&candidates)
}

fn collect_font_candidates() -> Vec<PathBuf> {
    let mut fonts = Vec::new();
    for dir in FONT_DIRS {
        let path = expand_home(dir);
        collect_font_candidates_from_dir(&path, &mut fonts);
    }
    fonts
}

fn collect_font_candidates_from_dir(dir: &Path, fonts: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_candidates_from_dir(&path, fonts);
        } else if is_supported_font_file(&path) {
            fonts.push(path);
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(path));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }

    PathBuf::from(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn select_cjk_font(candidates: &[PathBuf]) -> Option<PathBuf> {
    for preferred in PREFERRED_CJK_FONTS {
        if let Some(path) = candidates
            .iter()
            .find(|path| file_name_eq(path, preferred))
            .cloned()
        {
            return Some(path);
        }
    }

    candidates
        .iter()
        .filter_map(|path| cjk_font_score(path).map(|score| (score, path)))
        .max_by_key(|(score, path)| (*score, path.to_string_lossy().to_string()))
        .map(|(_, path)| path.clone())
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

fn cjk_font_score(path: &Path) -> Option<i32> {
    if !is_supported_font_file(path) {
        return None;
    }

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut score = 0;
    for marker in [
        "cjk",
        "japanese",
        "jp",
        "ipa",
        "takao",
        "vl-gothic",
        "sourcehan",
        "migu",
        "ume",
    ] {
        if name.contains(marker) {
            score += 20;
        }
    }

    if score == 0 {
        return None;
    }

    if name.contains("sans") || name.contains("gothic") || name.contains("goth") {
        score += 8;
    }
    if name.contains("regular") || name.contains("ipaexg") || name == "ipag.ttf" {
        score += 6;
    }
    if name.contains("bold") || name.contains("black") || name.contains("thin") {
        score -= 4;
    }
    if path.extension().and_then(|ext| ext.to_str()) == Some("ttc") {
        score -= 2;
    }

    Some(score)
}

fn is_supported_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            ext == "ttf" || ext == "otf" || ext == "ttc"
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_cjk_font_wins_over_later_candidates() {
        let candidates = vec![
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/opentype/ipaexfont-gothic/ipaexg.ttf"),
        ];

        let selected = select_cjk_font(&candidates).expect("select font");
        assert_eq!(selected.file_name().unwrap(), "ipaexg.ttf");
    }

    #[test]
    fn score_accepts_common_japanese_font_names() {
        let font = PathBuf::from("/usr/share/fonts/truetype/takao-gothic/TakaoGothic.ttf");
        assert!(cjk_font_score(&font).is_some());
    }

    #[test]
    fn non_font_files_are_not_selected() {
        let candidates = vec![PathBuf::from("/usr/share/fonts/readme.txt")];
        assert!(select_cjk_font(&candidates).is_none());
    }
}
