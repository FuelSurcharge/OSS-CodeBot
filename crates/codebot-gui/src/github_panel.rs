use codebot_core::{
    GithubClient, GithubConfig, GithubDocument, GithubItem, GithubKind, GithubPostRecord,
    GithubReceipt, GithubRepository, GithubWrite, PreparedGithubWrite,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

pub enum Action {
    Attach(String),
    Branch { branch: String, source: String },
    Save,
}
enum Event {
    Repository(GithubRepository),
    List(Vec<GithubItem>),
    Document(GithubDocument),
    Posted(
        Result<GithubReceipt, String>,
        Result<Vec<GithubPostRecord>, String>,
    ),
    Records(Vec<GithubPostRecord>),
}
struct Task {
    rx: Option<mpsc::Receiver<Result<Event, String>>>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.rx = None;
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
pub struct GithubPanel {
    pub config: GithubConfig,
    token: String,
    repository: Option<GithubRepository>,
    kind: GithubKind,
    page: u32,
    items: Vec<GithubItem>,
    document: Option<GithubDocument>,
    number: String,
    tab: u8,
    write_kind: u8,
    title: String,
    body: String,
    head: String,
    base: String,
    draft: bool,
    comment_number: String,
    plan: Option<PreparedGithubWrite>,
    confirmed: bool,
    focus_review: bool,
    records: Vec<GithubPostRecord>,
    receipt: Option<GithubReceipt>,
    message: Option<String>,
    task: Option<Task>,
    markdown: egui_commonmark::CommonMarkCache,
}
impl Default for GithubPanel {
    fn default() -> Self {
        Self {
            config: GithubConfig::default(),
            token: String::new(),
            repository: None,
            kind: GithubKind::Issue,
            page: 1,
            items: vec![],
            document: None,
            number: String::new(),
            tab: 0,
            write_kind: 0,
            title: String::new(),
            body: String::new(),
            head: String::new(),
            base: String::new(),
            draft: true,
            comment_number: String::new(),
            plan: None,
            confirmed: false,
            focus_review: false,
            records: vec![],
            receipt: None,
            message: None,
            task: None,
            markdown: Default::default(),
        }
    }
}
impl GithubPanel {
    pub fn report_message(&mut self, message: String) {
        self.message = Some(message);
    }
    pub fn is_busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn shutdown(&mut self) {
        self.task = None;
    }
    fn client(&self) -> Result<GithubClient, String> {
        GithubClient::new(&self.config, Some(&self.token)).map_err(|e| e.to_string())
    }
    fn ledger(&self) -> Result<PathBuf, String> {
        codebot_core::github_ledger_dir().ok_or_else(|| "送信記録の保存先を取得できません".into())
    }
    fn reset(&mut self) {
        self.plan = None;
        self.confirmed = false;
        self.focus_review = false;
    }
    fn source_changed(&mut self) {
        self.markdown = Default::default();
        self.repository = None;
        self.items.clear();
        self.document = None;
        self.page = 1;
        self.reset();
        self.receipt = None;
        self.message = None;
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce(&AtomicBool) -> Result<Event, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        let join = std::thread::spawn(move || {
            let _ = tx.send(job(&worker_cancel));
            ctx.request_repaint();
        });
        self.task = Some(Task {
            rx: Some(rx),
            cancel,
            join: Some(join),
        });
        self.message = None;
        self.receipt = None;
    }
    pub fn poll(&mut self) {
        let result = self
            .task
            .as_ref()
            .and_then(|task| task.rx.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(event) => Some(event),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "GitHub処理が中断しました。送信記録と一覧を確認してください".into(),
                )),
                _ => None,
            });
        let Some(result) = result else {
            return;
        };
        let task = self.task.take().unwrap();
        drop(task);
        match result {
            Ok(Event::Repository(repo)) => {
                if self.base.is_empty() {
                    self.base = repo.default_branch.clone();
                    self.reset();
                }
                self.repository = Some(repo);
                self.message = Some("リポジトリを読み込みました。".into());
            }
            Ok(Event::List(items)) => self.items = items,
            Ok(Event::Document(doc)) => {
                self.reset();
                self.comment_number = doc.item.number.to_string();
                self.document = Some(doc);
            }
            Ok(Event::Records(records)) => self.records = records,
            Ok(Event::Posted(receipt, records)) => {
                match receipt {
                    Ok(receipt) => {
                        self.message = Some(receipt.note.clone());
                        self.receipt = Some(receipt);
                    }
                    Err(error) => self.message = Some(error),
                }
                match records {
                    Ok(records) => self.records = records,
                    Err(error) => {
                        self.message = Some(format!(
                            "{} 送信記録の読み込み：{error}",
                            self.message.as_deref().unwrap_or("")
                        ))
                    }
                }
            }
            Err(error) => self.message = Some(error),
        }
    }
    fn prepare(&mut self) {
        let action = match self.write_kind {
            0 => Ok(GithubWrite::CreateIssue {
                title: self.title.clone(),
                body: self.body.clone(),
            }),
            1 => Ok(GithubWrite::CreatePull {
                title: self.title.clone(),
                body: self.body.clone(),
                head: self.head.clone(),
                base: self.base.clone(),
                draft: self.draft,
            }),
            _ => self
                .comment_number
                .trim()
                .parse::<u64>()
                .map(|number| GithubWrite::Comment {
                    number,
                    body: self.body.clone(),
                })
                .map_err(|_| "コメント先のIssue/PR番号を指定してください".to_owned()),
        };
        match action.and_then(|action| {
            self.client()?
                .prepare_write(action)
                .map_err(|e| e.to_string())
        }) {
            Ok(plan) => {
                self.plan = Some(plan);
                self.confirmed = false;
                self.focus_review = true;
                self.message = None;
            }
            Err(error) => self.message = Some(error),
        }
    }
    fn post(&mut self, ctx: &egui::Context) {
        let Some(plan) = self.plan.take() else {
            return;
        };
        let directory = match self.ledger() {
            Ok(path) => path,
            Err(error) => {
                self.message = Some(error);
                return;
            }
        };
        self.confirmed = false;
        self.start(ctx, move |cancel| {
            let receipt = codebot_core::post_github_write(&plan, &directory, cancel)
                .map_err(|e| e.to_string());
            let records = codebot_core::github_post_records(&directory).map_err(|e| e.to_string());
            Ok(Event::Posted(receipt, records))
        });
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, can_post: bool) -> Option<Action> {
        ui.heading("GitHub");
        let busy = self.is_busy();
        let mut action = None;
        let mut changed = false;
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap());
        }
        if let Some(receipt) = &self.receipt {
            ui.hyperlink_to(format!("投稿先を開く：{}", receipt.number), &receipt.url);
        }
        ui.add_enabled_ui(!busy,|ui| {
            ui.label("リポジトリ（owner/repository）");changed|=ui.add(egui::TextEdit::singleline(&mut self.config.repository).desired_width(f32::INFINITY)).changed();
            ui.collapsing("認証と接続先",|ui| {
                ui.label("GitHub API URL");if ui.add(egui::TextEdit::singleline(&mut self.config.api_url).desired_width(f32::INFINITY)).changed() {self.token.clear();self.config.token_env.clear();changed=true;}
                ui.label("トークン（この起動中だけ使用。保存しません）");if ui.add(egui::TextEdit::singleline(&mut self.token).password(true).desired_width(f32::INFINITY)).changed() {self.reset();}
                ui.label("トークンの環境変数名");if ui.add(egui::TextEdit::singleline(&mut self.config.token_env).desired_width(f32::INFINITY)).changed() {self.reset();}
                ui.horizontal(|ui| {ui.label("待機時間（秒）");if ui.add(egui::DragValue::new(&mut self.config.timeout_seconds).range(1..=300)).changed() {self.reset();}});
                ui.label("読み込みに必要なアクセス権と、投稿にはIssue/PRへの書き込み権が必要です。公開リポジトリの読み込みはトークンなしでも可能です。");
            });
            if changed {self.source_changed();}
            ui.horizontal(|ui| {
                if ui.button("接続を確認").clicked() {match self.client() {Ok(client)=>self.start(ui.ctx(),move|cancel|client.repository(cancel).map(Event::Repository).map_err(|e|e.to_string())),Err(error)=>self.message=Some(error)}}
                if ui.button("接続設定を保存").clicked() {action=Some(Action::Save);}
            });
        });
        if let Some(repo) = &self.repository {
            ui.label(format!(
                "{} / {} / {}",
                repo.full_name,
                repo.default_branch,
                if repo.private { "非公開" } else { "公開" }
            ));
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, 0, "参照");
            ui.selectable_value(&mut self.tab, 1, "投稿");
            ui.selectable_value(&mut self.tab, 2, "送信記録");
        });
        if self.tab == 0 {
            let mut request_doc = None;
            let mut request_list = false;
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    let old = self.kind;
                    ui.selectable_value(&mut self.kind, GithubKind::Issue, "Issue");
                    ui.selectable_value(&mut self.kind, GithubKind::Pull, "Pull Request");
                    if old != self.kind {
                        self.items.clear();
                        self.document = None;
                        self.page = 1;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("ページ（1ページ50件）");
                    ui.add(egui::DragValue::new(&mut self.page).range(1..=1000));
                    if ui.button("一覧を取得").clicked() {
                        request_list = true;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("github-items")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for item in &self.items {
                            if ui
                                .selectable_label(
                                    self.document.as_ref().is_some_and(|d| {
                                        d.item.number == item.number && d.item.kind == item.kind
                                    }),
                                    format!("#{} {}", item.number, item.title),
                                )
                                .clicked()
                            {
                                request_doc = Some((item.kind, item.number));
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("番号");
                    ui.add(egui::TextEdit::singleline(&mut self.number).desired_width(100.0));
                    if ui.button("本文を読む").clicked() {
                        match self.number.trim().parse::<u64>() {
                            Ok(number) => request_doc = Some((self.kind, number)),
                            Err(_) => self.message = Some("Issue/PR番号を入力してください".into()),
                        }
                    }
                });
            });
            if request_list {
                match self.client() {
                    Ok(client) => {
                        let kind = self.kind;
                        let page = self.page;
                        self.start(ui.ctx(), move |cancel| {
                            client
                                .list(kind, page, cancel)
                                .map(Event::List)
                                .map_err(|e| e.to_string())
                        });
                    }
                    Err(error) => self.message = Some(error),
                }
            }
            if let Some((kind, number)) = request_doc {
                match self.client() {
                    Ok(client) => self.start(ui.ctx(), move |cancel| {
                        client
                            .document(kind, number, cancel)
                            .map(Event::Document)
                            .map_err(|e| e.to_string())
                    }),
                    Err(error) => self.message = Some(error),
                }
            }
            if let Some(doc) = &self.document {
                ui.separator();
                ui.label(format!(
                    "{} #{}：{}",
                    if doc.item.kind == GithubKind::Pull {
                        "PR"
                    } else {
                        "Issue"
                    },
                    doc.item.number,
                    doc.item.title
                ));
                ui.hyperlink_to("GitHubで開く", &doc.url);
                egui::ScrollArea::vertical()
                    .id_salt("github-body")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        let _ = crate::markdown::show(
                            ui,
                            &mut self.markdown,
                            (doc.item.number as usize, 0),
                            &doc.body,
                        );
                    });
                if let Some(diff) = &doc.diff {
                    crate::git_panel::diff(ui, "github-pr", diff);
                }
                ui.add_enabled_ui(!busy, |ui| {
                    if ui
                        .button("この本文と差分をチャットの下書きへ追加")
                        .clicked()
                    {
                        action = Some(Action::Attach(doc.draft_context()));
                    }
                    if doc.item.kind == GithubKind::Issue
                        && ui.button("このIssueから作業ブランチを準備").clicked()
                    {
                        action = Some(Action::Branch {
                            branch: doc.branch_name(),
                            source: doc.url.clone(),
                        });
                    }
                    if ui.button("この対象にコメントを作成").clicked() {
                        self.comment_number = doc.item.number.to_string();
                        self.write_kind = 2;
                        self.tab = 1;
                        self.plan = None;
                        self.confirmed = false;
                        self.focus_review = false;
                    }
                });
            }
        } else if self.tab == 1 {
            let mut edit = false;
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    for (kind, label) in [(0, "Issue作成"), (1, "PR作成"), (2, "コメント")]
                    {
                        edit |= ui
                            .selectable_value(&mut self.write_kind, kind, label)
                            .changed();
                    }
                });
                if self.write_kind != 2 {
                    ui.label("タイトル");
                    edit |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.title)
                                .desired_width(f32::INFINITY),
                        )
                        .changed();
                } else {
                    ui.label("投稿先のIssue/PR番号");
                    edit |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.comment_number)
                                .desired_width(140.0),
                        )
                        .changed();
                }
                if self.write_kind == 1 {
                    ui.label("送信元ブランチ（head）");
                    edit |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.head).desired_width(f32::INFINITY),
                        )
                        .changed();
                    ui.label("対象ブランチ（base）");
                    edit |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.base).desired_width(f32::INFINITY),
                        )
                        .changed();
                    edit |= ui
                        .checkbox(&mut self.draft, "下書きのPRとして作成")
                        .changed();
                }
                ui.label("投稿本文");
                edit |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.body)
                            .desired_rows(6)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if edit {
                    self.reset();
                }
                if ui.button("送信先と本文をプレビュー").clicked() {
                    self.prepare();
                }
            });
            if let Some(plan) = &self.plan {
                if std::mem::take(&mut self.focus_review) {
                    ui.scroll_to_cursor(Some(egui::Align::Min));
                }
                ui.separator();
                ui.label(format!(
                    "送信先：{} / {}",
                    plan.api_url(),
                    plan.repository()
                ));
                let body = match plan.action() {
                    GithubWrite::CreateIssue { title, body } => {
                        ui.label(format!("Issue作成：{title}"));
                        body
                    }
                    GithubWrite::CreatePull {
                        title,
                        body,
                        head,
                        base,
                        draft,
                    } => {
                        ui.label(format!("PR作成：{title}\n{head} → {base}\n下書き：{draft}"));
                        body
                    }
                    GithubWrite::Comment { number, body } => {
                        ui.label(format!("Issue/PR #{number}へのコメント"));
                        body
                    }
                };
                egui::ScrollArea::vertical()
                    .id_salt("github-post-preview")
                    .min_scrolled_height(120.0)
                    .max_height(240.0)
                    .show(ui, |ui| {
                        let _ =
                            crate::markdown::show(ui, &mut self.markdown, (usize::MAX, 1), body);
                    });
                if let Ok(path) = self.ledger() {
                    ui.label(format!("送信記録：{}", path.display()));
                }
                let recorded = self
                    .records
                    .iter()
                    .any(|r| r.fingerprint == plan.fingerprint());
                if recorded {
                    ui.label(
                        "同じ内容の送信記録があります。GitHubの一覧で結果を確認してください。",
                    );
                }
                ui.add_enabled(
                    can_post && !busy && !recorded,
                    egui::Checkbox::new(&mut self.confirmed, "投稿先と本文を確認しました"),
                );
                if ui
                    .add_enabled(
                        can_post && !busy && !recorded && self.confirmed,
                        egui::Button::new("確認した内容をGitHubへ投稿"),
                    )
                    .clicked()
                {
                    self.post(ui.ctx());
                }
            }
        } else {
            if ui
                .add_enabled(!busy, egui::Button::new("送信記録を読み込む"))
                .clicked()
            {
                match self.ledger() {
                    Ok(directory) => self.start(ui.ctx(), move |_| {
                        codebot_core::github_post_records(&directory)
                            .map(Event::Records)
                            .map_err(|e| e.to_string())
                    }),
                    Err(error) => self.message = Some(error),
                }
            }
            ui.add(egui::Label::new("pending-or-uncertainは送信結果が不明です。GitHubで確認し、同じ内容を再投稿しないでください。トークンと投稿本文は記録しません。").wrap());
            for record in self.records.iter().filter(|r| {
                r.repository
                    .eq_ignore_ascii_case(self.config.repository.trim())
            }) {
                ui.label(format!(
                    "{} / {} / {} / {}",
                    record.api_url,
                    record.repository,
                    record.state,
                    record
                        .number
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "番号不明".into())
                ));
            }
        }
        if let Some(task) = &self.task {
            ui.horizontal(|ui| {
                ui.label("GitHubを処理中…");
                if ui.button("通信を中止").clicked() {
                    task.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_the_selected_issue_invalidates_the_old_comment_confirmation() {
        let mut panel = GithubPanel::default();
        panel.config.repository = "owner/repo".into();
        panel.token = "fixture-private-key".into();
        panel.write_kind = 2;
        panel.comment_number = "42".into();
        panel.body = "Comment".into();
        panel.prepare();
        panel.confirmed = true;
        assert!(panel.plan.is_some());
        let (tx, rx) = mpsc::channel();
        panel.task = Some(Task {
            rx: Some(rx),
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
        });
        tx.send(Ok(Event::Document(GithubDocument {
            repository: "owner/repo".into(),
            item: GithubItem {
                number: 99,
                title: "Other issue".into(),
                state: "open".into(),
                kind: GithubKind::Issue,
            },
            body: "Body".into(),
            diff: None,
            url: "https://github.com/owner/repo/issues/99".into(),
            head: None,
            base: None,
        })))
        .unwrap();
        panel.poll();
        assert!(panel.plan.is_none());
        assert!(!panel.confirmed);
        assert_eq!(panel.comment_number, "99");
        assert_eq!(panel.body, "Comment");
        assert!(!serde_json::to_string(&panel.config)
            .unwrap()
            .contains("fixture-private-key"));
        panel.prepare();
        panel.confirmed = true;
        panel.config.repository = "owner/other".into();
        panel.source_changed();
        assert!(panel.plan.is_none());
        assert!(!panel.confirmed);
        assert!(panel.document.is_none());
    }
}
