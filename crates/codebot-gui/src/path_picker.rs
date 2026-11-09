use eframe::egui;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum Target {
    ModelFile,
    ModelFolder,
    Python,
    Server,
    PluginManifest,
}
pub struct PathPicker {
    pub target: Target,
    directory: String,
    entries: Vec<(PathBuf, bool)>,
    error: Option<String>,
    open: bool,
}
impl PathPicker {
    pub fn new(target: Target, current: &str) -> Self {
        let path = PathBuf::from(current);
        let directory = if path.is_dir() {
            path
        } else {
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        };
        let mut picker = Self {
            target,
            directory: directory.to_string_lossy().into_owned(),
            entries: vec![],
            error: None,
            open: true,
        };
        picker.refresh();
        picker
    }
    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&self.directory) {
            Ok(entries) => {
                for entry in entries.take(501).filter_map(Result::ok) {
                    if self.entries.len() == 500 {
                        self.error =
                            Some("500件まで表示します。フォルダを絞り込んでください。".into());
                        break;
                    }
                    if let Ok(kind) = entry.path().metadata() {
                        if kind.is_dir() || kind.is_file() {
                            self.entries.push((entry.path(), kind.is_dir()));
                        }
                    }
                }
                self.entries
                    .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            }
            Err(_) => {
                self.error = Some("フォルダを読み込めません。パスと権限を確認してください。".into())
            }
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut open = self.open;
        let mut selected = None;
        let mut next = None;
        let title = match self.target {
            Target::ModelFile => "GGUFファイルを選択",
            Target::ModelFolder => "モデルフォルダを選択",
            Target::Python => "Python実行ファイルを選択",
            Target::Server => "llama-serverを選択",
            Target::PluginManifest => "拡張のmanifestを選択",
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.label("フォルダのパス");
                ui.add(
                    egui::TextEdit::singleline(&mut self.directory).desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button("読み込む").clicked() {
                        self.refresh();
                    }
                    if ui.button("親フォルダ").clicked() {
                        next = PathBuf::from(&self.directory).parent().map(PathBuf::from);
                    }
                    if matches!(self.target, Target::ModelFolder)
                        && ui.button("このフォルダを選択").clicked()
                        && PathBuf::from(&self.directory).is_dir()
                    {
                        selected = Some(self.directory.clone());
                    }
                });
                if let Some(error) = &self.error {
                    ui.label(error);
                }
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (path, is_dir) in &self.entries {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if *is_dir {
                                if ui.button(format!("{name}/")).clicked() {
                                    next = Some(path.clone());
                                }
                            } else if !matches!(self.target, Target::ModelFolder) {
                                let selectable = !matches!(self.target, Target::ModelFile)
                                    || path
                                        .extension()
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"));
                                let selectable = selectable
                                    && (!matches!(self.target, Target::PluginManifest)
                                        || path
                                            .extension()
                                            .is_some_and(|ext| ext.eq_ignore_ascii_case("json")));
                                if ui
                                    .add_enabled(selectable, egui::Button::new(name))
                                    .clicked()
                                {
                                    selected = Some(path.to_string_lossy().into_owned());
                                }
                            }
                        }
                    });
            });
        if let Some(path) = next {
            self.directory = path.to_string_lossy().into_owned();
            self.refresh();
        }
        self.open = open && selected.is_none();
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picker_reads_directories_and_keeps_the_current_folder_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("model")).unwrap();
        std::fs::write(dir.path().join("test.gguf"), b"GGUF").unwrap();
        let picker = PathPicker::new(
            Target::ModelFile,
            &dir.path().join("test.gguf").to_string_lossy(),
        );
        assert!(picker
            .entries
            .iter()
            .any(|(p, is_dir)| p.ends_with("model") && *is_dir));
        assert!(picker
            .entries
            .iter()
            .any(|(p, is_dir)| p.ends_with("test.gguf") && !*is_dir));
        assert_eq!(
            std::fs::read(dir.path().join("test.gguf")).unwrap(),
            b"GGUF"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                dir.path().join("test.gguf"),
                dir.path().join("linked-python"),
            )
            .unwrap();
            let picker = PathPicker::new(Target::Python, &dir.path().to_string_lossy());
            assert!(picker
                .entries
                .iter()
                .any(|(p, is_dir)| p.ends_with("linked-python") && !*is_dir));
        }
    }
}
