use anyhow::{Context, Result};
use chrono::{Datelike, Days, Local, NaiveDate};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wechat_export::core::{
    export::{self, ExportConfig, ExportResult, Progress},
    model::{Conversation, Count, ExportFields, Filter, Format},
    wechat::{
        database::WechatClient,
        discovery::{self, Account},
    },
};

enum Command {
    Discover(PathBuf),
    Connect(Account),
    Count(Filter),
    Export(ExportConfig),
    Disconnect,
    Quit,
}
enum Event {
    Found(String, Vec<Account>),
    Connected(String, String, Vec<Conversation>),
    Preview(Filter, Count),
    Status(String),
    Progress(Progress),
    Done(ExportResult),
    Failure(String),
    Disconnected,
}
fn spawn(
    ctx: egui::Context,
    work: PathBuf,
    cancel: Arc<AtomicBool>,
) -> (Sender<Command>, Receiver<Event>, JoinHandle<()>) {
    let (tx, commands) = mpsc::channel();
    let (events, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let send = |event| {
            let _ = events.send(event);
            ctx.request_repaint();
        };
        let mut client: Option<WechatClient> = None;
        while let Ok(command) = commands.recv() {
            if matches!(command, Command::Quit) {
                break;
            }
            let result: Result<()> = (|| {
                match command {
                    Command::Quit => return Ok(()),
                    Command::Disconnect => {
                        client = None;
                        send(Event::Disconnected);
                    }
                    Command::Discover(root) => {
                        client = None;
                        let version = discovery::detect().unwrap_or_else(|e| e.to_string());
                        let accounts = discovery::accounts(&root)?;
                        send(Event::Found(version, accounts));
                    }
                    Command::Connect(account) => {
                        client = None;
                        let connected = WechatClient::open(&account, &work, cancel.clone(), |s| {
                            send(Event::Status(s.into()))
                        })?;
                        send(Event::Connected(
                            connected.version.clone(),
                            connected.self_name.clone(),
                            connected.conversations.clone(),
                        ));
                        client = Some(connected);
                    }
                    Command::Count(filter) => {
                        let count = client
                            .as_ref()
                            .context("请先连接微信")?
                            .count_messages(&filter, &cancel)?;
                        send(Event::Preview(filter, count));
                    }
                    Command::Export(config) => {
                        let result = export::export(
                            client.as_ref().context("请先连接微信")?,
                            &config,
                            &cancel,
                            |p| send(Event::Progress(p)),
                        )?;
                        send(Event::Done(result));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let cancelled = cancel.load(Ordering::Relaxed);
                if !cancelled {
                    let _ = export::write_log(&work, &error);
                }
                send(Event::Failure(if cancelled {
                    "操作已取消".into()
                } else {
                    error.to_string()
                }));
            }
        }
        drop(client);
    });
    (tx, rx, handle)
}

pub struct App {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    root: String,
    accounts: Vec<Account>,
    account: usize,
    version: String,
    current_name: String,
    connected: bool,
    conversations: Vec<Conversation>,
    all: bool,
    selected: BTreeSet<String>,
    search: String,
    preset: usize,
    start: String,
    end: String,
    start_month: (i32, u32),
    end_month: (i32, u32),
    output: String,
    format: Format,
    fields: ExportFields,
    preview: Option<(Filter, Count)>,
    status: String,
    error: bool,
    busy: bool,
    progress: Option<Progress>,
    started: Option<Instant>,
    closing: bool,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_visuals(egui::Visuals::light());
        let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ctx.set_style_of(egui::Theme::Light, style);
        let mut fonts = egui::FontDefinitions::default();
        if let Some(windows) = std::env::var_os("WINDIR") {
            for (name, file) in [("cjk", "msyh.ttc"), ("emoji", "seguiemj.ttf")] {
                if let Ok(bytes) = std::fs::read(PathBuf::from(&windows).join("Fonts").join(file)) {
                    fonts
                        .font_data
                        .insert(name.into(), egui::FontData::from_owned(bytes).into());
                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push(name.into());
                }
            }
        }
        ctx.set_fonts(fonts);
        let parent = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx, worker) = spawn(
            ctx.clone(),
            parent.join(".wechat-export-work"),
            cancel.clone(),
        );
        let today = Local::now().date_naive();
        let root = discovery::default_root()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let busy = !root.is_empty();
        if busy {
            let _ = tx.send(Command::Discover(PathBuf::from(&root)));
        }
        Self {
            tx,
            rx,
            worker: Some(worker),
            cancel,
            root,
            accounts: Vec::new(),
            account: 0,
            version: "检测中".into(),
            current_name: String::new(),
            connected: false,
            conversations: Vec::new(),
            all: true,
            selected: BTreeSet::new(),
            search: String::new(),
            preset: 0,
            start: today.to_string(),
            end: today.to_string(),
            start_month: (today.year(), today.month()),
            end_month: (today.year(), today.month()),
            output: parent.to_string_lossy().into(),
            format: Format::Jsonl,
            fields: ExportFields::default(),
            preview: None,
            status: "选择账号后连接微信".into(),
            error: false,
            busy,
            progress: None,
            started: None,
            closing: false,
        }
    }
    fn run(&mut self, command: Command) {
        self.cancel.store(false, Ordering::Relaxed);
        self.error = false;
        self.busy = true;
        self.progress = None;
        self.started = Some(Instant::now());
        if self.tx.send(command).is_err() {
            self.status = "后台线程已结束，请重新启动工具".into();
            self.error = true;
            self.busy = false;
        }
    }
    fn filter(&self) -> Result<Filter> {
        let today = Local::now().date_naive();
        let mut filter = match self.preset {
            0 => Filter::default(),
            1..=3 => {
                let days = [7, 30, 90][self.preset - 1];
                Filter::dates(
                    today
                        .checked_sub_days(Days::new(days - 1))
                        .context("日期无效")?,
                    today,
                )?
            }
            4 => Filter::dates(
                NaiveDate::from_ymd_opt(today.year(), 1, 1).context("日期无效")?,
                today,
            )?,
            _ => Filter::dates(
                NaiveDate::parse_from_str(&self.start, "%Y-%m-%d")
                    .context("开始日期格式应为 YYYY-MM-DD")?,
                NaiveDate::parse_from_str(&self.end, "%Y-%m-%d")
                    .context("结束日期格式应为 YYYY-MM-DD")?,
            )?,
        };
        filter.conversations = if self.all {
            None
        } else {
            Some(self.selected.clone())
        };
        Ok(filter)
    }
    fn receive(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Found(version, accounts) => {
                    self.version = version;
                    self.accounts = accounts;
                    self.account = 0;
                    self.connected = false;
                    self.busy = false;
                    self.status = if self.accounts.is_empty() {
                        "未找到账号，请选择 xwechat_files 或账号目录"
                    } else {
                        "已找到账号，请点击连接"
                    }
                    .into();
                }
                Event::Connected(version, name, conversations) => {
                    self.version = version;
                    self.current_name = name;
                    self.conversations = conversations;
                    self.connected = true;
                    self.busy = false;
                    self.status = "已连接只读副本；重新连接可获取更新的消息".into();
                    self.preview = None;
                }
                Event::Preview(filter, count) => {
                    self.preview = Some((filter, count));
                    self.busy = false;
                    self.status = "统计完成".into();
                }
                Event::Status(status) => self.status = status,
                Event::Progress(progress) => {
                    self.status = "正在导出".into();
                    self.progress = Some(progress);
                }
                Event::Done(result) => {
                    self.status = format!(
                        "成功导出 {} 条，耗时 {:.1} 秒\n{}",
                        result.messages,
                        result.elapsed,
                        result.path.display()
                    );
                    self.busy = false;
                }
                Event::Failure(error) => {
                    self.status = error;
                    self.error = true;
                    self.busy = false;
                }
                Event::Disconnected => {
                    self.connected = false;
                    self.busy = false;
                    self.conversations.clear();
                    self.selected.clear();
                    self.preview = None;
                }
            }
        }
    }
    fn disconnect(&mut self) {
        self.connected = false;
        self.preview = None;
        self.conversations.clear();
        self.selected.clear();
        let _ = self.tx.send(Command::Disconnect);
    }
    fn content(&mut self, ui: &mut egui::Ui) {
        ui.heading("微信聊天导出");
        ui.label(
            RichText::new("选择聊天与时间，保存为 JSONL / JSON").color(Color32::from_gray(95)),
        );
        ui.separator();
        ui.label(format!("微信：{}", self.version));
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.horizontal(|ui| {
                ui.label("数据目录");
                if ui.button("选择…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("选择 xwechat_files 或账号目录")
                        .pick_folder()
                {
                    self.root = path.to_string_lossy().into();
                    self.disconnect();
                    self.run(Command::Discover(path));
                }
            });
            if ui
                .add(egui::TextEdit::singleline(&mut self.root).desired_width(f32::INFINITY))
                .changed()
            {
                self.disconnect();
                self.accounts.clear();
            }
            ui.horizontal(|ui| {
                if ui.button("检测").clicked() {
                    self.disconnect();
                    self.run(Command::Discover(PathBuf::from(
                        self.root.trim_matches('"'),
                    )));
                }
                let before = self.account;
                egui::ComboBox::from_id_salt("account")
                    .width((ui.available_width() - 75.0).max(100.0))
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .selected_text(
                        self.accounts
                            .get(self.account)
                            .map(|a| a.label.as_str())
                            .unwrap_or("无账号"),
                    )
                    .show_ui(ui, |ui| {
                        for (i, account) in self.accounts.iter().enumerate() {
                            ui.selectable_value(&mut self.account, i, &account.label);
                        }
                    });
                if before != self.account {
                    self.disconnect();
                }
                if ui
                    .add_enabled(
                        !self.accounts.is_empty(),
                        egui::Button::new(if self.connected { "重连" } else { "连接" }),
                    )
                    .clicked()
                    && let Some(account) = self.accounts.get(self.account).cloned()
                {
                    self.connected = false;
                    self.preview = None;
                    self.run(Command::Connect(account));
                }
            });
        });
        if self.connected {
            ui.colored_label(
                Color32::from_rgb(20, 115, 80),
                format!(
                    "已连接 · {} · {} 个会话",
                    self.current_name,
                    self.conversations.len()
                ),
            );
        }
        ui.separator();
        ui.add_enabled_ui(self.connected && !self.busy, |ui| {
            ui.horizontal(|ui| {
                ui.strong("会话范围");
                ui.radio_value(&mut self.all, true, "全部会话");
                ui.radio_value(&mut self.all, false, "选择会话");
            });
            if !self.all {
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索联系人 / 群聊")
                        .desired_width(f32::INFINITY),
                );
                let query = self.search.to_lowercase();
                let visible: Vec<_> = self
                    .conversations
                    .iter()
                    .filter(|c| {
                        c.name.to_lowercase().contains(&query)
                            || c.id.to_lowercase().contains(&query)
                    })
                    .collect();
                ui.horizontal(|ui| {
                    if ui.button("全选搜索结果").clicked() {
                        self.selected.extend(visible.iter().map(|c| c.id.clone()));
                    }
                    if ui.button("清空").clicked() {
                        self.selected.clear();
                    }
                    ui.label(format!("已选 {}", self.selected.len()));
                });
                egui::ScrollArea::vertical()
                    .id_salt("conversations")
                    .max_height(155.0)
                    .show_rows(ui, 24.0, visible.len(), |ui, range| {
                        for i in range {
                            let c = visible[i];
                            let mut selected = self.selected.contains(&c.id);
                            if ui
                                .checkbox(
                                    &mut selected,
                                    format!("{}{}", if c.is_group { "[群] " } else { "" }, c.name),
                                )
                                .on_hover_text(&c.id)
                                .changed()
                            {
                                if selected {
                                    self.selected.insert(c.id.clone());
                                } else {
                                    self.selected.remove(&c.id);
                                }
                            }
                        }
                    });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("时间范围");
                let labels = [
                    "全部时间",
                    "近 7 天",
                    "近 30 天",
                    "近 90 天",
                    "今年",
                    "自定义",
                ];
                egui::ComboBox::from_id_salt("dates")
                    .selected_text(labels[self.preset])
                    .show_ui(ui, |ui| {
                        for (i, label) in labels.iter().enumerate() {
                            ui.selectable_value(&mut self.preset, i, *label);
                        }
                    });
            });
            if self.preset == 5 {
                ui.horizontal(|ui| {
                    ui.label("开始");
                    date_input(ui, "start", &mut self.start, &mut self.start_month);
                });
                ui.horizontal(|ui| {
                    ui.label("结束");
                    date_input(ui, "end", &mut self.end, &mut self.end_month);
                });
            }
            ui.small("按本机时区，包含结束日期当天的全部消息");
        });
        ui.separator();
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.horizontal(|ui| {
                ui.strong("输出");
                ui.radio_value(&mut self.format, Format::Jsonl, "JSONL（推荐）");
                ui.radio_value(&mut self.format, Format::Json, "JSON");
            });
            ui.horizontal(|ui| {
                ui.strong("导出字段");
                if ui.button("精简").clicked() {
                    self.fields = ExportFields::default();
                }
                if ui.button("全选字段").clicked() {
                    self.fields = ExportFields::all();
                }
                ui.label(format!("已选 {} 项", self.fields.selected.len()));
            });
            field_checkboxes(
                ui,
                "common_fields",
                &mut self.fields,
                &ExportFields::OPTIONS[..4],
            );
            ui.collapsing("更多字段", |ui| {
                field_checkboxes(
                    ui,
                    "more_fields",
                    &mut self.fields,
                    &ExportFields::OPTIONS[4..],
                );
            });
            if self.fields.selected.is_empty() {
                ui.colored_label(Color32::DARK_RED, "请至少勾选一个导出字段");
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.output)
                        .desired_width(ui.available_width() - 70.0),
                );
                if ui.button("选择…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("选择导出目录")
                        .pick_folder()
                {
                    self.output = path.to_string_lossy().into();
                }
            });
        });
        ui.separator();
        let filter = self.filter();
        let preview = filter
            .as_ref()
            .ok()
            .and_then(|f| self.preview.as_ref().filter(|(old, _)| old == f));
        if let Some((_, count)) = preview {
            ui.strong(format!(
                "预计导出：{} 个会话 / {} 条消息",
                count.conversations, count.messages
            ));
        } else {
            ui.label("预计导出：请先统计当前范围");
        }
        if let Ok(f) = &filter {
            use chrono::TimeZone;
            let start = f
                .start
                .and_then(|t| Local.timestamp_opt(t, 0).single())
                .map(|v| v.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "最早记录".into());
            let end = f
                .end
                .and_then(|t| Local.timestamp_opt(t - 1, 0).single())
                .map(|v| v.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "最新记录".into());
            ui.small(format!("{start} ~ {end}"));
        } else if let Err(error) = &filter {
            ui.colored_label(Color32::DARK_RED, error.to_string());
        }
        let can_export = preview.is_some() && !self.fields.selected.is_empty();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.connected && !self.busy && filter.is_ok(),
                    egui::Button::new("统计消息"),
                )
                .clicked()
                && let Ok(filter) = &filter
            {
                self.run(Command::Count(filter.clone()));
            }
            if ui
                .add_enabled(
                    self.connected && !self.busy && can_export,
                    egui::Button::new("开始导出"),
                )
                .clicked()
                && let Ok(filter) = &filter
            {
                self.run(Command::Export(ExportConfig {
                    filter: filter.clone(),
                    format: self.format,
                    fields: self.fields.clone(),
                    directory: PathBuf::from(&self.output),
                }));
            }
            if ui
                .add_enabled(self.busy, egui::Button::new("取消"))
                .clicked()
            {
                self.cancel.store(true, Ordering::Relaxed);
                self.status = "正在取消…".into();
            }
        });
        if let Some(progress) = &self.progress {
            let fraction = if progress.total == 0 {
                1.0
            } else {
                progress.done as f32 / progress.total as f32
            };
            ui.add(egui::ProgressBar::new(fraction).show_percentage());
            ui.label(format!(
                "{} / {} 条 · {:.1} 秒",
                progress.done, progress.total, progress.elapsed
            ));
            ui.label(format!("当前会话：{}", progress.conversation));
        } else if self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "处理中 · {:.1} 秒",
                    self.started
                        .map(|s| s.elapsed().as_secs_f64())
                        .unwrap_or(0.0)
                ));
            });
        }
        ui.add_space(4.0);
        ui.colored_label(
            if self.error {
                Color32::DARK_RED
            } else {
                Color32::from_gray(70)
            },
            &self.status,
        );
    }
}
fn field_checkboxes(
    ui: &mut egui::Ui,
    id: &str,
    fields: &mut ExportFields,
    options: &[(&'static str, &'static str)],
) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        for (i, &(key, label)) in options.iter().enumerate() {
            let mut selected = fields.selected.contains(key);
            if ui
                .checkbox(&mut selected, label)
                .on_hover_text(key)
                .changed()
            {
                if selected {
                    fields.selected.insert(key);
                } else {
                    fields.selected.remove(key);
                }
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

fn date_input(ui: &mut egui::Ui, id: &str, text: &mut String, month: &mut (i32, u32)) {
    ui.push_id(id, |ui| {
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(92.0)
                .hint_text("YYYY-MM-DD"),
        );
        let button = ui.button("▦").on_hover_text("选择日期");
        egui::Popup::from_toggle_button_response(&button).show(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut month.0).range(1970..=9998));
                ui.label("年");
                ui.add(egui::DragValue::new(&mut month.1).range(1..=12));
                ui.label("月");
            });
            if let Some(first) = NaiveDate::from_ymd_opt(month.0, month.1, 1) {
                egui::Grid::new("calendar").show(ui, |ui| {
                    for day in ["一", "二", "三", "四", "五", "六", "日"] {
                        ui.label(day);
                    }
                    ui.end_row();
                    let offset = first.weekday().num_days_from_monday();
                    for cell in 0..42u32 {
                        if let Some(day) = cell
                            .checked_sub(offset)
                            .and_then(|n| NaiveDate::from_ymd_opt(month.0, month.1, n + 1))
                        {
                            if ui.button(day.day().to_string()).clicked() {
                                *text = day.to_string();
                                ui.close();
                            }
                        } else {
                            ui.label(" ");
                        }
                        if cell % 7 == 6 {
                            ui.end_row();
                        }
                    }
                });
            }
        });
    });
}
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.receive();
        if ctx.input(|i| i.viewport().close_requested()) && self.busy && !self.closing {
            self.closing = true;
            self.cancel.store(true, Ordering::Relaxed);
            let _ = self.tx.send(Command::Quit);
        }
        if self.closing {
            if self.worker.as_ref().is_some_and(|h| h.is_finished()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status = "正在取消并清理临时副本…".into();
            }
        }
        if self.busy || self.closing {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.content(ui));
        });
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Command::Quit);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
