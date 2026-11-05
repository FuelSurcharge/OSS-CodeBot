use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    #[default]
    #[serde(rename = "line-seed-jp")]
    LineSeedJp,
    #[serde(rename = "biz-udp-gothic")]
    BizUdpGothic,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiPreferences {
    pub font: FontChoice,
}

impl UiPreferences {
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|_| "GUI設定の形式が不正なため、標準フォントで起動しました。".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("GUI設定を読み込めません。標準フォントで起動しました。".into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|_| "フォント設定を保存できません。")?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|_| "GUI設定フォルダを作成できません。")?;
        }
        fs::write(path, text)
            .map_err(|_| "フォント設定を保存できません。この起動中のみ反映されます。".into())
    }
}

pub fn apply_font(ctx: &egui::Context, choice: FontChoice) {
    let (regular, emphasis): (&'static [u8], &'static [u8]) = match choice {
        FontChoice::LineSeedJp => {
            let regular =
                include_bytes!("../../../assets/fonts/line-seed-jp/LINESeedJP-Regular.ttf");
            (regular, regular)
        }
        FontChoice::BizUdpGothic => (
            include_bytes!("../../../assets/fonts/biz-udp-gothic/BIZUDPGothic-Regular.ttf"),
            include_bytes!("../../../assets/fonts/biz-udp-gothic/BIZUDPGothic-Bold.ttf"),
        ),
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "ui-regular".into(),
        egui::FontData::from_static(regular).into(),
    );
    fonts.font_data.insert(
        "ui-emphasis".into(),
        egui::FontData::from_static(emphasis).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "ui-regular".into());
    // Preserve the bundled font fallbacks for symbols and monospace Latin code.
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("ui-regular".into());
    let mut emphasis_family = fonts.families[&egui::FontFamily::Proportional].clone();
    emphasis_family.insert(0, "ui-emphasis".into());
    fonts
        .families
        .insert(egui::FontFamily::Name("ui-bold".into()), emphasis_family);
    ctx.set_fonts(fonts);
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "codebot-ui-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn missing_settings_default_to_line_seed_and_both_choices_survive_restart() {
        let dir = TestDir::new();
        let path = dir.0.join("settings/gui-preferences.toml");
        assert_eq!(
            UiPreferences::load(&path).unwrap().font,
            FontChoice::LineSeedJp
        );
        for font in [FontChoice::BizUdpGothic, FontChoice::LineSeedJp] {
            UiPreferences { font }.save(&path).unwrap();
            assert_eq!(UiPreferences::load(&path).unwrap().font, font);
        }
    }
    #[test]
    fn invalid_settings_are_reported_without_rewriting_the_file() {
        let dir = TestDir::new();
        let path = dir.0.join("gui-preferences.toml");
        let original = "font = 'unknown-font'";
        fs::write(&path, original).unwrap();
        assert!(UiPreferences::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
    #[test]
    fn write_failures_are_reported() {
        let dir = TestDir::new();
        assert!(UiPreferences::default().save(&dir.0).is_err());
        assert!(dir.0.is_dir());
    }
}
